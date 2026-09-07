use sigil_client::{
    recovery::{account_scope, Operation},
    ClientStore, Error,
};
use sigil_crypto::{
    recovery::{Content, Direction, Head, Object, Record, RecoveryKey},
    storage::StorageKey,
    DhKey, Secret32,
};
use sigil_protocol::recovery as wire;
use std::{collections::BTreeMap, path::Path};
use zeroize::Zeroizing;
mod common;
const ACCOUNT: [u8; 32] = [3; 32];
const SECRET: [u8; 32] = [7; 32];
fn private_dir() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}
fn open(path: &Path) -> ClientStore {
    ClientStore::open(
        path,
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap()
}
fn configured(path: &Path) -> ClientStore {
    let mut store = open(path);
    store
        .configure_recovery("chat.example", ACCOUNT, Secret32::from_bytes(SECRET))
        .unwrap();
    store
}
fn archive_key() -> RecoveryKey {
    RecoveryKey::from_secret(
        Secret32::from_bytes(SECRET),
        account_scope("chat.example", ACCOUNT).unwrap(),
    )
    .unwrap()
}
fn record(number: u16, author: [u8; 32]) -> Record {
    let mut id = [0; 32];
    id[30..].copy_from_slice(&number.to_be_bytes());
    Record {
        id,
        revision: 1,
        conversation: [5; 32],
        author,
        created_at: 1000,
        direction: Direction::Outgoing,
        content: Content::Retained(Zeroizing::new(
            format!("Synthetic retained {number}").into_bytes(),
        )),
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn response(head: Head) -> wire::Head {
    wire::Head {
        generation: head.generation,
        manifest: Some(hex(&head.manifest)),
        restored_checkpoint: false,
    }
}
fn snapshot(
    key: &RecoveryKey,
    previous: Option<&Head>,
    records: &[Record],
) -> (Head, Object, Object, Vec<Object>) {
    let mut references = Vec::new();
    let mut objects = Vec::new();
    for record in records {
        let (reference, object) = key.seal_record(record).unwrap();
        references.push(reference);
        objects.push(object);
    }
    let (page_ref, page) = key.seal_page(&references).unwrap();
    let (head, manifest) = key.seal_manifest(previous, 1000, &[page_ref]).unwrap();
    (head, manifest, page, objects)
}

#[test]
fn history_pages_are_bounded_authenticated_and_include_deletions() {
    let dir = private_dir();
    let path = dir.path().join("history.db");
    let mut store = configured(&path);
    assert!(store.recovery_records(None).unwrap().is_empty());
    let author = DhKey::generate().unwrap().public_key();
    for number in (0..18).rev() {
        store
            .retain_recovery_record(&record(number, author))
            .unwrap();
    }
    let mut deleted = record(3, author);
    deleted.revision = 2;
    deleted.content = Content::Deleted;
    store.retain_recovery_record(&deleted).unwrap();
    let first = store.recovery_records(None).unwrap();
    assert_eq!(first.len(), 16);
    for (number, actual) in first.iter().enumerate() {
        assert_eq!(actual.id, record(number as u16, author).id);
        assert_eq!(actual.author, author);
    }
    assert_eq!(first[3].revision, 2);
    assert!(matches!(first[3].content, Content::Deleted));
    let after = first.last().unwrap().id;
    drop(store);
    let store = open(&path);
    let second = store.recovery_records(Some(after)).unwrap();
    assert_eq!(second.len(), 2);
    assert_eq!(second[0].id, record(16, author).id);
    assert_eq!(second[1].id, record(17, author).id);
    assert!(store
        .recovery_records(Some(second[1].id))
        .unwrap()
        .is_empty());
    assert!(store.recovery_records(Some([255; 32])).unwrap().is_empty());
    let db = rusqlite::Connection::open(&path).unwrap();
    // A plausible index/reference cannot substitute another encrypted record;
    // a later failure returns no partially authenticated page to the caller.
    db.execute("UPDATE archive_records SET data=(SELECT data FROM archive_records WHERE id=?1),object=(SELECT object FROM archive_records WHERE id=?1) WHERE id=?2", (record(1, author).id.as_slice(), record(15, author).id.as_slice())).unwrap();
    assert!(store.recovery_records(None).is_err());
    assert_eq!(store.recovery_records(Some(after)).unwrap().len(), 2);
}

#[test]
fn uploads_and_multi_page_import_resume_exactly_across_restart() {
    let dir = private_dir();
    let path = dir.path().join("source.db");
    let mut source = configured(&path);
    let author = DhKey::generate().unwrap().public_key();
    for number in 0..257 {
        source
            .retain_recovery_record(&record(number, author))
            .unwrap();
    }
    let head = source.prepare_recovery_upload(1000).unwrap();
    assert!(matches!(
        source.recovery_publication(),
        Err(Error::Unprepared)
    ));
    // New local edits are retained while the frozen upload remains unchanged.
    source.retain_recovery_record(&record(300, author)).unwrap();
    assert!(source.acknowledge_recovery_head(&response(head)).is_err());
    assert!(source.acknowledge_recovery_object(head, [99; 32]).is_err());
    let first: Vec<_> = source
        .pending_recovery_objects()
        .unwrap()
        .iter()
        .map(|o| o.bytes().to_vec())
        .collect();
    drop(source);
    source = open(&path);
    assert_eq!(source.prepare_recovery_upload(2000).unwrap(), head);
    assert_eq!(
        source
            .pending_recovery_objects()
            .unwrap()
            .iter()
            .map(|o| o.bytes().to_vec())
            .collect::<Vec<_>>(),
        first
    );
    let mut uploaded = BTreeMap::new();
    loop {
        let batch = source.pending_recovery_objects().unwrap();
        if batch.is_empty() {
            break;
        }
        assert!(batch.len() <= 16);
        for object in batch {
            let id = object.id();
            uploaded.insert(id, object);
            source.acknowledge_recovery_object(head, id).unwrap();
            source.acknowledge_recovery_object(head, id).unwrap();
        }
        drop(source);
        source = open(&path);
    }
    assert_eq!(uploaded.len(), 260);
    let request = source.recovery_publication().unwrap();
    assert_eq!(request.expected_generation, 0);
    assert_eq!(request.expected_manifest, None);
    source.acknowledge_recovery_head(&response(head)).unwrap();
    source.acknowledge_recovery_head(&response(head)).unwrap();
    assert_eq!(source.recovery_status().unwrap().anchor, Some(head));
    let fresh_path = dir.path().join("fresh.db");
    let mut fresh = configured(&fresh_path);
    let manifest_object = &uploaded[&head.manifest];
    assert!(fresh
        .begin_recovery_import(&response(head), manifest_object, false)
        .is_err());
    fresh
        .begin_recovery_import(&response(head), manifest_object, true)
        .unwrap();
    let manifest = archive_key()
        .open_manifest(&head, manifest_object.bytes())
        .unwrap();
    assert_eq!(fresh.missing_recovery_pages().unwrap(), [0, 1]);
    assert!(fresh.finish_recovery_import().is_err());
    for (index, page_ref) in manifest.pages.iter().enumerate() {
        let page = &uploaded[&page_ref.object];
        let refs = archive_key().open_page(page_ref, page.bytes()).unwrap();
        let objects: Vec<_> = refs
            .iter()
            .map(|r| Object::from_bytes(uploaded[&r.object].bytes().to_vec()).unwrap())
            .collect();
        fresh.import_recovery_page(index, page, &objects).unwrap();
        assert!(fresh.recovery_records(None).unwrap().is_empty());
        assert!(matches!(
            fresh.recovery_record(record(0, author).id),
            Err(Error::NotFound)
        ));
        drop(fresh);
        if index == 0 {
            let db = rusqlite::Connection::open(&fresh_path).unwrap();
            common::rewind(&db, 8);
        }
        fresh = open(&fresh_path);
        fresh
            .begin_recovery_import(&response(head), manifest_object, false)
            .unwrap();
    }
    assert!(fresh.missing_recovery_pages().unwrap().is_empty());
    assert_eq!(fresh.finish_recovery_import().unwrap(), head);
    for number in [0, 128, 256] {
        let restored = fresh.recovery_record(record(number, author).id).unwrap();
        assert_eq!(restored.author, author);
        assert!(
            matches!(restored.content, Content::Retained(bytes) if bytes.as_slice() == format!("Synthetic retained {number}").as_bytes())
        );
    }
    let db = rusqlite::Connection::open(&fresh_path).unwrap();
    for table in [
        "sessions",
        "identity",
        "prekeys",
        "inbox",
        "outbox",
        "deliveries",
    ] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    assert_eq!(
        db.query_row("SELECT count(*) FROM archive_import", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn failed_snapshot_and_import_transactions_preserve_prior_state() {
    let dir = private_dir();
    let path = dir.path().join("client.db");
    let mut store = configured(&path);
    let author = DhKey::generate().unwrap().public_key();
    let first = record(1, author);
    let second = record(2, author);
    store.retain_recovery_record(&record(0, author)).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_snapshot BEFORE INSERT ON archive_objects BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(store.prepare_recovery_upload(1000).is_err());
    assert_eq!(store.recovery_status().unwrap().pending, None);
    assert_eq!(
        db.query_row("SELECT count(*) FROM archive_objects", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    db.execute_batch("DROP TRIGGER fail_snapshot;").unwrap();
    let (head, manifest, page, objects) = snapshot(&archive_key(), None, &[first, second]);
    store
        .begin_recovery_import(&response(head), &manifest, true)
        .unwrap();
    let mut corrupted = objects[1].bytes().to_vec();
    corrupted[40] ^= 1;
    let bad = [
        Object::from_bytes(objects[0].bytes().to_vec()).unwrap(),
        Object::from_bytes(corrupted).unwrap(),
    ];
    assert!(store.import_recovery_page(0, &page, &bad).is_err());
    assert_eq!(
        db.query_row("SELECT count(*) FROM archive_import", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    store.import_recovery_page(0, &page, &objects).unwrap();
    db.execute_batch("CREATE TRIGGER fail_import BEFORE INSERT ON archive_records WHEN hex(NEW.id)='0000000000000000000000000000000000000000000000000000000000000002' BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(store.finish_recovery_import().is_err());
    assert_eq!(store.recovery_status().unwrap().anchor, None);
    assert!(store.recovery_record(record(1, author).id).is_err());
    assert!(store.recovery_record(record(2, author).id).is_err());
    db.execute_batch("DROP TRIGGER fail_import;").unwrap();
    drop(store);
    store = open(&path);
    assert_eq!(store.finish_recovery_import().unwrap(), head);
    assert!(store.recovery_record(record(2, author).id).is_ok());
}

#[test]
fn tombstones_survive_older_imports_and_reject_newer_resurrection() {
    let dir = private_dir();
    let path = dir.path().join("client.db");
    let mut store = configured(&path);
    let author = DhKey::generate().unwrap().public_key();
    let mut value = record(1, author);
    let first = store.retain_recovery_record(&value).unwrap();
    assert_eq!(store.retain_recovery_record(&value).unwrap(), first);
    let (head, manifest, page, objects) = snapshot(&archive_key(), None, &[record(1, author)]);
    value.revision = 2;
    value.content = Content::Deleted;
    store.retain_recovery_record(&value).unwrap();
    assert!(store.retain_recovery_record(&record(1, author)).is_err());
    store
        .begin_recovery_import(&response(head), &manifest, true)
        .unwrap();
    store.import_recovery_page(0, &page, &objects).unwrap();
    store.finish_recovery_import().unwrap();
    assert!(matches!(
        store.recovery_record(value.id).unwrap().content,
        Content::Deleted
    ));
    let mut resurrection = record(1, author);
    resurrection.revision = 3;
    let (next, manifest, page, objects) = snapshot(&archive_key(), Some(&head), &[resurrection]);
    store
        .begin_recovery_import(&response(next), &manifest, false)
        .unwrap();
    store.import_recovery_page(0, &page, &objects).unwrap();
    assert!(matches!(
        store.finish_recovery_import(),
        Err(Error::Conflict)
    ));
    assert_eq!(store.recovery_status().unwrap().anchor, Some(head));
    store.cancel_recovery_import(next).unwrap();
    assert_eq!(store.recovery_status().unwrap().pending, None);
    assert!(matches!(
        store.recovery_record(value.id).unwrap().content,
        Content::Deleted
    ));
}

#[test]
fn restored_heads_wrong_scopes_and_incomplete_staging_fail_closed() {
    let dir = private_dir();
    let path = dir.path().join("client.db");
    let mut store = configured(&path);
    let author = DhKey::generate().unwrap().public_key();
    let (head, manifest, page, objects) = snapshot(&archive_key(), None, &[record(1, author)]);
    let mut restored = response(head);
    restored.restored_checkpoint = true;
    assert!(store
        .begin_recovery_import(&restored, &manifest, true)
        .is_err());
    restored = response(head);
    restored.generation += 1;
    assert!(store
        .begin_recovery_import(&restored, &manifest, true)
        .is_err());
    let wrong = RecoveryKey::from_secret(
        Secret32::from_bytes(SECRET),
        account_scope("other.example", ACCOUNT).unwrap(),
    )
    .unwrap();
    let (bad, bad_manifest, _, _) = snapshot(&wrong, None, &[record(1, author)]);
    assert!(store
        .begin_recovery_import(&response(bad), &bad_manifest, true)
        .is_err());
    store
        .begin_recovery_import(&response(head), &manifest, true)
        .unwrap();
    assert!(store.import_recovery_page(1, &page, &objects).is_err());
    store.import_recovery_page(0, &page, &objects).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute("DELETE FROM archive_import", []).unwrap();
    assert!(store.finish_recovery_import().is_err());
    assert_eq!(store.recovery_status().unwrap().anchor, None);
    store.import_recovery_page(0, &page, &objects).unwrap();
    store.finish_recovery_import().unwrap();
    let (fork, fork_manifest, _, _) = snapshot(&archive_key(), None, &[record(2, author)]);
    assert!(store
        .begin_recovery_import(&response(fork), &fork_manifest, true)
        .is_err());
    let (next, _, _, _) = snapshot(&archive_key(), Some(&head), &[record(1, author)]);
    let (skipped, skipped_manifest, _, _) =
        snapshot(&archive_key(), Some(&next), &[record(1, author)]);
    store
        .begin_recovery_import(&response(skipped), &skipped_manifest, true)
        .unwrap();
    assert_eq!(
        store.next_recovery_download().unwrap(),
        sigil_client::recovery::Download::Manifest { head: next }
    );
    assert!(store.finish_recovery_import().is_err());
    store.cancel_recovery_import(skipped).unwrap();
    assert_eq!(store.recovery_status().unwrap().anchor, Some(head));
    assert!(account_scope("CHAT.example", ACCOUNT).is_err());
    assert_ne!(
        account_scope("chat.example", ACCOUNT).unwrap(),
        account_scope("chat.example", [4; 32]).unwrap()
    );
    assert_ne!(
        account_scope("chat.example", ACCOUNT).unwrap(),
        account_scope("other.example", ACCOUNT).unwrap()
    );
}

#[test]
fn operator_restore_repairs_exact_trusted_checkpoint_and_preserves_local_deletions() {
    restored_upload(false, false);
}

#[test]
fn operator_restore_resolves_published_successor_with_lost_acknowledgement() {
    restored_upload(true, false);
}

#[test]
fn operator_restore_repairs_older_backup_without_rolling_back_client_history() {
    restored_upload(false, true);
}

fn restored_upload(published_before_backup: bool, newer_local_anchor: bool) {
    use sigil_protocol::{
        accounts::{Enrollment, InviteRequest},
        Configure,
    };
    use sigil_server::{auth::random_secret, store::Store};
    let dir = private_dir();
    let mut server = Store::open(&dir.path().join("server.db")).unwrap();
    server
        .configure(Configure {
            expected_revision: 0,
            settings: settings(),
        })
        .unwrap();
    let invite = server
        .invite(
            InviteRequest {
                username: "alice".into(),
                expires_in_seconds: 60,
            },
            1000,
        )
        .unwrap();
    let token = random_secret().unwrap();
    let session = server
        .enroll(
            Enrollment {
                invitation: invite.secret,
                device_credential: token.clone(),
                device_label: "Synthetic".into(),
            },
            1000,
        )
        .unwrap();
    let decode = |value: &str| -> Vec<u8> {
        value
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    };
    let account: [u8; 32] = decode(&session.account_id).try_into().unwrap();
    let mut source = open(&dir.path().join("source.db"));
    source
        .configure_recovery("chat.example", account, Secret32::from_bytes(SECRET))
        .unwrap();
    let author = DhKey::generate().unwrap().public_key();
    source.retain_recovery_record(&record(1, author)).unwrap();
    let head = source.prepare_recovery_upload(1000).unwrap();
    for object in source.pending_recovery_objects().unwrap() {
        server
            .put_recovery_object(
                &token,
                &hex(&object.id()),
                wire::PutObject {
                    ciphertext: hex(object.bytes()),
                },
                1000,
            )
            .unwrap();
        source
            .acknowledge_recovery_object(head, object.id())
            .unwrap();
    }
    let published = server
        .publish_recovery_head(&token, source.recovery_publication().unwrap(), 1000)
        .unwrap();
    source.acknowledge_recovery_head(&published).unwrap();
    let backup = dir.path().join("backup.db");
    let restored_path = dir.path().join("restored.db");
    server.backup(&backup).unwrap();
    let mut deleted = record(1, author);
    deleted.revision = 2;
    deleted.content = Content::Deleted;
    source.retain_recovery_record(&deleted).unwrap();
    source.retain_recovery_record(&record(2, author)).unwrap();
    let intended = source.prepare_recovery_upload(1001).unwrap();
    let frozen: BTreeMap<_, _> = source
        .pending_recovery_objects()
        .unwrap()
        .into_iter()
        .map(|object| (object.id(), object.bytes().to_vec()))
        .collect();
    for (id, bytes) in &frozen {
        server
            .put_recovery_object(
                &token,
                &hex(id),
                wire::PutObject {
                    ciphertext: hex(bytes),
                },
                1001,
            )
            .unwrap();
        source.acknowledge_recovery_object(intended, *id).unwrap();
    }
    assert!(source.pending_recovery_objects().unwrap().is_empty());
    if newer_local_anchor {
        let published = server
            .publish_recovery_head(&token, source.recovery_publication().unwrap(), 1001)
            .unwrap();
        source.acknowledge_recovery_head(&published).unwrap();
    }
    let backup = if published_before_backup {
        server
            .publish_recovery_head(&token, source.recovery_publication().unwrap(), 1001)
            .unwrap();
        let backup = dir.path().join("published-backup.db");
        server.backup(&backup).unwrap();
        backup
    } else {
        backup
    };
    if published_before_backup || newer_local_anchor {
        // Local deletion after the frozen snapshot must enter the repair snapshot.
        let mut later = record(2, author);
        later.revision = 2;
        later.content = Content::Deleted;
        source.retain_recovery_record(&later).unwrap();
        source.retain_recovery_record(&record(3, author)).unwrap();
    }
    drop(server);
    Store::restore(&backup, &restored_path).unwrap();
    let mut server = Store::open(&restored_path).unwrap();
    let invitation = server
        .invite_reauthorization(&session.account_id, 60, 1000)
        .unwrap();
    let replacement = random_secret().unwrap();
    server
        .reauthorize(
            Enrollment {
                invitation: invitation.secret,
                device_credential: replacement.clone(),
                device_label: "Synthetic new phone".into(),
            },
            1000,
        )
        .unwrap();
    assert!(server.recovery_head(&token, 1000).is_err());
    let restored = server.recovery_head(&replacement, 1000).unwrap();
    assert!(restored.restored_checkpoint);
    assert_eq!(
        restored.generation,
        if published_before_backup {
            intended.generation
        } else {
            head.generation
        }
    );
    for id in frozen.keys() {
        assert_eq!(
            server.recovery_object(&replacement, &hex(id), 1001).is_ok(),
            published_before_backup
        );
    }
    if published_before_backup {
        let before = source.recovery_status().unwrap();
        let mut fork = restored.clone();
        fork.manifest = Some(hex(&[42; 32]));
        assert!(source.reconcile_restored_recovery_head(&fork).is_err());
        let db = rusqlite::Connection::open(dir.path().join("source.db")).unwrap();
        db.execute(
            "UPDATE archive_objects SET data=zeroblob(length(data)) WHERE id=?1",
            [intended.manifest.as_slice()],
        )
        .unwrap();
        assert!(source.reconcile_restored_recovery_head(&restored).is_err());
        db.execute(
            "UPDATE archive_objects SET data=?1 WHERE id=?2",
            (&frozen[&intended.manifest], intended.manifest.as_slice()),
        )
        .unwrap();
        db.execute_batch("CREATE TRIGGER fail_resolution BEFORE UPDATE ON archive BEGIN SELECT RAISE(FAIL,'injected'); END;").unwrap();
        assert!(source.reconcile_restored_recovery_head(&restored).is_err());
        assert_eq!(source.recovery_status().unwrap(), before);
        assert_eq!(
            db.query_row("SELECT count(*) FROM archive_objects", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            frozen.len() as i64
        );
        db.execute_batch("DROP TRIGGER fail_resolution;").unwrap();
    }
    source.reconcile_restored_recovery_head(&restored).unwrap();
    drop(source);
    let mut source = open(&dir.path().join("source.db"));
    let head = source.prepare_recovery_upload(1001).unwrap();
    if published_before_backup || newer_local_anchor {
        assert_eq!(head.generation, intended.generation + 1);
        assert_eq!(source.recovery_status().unwrap().anchor, Some(intended));
    } else {
        assert_eq!(head, intended);
        let queued = source
            .pending_recovery_objects()
            .unwrap()
            .into_iter()
            .map(|object| (object.id(), object.bytes().to_vec()))
            .collect::<BTreeMap<_, _>>();
        for (id, bytes) in &frozen {
            assert_eq!(queued.get(id), Some(bytes));
        }
        assert_eq!(queued.len(), frozen.len() + 1); // Trusted predecessor proof.
    }
    assert!(source.recovery_publication().is_err());
    for object in source.pending_recovery_objects().unwrap() {
        server
            .put_recovery_object(
                &replacement,
                &hex(&object.id()),
                wire::PutObject {
                    ciphertext: hex(object.bytes()),
                },
                1001,
            )
            .unwrap();
        source
            .acknowledge_recovery_object(head, object.id())
            .unwrap();
    }
    let publication = source.recovery_publication().unwrap();
    assert!(publication.acknowledge_restored_checkpoint);
    let mut unacknowledged = source.recovery_publication().unwrap();
    unacknowledged.acknowledge_restored_checkpoint = false;
    assert!(server
        .publish_recovery_head(&replacement, unacknowledged, 1001)
        .is_err());
    let published = server
        .publish_recovery_head(&replacement, publication, 1001)
        .unwrap();
    assert!(!published.restored_checkpoint);
    // Simulate a lost publication response and restart: retry the exact bytes.
    drop(source);
    let mut source = open(&dir.path().join("source.db"));
    assert_eq!(source.prepare_recovery_upload(2000).unwrap(), head);
    assert_eq!(
        server
            .publish_recovery_head(&replacement, source.recovery_publication().unwrap(), 1001)
            .unwrap(),
        published
    );
    source.acknowledge_recovery_head(&published).unwrap();
    let mut fresh = open(&dir.path().join("fresh.db"));
    fresh
        .configure_recovery("chat.example", account, Secret32::from_bytes(SECRET))
        .unwrap();
    let key = RecoveryKey::from_secret(
        Secret32::from_bytes(SECRET),
        account_scope("chat.example", account).unwrap(),
    )
    .unwrap();
    let mut download = |id: [u8; 32]| {
        Object::from_bytes(decode(
            &server
                .recovery_object(&replacement, &hex(&id), 1000)
                .unwrap()
                .ciphertext,
        ))
        .unwrap()
    };
    let manifest_object = download(head.manifest);
    fresh
        .begin_recovery_import(&published, &manifest_object, true)
        .unwrap();
    let manifest = key.open_manifest(&head, manifest_object.bytes()).unwrap();
    for (index, reference) in manifest.pages.iter().enumerate() {
        let page = download(reference.object);
        let records: Vec<_> = key
            .open_page(reference, page.bytes())
            .unwrap()
            .iter()
            .map(|r| download(r.object))
            .collect();
        fresh.import_recovery_page(index, &page, &records).unwrap();
    }
    fresh.finish_recovery_import().unwrap();
    assert!(matches!(
        fresh.recovery_record(record(1, author).id).unwrap().content,
        Content::Deleted
    ));
    let second = fresh.recovery_record(record(2, author).id).unwrap();
    if published_before_backup || newer_local_anchor {
        assert!(matches!(second.content, Content::Deleted));
        assert!(fresh.recovery_record(record(3, author).id).is_ok());
    } else {
        assert!(matches!(second.content, Content::Retained(_)));
    }
    assert_eq!(fresh.recovery_status().unwrap().pending, None);
    let next = source.prepare_recovery_upload(1001).unwrap();
    assert_eq!(
        source.recovery_status().unwrap().pending,
        Some((Operation::Upload, next))
    );
    assert!(source.acknowledge_recovery_head(&published).is_err());
    for object in source.pending_recovery_objects().unwrap() {
        source
            .acknowledge_recovery_object(next, object.id())
            .unwrap();
    }
    assert!(
        !source
            .recovery_publication()
            .unwrap()
            .acknowledge_restored_checkpoint
    );
}

fn settings() -> sigil_protocol::Settings {
    sigil_protocol::Settings {
        server_name: "chat.example".into(),
        default_quota_bytes: sigil_protocol::DEFAULT_QUOTA,
        max_attachment_bytes: sigil_protocol::DEFAULT_ATTACHMENT_LIMIT,
    }
}

#[test]
fn competing_backup_merges_local_edits_and_keeps_pending_deletions() {
    let dir = private_dir();
    let path = dir.path().join("client.db");
    let mut store = configured(&path);
    let author = DhKey::generate().unwrap().public_key();
    let mut value = record(1, author);
    store.retain_recovery_record(&value).unwrap();
    let intended = store.prepare_recovery_upload(1000).unwrap();
    value.revision = 2;
    value.content = Content::Deleted;
    store.retain_recovery_record(&value).unwrap();
    let key = archive_key();
    let (winner, manifest, page, objects) = snapshot(&key, None, &[record(2, author)]);
    assert_ne!(winner.manifest, intended.manifest);
    let (later, later_manifest, _, _) = snapshot(&key, Some(&winner), &[record(2, author)]);
    assert!(store
        .begin_recovery_import(&response(later), &later_manifest, true)
        .is_err());
    assert_eq!(
        store.recovery_status().unwrap().pending,
        Some((Operation::Upload, intended))
    );
    store
        .begin_recovery_import(&response(winner), &manifest, false)
        .unwrap();
    assert!(store
        .acknowledge_recovery_head(&response(intended))
        .is_err());
    store.import_recovery_page(0, &page, &objects).unwrap();
    store.finish_recovery_import().unwrap();
    assert!(matches!(
        store.recovery_record(value.id).unwrap().content,
        Content::Deleted
    ));
    assert!(store.recovery_record(record(2, author).id).is_ok());
    let next = store.prepare_recovery_upload(1001).unwrap();
    assert_eq!(next.generation, 2);
    let queued: BTreeMap<_, _> = store
        .pending_recovery_objects()
        .unwrap()
        .into_iter()
        .map(|o| (o.id(), o))
        .collect();
    let manifest = key
        .open_manifest(&next, queued[&next.manifest].bytes())
        .unwrap();
    assert!(manifest.continues(&winner));
    let refs = key
        .open_page(
            &manifest.pages[0],
            queued[&manifest.pages[0].object].bytes(),
        )
        .unwrap();
    assert_eq!(refs.len(), 2);
    let deleted = key
        .open_record(&refs[0], queued[&refs[0].object].bytes())
        .unwrap();
    assert_eq!(deleted.revision, 2);
    assert!(matches!(deleted.content, Content::Deleted));
}

#[test]
fn restored_checkpoint_repair_rejects_rollback_and_preserves_pending_operations() {
    let dir = private_dir();
    let path = dir.path().join("client.db");
    let mut store = configured(&path);
    let (head, manifest, page, objects) = snapshot(
        &archive_key(),
        None,
        &[record(1, DhKey::generate().unwrap().public_key())],
    );
    let mut restored = response(head);
    restored.restored_checkpoint = true;
    assert!(matches!(
        store.reconcile_restored_recovery_head(&restored),
        Err(Error::Conflict)
    ));
    store
        .begin_recovery_import(&response(head), &manifest, true)
        .unwrap();
    assert!(store.reconcile_restored_recovery_head(&restored).is_err());
    store.import_recovery_page(0, &page, &objects).unwrap();
    store.finish_recovery_import().unwrap();
    assert!(store
        .reconcile_restored_recovery_head(&response(head))
        .is_err());
    for changed in [
        wire::Head {
            generation: 2,
            ..restored.clone()
        },
        wire::Head {
            manifest: Some(hex(&[0; 32])),
            ..restored.clone()
        },
        wire::Head {
            generation: 0,
            manifest: None,
            restored_checkpoint: true,
        },
    ] {
        assert!(store.reconcile_restored_recovery_head(&changed).is_err());
        assert_eq!(store.recovery_status().unwrap().anchor, Some(head));
    }
    // A failed marker write cannot authorize later publication.
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_repair BEFORE UPDATE ON archive BEGIN SELECT RAISE(FAIL,'injected'); END;").unwrap();
    assert!(store.reconcile_restored_recovery_head(&restored).is_err());
    db.execute_batch("DROP TRIGGER fail_repair;").unwrap();
    let next = store.prepare_recovery_upload(1001).unwrap();
    assert_eq!(
        store.recovery_status().unwrap().pending,
        Some((Operation::Upload, next))
    );
    for object in store.pending_recovery_objects().unwrap() {
        store
            .acknowledge_recovery_object(next, object.id())
            .unwrap();
    }
    assert!(
        !store
            .recovery_publication()
            .unwrap()
            .acknowledge_restored_checkpoint
    );
    db.execute_batch("CREATE TRIGGER fail_repair BEFORE UPDATE ON archive BEGIN SELECT RAISE(FAIL,'injected'); END;").unwrap();
    assert!(store.reconcile_restored_recovery_head(&restored).is_err());
    assert!(store.pending_recovery_objects().unwrap().is_empty());
    assert!(
        !store
            .recovery_publication()
            .unwrap()
            .acknowledge_restored_checkpoint
    );
    db.execute_batch("DROP TRIGGER fail_repair;").unwrap();
    store.reconcile_restored_recovery_head(&restored).unwrap();
    assert_eq!(store.prepare_recovery_upload(2000).unwrap(), next);
    assert!(store.recovery_publication().is_err());
    for object in store.pending_recovery_objects().unwrap() {
        store
            .acknowledge_recovery_object(next, object.id())
            .unwrap();
    }
    assert!(
        store
            .recovery_publication()
            .unwrap()
            .acknowledge_restored_checkpoint
    );
    store.acknowledge_recovery_head(&response(next)).unwrap();
    store.reconcile_restored_recovery_head(&restored).unwrap();
    restored = response(next);
    restored.restored_checkpoint = true;
    store.reconcile_restored_recovery_head(&restored).unwrap();
    store.reconcile_restored_recovery_head(&restored).unwrap();
    drop(store);
    let mut store = open(&path);
    assert!(store
        .begin_recovery_import(&response(head), &manifest, true)
        .is_err());
    let successor = store.prepare_recovery_upload(1002).unwrap();
    for object in store.pending_recovery_objects().unwrap() {
        store
            .acknowledge_recovery_object(successor, object.id())
            .unwrap();
    }
    assert!(
        store
            .recovery_publication()
            .unwrap()
            .acknowledge_restored_checkpoint
    );
    // A rejected local commit retains the exact authorized repair for retry.
    db.execute_batch("CREATE TRIGGER fail_ack BEFORE UPDATE ON archive BEGIN SELECT RAISE(FAIL,'injected'); END;").unwrap();
    assert!(store
        .acknowledge_recovery_head(&response(successor))
        .is_err());
    assert_eq!(store.recovery_status().unwrap().anchor, Some(next));
    assert!(
        store
            .recovery_publication()
            .unwrap()
            .acknowledge_restored_checkpoint
    );
    db.execute_batch("DROP TRIGGER fail_ack;").unwrap();
    store
        .acknowledge_recovery_head(&response(successor))
        .unwrap();
    assert_eq!(store.recovery_status().unwrap().anchor, Some(successor));
}

#[test]
fn restored_ancestor_proof_is_bounded_authenticated_and_migration_fails_closed() {
    let dir = private_dir();
    let path = dir.path().join("ancestry.db");
    let mut store = configured(&path);
    let mut heads = Vec::new();
    for time in 1..=70 {
        let head = store.prepare_recovery_upload(time).unwrap();
        for object in store.pending_recovery_objects().unwrap() {
            store
                .acknowledge_recovery_object(head, object.id())
                .unwrap();
        }
        store.acknowledge_recovery_head(&response(head)).unwrap();
        heads.push(head);
    }
    let db = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM archive_checkpoints", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        64
    );
    let mut restored = response(heads[4]);
    restored.restored_checkpoint = true;
    assert!(store.reconcile_restored_recovery_head(&restored).is_err());
    restored = response(heads[5]);
    restored.restored_checkpoint = true;
    let mut fork = restored.clone();
    fork.manifest = Some(hex(&[42; 32]));
    assert!(store.reconcile_restored_recovery_head(&fork).is_err());
    let bytes: Vec<u8> = db
        .query_row(
            "SELECT data FROM archive_checkpoints WHERE generation=40",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db.execute(
        "UPDATE archive_checkpoints SET data=zeroblob(length(data)) WHERE generation=40",
        [],
    )
    .unwrap();
    assert!(store.reconcile_restored_recovery_head(&restored).is_err());
    assert_eq!(store.recovery_status().unwrap().anchor, Some(heads[69]));
    db.execute(
        "UPDATE archive_checkpoints SET data=?1 WHERE generation=40",
        [bytes],
    )
    .unwrap();
    store.reconcile_restored_recovery_head(&restored).unwrap();
    drop(store);
    let mut store = open(&path);
    let pending = store.prepare_recovery_upload(71).unwrap();
    assert_eq!(pending.generation, 71);
    while !store.pending_recovery_objects().unwrap().is_empty() {
        for object in store.pending_recovery_objects().unwrap() {
            store
                .acknowledge_recovery_object(pending, object.id())
                .unwrap();
        }
    }
    let publication = store.recovery_publication().unwrap();
    assert_eq!(publication.expected_generation, 6);
    assert_eq!(publication.restore_generation, Some(71));
    store.acknowledge_recovery_head(&response(pending)).unwrap();
    drop(store);
    common::rewind(&db, 17);
    let mut store = open(&path);
    assert_eq!(store.recovery_status().unwrap().anchor, Some(pending));
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        55
    );
    // Migration cannot invent evidence for publications predating the ledger.
    assert!(store.reconcile_restored_recovery_head(&restored).is_err());
    let mut current = response(pending);
    current.restored_checkpoint = true;
    store.reconcile_restored_recovery_head(&current).unwrap();
}

#[test]
fn multi_generation_import_checks_each_link_and_resumes_without_exposing_history() {
    use sigil_client::recovery::Download;
    let dir = private_dir();
    let path = dir.path().join("catchup.db");
    let mut store = configured(&path);
    let author = DhKey::generate().unwrap().public_key();
    let key = archive_key();
    let first = snapshot(&key, None, &[record(1, author)]);
    store
        .begin_recovery_import(&response(first.0), &first.1, true)
        .unwrap();
    store.import_recovery_page(0, &first.2, &first.3).unwrap();
    store.finish_recovery_import().unwrap();
    let mut prior = first.0;
    let mut chain = Vec::new();
    for _ in 0..4 {
        let next = snapshot(&key, Some(&prior), &[record(2, author)]);
        prior = next.0;
        chain.push(next);
    }
    let last = chain.last().unwrap();
    store
        .begin_recovery_import(&response(last.0), &last.1, false)
        .unwrap();
    assert!(store.import_recovery_page(0, &last.2, &last.3).is_err());
    assert!(store.retain_recovery_record(&record(3, author)).is_err());
    for index in (0..3).rev() {
        assert_eq!(
            store.next_recovery_download().unwrap(),
            Download::Manifest {
                head: chain[index].0
            }
        );
        let mut bad = chain[index].1.bytes().to_vec();
        bad[40] ^= 1;
        assert!(store
            .stage_recovery_manifest(&Object::from_bytes(bad).unwrap())
            .is_err());
        assert!(store.stage_recovery_manifest(&first.1).is_err());
        store.stage_recovery_manifest(&chain[index].1).unwrap();
        drop(store);
        store = open(&path);
        assert_eq!(store.recovery_status().unwrap().anchor, Some(first.0));
        assert!(store.recovery_record(record(2, author).id).is_err());
    }
    store.import_recovery_page(0, &last.2, &last.3).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_finish BEFORE UPDATE ON archive BEGIN SELECT RAISE(FAIL,'injected'); END;").unwrap();
    assert!(store.finish_recovery_import().is_err());
    assert_eq!(store.recovery_status().unwrap().anchor, Some(first.0));
    assert!(store.recovery_record(record(2, author).id).is_err());
    db.execute_batch("DROP TRIGGER fail_finish;").unwrap();
    assert_eq!(store.finish_recovery_import().unwrap(), last.0);
    assert!(store.recovery_record(record(2, author).id).is_ok());
    assert_eq!(
        db.query_row("SELECT count(*) FROM archive_checkpoints", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        5
    );
    // A valid chain on another branch cannot terminate at the trusted anchor.
    let fork = snapshot(&key, Some(&first.0), &[record(3, author)]);
    let mut fork_head = fork.0;
    let mut forks = vec![fork];
    while fork_head.generation < 7 {
        let next = snapshot(&key, Some(&fork_head), &[record(3, author)]);
        fork_head = next.0;
        forks.push(next);
    }
    let top = forks.last().unwrap();
    store
        .begin_recovery_import(&response(top.0), &top.1, false)
        .unwrap();
    assert!(store
        .stage_recovery_manifest(&forks[forks.len() - 2].1)
        .is_err());
    assert_eq!(store.recovery_status().unwrap().anchor, Some(last.0));
    store.cancel_recovery_import(top.0).unwrap();
    let too_far = snapshot(
        &key,
        Some(&Head {
            generation: last.0.generation + 64,
            manifest: last.0.manifest,
        }),
        &[record(3, author)],
    );
    assert!(store
        .begin_recovery_import(&response(too_far.0), &too_far.1, false)
        .is_err());
}

#[test]
fn competing_ancestry_preserves_pending_upload_until_atomic_handover() {
    let dir = private_dir();
    let path = dir.path().join("competing.db");
    let mut store = configured(&path);
    let key = archive_key();
    let author = DhKey::generate().unwrap().public_key();
    let first = snapshot(&key, None, &[record(1, author)]);
    store
        .begin_recovery_import(&response(first.0), &first.1, true)
        .unwrap();
    store.import_recovery_page(0, &first.2, &first.3).unwrap();
    store.finish_recovery_import().unwrap();
    let intended = store.prepare_recovery_upload(1001).unwrap();
    let frozen: BTreeMap<_, _> = store
        .pending_recovery_objects()
        .unwrap()
        .into_iter()
        .map(|o| (o.id(), o.bytes().to_vec()))
        .collect();
    let winner = snapshot(&key, Some(&first.0), &[record(2, author)]);
    let latest = snapshot(&key, Some(&winner.0), &[record(2, author)]);
    store
        .begin_recovery_competition(&response(latest.0), &latest.1)
        .unwrap();
    assert!(store.finish_recovery_competition().is_err());
    drop(store);
    let mut store = open(&path);
    assert_eq!(
        store.next_recovery_competition_manifest().unwrap(),
        Some(winner.0)
    );
    assert!(store.stage_recovery_competition_manifest(&first.1).is_err());
    store.cancel_recovery_competition(latest.0).unwrap();
    let fork = snapshot(
        &key,
        Some(&Head {
            generation: first.0.generation,
            manifest: [42; 32],
        }),
        &[record(2, author)],
    );
    let fork_tip = snapshot(&key, Some(&fork.0), &[record(2, author)]);
    store
        .begin_recovery_competition(&response(fork_tip.0), &fork_tip.1)
        .unwrap();
    assert!(store.stage_recovery_competition_manifest(&fork.1).is_err());
    assert_eq!(
        store.next_recovery_competition_manifest().unwrap(),
        Some(fork.0)
    );
    store.cancel_recovery_competition(fork_tip.0).unwrap();
    let far = snapshot(
        &key,
        Some(&Head {
            generation: first.0.generation + 64,
            manifest: first.0.manifest,
        }),
        &[record(2, author)],
    );
    assert!(store
        .begin_recovery_competition(&response(far.0), &far.1)
        .is_err());
    assert_eq!(
        store.recovery_status().unwrap().pending,
        Some((Operation::Upload, intended))
    );
    assert_eq!(
        store
            .pending_recovery_objects()
            .unwrap()
            .into_iter()
            .map(|o| (o.id(), o.bytes().to_vec()))
            .collect::<BTreeMap<_, _>>(),
        frozen
    );
    store
        .begin_recovery_competition(&response(latest.0), &latest.1)
        .unwrap();
    store
        .stage_recovery_competition_manifest(&winner.1)
        .unwrap();
    let mut deleted = record(1, author);
    deleted.revision = 2;
    deleted.content = Content::Deleted;
    store.retain_recovery_record(&deleted).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_handover BEFORE UPDATE ON archive BEGIN SELECT RAISE(FAIL,'injected'); END;").unwrap();
    assert!(store.finish_recovery_competition().is_err());
    assert_eq!(
        store.recovery_status().unwrap().pending,
        Some((Operation::Upload, intended))
    );
    assert_eq!(
        store
            .pending_recovery_objects()
            .unwrap()
            .into_iter()
            .map(|o| (o.id(), o.bytes().to_vec()))
            .collect::<BTreeMap<_, _>>(),
        frozen
    );
    assert_eq!(store.next_recovery_competition_manifest().unwrap(), None);
    db.execute_batch("DROP TRIGGER fail_handover;").unwrap();
    // A changed committed archive state invalidates a separately staged proof.
    for object in store.pending_recovery_objects().unwrap() {
        store
            .acknowledge_recovery_object(intended, object.id())
            .unwrap();
    }
    store
        .acknowledge_recovery_head(&response(intended))
        .unwrap();
    assert!(store.finish_recovery_competition().is_err());
    assert_eq!(store.recovery_competition().unwrap(), Some(latest.0));
    store.cancel_recovery_competition(latest.0).unwrap();
    assert_eq!(store.recovery_status().unwrap().anchor, Some(intended));
    // Repeat from an unchanged original anchor to exercise successful handover.
    let mut clean = configured(&dir.path().join("clean.db"));
    clean
        .begin_recovery_import(&response(first.0), &first.1, true)
        .unwrap();
    clean.import_recovery_page(0, &first.2, &first.3).unwrap();
    clean.finish_recovery_import().unwrap();
    clean.prepare_recovery_upload(1001).unwrap();
    clean.retain_recovery_record(&deleted).unwrap();
    clean
        .begin_recovery_competition(&response(latest.0), &latest.1)
        .unwrap();
    clean
        .stage_recovery_competition_manifest(&winner.1)
        .unwrap();
    clean.finish_recovery_competition().unwrap();
    assert_eq!(clean.recovery_status().unwrap().anchor, Some(first.0));
    assert!(clean.recovery_record(record(2, author).id).is_err());
    clean.import_recovery_page(0, &latest.2, &latest.3).unwrap();
    clean.finish_recovery_import().unwrap();
    assert_eq!(clean.recovery_status().unwrap().anchor, Some(latest.0));
    assert!(matches!(
        clean.recovery_record(deleted.id).unwrap().content,
        Content::Deleted
    ));
    assert!(clean.recovery_record(record(2, author).id).is_ok());
}
