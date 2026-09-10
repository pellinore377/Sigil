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
fn expired_unregistered_call_does_not_block_new_calls() {
    delayed_registration(false);
}
#[test]
fn lost_registration_response_still_closes_a_timed_out_call() {
    delayed_registration(true);
}
fn delayed_registration(accepted: bool) {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    configure(dir.path());
    let (_, peer) = trust(&mut alice, &mut bob);
    let old = [1; 32];
    alice.start_call(old, now, true, &[peer]).unwrap();
    if accepted {
        let record = load(&alice.db, &alice.key, &old).unwrap();
        alice
            .connected_client()
            .unwrap()
            .publish_call(&record.commits[0])
            .unwrap();
    }
    let later = now + 61;
    rusqlite::Connection::open(dir.path().join("server.db"))
        .unwrap()
        .execute("UPDATE call_configuration SET clock=?1", [later as i64])
        .unwrap();
    for attempt in alice.resume_calls_online(later).unwrap() {
        attempt.result.unwrap();
    }
    let record = load(&alice.db, &alice.key, &old).unwrap();
    assert!(record.phase == Phase::Ended);
    assert!(record.secret.is_none() && record.commits.is_empty());
    let live: i64 = rusqlite::Connection::open(dir.path().join("server.db"))
        .unwrap()
        .query_row("SELECT count(*) FROM calls WHERE closed=0", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(live, 0);
    let next = [2; 32];
    alice.start_call(next, later, true, &[peer]).unwrap();
    for attempt in alice.resume_calls_online(later).unwrap() {
        attempt.result.unwrap();
    }
    assert!(load(&alice.db, &alice.key, &next)
        .unwrap()
        .commits
        .is_empty());
}
#[test]
fn fresh_registration_conflict_is_not_discarded() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    configure(dir.path());
    let (_, peer) = trust(&mut alice, &mut bob);
    let id = [3; 32];
    alice.start_call(id, now, true, &[peer]).unwrap();
    let mut record = load(&alice.db, &alice.key, &id).unwrap();
    let client = alice.connected_client().unwrap();
    client.publish_call(&record.commits[0]).unwrap();
    end(&alice.key, &mut record).unwrap();
    client.publish_call(record.commits.last().unwrap()).unwrap();
    assert!(alice
        .resume_calls_online(now)
        .unwrap()
        .into_iter()
        .any(|attempt| matches!(
            attempt.result,
            Err(Error::Network(crate::network::Error::Status {
                code: 409,
                ..
            }))
        )));
    let record = load(&alice.db, &alice.key, &id).unwrap();
    assert!(record.secret.is_some() && !record.commits.is_empty());
}
#[test]
fn caller_hangup_cancels_ringing_and_an_answer_in_flight() {
    for answer in [false, true] {
        let (dir, _fixture, mut alice, mut bob, now) = pair();
        configure(dir.path());
        let (_, peer) = trust(&mut alice, &mut bob);
        let id = [96; 32];
        alice.create_direct_call(id, now, 3600).unwrap();
        alice.invite_to_call(id, peer, now).unwrap();
        pump(&mut alice, &mut bob, now);
        if answer {
            bob.answer_call(id, true, now).unwrap();
        }
        bob.replenish_prekey_online().unwrap();
        alice.leave_call(id, now).unwrap();
        pump(&mut alice, &mut bob, now);
        assert!(
            bob.call(id, now).unwrap().phase == Phase::Ended,
            "answer in flight: {answer}"
        );
        let record = load(&bob.db, &bob.key, &id).unwrap();
        assert!(record.secret.is_none() && record.shares.is_empty());
    }
}
#[test]
fn unanswered_direct_call_expires_without_leaving_the_caller_active() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    configure(dir.path());
    let (_, peer) = trust(&mut alice, &mut bob);
    let id = [94; 32];
    alice.create_direct_call(id, now, 3600).unwrap();
    alice.invite_to_call(id, peer, now).unwrap();
    pump(&mut alice, &mut bob, now);
    let mut media = alice
        .start_call_media(
            id,
            Tracks {
                audio: true,
                ..Default::default()
            },
            now,
        )
        .unwrap();
    bob.replenish_prekey_online().unwrap();
    for attempt in alice.resume_calls_online(now + 60).unwrap() {
        attempt.result.unwrap();
    }
    assert!(alice.call(id, now + 60).unwrap().phase == Phase::Ended);
    assert!(bob.call(id, now + 60).unwrap().phase == Phase::Declined);
    assert!(alice.refresh_call_media(&mut media, now + 60).is_err());
}
#[test]
fn direct_callee_hangup_closes_the_call_and_erases_both_media_leases() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    configure(dir.path());
    let (_, peer) = trust(&mut alice, &mut bob);
    let id = [95; 32];
    alice.create_direct_call(id, now, 3600).unwrap();
    alice.invite_to_call(id, peer, now).unwrap();
    pump(&mut alice, &mut bob, now);
    assert!(bob.call(id, now).unwrap().direct);
    bob.answer_call(id, true, now).unwrap();
    pump(&mut bob, &mut alice, now);
    let mut media = alice
        .start_call_media(
            id,
            Tracks {
                audio: true,
                ..Default::default()
            },
            now,
        )
        .unwrap();
    bob.leave_call(id, now).unwrap();
    pump(&mut bob, &mut alice, now);
    assert!(alice.call(id, now).unwrap().phase == Phase::Ended);
    for store in [&mut alice, &mut bob] {
        let record = load(&store.db, &store.key, &id).unwrap();
        assert!(record.secret.is_none());
        assert!(record.shares.is_empty());
        assert_eq!(record.lease, [0; 32]);
    }
    assert!(alice
        .seal_call_frame(
            &mut media,
            sigil_calls::MediaKind::Audio,
            0,
            false,
            b"no",
            now
        )
        .is_err());
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
