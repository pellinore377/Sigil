//! Cross-feature acceptance using synthetic devices and the real HTTPS fixture.
use super::*;

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "bulk cryptographic boundary acceptance; run cargo test --release -p sigil-client --lib"
)]
fn retained_recovery_controls_journals_and_proofs_cross_old_limits() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let original = next(&bob);
    bob.db.execute("UPDATE prekeys SET state=NULL", []).unwrap();
    let alice_own = alice.own_device_binding().unwrap();
    let bob_own = bob.own_device_binding().unwrap();
    let requester = device_fingerprint(&bob_own).unwrap();
    let target = device_fingerprint(&alice_own).unwrap();
    let bob_tx = bob.db.transaction().unwrap();
    let identity = handshake::identity(&bob_tx, &bob.key).unwrap();
    let tx = alice.db.transaction().unwrap();
    for n in 0u32..4096 {
        let mut message = [233; 32];
        message[..4].copy_from_slice(&n.to_be_bytes());
        let mut request = Request {
            message,
            requester,
            target,
            expires_at: now - 1,
            signature: [0; 64],
        };
        request.signature = identity.sign(&request.signing_bytes().unwrap()).unwrap();
        let id = super::super::id(&request);
        save(
            &tx,
            &alice.key,
            "retry_requests",
            &id,
            &Record {
                peer: b,
                packet: request.to_bytes().unwrap(),
                receipt: None,
                finished: 2,
            },
        )
        .unwrap();
        journal_save(&tx, &alice.key, &alice_own, 10000 + n as i64, id, true).unwrap();
        let mut tombstone = target.to_vec();
        tombstone.extend_from_slice(&b);
        tombstone.extend_from_slice(&Sha256::digest(request.to_bytes().unwrap()));
        tombstone.extend_from_slice(&(now - 1).to_be_bytes());
        let tombstone_id: Id =
            Sha256::digest([b"synthetic retired acceptance".as_slice(), &n.to_be_bytes()].concat())
                .into();
        tx.execute(
            "INSERT INTO retired_retry_requests VALUES(?1,?2)",
            (
                tombstone_id.as_slice(),
                alice
                    .key
                    .seal(
                        &tombstone,
                        &binding(26, &tombstone_id, b"accepted retry retired"),
                    )
                    .unwrap(),
            ),
        )
        .unwrap();
    }
    tx.commit().unwrap();
    bob_tx.commit().unwrap();
    crate::test_schema::rewind(&alice.db, 41);
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    let step = bob.sync_step_online(now);
    let RecoveryAdvice::Offer(action) = &step.incoming[0].recovery else {
        panic!("missing-key action");
    };
    let control = bob.approve_recovery(action, now).unwrap();
    assert!(bob.resume_retry_controls_online(now).unwrap()[0]
        .result
        .is_ok());
    let step = alice.sync_step_online(now);
    assert!(step.failure.is_none(), "{:?}", step.failure);
    assert!(step
        .incoming
        .iter()
        .any(|v| matches!(&v.result,Ok(MailboxEvent::Retry(r)) if r.id==control)));
    assert_eq!(count(&alice, "retry_requests"), 4097);
    assert_eq!(count(&alice, "retry_incoming"), 4097);
    let response = bob
        .connected_client()
        .unwrap()
        .mailbox()
        .unwrap()
        .into_iter()
        .find(|d| d.message_id == transport::hex(&control))
        .unwrap();
    assert!(!bob.accept_delivery(&response).unwrap().duplicate);
    assert!(bob.resolve_failed_delivery(&original).unwrap());
    let sealed: Vec<u8> = bob
        .db
        .query_row(
            "SELECT state FROM recovered_deliveries WHERE sequence=?1",
            [original.sequence],
            |r| r.get(0),
        )
        .unwrap();
    let raw = bob
        .key
        .open(
            &sealed,
            &binding(27, &requester, &original.sequence.to_be_bytes()),
        )
        .unwrap();
    // Authenticated duplicate-delivery proof fixtures; the next real resolution
    // must still work when 4,096 historical acknowledgements are retained.
    let mut value: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    value["acknowledged"] = true.into();
    let raw = serde_json::to_vec(&value).unwrap();
    let tx = bob.db.transaction().unwrap();
    tx.execute(
        "DELETE FROM recovered_deliveries WHERE sequence=?1",
        [original.sequence],
    )
    .unwrap();
    for sequence in 20000i64..24096 {
        tx.execute(
            "INSERT INTO recovered_deliveries VALUES(?1,1,?2)",
            (
                sequence,
                bob.key
                    .seal(&raw, &binding(27, &requester, &sequence.to_be_bytes()))
                    .unwrap(),
            ),
        )
        .unwrap();
    }
    tx.commit().unwrap();
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    assert!(bob.resolve_failed_delivery(&original).unwrap());
    assert_eq!(count(&bob, "recovered_deliveries"), 4097);
    bob.acknowledge_incoming_online().unwrap();
    assert!(bob
        .connected_client()
        .unwrap()
        .mailbox()
        .unwrap()
        .is_empty());
    assert!(bob.accept_delivery(&response).unwrap().duplicate);
    assert!(bob.peer(a).unwrap().trusted);
    let journal = alice.reclaim_retry_journals(now).unwrap();
    assert_eq!((journal.scanned, journal.reclaimed), (16, 15));
    let cleanup = alice.reclaim_accepted_retry_controls(now).unwrap();
    assert!(cleanup.scanned <= 16);
    // Cursor batches may first visit records still pinned by their journals.
    for _ in 0..260 {
        alice.reclaim_accepted_retry_controls(now).unwrap();
    }
    assert!(count(&alice, "retired_retry_requests") > 4096);
    assert!(alice
        .delivery_receipt(work::session(&control), control)
        .unwrap()
        .is_some());
}

#[test]
fn lost_prekey_recovers_across_restart_key_unavailability_and_cleanup() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (a, b) = trust(&mut alice, &mut bob);
    configure(&mut alice);
    configure(&mut bob);
    start(&mut alice, b, now);
    let original = next(&bob);
    // Simulate irreversible loss of the old private prekey, not a damaged wire
    // packet. New publication must supply a different key for recovery to work.
    bob.db.execute("UPDATE prekeys SET state=NULL", []).unwrap();
    assert!(bob
        .receive_mailbox_online(now)
        .unwrap()
        .remove(0)
        .result
        .is_err());
    assert_eq!(bob.acknowledge_incoming_online().unwrap(), 0);
    let id = bob.prepare_retry_request(&original, now).unwrap();
    bob.send_retry_request_online(id, now).unwrap();
    let MailboxEvent::Retry(request) = alice
        .receive_mailbox_online(now)
        .unwrap()
        .remove(0)
        .result
        .unwrap()
    else {
        panic!("expected recovery work")
    };
    assert_eq!(request.id, id);
    alice.acknowledge_incoming_online().unwrap();
    let unavailable = alice.resume_retries_online(now).unwrap().remove(0);
    assert!(matches!(unavailable.result, Err(Error::Network(_))));
    assert_eq!(count(&alice, "sessions"), 1);
    drop(alice);
    drop(bob);
    let mut alice = open(&dir.path().join("alice.db"));
    let mut bob = open(&dir.path().join("bob.db"));
    bob.prepare_prekey_publication([80; 32], true, 3600)
        .unwrap();
    bob.publish_prekey_online([80; 32]).unwrap();
    assert!(alice.resume_retries_online(now).unwrap().is_empty()); // scan wraps
    assert!(alice
        .resume_retries_online(now)
        .unwrap()
        .remove(0)
        .result
        .is_ok());
    let attempts = bob.receive_mailbox_online(now).unwrap();
    assert_eq!(attempts.len(), 1); // old failure did not block the later response
    let MailboxEvent::Text(received) = attempts.into_iter().next().unwrap().result.unwrap() else {
        panic!("expected recovered text")
    };
    assert_eq!(received.text().unwrap().message, [4; 32]);
    assert_eq!(received.text().unwrap().body, "synthetic initial");
    assert!(!received.duplicate);
    assert_eq!(count(&bob, "archive_records"), 1);
    assert_eq!(bob.acknowledge_incoming_online().unwrap(), 1);
    assert_eq!(bob.connected_client().unwrap().mailbox().unwrap().len(), 1);
    assert!(bob.receive_mailbox_online(now).unwrap().is_empty()); // wraps
    assert!(
        matches!(bob.receive_mailbox_online(now).unwrap().remove(0).result, Ok(MailboxEvent::RecoveredDelivery(message)) if message == [4; 32])
    );
    assert_eq!(bob.acknowledge_incoming_online().unwrap(), 1);
    assert!(bob
        .connected_client()
        .unwrap()
        .mailbox()
        .unwrap()
        .is_empty());
    assert!(!alice.session_peer_confirmed(work::session(&id)).unwrap());
    let (reply_session, _) = bob
        .send_peer_text(a, [88; 32], "synthetic recovered reply", now, now)
        .unwrap();
    bob.send_pending_online(reply_session, now).unwrap();
    assert!(matches!(
        alice.receive_mailbox_online(now).unwrap().remove(0).result,
        Ok(MailboxEvent::Text(_))
    ));
    assert!(alice.session_peer_confirmed(work::session(&id)).unwrap());
    alice.acknowledge_incoming_online().unwrap();
    assert_eq!(alice.reclaim_retry_journals(now).unwrap().reclaimed, 0);
    drop(alice);
    drop(bob);
    let mut alice = open(&dir.path().join("alice.db"));
    let mut bob = open(&dir.path().join("bob.db"));
    let (session, _) = alice
        .send_peer_text(
            b,
            [89; 32],
            "synthetic continued conversation",
            now + 1,
            now + 1,
        )
        .unwrap();
    assert_eq!(session, work::session(&id));
    alice.send_pending_online(session, now + 1).unwrap();
    let attempts = bob.receive_mailbox_online(now + 1).unwrap();
    assert!(attempts.into_iter().any(|a| matches!(a.result, Ok(MailboxEvent::Text(message)) if message.text().unwrap().body == "synthetic continued conversation")));
    // Advance only the local maintenance clock after network acceptance checks.
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
    assert_eq!(
        alice
            .reclaim_accepted_retry_controls(request.expires_at)
            .unwrap()
            .retired,
        0
    );
    assert_eq!(
        bob.reclaim_outgoing_retry_controls(request.expires_at)
            .unwrap()
            .retired,
        0
    );
}

#[test]
fn active_outgoing_quota_recovers_slots_beyond_the_old_tombstone_limit() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let mut failed = next(&bob);
    for n in 0..1024u64 {
        let mut message = [91; 32];
        message[..8].copy_from_slice(&n.to_be_bytes());
        failed.message_id = transport::hex(&message);
        bob.prepare_retry_request(&failed, now).unwrap();
    }
    failed.message_id = transport::hex(&[92; 32]);
    assert!(matches!(
        bob.prepare_retry_request(&failed, now),
        Err(Error::Limit)
    ));
    assert_eq!(count(&bob, "retry_outbox"), 1024);
    assert_eq!(
        bob.reclaim_outgoing_retry_controls(now + 604800)
            .unwrap()
            .retired,
        16
    );
    let live = bob.prepare_retry_request(&failed, now + 604800).unwrap();
    assert_eq!(count(&bob, "retry_outbox"), 1009);
    // Cross the previous compact-record cap with authenticated expiry tombstones.
    // Their IDs are separate from the full-control IDs under test.
    let own = device_fingerprint(&bob.own_device_binding().unwrap()).unwrap();
    let tx = bob.db.transaction().unwrap();
    for n in 0..4080u64 {
        let id: Id =
            Sha256::digest([b"synthetic quota tombstone".as_slice(), &n.to_be_bytes()].concat())
                .into();
        let mut bytes = own.to_vec();
        bytes.extend_from_slice(&(now + 604800).to_be_bytes());
        let state = bob
            .key
            .seal(&bytes, &binding(25, &id, b"outgoing retry retired"))
            .unwrap();
        tx.execute(
            "INSERT INTO retired_retry_outbox VALUES(?1,?2)",
            (id.as_slice(), state),
        )
        .unwrap();
    }
    tx.commit().unwrap();
    assert_eq!(count(&bob, "retired_retry_outbox"), 4096);
    let progress = bob.reclaim_outgoing_retry_controls(now + 604800).unwrap();
    assert_eq!(progress.scanned, 16);
    // The fresh, unexpired control may sort into this bounded batch.
    assert!((15..=16).contains(&progress.retired));
    assert_eq!(count(&bob, "retry_outbox"), 1009 - progress.retired as i64);
    assert_eq!(
        count(&bob, "retired_retry_outbox"),
        4096 + progress.retired as i64
    );
    assert_eq!(
        bob.prepare_retry_request(&failed, now + 604800).unwrap(),
        live
    );
}

#[test]
fn recovered_ack_requires_committed_proof_and_survives_lost_receipt() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let mut failed = next(&bob);
    bob.db.execute("UPDATE prekeys SET state=NULL", []).unwrap();
    let request = stage(&mut alice, &mut bob, &failed, now);
    let (session, _) = alice.resend_event(request.id, now).unwrap();
    alice.send_pending_online(session, now).unwrap();
    assert!(!bob.resolve_failed_delivery(&failed).unwrap());
    assert_eq!(bob.acknowledge_incoming_online().unwrap(), 0);
    let response = bob
        .connected_client()
        .unwrap()
        .mailbox()
        .unwrap()
        .into_iter()
        .find(|d| d.message_id == transport::hex(&request.id))
        .unwrap();
    bob.accept_delivery(&response).unwrap();
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON recovered_deliveries BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(bob.resolve_failed_delivery(&failed).is_err());
    assert_eq!(count(&bob, "recovered_deliveries"), 0);
    bob.db.execute_batch("DROP TRIGGER fail;").unwrap();
    assert!(bob.resolve_failed_delivery(&failed).unwrap());
    let packet = failed.payload.clone();
    failed.payload.replace_range(0..2, "00");
    assert!(matches!(
        bob.resolve_failed_delivery(&failed),
        Err(Error::Conflict)
    ));
    failed.payload = packet;
    failed.expires_at += 1;
    assert!(matches!(
        bob.resolve_failed_delivery(&failed),
        Err(Error::Conflict)
    ));
    failed.expires_at -= 1;
    let state: Vec<u8> = bob
        .db
        .query_row("SELECT state FROM recovered_deliveries", [], |r| r.get(0))
        .unwrap();
    bob.db
        .execute(
            "UPDATE recovered_deliveries SET state=zeroblob(length(state))",
            [],
        )
        .unwrap();
    assert!(bob.acknowledge_incoming_online().is_err());
    assert_eq!(bob.connected_client().unwrap().mailbox().unwrap().len(), 2);
    bob.db
        .execute("UPDATE recovered_deliveries SET state=?1", [state])
        .unwrap();
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON recovered_deliveries BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(bob.acknowledge_incoming_online().is_err());
    assert_eq!(bob.connected_client().unwrap().mailbox().unwrap().len(), 1);
    bob.db.execute_batch("DROP TRIGGER fail;").unwrap();
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    assert_eq!(bob.acknowledge_incoming_online().unwrap(), 2);
    assert_eq!(bob.acknowledge_incoming_online().unwrap(), 0);
    assert!(bob
        .connected_client()
        .unwrap()
        .mailbox()
        .unwrap()
        .is_empty());
    assert!(bob.resolve_failed_delivery(&failed).unwrap());
    assert_eq!(count(&bob, "sessions"), 1);
    assert_eq!(count(&bob, "inbox"), 1);
}

#[test]
fn recovered_journal_gc_preserves_pending_work_and_commits_with_cursor() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let failed = next(&bob);
    bob.db.execute("UPDATE prekeys SET state=NULL", []).unwrap();
    let request = stage(&mut alice, &mut bob, &failed, now);
    let (session, _) = alice.resend_event(request.id, now).unwrap();
    alice.send_pending_online(session, now).unwrap();
    let response = bob
        .connected_client()
        .unwrap()
        .mailbox()
        .unwrap()
        .into_iter()
        .find(|d| d.message_id == transport::hex(&request.id))
        .unwrap();
    bob.accept_delivery(&response).unwrap();
    bob.resolve_failed_delivery(&failed).unwrap();
    assert_eq!(
        bob.reclaim_recovered_journals(failed.expires_at)
            .unwrap()
            .reclaimed,
        0
    );
    assert_eq!(bob.reclaim_recovered_journals(now).unwrap().scanned, 0);
    bob.db
        .execute("UPDATE recovered_deliveries SET acknowledged=1", [])
        .unwrap();
    assert!(bob.reclaim_recovered_journals(failed.expires_at).is_err());
    assert_eq!(count(&bob, "recovered_deliveries"), 1);
    bob.db
        .execute("UPDATE recovered_deliveries SET acknowledged=0", [])
        .unwrap();
    assert_eq!(bob.acknowledge_incoming_online().unwrap(), 2);
    assert_eq!(bob.reclaim_recovered_journals(now).unwrap().reclaimed, 0);
    assert_eq!(bob.reclaim_recovered_journals(now).unwrap().scanned, 0);
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON retry_gc_cursor WHEN NEW.id=2 BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(bob.reclaim_recovered_journals(failed.expires_at).is_err());
    assert_eq!(count(&bob, "recovered_deliveries"), 1);
    bob.db.execute_batch("DROP TRIGGER fail;").unwrap();
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    assert_eq!(
        bob.reclaim_recovered_journals(failed.expires_at)
            .unwrap()
            .reclaimed,
        1
    );
    assert_eq!(count(&bob, "recovered_deliveries"), 0);
    assert_eq!(count(&bob, "inbox"), 1);
    assert_eq!(count(&bob, "retry_outbox"), 1);
    assert!(bob.resolve_failed_delivery(&failed).unwrap());
    assert_eq!(bob.acknowledge_incoming_online().unwrap(), 1);
    assert_eq!(count(&bob, "sessions"), 1);
    assert!(bob.accept_delivery(&response).unwrap().duplicate);
}

#[test]
fn recovered_journal_gc_is_bounded_and_cursors_are_independently_authenticated() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let mut failed = next(&bob);
    let request = stage(&mut alice, &mut bob, &failed, now);
    let (session, _) = alice.resend_event(request.id, now).unwrap();
    alice.send_pending_online(session, now).unwrap();
    let response = bob
        .connected_client()
        .unwrap()
        .mailbox()
        .unwrap()
        .into_iter()
        .find(|d| d.message_id == transport::hex(&request.id))
        .unwrap();
    bob.accept_delivery(&response).unwrap();
    // Synthetic extra delivery sequences exercise the local scan boundary.
    for n in 0..18 {
        failed.sequence = 100 + n;
        bob.resolve_failed_delivery(&failed).unwrap();
        if n >= 16 {
            // Seed authenticated local acknowledgement fixtures; these extra
            // sequences were not issued by the HTTPS server.
            let own = device_fingerprint(&bob.own_device_binding().unwrap()).unwrap();
            let aad = binding(27, &own, &failed.sequence.to_be_bytes());
            let sealed: Vec<u8> = bob
                .db
                .query_row(
                    "SELECT state FROM recovered_deliveries WHERE sequence=?1",
                    [failed.sequence],
                    |r| r.get(0),
                )
                .unwrap();
            let mut record: serde_json::Value =
                serde_json::from_slice(&bob.key.open(&sealed, &aad).unwrap()).unwrap();
            record["acknowledged"] = true.into();
            let sealed = bob
                .key
                .seal(&serde_json::to_vec(&record).unwrap(), &aad)
                .unwrap();
            bob.db
                .execute(
                    "UPDATE recovered_deliveries SET acknowledged=1,state=?1 WHERE sequence=?2",
                    (sealed, failed.sequence),
                )
                .unwrap();
        }
    }
    let first = bob.reclaim_recovered_journals(failed.expires_at).unwrap();
    assert_eq!((first.scanned, first.reclaimed), (16, 0));
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    let second = bob.reclaim_recovered_journals(failed.expires_at).unwrap();
    assert_eq!((second.scanned, second.reclaimed), (2, 2));
    assert_eq!(count(&bob, "recovered_deliveries"), 16);
    bob.reclaim_retry_journals(now).unwrap();
    bob.db.execute("UPDATE retry_gc_cursor SET state=(SELECT state FROM retry_gc_cursor WHERE id=1) WHERE id=2", []).unwrap();
    assert!(bob.reclaim_recovered_journals(failed.expires_at).is_err());
    assert_eq!(count(&bob, "recovered_deliveries"), 16);
}
