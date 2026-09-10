use super::*;
use crate::network::tests::{Fixture, CA};
use sigil_crypto::{triple::Session, DhKey, Secret32};
use sigil_server::{auth::AdminToken, store::Store};
use std::fs;

fn open(path: &Path) -> ClientStore {
    ClientStore::open(
        path,
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap()
}
#[test]
fn connected_requests_share_discovery_and_invalidate_on_credential_rotation() {
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
    let (dir, old, invite, _) = setup();
    drop(old);
    let discoveries = Arc::new(AtomicUsize::new(0));
    let hits = discoveries.clone();
    let router = sigil_server::router(
        Store::open(&dir.path().join("server.db")).unwrap(),
        AdminToken::load_or_create(&dir.path().join("admin.token")).unwrap(),
    ).layer(axum::middleware::from_fn(move |request: axum::extract::Request, next: axum::middleware::Next| {
        let hits = hits.clone();
        async move {
            if request.uri().path() == sigil_protocol::discovery::PATH { hits.fetch_add(1, Ordering::SeqCst); }
            next.run(request).await
        }
    }));
    let fixture = Fixture::new(router);
    let mut store = open(&dir.path().join("client.db"));
    prepare(&mut store, &fixture, &invite.secret);
    store.enroll_online().unwrap();
    discoveries.store(0, Ordering::SeqCst);
    let old = store.connected_client().unwrap();
    old.session().unwrap();
    store.connected_client().unwrap().session().unwrap();
    assert_eq!(discoveries.load(Ordering::SeqCst), 1);
    store.prepare_credential_rotation().unwrap();
    assert!(matches!(store.connected_client(), Err(Error::Unprepared)));
    store.rotate_credential_online().unwrap();
    discoveries.store(0, Ordering::SeqCst);
    store.connected_client().unwrap().session().unwrap();
    store.connected_client().unwrap().session().unwrap();
    assert_eq!(discoveries.load(Ordering::SeqCst), 1);
    assert!(matches!(old.session(), Err(network::Error::Status { code: 401, .. })));
    store.connection.borrow_mut().as_mut().unwrap().1 = std::time::Instant::now() - std::time::Duration::from_secs(60);
    store.connected_client().unwrap().session().unwrap();
    assert_eq!(discoveries.load(Ordering::SeqCst), 2);
}
pub(crate) fn setup() -> (tempfile::TempDir, Fixture, accounts::Invitation, u64) {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut server = Store::open(&dir.path().join("server.db")).unwrap();
    server
        .configure(sigil_protocol::Configure {
            expected_revision: 0,
            settings: sigil_protocol::Settings {
                server_name: "chat.example".into(),
                default_quota_bytes: sigil_protocol::DEFAULT_QUOTA,
                max_attachment_bytes: sigil_protocol::DEFAULT_ATTACHMENT_LIMIT,
            },
        })
        .unwrap();
    let invite = server
        .invite(
            accounts::InviteRequest {
                username: "alice".into(),
                expires_in_seconds: 60,
            },
            now,
        )
        .unwrap();
    let fixture = Fixture::new(sigil_server::router(
        server,
        AdminToken::load_or_create(&dir.path().join("admin.token")).unwrap(),
    ));
    (dir, fixture, invite, now)
}
pub(crate) fn prepare(store: &mut ClientStore, fixture: &Fixture, invitation: &str) {
    store
        .prepare_enrollment(
            "chat.example",
            fixture.port(),
            &[CA.to_vec()],
            invitation,
            "Synthetic",
            false,
        )
        .unwrap();
}

#[test]
fn enrollment_commit_failure_recovers_the_original_session_after_restart() {
    let (dir, fixture, invitation, now) = setup();
    let path = dir.path().join("client.db");
    let mut store = open(&path);
    prepare(&mut store, &fixture, &invitation.secret);
    assert!(matches!(store.connected_client(), Err(Error::Unprepared)));
    let (profile, before) = load(&store.db, &store.key).unwrap();
    assert!(!before
        .windows(64)
        .any(|bytes| bytes == profile.credential.as_bytes()));
    store.db.execute_batch("CREATE TRIGGER fail_enrollment BEFORE UPDATE ON connection BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(store.enroll_online().is_err());
    assert_eq!(store.connection_session().unwrap(), None);
    assert_eq!(load(&store.db, &store.key).unwrap().1, before);
    let server = Store::open(&dir.path().join("server.db")).unwrap();
    let enrolled = server.session(&profile.credential, now).unwrap();
    assert!(store.cancel_unused_enrollment().is_err());
    assert_eq!(load(&store.db, &store.key).unwrap().1, before);
    store
        .db
        .execute_batch("DROP TRIGGER fail_enrollment;")
        .unwrap();
    drop(store);
    store = open(&path);
    assert_eq!(store.enroll_online().unwrap(), enrolled);
    assert_eq!(store.connection_session().unwrap(), Some(enrolled));
    let (after, _) = load(&store.db, &store.key).unwrap();
    assert_eq!(profile.credential, after.credential);
    assert!(after.invitation.is_none());
    assert!(store.connected_client().unwrap().session().is_ok());
}

#[test]
fn cancelling_unused_login_preserves_uncertain_credentials_and_existing_keys() {
    let (dir, fixture, invitation, _) = setup();
    let mut empty = open(&dir.path().join("unused.db"));
    prepare(&mut empty, &fixture, &invitation.secret);
    empty.cancel_unused_enrollment().unwrap();
    assert_eq!(empty.enrollment_kind().unwrap(), "new");
    empty.cancel_unused_enrollment().unwrap();
    assert_eq!(
        empty
            .db
            .query_row("SELECT count(*) FROM connection_roots", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    prepare(&mut empty, &fixture, &invitation.secret);
    let before = load(&empty.db, &empty.key).unwrap().1;
    let identity = empty.identity().unwrap();
    assert!(empty.cancel_unused_enrollment().is_err());
    assert_eq!(empty.identity().unwrap(), identity);
    assert_eq!(load(&empty.db, &empty.key).unwrap().1, before);
    let mut offline = open(&dir.path().join("offline.db"));
    prepare(&mut offline, &fixture, &invitation.secret);
    let before = load(&offline.db, &offline.key).unwrap().1;
    drop(fixture);
    assert!(offline.cancel_unused_enrollment().is_err());
    assert_eq!(load(&offline.db, &offline.key).unwrap().1, before);
}

#[test]
fn credential_rotation_recovers_after_server_commit_and_local_failure() {
    let (dir, fixture, invitation, now) = setup();
    let path = dir.path().join("client.db");
    let mut store = open(&path);
    prepare(&mut store, &fixture, &invitation.secret);
    let enrolled = store.enroll_online().unwrap();
    let (original, _) = load(&store.db, &store.key).unwrap();
    store.prepare_credential_rotation().unwrap();
    let (pending, frozen) = load(&store.db, &store.key).unwrap();
    store.prepare_credential_rotation().unwrap();
    assert_eq!(load(&store.db, &store.key).unwrap().1, frozen);
    assert!(matches!(store.connected_client(), Err(Error::Unprepared)));
    store.db.execute_batch("CREATE TRIGGER fail_rotation BEFORE UPDATE ON connection BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(store.rotate_credential_online().is_err());
    assert_eq!(load(&store.db, &store.key).unwrap().1, frozen);
    let server = Store::open(&dir.path().join("server.db")).unwrap();
    assert!(server.session(&original.credential, now).is_err());
    assert!(server
        .session(pending.rotation.as_deref().unwrap(), now)
        .is_ok());
    store
        .db
        .execute_batch("DROP TRIGGER fail_rotation;")
        .unwrap();
    drop(store);
    store = open(&path);
    let rotated = store.rotate_credential_online().unwrap();
    assert_eq!(rotated.device_id, enrolled.device_id);
    assert_eq!(rotated.account_id, enrolled.account_id);
    let (after, _) = load(&store.db, &store.key).unwrap();
    assert!(after.rotation.is_none());
    assert_eq!(after.credential, pending.rotation.unwrap());
    assert!(store.connected_client().unwrap().session().is_ok());
}

#[test]
fn real_https_outbox_retry_preserves_packet_until_local_receipt_commits() {
    let (dir, fixture, invitation, now) = setup();
    let path = dir.path().join("client.db");
    let mut store = open(&path);
    prepare(&mut store, &fixture, &invitation.secret);
    let account = store.enroll_online().unwrap();
    let dh = DhKey::generate().unwrap();
    let sending =
        Session::initiator(Secret32::from_bytes([7; 32]), dh.public_key(), [8; 32]).unwrap();
    let mut receiving = Session::responder(Secret32::from_bytes([7; 32]), dh, [8; 32]).unwrap();
    let session = [1; 32];
    let message = [2; 32];
    store.insert_session(session, sending).unwrap();
    let packet = store
        .send(session, message, b"Synthetic HTTPS delivery")
        .unwrap();
    store
        .prepare_delivery(
            session,
            message,
            decode_id(&account.device_id).unwrap(),
            now + 60,
            now,
        )
        .unwrap();
    store.db.execute_batch("CREATE TRIGGER fail_receipt BEFORE UPDATE OF receipt ON deliveries BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(store.send_pending_online(session, now).is_err());
    assert_eq!(
        store.pending(session).unwrap(),
        vec![(message, packet.clone())]
    );
    let network = store.connected_client().unwrap();
    let delivery = network.mailbox().unwrap().remove(0);
    assert_eq!(delivery.payload, super::super::transport::hex(&packet));
    store
        .db
        .execute_batch("DROP TRIGGER fail_receipt;")
        .unwrap();
    drop(store);
    store = open(&path);
    assert_eq!(store.send_pending_online(session, now).unwrap().accepted, 1);
    assert!(store.pending(session).unwrap().is_empty());
    let deliveries = network.mailbox().unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].sequence, delivery.sequence);
    assert_eq!(
        receiving
            .receive(&sigil_crypto::triple::Packet::from_bytes(&packet).unwrap())
            .unwrap(),
        b"Synthetic HTTPS delivery"
    );
    network.acknowledge_delivery(delivery.sequence).unwrap();
    assert_eq!(store.send_pending_online(session, now).unwrap().accepted, 0);
}

#[test]
fn initialization_and_root_failures_do_not_replace_existing_credentials() {
    let (dir, fixture, invitation, _) = setup();
    let path = dir.path().join("client.db");
    let mut store = open(&path);
    store.db.execute_batch("CREATE TRIGGER fail_root BEFORE INSERT ON connection_roots BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(store
        .prepare_enrollment(
            "chat.example",
            fixture.port(),
            &[CA.to_vec()],
            &invitation.secret,
            "Synthetic",
            false
        )
        .is_err());
    assert!(matches!(store.connection_session(), Err(Error::NotFound)));
    store.db.execute_batch("DROP TRIGGER fail_root;").unwrap();
    prepare(&mut store, &fixture, &invitation.secret);
    let before = load(&store.db, &store.key).unwrap().1;
    assert!(store
        .prepare_enrollment(
            "chat.example",
            fixture.port(),
            &[CA.to_vec()],
            &invitation.secret,
            "Synthetic",
            false
        )
        .is_err());
    assert_eq!(load(&store.db, &store.key).unwrap().1, before);
    store
        .db
        .execute(
            "UPDATE connection_roots SET data=zeroblob(length(data))",
            [],
        )
        .unwrap();
    assert!(store.enroll_online().is_err());
    assert_eq!(load(&store.db, &store.key).unwrap().1, before);
    let mut live = open(&dir.path().join("live.db"));
    live.identity().unwrap();
    assert!(live
        .prepare_enrollment(
            "chat.example",
            fixture.port(),
            &[],
            &invitation.secret,
            "Synthetic",
            true
        )
        .is_err());
}

#[test]
fn lost_phone_history_recovers_over_https_with_per_record_restart_progress() {
    use crate::recovery::Download;
    use sigil_crypto::recovery::{Content, Direction, Record};
    let (dir, fixture, invitation, now) = setup();
    let mut source = open(&dir.path().join("source.db"));
    prepare(&mut source, &fixture, &invitation.secret);
    let session = source.enroll_online().unwrap();
    let account = decode_id(&session.account_id).unwrap();
    source
        .configure_recovery("chat.example", account, Secret32::from_bytes([7; 32]))
        .unwrap();
    let author = DhKey::generate().unwrap().public_key();
    for number in 1..=17 {
        source
            .retain_recovery_record(&Record {
                id: [number; 32],
                revision: 1,
                conversation: [5; 32],
                author,
                created_at: now,
                direction: Direction::Outgoing,
                content: Content::Retained(Zeroizing::new(vec![number; 100])),
            })
            .unwrap();
    }
    let head = source.prepare_recovery_upload(now).unwrap();
    assert_eq!(source.upload_recovery_step().unwrap(), None);
    assert_eq!(source.upload_recovery_step().unwrap(), Some(head));
    let mut server = Store::open(&dir.path().join("server.db")).unwrap();
    let invitation = server
        .invite_reauthorization(&session.account_id, 60, now)
        .unwrap();
    let path = dir.path().join("replacement.db");
    let mut replacement = open(&path);
    replacement
        .prepare_enrollment(
            "chat.example",
            fixture.port(),
            &[CA.to_vec()],
            &invitation.secret,
            "Synthetic replacement",
            true,
        )
        .unwrap();
    let reauthorized = replacement.enroll_online().unwrap();
    assert_eq!(reauthorized.account_id, session.account_id);
    assert_ne!(reauthorized.device_id, session.device_id);
    replacement
        .configure_recovery("chat.example", account, Secret32::from_bytes([7; 32]))
        .unwrap();
    assert_eq!(replacement.download_recovery_step(true).unwrap(), None);
    replacement.db.execute_batch("CREATE TRIGGER fail_fourth BEFORE INSERT ON archive_import WHEN substr(hex(NEW.id),1,2)='04' BEGIN SELECT RAISE(ABORT,'synthetic interruption'); END;").unwrap();
    assert!(replacement.download_recovery_step(false).is_err());
    assert_eq!(
        replacement
            .db
            .query_row("SELECT count(*) FROM archive_import", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert!(replacement.recovery_record([1; 32]).is_err());
    replacement
        .db
        .execute_batch("DROP TRIGGER fail_fourth;")
        .unwrap();
    drop(replacement);
    replacement = open(&path);
    match replacement.next_recovery_download().unwrap() {
        Download::Records { index, records } => {
            assert_eq!(index, 0);
            assert_eq!(records[0].id, [4; 32]);
            assert_eq!(records.len(), 14);
        }
        other => panic!("unexpected progress {other:?}"),
    }
    assert_eq!(replacement.download_recovery_step(false).unwrap(), None);
    assert_eq!(
        replacement.download_recovery_step(false).unwrap(),
        Some(head)
    );
    assert!(
        matches!(replacement.recovery_record([17; 32]).unwrap().content, Content::Retained(bytes) if bytes.as_slice() == [17; 100])
    );
    for table in ["sessions", "identity", "prekeys", "inbox", "outbox"] {
        assert_eq!(
            replacement
                .db
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn archive_upload_rate_rejection_preserves_acknowledged_object_progress() {
    use sigil_crypto::recovery::{Content, Direction, Record};
    let (dir, fixture, invitation, now) = setup();
    let path = dir.path().join("client.db");
    let mut store = open(&path);
    prepare(&mut store, &fixture, &invitation.secret);
    let session = store.enroll_online().unwrap();
    store
        .configure_recovery(
            "chat.example",
            decode_id(&session.account_id).unwrap(),
            Secret32::from_bytes([7; 32]),
        )
        .unwrap();
    let author = DhKey::generate().unwrap().public_key();
    for number in 1..=96 {
        store
            .retain_recovery_record(&Record {
                id: [number; 32],
                revision: 1,
                conversation: [5; 32],
                author,
                created_at: now,
                direction: Direction::Incoming,
                content: Content::Retained(Zeroizing::new(vec![number])),
            })
            .unwrap();
    }
    store.prepare_recovery_upload(now).unwrap();
    assert_eq!(store.upload_recovery_step().unwrap(), None);
    let rejected = loop {
        match store.upload_recovery_step() {
            Ok(None) => (),
            result => break result,
        }
    };
    assert!(matches!(
        rejected,
        Err(Error::Network(network::Error::Status {
            code: 429,
            retry_after_seconds: Some(_)
        }))
    ));
    let uploaded: i64 = store
        .db
        .query_row(
            "SELECT count(*) FROM archive_objects WHERE uploaded=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!((60..98).contains(&uploaded));
    assert_eq!(store.recovery_status().unwrap().anchor, None);
    drop(store);
    store = open(&path);
    assert_eq!(
        store
            .db
            .query_row(
                "SELECT count(*) FROM archive_objects WHERE uploaded=1",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        uploaded
    );
    assert!(store.recovery_publication().is_err());
}

pub(crate) fn credential(store: &ClientStore) -> Zeroizing<String> {
    load(&store.db, &store.key).unwrap().0.credential
}

#[test]
fn trusted_checkpoint_repair_uses_authenticated_account_over_https() {
    trusted_checkpoint_repair(false);
}

#[test]
fn published_checkpoint_resolution_survives_handoff_and_https_repair() {
    trusted_checkpoint_repair(true);
}

fn trusted_checkpoint_repair(published: bool) {
    use sigil_crypto::recovery::{Content, Direction, Record};
    let (dir, fixture, invitation, now) = setup();
    let path = dir.path().join("repair.db");
    let mut store = open(&path);
    prepare(&mut store, &fixture, &invitation.secret);
    let session = store.enroll_online().unwrap();
    store
        .configure_recovery(
            "chat.example",
            decode_id(&session.account_id).unwrap(),
            Secret32::from_bytes([7; 32]),
        )
        .unwrap();
    let mut retained = Record {
        id: [1; 32],
        revision: 1,
        conversation: [5; 32],
        author: store.identity().unwrap(),
        created_at: now,
        direction: Direction::Outgoing,
        content: Content::Retained(Zeroizing::new(b"Synthetic history".to_vec())),
    };
    store.retain_recovery_record(&retained).unwrap();
    let head = store.prepare_recovery_upload(now).unwrap();
    assert_eq!(store.upload_recovery_step().unwrap(), Some(head));
    assert!(store.reconcile_restored_recovery().is_err());
    // Isolate the HTTPS adapter's restored-head handling. The storage integration
    // test separately performs actual backup/restore and credential revocation.
    let server = rusqlite::Connection::open(dir.path().join("server.db")).unwrap();
    server
        .execute("UPDATE recovery_heads SET restored_checkpoint=1", [])
        .unwrap();
    let mut control = Store::open(&dir.path().join("server.db")).unwrap();
    let invitation = control
        .invite_reauthorization(&session.account_id, 60, now)
        .unwrap();
    let replacement_path = dir.path().join("replacement-repair.db");
    let reopen = || {
        ClientStore::open(
            &replacement_path,
            StorageKey::new(Secret32::from_bytes([11; 32])).unwrap(),
        )
        .unwrap()
    };
    let mut replacement = reopen();
    replacement
        .prepare_enrollment(
            "chat.example",
            fixture.port(),
            &[CA.to_vec()],
            &invitation.secret,
            "Synthetic replacement",
            true,
        )
        .unwrap();
    replacement.enroll_online().unwrap();
    let mut wrong_scope = open(&dir.path().join("wrong-scope.db"));
    wrong_scope
        .configure_recovery(
            "other.example",
            decode_id(&session.account_id).unwrap(),
            Secret32::from_bytes([7; 32]),
        )
        .unwrap();
    let wrong_head = wrong_scope.prepare_recovery_upload(now).unwrap();
    assert!(wrong_scope
        .copy_recovery_history_to(&mut replacement)
        .is_err());
    for object in wrong_scope.pending_recovery_objects().unwrap() {
        wrong_scope
            .acknowledge_recovery_object(wrong_head, object.id())
            .unwrap();
    }
    wrong_scope
        .acknowledge_recovery_head(&sigil_protocol::recovery::Head {
            generation: wrong_head.generation,
            manifest: Some(super::super::transport::hex(&wrong_head.manifest)),
            restored_checkpoint: false,
        })
        .unwrap();
    assert!(wrong_scope
        .copy_recovery_history_to(&mut replacement)
        .is_err());
    assert!(matches!(
        replacement.recovery_status(),
        Err(Error::NotFound)
    ));
    assert!(store.reconcile_restored_recovery().is_err());
    retained.revision = 2;
    retained.content = Content::Deleted;
    store.retain_recovery_record(&retained).unwrap();
    let intended = store.prepare_recovery_upload(now + 1).unwrap();
    let frozen: std::collections::BTreeMap<_, _> = store
        .pending_recovery_objects()
        .unwrap()
        .into_iter()
        .map(|object| (object.id(), object.bytes().to_vec()))
        .collect();
    // Model acknowledgements from before restore; none proves present storage.
    for id in frozen.keys() {
        store.acknowledge_recovery_object(intended, *id).unwrap();
    }
    assert!(store.pending_recovery_objects().unwrap().is_empty());
    if published {
        let network = replacement.connected_client().unwrap();
        for bytes in frozen.values() {
            network
                .upload_recovery_object(
                    &sigil_crypto::recovery::Object::from_bytes(bytes.clone()).unwrap(),
                )
                .unwrap();
        }
        let mut request = store.recovery_publication().unwrap();
        request.acknowledge_restored_checkpoint = true;
        network.publish_recovery_head(&request).unwrap();
        // Model a backup restored after publication but before local commit.
        server
            .execute("UPDATE recovery_heads SET restored_checkpoint=1", [])
            .unwrap();
    }
    let (id, bytes) = frozen.first_key_value().unwrap();
    store
        .db
        .execute(
            "UPDATE archive_objects SET data=zeroblob(length(data)) WHERE id=?1",
            [id.as_slice()],
        )
        .unwrap();
    assert!(store.copy_recovery_history_to(&mut replacement).is_err());
    assert!(matches!(
        replacement.recovery_status(),
        Err(Error::NotFound)
    ));
    store
        .db
        .execute(
            "UPDATE archive_objects SET data=?1 WHERE id=?2",
            (bytes, id.as_slice()),
        )
        .unwrap();
    replacement.db.execute_batch("CREATE TRIGGER fail_handoff BEFORE INSERT ON archive_records BEGIN SELECT RAISE(FAIL,'injected'); END;").unwrap();
    assert!(store.copy_recovery_history_to(&mut replacement).is_err());
    assert!(matches!(
        replacement.recovery_status(),
        Err(Error::NotFound)
    ));
    replacement
        .db
        .execute_batch("DROP TRIGGER fail_handoff;")
        .unwrap();
    store.copy_recovery_history_to(&mut replacement).unwrap();
    assert_eq!(
        replacement.recovery_status().unwrap().pending,
        Some((crate::recovery::Operation::Upload, intended))
    );
    assert_eq!(
        replacement
            .pending_recovery_objects()
            .unwrap()
            .into_iter()
            .map(|object| (object.id(), object.bytes().to_vec()))
            .collect::<std::collections::BTreeMap<_, _>>(),
        frozen
    );
    assert!(replacement.recovery_publication().is_err());
    assert!(store.pending_recovery_objects().unwrap().is_empty());
    assert!(store.copy_recovery_history_to(&mut replacement).is_err());
    assert_eq!(store.recovery_status().unwrap().anchor, Some(head));
    assert!(matches!(
        replacement.recovery_record(retained.id).unwrap().content,
        Content::Deleted
    ));
    for table in [
        "identity",
        "sessions",
        "prekeys",
        "peers",
        "own_device_binding",
        "inbox",
        "outbox",
    ] {
        assert_eq!(
            replacement
                .db
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    drop(replacement);
    let mut store = reopen();
    store.reconcile_restored_recovery().unwrap();
    drop(store);
    let mut store = reopen();
    let repaired = store.prepare_recovery_upload(now + 1).unwrap();
    if published {
        assert_eq!(repaired.generation, intended.generation + 1);
        assert_eq!(store.recovery_status().unwrap().anchor, Some(intended));
    } else {
        assert_eq!(repaired, intended);
    }
    assert_eq!(store.upload_recovery_step().unwrap(), Some(repaired));
    let published = store.connected_client().unwrap().recovery_head().unwrap();
    assert_eq!(published.generation, repaired.generation);
    assert!(!published.restored_checkpoint);
    assert_eq!(store.recovery_status().unwrap().anchor, Some(repaired));
    // A proven ancestor authorizes repair without lowering the local anchor.
    server
        .execute(
            "UPDATE recovery_heads SET generation=?1,manifest=?2,restored_checkpoint=1",
            (
                head.generation as i64,
                super::super::transport::hex(&head.manifest),
            ),
        )
        .unwrap();
    store.reconcile_restored_recovery().unwrap();
    assert_eq!(store.recovery_status().unwrap().anchor, Some(repaired));
    let advanced = store.prepare_recovery_upload(now + 2).unwrap();
    assert_eq!(advanced.generation, repaired.generation + 1);
    assert_eq!(store.upload_recovery_step().unwrap(), Some(advanced));
    assert_eq!(
        store
            .connected_client()
            .unwrap()
            .recovery_head()
            .unwrap()
            .generation,
        advanced.generation
    );
    // A lagging trusted reader must verify the repaired chain, not waive its anchor.
    let invitation = control
        .invite_reauthorization(&session.account_id, 60, now)
        .unwrap();
    let lagging_path = dir.path().join("lagging.db");
    let mut lagging = open(&lagging_path);
    lagging
        .prepare_enrollment(
            "chat.example",
            fixture.port(),
            &[CA.to_vec()],
            &invitation.secret,
            "Synthetic lagging reader",
            true,
        )
        .unwrap();
    lagging.enroll_online().unwrap();
    lagging
        .configure_recovery(
            "chat.example",
            decode_id(&session.account_id).unwrap(),
            Secret32::from_bytes([7; 32]),
        )
        .unwrap();
    let object = lagging
        .connected_client()
        .unwrap()
        .download_recovery_object(head.manifest)
        .unwrap();
    lagging
        .begin_recovery_import(
            &sigil_protocol::recovery::Head {
                generation: head.generation,
                manifest: Some(super::super::transport::hex(&head.manifest)),
                restored_checkpoint: false,
            },
            &object,
            true,
        )
        .unwrap();
    for _ in 0..8 {
        if lagging.download_recovery_step(false).unwrap() == Some(head) {
            break;
        }
    }
    assert_eq!(lagging.recovery_status().unwrap().anchor, Some(head));
    for _ in 0..12 {
        if lagging.download_recovery_step(false).unwrap() == Some(advanced) {
            break;
        }
        assert_eq!(lagging.recovery_status().unwrap().anchor, Some(head));
        drop(lagging);
        lagging = open(&lagging_path);
    }
    assert_eq!(lagging.recovery_status().unwrap().anchor, Some(advanced));
    assert!(matches!(
        lagging.recovery_record(retained.id).unwrap().content,
        Content::Deleted
    ));
}

#[test]
fn competing_restore_repair_merges_over_https_without_losing_local_tombstones() {
    competing_repair(false, false);
}

#[test]
fn competing_generation_jump_repair_clears_the_obsolete_cas_base() {
    competing_repair(true, false);
}

#[test]
fn distant_repair_winner_is_verified_over_https_before_discarding_upload() {
    competing_repair(true, true);
}

fn competing_repair(older_backup: bool, distant: bool) {
    use sigil_crypto::recovery::{Content, Direction, Record, RecoveryKey};
    let (dir, fixture, invitation, now) = setup();
    let path = dir.path().join("repair-race.db");
    let mut store = open(&path);
    prepare(&mut store, &fixture, &invitation.secret);
    let session = store.enroll_online().unwrap();
    let account = decode_id(&session.account_id).unwrap();
    store
        .configure_recovery("chat.example", account, Secret32::from_bytes([7; 32]))
        .unwrap();
    let author = store.identity().unwrap();
    let mut local = Record {
        id: [1; 32],
        revision: 1,
        conversation: [5; 32],
        author,
        created_at: now,
        direction: Direction::Outgoing,
        content: Content::Retained(Zeroizing::new(b"Synthetic local history".to_vec())),
    };
    store.retain_recovery_record(&local).unwrap();
    let anchor = store.prepare_recovery_upload(now).unwrap();
    assert_eq!(store.upload_recovery_step().unwrap(), Some(anchor));
    let base = anchor;
    let anchor = if older_backup {
        let next = store.prepare_recovery_upload(now + 1).unwrap();
        assert_eq!(store.upload_recovery_step().unwrap(), Some(next));
        next
    } else {
        anchor
    };
    let server = rusqlite::Connection::open(dir.path().join("server.db")).unwrap();
    server
        .execute(
            "UPDATE recovery_heads SET generation=?1,manifest=?2,restored_checkpoint=1",
            (
                base.generation as i64,
                super::super::transport::hex(&base.manifest),
            ),
        )
        .unwrap();
    store.reconcile_restored_recovery().unwrap();
    let intended = store.prepare_recovery_upload(now + 1).unwrap();
    // This deletion occurs after freezing the losing snapshot.
    local.revision = 2;
    local.content = Content::Deleted;
    store.retain_recovery_record(&local).unwrap();
    let key = RecoveryKey::from_secret(
        Secret32::from_bytes([7; 32]),
        crate::recovery::account_scope("chat.example", account).unwrap(),
    )
    .unwrap();
    let remote = Record {
        id: [2; 32],
        revision: 1,
        conversation: [5; 32],
        author,
        created_at: now,
        direction: Direction::Outgoing,
        content: Content::Retained(Zeroizing::new(b"Synthetic competing history".to_vec())),
    };
    let (reference, record) = key.seal_record(&remote).unwrap();
    let (page_reference, page) = key.seal_page(&[reference]).unwrap();
    let (winner, manifest) = key
        .seal_manifest(Some(&anchor), now + 1, &[page_reference])
        .unwrap();
    let network = store.connected_client().unwrap();
    for object in [&record, &page, &manifest] {
        network.upload_recovery_object(object).unwrap();
    }
    let mut response = network
        .publish_recovery_head(&sigil_protocol::recovery::PublishHead {
            expected_generation: base.generation,
            expected_manifest: Some(super::super::transport::hex(&base.manifest)),
            manifest: super::super::transport::hex(&winner.manifest),
            acknowledge_restored_checkpoint: true,
            restore_generation: older_backup.then_some(winner.generation),
        })
        .unwrap();
    let winner = if distant {
        let (next, object) = key
            .seal_manifest(Some(&winner), now + 2, &[page_reference])
            .unwrap();
        network.upload_recovery_object(&object).unwrap();
        response = network
            .publish_recovery_head(&sigil_protocol::recovery::PublishHead {
                expected_generation: winner.generation,
                expected_manifest: Some(super::super::transport::hex(&winner.manifest)),
                manifest: super::super::transport::hex(&next.manifest),
                acknowledge_restored_checkpoint: false,
                restore_generation: None,
            })
            .unwrap();
        next
    } else {
        winner
    };
    let mut conflicted = false;
    for _ in 0..12 {
        match store.upload_recovery_step() {
            Err(Error::Network(network::Error::Status { code: 409, .. })) => {
                conflicted = true;
                break;
            }
            Err(Error::Network(network::Error::Status {
                code: 429,
                retry_after_seconds: Some(seconds),
            })) if seconds <= 5 => {
                std::thread::sleep(std::time::Duration::from_secs(seconds.max(1)))
            }
            other => panic!("unexpected losing upload result: {other:?}"),
        }
    }
    assert!(conflicted);
    assert_eq!(
        store.recovery_status().unwrap().pending,
        Some((crate::recovery::Operation::Upload, intended))
    );
    let before = store.recovery_publication().unwrap();
    assert!(before.acknowledge_restored_checkpoint);
    assert_eq!(
        before.restore_generation,
        older_backup.then_some(intended.generation)
    );
    let (fork, fork_manifest) = key
        .seal_manifest(
            Some(&sigil_crypto::recovery::Head {
                generation: anchor.generation,
                manifest: [42; 32],
            }),
            now + 1,
            &[page_reference],
        )
        .unwrap();
    let mut fork_response = response.clone();
    fork_response.manifest = Some(super::super::transport::hex(&fork.manifest));
    assert!(store
        .begin_recovery_import(&fork_response, &fork_manifest, false)
        .is_err());
    assert_eq!(
        store.recovery_publication().unwrap().manifest,
        before.manifest
    );
    // Failed handover must retain the losing snapshot and its authorization.
    store.db.execute_batch("CREATE TRIGGER fail_handover BEFORE UPDATE ON archive BEGIN SELECT RAISE(FAIL,'injected'); END;").unwrap();
    assert!(store
        .begin_recovery_import(&response, &manifest, false)
        .is_err());
    assert_eq!(
        store.recovery_status().unwrap().pending,
        Some((crate::recovery::Operation::Upload, intended))
    );
    assert_eq!(
        store.recovery_publication().unwrap().manifest,
        before.manifest
    );
    assert!(
        store
            .recovery_publication()
            .unwrap()
            .acknowledge_restored_checkpoint
    );
    store
        .db
        .execute_batch("DROP TRIGGER fail_handover;")
        .unwrap();
    for _ in 0..8 {
        if store.download_recovery_step(false).unwrap() == Some(winner) {
            break;
        }
        drop(store);
        store = open(&path);
        assert_eq!(store.recovery_status().unwrap().anchor, Some(anchor));
        assert!(store.recovery_record(remote.id).is_err());
    }
    assert_eq!(store.recovery_status().unwrap().anchor, Some(winner));
    assert!(matches!(
        store.recovery_record(local.id).unwrap().content,
        Content::Deleted
    ));
    assert!(matches!(
        store.recovery_record(remote.id).unwrap().content,
        Content::Retained(_)
    ));
    let merged = store.prepare_recovery_upload(now + 2).unwrap();
    for _ in 0..12 {
        match store.upload_recovery_step() {
            Ok(Some(head)) => {
                assert_eq!(head, merged);
                break;
            }
            Ok(None) => {}
            Err(Error::Network(network::Error::Status {
                code: 429,
                retry_after_seconds: Some(seconds),
            })) if seconds <= 5 => {
                std::thread::sleep(std::time::Duration::from_secs(seconds.max(1)));
            }
            other => panic!("unexpected repair upload result: {other:?}"),
        }
    }
    assert_eq!(store.recovery_status().unwrap().anchor, Some(merged));
    assert_eq!(merged.generation, winner.generation + 1);
    assert!(!network.recovery_head().unwrap().restored_checkpoint);
}

#[test]
fn online_expiry_is_bounded_atomic_and_resumes_after_lost_receipt() {
    let (dir, fixture, invitation, now) = setup();
    let path = dir.path().join("client.db");
    let mut store = open(&path);
    prepare(&mut store, &fixture, &invitation.secret);
    let account = store.enroll_online().unwrap();
    let dh = DhKey::generate().unwrap();
    let public = dh.public_key();
    let mut receiving = Session::responder(Secret32::from_bytes([7; 32]), dh, [8; 32]).unwrap();
    let session = [1; 32];
    store
        .insert_session(
            session,
            Session::initiator(Secret32::from_bytes([7; 32]), public, [8; 32]).unwrap(),
        )
        .unwrap();
    let recipient = decode_id(&account.device_id).unwrap();
    let mut last = Vec::new();
    for n in 1..=18 {
        last = store
            .send(session, [n; 32], b"Synthetic expiry batch")
            .unwrap();
        store
            .prepare_delivery(
                session,
                [n; 32],
                recipient,
                now + if n == 18 { 120 } else { 60 },
                now,
            )
            .unwrap();
    }
    let checkpoint: Vec<u8> = store
        .db
        .query_row("SELECT state FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert!(store.send_pending_online(session, u64::MAX).is_err());
    // Failure on the second expiry must roll back the first as well.
    store.db.execute_batch("CREATE TRIGGER fail_expiry BEFORE UPDATE OF expired ON deliveries WHEN OLD.id=X'0202020202020202020202020202020202020202020202020202020202020202' BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(store.send_pending_online(session, now + 60).is_err());
    assert!(!store.delivery_expired(session, [1; 32]).unwrap());
    store.db.execute_batch("DROP TRIGGER fail_expiry;").unwrap();
    assert_eq!(
        store.send_pending_online(session, now + 60).unwrap(),
        SendProgress {
            accepted: 0,
            expired: 16
        }
    );
    assert!(!store.delivery_expired(session, [17; 32]).unwrap());
    assert!(store
        .connected_client()
        .unwrap()
        .mailbox()
        .unwrap()
        .is_empty());
    assert_eq!(
        store
            .db
            .query_row("SELECT state FROM sessions", [], |r| r.get::<_, Vec<u8>>(0))
            .unwrap(),
        checkpoint
    );
    drop(store);
    let mut store = open(&path);
    store.db.execute_batch("CREATE TRIGGER fail_receipt BEFORE UPDATE OF receipt ON deliveries BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(store.send_pending_online(session, now + 60).is_err());
    assert!(store.delivery_expired(session, [17; 32]).unwrap());
    assert_eq!(
        store.pending(session).unwrap(),
        vec![([18; 32], last.clone())]
    );
    let network = store.connected_client().unwrap();
    let before = network.mailbox().unwrap();
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].payload, crate::transport::hex(&last));
    store
        .db
        .execute_batch("DROP TRIGGER fail_receipt;")
        .unwrap();
    drop(store);
    let mut store = open(&path);
    assert_eq!(
        store.send_pending_online(session, now + 60).unwrap(),
        SendProgress {
            accepted: 1,
            expired: 0
        }
    );
    assert_eq!(network.mailbox().unwrap()[0].sequence, before[0].sequence);
    assert_eq!(
        receiving
            .receive(&sigil_crypto::triple::Packet::from_bytes(&last).unwrap())
            .unwrap(),
        b"Synthetic expiry batch"
    );
    assert_eq!(
        store.send_pending_online(session, now + 60).unwrap(),
        SendProgress::default()
    );
    store.retire_session(session).unwrap();
}

#[test]
fn expiry_batch_rejects_unprepared_and_damaged_successors_without_partial_cleanup() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = dir.path().join("client.db");
    let mut store = open(&path);
    let dh = DhKey::generate().unwrap();
    let session = [1; 32];
    store
        .insert_session(
            session,
            Session::initiator(Secret32::from_bytes([7; 32]), dh.public_key(), [8; 32]).unwrap(),
        )
        .unwrap();
    store.send(session, [2; 32], b"Synthetic expired").unwrap();
    store
        .prepare_delivery(session, [2; 32], [9; 32], 1060, 1000)
        .unwrap();
    store
        .send(session, [3; 32], b"Synthetic unprepared")
        .unwrap();
    assert!(matches!(
        store.outgoing_batch(session, 1060, 16),
        Err(Error::Unprepared)
    ));
    assert!(!store.delivery_expired(session, [2; 32]).unwrap());
    store
        .prepare_delivery(session, [3; 32], [9; 32], 1120, 1000)
        .unwrap();
    store
        .db
        .execute(
            "UPDATE deliveries SET metadata=zeroblob(76) WHERE id=?1",
            [[3u8; 32].as_slice()],
        )
        .unwrap();
    assert!(store.outgoing_batch(session, 1060, 16).is_err());
    assert!(!store.delivery_expired(session, [2; 32]).unwrap());
    assert_eq!(store.pending(session).unwrap().len(), 2);
}

#[test]
fn device_inventory_uses_persisted_account_over_https() {
    let (dir, _fixture, alice, bob, _now) = crate::claims::tests::pair();
    let session = alice.connection_session().unwrap().unwrap();
    let page = alice.devices_online(None).unwrap();
    assert_eq!(page.account_id, session.account_id);
    assert_eq!(page.devices.len(), 1);
    assert_eq!(page.devices[0].id, session.device_id);
    assert!(!page.devices[0].revoked);
    assert!(page.next_after.is_none());
    assert_ne!(
        bob.devices_online(None).unwrap().account_id,
        page.account_id
    );
    assert!(alice
        .devices_online(Some(&session.device_id))
        .unwrap()
        .devices
        .is_empty());
    assert!(matches!(
        alice.devices_online(Some("invalid")),
        Err(Error::Network(crate::network::Error::Configuration))
    ));
    drop(alice);
    assert_eq!(
        open(&dir.path().join("alice.db"))
            .devices_online(None)
            .unwrap(),
        page
    );
}

#[test]
fn device_revocation_over_https_is_scoped_retryable_and_preserves_local_state() {
    let (dir, _fixture, alice, bob, now) = crate::claims::tests::pair();
    let session = alice.connection_session().unwrap().unwrap();
    let other = bob.connection_session().unwrap().unwrap();
    let before = load(&alice.db, &alice.key).unwrap().1;
    assert!(matches!(
        alice.revoke_device_online(&other.device_id),
        Err(Error::Network(crate::network::Error::Status {
            code: 404,
            ..
        }))
    ));
    assert!(bob.devices_online(None).is_ok());
    assert!(matches!(
        alice.revoke_device_online("../session"),
        Err(Error::Network(crate::network::Error::Configuration))
    ));
    // Synthetic second authorized device; this fixture does not implement or
    // assert encryption trust or a device-link protocol.
    let device = "cd".repeat(32);
    let token = "ef".repeat(32);
    let db = rusqlite::Connection::open(dir.path().join("server.db")).unwrap();
    db.execute("INSERT INTO devices(id,account_id,label,token_hash,expires_at) VALUES(?1,?2,'Synthetic second device',?3,?4)",
        (&device, &session.account_id, <sha2::Sha256 as sha2::Digest>::digest(token.as_bytes()).as_slice(), (now + 3600) as i64)).unwrap();
    alice.revoke_device_online(&device).unwrap();
    drop(alice);
    let alice = open(&dir.path().join("alice.db"));
    alice.revoke_device_online(&device).unwrap();
    assert!(alice
        .devices_online(None)
        .unwrap()
        .devices
        .iter()
        .any(|entry| entry.id == device && entry.revoked));
    let server = Store::open(&dir.path().join("server.db")).unwrap();
    assert!(server.session(&token, now).is_err());
    assert_eq!(load(&alice.db, &alice.key).unwrap().1, before);
    alice.revoke_device_online(&session.device_id).unwrap();
    assert!(matches!(
        alice.revoke_device_online(&session.device_id),
        Err(Error::Network(crate::network::Error::Status {
            code: 401,
            ..
        }))
    ));
    assert!(alice.devices_online(None).is_err());
    assert_eq!(load(&alice.db, &alice.key).unwrap().1, before);
}

#[test]
fn password_enrollment_reopens_the_same_account_without_storing_password() {
    let (dir, fixture, invite, now) = setup();
    let mut server = Store::open(&dir.path().join("server.db")).unwrap();
    let original = server
        .enroll(
            accounts::Enrollment {
                invitation: invite.secret,
                device_credential: "ab".repeat(32),
                device_label: "Original".into(),
            },
            now,
        )
        .unwrap();
    let password = "a synthetic password for native login";
    server
        .set_user_password(
            &original.account_id,
            sigil_server::password_login::SetPassword {
                password: Zeroizing::new(password.into()),
            },
        )
        .unwrap();
    server
        .configure_user_passwords(sigil_protocol::login::PasswordPolicy {
            revision: 0,
            enabled: true,
        })
        .unwrap();
    server
        .revoke_device(&"ab".repeat(32), &original.device_id, now)
        .unwrap();
    let path = dir.path().join("password-client.db");
    let mut client = open(&path);
    let session = client
        .sign_in_password_online(
            "chat.example",
            fixture.port(),
            &[CA.to_vec()],
            "alice",
            password,
        )
        .unwrap();
    assert_eq!(session.account_id, original.account_id);
    assert_eq!(session.address, original.address);
    let (profile, _) = load(&client.db, &client.key).unwrap();
    assert!(!serde_json::to_string(&profile).unwrap().contains(password));
    drop(client);
    let client = open(&path);
    assert_eq!(client.connection_session().unwrap(), Some(session.clone()));
    assert_eq!(
        client.connected_client().unwrap().session().unwrap(),
        session
    );
}
