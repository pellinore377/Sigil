use super::*;
use crate::{
    claims::tests::pair,
    incoming::tests::{next, trust},
};
use sigil_crypto::{
    recovery::{Content, Direction},
    Secret32,
};
fn account_id(client: &ClientStore) -> Id {
    crate::connection::decode_id(&client.connection_session().unwrap().unwrap().account_id).unwrap()
}
fn configure(client: &mut ClientStore, secret: u8) {
    client
        .configure_recovery(
            "chat.example",
            account_id(client),
            Secret32::from_bytes([secret; 32]),
        )
        .unwrap();
}
fn count(client: &ClientStore, table: &str) -> i64 {
    client
        .db
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}
fn fail_archive(client: &ClientStore, fail: bool) {
    client.db.execute_batch(if fail { "CREATE TRIGGER fail BEFORE INSERT ON archive_records BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;" } else { "DROP TRIGGER fail;" }).unwrap();
}
fn body(client: &ClientStore, id: Id) -> Zeroizing<Vec<u8>> {
    match client.recovery_record(id).unwrap().content {
        Content::Retained(v) => v,
        _ => panic!("unexpected non-text recovery content"),
    }
}
fn live_state(client: &ClientStore) -> Vec<u8> {
    client
        .db
        .query_row("SELECT state FROM sessions", [], |r| r.get(0))
        .unwrap()
}
#[test]
fn accepted_text_is_archived_atomically_and_a_fresh_device_recovers_it_over_https() {
    let (dir, fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    configure(&mut alice, 7);
    configure(&mut bob, 8);
    alice.prepare_peer_claim([1; 32], b).unwrap();
    alice.claim_prekey_online([1; 32], now).unwrap();
    fail_archive(&alice, true);
    assert!(alice
        .start_claimed_text([1; 32], [3; 32], [4; 32], "recoverable initial", now, now)
        .is_err());
    for table in [
        "sessions",
        "initiations",
        "outbox",
        "deliveries",
        "archive_records",
    ] {
        assert_eq!(count(&alice, table), 0, "{table}");
    }
    fail_archive(&alice, false);
    alice
        .start_claimed_text([1; 32], [3; 32], [4; 32], "recoverable initial", now, now)
        .unwrap();
    assert_eq!(count(&alice, "archive_records"), 1);
    let first_head = alice.prepare_recovery_upload(now).unwrap();
    alice.send_pending_online([3; 32], now).unwrap();
    let initial = next(&bob);
    fail_archive(&bob, true);
    assert!(bob.accept_delivery(&initial).is_err());
    for table in [
        "sessions",
        "inbox",
        "incoming",
        "text_events",
        "archive_records",
    ] {
        assert_eq!(count(&bob, table), 0, "{table}");
    }
    assert_eq!(
        bob.db
            .query_row("SELECT count(state) FROM prekeys", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(bob.acknowledge_incoming_online().unwrap(), 0);
    fail_archive(&bob, false);
    let incoming = bob.accept_delivery(&initial).unwrap();
    let first_id = text_history_id(&incoming.text().unwrap(), &alice.identity().unwrap());
    assert_eq!(body(&alice, first_id).as_slice(), b"recoverable initial");
    assert_eq!(body(&bob, first_id).as_slice(), b"recoverable initial");
    assert_eq!(
        alice.recovery_record(first_id).unwrap().direction,
        Direction::Outgoing
    );
    assert_eq!(
        bob.recovery_record(first_id).unwrap().direction,
        Direction::Incoming
    );
    let before = live_state(&bob);
    fail_archive(&bob, true);
    assert!(bob
        .send_text(incoming.session, [5; 32], "recoverable reply", now, now)
        .is_err());
    assert_eq!(live_state(&bob), before);
    assert_eq!(count(&bob, "outbox"), 0);
    fail_archive(&bob, false);
    bob.send_text(incoming.session, [5; 32], "recoverable reply", now, now)
        .unwrap();
    bob.send_pending_online(incoming.session, now).unwrap();
    let reply = next(&alice);
    let before = live_state(&alice);
    fail_archive(&alice, true);
    assert!(alice.accept_delivery(&reply).is_err());
    assert_eq!(live_state(&alice), before);
    assert_eq!(count(&alice, "inbox"), 0);
    fail_archive(&alice, false);
    let received = alice.accept_delivery(&reply).unwrap();
    let second_id = text_history_id(&received.text().unwrap(), &bob.identity().unwrap());
    assert_eq!(count(&alice, "archive_records"), 2);
    assert_eq!(body(&alice, second_id).as_slice(), b"recoverable reply");
    // Newly accepted text does not alter the already frozen upload snapshot.
    assert_eq!(alice.upload_recovery_step().unwrap(), Some(first_head));
    let latest = alice.prepare_recovery_upload(now + 1).unwrap();
    assert_eq!(latest.generation, first_head.generation + 1);
    assert_eq!(alice.upload_recovery_step().unwrap(), Some(latest));
    let original = alice.connection_session().unwrap().unwrap();
    let mut server = sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap();
    let invite = server
        .invite_reauthorization(&original.account_id, 60, now)
        .unwrap();
    let path = dir.path().join("restored.db");
    let open = || {
        ClientStore::open(
            &path,
            StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
        )
        .unwrap()
    };
    let mut replacement = open();
    replacement
        .prepare_enrollment(
            "chat.example",
            fixture.port(),
            &[crate::network::tests::CA.to_vec()],
            &invite.secret,
            "Synthetic recovered device",
            true,
        )
        .unwrap();
    let enrolled = replacement.enroll_online().unwrap();
    assert_eq!(enrolled.account_id, original.account_id);
    assert_ne!(enrolled.device_id, original.device_id);
    configure(&mut replacement, 7);
    let mut complete = false;
    for _ in 0..8 {
        if replacement.download_recovery_step(true).unwrap() == Some(latest) {
            complete = true;
            break;
        }
        assert!(replacement.recovery_records(None).unwrap().is_empty());
        drop(replacement);
        replacement = open();
    }
    assert!(complete);
    let discovered = replacement.recovery_records(None).unwrap();
    assert_eq!(discovered.len(), 2);
    for id in [first_id, second_id] {
        assert!(discovered.iter().any(|record| record.id == id));
    }
    assert_eq!(
        body(&replacement, first_id).as_slice(),
        b"recoverable initial"
    );
    assert_eq!(
        body(&replacement, second_id).as_slice(),
        b"recoverable reply"
    );
    for table in ["sessions", "identity", "prekeys", "peers", "text_events"] {
        assert_eq!(count(&replacement, table), 0, "{table}");
    }
}

#[test]
fn automatic_capture_preserves_archival_tombstones_and_rejects_wrong_account_scope() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    configure(&mut alice, 7);
    alice.prepare_peer_claim([1; 32], b).unwrap();
    alice.claim_prekey_online([1; 32], now).unwrap();
    let packet = alice
        .start_claimed_text([1; 32], [3; 32], [4; 32], "ordinary text", now, now)
        .unwrap();
    let plaintext = alice.outgoing_message([3; 32], [4; 32]).unwrap();
    let event = Text::from_bytes(&plaintext).unwrap();
    let id = text_history_id(&event, &alice.identity().unwrap());
    let mut record = alice.recovery_record(id).unwrap();
    record.revision = 2;
    record.content = Content::Deleted;
    alice.retain_recovery_record(&record).unwrap();
    assert_eq!(
        alice
            .start_claimed_text([1; 32], [3; 32], [4; 32], "ordinary text", now, now + 1)
            .unwrap(),
        packet
    );
    assert!(matches!(
        alice.recovery_record(id).unwrap().content,
        Content::Deleted
    ));
    alice.send_pending_online([3; 32], now).unwrap();
    bob.configure_recovery(
        "wrong.example",
        account_id(&bob),
        Secret32::from_bytes([8; 32]),
    )
    .unwrap();
    assert!(matches!(
        bob.accept_delivery(&next(&bob)),
        Err(Error::Conflict)
    ));
    assert_eq!(count(&bob, "sessions"), 0);
    assert_eq!(count(&bob, "archive_records"), 0);
    assert_eq!(bob.acknowledge_incoming_online().unwrap(), 0);
}
