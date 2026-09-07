use super::*;
use crate::connection::tests::{prepare, setup};
use sigil_crypto::{handshake::initiate_session, IdentityKey, Secret32};

const SLOT: Id = [1; 32];

#[test]
fn online_retirement_defers_unread_initial_and_sync_processes_it_before_cleanup() {
    let (dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    let (a, _) = crate::incoming::tests::trust(&mut alice, &mut bob);
    alice.prepare_prekey_publication(SLOT, true, 3600).unwrap();
    // Independent clock origins make this key locally overdue while the server
    // still has a live delivery. The empty-mailbox guard must remain authoritative.
    alice.publish_prekey_with_clock(SLOT, || Ok(1000)).unwrap();
    bob.prepare_peer_claim([91; 32], a).unwrap();
    bob.claim_prekey_online([91; 32], now).unwrap();
    bob.start_claimed_text(
        [91; 32],
        [92; 32],
        [93; 32],
        "delayed initial survives cleanup",
        now,
        now,
    )
    .unwrap();
    bob.send_pending_online([92; 32], now).unwrap();
    alice
        .prepare_prekey_publication([2; 32], true, 3600)
        .unwrap();
    alice
        .publish_prekey_with_clock([2; 32], || Ok(1000))
        .unwrap();
    assert_eq!(alice.maintain_sessions(now).unwrap().prekeys_retired, 0);
    assert_eq!(
        alice.maintain_sessions_online(now).unwrap().prekeys_retired,
        0
    );
    assert!(live(&alice.db, &SLOT).unwrap());
    assert!(live(&alice.db, &[2; 32]).unwrap());
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    let step = alice.sync_step_online(now);
    assert!(step.failure.is_none());
    assert!(
        matches!(&step.incoming[0].result,Ok(MailboxEvent::Text(t)) if t.text().unwrap().body == "delayed initial survives cleanup")
    );
    assert_eq!(step.acknowledged, 1);
    assert_eq!(step.maintenance.unwrap().prekeys_retired, 1);
    assert!(!live(&alice.db, &SLOT).unwrap());
    assert!(!live(&alice.db, &[2; 32]).unwrap());
    assert!(alice
        .connected_client()
        .unwrap()
        .mailbox()
        .unwrap()
        .is_empty());
    assert!(matches!(
        alice.create_prekey([2; 32], true),
        Err(Error::AlreadyDelivered)
    ));
}

#[test]
fn scheduled_prekey_cleanup_rolls_back_with_maintenance_and_resumes_bounded_batches() {
    let (dir, _fixture, mut alice, _bob, now) = crate::claims::tests::pair();
    for n in 0..17u8 {
        alice
            .prepare_prekey_publication([n; 32], true, 3600)
            .unwrap();
        alice
            .publish_prekey_with_clock([n; 32], || Ok(1000))
            .unwrap();
    }
    alice.db.execute_batch("CREATE TRIGGER fail_second_retirement BEFORE UPDATE ON prekeys WHEN NEW.state IS NULL AND (SELECT count(*) FROM prekeys WHERE state IS NULL)=1 BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    let failed = alice.sync_step_online(now);
    assert!(matches!(
        failed.failure,
        Some(SyncFailure::Maintenance(Error::Storage(_)))
    ));
    assert!(failed.maintenance.is_none());
    assert_eq!(
        alice
            .db
            .query_row("SELECT count(state) FROM prekeys", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        17
    );
    assert_eq!(
        alice
            .db
            .query_row("SELECT count(*) FROM session_maintenance", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let identity = alice.identity().unwrap();
    for n in 0..17u8 {
        assert!(
            !load(&alice.db, &alice.key, &[n; 32], &identity)
                .unwrap()
                .0
                .retired
        );
    }
    alice
        .db
        .execute_batch("DROP TRIGGER fail_second_retirement;")
        .unwrap();
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    let first = alice.sync_step_online(now);
    assert!(first.failure.is_none());
    assert_eq!(first.maintenance.unwrap().prekeys_retired, 16);
    // The existing sealed maintenance clock also protects automatic prekey cleanup.
    assert!(matches!(
        alice.maintain_sessions_online(now - 1),
        Err(Error::Expired)
    ));
    assert_eq!(
        alice
            .db
            .query_row("SELECT count(state) FROM prekeys", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    assert_eq!(
        alice
            .sync_step_online(now)
            .maintenance
            .unwrap()
            .prekeys_retired,
        1
    );
    assert_eq!(
        alice
            .sync_step_online(now)
            .maintenance
            .unwrap()
            .prekeys_retired,
        0
    );
    assert_eq!(
        alice
            .db
            .query_row("SELECT count(*) FROM prekeys", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        17
    );
}
fn open(path: &Path) -> ClientStore {
    ClientStore::open(
        path,
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap()
}

#[test]
fn publication_retries_original_server_commit_and_accepts_delayed_initial() {
    let (dir, fixture, invitation, _) = setup();
    let path = dir.path().join("client.db");
    let mut bob = open(&path);
    prepare(&mut bob, &fixture, &invitation.secret);
    let account = bob.enroll_online().unwrap();
    let id = bob.prepare_prekey_publication(SLOT, true, 3600).unwrap();
    let identity = bob.identity().unwrap();
    let before = load(&bob.db, &bob.key, &SLOT, &identity).unwrap().2;
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON prekey_publications BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(bob.publish_prekey_online(SLOT).is_err());
    assert_eq!(before, load(&bob.db, &bob.key, &SLOT, &identity).unwrap().2);
    let server = rusqlite::Connection::open(dir.path().join("server.db")).unwrap();
    let expiry: i64 = server
        .query_row(
            "SELECT expires_at FROM prekeys WHERE id=?1",
            [transport::hex(&id)],
            |r| r.get(0),
        )
        .unwrap();
    bob.db.execute_batch("DROP TRIGGER fail;").unwrap();
    drop(bob);
    let mut bob = open(&path);
    let receipt = bob.publish_prekey_with_clock(SLOT, || Ok(1000)).unwrap();
    assert_eq!(receipt.expires_at, expiry as u64);
    let deadline = 1000 + 3600 + DELIVERY_LIFETIME;
    assert_eq!(bob.publish_prekey_online(SLOT).unwrap(), receipt);
    assert_eq!(
        bob.prepare_prekey_publication(SLOT, true, 3600).unwrap(),
        id
    );
    assert!(matches!(
        bob.prepare_prekey_publication(SLOT, true, 7200),
        Err(Error::Conflict)
    ));
    assert_eq!(
        server
            .query_row("SELECT count(*) FROM prekeys", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    let claimed = bob
        .connected_client()
        .unwrap()
        .claim_prekey(&account.device_id, &transport::hex(&[3; 32]))
        .unwrap();
    let bundle = Bundle::from_bytes(&decode(&claimed.bundle).unwrap(), &identity).unwrap();
    assert_eq!(transport::hex(&bundle.prekey_id()), claimed.prekey_id);
    let sender = IdentityKey::generate().unwrap();
    let (mut alice, initial) = initiate_session(&sender, &identity, &bundle, b"").unwrap();
    let packet = sigil_protocol::initial::encode(
        &initial.to_bytes(),
        &alice.send(b"delayed synthetic initial").unwrap().to_bytes(),
    )
    .unwrap();
    assert_eq!(bob.retire_prekeys(1000 + 3600).unwrap(), 0);
    assert_eq!(bob.retire_prekeys(deadline - 1).unwrap(), 0);
    assert_eq!(bob.initial_prekey_slot(&packet).unwrap(), SLOT);
    bob.accept_initial(SLOT, [2; 32], [3; 32], sender.public_key(), &packet)
        .unwrap();
    assert_eq!(bob.initial_prekey_slot(&packet).unwrap(), SLOT);
    let reply = bob.send([2; 32], [4; 32], b"reply").unwrap();
    assert_eq!(
        alice.receive(&Packet::from_bytes(&reply).unwrap()).unwrap(),
        b"reply"
    );
    assert_eq!(bob.retire_prekeys(deadline).unwrap(), 0);
    assert!(!live(&bob.db, &SLOT).unwrap());
}

#[test]
fn retirement_authenticates_deadline_and_rolls_back_with_private_key() {
    let (dir, fixture, invitation, _) = setup();
    let mut store = open(&dir.path().join("client.db"));
    prepare(&mut store, &fixture, &invitation.secret);
    store.enroll_online().unwrap();
    store.prepare_prekey_publication(SLOT, false, 3600).unwrap();
    let expiry = store
        .publish_prekey_with_clock(SLOT, || Ok(1000))
        .unwrap()
        .expires_at;
    let deadline = 1000 + 3600 + DELIVERY_LIFETIME;
    let identity = store.identity().unwrap();
    let before = load(&store.db, &store.key, &SLOT, &identity).unwrap().2;
    store
        .db
        .execute("UPDATE prekey_publications SET retire_at=1000", [])
        .unwrap();
    assert!(store.retire_prekeys(1000).is_err());
    assert!(live(&store.db, &SLOT).unwrap());
    store
        .db
        .execute(
            "UPDATE prekey_publications SET retire_at=?1",
            [deadline as i64],
        )
        .unwrap();
    store.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON prekeys BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(store.retire_prekeys(deadline).is_err());
    assert!(live(&store.db, &SLOT).unwrap());
    assert_eq!(
        load(&store.db, &store.key, &SLOT, &identity).unwrap().2,
        before
    );
    store.db.execute_batch("DROP TRIGGER fail;").unwrap();
    assert_eq!(store.retire_prekeys(deadline).unwrap(), 1);
    assert_eq!(store.retire_prekeys(deadline).unwrap(), 0);
    assert!(!live(&store.db, &SLOT).unwrap());
    assert!(matches!(
        store.create_prekey(SLOT, false),
        Err(Error::AlreadyDelivered)
    ));
    assert_eq!(
        store.publish_prekey_online(SLOT).unwrap().expires_at,
        expiry
    );
}

#[test]
fn failed_public_metadata_commit_rolls_back_private_slot_and_migrates_schema_nine() {
    let (dir, fixture, invitation, _) = setup();
    let path = dir.path().join("client.db");
    let mut store = open(&path);
    prepare(&mut store, &fixture, &invitation.secret);
    store.enroll_online().unwrap();
    crate::test_schema::rewind(&store.db, 9);
    drop(store);
    let mut store = open(&path);
    assert_eq!(
        store
            .db
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        55
    );
    store.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON prekey_publications BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(store.prepare_prekey_publication(SLOT, true, 3600).is_err());
    assert!(!live(&store.db, &SLOT).unwrap());
    let identity = store.identity().unwrap();
    let bundle = store.create_prekey(SLOT, true).unwrap();
    store.db.execute_batch("DROP TRIGGER fail;").unwrap();
    assert_eq!(
        store.prepare_prekey_publication(SLOT, true, 3600).unwrap(),
        Bundle::from_bytes(&bundle, &identity).unwrap().prekey_id()
    );
    let (publication, _, _) = load(&store.db, &store.key, &SLOT, &identity).unwrap();
    assert_eq!(decode(&publication.bundle).unwrap(), bundle);
    assert!(publication.expiry.is_none());
    assert!(store.retire_prekeys(i64::MAX as u64).unwrap() == 0);
}

#[test]
fn retirement_is_bounded_and_concurrent_preparation_freezes_one_lifetime() {
    let (dir, fixture, invitation, _) = setup();
    let path = dir.path().join("client.db");
    let mut store = open(&path);
    prepare(&mut store, &fixture, &invitation.secret);
    store.enroll_online().unwrap();
    let barrier = std::sync::Barrier::new(2);
    let results = std::thread::scope(|scope| {
        let run = |lifetime| {
            let mut store = open(&path);
            barrier.wait();
            store.prepare_prekey_publication(SLOT, true, lifetime)
        };
        let a = scope.spawn(move || run(3600));
        let b = scope.spawn(move || run(7200));
        [a.join().unwrap(), b.join().unwrap()]
    });
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert!(results.iter().any(|r| matches!(r, Err(Error::Conflict))));
    store.publish_prekey_with_clock(SLOT, || Ok(1000)).unwrap();
    for n in 2..=17 {
        store
            .prepare_prekey_publication([n; 32], false, 3600)
            .unwrap();
        store
            .publish_prekey_with_clock([n; 32], || Ok(1000))
            .unwrap();
    }
    assert_eq!(
        store
            .retire_prekeys(1000 + 7200 + DELIVERY_LIFETIME)
            .unwrap(),
        16
    );
    assert_eq!(
        store
            .retire_prekeys(1000 + 7200 + DELIVERY_LIFETIME)
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .retire_prekeys(1000 + 7200 + DELIVERY_LIFETIME)
            .unwrap(),
        0
    );
}

#[test]
fn schema_eleven_retention_requires_a_new_local_deadline_not_the_remote_clock() {
    let (dir, fixture, invitation, _) = setup();
    let path = dir.path().join("client.db");
    let mut store = open(&path);
    prepare(&mut store, &fixture, &invitation.secret);
    store.enroll_online().unwrap();
    store.prepare_prekey_publication(SLOT, false, 3600).unwrap();
    let receipt = store.publish_prekey_online(SLOT).unwrap();
    let identity = store.identity().unwrap();
    let (publication, _, _) = load(&store.db, &store.key, &SLOT, &identity).unwrap();
    let mut legacy = serde_json::to_value(publication).unwrap();
    legacy.as_object_mut().unwrap().remove("retire_at");
    let bytes = store
        .key
        .seal(
            &serde_json::to_vec(&legacy).unwrap(),
            &binding(12, &SLOT, &identity),
        )
        .unwrap();
    store
        .db
        .execute("UPDATE prekey_publications SET state=?1", [bytes])
        .unwrap();
    crate::test_schema::rewind(&store.db, 11);
    drop(store);
    let mut store = open(&path);
    assert_eq!(store.retire_prekeys(i64::MAX as u64).unwrap(), 0);
    assert!(live(&store.db, &SLOT).unwrap());
    // Local and server clock origins deliberately differ by decades.
    assert_eq!(
        store.publish_prekey_with_clock(SLOT, || Ok(2000)).unwrap(),
        receipt
    );
    let deadline = 2000 + 3600 + DELIVERY_LIFETIME;
    assert_eq!(store.retire_prekeys(deadline - 1).unwrap(), 0);
    assert_eq!(store.retire_prekeys(deadline).unwrap(), 1);
}
