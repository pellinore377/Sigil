use super::*;
use crate::{claims::tests::pair, incoming::tests::trust};
use sigil_crypto::IdentityKey;
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
    // The cursor wraps within the pass, which creates the session but fails
    // intent removal after packet commit; ordinary sending still uses that packet.
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
#[test]
fn queued_direct_text_is_wake_worthy() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (_, b) = trust(&mut alice, &mut bob);
    alice.queue_peer_text(b, [7; 32], "wake me", now, now).unwrap();
    let step = alice.sync_step_online(now);
    assert!(step.failure.is_none());
    let wake = |alice: &ClientStore, id: [u8; 32]| -> Vec<bool> {
        alice
            .db
            .prepare("SELECT wake FROM outbox WHERE id=?1")
            .unwrap()
            .query_map([id.as_slice()], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(wake(&alice, [7; 32]), vec![true]);
    // Conversation posts (the mobile and browser send path) wake; receipts stay silent.
    let peer_hex = crate::transport::hex(&b);
    let post: serde_json::Value = serde_json::from_str(&alice.mobile_command(&format!(
        r#"{{"command":"post","peer":"{peer_hex}","request":"{}","text":"conversation post","timestamp":{now}}}"#,
        crate::transport::hex(&[8; 32])
    )))
    .unwrap();
    assert_eq!(post["ok"], true, "{post}");
    // The post command stamps intents with the wall clock.
    let step = alice.sync_step_online(now.max(crate::conversations::now()) + 1);
    assert!(step.failure.is_none());
    let posted: Vec<bool> = alice
        .db
        .prepare("SELECT wake FROM outbox WHERE id<>x'0707070707070707070707070707070707070707070707070707070707070707'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(posted, vec![true]);
}
#[test]
fn intent_row_consumed_before_prepare_is_not_reported_as_local_missing() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (_, b) = trust(&mut alice, &mut bob);
    alice.queue_peer_text(b, [1; 32], "first", now, now).unwrap();
    alice.queue_peer_text(b, [2; 32], "second", now, now).unwrap();
    // Simulate another pass consuming the second row after this pass selected it.
    alice.db.execute_batch("CREATE TRIGGER consume AFTER DELETE ON send_intents WHEN old.id=x'0101010101010101010101010101010101010101010101010101010101010101' BEGIN DELETE FROM send_intents WHERE id=x'0202020202020202020202020202020202020202020202020202020202020202'; END;").unwrap();
    let step = alice.sync_step_online(now);
    assert!(step.failure.is_none());
    assert_eq!(step.sends.len(), 2);
    assert!(step.sends[0].result.is_ok());
    // The row was consumed by the first send's trigger; a gone row is Obsolete, not missing.
    assert!(matches!(step.sends[1].result, Err(Error::Obsolete)), "{:?}", step.sends[1].result);
    assert!(step.issue().is_none(), "{:?}", step.issue());
}
#[test]
fn intent_to_replaced_peer_waits_untrusted_then_retargets_to_trusted_replacement() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let old_bytes = bob.own_device_binding().unwrap();
    let old = alice.observe_peer_binding(&old_bytes).unwrap();
    alice.confirm_peer(old.id, old.fingerprint).unwrap();
    crate::incoming::tests::start(&mut alice, old.id, now);
    alice
        .queue_peer_text(old.id, [5; 32], "to replacement", now, now)
        .unwrap();
    let key = IdentityKey::generate().unwrap();
    let mut replacement = crate::peers::parse(&old_bytes).unwrap();
    replacement.binding.device = [90; 32];
    replacement.binding.identity = key.public_key();
    replacement.signature = key
        .sign(&replacement.binding.signing_bytes().unwrap())
        .unwrap();
    let new = alice
        .observe_peer_binding(&replacement.to_bytes().unwrap())
        .unwrap();
    alice
        .approve_peer_replacement(old.id, new.id, old.fingerprint, new.fingerprint)
        .unwrap();
    let own = device_fingerprint(&alice.own_device_binding().unwrap()).unwrap();
    let target = |a: &ClientStore| read(&a.db, &a.key, &own, &[5; 32]).unwrap().0.peer;
    let retired = |a: &ClientStore| {
        a.db.query_row(
            "SELECT retired FROM sessions WHERE id=?1",
            [[3u8; 32].as_slice()],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    // Approval itself retires the old device's session; the intent survives it.
    assert_eq!(retired(&alice), 1);
    // Replacement not yet trusted: wait, never re-point, never cancel.
    alice.block_peer(new.id, true).unwrap();
    let waited = alice.resume_send_intents_online(now).unwrap();
    assert!(matches!(waited[0].result, Err(Error::Unprepared)));
    assert_eq!(target(&alice), old.id);
    drop(waited);
    // Trusted replacement: re-point the intent to the new device.
    alice.block_peer(new.id, false).unwrap();
    let _ = alice.resume_send_intents_online(now);
    assert_eq!(target(&alice), new.id);
    assert_eq!(retired(&alice), 1);
    let _ = dir;
}
#[test]
fn fresh_intent_precedes_due_backlog_in_the_same_pass() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (_, b) = trust(&mut alice, &mut bob);
    bob.prepare_prekey_publication([0x81; 32], true, 3600).unwrap();
    bob.publish_prekey_online([0x81; 32]).unwrap();
    let own = device_fingerprint(&alice.own_device_binding().unwrap()).unwrap();
    // Cursor parked mid-range with a due backlog intent ahead of it.
    let sealed = alice
        .key
        .seal(&[0x80; 32], &binding(37, &own, b"send intents"))
        .unwrap();
    alice
        .db
        .execute("INSERT INTO send_intent_cursor VALUES(1,?1)", [sealed])
        .unwrap();
    alice.queue_peer_text(b, [0x90; 32], "backlog", now, now).unwrap();
    alice
        .db
        .execute(
            "INSERT INTO send_intent_backoff VALUES(?1,?2)",
            ([0x90u8; 32].as_slice(), now as i64),
        )
        .unwrap();
    alice.queue_peer_text(b, [0x10; 32], "fresh", now, now).unwrap();
    let step = alice.sync_step_online(now);
    assert!(step.failure.is_none(), "{:?}", step.failure);
    assert_eq!(step.sends[0].id, [0x10; 32], "fresh intent must go first");
    assert!(step.sends.iter().any(|s| s.id == [0x10; 32] && s.result.is_ok()));
    assert!(step
        .timings
        .iter()
        .any(|(name, _)| *name == "resume_send_intents_online"));
}

/// A cancelled intent must not keep holding its prekey claim: pending claims are
/// capped at 64, and every dead one starves a later new-session send.
#[test]
fn a_cancelled_intent_releases_its_pending_prekey_claim() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    // A trusted binding for a device the server does not stock keeps every claim pending.
    let key = IdentityKey::generate().unwrap();
    let mut forged = crate::peers::parse(&bob.own_device_binding().unwrap()).unwrap();
    forged.binding.device = [90; 32];
    forged.binding.identity = key.public_key();
    forged.signature = key
        .sign(&forged.binding.signing_bytes().unwrap())
        .unwrap();
    let peer = alice
        .observe_peer_binding(&forged.to_bytes().unwrap())
        .unwrap();
    alice.confirm_peer(peer.id, peer.fingerprint).unwrap();
    let pending = |store: &ClientStore| -> i64 {
        store
            .db
            .query_row("SELECT count(*) FROM prekey_claims WHERE phase=0", [], |r| r.get(0))
            .unwrap()
    };
    let intents = |store: &ClientStore| -> i64 {
        store
            .db
            .query_row("SELECT count(*) FROM send_intents", [], |r| r.get(0))
            .unwrap()
    };
    let op = alice
        .conversation_operation(
            [86; 32],
            sigil_protocol::conversation::Action::Typing {
                active: true,
                until: now + 30,
            },
        )
        .unwrap();
    alice.queue_peer_operation(peer.id, &op, now, now).unwrap();
    let first = alice.resume_send_intents_online(now).unwrap();
    assert_eq!(first.len(), 1);
    assert!(first[0].result.is_err());
    assert_eq!((intents(&alice), pending(&alice)), (1, 1));
    // The notice lapses, the intent is dropped, and its claim goes with it.
    let second = alice.resume_send_intents_online(now + 61).unwrap();
    assert!(matches!(second[0].result, Err(Error::Obsolete)));
    assert_eq!((intents(&alice), pending(&alice)), (0, 0));
}
