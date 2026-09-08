use super::*;
use crate::{
    claims::tests::pair,
    incoming::tests::{next, start, trust},
};
use sigil_protocol::mailbox::Delivery;

fn delivery(sender: &ClientStore, sequence: i64, message: Id, packet: &[u8], now: u64) -> Delivery {
    Delivery {
        origin: None,
        sequence,
        sender_device: sender.connection_session().unwrap().unwrap().device_id,
        message_id: transport::hex(&message),
        payload: transport::hex(packet),
        expires_at: now + 60,
    }
}
fn reopen(path: &Path) -> ClientStore {
    ClientStore::open(
        path,
        StorageKey::new(sigil_crypto::Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap()
}
#[test]
fn authenticated_traffic_selects_sessions_without_moving_retries_or_replay_selection() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (a, b) = trust(&mut alice, &mut bob);
    assert_eq!(alice.active_session(b).unwrap(), None);
    start(&mut alice, b, now);
    let first = bob.accept_delivery(&next(&bob)).unwrap().session;
    assert_eq!(alice.active_session(b).unwrap(), Some([3; 32]));
    assert_eq!(bob.active_session(a).unwrap(), Some(first));
    bob.prepare_prekey_publication([20; 32], true, 3600)
        .unwrap();
    bob.publish_prekey_online([20; 32]).unwrap();
    alice.prepare_peer_claim([21; 32], b).unwrap();
    alice.claim_prekey_online([21; 32], now).unwrap();
    let packet = alice
        .start_claimed_text([21; 32], [22; 32], [23; 32], "second initial", now, now)
        .unwrap();
    let initial = delivery(&alice, 500, [23; 32], &packet, now);
    let second = bob.accept_delivery(&initial).unwrap().session;
    let confirmation = bob
        .send_text(second, [90; 32], "confirm second", now, now)
        .unwrap();
    alice
        .accept_delivery(&delivery(&bob, 490, [90; 32], &confirmation, now))
        .unwrap();
    let old_wins = load(&alice.db, &alice.key, &[3; 32])
        .unwrap()
        .1
        .convergence_id()
        < load(&alice.db, &alice.key, &[22; 32])
            .unwrap()
            .1
            .convergence_id();
    let expected_alice = if old_wins { [3; 32] } else { [22; 32] };
    let expected_bob = if old_wins { first } else { second };
    assert_eq!(alice.active_session(b).unwrap(), Some([22; 32]));
    let frozen = bob
        .send_peer_text(a, [24; 32], "queued reply", now, now)
        .unwrap();
    assert_eq!(frozen.0, second);
    let packet = bob
        .send_text(first, [25; 32], "old session reply", now, now)
        .unwrap();
    let incoming = delivery(&bob, 501, [25; 32], &packet, now);
    let mut damaged = packet.clone();
    damaged[10] ^= 1;
    let bad = delivery(&bob, 501, [25; 32], &damaged, now);
    assert!(alice.accept_delivery(&bad).is_err());
    assert_eq!(alice.active_session(b).unwrap(), Some([22; 32]));
    if old_wins {
        alice.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON active_sessions BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
        assert!(alice.accept_delivery(&incoming).is_err());
        assert_eq!(alice.active_session(b).unwrap(), Some([22; 32]));
        alice.db.execute_batch("DROP TRIGGER fail;").unwrap();
    }
    alice.accept_delivery(&incoming).unwrap();
    assert_eq!(alice.active_session(b).unwrap(), Some(expected_alice));
    let follow = alice
        .send_peer_text(b, [26; 32], "selected reply", now, now)
        .unwrap();
    assert_eq!(follow.0, expected_alice);
    bob.accept_delivery(&delivery(&alice, 502, [26; 32], &follow.1, now))
        .unwrap();
    assert_eq!(bob.active_session(a).unwrap(), Some(expected_bob));
    bob.accept_delivery(&initial).unwrap();
    let mut replay = initial;
    replay.sequence = 503;
    bob.accept_delivery(&replay).unwrap();
    assert_eq!(bob.active_session(a).unwrap(), Some(expected_bob));
    drop(bob);
    let mut bob = reopen(&dir.path().join("bob.db"));
    assert_eq!(bob.active_session(a).unwrap(), Some(expected_bob));
    assert_eq!(
        bob.send_peer_text(a, [24; 32], "queued reply", now, now)
            .unwrap(),
        frozen
    );
    assert!(matches!(
        bob.send_peer_text(a, [24; 32], "changed", now, now),
        Err(Error::Conflict)
    ));
    bob.block_peer(a, true).unwrap();
    assert!(bob.active_session(a).is_err());
    assert!(bob
        .send_peer_text(a, [24; 32], "queued reply", now, now)
        .is_err());
}

#[test]
fn migration_does_not_infer_selection_and_retirement_clears_it_atomically() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let incoming = bob.accept_delivery(&next(&bob)).unwrap();
    let packet = bob
        .send_text(incoming.session, [30; 32], "migration reply", now, now)
        .unwrap();
    let reply = delivery(&bob, 510, [30; 32], &packet, now);
    drop(alice);
    let path = dir.path().join("alice.db");
    crate::test_schema::rewind(&Connection::open(&path).unwrap(), 21);
    let mut alice = reopen(&path);
    assert_eq!(alice.active_session(b).unwrap(), None);
    assert!(matches!(
        alice.send_peer_text(b, [31; 32], "no selection", now, now),
        Err(Error::Unprepared)
    ));
    alice.accept_delivery(&reply).unwrap();
    assert_eq!(alice.active_session(b).unwrap(), Some([3; 32]));
    let sealed: Vec<u8> = alice
        .db
        .query_row("SELECT state FROM active_sessions", [], |r| r.get(0))
        .unwrap();
    alice
        .db
        .execute("UPDATE active_sessions SET state=zeroblob(100)", [])
        .unwrap();
    assert!(alice.active_session(b).is_err());
    assert!(alice.retire_session([3; 32]).is_err());
    alice
        .db
        .execute("UPDATE active_sessions SET state=?1", [sealed])
        .unwrap();
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE DELETE ON active_sessions BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(alice.retire_session([3; 32]).is_err());
    assert_eq!(alice.active_session(b).unwrap(), Some([3; 32]));
    alice.db.execute_batch("DROP TRIGGER fail;").unwrap();
    alice.block_peer(b, true).unwrap();
    alice.retire_session([3; 32]).unwrap();
    alice.block_peer(b, false).unwrap();
    assert_eq!(alice.active_session(b).unwrap(), None);
    assert!(!alice.outgoing_message([3; 32], [4; 32]).unwrap().is_empty());
}

#[test]
fn retirement_policy_protects_active_and_queued_sessions_and_commits_grace_across_restart() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let first = bob.accept_delivery(&next(&bob)).unwrap().session;
    bob.acknowledge_incoming_online().unwrap();
    // Establish another authenticated session, making the first inactive.
    bob.prepare_prekey_publication([20; 32], true, 3600)
        .unwrap();
    bob.publish_prekey_online([20; 32]).unwrap();
    alice.prepare_peer_claim([21; 32], b).unwrap();
    alice.claim_prekey_online([21; 32], now).unwrap();
    let packet = alice
        .start_claimed_text([21; 32], [22; 32], [23; 32], "second", now, now)
        .unwrap();
    let second = bob
        .accept_delivery(&delivery(&alice, 700, [23; 32], &packet, now))
        .unwrap()
        .session;
    assert_ne!(first, second);
    let pending = bob.send_text(first, [24; 32], "pending", now, now).unwrap();
    assert!(!pending.is_empty());
    let grace = crate::INACTIVE_GRACE_SECONDS;
    assert_eq!(bob.maintain_sessions(now).unwrap().retired, 0);
    assert_eq!(bob.maintain_sessions(now + grace).unwrap().retired, 0);
    bob.expire_delivery(first, [24; 32], now + grace).unwrap();
    assert_eq!(bob.maintain_sessions(now + grace).unwrap().retired, 0);
    drop(bob);
    let mut bob = reopen(&dir.path().join("bob.db"));
    assert!(bob.maintain_sessions(now).is_err());
    let server = Connection::open(dir.path().join("server.db")).unwrap();
    let account = bob.connection_session().unwrap().unwrap().account_id;
    server
        .execute("UPDATE accounts SET disabled=1 WHERE id=?1", [&account])
        .unwrap();
    assert!(bob.maintain_sessions_online(now + 2 * grace).is_err());
    assert!(!bob
        .db
        .query_row(
            "SELECT retired FROM sessions WHERE id=?1",
            [first.as_slice()],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
    server
        .execute("UPDATE accounts SET disabled=0 WHERE id=?1", [&account])
        .unwrap();
    bob.db.execute_batch("CREATE TRIGGER fail_maintenance BEFORE UPDATE ON session_maintenance BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(bob.maintain_sessions_online(now + 2 * grace).is_err());
    bob.db
        .execute_batch("DROP TRIGGER fail_maintenance;")
        .unwrap();
    assert_eq!(bob.active_session(a).unwrap(), Some(second));
    assert!(!bob
        .db
        .query_row(
            "SELECT retired FROM sessions WHERE id=?1",
            [first.as_slice()],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
    assert_eq!(bob.maintain_sessions(now + 2 * grace).unwrap().retired, 0);
    assert_eq!(
        bob.maintain_sessions_online(now + 2 * grace)
            .unwrap()
            .retired,
        1
    );
    assert!(bob
        .db
        .query_row(
            "SELECT retired FROM sessions WHERE id=?1",
            [first.as_slice()],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
    assert_eq!(bob.active_session(a).unwrap(), Some(second));
    assert_eq!(bob.maintain_sessions(now + 3 * grace).unwrap().retired, 0);
}

#[test]
fn simultaneous_opposite_initiations_converge_without_alternating_sessions() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (a, b) = trust(&mut alice, &mut bob);
    alice
        .prepare_prekey_publication([60; 32], true, 3600)
        .unwrap();
    alice.publish_prekey_online([60; 32]).unwrap();
    start(&mut alice, b, now);
    bob.prepare_peer_claim([61; 32], a).unwrap();
    bob.claim_prekey_online([61; 32], now).unwrap();
    let packet = bob
        .start_claimed_text([61; 32], [62; 32], [63; 32], "crossed initial", now, now)
        .unwrap();
    let at_alice = alice
        .accept_delivery(&delivery(&bob, 900, [63; 32], &packet, now))
        .unwrap()
        .session;
    let at_bob = bob.accept_delivery(&next(&bob)).unwrap().session;
    let expected = if load(&alice.db, &alice.key, &[3; 32])
        .unwrap()
        .1
        .convergence_id()
        < load(&alice.db, &alice.key, &at_alice)
            .unwrap()
            .1
            .convergence_id()
    {
        ([3; 32], at_bob)
    } else {
        (at_alice, [62; 32])
    };
    assert_eq!(
        (
            alice.active_session(b).unwrap().unwrap(),
            bob.active_session(a).unwrap().unwrap()
        ),
        (at_alice, at_bob)
    );
    drop(alice);
    drop(bob);
    let mut alice = reopen(&dir.path().join("alice.db"));
    let mut bob = reopen(&dir.path().join("bob.db"));
    for n in 70..74 {
        if n == 72 {
            // Model a late initial promoting the other, now confirmed session.
            let other = if expected.0 == at_alice {
                [3; 32]
            } else {
                at_alice
            };
            assert!(alice.session_peer_confirmed(other).unwrap());
            let tx = alice
                .db
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
            activate(&tx, &alice.key, &b, &other).unwrap();
            tx.commit().unwrap();
        }
        // Both prepare before receiving: lockstep traffic used to swap selections.
        let ar = alice.send_peer_text(b, [n; 32], "alice", now, now).unwrap();
        let br = bob
            .send_peer_text(a, [n + 10; 32], "bob", now, now)
            .unwrap();
        if n > 70 && n != 72 {
            assert_eq!((ar.0, br.0), expected);
        }
        alice
            .accept_delivery(&delivery(
                &bob,
                1000 + i64::from(n),
                [n + 10; 32],
                &br.1,
                now,
            ))
            .unwrap();
        bob.accept_delivery(&delivery(&alice, 1000 + i64::from(n), [n; 32], &ar.1, now))
            .unwrap();
        assert_eq!(
            (
                alice.active_session(b).unwrap().unwrap(),
                bob.active_session(a).unwrap().unwrap()
            ),
            expected
        );
    }
}

#[test]
fn retirement_cursor_visits_zero_id_and_resumes_bounded_batches() {
    let (dir, _fixture, mut alice, _bob, now) = pair();
    for n in 0..17 {
        let dh = sigil_crypto::DhKey::generate().unwrap();
        let state = Session::initiator(
            sigil_crypto::Secret32::from_bytes([7; 32]),
            dh.public_key(),
            [n; 32],
        )
        .unwrap();
        alice.insert_session([n; 32], state).unwrap();
    }
    assert_eq!(
        alice.maintain_sessions(now).unwrap(),
        crate::SessionMaintenance {
            scanned: 16,
            retired: 0,
            expired: 0,
            prekeys_retired: 0
        }
    );
    assert!(alice
        .db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM session_activity WHERE id=?1)",
            [[0_u8; 32].as_slice()],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
    drop(alice);
    let mut alice = reopen(&dir.path().join("alice.db"));
    assert_eq!(alice.maintain_sessions(now + 1).unwrap().scanned, 1);
    assert_eq!(
        alice
            .maintain_sessions(now + crate::INACTIVE_GRACE_SECONDS)
            .unwrap(),
        crate::SessionMaintenance {
            scanned: 16,
            retired: 0,
            expired: 0,
            prekeys_retired: 0
        }
    );
    alice
        .db
        .execute("UPDATE session_maintenance SET state=zeroblob(77)", [])
        .unwrap();
    assert!(alice
        .maintain_sessions(now + crate::INACTIVE_GRACE_SECONDS)
        .is_err());
}

#[test]
fn unconfirmed_selection_yields_to_a_usable_reply_and_repairs_expired_selection() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let first_packet = next(&bob);
    bob.prepare_prekey_publication([20; 32], true, 3600)
        .unwrap();
    bob.publish_prekey_online([20; 32]).unwrap();
    alice.prepare_peer_claim([21; 32], b).unwrap();
    alice.claim_prekey_online([21; 32], now).unwrap();
    let packet = alice
        .start_claimed_text([21; 32], [22; 32], [23; 32], "other initial", now, now)
        .unwrap();
    let second_packet = delivery(&alice, 500, [23; 32], &packet, now);
    let rank = |id| load(&alice.db, &alice.key, &id).unwrap().1.convergence_id();
    let (low, high, delivered) = if rank([3; 32]) < rank([22; 32]) {
        ([3; 32], [22; 32], second_packet)
    } else {
        ([22; 32], [3; 32], first_packet)
    };
    // Model the lower-hash initiation being newest, without random ordering in the test.
    let tx = alice
        .db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    activate(&tx, &alice.key, &b, &low).unwrap();
    tx.commit().unwrap();
    let received = bob.accept_delivery(&delivered).unwrap();
    let reply = bob
        .send_peer_text(a, [30; 32], "usable reply", now, now)
        .unwrap();
    assert_eq!(reply.0, received.session);
    alice.db.execute_batch("CREATE TRIGGER fail_promotion BEFORE UPDATE ON active_sessions BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(alice
        .accept_delivery(&delivery(&bob, 501, [30; 32], &reply.1, now))
        .is_err());
    assert!(!alice.session_peer_confirmed(high).unwrap());
    assert_eq!(alice.active_session(b).unwrap(), Some(low));
    alice
        .db
        .execute_batch("DROP TRIGGER fail_promotion;")
        .unwrap();
    alice
        .accept_delivery(&delivery(&bob, 501, [30; 32], &reply.1, now))
        .unwrap();
    assert!(alice.session_peer_confirmed(high).unwrap());
    assert!(!alice.session_peer_confirmed(low).unwrap());
    assert_eq!(alice.active_session(b).unwrap(), Some(high));
    // Model an old-version selection surviving restart. Exact pending retries
    // retain their original sessions, even while new sends repair selection.
    let pending = alice
        .send_text(low, [40; 32], "pending initial", now, now)
        .unwrap();
    let tx = alice
        .db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    activate(&tx, &alice.key, &b, &low).unwrap();
    tx.commit().unwrap();
    drop(alice);
    let mut alice = reopen(&dir.path().join("alice.db"));
    assert_eq!(
        alice
            .send_peer_text(b, [40; 32], "pending initial", now, now)
            .unwrap(),
        (low, pending.clone())
    );
    let later = now + crate::INACTIVE_GRACE_SECONDS + 1;
    alice
        .send_text(high, [31; 32], "usable alternative", later, later)
        .unwrap();
    alice.block_peer(b, true).unwrap();
    assert!(matches!(
        alice.send_peer_text(b, [32; 32], "ordinary send", later, later),
        Err(Error::Unprepared)
    ));
    alice.block_peer(b, false).unwrap();
    let saved: Vec<u8> = alice
        .db
        .query_row(
            "SELECT state FROM sessions WHERE id=?1",
            [high.as_slice()],
            |r| r.get(0),
        )
        .unwrap();
    alice
        .db
        .execute(
            "UPDATE sessions SET state=zeroblob(length(state)) WHERE id=?1",
            [high.as_slice()],
        )
        .unwrap();
    assert!(alice
        .send_peer_text(b, [32; 32], "ordinary send", later, later)
        .is_err());
    assert_eq!(alice.active_session(b).unwrap(), Some(low));
    alice
        .db
        .execute(
            "UPDATE sessions SET state=?1 WHERE id=?2",
            (&saved, high.as_slice()),
        )
        .unwrap();
    alice.db.execute_batch("CREATE TRIGGER fail_fallback BEFORE INSERT ON outbox BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(alice
        .send_peer_text(b, [32; 32], "ordinary send", later, later)
        .is_err());
    assert_eq!(alice.active_session(b).unwrap(), Some(low));
    alice
        .db
        .execute_batch("DROP TRIGGER fail_fallback;")
        .unwrap();
    let result = alice.send_peer_text(b, [32; 32], "ordinary send", later, later);
    assert!(
        result.is_ok(),
        "ordinary send must recover a usable selection; got {result:?}"
    );
    let sent = result.unwrap();
    assert_eq!(sent.0, high);
    assert_eq!(
        alice
            .send_peer_text(b, [32; 32], "ordinary send", later, later)
            .unwrap(),
        sent
    );
    assert!(
        !bob.accept_delivery(&delivery(&alice, 502, [32; 32], &sent.1, later))
            .unwrap()
            .duplicate
    );
    assert_eq!(alice.active_session(b).unwrap(), Some(high));
    assert!(matches!(
        alice.send_peer_text(b, [40; 32], "pending initial", now, later),
        Err(Error::Expired)
    ));
    assert_eq!(alice.active_session(b).unwrap(), Some(high));
    let sealed: Vec<u8> = alice
        .db
        .query_row(
            "SELECT packet FROM outbox WHERE session=?1 AND id=?2",
            (low.as_slice(), [40_u8; 32].as_slice()),
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        crate::open_packet(&alice.key, &sealed, &low, &[40; 32]).unwrap(),
        pending
    );
}

#[test]
fn expired_selection_without_a_confirmed_alternative_preserves_state() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let revision = load(&alice.db, &alice.key, &[3; 32]).unwrap().0;
    let later = now + crate::INACTIVE_GRACE_SECONDS + 1;
    assert!(matches!(
        alice.send_peer_text(b, [50; 32], "no alternate", later, later),
        Err(Error::Expired)
    ));
    assert_eq!(alice.active_session(b).unwrap(), Some([3; 32]));
    assert_eq!(load(&alice.db, &alice.key, &[3; 32]).unwrap().0, revision);
    assert_eq!(
        alice
            .db
            .query_row("SELECT count(*) FROM outbox", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        alice
            .db
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn retirement_waits_for_unread_unexpired_packets_to_be_processed() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let first = bob.accept_delivery(&next(&bob)).unwrap().session;
    bob.acknowledge_incoming_online().unwrap();
    bob.send_text(first, [30; 32], "confirm first", now, now)
        .unwrap();
    bob.send_pending_online(first, now).unwrap();
    alice.accept_delivery(&next(&alice)).unwrap();
    bob.prepare_prekey_publication([20; 32], true, 3600)
        .unwrap();
    bob.publish_prekey_online([20; 32]).unwrap();
    alice.prepare_peer_claim([21; 32], b).unwrap();
    alice.claim_prekey_online([21; 32], now).unwrap();
    alice
        .start_claimed_text([21; 32], [22; 32], [23; 32], "new initial", now, now)
        .unwrap();
    alice.send_pending_online([22; 32], now).unwrap();
    bob.accept_delivery(&next(&bob)).unwrap();
    bob.acknowledge_incoming_online().unwrap();
    assert_eq!(bob.maintain_sessions_online(now).unwrap().retired, 0);
    let later = now + crate::INACTIVE_GRACE_SECONDS - 1;
    let delayed = alice
        .send_text(
            [3; 32],
            [31; 32],
            "still valid on old session",
            later,
            later,
        )
        .unwrap();
    let unread = delivery(&alice, 501, [31; 32], &delayed, later);
    assert!(unread.expires_at > later + 1);
    let mut server = sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap();
    server
        .submit_message(
            &crate::connection::tests::credential(&alice),
            sigil_protocol::mailbox::Submit {
                recipient_device: bob.connection_session().unwrap().unwrap().device_id,
                message_id: unread.message_id,
                payload: unread.payload,
                expires_at: unread.expires_at,
            },
            later,
        )
        .unwrap();
    assert_eq!(bob.maintain_sessions(later + 1).unwrap().retired, 0);
    assert_eq!(bob.maintain_sessions_online(later + 1).unwrap().retired, 0);
    drop(bob);
    let mut bob = reopen(&dir.path().join("bob.db"));
    assert_eq!(bob.maintain_sessions_online(later + 1).unwrap().retired, 0);
    let incoming = bob.receive_mailbox_online(later + 1).unwrap();
    assert_eq!(incoming.len(), 1);
    assert!(incoming[0].result.is_ok());
    bob.acknowledge_incoming_online().unwrap();
    // Receiving the packet changed the revision, starting a new grace interval.
    assert_eq!(bob.maintain_sessions_online(later + 1).unwrap().retired, 0);
}
