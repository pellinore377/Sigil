use super::*;
use crate::{claims::tests::pair, network::tests::Fixture};
use sigil_crypto::recovery::Direction;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
fn configure(store: &mut ClientStore) {
    let account =
        crate::connection::decode_id(&store.connection_session().unwrap().unwrap().account_id)
            .unwrap();
    store
        .configure_recovery("chat.example", account, Secret32::from_bytes([7; 32]))
        .unwrap();
}
fn text(id: u8, now: u64) -> Record {
    let mut author = [0; 32];
    author[0] = 9; // Valid synthetic X25519 base point.
    Record {
        id: [id; 32],
        revision: 1,
        conversation: [1; 32],
        author,
        created_at: now,
        direction: Direction::Outgoing,
        content: Content::Retained(Zeroizing::new(b"synthetic".to_vec())),
    }
}
fn file(id: u8, now: u64, expiry: u64) -> Record {
    let key = sigil_crypto::attachment::FileKey::generate(0).unwrap();
    let descriptor = key.descriptor([0; 32]);
    let bytes = sigil_protocol::file::File {
        caption: "A retained attachment caption.",
        source: "chat.example",
        name: "synthetic.bin",
        media_type: "application/octet-stream",
        expires_at: Some(expiry),
        access: &[3; 32],
        descriptor: &descriptor,
    }
    .to_bytes()
    .unwrap();
    Record {
        content: Content::File(Zeroizing::new(bytes)),
        ..text(id, now)
    }
}
fn run(store: &mut ClientStore, begin: u64, end: u64) -> ScheduledRecovery {
    let mut times = [begin, end].into_iter();
    store
        .recovery_with_clock(|| Ok(times.next().unwrap()))
        .unwrap()
}
#[test]
fn file_expiry_erases_local_keys_after_remote_retention() {
    let (_dir, _fixture, mut store, _, now) = pair();
    configure(&mut store);
    store
        .retain_recovery_record(&file(1, now, now + 172800))
        .unwrap();
    store
        .set_recovery_policy(super::super::RecoveryPolicy {
            history_days: Some(1),
        })
        .unwrap();
    store.maintain_recovery(now + 86400).unwrap();
    assert!(matches!(
        store.recovery_record([1; 32]).unwrap().content,
        Content::Omitted
    ));
    assert!(matches!(
        store.retained_history_record([1; 32]).unwrap().content,
        Content::File(_)
    ));
    store.maintain_recovery(now + 172800).unwrap();
    assert!(matches!(
        store.retained_history_record([1; 32]).unwrap().content,
        Content::Deleted
    ));
    assert_eq!(
        store
            .db
            .query_row(
                "SELECT count(*) FROM archive_local WHERE id=?1",
                [[1u8; 32].as_slice()],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}
#[test]
fn expiry_is_bounded_atomic_and_restart_safe_with_terminal_deletions() {
    let (dir, _fixture, mut store, _bob, now) = pair();
    configure(&mut store);
    for id in 1..=34 {
        store
            .retain_recovery_record(&file(id, now, now + 10))
            .unwrap();
    }
    store
        .retain_recovery_record(&file(35, now, now + 100))
        .unwrap();
    store.retain_recovery_record(&text(36, now)).unwrap();
    assert_eq!(store.maintain_recovery(now + 9).unwrap().expired, 0);
    // This scan starts after the first 16 live records from the prior pass.
    let before = read(&store.db, &store.key, &store.recovery_scope().unwrap())
        .unwrap()
        .cursor;
    store.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON archive_work BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(matches!(
        store.maintain_recovery(now + 10),
        Err(Error::Storage(_))
    ));
    assert_eq!(
        read(&store.db, &store.key, &store.recovery_scope().unwrap())
            .unwrap()
            .cursor,
        before
    );
    assert!(matches!(
        store.recovery_record([17; 32]).unwrap().content,
        Content::File(_)
    ));
    store.db.execute_batch("DROP TRIGGER fail").unwrap();
    let mut expired = store.maintain_recovery(now + 10).unwrap().expired;
    assert_eq!(expired, 16);
    drop(store);
    let mut store = ClientStore::open(
        &dir.path().join("alice.db"),
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap();
    for _ in 0..2 {
        let result = store.maintain_recovery(now + 10).unwrap();
        assert!(result.checked <= 16);
        expired += result.expired;
    }
    assert_eq!(expired, 34);
    assert!(matches!(
        store.recovery_record([35; 32]).unwrap().content,
        Content::File(_)
    ));
    assert!(matches!(
        store.recovery_record([36; 32]).unwrap().content,
        Content::Retained(_)
    ));
    assert!(matches!(
        store.maintain_recovery(now + 9),
        Err(Error::Expired)
    ));
    let deleted = store.delete_recovery_record([36; 32], 1).unwrap();
    assert_eq!(deleted.revision, 2);
    assert_eq!(store.delete_recovery_record([36; 32], 1).unwrap(), deleted);
    assert!(matches!(
        store.delete_recovery_record([35; 32], 2),
        Err(Error::Conflict)
    ));
    let mut resurrect = text(36, now);
    resurrect.revision = 3;
    assert!(matches!(
        store.retain_recovery_record(&resurrect),
        Err(Error::Conflict)
    ));
}
#[test]
fn deletion_during_upload_gets_a_successor_and_idle_passes_do_not_republish() {
    let (_dir, _fixture, mut store, _bob, now) = pair();
    configure(&mut store);
    store.retain_recovery_record(&text(1, now)).unwrap();
    let first = store.prepare_recovery_upload(now).unwrap();
    store.delete_recovery_record([1; 32], 1).unwrap();
    let result = run(&mut store, now, now);
    assert_eq!(
        result.progress.unwrap().unwrap(),
        RecoveryProgress::Upload(Some(first))
    );
    assert!(
        read(&store.db, &store.key, &store.recovery_scope().unwrap())
            .unwrap()
            .dirty
    );
    let result = run(&mut store, now + 1, now + 1);
    let RecoveryProgress::Upload(Some(second)) = result.progress.unwrap().unwrap() else {
        panic!("no successor")
    };
    assert_eq!(second.generation, first.generation + 1);
    let key = RecoveryKey::from_secret(
        Secret32::from_bytes([7; 32]),
        store.recovery_scope().unwrap(),
    )
    .unwrap();
    let client = store.connected_client().unwrap();
    let manifest = key
        .open_manifest(
            &second,
            client
                .download_recovery_object(second.manifest)
                .unwrap()
                .bytes(),
        )
        .unwrap();
    let page = manifest.pages[0];
    let references = key
        .open_page(
            &page,
            client
                .download_recovery_object(page.object)
                .unwrap()
                .bytes(),
        )
        .unwrap();
    let record = key
        .open_record(
            &references[0],
            client
                .download_recovery_object(references[0].object)
                .unwrap()
                .bytes(),
        )
        .unwrap();
    assert!(matches!(record.content, Content::Deleted));
    assert_eq!(record.revision, 2);
    assert_eq!(
        run(&mut store, now + 2, now + 2).progress.unwrap().unwrap(),
        RecoveryProgress::Cleanup(2)
    );
    assert_eq!(
        run(&mut store, now + 3, now + 3).progress.unwrap().unwrap(),
        RecoveryProgress::Idle
    );
    assert!(run(&mut store, now + 4, now + 4).progress.is_none());
    assert_eq!(store.recovery_status().unwrap().anchor, Some(second));
    // The independent recovery scheduler did not create a messaging reservation.
    assert!(schedule::read(&store.db, &store.key, &[0; 32])
        .unwrap()
        .1
        .is_none());
}
#[test]
fn recovery_retry_after_and_failed_completion_preserve_exact_work() {
    let (dir, old, invite, now) = crate::connection::tests::setup();
    drop(old);
    let reject = Arc::new(AtomicBool::new(true));
    let hits = Arc::new(AtomicUsize::new(0));
    let (rejected, count) = (reject.clone(), hits.clone());
    let router = sigil_server::router(
        sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap(),
        sigil_server::auth::AdminToken::load_or_create(&dir.path().join("admin.token")).unwrap(),
    )
    .layer(axum::middleware::from_fn(
        move |request: axum::extract::Request, next: axum::middleware::Next| {
            let (reject, count) = (rejected.clone(), count.clone());
            async move {
                if request.uri().path().starts_with("/client/v0/recovery/") {
                    count.fetch_add(1, Ordering::SeqCst);
                    if reject.load(Ordering::SeqCst) {
                        return axum::http::Response::builder()
                            .status(429)
                            .header("retry-after", "120")
                            .body(axum::body::Body::empty())
                            .unwrap();
                    }
                }
                next.run(request).await
            }
        },
    ));
    let fixture = Fixture::new(router);
    let path = dir.path().join("client.db");
    let mut store = ClientStore::open(
        &path,
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap();
    crate::connection::tests::prepare(&mut store, &fixture, &invite.secret);
    store.enroll_online().unwrap();
    configure(&mut store);
    store.retain_recovery_record(&text(1, now)).unwrap();
    let result = run(&mut store, now, now + 20);
    assert_eq!(result.next_at, now + 140);
    assert!(matches!(
        result.progress.unwrap(),
        Err(Error::Network(crate::network::Error::Status {
            code: 429,
            ..
        }))
    ));
    let pending = store.recovery_status().unwrap().pending;
    drop(store);
    let mut store = ClientStore::open(
        &path,
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap();
    assert!(run(&mut store, now + 139, now + 139).progress.is_none());
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    reject.store(false, Ordering::SeqCst);
    let db = Connection::open(&path).unwrap();
    let mut called = false;
    let result=store.recovery_with_clock(||{if called{db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON sync_schedule BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();Ok(now+141)}else{called=true;Ok(now+140)}}).unwrap();
    assert_eq!(
        result.progress.unwrap().unwrap(),
        RecoveryProgress::Upload(pending.map(|(_, head)| head))
    );
    assert!(matches!(result.scheduling_error, Some(Error::Storage(_))));
    assert_eq!(result.next_at, now + 200);
    db.execute_batch("DROP TRIGGER fail").unwrap();
    drop(store);
    let mut store = ClientStore::open(
        &path,
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap();
    assert!(run(&mut store, now + 199, now + 199).progress.is_none());
    assert_eq!(
        run(&mut store, now + 200, now + 200)
            .progress
            .unwrap()
            .unwrap(),
        RecoveryProgress::Idle
    );
}

#[test]
fn snapshot_expires_every_file_and_schema46_preserves_pending_edits_and_schedule() {
    let (dir, _fixture, mut store, _bob, now) = pair();
    configure(&mut store);
    for id in 1..=34 {
        store
            .retain_recovery_record(&file(id, now, now + 10))
            .unwrap();
    }
    let head = store.prepare_recovery_upload(now + 10).unwrap();
    for id in 1..=34 {
        assert!(matches!(
            store.recovery_record([id; 32]).unwrap().content,
            Content::Deleted
        ));
    }
    store.retain_recovery_record(&text(35, now)).unwrap();
    let (mut messaging, _) = schedule::read(&store.db, &store.key, &[8; 32]).unwrap();
    messaging.last = now;
    messaging.next = now + 300;
    let tx = store.db.transaction().unwrap();
    let original = schedule::write(&tx, &store.key, &[8; 32], &messaging).unwrap();
    tx.commit().unwrap();
    crate::test_schema::rewind(&store.db, 46);
    drop(store);
    let mut store = ClientStore::open(
        &dir.path().join("alice.db"),
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap();
    assert_eq!(
        store
            .db
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        i64::from(crate::DATABASE_VERSION)
    );
    assert_eq!(
        schedule::read(&store.db, &store.key, &[8; 32]).unwrap().1,
        Some(original)
    );
    assert_eq!(
        store.recovery_status().unwrap().pending,
        Some((Operation::Upload, head))
    );
    assert!(
        read(&store.db, &store.key, &store.recovery_scope().unwrap())
            .unwrap()
            .dirty
    );
    // Migration acceptance uses the low-level acknowledgement contract;
    // separate tests exercise the real HTTPS scheduler and Retry-After.
    loop {
        let objects = store.pending_recovery_objects().unwrap();
        if objects.is_empty() {
            break;
        }
        for object in objects {
            store
                .acknowledge_recovery_object(head, object.id())
                .unwrap();
        }
    }
    store
        .acknowledge_recovery_head(&sigil_protocol::recovery::Head {
            generation: head.generation,
            manifest: Some(crate::transport::hex(&head.manifest)),
            restored_checkpoint: false,
        })
        .unwrap();
    assert_eq!(store.recovery_status().unwrap().anchor, Some(head));
    assert!(
        read(&store.db, &store.key, &store.recovery_scope().unwrap())
            .unwrap()
            .dirty
    );
    assert_eq!(
        store.prepare_recovery_upload(now + 11).unwrap().generation,
        head.generation + 1
    );
    assert!(
        !read(&store.db, &store.key, &store.recovery_scope().unwrap())
            .unwrap()
            .dirty
    );
}

#[test]
fn authorized_fresh_import_resumes_but_equal_history_does_not_echo_another_snapshot() {
    let (dir, fixture, mut source, _bob, now) = pair();
    configure(&mut source);
    source.retain_recovery_record(&text(1, now)).unwrap();
    let head = source.prepare_recovery_upload(now).unwrap();
    assert_eq!(source.upload_recovery_step().unwrap(), Some(head));
    let account = source.connection_session().unwrap().unwrap().account_id;
    let invite = sigil_server::store::Store::open(&dir.path().join("server.db"))
        .unwrap()
        .invite_reauthorization(&account, 60, now)
        .unwrap();
    let path = dir.path().join("fresh.db");
    let open = || {
        ClientStore::open(
            &path,
            StorageKey::new(Secret32::from_bytes([10; 32])).unwrap(),
        )
        .unwrap()
    };
    let mut fresh = open();
    fresh
        .prepare_enrollment(
            "chat.example",
            fixture.port(),
            &[crate::network::tests::CA.to_vec()],
            &invite.secret,
            "Synthetic recovery worker",
            true,
        )
        .unwrap();
    fresh.enroll_online().unwrap();
    configure(&mut fresh);
    assert!(matches!(
        fresh.download_recovery_step(false),
        Err(Error::Conflict)
    ));
    assert_eq!(fresh.recovery_status().unwrap().pending, None);
    assert_eq!(fresh.download_recovery_step(true).unwrap(), None);
    assert!(fresh.maintain_recovery(now).unwrap().deferred);
    let mut time = now;
    let mut complete = false;
    for _ in 0..8 {
        let result = run(&mut fresh, time, time);
        time = result.next_at;
        if result.progress.unwrap().unwrap() == RecoveryProgress::Import(Some(head)) {
            complete = true;
            break;
        }
        drop(fresh);
        fresh = open();
    }
    assert!(complete);
    assert_eq!(
        run(&mut fresh, time, time).progress.unwrap().unwrap(),
        RecoveryProgress::Idle
    );
    assert_eq!(fresh.recovery_status().unwrap().anchor, Some(head));
    for table in ["sessions", "identity", "prekeys", "peers"] {
        assert_eq!(
            fresh
                .db
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn media_reference_index_is_rekeyed_on_history_handoff_and_imported_deletion_is_atomic() {
    let (dir, fixture, mut source, _bob, now) = pair();
    configure(&mut source);
    let retained = file(1, now, now + 300);
    source.retain_recovery_record(&retained).unwrap();
    let head = source.prepare_recovery_upload(now).unwrap();
    for object in source.pending_recovery_objects().unwrap() {
        source
            .acknowledge_recovery_object(head, object.id())
            .unwrap();
    }
    let response = |head: Head| sigil_protocol::recovery::Head {
        generation: head.generation,
        manifest: Some(crate::transport::hex(&head.manifest)),
        restored_checkpoint: false,
    };
    source.acknowledge_recovery_head(&response(head)).unwrap();
    let account = source.connection_session().unwrap().unwrap().account_id;
    let invite = sigil_server::store::Store::open(&dir.path().join("server.db"))
        .unwrap()
        .invite_reauthorization(&account, 60, now)
        .unwrap();
    let mut destination = ClientStore::open(
        &dir.path().join("handoff.db"),
        StorageKey::new(Secret32::from_bytes([10; 32])).unwrap(),
    )
    .unwrap();
    destination
        .prepare_enrollment(
            "chat.example",
            fixture.port(),
            &[crate::network::tests::CA.to_vec()],
            &invite.secret,
            "Synthetic archive handoff",
            true,
        )
        .unwrap();
    destination.enroll_online().unwrap();
    source.copy_recovery_history_to(&mut destination).unwrap();
    let Content::File(bytes) = &retained.content else {
        panic!("missing file")
    };
    assert!(crate::recovery::references_file(
        &destination.db,
        &destination.key,
        destination.recovery_scope().unwrap(),
        bytes
    )
    .unwrap());
    let mut removed = source.recovery_record(retained.id).unwrap();
    removed.revision = 2;
    removed.content = Content::Deleted;
    let key = RecoveryKey::from_secret(
        Secret32::from_bytes([7; 32]),
        source.recovery_scope().unwrap(),
    )
    .unwrap();
    let (reference, record) = key.seal_record(&removed).unwrap();
    let (page_ref, page) = key.seal_page(&[reference]).unwrap();
    let (next, manifest) = key
        .seal_manifest(Some(&head), now + 1, &[page_ref])
        .unwrap();
    source
        .begin_recovery_import(&response(next), &manifest, false)
        .unwrap();
    source.stage_recovery_page(0, &page).unwrap();
    source
        .stage_recovery_record(0, retained.id, &record)
        .unwrap();
    source.db.execute_batch("CREATE TRIGGER fail BEFORE DELETE ON archive_media BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(matches!(
        source.finish_recovery_import(),
        Err(Error::Storage(_))
    ));
    assert!(matches!(
        source.recovery_record(retained.id).unwrap().content,
        Content::File(_)
    ));
    assert_eq!(
        source
            .db
            .query_row("SELECT count(*) FROM archive_media", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    source.db.execute_batch("DROP TRIGGER fail").unwrap();
    assert_eq!(source.finish_recovery_import().unwrap(), next);
    assert!(!crate::recovery::references_file(
        &source.db,
        &source.key,
        source.recovery_scope().unwrap(),
        bytes
    )
    .unwrap());
    assert!(
        !read(&source.db, &source.key, &source.recovery_scope().unwrap())
            .unwrap()
            .dirty
    );
}
