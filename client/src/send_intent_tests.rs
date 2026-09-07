use super::*;
use crate::{claims::tests::pair, incoming::tests::trust};
fn reopen(path: &Path) -> ClientStore {
    ClientStore::open(
        path,
        StorageKey::new(sigil_crypto::Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap()
}
fn pump(alice: &mut ClientStore, now: u64) {
    for _ in 0..3 {
        let step = alice.sync_step_online(now);
        assert!(step.failure.is_none());
        for send in step.sends {
            assert!(send.result.is_ok(), "{:?}", send.result);
        }
    }
}

#[test]
fn expired_claim_or_ambiguous_week_old_attempt_advances_without_rebinding_text() {
    for expired_receipt in [false, true] {
        let (dir, _fixture, mut alice, mut bob, now) = pair();
        let (_, b) = trust(&mut alice, &mut bob);
        let message = [88; 32];
        alice
            .queue_peer_text(
                b,
                message,
                "claim renewal",
                now,
                if expired_receipt { now } else { now - 604800 },
            )
            .unwrap();
        let own = device_fingerprint(&alice.own_device_binding().unwrap()).unwrap();
        let claim = work_id(&own, &message, 0, b"claim");
        alice.prepare_peer_claim(claim, b).unwrap();
        alice.claim_prekey_online(claim, now).unwrap();
        if expired_receipt {
            let identity = alice.identity().unwrap();
            let sealed: Vec<u8> = alice
                .db
                .query_row(
                    "SELECT state FROM prekey_claims WHERE id=?1",
                    [claim.as_slice()],
                    |r| r.get(0),
                )
                .unwrap();
            let aad = binding(13, &claim, &identity);
            let raw = alice.key.open(&sealed, &aad).unwrap();
            let mut value: serde_json::Value = serde_json::from_slice(&raw).unwrap();
            value["phase"]["Ready"]["expires_at"] = now.into();
            alice
                .db
                .execute(
                    "UPDATE prekey_claims SET state=?1 WHERE id=?2",
                    (
                        alice
                            .key
                            .seal(&serde_json::to_vec(&value).unwrap(), &aad)
                            .unwrap(),
                        claim.as_slice(),
                    ),
                )
                .unwrap();
            assert!(matches!(
                alice.prepare_send_intent_online(message, now),
                Err(Error::Expired)
            ));
        }
        bob.prepare_prekey_publication([89; 32], true, 3600)
            .unwrap();
        bob.publish_prekey_online([89; 32]).unwrap();
        drop(alice);
        let mut alice = reopen(&dir.path().join("alice.db"));
        pump(&mut alice, now);
        let incoming = bob.receive_mailbox_online(now).unwrap();
        assert!(incoming.iter().any(|v|matches!(&v.result,Ok(MailboxEvent::Text(text)) if text.text().unwrap().body=="claim renewal")));
        assert!(matches!(
            alice.claim_prekey_online(claim, now),
            Err(Error::AlreadyDelivered)
        ));
    }
}

#[test]
fn first_send_resumes_exact_claim_and_packet_after_failed_local_commits() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_, b) = trust(&mut alice, &mut bob);
    alice
        .queue_peer_text(b, [0; 32], "first intent", now, now)
        .unwrap();
    assert!(matches!(
        alice.queue_peer_text(b, [0; 32], "changed", now, now),
        Err(Error::Conflict)
    ));
    alice.db.execute_batch("CREATE TRIGGER fail_claim BEFORE UPDATE ON prekey_claims BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    let failed = alice.sync_step_online(now);
    assert!(matches!(failed.sends[0].result, Err(Error::Storage(_))));
    assert_eq!(
        alice
            .db
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    alice.db.execute_batch("DROP TRIGGER fail_claim;").unwrap();
    drop(alice);
    let mut alice = reopen(&dir.path().join("alice.db"));
    alice.db.execute_batch("CREATE TRIGGER fail_finish BEFORE DELETE ON send_intents BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    // First pass wraps the durable cursor. Second creates the session, but fails
    // intent removal after packet commit; ordinary sending still uses that packet.
    assert!(alice.sync_step_online(now).sends.is_empty());
    let failed = alice.sync_step_online(now);
    assert!(matches!(failed.sends[0].result, Err(Error::Storage(_))));
    assert_eq!(failed.outbound[0].result.as_ref().unwrap().accepted, 1);
    let session = alice.active_session(b).unwrap().unwrap();
    let receipt = alice.delivery_receipt(session, [0; 32]).unwrap().unwrap();
    alice.db.execute_batch("DROP TRIGGER fail_finish;").unwrap();
    drop(alice);
    let mut alice = reopen(&dir.path().join("alice.db"));
    pump(&mut alice, now);
    alice
        .queue_peer_text(b, [0; 32], "first intent", now, now)
        .unwrap();
    assert_eq!(
        alice.delivery_receipt(session, [0; 32]).unwrap().unwrap(),
        receipt
    );
    let received = bob.receive_mailbox_online(now).unwrap();
    assert_eq!(received.len(), 1);
    assert!(
        matches!(&received[0].result,Ok(MailboxEvent::Text(t)) if t.text().unwrap().body=="first intent")
    );
    assert_eq!(
        alice
            .db
            .query_row("SELECT count(*) FROM prekey_claims", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn expired_selection_starts_fresh_without_moving_old_packet_and_checks_queued_trust() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (_, b) = trust(&mut alice, &mut bob);
    alice.prepare_peer_claim([1; 32], b).unwrap();
    alice.claim_prekey_online([1; 32], now).unwrap();
    alice
        .start_claimed_text(
            [1; 32],
            [3; 32],
            [4; 32],
            "old frozen initial",
            now - 604800,
            now - 604800,
        )
        .unwrap();
    let frozen = alice.pending([3; 32]).unwrap();
    bob.prepare_prekey_publication([80; 32], true, 3600)
        .unwrap();
    bob.publish_prekey_online([80; 32]).unwrap();
    alice
        .queue_peer_text(b, [5; 32], "new session automatically", now, now)
        .unwrap();
    alice.block_peer(b, true).unwrap();
    assert!(alice.resume_send_intents_online(now).unwrap()[0]
        .result
        .is_err());
    assert!(alice.pending([3; 32]).is_err());
    alice.block_peer(b, false).unwrap();
    assert_eq!(alice.pending([3; 32]).unwrap(), frozen);
    assert!(alice.resume_send_intents_online(now).unwrap().is_empty());
    let next = alice.resume_send_intents_online(now).unwrap()[0]
        .result
        .as_ref()
        .copied()
        .unwrap();
    assert_ne!(next, [3; 32]);
    assert_eq!(alice.pending([3; 32]).unwrap(), frozen);
    alice.send_pending_online(next, now).unwrap();
    assert!(
        matches!(&bob.receive_mailbox_online(now).unwrap()[0].result,Ok(MailboxEvent::Text(t)) if t.text().unwrap().body=="new session automatically")
    );
}

#[test]
fn missing_stock_waits_across_restart_and_preparation_failure_leaves_no_intent() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_, b) = trust(&mut alice, &mut bob);
    alice.db.execute_batch("CREATE TRIGGER fail_intent BEFORE INSERT ON send_intents BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(alice
        .queue_peer_text(b, [9; 32], "waiting for stock", now, now)
        .is_err());
    assert_eq!(
        alice
            .db
            .query_row("SELECT count(*) FROM send_intents", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    alice.db.execute_batch("DROP TRIGGER fail_intent;").unwrap();
    alice
        .queue_peer_text(b, [9; 32], "waiting for stock", now, now)
        .unwrap();
    let server = Connection::open(dir.path().join("server.db")).unwrap();
    server
        .execute("UPDATE prekeys SET bundle=NULL", [])
        .unwrap();
    let failed = alice.sync_step_online(now);
    assert!(matches!(
        failed.failure,
        Some(SyncFailure::SendIntentNetwork(0))
    ));
    drop(alice);
    let mut alice = reopen(&dir.path().join("alice.db"));
    bob.replenish_prekey_online().unwrap();
    pump(&mut alice, now);
    assert!(
        matches!(&bob.receive_mailbox_online(now).unwrap()[0].result,Ok(MailboxEvent::Text(t)) if t.text().unwrap().body=="waiting for stock")
    );
}
