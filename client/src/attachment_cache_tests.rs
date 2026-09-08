use super::*;
use sigil_crypto::attachment::CHUNK_SIZE;
fn key() -> StorageKey {
    StorageKey::new(Secret32::from_bytes([91; 32])).unwrap()
}
fn open(path: &Path, budget: u64) -> Cache {
    Cache::open(path, key(), [1; 32], budget).unwrap()
}
fn metadata() -> Metadata {
    Metadata {
        name: "synthetic.bin".into(),
        media_type: "application/octet-stream".into(),
    }
}
const BUDGET: u64 = 8 * 1024 * 1024;
fn private_dir() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

#[test]
fn erased_cache_descriptor_is_removed_from_live_files() {
    let dir = private_dir();
    let path = dir.path().join("cache.db");
    let mut cache = open(&path, BUDGET);
    let file = cache.prepare_upload(5, metadata(), None).unwrap();
    cache.stage_chunk(file, 0, b"12345").unwrap();
    let old: Vec<u8> = cache
        .db
        .query_row("SELECT state FROM files", [], |r| r.get(0))
        .unwrap();
    cache.cancel(file).unwrap();
    cache.cleanup(file).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert!(!old
        .as_chunks::<64>()
        .0
        .iter()
        .skip(1)
        .any(|chunk| bytes.windows(64).any(|v| v == chunk)));
    assert!(!path.with_extension("db-wal").exists());
}

#[test]
fn exact_staging_restarts_and_never_exposes_incomplete_descriptors() {
    let dir = private_dir();
    let path = dir.path().join("cache.db");
    let mut cache = open(&path, BUDGET);
    let file = cache
        .prepare_upload(CHUNK_SIZE as u64 + 5, metadata(), None)
        .unwrap();
    assert!(matches!(cache.finish_staging(file), Err(Error::Unprepared)));
    assert!(cache.published_descriptor(file, 1).is_err());
    let hash = cache.stage_chunk(file, 1, b"final").unwrap();
    assert!(matches!(
        cache.stage_chunk(file, 1, b"other"),
        Err(Error::Conflict)
    ));
    let frozen = part(
        &cache.db,
        &cache.key,
        Shape {
            file,
            length: CHUNK_SIZE as u64 + 5,
        },
        1,
    )
    .unwrap()
    .unwrap()
    .1;
    drop(cache);
    let mut cache = open(&path, BUDGET);
    assert_eq!(cache.stage_chunk(file, 1, b"final").unwrap(), hash);
    assert_eq!(
        part(
            &cache.db,
            &cache.key,
            Shape {
                file,
                length: CHUNK_SIZE as u64 + 5
            },
            1
        )
        .unwrap()
        .unwrap()
        .1,
        frozen
    );
    cache.stage_chunk(file, 0, &vec![7; CHUNK_SIZE]).unwrap();
    cache.finish_staging(file).unwrap();
    cache.finish_staging(file).unwrap();
    assert_eq!(cache.phase(file).unwrap(), Phase::Ready);
    assert!(cache.published_descriptor(file, 1).is_err());
    let state = load(&cache.db, &cache.key, file).unwrap();
    let (file_key, root) = state.key().unwrap();
    let mut list = CiphertextList::new(state.shape()).unwrap();
    for index in 0..2 {
        let (_, data) = part(&cache.db, &cache.key, state.shape(), index)
            .unwrap()
            .unwrap();
        file_key.open_chunk(index, &data).unwrap();
        list.push(index, &data).unwrap();
    }
    assert_eq!(root, list.finish().unwrap());
    cache.cancel(file).unwrap();
    assert_eq!(cache.phase(file).unwrap(), Phase::Cancelled);
    assert!(matches!(
        cache.stage_chunk(file, 1, b"final"),
        Err(Error::Cancelled)
    ));
    assert_eq!(cache.cleanup(file).unwrap(), 2);
    assert_eq!(cache.cleanup(file).unwrap(), 0);
    assert!(load(&cache.db, &cache.key, file)
        .unwrap()
        .descriptor
        .is_none());
}

#[test]
fn failed_writes_corruption_and_page_budget_do_not_advance_work() {
    let dir = private_dir();
    let path = dir.path().join("cache.db");
    let mut cache = open(&path, 1024 * 1024);
    let file = cache.prepare_upload(900_000, metadata(), None).unwrap();
    cache.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON chunks BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(cache.stage_chunk(file, 0, &vec![1; 900_000]).is_err());
    assert_eq!(
        cache
            .db
            .query_row("SELECT count(*) FROM chunks", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    cache.db.execute_batch("DROP TRIGGER fail").unwrap();
    cache.stage_chunk(file, 0, &vec![1; 900_000]).unwrap();
    let second = cache.prepare_upload(200_000, metadata(), None).unwrap();
    assert!(
        matches!(cache.stage_chunk(second,0,&vec![2;200_000]),Err(Error::Storage(rusqlite::Error::SqliteFailure(e,_))) if e.code==rusqlite::ErrorCode::DiskFull)
    );
    drop(cache);
    let mut cache = open(&path, BUDGET);
    cache.stage_chunk(second, 0, &vec![2; 200_000]).unwrap();
    let state = load(&cache.db, &cache.key, file).unwrap();
    let (_, mut bytes) = part(&cache.db, &cache.key, state.shape(), 0)
        .unwrap()
        .unwrap();
    bytes[80] ^= 1;
    cache
        .db
        .execute(
            "UPDATE chunks SET data=?2 WHERE file=?1",
            (file.as_slice(), bytes),
        )
        .unwrap();
    assert!(matches!(
        cache.stage_chunk(file, 0, &vec![1; 900_000]),
        Err(Error::InvalidStore)
    ));
    assert!(Cache::open(
        &path,
        StorageKey::new(Secret32::from_bytes([92; 32])).unwrap(),
        [1; 32],
        BUDGET
    )
    .is_err());
    assert!(Cache::open(&path, key(), [2; 32], BUDGET).is_err());
}

#[test]
fn competing_stagers_freeze_one_ciphertext_and_private_metadata_stays_sealed() {
    let dir = private_dir();
    let path = dir.path().join("cache.db");
    let mut cache = open(&path, BUDGET);
    for name in ["", "..", "a/b", "a\\b", "bad\0name"] {
        assert!(cache
            .prepare_upload(
                0,
                Metadata {
                    name: name.into(),
                    ..metadata()
                },
                None
            )
            .is_err());
    }
    let file = cache.prepare_upload(0, metadata(), None).unwrap();
    let mut other = open(&path, BUDGET);
    let thread = std::thread::spawn(move || other.stage_chunk(file, 0, b"").unwrap());
    let first = cache.stage_chunk(file, 0, b"").unwrap();
    assert_eq!(first, thread.join().unwrap());
    cache.finish_staging(file).unwrap();
    let sealed: Vec<u8> = cache
        .db
        .query_row("SELECT state FROM files", [], |r| r.get(0))
        .unwrap();
    assert!(!sealed
        .windows(b"synthetic.bin".len())
        .any(|v| v == b"synthetic.bin"));
    cache
        .db
        .execute_batch("CREATE VIEW unsafe_schema AS SELECT * FROM files")
        .unwrap();
    assert!(matches!(
        Cache::open(&path, key(), [1; 32], BUDGET),
        Err(Error::InvalidStore)
    ));
}

#[test]
fn real_https_upload_resumes_ambiguous_receipts_and_cancellation() {
    let (dir, _fixture, alice, bob, _now) = crate::claims::tests::pair();
    let path = dir.path().join("attachment-cache.db");
    let mut cache = alice.open_attachment_cache(&path, BUDGET).unwrap();
    let file = cache
        .prepare_upload(CHUNK_SIZE as u64 + 5, metadata(), None)
        .unwrap();
    cache.stage_chunk(file, 0, &vec![6; CHUNK_SIZE]).unwrap();
    cache.stage_chunk(file, 1, b"final").unwrap();
    cache.finish_staging(file).unwrap();
    assert!(matches!(
        bob.upload_attachment_step(&mut cache, file),
        Err(Error::Conflict)
    ));
    assert_eq!(
        alice.upload_attachment_step(&mut cache, file).unwrap(),
        UploadStep::Begun
    );
    cache.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON chunks BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(matches!(
        alice.upload_attachment_step(&mut cache, file),
        Err(Error::Storage(_))
    ));
    let shape = load(&cache.db, &cache.key, file).unwrap().shape();
    let (_, frozen) = part(&cache.db, &cache.key, shape, 0).unwrap().unwrap();
    assert_eq!(
        alice
            .connected_client()
            .unwrap()
            .attachment_parts(shape, None)
            .unwrap()
            .chunks
            .len(),
        1
    );
    cache.db.execute_batch("DROP TRIGGER fail").unwrap();
    drop(cache);
    let mut cache = alice.open_attachment_cache(&path, BUDGET).unwrap();
    assert_eq!(
        alice.upload_attachment_step(&mut cache, file).unwrap(),
        UploadStep::Chunk(0)
    );
    assert_eq!(
        part(&cache.db, &cache.key, shape, 0).unwrap().unwrap().1,
        frozen
    );
    assert_eq!(
        alice.upload_attachment_step(&mut cache, file).unwrap(),
        UploadStep::Chunk(1)
    );
    cache.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON files BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(matches!(
        alice.upload_attachment_step(&mut cache, file),
        Err(Error::Storage(_))
    ));
    cache.db.execute_batch("DROP TRIGGER fail").unwrap();
    assert_eq!(
        alice.upload_attachment_step(&mut cache, file).unwrap(),
        UploadStep::Published
    );
    let descriptor = cache.published_descriptor(file, 1).unwrap();
    let (key, root) = FileKey::from_descriptor(&descriptor.bytes).unwrap();
    let mut list = CiphertextList::new(shape).unwrap();
    for index in 0..shape.chunks().unwrap() {
        let bytes = bob
            .connected_client()
            .unwrap()
            .attachment_chunk(shape, index, &descriptor.access)
            .unwrap();
        key.open_chunk(index, &bytes).unwrap();
        list.push(index, &bytes).unwrap();
    }
    assert_eq!(list.finish().unwrap(), root);
    cache.cancel(file).unwrap();
    assert_eq!(cache.phase(file).unwrap(), Phase::Cancelling);
    assert!(cache.published_descriptor(file, 1).is_err());
    assert_eq!(
        alice.upload_attachment_step(&mut cache, file).unwrap(),
        UploadStep::Removed
    );
    assert!(bob
        .connected_client()
        .unwrap()
        .attachment_chunk(shape, 0, &descriptor.access)
        .is_err());
    assert_eq!(cache.cleanup(file).unwrap(), 2);
}

#[test]
fn cancellation_of_ambiguous_creation_blocks_a_late_begin() {
    let (dir, _fixture, alice, _bob, _now) = crate::claims::tests::pair();
    let mut cache = alice
        .open_attachment_cache(&dir.path().join("cache.db"), BUDGET)
        .unwrap();
    let file = cache.prepare_upload(0, metadata(), None).unwrap();
    cache.stage_chunk(file, 0, b"").unwrap();
    cache.finish_staging(file).unwrap();
    let mut state = load(&cache.db, &cache.key, file).unwrap();
    let begin = sigil_protocol::attachments::Begin {
        plaintext_bytes: 0,
        access_token: state.access.as_ref().unwrap().to_string(),
        expires_at: None,
    };
    state.phase = Phase::Starting;
    save(&cache.db, &cache.key, &state, false).unwrap();
    cache.cancel(file).unwrap();
    assert_eq!(
        alice.upload_attachment_step(&mut cache, file).unwrap(),
        UploadStep::Removed
    );
    assert!(matches!(
        alice
            .connected_client()
            .unwrap()
            .begin_attachment(file, &begin),
        Err(crate::network::Error::Status { code: 409, .. })
    ));
    assert_eq!(cache.phase(file).unwrap(), Phase::Cancelled);
}

fn download_fixture(length: u64) -> (Descriptor, Vec<Vec<u8>>) {
    let key = FileKey::generate(length).unwrap();
    let mut list = CiphertextList::new(key.shape()).unwrap();
    let mut parts = Vec::new();
    for index in 0..key.shape().chunks().unwrap() {
        let part = key
            .seal_chunk(
                index,
                &vec![index as u8 + 3; key.shape().chunk_length(index).unwrap()],
            )
            .unwrap();
        list.push(index, &part).unwrap();
        parts.push(part);
    }
    (
        Descriptor {
            source: None,
            bytes: key.descriptor(list.finish().unwrap()),
            access: Zeroizing::new("ab".repeat(32)),
            metadata: metadata(),
            expires_at: None,
        },
        parts,
    )
}

#[test]
fn downloads_require_full_commitment_reauthenticate_and_resume_out_of_order() {
    let dir = private_dir();
    let path = dir.path().join("cache.db");
    let mut cache = open(&path, BUDGET);
    let (descriptor, parts) = download_fixture(CHUNK_SIZE as u64 + 5);
    let file = cache
        .prepare_authenticated_download(descriptor, 10)
        .unwrap();
    cache.accept_download_chunk(file, 1, &parts[1], 10).unwrap();
    assert!(matches!(
        cache.completed_chunk(file, 1, 10),
        Err(Error::Unprepared)
    ));
    assert!(cache.finish_download(file, 10).is_err());
    assert!(cache.accept_download_chunk(file, 0, &parts[1], 10).is_err());
    let mut bad = parts[0].clone();
    bad[100] ^= 1;
    assert!(cache.accept_download_chunk(file, 0, &bad, 10).is_err());
    assert_eq!(
        cache
            .db
            .query_row("SELECT count(*) FROM chunks", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(cache);
    let mut cache = open(&path, BUDGET);
    cache.accept_download_chunk(file, 1, &parts[1], 10).unwrap();
    cache.accept_download_chunk(file, 0, &parts[0], 10).unwrap();
    cache.finish_download(file, 10).unwrap();
    cache.finish_download(file, 10).unwrap();
    assert_eq!(
        cache.completed_chunk(file, 0, 10).unwrap().as_slice(),
        vec![3; CHUNK_SIZE]
    );
    assert_eq!(
        cache.completed_chunk(file, 1, 10).unwrap().as_slice(),
        [4; 5]
    );
    cache
        .db
        .execute(
            "UPDATE chunks SET data=?2 WHERE file=?1 AND part=0",
            (file.as_slice(), bad),
        )
        .unwrap();
    assert!(cache.completed_chunk(file, 0, 10).is_err());
    cache.cancel(file).unwrap();
    assert_eq!(cache.phase(file).unwrap(), Phase::Cancelled);
    assert!(matches!(
        cache.completed_chunk(file, 1, 10),
        Err(Error::Cancelled)
    ));
}

#[test]
fn authentic_chunks_with_wrong_complete_root_never_publish_and_expiry_blocks_read() {
    let dir = private_dir();
    let mut cache = open(&dir.path().join("cache.db"), BUDGET);
    let (mut descriptor, parts) = download_fixture(5);
    descriptor.bytes[84] ^= 1;
    let file = cache
        .prepare_authenticated_download(descriptor, 10)
        .unwrap();
    cache.accept_download_chunk(file, 0, &parts[0], 10).unwrap();
    assert!(matches!(
        cache.finish_download(file, 10),
        Err(Error::Crypto(sigil_crypto::Error::Authentication))
    ));
    assert_eq!(cache.phase(file).unwrap(), Phase::Downloading);
    assert!(cache.completed_chunk(file, 0, 10).is_err());
    let (mut descriptor, parts) = download_fixture(0);
    descriptor.expires_at = Some(20);
    let file = cache
        .prepare_authenticated_download(descriptor, 10)
        .unwrap();
    cache.accept_download_chunk(file, 0, &parts[0], 19).unwrap();
    cache.finish_download(file, 19).unwrap();
    assert_eq!(cache.completed_chunk(file, 0, 19).unwrap().len(), 0);
    assert!(matches!(
        cache.completed_chunk(file, 0, 20),
        Err(Error::Expired)
    ));
    assert!(matches!(
        cache.accept_download_chunk(file, 0, &parts[0], 20),
        Err(Error::Expired)
    ));
    assert!(cache.completed_chunk(file, 0, 0).is_err());
}

#[test]
fn download_write_failure_and_conflicting_descriptor_preserve_frozen_state() {
    let dir = private_dir();
    let mut cache = open(&dir.path().join("cache.db"), BUDGET);
    let (descriptor, parts) = download_fixture(5);
    let duplicate = Descriptor {
        source: None,
        bytes: descriptor.bytes.clone(),
        access: descriptor.access.clone(),
        metadata: descriptor.metadata.clone(),
        expires_at: None,
    };
    let file = cache
        .prepare_authenticated_download(descriptor, 10)
        .unwrap();
    assert_eq!(
        cache.prepare_authenticated_download(duplicate, 10).unwrap(),
        file
    );
    let mut state = load(&cache.db, &cache.key, file).unwrap();
    let conflict = Descriptor {
        source: None,
        bytes: state.descriptor.take().unwrap(),
        access: Zeroizing::new("cd".repeat(32)),
        metadata: metadata(),
        expires_at: None,
    };
    assert!(matches!(
        cache.prepare_authenticated_download(conflict, 10),
        Err(Error::Conflict)
    ));
    cache.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON chunks BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(matches!(
        cache.accept_download_chunk(file, 0, &parts[0], 10),
        Err(Error::Storage(_))
    ));
    cache.db.execute_batch("DROP TRIGGER fail").unwrap();
    cache.accept_download_chunk(file, 0, &parts[0], 10).unwrap();
    cache.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON files BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(cache.finish_download(file, 10).is_err());
    assert_eq!(cache.phase(file).unwrap(), Phase::Downloading);
    assert!(cache.completed_chunk(file, 0, 10).is_err());
    cache.db.execute_batch("DROP TRIGGER fail").unwrap();
    cache.finish_download(file, 10).unwrap();
    assert_eq!(
        cache.completed_chunk(file, 0, 10).unwrap().as_slice(),
        [3; 5]
    );
}

#[test]
fn real_https_download_restarts_and_local_cancel_does_not_delete_senders_file() {
    let (dir, _fixture, alice, bob, now) = crate::claims::tests::pair();
    let mut sender = alice
        .open_attachment_cache(&dir.path().join("sender-cache.db"), BUDGET)
        .unwrap();
    let file = sender
        .prepare_upload(CHUNK_SIZE as u64 + 5, metadata(), None)
        .unwrap();
    sender.stage_chunk(file, 0, &vec![8; CHUNK_SIZE]).unwrap();
    sender.stage_chunk(file, 1, b"final").unwrap();
    sender.finish_staging(file).unwrap();
    for expected in [
        UploadStep::Begun,
        UploadStep::Chunk(0),
        UploadStep::Chunk(1),
        UploadStep::Published,
    ] {
        assert_eq!(
            alice.upload_attachment_step(&mut sender, file).unwrap(),
            expected
        );
    }
    let path = dir.path().join("recipient-cache.db");
    let mut receiver = bob.open_attachment_cache(&path, BUDGET).unwrap();
    assert_eq!(
        receiver
            .prepare_authenticated_download(sender.published_descriptor(file, now).unwrap(), now)
            .unwrap(),
        file
    );
    assert!(matches!(
        alice.download_attachment_step(&mut receiver, file, now),
        Err(Error::Conflict)
    ));
    receiver.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON chunks BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(matches!(
        bob.download_attachment_step(&mut receiver, file, now),
        Err(Error::Storage(_))
    ));
    receiver.db.execute_batch("DROP TRIGGER fail").unwrap();
    assert_eq!(
        bob.download_attachment_step(&mut receiver, file, now)
            .unwrap(),
        DownloadStep::Chunk(0)
    );
    assert!(receiver.completed_chunk(file, 0, now).is_err());
    drop(receiver);
    let mut receiver = bob.open_attachment_cache(&path, BUDGET).unwrap();
    assert_eq!(
        bob.download_attachment_step(&mut receiver, file, now)
            .unwrap(),
        DownloadStep::Chunk(1)
    );
    assert_eq!(
        bob.download_attachment_step(&mut receiver, file, now)
            .unwrap(),
        DownloadStep::Complete
    );
    assert_eq!(
        receiver.completed_chunk(file, 1, now).unwrap().as_slice(),
        b"final"
    );
    receiver.cancel(file).unwrap();
    assert_eq!(
        bob.upload_attachment_step(&mut receiver, file).unwrap(),
        UploadStep::Idle
    );
    assert_eq!(
        alice
            .connected_client()
            .unwrap()
            .attachment_status(Shape {
                file,
                length: CHUNK_SIZE as u64 + 5
            })
            .unwrap()
            .state,
        sigil_protocol::attachments::State::Published
    );
}

#[test]
#[ignore = "explicit 1 GiB disk acceptance; run separately in release mode"]
fn one_gib_file_stages_restarts_and_releases_its_independent_cache_budget() {
    let dir = private_dir();
    let path = dir.path().join("cache.db");
    let budget = 2 * 1024 * 1024 * 1024;
    let mut cache = open(&path, budget);
    let file = cache
        .prepare_upload(1024 * 1024 * 1024, metadata(), None)
        .unwrap();
    let body = vec![7; CHUNK_SIZE];
    let started = std::time::Instant::now();
    for index in 0..1024 {
        cache.stage_chunk(file, index, &body).unwrap();
    }
    let staged = started.elapsed();
    let finalized = std::time::Instant::now();
    cache.finish_staging(file).unwrap();
    let finished = finalized.elapsed();
    let pages: i64 = cache
        .db
        .query_row("PRAGMA page_count", [], |r| r.get(0))
        .unwrap();
    let page_size: i64 = cache
        .db
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .unwrap();
    assert!((pages * page_size) as u64 <= budget);
    drop(cache);
    let mut cache = open(&path, budget);
    assert_eq!(cache.phase(file).unwrap(), Phase::Ready);
    let state = load(&cache.db, &cache.key, file).unwrap();
    let (file_key, root) = state.key().unwrap();
    assert_eq!(committed_root(&cache.db, &cache.key, &state).unwrap(), root);
    for index in [0, 512, 1023] {
        let (_, bytes) = part(&cache.db, &cache.key, state.shape(), index)
            .unwrap()
            .unwrap();
        assert_eq!(file_key.open_chunk(index, &bytes).unwrap().as_slice(), body);
    }
    cache.cancel(file).unwrap();
    cache.evict(file).unwrap();
    let cleanup = std::time::Instant::now();
    let mut removed = 0;
    loop {
        let count = cache.cleanup(file).unwrap();
        removed += count;
        if count == 0 {
            break;
        }
    }
    assert_eq!(removed, 1024);
    assert!(matches!(cache.phase(file), Err(Error::NotFound)));
    let elapsed = cleanup.elapsed();
    let next = cache
        .prepare_upload(1024 * 1024 * 1024, metadata(), None)
        .unwrap();
    cache.stage_chunk(next, 0, &body).unwrap();
    eprintln!("synthetic 1 GiB cache: stage={staged:?}, finalize={finished:?}, cleanup={elapsed:?}, database_bytes={}",pages*page_size);
}
