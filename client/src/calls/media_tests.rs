use super::*;
use crate::{claims::tests::pair, incoming::tests::trust};
fn lifecycle(blocked_end: bool) {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    crate::calls::tests::configure(dir.path());
    let (alice_peer, peer) = trust(&mut alice, &mut bob);
    let id = [76; 32];
    alice.create_call(id, now, 3600).unwrap();
    alice.invite_to_call(id, peer, now).unwrap();
    crate::calls::tests::pump(&mut alice, &mut bob, now);
    bob.answer_call(id, true, now).unwrap();
    crate::calls::tests::pump(&mut bob, &mut alice, now);
    let tracks = Tracks {
        audio: true,
        camera: false,
        screen: false,
    };
    let mut a = alice.start_call_media(id, tracks, now).unwrap();
    let mut b = bob.start_call_media(id, tracks, now).unwrap();
    crate::calls::tests::pump(&mut alice, &mut bob, now);
    alice.refresh_call_media(&mut a, now).unwrap();
    bob.refresh_call_media(&mut b, now).unwrap();
    crate::calls::tests::pump(&mut alice, &mut bob, now);
    let own = load(&alice.db, &alice.key, &id).unwrap().own_id().unwrap();
    let packet = alice
        .seal_call_frame(
            &mut a,
            sigil_calls::MediaKind::Audio,
            1,
            false,
            b"synthetic",
            now,
        )
        .unwrap();
    bob.open_call_frame(&mut b, own, sigil_calls::MediaKind::Audio, &packet, now)
        .unwrap();
    let generation = load(&bob.db, &bob.key, &id).unwrap().generation;
    b.sender = None;
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON calls BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(bob.refresh_call_media(&mut b, now).is_err());
    assert_eq!(load(&bob.db, &bob.key, &id).unwrap().generation, generation);
    bob.db.execute_batch("DROP TRIGGER fail;").unwrap();
    bob.refresh_call_media(&mut b, now).unwrap();
    assert_eq!(
        load(&bob.db, &bob.key, &id).unwrap().generation,
        generation + 1
    );
    assert!(matches!(
        bob.open_call_frame(&mut b, own, sigil_calls::MediaKind::Audio, &packet, now),
        Err(Error::Conflict)
    ));
    let lease = b.lease;
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON calls BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(bob.start_call_media(id, tracks, now).is_err());
    assert_eq!(load(&bob.db, &bob.key, &id).unwrap().lease, lease);
    bob.db.execute_batch("DROP TRIGGER fail;").unwrap();
    let packet = alice
        .seal_call_frame(
            &mut a,
            sigil_calls::MediaKind::Audio,
            2,
            false,
            b"still live",
            now,
        )
        .unwrap();
    assert_eq!(
        &*bob
            .open_call_frame(&mut b, own, sigil_calls::MediaKind::Audio, &packet, now)
            .unwrap()
            .data,
        b"still live"
    );
    let enabled = Tracks {
        audio: true,
        camera: true,
        screen: true,
    };
    alice.set_call_tracks(&mut a, enabled, now).unwrap();
    crate::calls::tests::pump(&mut alice, &mut bob, now);
    alice.refresh_call_media(&mut a, now).unwrap();
    bob.refresh_call_media(&mut b, now).unwrap();
    crate::calls::tests::pump(&mut alice, &mut bob, now);
    let video = alice
        .seal_call_frame(
            &mut a,
            sigil_calls::MediaKind::Camera,
            3,
            true,
            b"camera enabled",
            now,
        )
        .unwrap();
    assert_eq!(
        &*bob
            .open_call_frame(&mut b, own, sigil_calls::MediaKind::Camera, &video, now)
            .unwrap()
            .data,
        b"camera enabled"
    );
    alice.set_call_tracks(&mut a, tracks, now).unwrap();
    assert!(alice
        .seal_call_frame(
            &mut a,
            sigil_calls::MediaKind::Camera,
            4,
            true,
            b"camera disabled",
            now
        )
        .is_err());
    crate::calls::tests::pump(&mut alice, &mut bob, now);
    alice.refresh_call_media(&mut a, now).unwrap();
    bob.refresh_call_media(&mut b, now).unwrap();
    crate::calls::tests::pump(&mut alice, &mut bob, now);
    let packet = alice
        .seal_call_frame(
            &mut a,
            sigil_calls::MediaKind::Audio,
            3,
            false,
            b"after blocking",
            now,
        )
        .unwrap();
    bob.block_peer(alice_peer, true).unwrap();
    assert!(bob
        .open_call_frame(&mut b, own, sigil_calls::MediaKind::Audio, &packet, now)
        .is_err());
    assert!(bob
        .seal_call_frame(
            &mut b,
            sigil_calls::MediaKind::Audio,
            3,
            false,
            b"after blocking",
            now
        )
        .is_err());
    bob.block_peer(alice_peer, false).unwrap();
    b.sender = None;
    bob.refresh_call_media(&mut b, now).unwrap();
    if blocked_end {
        bob.block_peer(alice_peer, true).unwrap();
    }
    bob.leave_call(id, now).unwrap();
    for _ in 0..2 {
        for attempt in bob.resume_calls_online(now).unwrap() {
            attempt.result.unwrap();
        }
    }
}

#[test]
fn sender_key_renewal_keeps_receiver_replay_state_and_failed_commit_keeps_live_handle() {
    lifecycle(false);
    lifecycle(true);
}
