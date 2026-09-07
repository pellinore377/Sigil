use super::*;
use crate::{
    claims::tests::pair,
    incoming::tests::{next, start, trust},
};
use sigil_protocol::mailbox::Delivery;
fn state(client: &ClientStore) -> Vec<u8> {
    client
        .db
        .query_row("SELECT state FROM sessions", [], |r| r.get(0))
        .unwrap()
}
#[test]
fn altered_outer_id_and_authenticated_wrong_contexts_never_commit_plaintext_or_ratchets() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let mut initial = next(&bob);
    let original_id = initial.message_id.clone();
    initial.message_id = transport::hex(&[98; 32]);
    assert!(matches!(
        bob.accept_delivery(&initial),
        Err(Error::InvalidEvent)
    ));
    assert_eq!(
        bob.db
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        bob.db
            .query_row("SELECT count(state) FROM prekeys", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    initial.message_id = original_id;
    let incoming = bob.accept_delivery(&initial).unwrap();
    let own = bob.own_device_binding().unwrap();
    let ctx = context(&bob.db, &bob.key, &own, &a).unwrap();
    for (n, offset) in [0, 5, 8, 9, 10, 42, 74, 106, 146, 150]
        .into_iter()
        .enumerate()
    {
        let message = [n as u8 + 30; 32];
        let mut content = ctx
            .encode(message, Content::Text("synthetic body"), now)
            .unwrap();
        content[offset] ^= 128;
        let packet = bob.send(incoming.session, message, &content).unwrap();
        let delivery = Delivery {
            sequence: 100 + n as i64,
            sender_device: bob.connection_session().unwrap().unwrap().device_id,
            message_id: transport::hex(&message),
            payload: transport::hex(&packet),
            expires_at: now + 60,
        };
        let before = state(&alice);
        assert!(
            matches!(alice.accept_delivery(&delivery), Err(Error::InvalidEvent)),
            "field {offset}"
        );
        assert_eq!(state(&alice), before);
        assert_eq!(
            alice
                .db
                .query_row("SELECT count(*) FROM incoming", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(alice.acknowledge_incoming_online().unwrap(), 0);
    }
    let packet = bob
        .send_text(
            incoming.session,
            [90; 32],
            "valid after rejected events",
            now,
            now,
        )
        .unwrap();
    let accepted = alice
        .accept_delivery(&Delivery {
            sequence: 200,
            sender_device: bob.connection_session().unwrap().unwrap().device_id,
            message_id: transport::hex(&[90; 32]),
            payload: transport::hex(&packet),
            expires_at: now + 60,
        })
        .unwrap();
    assert_eq!(accepted.text().unwrap().body, "valid after rejected events");
    assert_eq!(
        accepted.text().unwrap().conversation,
        incoming.text().unwrap().conversation
    );
    assert_eq!(
        accepted.text().unwrap().sender,
        incoming.text().unwrap().recipient
    );
    assert_eq!(
        accepted.text().unwrap().recipient,
        incoming.text().unwrap().sender
    );
}

#[test]
fn text_retries_bind_timestamp_and_freeze_delivery_with_the_outgoing_transaction() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (a, b) = trust(&mut alice, &mut bob);
    alice.prepare_peer_claim([1; 32], b).unwrap();
    alice.claim_prekey_online([1; 32], now).unwrap();
    let first = alice
        .start_claimed_text([1; 32], [3; 32], [4; 32], "typed initial", now, now)
        .unwrap();
    assert_eq!(
        alice
            .start_claimed_text([1; 32], [3; 32], [4; 32], "typed initial", now, now + 1)
            .unwrap(),
        first
    );
    assert!(matches!(
        alice.start_claimed_text([1; 32], [3; 32], [4; 32], "typed initial", now + 1, now + 1),
        Err(Error::Conflict)
    ));
    alice.send_pending_online([3; 32], now).unwrap();
    let initial = next(&bob);
    let incoming = bob.accept_delivery(&initial).unwrap();
    let before = state(&bob);
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON deliveries BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(bob
        .send_text(incoming.session, [5; 32], "reply", now, now)
        .is_err());
    assert_eq!(state(&bob), before);
    assert_eq!(
        bob.db
            .query_row("SELECT count(*) FROM outbox", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    bob.db.execute_batch("DROP TRIGGER fail;").unwrap();
    let reply = bob
        .send_text(incoming.session, [5; 32], "reply", now, now)
        .unwrap();
    assert_eq!(
        bob.send_text(incoming.session, [5; 32], "reply", now, now + 1)
            .unwrap(),
        reply
    );
    assert!(matches!(
        bob.send_text(incoming.session, [5; 32], "changed", now, now),
        Err(Error::Conflict)
    ));
    assert!(matches!(
        bob.send_text(incoming.session, [5; 32], "reply", now + 1, now),
        Err(Error::Conflict)
    ));
    let pending = bob.pending_deliveries(incoming.session, now).unwrap();
    assert_eq!(pending.len(), 1);
    bob.block_peer(a, true).unwrap();
    assert!(bob
        .send_text(incoming.session, [6; 32], "blocked", now, now)
        .is_err());
    // Already accepted content can still be acknowledged after blocking; this
    // never authorizes a new message or replaces the original verified binding.
    assert_eq!(bob.acknowledge_incoming_online().unwrap(), 1);
}

#[test]
fn ordinary_raw_handshake_content_is_not_silently_reinterpreted_as_a_text_event() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    alice.prepare_peer_claim([1; 32], b).unwrap();
    alice.claim_prekey_online([1; 32], now).unwrap();
    alice
        .start_claimed_initial([1; 32], [3; 32], [4; 32], b"legacy raw plaintext", now)
        .unwrap();
    alice.send_pending_online([3; 32], now).unwrap();
    assert!(matches!(
        bob.accept_delivery(&next(&bob)),
        Err(Error::InvalidEvent)
    ));
    assert_eq!(bob.acknowledge_incoming_online().unwrap(), 0);
    assert_eq!(
        bob.db
            .query_row("SELECT count(state) FROM prekeys", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn maximum_unicode_text_fits_initial_and_followup_and_oversize_does_not_advance() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    alice.prepare_peer_claim([1; 32], b).unwrap();
    alice.claim_prekey_online([1; 32], now).unwrap();
    let mut body = "🖋".repeat(sigil_protocol::event::MAX_TEXT / 4);
    body.extend(std::iter::repeat_n(
        'a',
        sigil_protocol::event::MAX_TEXT - body.len(),
    ));
    assert_eq!(body.len(), sigil_protocol::event::MAX_TEXT);
    assert!(matches!(
        alice.start_claimed_text([1; 32], [3; 32], [4; 32], &(body.clone() + "a"), now, now),
        Err(Error::InvalidEvent)
    ));
    assert_eq!(
        alice
            .db
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    alice
        .start_claimed_text([1; 32], [3; 32], [4; 32], &body, now, now)
        .unwrap();
    alice.send_pending_online([3; 32], now).unwrap();
    let initial = bob.accept_delivery(&next(&bob)).unwrap();
    assert_eq!(initial.text().unwrap().body, body);
    let before = state(&bob);
    assert!(matches!(
        bob.send_text(initial.session, [5; 32], &(body.clone() + "a"), now, now),
        Err(Error::InvalidEvent)
    ));
    assert_eq!(state(&bob), before);
    bob.send_text(initial.session, [5; 32], &body, now, now)
        .unwrap();
    bob.send_pending_online(initial.session, now).unwrap();
    assert_eq!(
        alice
            .accept_delivery(&next(&alice))
            .unwrap()
            .text()
            .unwrap()
            .body,
        body
    );
}

#[test]
fn logical_ids_cannot_be_redefined_by_fresh_authenticated_sessions() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let original = bob.accept_delivery(&next(&bob)).unwrap();
    let original_body = original.text().unwrap().body.to_string();
    let own = alice.own_device_binding().unwrap();
    let ctx = context(&alice.db, &alice.key, &own, &b).unwrap();
    for (n, body) in [original_body.as_str(), "altered logical event"]
        .into_iter()
        .enumerate()
    {
        let slot = [n as u8 + 20; 32];
        bob.prepare_prekey_publication(slot, true, 3600).unwrap();
        let bundle = bob.create_prekey(slot, true).unwrap();
        let plaintext = ctx.encode([4; 32], Content::Text(body), now).unwrap();
        // A malicious sender bypasses the application's own frozen delivery
        // checks, and a malicious relay bypasses its normal message-ID dedup.
        let packet = alice
            .start_initial(
                [n as u8 + 40; 32],
                [4; 32],
                bob.identity().unwrap(),
                &bundle,
                &plaintext,
            )
            .unwrap();
        let before = state(&bob);
        let delivery = Delivery {
            sequence: 900 + n as i64,
            sender_device: alice.connection_session().unwrap().unwrap().device_id,
            message_id: transport::hex(&[4; 32]),
            payload: transport::hex(&packet),
            expires_at: now + 60,
        };
        assert!(matches!(
            bob.accept_delivery(&delivery),
            Err(Error::Conflict)
        ));
        assert_eq!(state(&bob), before);
        assert_eq!(
            bob.db
                .query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            bob.db
                .query_row("SELECT count(*) FROM inbox", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            bob.db
                .query_row("SELECT count(*) FROM text_events", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            bob.db
                .query_row("SELECT count(state) FROM prekeys", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            (n + 1) as i64
        );
    }
}

#[test]
fn deduplication_commit_failure_and_migration_cannot_authorize_early_acknowledgement() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let delivery = next(&bob);
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON text_events BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(bob.accept_delivery(&delivery).is_err());
    assert_eq!(
        bob.db
            .query_row("SELECT count(*) FROM incoming", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        bob.db
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(bob.acknowledge_incoming_online().unwrap(), 0);
    bob.db.execute_batch("DROP TRIGGER fail;").unwrap();
    bob.accept_delivery(&delivery).unwrap();
    crate::test_schema::rewind(&bob.db, 16);
    drop(bob);
    let mut bob = ClientStore::open(
        &dir.path().join("bob.db"),
        StorageKey::new(sigil_crypto::Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap();
    assert_eq!(
        bob.db
            .query_row("SELECT count(*) FROM text_events", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON text_events BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(bob.acknowledge_incoming_online().is_err());
    assert_eq!(next(&bob).sequence, delivery.sequence);
    bob.db.execute_batch("DROP TRIGGER fail;").unwrap();
    assert_eq!(bob.acknowledge_incoming_online().unwrap(), 1);
    assert_eq!(
        bob.db
            .query_row("SELECT count(*) FROM text_events", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    bob.db
        .execute("UPDATE text_events SET state=zeroblob(100)", [])
        .unwrap();
    assert!(bob.accept_delivery(&delivery).is_err());
}
