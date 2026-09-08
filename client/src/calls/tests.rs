use super::*;
use crate::{claims::tests::pair, incoming::tests::trust};
pub(super) fn configure(dir: &std::path::Path) {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let address = socket.local_addr().unwrap();
    drop(socket);
    let mut store = sigil_server::store::Store::open(&dir.join("server.db")).unwrap();
    store
        .configure_calls(sigil_server::call_config::Configure {
            expected_revision: 0,
            settings: Some(sigil_server::call_config::Settings {
                bind: address,
                advertised: address,
                max_calls: 1,
                turn_urls: Vec::new(),
            }),
            turn_secret: sigil_server::service_config::SecretUpdate::Clear,
        })
        .unwrap();
}
fn pass(sender: &mut ClientStore, receiver: &mut ClientStore, now: u64) {
    for attempt in sender.resume_calls_online(now).unwrap() {
        attempt.result.unwrap();
    }
    for attempt in sender.resume_outbound_online(now).unwrap() {
        attempt.result.unwrap();
    }
    for attempt in receiver.receive_mailbox_online(now).unwrap() {
        attempt.result.unwrap();
    }
    receiver.acknowledge_incoming_online().unwrap();
}
pub(super) fn pump(a: &mut ClientStore, b: &mut ClientStore, now: u64) {
    for _ in 0..4 {
        pass(a, b, now);
        pass(b, a, now);
    }
}
#[test]
fn authenticated_call_control_keys_replay_and_new_receiver_handle() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    configure(dir.path());
    let (_a, b) = trust(&mut alice, &mut bob);
    let id = [91; 32];
    alice.create_call(id, now, 3600).unwrap();
    alice.invite_to_call(id, b, now).unwrap();
    pump(&mut alice, &mut bob, now);
    assert!(bob.call(id, now).unwrap().phase == Phase::Ringing);
    bob.answer_call(id, true, now).unwrap();
    pump(&mut bob, &mut alice, now);
    assert!(alice.call(id, now).unwrap().phase == Phase::Active);
    assert!(bob.call(id, now).unwrap().phase == Phase::Active);
    let tracks = Tracks {
        audio: true,
        camera: true,
        screen: true,
    };
    let mut a = alice.start_call_media(id, tracks, now).unwrap();
    let mut b = bob.start_call_media(id, tracks, now).unwrap();
    pump(&mut alice, &mut bob, now);
    alice.refresh_call_media(&mut a, now).unwrap();
    bob.refresh_call_media(&mut b, now).unwrap();
    pump(&mut alice, &mut bob, now);
    let own = load(&alice.db, &alice.key, &id).unwrap().own_id().unwrap();
    let ciphertext = alice
        .seal_call_frame(
            &mut a,
            sigil_calls::MediaKind::Audio,
            10,
            false,
            b"synthetic audio",
            now,
        )
        .unwrap();
    assert_eq!(
        &*bob
            .open_call_frame(&mut b, own, sigil_calls::MediaKind::Audio, &ciphertext, now)
            .unwrap()
            .data,
        b"synthetic audio"
    );
    assert!(bob
        .open_call_frame(&mut b, own, sigil_calls::MediaKind::Audio, &ciphertext, now)
        .is_err());
    let mut replacement = bob.start_call_media(id, tracks, now).unwrap();
    assert!(bob
        .open_call_frame(&mut b, own, sigil_calls::MediaKind::Audio, &ciphertext, now)
        .is_err());
    assert!(bob
        .open_call_frame(
            &mut replacement,
            own,
            sigil_calls::MediaKind::Audio,
            &ciphertext,
            now
        )
        .is_err());
    pump(&mut bob, &mut alice, now);
    alice.refresh_call_media(&mut a, now).unwrap();
    bob.refresh_call_media(&mut replacement, now).unwrap();
    pump(&mut alice, &mut bob, now);
    assert!(bob
        .open_call_frame(
            &mut replacement,
            own,
            sigil_calls::MediaKind::Audio,
            &ciphertext,
            now
        )
        .is_err());
    let fresh = alice
        .seal_call_frame(
            &mut a,
            sigil_calls::MediaKind::Screen,
            11,
            true,
            b"synthetic screen",
            now,
        )
        .unwrap();
    assert_eq!(
        &*bob
            .open_call_frame(
                &mut replacement,
                own,
                sigil_calls::MediaKind::Screen,
                &fresh,
                now
            )
            .unwrap()
            .data,
        b"synthetic screen"
    );
    drop(replacement);
    drop(bob);
    let mut bob = ClientStore::open(
        &dir.path().join("bob.db"),
        StorageKey::new(sigil_crypto::Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap();
    let mut replacement = bob.start_call_media(id, tracks, now).unwrap();
    assert!(bob
        .open_call_frame(
            &mut replacement,
            own,
            sigil_calls::MediaKind::Screen,
            &fresh,
            now
        )
        .is_err());
    pump(&mut bob, &mut alice, now);
    alice.refresh_call_media(&mut a, now).unwrap();
    bob.refresh_call_media(&mut replacement, now).unwrap();
    pump(&mut alice, &mut bob, now);
    assert!(bob
        .open_call_frame(
            &mut replacement,
            own,
            sigil_calls::MediaKind::Screen,
            &fresh,
            now
        )
        .is_err());
    let after_restart = alice
        .seal_call_frame(
            &mut a,
            sigil_calls::MediaKind::Camera,
            12,
            true,
            b"synthetic camera",
            now,
        )
        .unwrap();
    assert_eq!(
        &*bob
            .open_call_frame(
                &mut replacement,
                own,
                sigil_calls::MediaKind::Camera,
                &after_restart,
                now
            )
            .unwrap()
            .data,
        b"synthetic camera"
    );
    alice.leave_call(id, now).unwrap();
    pump(&mut alice, &mut bob, now);
    assert!(bob.call(id, now).unwrap().phase == Phase::Ended);
    assert!(bob
        .open_call_frame(
            &mut replacement,
            own,
            sigil_calls::MediaKind::Screen,
            &fresh,
            now
        )
        .is_err());
}
