use super::*;
use crate::claims::tests::pair;
use sigil_crypto::Secret32;
fn open(path: &Path) -> ClientStore {
    ClientStore::open(
        path,
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap()
}
pub(crate) fn trust(alice: &mut ClientStore, bob: &mut ClientStore) -> (Id, Id) {
    let a = alice.own_device_binding().unwrap();
    let b = bob.own_device_binding().unwrap();
    let a_peer = bob.observe_peer_binding(&a).unwrap();
    let b_peer = alice.observe_peer_binding(&b).unwrap();
    alice
        .confirm_peer(b_peer.id, device_fingerprint(&b).unwrap())
        .unwrap();
    bob.confirm_peer(a_peer.id, device_fingerprint(&a).unwrap())
        .unwrap();
    (a_peer.id, b_peer.id)
}
pub(crate) fn start(alice: &mut ClientStore, peer: Id, now: u64) {
    alice.prepare_peer_claim([1; 32], peer).unwrap();
    alice.claim_prekey_online([1; 32], now).unwrap();
    alice
        .start_claimed_text([1; 32], [3; 32], [4; 32], "synthetic initial", now, now)
        .unwrap();
    alice.send_pending_online([3; 32], now).unwrap();
}
pub(crate) fn next(client: &ClientStore) -> Delivery {
    client
        .connected_client()
        .unwrap()
        .mailbox()
        .unwrap()
        .remove(0)
}
fn count(client: &ClientStore, table: &str) -> i64 {
    client
        .db
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}
#[test]
fn real_https_routes_initial_and_reply_and_acknowledgements_survive_local_failure() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let delivery = next(&bob);
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON incoming BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(bob.accept_delivery(&delivery).is_err());
    assert_eq!(count(&bob, "sessions"), 0);
    assert_eq!(count(&bob, "inbox"), 0);
    assert_eq!(
        bob.db
            .query_row("SELECT count(state) FROM prekeys", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(bob.acknowledge_incoming_online().unwrap(), 0);
    assert_eq!(next(&bob).sequence, delivery.sequence);
    bob.db.execute_batch("DROP TRIGGER fail;").unwrap();
    let incoming = bob
        .receive_mailbox_online(now)
        .unwrap()
        .remove(0)
        .result
        .unwrap();
    let MailboxEvent::Text(incoming) = incoming else {
        panic!("expected text")
    };
    assert_eq!(incoming.peer, a);
    assert_ne!(incoming.session, [3; 32]);
    assert_eq!(incoming.text().unwrap().body, "synthetic initial");
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    assert_eq!(
        bob.accept_delivery(&delivery).unwrap().session,
        incoming.session
    );
    assert_eq!(count(&bob, "inbox"), 1);
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON incoming BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(bob.acknowledge_incoming_online().is_err());
    assert!(bob
        .connected_client()
        .unwrap()
        .mailbox()
        .unwrap()
        .is_empty());
    bob.db.execute_batch("DROP TRIGGER fail;").unwrap();
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    assert_eq!(bob.acknowledge_incoming_online().unwrap(), 1);
    assert_eq!(bob.acknowledge_incoming_online().unwrap(), 0);
    bob.send_text(incoming.session, [5; 32], "immediate reply", now, now)
        .unwrap();
    bob.send_pending_online(incoming.session, now).unwrap();
    let reply = next(&alice);
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON incoming BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    let before: Vec<u8> = alice
        .db
        .query_row("SELECT state FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert!(alice.accept_delivery(&reply).is_err());
    assert_eq!(
        alice
            .db
            .query_row("SELECT state FROM sessions", [], |r| r.get::<_, Vec<u8>>(0))
            .unwrap(),
        before
    );
    assert_eq!(count(&alice, "inbox"), 0);
    alice.db.execute_batch("DROP TRIGGER fail;").unwrap();
    let accepted = alice.accept_delivery(&reply).unwrap();
    assert_eq!(accepted.session, [3; 32]);
    assert_eq!(accepted.peer, b);
    assert_eq!(accepted.text().unwrap().body, "immediate reply");
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    assert_eq!(
        alice.accept_delivery(&reply).unwrap().text().unwrap().body,
        "immediate reply"
    );
    assert_eq!(alice.acknowledge_incoming_online().unwrap(), 1);
    assert!(alice
        .connected_client()
        .unwrap()
        .mailbox()
        .unwrap()
        .is_empty());
}

#[test]
fn unverified_blocked_tampered_and_substituted_deliveries_never_acknowledge() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let mut delivery = next(&bob);
    bob.block_peer(a, true).unwrap();
    assert!(bob.accept_delivery(&delivery).is_err());
    assert_eq!(bob.acknowledge_incoming_online().unwrap(), 0);
    bob.block_peer(a, false).unwrap();
    let original = delivery.payload.clone();
    let end = delivery.payload.len();
    delivery.payload.replace_range(
        end - 2..,
        if &original[end - 2..] == "00" {
            "01"
        } else {
            "00"
        },
    );
    assert!(bob.accept_delivery(&delivery).is_err());
    assert_eq!(count(&bob, "incoming"), 0);
    delivery.payload = original;
    let sender = delivery.sender_device.clone();
    delivery.sender_device = transport::hex(&[99; 32]);
    assert!(bob.accept_delivery(&delivery).is_err());
    delivery.sender_device = sender;
    bob.accept_delivery(&delivery).unwrap();
    delivery.expires_at += 1;
    assert!(matches!(
        bob.accept_delivery(&delivery),
        Err(Error::Conflict)
    ));
    delivery.expires_at -= 1;
    delivery.message_id = transport::hex(&[98; 32]);
    assert!(matches!(
        bob.accept_delivery(&delivery),
        Err(Error::Conflict)
    ));
    // An unauthenticated acknowledgement index cannot authorize deletion.
    bob.db
        .execute("UPDATE incoming SET state=zeroblob(173)", [])
        .unwrap();
    assert!(bob.acknowledge_incoming_online().is_err());
    assert_eq!(next(&bob).sequence, delivery.sequence);
}

#[test]
fn migration_adds_no_acknowledgements_and_forged_rows_cannot_delete_mail() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let delivery = next(&bob);
    crate::test_schema::rewind(&bob.db, 14);
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    assert_eq!(count(&bob, "incoming"), 0);
    bob.db
        .execute(
            "INSERT INTO incoming VALUES(?1,0,zeroblob(173))",
            [delivery.sequence],
        )
        .unwrap();
    assert!(bob.acknowledge_incoming_online().is_err());
    assert_eq!(next(&bob).sequence, delivery.sequence);
    bob.db.execute("DELETE FROM incoming", []).unwrap();
    bob.accept_delivery(&delivery).unwrap();
    bob.db
        .execute("UPDATE incoming SET sequence=sequence+1", [])
        .unwrap();
    assert!(bob.acknowledge_incoming_online().is_err());
    assert_eq!(next(&bob).sequence, delivery.sequence);
}

#[test]
fn routing_handles_eight_sessions_without_mutating_wrong_candidates_and_refuses_a_ninth() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    let mut replies = Vec::new();
    for n in 1..=8u8 {
        if n > 1 {
            bob.prepare_prekey_publication([n + 20; 32], true, 3600)
                .unwrap();
            bob.publish_prekey_online([n + 20; 32]).unwrap();
        }
        alice.prepare_peer_claim([n; 32], b).unwrap();
        alice.claim_prekey_online([n; 32], now).unwrap();
        alice
            .start_claimed_text([n; 32], [n; 32], [n; 32], &n.to_string(), now, now)
            .unwrap();
        alice.send_pending_online([n; 32], now).unwrap();
        let received = bob.accept_delivery(&next(&bob)).unwrap();
        bob.acknowledge_incoming_online().unwrap();
        let reply = bob
            .send_text(
                received.session,
                [n + 40; 32],
                &(n + 40).to_string(),
                now,
                now,
            )
            .unwrap();
        replies.push(Delivery {
            origin: None,
            sequence: 100 + i64::from(n),
            sender_device: bob.connection_session().unwrap().unwrap().device_id,
            message_id: transport::hex(&[n + 40; 32]),
            payload: transport::hex(&reply),
            expires_at: now + 60,
        });
    }
    for (index, delivery) in replies.iter().enumerate().rev() {
        let states: Vec<(Vec<u8>, Vec<u8>)> = alice
            .db
            .prepare("SELECT id,state FROM sessions ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let received = alice.accept_delivery(delivery).unwrap();
        assert_eq!(received.session, [index as u8 + 1; 32]);
        assert_eq!(received.text().unwrap().body, (index + 41).to_string());
        for (id, before) in states {
            if id != received.session {
                assert_eq!(
                    alice
                        .db
                        .query_row("SELECT state FROM sessions WHERE id=?1", [id], |r| r
                            .get::<_, Vec<u8>>(0))
                        .unwrap(),
                    before
                );
            }
        }
    }
    bob.prepare_prekey_publication([29; 32], true, 3600)
        .unwrap();
    bob.publish_prekey_online([29; 32]).unwrap();
    alice.prepare_peer_claim([9; 32], b).unwrap();
    alice.claim_prekey_online([9; 32], now).unwrap();
    assert!(matches!(
        alice.start_claimed_initial([9; 32], [9; 32], [9; 32], b"ninth", now),
        Err(Error::Limit)
    ));
    assert_eq!(count(&alice, "sessions"), 8);
    assert_eq!(count(&alice, "outbox"), 8);
    assert_eq!(count(&alice, "initiations"), 8);
    // Corruption of even a nonmatching candidate is an error, never hidden by
    // trying another session. Accepted historical results remain readable.
    let state: Vec<u8> = alice
        .db
        .query_row(
            "SELECT state FROM sessions WHERE id=?1",
            [[1; 32].as_slice()],
            |r| r.get(0),
        )
        .unwrap();
    alice
        .db
        .execute(
            "UPDATE sessions SET state=zeroblob(length(state)) WHERE id=?1",
            [[1; 32].as_slice()],
        )
        .unwrap();
    let mut forged = replies.pop().unwrap();
    forged.sequence += 1000;
    assert!(alice.accept_delivery(&forged).is_err());
    alice
        .db
        .execute(
            "UPDATE sessions SET state=?1 WHERE id=?2",
            (state, [1; 32].as_slice()),
        )
        .unwrap();
}

#[test]
fn invalid_head_messages_do_not_starve_later_deliveries_and_scan_restarts_are_safe() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    let recipient = bob.connection_session().unwrap().unwrap().device_id;
    for n in 40..56u8 {
        alice
            .connected_client()
            .unwrap()
            .submit(&sigil_protocol::mailbox::Submit {
                recipient_device: recipient.clone(),
                message_id: transport::hex(&[n; 32]),
                payload: transport::hex(&[0; 32]),
                expires_at: now + 60,
            })
            .unwrap();
    }
    start(&mut alice, b, now);
    let first = bob.receive_mailbox_online(now).unwrap();
    assert_eq!(first.len(), 16);
    assert!(first.iter().all(|v| v.result.is_err()));
    assert_eq!(count(&bob, "incoming"), 0);
    assert_eq!(bob.acknowledge_incoming_online().unwrap(), 0);
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON incoming_cursor BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(bob.receive_mailbox_online(now).is_err());
    assert_eq!(count(&bob, "incoming"), 1);
    assert_eq!(count(&bob, "inbox"), 1);
    bob.db.execute_batch("DROP TRIGGER fail;").unwrap();
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    let later = bob.receive_mailbox_online(now).unwrap();
    assert_eq!(later.len(), 1);
    assert!(later[0].result.is_ok());
    assert!(later[0].sequence > first[15].sequence);
    assert_eq!(count(&bob, "inbox"), 1);
    assert_eq!(bob.acknowledge_incoming_online().unwrap(), 1);
    assert!(bob.receive_mailbox_online(now).unwrap().is_empty()); // wraps to zero
    let revisited = bob.receive_mailbox_online(now).unwrap();
    assert_eq!(revisited.len(), 16);
    assert_eq!(revisited[0].sequence, first[0].sequence);
    assert!(revisited.iter().all(|v| v.result.is_err()));
    assert_eq!(bob.connected_client().unwrap().mailbox().unwrap().len(), 16);
    assert!(bob.connected_client().unwrap().mailbox_after(-1).is_err());
    crate::test_schema::rewind(&bob.db, 15);
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    assert_eq!(count(&bob, "incoming"), 1);
    assert_eq!(
        bob.receive_mailbox_online(now).unwrap()[0].sequence,
        first[0].sequence
    );
    bob.db
        .execute("UPDATE incoming_cursor SET state=zeroblob(44)", [])
        .unwrap();
    assert!(bob.receive_mailbox_online(now).is_err());
    assert_eq!(bob.acknowledge_incoming_online().unwrap(), 0);
}

#[test]
fn lost_first_packet_recovers_over_https_and_only_peer_traffic_confirms() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let lost = next(&bob);
    assert!(alice.delivery_receipt([3; 32], [4; 32]).unwrap().is_some());
    assert!(!alice.session_peer_confirmed([3; 32]).unwrap());
    // Model loss after server acceptance, before the receiving client sees it.
    bob.connected_client()
        .unwrap()
        .acknowledge_delivery(lost.sequence)
        .unwrap();
    let second = alice
        .send_peer_text(b, [50; 32], "survives first packet loss", now, now)
        .unwrap();
    assert!(sigil_protocol::initial::decode(&second.1).is_ok());
    alice.send_pending_online(second.0, now).unwrap();
    let received = bob.accept_delivery(&next(&bob)).unwrap();
    assert_eq!(received.text().unwrap().body, "survives first packet loss");
    assert!(bob.session_peer_confirmed(received.session).unwrap());
    assert_eq!(count(&bob, "sessions"), 1);
    // The delayed first packet resolves to the same session and skipped key.
    assert_eq!(
        bob.accept_delivery(&lost).unwrap().session,
        received.session
    );
    assert_eq!(count(&bob, "sessions"), 1);
    let frozen = alice
        .send_peer_text(b, [51; 32], "queued before confirmation", now, now)
        .unwrap();
    bob.send_peer_text(a, [52; 32], "peer reply", now, now)
        .unwrap();
    bob.send_pending_online(received.session, now).unwrap();
    let reply = next(&alice);
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    assert!(!alice.session_peer_confirmed([3; 32]).unwrap());
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON incoming BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(alice.accept_delivery(&reply).is_err());
    assert!(!alice.session_peer_confirmed([3; 32]).unwrap());
    alice.db.execute_batch("DROP TRIGGER fail;").unwrap();
    alice.accept_delivery(&reply).unwrap();
    assert!(alice.session_peer_confirmed([3; 32]).unwrap());
    assert_eq!(
        alice
            .send_peer_text(b, [51; 32], "queued before confirmation", now, now)
            .unwrap(),
        frozen
    );
    let regular = alice
        .send_peer_text(b, [53; 32], "after confirmation", now, now)
        .unwrap();
    assert!(Packet::from_bytes(&regular.1).is_ok());
    let sender = alice.connection_session().unwrap().unwrap().device_id;
    for (sequence, message, packet, body) in [
        (600, [53; 32], regular.1, "after confirmation"),
        (601, [51; 32], frozen.1, "queued before confirmation"),
    ] {
        let delivery = Delivery {
            origin: None,
            sequence,
            sender_device: sender.clone(),
            message_id: transport::hex(&message),
            payload: transport::hex(&packet),
            expires_at: now + 60,
        };
        assert_eq!(
            bob.accept_delivery(&delivery).unwrap().text().unwrap().body,
            body
        );
    }
    drop(alice);
    assert!(open(&dir.path().join("alice.db"))
        .session_peer_confirmed([3; 32])
        .unwrap());
}

#[test]
fn retained_initial_headers_fail_closed_and_migration_cannot_invent_them() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let initial = next(&bob);
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON initial_headers BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(bob.accept_delivery(&initial).is_err());
    assert_eq!(count(&bob, "sessions"), 0);
    bob.db.execute_batch("DROP TRIGGER fail;").unwrap();
    let received = bob.accept_delivery(&initial).unwrap();
    let next = alice
        .send_peer_text(b, [60; 32], "header authenticated", now, now)
        .unwrap();
    let header: Vec<u8> = bob
        .db
        .query_row("SELECT data FROM initial_headers", [], |r| r.get(0))
        .unwrap();
    bob.db
        .execute("UPDATE initial_headers SET data=zeroblob(length(data))", [])
        .unwrap();
    assert!(bob.receive(received.session, [60; 32], &next.1).is_err());
    bob.db
        .execute("UPDATE initial_headers SET data=?1", [header])
        .unwrap();
    bob.receive(received.session, [60; 32], &next.1).unwrap();
    let path = dir.path().join("alice.db");
    drop(alice);
    crate::test_schema::rewind(&Connection::open(&path).unwrap(), 22);
    let mut alice = open(&path);
    assert!(!alice.session_peer_confirmed([3; 32]).unwrap());
    assert!(matches!(
        alice.send_peer_text(b, [61; 32], "missing header", now, now),
        Err(Error::UnsupportedSession)
    ));
    assert!(matches!(
        alice.outgoing_message([3; 32], [61; 32]),
        Err(Error::NotFound)
    ));
    // Already committed current envelopes remain exact retries after migration.
    assert_eq!(
        alice
            .send_peer_text(b, [60; 32], "header authenticated", now, now)
            .unwrap(),
        next
    );
}

#[test]
fn unconfirmed_delivery_window_cannot_slide_and_peer_confirmation_releases_it() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let deadline = alice
        .delivery_receipt([3; 32], [4; 32])
        .unwrap()
        .unwrap()
        .expires_at;
    let later = now + 60;
    alice
        .send_peer_text(b, [70; 32], "later initiating text", later, later)
        .unwrap();
    assert_eq!(
        alice.pending_deliveries([3; 32], later).unwrap()[0].expires_at,
        deadline
    );
    let checkpoint: Vec<u8> = alice
        .db
        .query_row("SELECT state FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert!(matches!(
        alice.send_peer_text(b, [71; 32], "expired initiation", deadline, deadline),
        Err(Error::Expired)
    ));
    assert_eq!(
        alice
            .db
            .query_row("SELECT state FROM sessions", [], |r| r.get::<_, Vec<u8>>(0))
            .unwrap(),
        checkpoint
    );
    assert!(matches!(
        alice.outgoing_message([3; 32], [71; 32]),
        Err(Error::NotFound)
    ));
    let raw = alice
        .send([3; 32], [72; 32], b"explicit expiry attempt")
        .unwrap();
    let recipient = bob.connection_session().unwrap().unwrap().device_id;
    let recipient = crate::connection::decode_id(&recipient).unwrap();
    assert!(matches!(
        alice.prepare_delivery([3; 32], [72; 32], recipient, deadline + 1, later),
        Err(Error::Expired)
    ));
    assert!(alice.pending([3; 32]).unwrap().contains(&([72; 32], raw)));
    let incoming = bob.accept_delivery(&next(&bob)).unwrap();
    bob.send_peer_text(a, [73; 32], "authenticated confirmation", now, now)
        .unwrap();
    bob.send_pending_online(incoming.session, now).unwrap();
    alice.accept_delivery(&next(&alice)).unwrap();
    alice
        .send_peer_text(
            b,
            [71; 32],
            "confirmed continuation",
            deadline + 1,
            deadline + 1,
        )
        .unwrap();
    let request = alice
        .prepare_delivery(
            [3; 32],
            [71; 32],
            recipient,
            deadline + 1 + 604800,
            deadline + 1,
        )
        .unwrap();
    assert_eq!(request.expires_at, deadline + 1 + 604800);
}

#[test]
fn initial_deadline_write_is_atomic_and_legacy_headers_do_not_reset_it() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    alice.prepare_peer_claim([1; 32], b).unwrap();
    alice.claim_prekey_online([1; 32], now).unwrap();
    for trigger in [
        "CREATE TRIGGER fail BEFORE UPDATE ON initial_headers BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;",
        "CREATE TRIGGER fail BEFORE INSERT ON deliveries BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;",
    ] {
        alice.db.execute_batch(trigger).unwrap();
        assert!(alice.start_claimed_text([1;32],[3;32],[4;32],"initial",now,now).is_err());
        assert_eq!(count(&alice,"initial_headers"),0);
        assert_eq!(count(&alice,"sessions"),0);
        alice.db.execute_batch("DROP TRIGGER fail;").unwrap();
    }
    alice
        .start_claimed_text([1; 32], [3; 32], [4; 32], "initial", now, now)
        .unwrap();
    let aad = binding(20, &[3; 32], b"initial header");
    let original: Vec<u8> = alice
        .db
        .query_row("SELECT data FROM initial_headers", [], |r| r.get(0))
        .unwrap();
    alice
        .db
        .execute("UPDATE initial_headers SET data=zeroblob(length(data))", [])
        .unwrap();
    assert!(alice
        .send_peer_text(b, [74; 32], "corrupt deadline", now, now)
        .is_err());
    let header = alice.key.open(&original, &aad).unwrap();
    let legacy = alice.key.seal(&header[..1694], &aad).unwrap();
    alice
        .db
        .execute("UPDATE initial_headers SET data=?1", [legacy])
        .unwrap();
    crate::test_schema::rewind(&alice.db, 23);
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    assert!(matches!(
        alice.send_peer_text(b, [74; 32], "missing legacy deadline", now + 1, now + 1),
        Err(Error::UnsupportedSession)
    ));
    // The original immutable request remains retryable without inventing a deadline.
    alice
        .start_claimed_text([1; 32], [3; 32], [4; 32], "initial", now, now + 1)
        .unwrap();
}
