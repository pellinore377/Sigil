use super::*;
use crate::network::tests::Fixture;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
const BUDGET: u64 = 32 * 1024 * 1024;
fn metadata() -> Metadata {
    Metadata {
        name: "synthetic.bin".into(),
        media_type: "application/octet-stream".into(),
    }
}
fn run(store: &ClientStore, cache: &mut Cache, begin: u64, end: u64) -> ScheduledTransfers {
    let mut times = [begin, end].into_iter();
    store
        .attachments_with_clock(cache, || Ok(times.next().unwrap()))
        .unwrap()
}
#[test]
fn retry_after_and_failed_completion_preserve_reservation_across_restart() {
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
                if request.uri().path().starts_with("/client/v0/attachments/") {
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
    let mut store = ClientStore::open(
        &dir.path().join("client.db"),
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap();
    crate::connection::tests::prepare(&mut store, &fixture, &invite.secret);
    store.enroll_online().unwrap();
    let path = dir.path().join("cache.db");
    let mut cache = store.open_attachment_cache(&path, BUDGET).unwrap();
    let file = cache.prepare_upload(0, metadata(), None).unwrap();
    cache.stage_chunk(file, 0, b"").unwrap();
    cache.finish_staging(file).unwrap();
    let first = run(&store, &mut cache, now, now + 30);
    assert!(matches!(
        first.attempt.unwrap().result,
        Err(Error::Network(crate::network::Error::Status {
            code: 429,
            ..
        }))
    ));
    assert_eq!(first.next_at, now + 150);
    assert!(first.scheduling_error.is_none());
    assert_eq!(cache.phase(file).unwrap(), Phase::Starting);
    drop(cache);
    let mut cache = store.open_attachment_cache(&path, BUDGET).unwrap();
    assert!(run(&store, &mut cache, now + 149, now + 149)
        .attempt
        .is_none());
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    reject.store(false, Ordering::SeqCst);
    let resumed = run(&store, &mut cache, now + 150, now + 151);
    assert_eq!(
        resumed.attempt.unwrap().result.unwrap(),
        TransferProgress::Upload(UploadStep::Begun)
    );
    assert_eq!(resumed.next_at, now + 151);
    assert_eq!(
        schedule::read(&cache.db, &cache.key, &cache.scope)
            .unwrap()
            .0
            .failures,
        0
    );
    let db = Connection::open(&path).unwrap();
    let mut called = false;
    let result=store.attachments_with_clock(&mut cache,|| {
        if called {db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON sync_schedule BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();Ok(now+154)}
        else {called=true;Ok(now+152)}
    }).unwrap();
    assert_eq!(
        result.attempt.unwrap().result.unwrap(),
        TransferProgress::Upload(UploadStep::Chunk(0))
    );
    assert!(matches!(result.scheduling_error, Some(Error::Storage(_))));
    assert_eq!(result.next_at, now + 212);
    db.execute_batch("DROP TRIGGER fail").unwrap();
    drop(cache);
    let mut cache = store.open_attachment_cache(&path, BUDGET).unwrap();
    assert!(run(&store, &mut cache, now + 211, now + 211)
        .attempt
        .is_none());
    assert!(matches!(
        store.attachments_with_clock(&mut cache, || Ok(now + 151)),
        Err(Error::Expired)
    ));
    let final_step = run(&store, &mut cache, now + 212, now + 213);
    assert_eq!(
        final_step.attempt.unwrap().result.unwrap(),
        TransferProgress::Upload(UploadStep::Published)
    );
}

#[test]
fn cursor_reaches_pending_files_expiry_clears_keys_and_schema_one_upgrades() {
    let (dir, _fixture, store, _bob, now) = crate::claims::tests::pair();
    let path = dir.path().join("cache.db");
    let mut cache = store.open_attachment_cache(&path, BUDGET).unwrap();
    for _ in 0..40 {
        cache.prepare_upload(0, metadata(), None).unwrap();
    }
    let mut active = Vec::new();
    for _ in 0..2 {
        let file = cache.prepare_upload(0, metadata(), None).unwrap();
        cache.stage_chunk(file, 0, b"").unwrap();
        cache.finish_staging(file).unwrap();
        active.push(file);
    }
    let expired = cache.prepare_upload(1, metadata(), Some(now + 1)).unwrap();
    cache.stage_chunk(expired, 0, b"x").unwrap();
    cache
        .db
        .execute_batch(
            "DROP TABLE sync_schedule; DROP TABLE transfer_cursor; DROP TABLE recovery_transfers; DROP TABLE recovery_cursor; PRAGMA user_version=1",
        )
        .unwrap();
    drop(cache);
    let mut cache = store.open_attachment_cache(&path, BUDGET).unwrap();
    assert_eq!(
        cache
            .db
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        5
    );
    let mut time = now + 2;
    for _ in 0..30 {
        let step = run(&store, &mut cache, time, time);
        time = step.next_at;
        if let Some(attempt) = step.attempt {
            assert!(attempt.result.is_ok());
        }
        if active
            .iter()
            .all(|file| cache.phase(*file).unwrap() == Phase::Published)
            && cache.phase(expired).unwrap() == Phase::Expired
        {
            break;
        }
    }
    assert!(active
        .iter()
        .all(|file| cache.phase(*file).unwrap() == Phase::Published));
    assert!(load(&cache.db, &cache.key, expired)
        .unwrap()
        .descriptor
        .is_none());
    assert_eq!(
        cache
            .db
            .query_row(
                "SELECT count(*) FROM chunks WHERE file=?1",
                [expired.as_slice()],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn bounded_eviction_releases_capacity_without_server_deletion_or_job_resurrection() {
    let (dir, _fixture, store, _bob, now) = crate::claims::tests::pair();
    let path = dir.path().join("cache.db");
    let mut cache = store.open_attachment_cache(&path, BUDGET).unwrap();
    let file = cache
        .prepare_upload(16 * 1024 * 1024 + 1, metadata(), None)
        .unwrap();
    assert!(matches!(cache.evict(file), Err(Error::Conflict)));
    let chunk = vec![1; 1024 * 1024];
    for index in 0..16 {
        cache.stage_chunk(file, index, &chunk).unwrap();
    }
    cache.stage_chunk(file, 16, b"x").unwrap();
    cache.cancel(file).unwrap();
    cache.evict(file).unwrap();
    assert_eq!(cache.phase(file).unwrap(), Phase::Evicting);
    let first = run(&store, &mut cache, now, now);
    assert_eq!(
        first.attempt.unwrap().result.unwrap(),
        TransferProgress::Cleanup(16)
    );
    assert_eq!(cache.phase(file).unwrap(), Phase::Evicting);
    drop(cache);
    let mut cache = store.open_attachment_cache(&path, BUDGET).unwrap();
    let second = run(&store, &mut cache, first.next_at, first.next_at);
    assert_eq!(
        second.attempt.unwrap().result.unwrap(),
        TransferProgress::Cleanup(1)
    );
    assert!(matches!(cache.phase(file), Err(Error::NotFound)));
    let next = cache
        .prepare_upload(16 * 1024 * 1024 + 1, metadata(), None)
        .unwrap();
    assert_ne!(file, next);
    cache.stage_chunk(next, 0, &chunk).unwrap();
    assert_eq!(
        sigil_server::store::Store::open(&dir.path().join("server.db"))
            .unwrap()
            .attachment_status(
                &crate::connection::tests::credential(&store),
                &crate::transport::hex(&file),
                now
            )
            .err()
            .map(|e| matches!(e, sigil_server::store::StoreError::NotFound)),
        Some(true)
    );
}

#[test]
fn conflicts_reconcile_removed_files_and_restored_markers_stop_automatic_publication() {
    let (dir, _fixture, store, _bob, now) = crate::claims::tests::pair();
    let mut cache = store
        .open_attachment_cache(&dir.path().join("cache.db"), BUDGET)
        .unwrap();
    let removed = cache.prepare_upload(0, metadata(), None).unwrap();
    cache.stage_chunk(removed, 0, b"").unwrap();
    cache.finish_staging(removed).unwrap();
    let first = run(&store, &mut cache, now, now);
    assert_eq!(
        first.attempt.unwrap().result.unwrap(),
        TransferProgress::Upload(UploadStep::Begun)
    );
    store
        .connected_client()
        .unwrap()
        .remove_attachment(removed)
        .unwrap();
    let failed = run(&store, &mut cache, first.next_at, first.next_at);
    assert!(matches!(
        failed.attempt.unwrap().result,
        Err(Error::Network(crate::network::Error::Status {
            code: 409,
            ..
        }))
    ));
    assert_eq!(cache.phase(removed).unwrap(), Phase::Checking);
    let checked = run(&store, &mut cache, failed.next_at, failed.next_at);
    assert_eq!(
        checked.attempt.unwrap().result.unwrap(),
        TransferProgress::Reconciled(Phase::Cancelled)
    );
    assert!(load(&cache.db, &cache.key, removed)
        .unwrap()
        .descriptor
        .is_none());
    cache.cleanup(removed).unwrap();

    let restored = cache.prepare_upload(0, metadata(), None).unwrap();
    cache.stage_chunk(restored, 0, b"").unwrap();
    cache.finish_staging(restored).unwrap();
    assert_eq!(
        store.upload_attachment_step(&mut cache, restored).unwrap(),
        UploadStep::Begun
    );
    assert_eq!(
        store.upload_attachment_step(&mut cache, restored).unwrap(),
        UploadStep::Chunk(0)
    );
    cache.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON files BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(matches!(
        store.upload_attachment_step(&mut cache, restored),
        Err(Error::Storage(_))
    ));
    cache.db.execute_batch("DROP TRIGGER fail").unwrap();
    // Isolate the post-restore marker boundary. Full restore/credential revocation
    // is exercised by the server and container suites, not simulated here.
    let server = Connection::open(dir.path().join("server.db")).unwrap();
    server
        .execute(
            "UPDATE attachments SET restored_checkpoint=1 WHERE id=?1",
            [crate::transport::hex(&restored)],
        )
        .unwrap();
    let failed = run(&store, &mut cache, checked.next_at, checked.next_at);
    assert!(failed.attempt.unwrap().result.is_err());
    assert_eq!(cache.phase(restored).unwrap(), Phase::Checking);
    let mut checked = run(&store, &mut cache, failed.next_at, failed.next_at);
    // With two random file IDs, the bounded scan may first visit the terminal
    // file at the end of the ordering, then wrap on the following pass.
    if checked.attempt.is_none() {
        assert_eq!(checked.next_at, failed.next_at + 5);
        checked = run(&store, &mut cache, checked.next_at, checked.next_at);
    }
    assert_eq!(
        checked.attempt.unwrap().result.unwrap(),
        TransferProgress::Reconciled(Phase::Restored)
    );
    assert!(cache.published_descriptor(restored, now).is_err());
    assert_eq!(cache.completed_chunk(restored, 0, now).unwrap().len(), 0);
    let idle = run(&store, &mut cache, checked.next_at, checked.next_at);
    assert!(idle.attempt.is_none());
    assert!(server
        .query_row(
            "SELECT restored_checkpoint FROM attachments WHERE id=?1",
            [crate::transport::hex(&restored)],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
    cache.evict(restored).unwrap();
    cache.cleanup(restored).unwrap();
}
