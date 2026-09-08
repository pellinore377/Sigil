use super::*;
use crate::{
    claims::tests::pair,
    incoming::tests::{next, start, trust},
};
fn open(path: &Path) -> ClientStore {
    ClientStore::open(
        path,
        StorageKey::new(sigil_crypto::Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap()
}
fn count(client: &ClientStore, table: &str) -> i64 {
    client
        .db
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}
#[test]
fn signed_retry_survives_restart_lost_receipt_and_atomic_claim_staging() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let mut failed = next(&bob);
    failed.payload.replace_range(0..2, "00");
    assert!(bob.accept_delivery(&failed).is_err());
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON retry_outbox BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(bob.prepare_retry_request(&failed, now).is_err());
    assert_eq!(count(&bob, "retry_outbox"), 0);
    bob.db.execute_batch("DROP TRIGGER fail;").unwrap();
    let id = bob.prepare_retry_request(&failed, now).unwrap();
    let frozen = read(&bob.db, &bob.key, "retry_outbox", &id)
        .unwrap()
        .unwrap()
        .packet;
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    assert_eq!(bob.prepare_retry_request(&failed, now + 1).unwrap(), id);
    assert_eq!(
        read(&bob.db, &bob.key, "retry_outbox", &id)
            .unwrap()
            .unwrap()
            .packet,
        frozen
    );
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON retry_outbox BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(bob.send_retry_request_online(id, now).is_err());
    let first = next(&alice);
    assert_eq!(first.payload, transport::hex(&frozen));
    bob.db.execute_batch("DROP TRIGGER fail;").unwrap();
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    assert_eq!(
        bob.send_retry_request_online(id, now).unwrap().sequence,
        first.sequence
    );
    assert_eq!(
        alice.connected_client().unwrap().mailbox().unwrap().len(),
        1
    );
    alice.retire_session([3; 32]).unwrap();
    let claims = count(&alice, "prekey_claims");
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON retry_requests BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(alice.accept_retry_request(&first, now).is_err());
    assert_eq!(count(&alice, "prekey_claims"), claims);
    assert_eq!(count(&alice, "retry_requests"), 0);
    alice.db.execute_batch("DROP TRIGGER fail;").unwrap();
    let request = alice.accept_retry_request(&first, now).unwrap();
    assert_eq!(request.message, [4; 32]);
    assert_eq!(request.original_session, [3; 32]);
    assert_eq!(alice.active_session(b).unwrap(), None);
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    assert_eq!(alice.retry_request(id, now + 1).unwrap(), request);
    assert_eq!(
        alice.accept_retry_request(&first, now + 1).unwrap(),
        request
    );
    assert_eq!(count(&alice, "prekey_claims"), claims + 1);
    assert_eq!(count(&alice, "sessions"), 1);
    // The staged claim is usable, but acceptance alone never replaces a session.
    bob.prepare_prekey_publication([80; 32], true, 3600)
        .unwrap();
    bob.publish_prekey_online([80; 32]).unwrap();
    alice.claim_prekey_online(request.claim, now).unwrap();
    assert_eq!(alice.active_session(b).unwrap(), None);
}

#[test]
fn retry_signature_scope_expiry_and_quarantine_cannot_be_bypassed() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let id = bob.prepare_retry_request(&next(&bob), now).unwrap();
    bob.send_retry_request_online(id, now).unwrap();
    let mut delivery = next(&alice);
    let original = decode(&delivery.payload).unwrap();
    let claims = count(&alice, "prekey_claims");
    for n in 0..original.len() {
        let mut bad = original.clone();
        bad[n] ^= 1;
        delivery.payload = transport::hex(&bad);
        assert!(
            alice.accept_retry_request(&delivery, now).is_err(),
            "field {n}"
        );
    }
    delivery.payload = transport::hex(&original);
    assert_eq!(count(&alice, "retry_requests"), 0);
    assert_eq!(count(&alice, "prekey_claims"), claims);
    assert!(matches!(
        alice.accept_retry_request(&delivery, delivery.expires_at),
        Err(Error::Expired)
    ));
    let actual_sender = delivery.sender_device.clone();
    delivery.sender_device = alice.connection_session().unwrap().unwrap().device_id;
    assert!(alice.accept_retry_request(&delivery, now).is_err());
    delivery.sender_device = actual_sender;
    alice.block_peer(b, true).unwrap();
    assert!(alice.accept_retry_request(&delivery, now).is_err());
    alice.block_peer(b, false).unwrap();
    let accepted = alice.accept_retry_request(&delivery, now).unwrap();
    let mut altered = Request::from_bytes(&original).unwrap();
    altered.expires_at += 1;
    let tx = bob.db.transaction().unwrap();
    altered.signature = handshake::identity(&tx, &bob.key)
        .unwrap()
        .sign(&altered.signing_bytes().unwrap())
        .unwrap();
    drop(tx);
    delivery.payload = transport::hex(&altered.to_bytes().unwrap());
    delivery.expires_at = altered.expires_at;
    assert!(matches!(
        alice.accept_retry_request(&delivery, now + 1),
        Err(Error::Conflict)
    ));
    assert_eq!(count(&alice, "prekey_claims"), claims + 1);
    assert_eq!(count(&alice, "retry_requests"), 1);
    assert_eq!(accepted.id, id);
    altered.message = [99; 32];
    let tx = bob.db.transaction().unwrap();
    altered.signature = handshake::identity(&tx, &bob.key)
        .unwrap()
        .sign(&altered.signing_bytes().unwrap())
        .unwrap();
    drop(tx);
    delivery.message_id = transport::hex(&super::id(&altered));
    delivery.payload = transport::hex(&altered.to_bytes().unwrap());
    assert!(matches!(
        alice.accept_retry_request(&delivery, now + 1),
        Err(Error::NotFound)
    ));
    assert_eq!(count(&alice, "prekey_claims"), claims + 1);
}

fn configure(client: &mut ClientStore) {
    let account = decode_id(&client.connection_session().unwrap().unwrap().account_id).unwrap();
    client
        .configure_recovery(
            "chat.example",
            account,
            sigil_crypto::Secret32::from_bytes([7; 32]),
        )
        .unwrap();
}
fn stage(
    alice: &mut ClientStore,
    bob: &mut ClientStore,
    failed: &sigil_protocol::mailbox::Delivery,
    now: u64,
) -> RetryRequest {
    let id = bob.prepare_retry_request(failed, now).unwrap();
    bob.send_retry_request_online(id, now).unwrap();
    let request = alice.accept_retry_request(&next(alice), now).unwrap();
    bob.prepare_prekey_publication([80; 32], true, 3600)
        .unwrap();
    bob.publish_prekey_online([80; 32]).unwrap();
    alice.claim_prekey_online(request.claim, now).unwrap();
    request
}
#[test]
fn resend_is_atomic_restart_stable_and_deduplicates_delayed_original() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    configure(&mut bob);
    start(&mut alice, b, now);
    let original = next(&bob);
    let request = stage(&mut alice, &mut bob, &original, now);
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON deliveries BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(alice.resend_event(request.id, now).is_err());
    assert_eq!(count(&alice, "sessions"), 1);
    assert_eq!(alice.active_session(b).unwrap(), Some([3; 32]));
    alice.db.execute_batch("DROP TRIGGER fail;").unwrap();
    let (session, packet) = alice.resend_event(request.id, now).unwrap();
    assert_ne!(session, [3; 32]);
    assert_ne!(request.id, [4; 32]);
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    assert_eq!(
        alice.resend_event(request.id, now + 1).unwrap(),
        (session, packet)
    );
    alice.send_pending_online(session, now + 1).unwrap();
    let mut response = bob
        .connected_client()
        .unwrap()
        .mailbox()
        .unwrap()
        .into_iter()
        .find(|d| d.message_id == transport::hex(&request.id))
        .unwrap();
    let original_id = response.message_id.clone();
    response.message_id = transport::hex(&[99; 32]);
    assert!(bob.accept_delivery(&response).is_err());
    response.message_id = original_id;
    assert_eq!(count(&bob, "sessions"), 0);
    let received = bob.accept_delivery(&response).unwrap();
    assert!(!received.duplicate);
    assert_eq!(received.text().unwrap().message, [4; 32]);
    assert_eq!(received.message, request.id);
    let delayed = bob.accept_delivery(&original).unwrap();
    assert!(delayed.duplicate);
    assert_eq!(delayed.plaintext, received.plaintext);
    assert!(bob.accept_delivery(&response).unwrap().duplicate);
    assert_eq!(count(&bob, "archive_records"), 1);
}

#[test]
fn deletion_blocks_prepared_resend_at_sender_and_recipient() {
    use sigil_crypto::recovery::Content;
    for delete_sender in [true, false] {
        let (_dir, _fixture, mut alice, mut bob, now) = pair();
        let (_a, b) = trust(&mut alice, &mut bob);
        configure(&mut alice);
        configure(&mut bob);
        start(&mut alice, b, now);
        let original = next(&bob);
        let received = bob.accept_delivery(&original).unwrap();
        let history = crate::text_history_id(&received.text().unwrap(), &alice.identity().unwrap());
        let request = stage(&mut alice, &mut bob, &original, now);
        let (session, _) = alice.resend_event(request.id, now).unwrap();
        let store = if delete_sender { &mut alice } else { &mut bob };
        let mut record = store.recovery_record(history).unwrap();
        record.revision += 1;
        record.content = Content::Deleted;
        store.retain_recovery_record(&record).unwrap();
        if delete_sender {
            assert!(matches!(
                alice.resend_event(request.id, now),
                Err(Error::Obsolete)
            ));
            assert!(matches!(
                alice.send_pending_online(session, now),
                Err(Error::Obsolete)
            ));
            assert_eq!(bob.connected_client().unwrap().mailbox().unwrap().len(), 1);
        } else {
            alice.send_pending_online(session, now).unwrap();
            let response = bob
                .connected_client()
                .unwrap()
                .mailbox()
                .unwrap()
                .into_iter()
                .find(|d| d.message_id == transport::hex(&request.id))
                .unwrap();
            assert!(matches!(
                bob.accept_delivery(&response),
                Err(Error::Obsolete)
            ));
            assert_eq!(count(&bob, "sessions"), 1);
            assert!(matches!(
                bob.recovery_record(history).unwrap().content,
                Content::Deleted
            ));
        }
    }
}

#[test]
fn resend_chain_stops_after_three_fresh_sessions() {
    use sigil_protocol::mailbox::Delivery;
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let mut failed = next(&bob);
    for hop in 0..4u8 {
        let id = bob.prepare_retry_request(&failed, now).unwrap();
        let record = read(&bob.db, &bob.key, "retry_outbox", &id)
            .unwrap()
            .unwrap();
        let control = Delivery {
            origin: None,
            sequence: i64::from(hop) + 10,
            sender_device: bob.connection_session().unwrap().unwrap().device_id,
            message_id: transport::hex(&id),
            payload: transport::hex(&record.packet),
            expires_at: Request::from_bytes(&record.packet).unwrap().expires_at,
        };
        if hop == 3 {
            let claims = count(&alice, "prekey_claims");
            assert!(matches!(
                alice.accept_retry_request(&control, now),
                Err(Error::Limit)
            ));
            assert_eq!(count(&alice, "prekey_claims"), claims);
            assert_eq!(count(&alice, "sessions"), 4);
            break;
        }
        let request = alice.accept_retry_request(&control, now).unwrap();
        bob.prepare_prekey_publication([80 + hop; 32], true, 3600)
            .unwrap();
        bob.publish_prekey_online([80 + hop; 32]).unwrap();
        alice.claim_prekey_online(request.claim, now).unwrap();
        let (_, packet) = alice.resend_event(id, now).unwrap();
        failed = Delivery {
            origin: None,
            sequence: i64::from(hop) + 20,
            sender_device: alice.connection_session().unwrap().unwrap().device_id,
            message_id: transport::hex(&id),
            payload: transport::hex(&packet),
            expires_at: request.expires_at,
        };
    }
    let received = bob.accept_delivery(&failed).unwrap();
    assert!(!received.duplicate);
    assert_eq!(received.text().unwrap().message, [4; 32]);
    let original = next(&bob);
    assert!(bob.resolve_failed_delivery(&original).unwrap());
}

#[test]
fn mailbox_dispatch_commits_control_before_ack_and_recovers_lost_ack_receipt() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let original = next(&bob);
    let received = bob.accept_delivery(&original).unwrap();
    let id = bob.prepare_retry_request(&original, now).unwrap();
    bob.send_retry_request_online(id, now).unwrap();
    let control = next(&alice);
    let claims = count(&alice, "prekey_claims");
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON retry_incoming BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(alice.accept_retry_request(&control, now).is_err());
    assert_eq!(count(&alice, "retry_requests"), 0);
    assert_eq!(count(&alice, "prekey_claims"), claims);
    assert_eq!(alice.acknowledge_incoming_online().unwrap(), 0);
    alice.db.execute_batch("DROP TRIGGER fail;").unwrap();
    bob.send_text(received.session, [90; 32], "synthetic reply", now, now)
        .unwrap();
    bob.send_pending_online(received.session, now).unwrap();
    let mut attempts = alice.receive_mailbox_online(now).unwrap();
    assert_eq!(attempts.len(), 2);
    let MailboxEvent::Retry(request) = attempts.remove(0).result.unwrap() else {
        panic!("expected retry")
    };
    assert_eq!(request.id, id);
    assert!(matches!(
        attempts.remove(0).result,
        Ok(MailboxEvent::Text(_))
    ));
    assert_eq!(count(&alice, "retry_incoming"), 1);
    assert_eq!(count(&alice, "prekey_claims"), claims + 1);
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON retry_incoming BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(alice.acknowledge_incoming_online().is_err());
    assert_eq!(
        alice.connected_client().unwrap().mailbox().unwrap().len(),
        1
    );
    alice.db.execute_batch("DROP TRIGGER fail;").unwrap();
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    assert_eq!(alice.retry_request(id, now).unwrap(), request);
    assert_eq!(alice.acknowledge_incoming_online().unwrap(), 2);
    assert_eq!(alice.acknowledge_incoming_online().unwrap(), 0);
    assert!(alice
        .connected_client()
        .unwrap()
        .mailbox()
        .unwrap()
        .is_empty());
    bob.prepare_prekey_publication([80; 32], true, 3600)
        .unwrap();
    bob.publish_prekey_online([80; 32]).unwrap();
    alice.claim_prekey_online(request.claim, now).unwrap();
    alice.resend_event(id, now).unwrap();
}

#[test]
fn control_ack_requires_authentic_journal_but_not_continued_peer_trust() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let id = bob.prepare_retry_request(&next(&bob), now).unwrap();
    bob.send_retry_request_online(id, now).unwrap();
    let mut control = next(&alice);
    let original = control.payload.clone();
    control.payload.replace_range(20..22, "ff");
    assert!(alice.accept_retry_request(&control, now).is_err());
    assert_eq!(alice.acknowledge_incoming_online().unwrap(), 0);
    control.payload = original;
    alice.accept_retry_request(&control, now).unwrap();
    let sealed: Vec<u8> = alice
        .db
        .query_row("SELECT state FROM retry_incoming", [], |r| r.get(0))
        .unwrap();
    alice
        .db
        .execute("UPDATE retry_incoming SET sequence=sequence+1", [])
        .unwrap();
    assert!(alice.acknowledge_incoming_online().is_err());
    assert_eq!(
        alice.connected_client().unwrap().mailbox().unwrap().len(),
        1
    );
    alice
        .db
        .execute("UPDATE retry_incoming SET sequence=sequence-1", [])
        .unwrap();
    alice
        .db
        .execute("UPDATE retry_incoming SET state=zeroblob(69)", [])
        .unwrap();
    assert!(alice.acknowledge_incoming_online().is_err());
    alice
        .db
        .execute("UPDATE retry_incoming SET state=?1", [sealed])
        .unwrap();
    alice.block_peer(b, true).unwrap();
    assert!(alice.retry_request(id, now).is_err());
    assert_eq!(alice.acknowledge_incoming_online().unwrap(), 1);
    assert!(alice
        .connected_client()
        .unwrap()
        .mailbox()
        .unwrap()
        .is_empty());
}

#[test]
fn scheduler_resumes_claim_and_exact_send_after_lost_receipt_and_cursor_failure() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let original = next(&bob);
    let id = bob.prepare_retry_request(&original, now).unwrap();
    bob.send_retry_request_online(id, now).unwrap();
    alice.accept_retry_request(&next(&alice), now).unwrap();
    alice.acknowledge_incoming_online().unwrap();
    bob.prepare_prekey_publication([80; 32], true, 3600)
        .unwrap();
    bob.publish_prekey_online([80; 32]).unwrap();
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON deliveries BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    let mut attempts = alice.resume_retries_online(now).unwrap();
    assert_eq!(attempts.len(), 1);
    assert!(attempts.remove(0).result.is_err());
    let response = bob
        .connected_client()
        .unwrap()
        .mailbox()
        .unwrap()
        .into_iter()
        .find(|d| d.message_id == transport::hex(&id))
        .unwrap();
    assert_eq!(count(&alice, "sessions"), 2);
    alice.db.execute_batch("DROP TRIGGER fail;").unwrap();
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    assert!(alice.resume_retries_online(now).unwrap().is_empty());
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON retry_cursor BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(alice.resume_retries_online(now).is_err());
    assert_eq!(
        alice
            .delivery_receipt(work::session(&id), id)
            .unwrap()
            .unwrap()
            .sequence,
        response.sequence
    );
    alice.db.execute_batch("DROP TRIGGER fail;").unwrap();
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    // Completed receipt remains observable even after expiry and peer blocking;
    // neither condition authorizes fresh traffic.
    alice.block_peer(b, true).unwrap();
    assert!(alice
        .resume_retries_online(now + 604801)
        .unwrap()
        .is_empty());
    let receipt = alice
        .delivery_receipt(work::session(&id), id)
        .unwrap()
        .unwrap();
    assert_eq!(receipt.sequence, response.sequence);
    assert_eq!(count(&alice, "sessions"), 2);
    assert_eq!(bob.connected_client().unwrap().mailbox().unwrap().len(), 2);
    assert!(!bob.accept_delivery(&response).unwrap().duplicate);
}

#[test]
fn scheduler_bounds_work_and_persists_fair_scan_across_blocked_requests() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let mut failed = next(&bob);
    for n in 0..18u8 {
        let message = [100 + n; 32];
        alice
            .send_text([3; 32], message, "synthetic queued text", now, now)
            .unwrap();
        failed.message_id = transport::hex(&message);
        let id = bob.prepare_retry_request(&failed, now).unwrap();
        let record = read(&bob.db, &bob.key, "retry_outbox", &id)
            .unwrap()
            .unwrap();
        let control = sigil_protocol::mailbox::Delivery {
            origin: None,
            sequence: 100 + i64::from(n),
            sender_device: bob.connection_session().unwrap().unwrap().device_id,
            message_id: transport::hex(&id),
            payload: transport::hex(&record.packet),
            expires_at: Request::from_bytes(&record.packet).unwrap().expires_at,
        };
        alice.accept_retry_request(&control, now).unwrap();
    }
    alice.block_peer(b, true).unwrap();
    let first = alice.resume_retries_online(now).unwrap();
    assert_eq!(first.len(), 16);
    assert!(first.iter().all(|a| a.result.is_err()));
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    let second = alice.resume_retries_online(now).unwrap();
    assert_eq!(second.len(), 2);
    assert!(second
        .iter()
        .all(|a| a.result.is_err() && !first.iter().any(|b| b.id == a.id)));
    assert!(alice.resume_retries_online(now).unwrap().is_empty());
    assert_eq!(alice.resume_retries_online(now).unwrap().len(), 16);
    assert_eq!(count(&alice, "sessions"), 1);
    alice
        .db
        .execute("UPDATE retry_cursor SET state=zeroblob(68)", [])
        .unwrap();
    assert!(alice.resume_retries_online(now).is_err());
}

#[test]
fn cancellation_rolls_back_atomically_survives_restart_and_preserves_late_receipt() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let original = next(&bob);
    let request = stage(&mut alice, &mut bob, &original, now);
    let (session, packet) = alice.resend_event(request.id, now).unwrap();
    // A response already in flight may reach the server before cancellation.
    let submit = alice.pending_deliveries(session, now).unwrap().remove(0);
    let receipt = alice.connected_client().unwrap().submit(&submit).unwrap();
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON retry_requests BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(alice.cancel_retry_request(request.id).is_err());
    assert_eq!(
        alice.resend_event(request.id, now).unwrap(),
        (session, packet)
    );
    alice.db.execute_batch("DROP TRIGGER fail;").unwrap();
    assert!(alice.cancel_retry_request(request.id).unwrap());
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    assert!(alice.cancel_retry_request(request.id).unwrap());
    assert!(matches!(
        alice.retry_request(request.id, now),
        Err(Error::Cancelled)
    ));
    assert!(matches!(
        alice.resend_event(request.id, now),
        Err(Error::Cancelled)
    ));
    assert!(alice.pending_deliveries(session, now).unwrap().is_empty());
    assert!(alice.resume_retries_online(now).unwrap().is_empty());
    assert!(!alice
        .expire_delivery(session, request.id, now + 604801)
        .unwrap());
    assert!(alice.claim_prekey_online(request.claim, now).is_err());
    // Cancellation preserves acknowledgement evidence and late server receipts.
    assert_eq!(alice.acknowledge_incoming_online().unwrap(), 1);
    alice
        .acknowledge_sent(session, request.id, &receipt)
        .unwrap();
    assert!(!alice.cancel_retry_request(request.id).unwrap());
    assert_eq!(
        alice
            .delivery_receipt(session, request.id)
            .unwrap()
            .unwrap(),
        receipt
    );
    assert_eq!(
        read(&alice.db, &alice.key, "retry_requests", &request.id)
            .unwrap()
            .unwrap()
            .finished,
        1
    );
    assert!(alice.resume_retries_online(now).unwrap().is_empty());
    assert_eq!(count(&alice, "sessions"), 2);
}

#[test]
fn cancellation_before_claim_prevents_replay_from_reopening_work() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let id = bob.prepare_retry_request(&next(&bob), now).unwrap();
    bob.send_retry_request_online(id, now).unwrap();
    let control = next(&alice);
    let request = alice.accept_retry_request(&control, now).unwrap();
    assert!(alice.cancel_retry_request(id).unwrap());
    assert_eq!(alice.accept_retry_request(&control, now).unwrap(), request);
    assert_eq!(count(&alice, "prekey_claims"), 2);
    assert!(alice.resume_retries_online(now).unwrap().is_empty());
    assert_eq!(count(&alice, "sessions"), 1);
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    alice
        .db
        .execute(
            "UPDATE retry_requests SET finished=0 WHERE id=?1",
            [id.as_slice()],
        )
        .unwrap();
    assert!(alice
        .resume_retries_online(now)
        .unwrap()
        .remove(0)
        .result
        .is_err());
    assert!(alice.cancel_retry_request(id).is_err());
    assert_eq!(count(&alice, "sessions"), 1);
}

#[test]
fn scheduler_cancels_expired_deleted_and_superseded_work_atomically() {
    use sigil_crypto::recovery::Content;
    for reason in 0..3 {
        let (_dir, _fixture, mut alice, mut bob, now) = pair();
        let (_a, b) = trust(&mut alice, &mut bob);
        configure(&mut alice);
        start(&mut alice, b, now);
        let failed = next(&bob);
        let request = stage(&mut alice, &mut bob, &failed, now);
        let session = work::session(&request.id);
        if reason != 0 {
            alice.resend_event(request.id, now).unwrap();
            let plaintext = alice.outgoing_message([3; 32], [4; 32]).unwrap();
            let text = sigil_protocol::event::Text::from_bytes(&plaintext).unwrap();
            let history = crate::text_history_id(&text, &alice.identity().unwrap());
            let mut record = alice.recovery_record(history).unwrap();
            record.revision += 1;
            record.content = if reason == 1 {
                Content::Deleted
            } else {
                Content::Retained(b"synthetic replacement".to_vec().into())
            };
            alice.retain_recovery_record(&record).unwrap();
        }
        let clock = if reason == 0 { request.expires_at } else { now };
        alice.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON retry_requests BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
        assert!(alice
            .resume_retries_online(clock)
            .unwrap()
            .remove(0)
            .result
            .is_err());
        assert_eq!(
            read(&alice.db, &alice.key, "retry_requests", &request.id)
                .unwrap()
                .unwrap()
                .finished,
            0
        );
        if reason != 0 {
            assert_eq!(alice.pending_deliveries(session, now).unwrap().len(), 1);
        }
        alice.db.execute_batch("DROP TRIGGER fail;").unwrap();
        assert!(alice.resume_retries_online(clock).unwrap().is_empty());
        assert!(matches!(
            alice.resume_retries_online(clock).unwrap().remove(0).result,
            Err(Error::Cancelled)
        ));
        assert_eq!(
            read(&alice.db, &alice.key, "retry_requests", &request.id)
                .unwrap()
                .unwrap()
                .finished,
            2
        );
        assert!(alice.claim_prekey_online(request.claim, now).is_err());
        if reason != 0 {
            assert!(alice.pending_deliveries(session, now).unwrap().is_empty());
        }
        assert_eq!(count(&alice, "sessions"), if reason == 0 { 1 } else { 2 });
        assert_eq!(alice.acknowledge_incoming_online().unwrap(), 1);
        assert_eq!(bob.connected_client().unwrap().mailbox().unwrap().len(), 1);
    }
}

#[test]
fn scheduler_does_not_cancel_on_trust_failure_corruption_or_clock_rollback() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    configure(&mut alice);
    start(&mut alice, b, now);
    let failed = next(&bob);
    let request = stage(&mut alice, &mut bob, &failed, now);
    let (session, packet) = alice.resend_event(request.id, now).unwrap();
    alice.block_peer(b, true).unwrap();
    assert!(alice
        .resume_retries_online(now)
        .unwrap()
        .remove(0)
        .result
        .is_err());
    alice.block_peer(b, false).unwrap();
    assert!(alice.resume_retries_online(now).unwrap().is_empty());
    let data: Vec<u8> = alice
        .db
        .query_row("SELECT data FROM archive_records", [], |r| r.get(0))
        .unwrap();
    alice
        .db
        .execute("UPDATE archive_records SET data=zeroblob(length(data))", [])
        .unwrap();
    assert!(alice
        .resume_retries_online(now)
        .unwrap()
        .remove(0)
        .result
        .is_err());
    alice
        .db
        .execute("UPDATE archive_records SET data=?1", [data])
        .unwrap();
    assert!(alice.resume_retries_online(now).unwrap().is_empty());
    assert!(matches!(
        alice
            .resume_retries_online(now - 1)
            .unwrap()
            .remove(0)
            .result,
        Err(Error::Expired)
    ));
    assert_eq!(
        read(&alice.db, &alice.key, "retry_requests", &request.id)
            .unwrap()
            .unwrap()
            .finished,
        0
    );
    assert_eq!(
        alice.resend_event(request.id, now).unwrap(),
        (session, packet)
    );
    assert!(alice.resume_retries_online(now).unwrap().is_empty());
    assert!(alice
        .resume_retries_online(now)
        .unwrap()
        .remove(0)
        .result
        .is_ok());
}

#[test]
fn obsolete_incoming_controls_are_journaled_without_claims_and_ack_after_restart() {
    use sigil_crypto::recovery::Content;
    for reason in 0..3 {
        let (dir, _fixture, mut alice, mut bob, now) = pair();
        let (_a, b) = trust(&mut alice, &mut bob);
        configure(&mut alice);
        start(&mut alice, b, now);
        let id = bob.prepare_retry_request(&next(&bob), now).unwrap();
        bob.send_retry_request_online(id, now).unwrap();
        let control = next(&alice);
        if reason != 0 {
            let plaintext = alice.outgoing_message([3; 32], [4; 32]).unwrap();
            let text = sigil_protocol::event::Text::from_bytes(&plaintext).unwrap();
            let history = crate::text_history_id(&text, &alice.identity().unwrap());
            let mut record = alice.recovery_record(history).unwrap();
            record.revision += 1;
            record.content = if reason == 1 {
                Content::Deleted
            } else {
                Content::Retained(b"synthetic replacement".to_vec().into())
            };
            alice.retain_recovery_record(&record).unwrap();
        }
        let clock = if reason == 0 { control.expires_at } else { now };
        alice.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON retry_incoming BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
        assert!(alice
            .receive_mailbox_online(clock)
            .unwrap()
            .remove(0)
            .result
            .is_err());
        assert_eq!(count(&alice, "retry_requests"), 0);
        assert_eq!(count(&alice, "prekey_claims"), 1);
        assert_eq!(alice.acknowledge_incoming_online().unwrap(), 0);
        alice.db.execute_batch("DROP TRIGGER fail;").unwrap();
        assert!(alice.receive_mailbox_online(clock).unwrap().is_empty());
        assert!(
            matches!(alice.receive_mailbox_online(clock).unwrap().remove(0).result, Ok(MailboxEvent::DiscardedRetry(actual)) if actual == id)
        );
        drop(alice);
        let mut alice = open(&dir.path().join("alice.db"));
        assert!(alice.resume_retries_online(clock).unwrap().is_empty());
        assert_eq!(alice.acknowledge_incoming_online().unwrap(), 1);
        assert!(alice
            .connected_client()
            .unwrap()
            .mailbox()
            .unwrap()
            .is_empty());
        assert!(
            matches!(alice.route_retry(&control, clock, true).unwrap(), MailboxEvent::DiscardedRetry(actual) if actual == id)
        );
        assert_eq!(alice.acknowledge_incoming_online().unwrap(), 0);
        assert_eq!(count(&alice, "prekey_claims"), 1);
        assert_eq!(count(&alice, "sessions"), 1);
        assert_eq!(count(&alice, "retry_requests"), 1);
    }
}

#[test]
fn discard_never_acknowledges_unverified_or_corrupt_expired_controls() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    configure(&mut alice);
    start(&mut alice, b, now);
    let id = bob.prepare_retry_request(&next(&bob), now).unwrap();
    bob.send_retry_request_online(id, now).unwrap();
    let mut control = next(&alice);
    let packet = decode(&control.payload).unwrap();
    for n in [8, 40, 72, 104, 112, 175] {
        let mut bad = packet.clone();
        bad[n] ^= 1;
        control.payload = transport::hex(&bad);
        assert!(alice
            .route_retry(&control, control.expires_at, true)
            .is_err());
    }
    control.payload = transport::hex(&packet);
    alice.block_peer(b, true).unwrap();
    assert!(alice
        .route_retry(&control, control.expires_at, true)
        .is_err());
    alice.block_peer(b, false).unwrap();
    assert!(alice.route_retry(&control, now - 1, true).is_err());
    alice
        .db
        .execute("UPDATE archive_records SET data=zeroblob(length(data))", [])
        .unwrap();
    assert!(alice
        .route_retry(&control, control.expires_at, true)
        .is_err());
    assert_eq!(alice.acknowledge_incoming_online().unwrap(), 0);
    assert_eq!(count(&alice, "retry_requests"), 0);
    assert_eq!(count(&alice, "prekey_claims"), 1);
    assert_eq!(
        alice.connected_client().unwrap().mailbox().unwrap().len(),
        1
    );
}

#[test]
fn redispatched_expired_control_cancels_queued_response_without_reopening_claim() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let failed = next(&bob);
    let request = stage(&mut alice, &mut bob, &failed, now);
    let (session, _) = alice.resend_event(request.id, now).unwrap();
    let control = next(&alice);
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON retry_requests BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(alice
        .route_retry(&control, request.expires_at, true)
        .is_err());
    assert_eq!(alice.pending_deliveries(session, now).unwrap().len(), 1);
    alice.db.execute_batch("DROP TRIGGER fail;").unwrap();
    assert!(
        matches!(alice.route_retry(&control, request.expires_at, true).unwrap(), MailboxEvent::DiscardedRetry(id) if id == request.id)
    );
    assert!(alice.pending_deliveries(session, now).unwrap().is_empty());
    assert!(alice.claim_prekey_online(request.claim, now).is_err());
    assert_eq!(count(&alice, "prekey_claims"), 2);
    assert_eq!(count(&alice, "sessions"), 2);
    assert_eq!(alice.acknowledge_incoming_online().unwrap(), 1);
    assert!(alice
        .resume_retries_online(request.expires_at)
        .unwrap()
        .is_empty());
}

#[test]
fn journal_reclamation_requires_ack_deadline_and_atomic_cursor_commit() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let id = bob.prepare_retry_request(&next(&bob), now).unwrap();
    bob.send_retry_request_online(id, now).unwrap();
    let control = next(&alice);
    alice.accept_retry_request(&control, now).unwrap();
    alice.cancel_retry_request(id).unwrap();
    assert_eq!(
        alice
            .reclaim_retry_journals(control.expires_at)
            .unwrap()
            .reclaimed,
        0
    );
    alice.acknowledge_incoming_online().unwrap();
    assert_eq!(alice.reclaim_retry_journals(now).unwrap().scanned, 0);
    assert_eq!(alice.reclaim_retry_journals(now).unwrap().reclaimed, 0);
    assert_eq!(alice.reclaim_retry_journals(now).unwrap().scanned, 0);
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON retry_gc_cursor BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(alice.reclaim_retry_journals(control.expires_at).is_err());
    assert_eq!(count(&alice, "retry_incoming"), 1);
    alice.db.execute_batch("DROP TRIGGER fail;").unwrap();
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    assert_eq!(
        alice
            .reclaim_retry_journals(control.expires_at)
            .unwrap()
            .reclaimed,
        1
    );
    assert_eq!(count(&alice, "retry_requests"), 1);
    assert_eq!(count(&alice, "retry_incoming"), 0);
    assert!(
        matches!(alice.route_retry(&control, control.expires_at, true).unwrap(), MailboxEvent::DiscardedRetry(actual) if actual == id)
    );
    assert_eq!(alice.acknowledge_incoming_online().unwrap(), 1);
    assert_eq!(count(&alice, "prekey_claims"), 2);
    assert_eq!(count(&alice, "sessions"), 1);
    assert!(alice
        .resume_retries_online(control.expires_at)
        .unwrap()
        .is_empty());
}

#[test]
fn journal_gc_skips_pending_prefix_across_restart_and_rejects_forged_ack() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let id = bob.prepare_retry_request(&next(&bob), now).unwrap();
    bob.send_retry_request_online(id, now).unwrap();
    let mut control = next(&alice);
    for n in 0..18 {
        control.sequence = 100 + n;
        alice
            .route_retry(&control, control.expires_at, true)
            .unwrap();
        if n >= 16 {
            let own = alice.own_device_binding().unwrap();
            let tx = alice.db.transaction().unwrap();
            journal_save(&tx, &alice.key, &own, control.sequence, id, true).unwrap();
            tx.commit().unwrap();
        }
    }
    let first = alice.reclaim_retry_journals(control.expires_at).unwrap();
    assert_eq!((first.scanned, first.reclaimed), (16, 0));
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    let second = alice.reclaim_retry_journals(control.expires_at).unwrap();
    assert_eq!((second.scanned, second.reclaimed), (2, 2));
    assert_eq!(count(&alice, "retry_incoming"), 16);
    assert_eq!(
        alice
            .reclaim_retry_journals(control.expires_at)
            .unwrap()
            .scanned,
        0
    );
    alice
        .db
        .execute(
            "UPDATE retry_incoming SET acknowledged=1 WHERE sequence=100",
            [],
        )
        .unwrap();
    assert!(alice.reclaim_retry_journals(control.expires_at).is_err());
    assert_eq!(count(&alice, "retry_incoming"), 16);
    alice
        .db
        .execute(
            "UPDATE retry_incoming SET acknowledged=0 WHERE sequence=100",
            [],
        )
        .unwrap();
    alice
        .db
        .execute("UPDATE retry_gc_cursor SET state=zeroblob(44)", [])
        .unwrap();
    assert!(alice.reclaim_retry_journals(control.expires_at).is_err());
}

#[test]
fn journal_gc_preserves_active_work_and_delayed_completed_response() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let failed = next(&bob);
    let request = stage(&mut alice, &mut bob, &failed, now);
    alice.acknowledge_incoming_online().unwrap();
    assert_eq!(
        alice
            .reclaim_retry_journals(request.expires_at)
            .unwrap()
            .reclaimed,
        0
    );
    let (session, _) = alice.resend_event(request.id, now).unwrap();
    alice.send_pending_online(session, now).unwrap();
    assert_eq!(
        alice
            .reclaim_retry_journals(request.expires_at)
            .unwrap()
            .scanned,
        0
    );
    assert_eq!(
        alice
            .reclaim_retry_journals(request.expires_at)
            .unwrap()
            .reclaimed,
        1
    );
    assert!(alice
        .delivery_receipt(session, request.id)
        .unwrap()
        .is_some());
    let response = bob
        .connected_client()
        .unwrap()
        .mailbox()
        .unwrap()
        .into_iter()
        .find(|d| d.message_id == transport::hex(&request.id))
        .unwrap();
    assert!(!bob.accept_delivery(&response).unwrap().duplicate);
    assert!(bob.accept_delivery(&failed).unwrap().duplicate);
    assert_eq!(count(&alice, "retry_requests"), 1);
}

#[test]
fn outgoing_control_gc_prevents_renewal_and_rolls_back_failed_retirement() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let failed = next(&bob);
    let id = bob.prepare_retry_request(&failed, now).unwrap();
    assert_eq!(bob.reclaim_outgoing_retry_controls(now).unwrap().retired, 0);
    assert_eq!(bob.reclaim_outgoing_retry_controls(now).unwrap().scanned, 0); // wrap
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE DELETE ON retry_outbox BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(bob.reclaim_outgoing_retry_controls(now + 604800).is_err());
    assert_eq!(count(&bob, "retry_outbox"), 1);
    assert_eq!(count(&bob, "retired_retry_outbox"), 0);
    bob.db.execute_batch("DROP TRIGGER fail;").unwrap();
    assert_eq!(
        bob.reclaim_outgoing_retry_controls(now + 604800)
            .unwrap()
            .retired,
        1
    );
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    assert!(matches!(
        bob.prepare_retry_request(&failed, now + 604800),
        Err(Error::Expired)
    ));
    assert!(bob.send_retry_request_online(id, now + 604800).is_err());
    assert_eq!(count(&bob, "retry_outbox"), 0);
    assert!(!bob.accept_delivery(&failed).unwrap().duplicate);
    bob.db
        .execute("UPDATE retired_retry_outbox SET state=zeroblob(76)", [])
        .unwrap();
    assert!(bob.prepare_retry_request(&failed, now + 604800).is_err());
    assert_eq!(count(&bob, "retry_outbox"), 0);
}

#[test]
fn outgoing_control_gc_preserves_response_and_descendant_proofs() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let mut failed = next(&bob);
    let request = stage(&mut alice, &mut bob, &failed, now);
    let (session, _) = alice.resend_event(request.id, now).unwrap();
    alice.send_pending_online(session, now).unwrap();
    failed.message_id = transport::hex(&request.id);
    let child = bob.prepare_retry_request(&failed, now + 1).unwrap();
    assert_eq!(
        bob.reclaim_outgoing_retry_controls(request.expires_at)
            .unwrap()
            .retired,
        0
    );
    let response = bob
        .connected_client()
        .unwrap()
        .mailbox()
        .unwrap()
        .into_iter()
        .find(|d| d.message_id == transport::hex(&request.id))
        .unwrap();
    assert!(!bob.accept_delivery(&response).unwrap().duplicate);
    bob.acknowledge_incoming_online().unwrap();
    assert_eq!(
        bob.reclaim_outgoing_retry_controls(request.expires_at + 1)
            .unwrap()
            .scanned,
        0
    ); // wrap
    assert_eq!(
        bob.reclaim_outgoing_retry_controls(request.expires_at + 1)
            .unwrap()
            .retired,
        1
    );
    assert!(read(&bob.db, &bob.key, "retry_outbox", &child)
        .unwrap()
        .is_none());
    assert_eq!(
        bob.reclaim_outgoing_retry_controls(request.expires_at + 1)
            .unwrap()
            .retired,
        0
    );
    assert!(bob.accept_delivery(&response).unwrap().duplicate);
    assert_eq!(count(&bob, "retry_outbox"), 1);
}

#[test]
fn outgoing_control_gc_bounds_retirement_and_authenticates_dependencies() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let mut failed = next(&bob);
    for n in 0..18u8 {
        failed.message_id = transport::hex(&[100 + n; 32]);
        bob.prepare_retry_request(&failed, now).unwrap();
    }
    let (id, state): (Vec<u8>, Vec<u8>) = bob
        .db
        .query_row(
            "SELECT id,state FROM retry_outbox ORDER BY id LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    bob.db
        .execute(
            "UPDATE retry_outbox SET state=zeroblob(length(state)) WHERE id=?1",
            [id.as_slice()],
        )
        .unwrap();
    assert!(bob.reclaim_outgoing_retry_controls(now + 604800).is_err());
    assert_eq!(count(&bob, "retired_retry_outbox"), 0);
    bob.db
        .execute("UPDATE retry_outbox SET state=?1 WHERE id=?2", (state, id))
        .unwrap();
    let first = bob.reclaim_outgoing_retry_controls(now + 604800).unwrap();
    assert_eq!((first.scanned, first.retired), (16, 16));
    assert_eq!(
        bob.reclaim_outgoing_retry_controls(now + 604800)
            .unwrap()
            .retired,
        2
    );
    assert_eq!(count(&bob, "retry_outbox"), 0);
    assert_eq!(count(&bob, "retired_retry_outbox"), 18);
}

#[test]
fn accepted_control_gc_is_atomic_and_replays_use_compact_proof() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let id = bob.prepare_retry_request(&next(&bob), now).unwrap();
    bob.send_retry_request_online(id, now).unwrap();
    let mut control = next(&alice);
    let deadline = control.expires_at;
    alice.route_retry(&control, deadline, true).unwrap();
    assert_eq!(
        alice
            .reclaim_accepted_retry_controls(deadline)
            .unwrap()
            .retired,
        0
    );
    alice.acknowledge_incoming_online().unwrap();
    assert_eq!(
        alice
            .reclaim_accepted_retry_controls(deadline)
            .unwrap()
            .retired,
        0
    );
    alice.reclaim_retry_journals(deadline).unwrap();
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE DELETE ON retry_requests BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(alice.reclaim_accepted_retry_controls(deadline).is_err());
    assert_eq!(count(&alice, "retry_requests"), 1);
    assert_eq!(count(&alice, "retired_retry_requests"), 0);
    alice.db.execute_batch("DROP TRIGGER fail;").unwrap();
    assert_eq!(
        alice
            .reclaim_accepted_retry_controls(deadline)
            .unwrap()
            .retired,
        1
    );
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    assert!(
        matches!(alice.route_retry(&control, deadline, true).unwrap(), MailboxEvent::DiscardedRetry(actual) if actual == id)
    );
    assert_eq!(alice.acknowledge_incoming_online().unwrap(), 1);
    alice.reclaim_retry_journals(deadline).unwrap(); // wraps past reclaimed sequence
    assert_eq!(alice.reclaim_retry_journals(deadline).unwrap().reclaimed, 1);
    assert_eq!(count(&alice, "retry_requests"), 0);
    assert_eq!(count(&alice, "prekey_claims"), 1);
    assert_eq!(count(&alice, "sessions"), 1);
    let original = control.payload.clone();
    let mut renewed = Request::from_bytes(&decode(&original).unwrap()).unwrap();
    renewed.expires_at += 1;
    let tx = bob.db.transaction().unwrap();
    renewed.signature = handshake::identity(&tx, &bob.key)
        .unwrap()
        .sign(&renewed.signing_bytes().unwrap())
        .unwrap();
    drop(tx);
    control.payload = transport::hex(&renewed.to_bytes().unwrap());
    control.expires_at = renewed.expires_at;
    assert!(matches!(
        alice.route_retry(&control, deadline, true),
        Err(Error::Conflict)
    ));
    assert_eq!(count(&alice, "retry_incoming"), 0);
    control.payload = original;
    control.expires_at = deadline;
    alice
        .db
        .execute("UPDATE retired_retry_requests SET state=zeroblob(140)", [])
        .unwrap();
    assert!(alice.route_retry(&control, deadline, true).is_err());
    assert_eq!(count(&alice, "retry_incoming"), 0);
}

#[test]
fn accepted_control_gc_retains_active_live_and_prepared_response_evidence() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let failed = next(&bob);
    let request = stage(&mut alice, &mut bob, &failed, now);
    alice.acknowledge_incoming_online().unwrap();
    assert_eq!(
        alice
            .reclaim_accepted_retry_controls(request.expires_at)
            .unwrap()
            .retired,
        0
    );
    let (session, _) = alice.resend_event(request.id, now).unwrap();
    alice.cancel_retry_request(request.id).unwrap();
    alice.reclaim_retry_journals(request.expires_at).unwrap();
    assert_eq!(
        alice.reclaim_accepted_retry_controls(now).unwrap().retired,
        0
    );
    assert_eq!(
        alice
            .reclaim_accepted_retry_controls(request.expires_at)
            .unwrap()
            .retired,
        0
    );
    assert!(matches!(
        alice.retry_request(request.id, now),
        Err(Error::Cancelled)
    ));
    assert!(alice.pending_deliveries(session, now).unwrap().is_empty());
    assert_eq!(count(&alice, "retry_requests"), 1);
    assert_eq!(count(&alice, "retired_retry_requests"), 0);
    assert_eq!(
        alice
            .reclaim_accepted_retry_controls(request.expires_at)
            .unwrap()
            .scanned,
        0
    ); // wrap
    alice
        .db
        .execute(
            "UPDATE retry_requests SET state=zeroblob(length(state))",
            [],
        )
        .unwrap();
    assert!(alice
        .reclaim_accepted_retry_controls(request.expires_at)
        .is_err());
    assert_eq!(count(&alice, "retired_retry_requests"), 0);
}

#[path = "retry_acceptance_tests.rs"]
mod acceptance;
