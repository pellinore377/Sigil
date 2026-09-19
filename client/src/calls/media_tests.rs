use super::*;
use crate::{claims::tests::pair, incoming::tests::trust};
#[test]
fn fragmented_video_keeps_audio_and_authentication_live() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    crate::calls::tests::configure(dir.path());
    let (alice_peer, peer) = trust(&mut alice, &mut bob);
    let id = [79; 32];
    alice.start_call(id, now, true, &[peer]).unwrap();
    crate::calls::tests::pump(&mut alice, &mut bob, now);
    bob.answer_call(id, true, now).unwrap();
    crate::calls::tests::pump(&mut alice, &mut bob, now);
    let tracks = Tracks {
        audio: true,
        camera: true,
        screen: false,
    };
    let mut a = alice.start_call_media(id, tracks, now).unwrap();
    let mut b = bob.start_call_media(id, tracks, now).unwrap();
    crate::calls::tests::pump(&mut alice, &mut bob, now);
    alice.refresh_call_media(&mut a, now).unwrap();
    bob.refresh_call_media(&mut b, now).unwrap();
    crate::calls::tests::pump(&mut alice, &mut bob, now);
    let own = load(&alice.db, &alice.key, &id).unwrap().own_id().unwrap();
    let started = std::time::Instant::now();
    let video = vec![42; 64 * 1024];
    for timestamp in 0..20 {
        let packets = alice
            .seal_call_packets(
                &mut a,
                sigil_calls::MediaKind::Camera,
                timestamp,
                true,
                &video,
                now,
            )
            .unwrap();
        for (index, packet) in packets.iter().enumerate() {
            let frame = bob
                .assemble_call_packet(&mut b, own, sigil_calls::MediaKind::Camera, packet, now)
                .unwrap();
            assert_eq!(frame.is_some(), index + 1 == packets.len());
            if let Some(frame) = frame {
                assert_eq!(*frame.data, video);
            }
        }
        let audio = alice
            .seal_call_frame(
                &mut a,
                sigil_calls::MediaKind::Audio,
                timestamp,
                false,
                b"synthetic audio",
                now,
            )
            .unwrap();
        assert_eq!(
            &*bob
                .open_call_frame(&mut b, own, sigil_calls::MediaKind::Audio, &audio, now)
                .unwrap()
                .data,
            b"synthetic audio"
        );
    }
    eprintln!("20 fragmented video/audio frames: {:?}", started.elapsed());
    let packets = alice
        .seal_call_packets(
            &mut a,
            sigil_calls::MediaKind::Camera,
            21,
            true,
            &video,
            now,
        )
        .unwrap();
    for packet in &packets[..packets.len() - 1] {
        assert!(bob
            .assemble_call_packet(&mut b, own, sigil_calls::MediaKind::Camera, packet, now)
            .unwrap()
            .is_none());
    }
    bob.block_peer(alice_peer, true).unwrap();
    assert!(bob
        .assemble_call_packet(
            &mut b,
            own,
            sigil_calls::MediaKind::Camera,
            packets.last().unwrap(),
            now
        )
        .is_err());
}
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

/// Counts the mailbox round trips a direct call needs between answering and both
/// sides holding each other's media key. Each trip is a poll plus a server hop on a
/// real device, so this number is the floor on how long "securing" lasts: joining
/// up, the roster back, readiness and our key up, the roster and their key back.
#[test]
fn direct_call_secures_within_three_harness_trips_of_answering() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    crate::calls::tests::configure(dir.path());
    let (_alice_peer, peer) = trust(&mut alice, &mut bob);
    let id = [83; 32];
    alice.start_call(id, now, true, &[peer]).unwrap();
    crate::calls::tests::pump(&mut alice, &mut bob, now);
    bob.answer_call(id, true, now).unwrap();
    let tracks = Tracks {
        audio: true,
        camera: false,
        screen: false,
    };
    let mut a = Some(alice.start_call_media(id, tracks, now).unwrap());
    let mut b = None;
    let mut trips = 0;
    for _ in 0..12 {
        // Each device opens its media handle as soon as the call admits it.
        if b.is_none() {
            b = bob.start_call_media(id, tracks, now).ok();
        }
        let secured = |store: &mut ClientStore, media: Option<&mut Media>| {
            media.is_some_and(|m| matches!(store.refresh_call_media(m, now), Ok(n) if n > 0))
        };
        // Both refresh every trip, as live clients do; the owner secures on the callee's first refresh.
        let alice_secured = secured(&mut alice, a.as_mut());
        let bob_secured = secured(&mut bob, b.as_mut());
        if alice_secured && bob_secured {
            break;
        }
        trips += 1;
        crate::calls::tests::round_trip(&mut alice, &mut bob, now);
    }
    // Join and readiness travel together, so the callee secures on the owner's first reply and the
    // owner on the callee's first refresh; the harness only refreshes between trips, hence three.
    assert_eq!(trips, 3, "round trips from answering to secured");
}

#[test]
fn a_track_change_keeps_both_sides_secured_throughout() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    crate::calls::tests::configure(dir.path());
    let (_alice_peer, peer) = trust(&mut alice, &mut bob);
    let id = [84; 32];
    alice.start_call(id, now, true, &[peer]).unwrap();
    crate::calls::tests::pump(&mut alice, &mut bob, now);
    bob.answer_call(id, true, now).unwrap();
    let tracks = Tracks { audio: true, camera: false, screen: false };
    let mut a = alice.start_call_media(id, tracks, now).unwrap();
    let mut b = None;
    for _ in 0..6 {
        if b.is_none() {
            b = bob.start_call_media(id, tracks, now).ok();
        }
        let _ = alice.refresh_call_media(&mut a, now);
        if let Some(b) = b.as_mut() {
            let _ = bob.refresh_call_media(b, now);
        }
        crate::calls::tests::round_trip(&mut alice, &mut bob, now);
    }
    let mut b = b.unwrap();
    assert_eq!(alice.refresh_call_media(&mut a, now).unwrap(), 1);
    assert_eq!(bob.refresh_call_media(&mut b, now).unwrap(), 1);
    // Bob mutes: readiness changes, the roster does not, and nobody loses a receiver while it propagates.
    bob.set_call_tracks(&mut b, Tracks { audio: false, camera: false, screen: false }, now).unwrap();
    assert_eq!(bob.refresh_call_media(&mut b, now).map_err(|e| format!("{e:?}")), Ok(1));
    for _ in 0..4 {
        crate::calls::tests::round_trip(&mut alice, &mut bob, now);
        assert_eq!(alice.refresh_call_media(&mut a, now).map_err(|e| format!("{e:?}")), Ok(1));
        assert_eq!(bob.refresh_call_media(&mut b, now).map_err(|e| format!("{e:?}")), Ok(1));
    }
}
