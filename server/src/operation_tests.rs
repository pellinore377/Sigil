use super::*;
use sigil_protocol::admin::{AccountUpdate, Role};

fn execute(store: &mut Store, action: Action, credential: &str, now: u64) -> Operation {
    let prepared = store
        .prepare_operation(credential, true, action, now)
        .unwrap();
    assert_eq!(prepared.state, "confirmation_required");
    assert!(store
        .claim_operation(&digest(credential), now)
        .unwrap()
        .is_none());
    store
        .confirm_operation(&prepared.id, credential, now)
        .unwrap();
    let job = store
        .claim_operation(&digest(credential), now)
        .unwrap()
        .unwrap();
    let result = job.perform();
    store.finish_operation(&job.id, result).unwrap();
    store.operation(&job.id).unwrap()
}
#[test]
fn private_backup_resumable_import_restore_and_interrupted_activation() {
    let (dir, mut store, alice, _, now) = crate::admin::tests::setup();
    let root = "aa".repeat(32);
    let backup = execute(&mut store, Action::Backup, &root, now);
    assert_eq!(backup.state, "complete");
    let bytes = store.backup_chunk(&backup.id, 0).unwrap();
    assert!(!bytes.is_empty());
    let hash = crate::federation_auth::hex(&Sha256::digest(&bytes));
    let id = store
        .prepare_backup_upload(Upload {
            bytes: bytes.len() as u64,
            sha256: hash,
        })
        .unwrap();
    let split = bytes.len() / 2;
    assert!(store.upload_backup_chunk(&id, 1, &bytes[..split]).is_err());
    store.upload_backup_chunk(&id, 0, &bytes[..split]).unwrap();
    let partial = file(&directory(&store.0).unwrap(), &id, "part").unwrap();
    let mut output = fs::OpenOptions::new().append(true).open(&partial).unwrap();
    output.write_all(b"uncommitted synthetic bytes").unwrap();
    output.sync_all().unwrap();
    drop(output);
    drop(store);
    let mut store = Store::open(&dir.path().join("sigil.db")).unwrap();
    assert_eq!(store.backup_upload(&id).unwrap()["received"], split as u64);
    assert_eq!(
        store.upload_backup_chunk(&id, 0, &bytes[..split]).unwrap(),
        split as u64
    );
    assert!(store.upload_backup_chunk(&id, 0, b"wrong").is_err());
    store
        .upload_backup_chunk(&id, split as u64, &bytes[split..])
        .unwrap();
    let imported = execute(&mut store, Action::Import { file: id.clone() }, &root, now);
    assert_eq!(imported.state, "complete");
    assert!(matches!(
        store.backup_upload(&id),
        Err(StoreError::NotFound)
    ));
    assert_eq!(store.backup_chunk(&id, 0).unwrap(), bytes);
    let inspect = execute(&mut store, Action::Inspect { file: id.clone() }, &root, now);
    assert_eq!(inspect.state, "complete");
    let restore = execute(&mut store, Action::Restore { file: id }, &root, now);
    assert_eq!(restore.state, "complete");
    assert_eq!(restore.result.unwrap()["restart_required"], true);
    assert!(store.session(&alice, now).is_ok());
    drop(store);
    let current = dir.path().join("sigil.db");
    let previous = dir
        .path()
        .join(format!("sigil.pre-restore-{}.db", restore.id));
    fs::rename(&current, &previous).unwrap();
    activate_restore(dir.path()).unwrap();
    activate_restore(dir.path()).unwrap();
    assert!(previous.exists());
    let store = Store::open(&current).unwrap();
    assert!(store.session(&alice, now).is_err());
    assert!(store.operations().unwrap().is_empty());
    assert!(store.configuration().unwrap().settings.is_some());
}

#[test]
fn restore_activation_keeps_hot_journal_with_previous_database() {
    const CHILD_PATH: &str = "SIGIL_SYNTHETIC_RESTORE_JOURNAL";
    if let Some(path) = std::env::var_os(CHILD_PATH) {
        let store = Store::open(Path::new(&path)).unwrap();
        store
            .0
            .execute_batch("BEGIN IMMEDIATE; UPDATE devices SET revoked=1,token_hash=NULL;")
            .unwrap();
        store.0.cache_flush().unwrap();
        std::process::exit(0); // Leave an actual hot SQLite journal, without running destructors.
    }
    let (dir, mut store, alice, _, now) = crate::admin::tests::setup();
    let root = "aa".repeat(32);
    let backup = execute(&mut store, Action::Backup, &root, now);
    let restore = execute(&mut store, Action::Restore { file: backup.id }, &root, now);
    assert_eq!(restore.state, "complete");
    drop(store);
    let current = dir.path().join("sigil.db");
    assert!(std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "operations::tests::restore_activation_keeps_hot_journal_with_previous_database"
        ])
        .env(CHILD_PATH, &current)
        .status()
        .unwrap()
        .success());
    let journal = dir.path().join("sigil.db-journal");
    let bytes = fs::read(&journal).unwrap();
    assert_eq!(
        &bytes[..8],
        &[0xd9, 0xd5, 0x05, 0xf9, 0x20, 0xa1, 0x63, 0xd7]
    );
    activate_restore(dir.path()).unwrap();
    let restored = Store::open(&current).unwrap();
    assert!(
        restored.session(&alice, now).is_err(),
        "restore must not resurrect the pre-restore device credential"
    );
    let previous = dir
        .path()
        .join(format!("sigil.pre-restore-{}.db", restore.id));
    let original = Store::open(&previous).unwrap();
    assert!(
        original.session(&alice, now).is_ok(),
        "original must roll back its interrupted transaction"
    );
}
#[test]
fn jobs_require_confirmation_and_current_authorization_and_bound_resources() {
    let (dir, mut store, alice, bob, now) = crate::admin::tests::setup();
    let root = "aa".repeat(32);
    let account = store.session(&alice, now).unwrap().account_id;
    store
        .admin_update_account(
            &account,
            AccountUpdate {
                expected_revision: 0,
                role: Role::Administrator,
                disabled: false,
                quota_bytes: None,
                confirm: true,
            },
            now,
        )
        .unwrap();
    let prepared = store
        .prepare_operation(&alice, false, Action::Backup, now)
        .unwrap();
    assert!(store.confirm_operation(&prepared.id, &bob, now).is_err());
    store.confirm_operation(&prepared.id, &alice, now).unwrap();
    store
        .admin_revoke_device(&account, &store.session(&alice, now).unwrap().device_id)
        .unwrap();
    assert!(store
        .claim_operation(&digest(&root), now)
        .unwrap()
        .is_none());
    assert_eq!(store.operation(&prepared.id).unwrap().state, "failed");
    let expired = store
        .prepare_operation(&root, true, Action::Backup, now)
        .unwrap();
    assert!(store
        .confirm_operation(&expired.id, &root, now + 300)
        .is_err());
    let queued = store
        .prepare_operation(&root, true, Action::Backup, now)
        .unwrap();
    store.confirm_operation(&queued.id, &root, now).unwrap();
    assert!(store
        .claim_operation(&digest(&root), now + 86400)
        .unwrap()
        .is_none());
    assert_eq!(store.operation(&queued.id).unwrap().state, "failed");
    let bounded = dir.path().join("too-small.db");
    assert!(store.backup_bounded(&bounded, 4096).is_err());
    assert!(!bounded.exists());
    assert!(store
        .prepare_backup_upload(Upload {
            bytes: u64::MAX,
            sha256: root.clone()
        })
        .is_err());
    let malformed = store
        .prepare_backup_upload(Upload {
            bytes: 4096,
            sha256: root.clone(),
        })
        .unwrap();
    store
        .upload_backup_chunk(&malformed, 0, &vec![0; 4096])
        .unwrap();
    let failed = execute(
        &mut store,
        Action::Import {
            file: malformed.clone(),
        },
        &root,
        now,
    );
    assert_eq!(failed.state, "failed");
    store.delete_backup(&malformed).unwrap();
    assert!(store.backup_chunk("../../sigil.db", 0).is_err());
    let bad = "bb".repeat(32);
    let link = file(&directory(&store.0).unwrap(), &bad, "db").unwrap();
    std::os::unix::fs::symlink(dir.path().join("sigil.db"), link).unwrap();
    assert!(store.backup_chunk(&bad, 0).is_err());
}
#[test]
fn corrupt_restore_stage_does_not_move_live_database() {
    let (dir, store, alice, _, now) = crate::admin::tests::setup();
    let id = "aa".repeat(32);
    let directory = directory(&store.0).unwrap();
    private_file(&directory.join(format!("{id}.restore")))
        .unwrap()
        .write_all(b"invalid")
        .unwrap();
    private_file(&dir.path().join("restore.pending"))
        .unwrap()
        .write_all(id.as_bytes())
        .unwrap();
    assert!(activate_restore(dir.path()).is_err());
    assert!(dir.path().join("sigil.db").exists());
    assert!(store.session(&alice, now).is_ok());
}
#[test]
fn backup_retry_recovers_partial_files_and_a_failed_completion_commit() {
    let (dir, mut store, _, _, now) = crate::admin::tests::setup();
    let root = "aa".repeat(32);
    let prepared = store
        .prepare_operation(&root, true, Action::Backup, now)
        .unwrap();
    store.confirm_operation(&prepared.id, &root, now).unwrap();
    let job = store.claim_operation(&digest(&root), now).unwrap().unwrap();
    let partial = file(&job.directory, &job.id, "backup").unwrap();
    private_file(&partial)
        .unwrap()
        .write_all(b"incomplete backup")
        .unwrap();
    let result = job.perform().unwrap();
    assert!(!partial.exists());
    store.0.execute_batch("CREATE TRIGGER synthetic BEFORE UPDATE ON operations BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(store.finish_operation(&job.id, Ok(result.clone())).is_err());
    store.0.execute_batch("DROP TRIGGER synthetic").unwrap();
    drop(store);
    let mut store = Store::open(&dir.path().join("sigil.db")).unwrap();
    let resumed = store
        .claim_operation(&digest(&root), now + 121)
        .unwrap()
        .unwrap();
    assert_eq!(resumed.id, job.id);
    let retried = resumed.perform().unwrap();
    assert_eq!(retried, result);
    store.finish_operation(&resumed.id, Ok(retried)).unwrap();
    assert_eq!(store.operation(&job.id).unwrap().state, "complete");
    let current = Release {
        version: env!("CARGO_PKG_VERSION").into(),
        image_digest: format!("sha256:{}", root),
        minimum_schema: 25,
        target_schema: 27,
    };
    validate_release(&current).unwrap();
    assert!(Action::Upgrade { release: current }.validate().is_err());
    assert!(version("+1.0.0").is_none());
}
#[test]
fn signed_updates_and_https_endpoint_check_use_configured_trust() {
    use axum::{extract::Path, routing::get, Json, Router};
    use ring::signature::KeyPair;
    let _guard = crate::egress::tests::NETWORK.lock().unwrap();
    let (dir, mut store, _, _, now) = crate::admin::tests::setup();
    let root = "aa".repeat(32);
    let key = ring::signature::Ed25519KeyPair::from_seed_unchecked(&[37; 32]).unwrap();
    let release = Release {
        version: "0.2.0".into(),
        image_digest: format!("sha256:{}", "ab".repeat(32)),
        minimum_schema: 25,
        target_schema: 27,
    };
    let signature =
        crate::federation_auth::hex(key.sign(encode(&release).unwrap().as_bytes()).as_ref());
    let envelope = serde_json::json!({"release":release,"signature":signature});
    let db = dir.path().join("sigil.db");
    let app = Router::new()
        .route(
            "/release",
            get(move || {
                let value = envelope.clone();
                async move { Json(value) }
            }),
        )
        .route(
            "/setup/v0/probe/{id}",
            get(move |Path(id): Path<String>| {
                let db = db.clone();
                async move {
                    Store::open(&db)
                        .unwrap()
                        .endpoint_probe(&id, crate::enrollment::now().unwrap())
                        .unwrap()
                }
            }),
        );
    let fixture = crate::egress::tests::Fixture::local(app);
    let origin = fixture.uri("127.0.0.1", "");
    let mut policy = store.administration_policy().unwrap();
    policy.public_origin = Some(origin.clone());
    store.configure_administration(policy).unwrap();
    let mut config = store.operation_configuration().unwrap();
    config.release_url = Some(format!("{origin}/release"));
    config.release_key = Some(crate::federation_auth::hex(key.public_key().as_ref()));
    config.exceptions = vec![fixture.exception("127.0.0.1")];
    store.configure_operations(config).unwrap();
    assert_eq!(
        execute(&mut store, Action::CheckEndpoint, &root, now).state,
        "complete"
    );
    assert_eq!(
        execute(&mut store, Action::CheckUpdate, &root, now).state,
        "complete"
    );
    assert_eq!(
        execute(
            &mut store,
            Action::Upgrade {
                release: release.clone()
            },
            &root,
            now
        )
        .state,
        "complete"
    );
    let mut modified = release;
    modified.version = "0.3.0".into();
    assert_eq!(
        execute(
            &mut store,
            Action::Upgrade { release: modified },
            &root,
            now
        )
        .state,
        "failed"
    );
    let mut config = store.operation_configuration().unwrap();
    config.release_key = Some("01".repeat(32));
    store.configure_operations(config).unwrap();
    assert_eq!(
        execute(&mut store, Action::CheckUpdate, &root, now).state,
        "failed"
    );
}

#[test]
fn failed_restore_artifacts_can_be_deleted_without_touching_pending_restore() {
    let (dir, mut store, _, _, now) = crate::admin::tests::setup();
    let root = "aa".repeat(32);
    let backup = execute(&mut store, Action::Backup, &root, now);
    let first = execute(
        &mut store,
        Action::Restore {
            file: backup.id.clone(),
        },
        &root,
        now,
    );
    assert_eq!(first.state, "complete");
    let second = execute(&mut store, Action::Restore { file: backup.id }, &root, now);
    assert_eq!(second.state, "failed");
    let maintenance = directory(&store.0).unwrap();
    let failed_stage = file(&maintenance, &second.id, "restore").unwrap();
    assert!(
        failed_stage.exists(),
        "fixture must reach the ordinary failed staging path"
    );
    assert!(store
        .backup_files()
        .unwrap()
        .iter()
        .any(|v| v["id"] == second.id));
    store.delete_backup(&second.id).unwrap();
    assert!(file(&maintenance, &first.id, "restore").unwrap().exists());
    assert_eq!(
        fs::read_to_string(dir.path().join("restore.pending")).unwrap(),
        first.id
    );
    assert!(
        !failed_stage.exists(),
        "deleting the failed restore's backup leaves its hidden stage behind"
    );
}

#[test]
fn artifact_sweep_is_bounded_and_preserves_live_work_uploads_and_pending_restore() {
    let (dir, mut store, _, _, now) = crate::admin::tests::setup();
    let root = "aa".repeat(32);
    let make_upload = || Upload {
        bytes: 4096,
        sha256: root.clone(),
    };
    let upload = store.prepare_backup_upload(make_upload()).unwrap();
    store
        .upload_backup_chunk(&upload, 0, &vec![0; 4096])
        .unwrap();
    let prepared = store
        .prepare_operation(&root, true, Action::Backup, now)
        .unwrap();
    store.confirm_operation(&prepared.id, &root, now).unwrap();
    let job = store.claim_operation(&digest(&root), now).unwrap().unwrap();
    let source = "bb".repeat(32);
    let queued = store
        .prepare_operation(
            &root,
            true,
            Action::Inspect {
                file: source.clone(),
            },
            now,
        )
        .unwrap();
    store.confirm_operation(&queued.id, &root, now).unwrap();
    let maintenance = directory(&store.0).unwrap();
    let pending = "cc".repeat(32);
    let marker = dir.path().join("restore.pending");
    private_file(&marker)
        .unwrap()
        .write_all(pending.as_bytes())
        .unwrap();
    let mut protected = Vec::new();
    for (id, ext) in [
        (&job.id, "backup"),
        (&source, "restore"),
        (&pending, "restore"),
        (&upload, "part-wal"),
    ] {
        let path = file(&maintenance, id, ext).unwrap();
        private_file(&path).unwrap();
        protected.push(path);
    }
    let snapshot = file(&maintenance, &"dd".repeat(32), "db").unwrap();
    private_file(&snapshot).unwrap();
    protected.push(snapshot);
    let orphan = "ee".repeat(32);
    let temporary = format!("restore-{}", "ff".repeat(32));
    let extensions = [
        "backup",
        "marker",
        "restore",
        "part",
        "part-wal",
        "part-shm",
        "part-journal",
        &temporary,
    ];
    for ext in extensions {
        private_file(&file(&maintenance, &orphan, ext).unwrap()).unwrap();
    }
    // Model bounded crash leftovers at the admission threshold using empty files.
    for n in 0..120 {
        private_file(&file(&maintenance, &format!("{:064x}", n + 1), "backup").unwrap()).unwrap();
    }
    assert!(fs::read_dir(&maintenance).unwrap().count() >= 120);
    store.prepare_backup_upload(make_upload()).unwrap();
    for path in protected {
        assert!(path.exists(), "live artifact or retained snapshot removed");
    }
    for ext in extensions {
        assert!(!file(&maintenance, &orphan, ext).unwrap().exists());
    }
    assert!(fs::read_dir(&maintenance).unwrap().count() < 120);
    for id in [&job.id, &source, &pending] {
        assert!(matches!(store.delete_backup(id), Err(StoreError::Busy)));
    }
    store.delete_backup(&upload).unwrap();
    assert!(matches!(
        store.backup_upload(&upload),
        Err(StoreError::NotFound)
    ));
    assert!(!file(&maintenance, &upload, "part-wal").unwrap().exists());
    fs::write(&marker, b"invalid").unwrap();
    let preserve = file(&maintenance, &orphan, "backup").unwrap();
    private_file(&preserve).unwrap();
    assert!(store.prepare_backup_upload(make_upload()).is_err());
    assert!(
        preserve.exists(),
        "malformed restore marker must fail closed before cleanup"
    );
}

#[test]
fn selected_artifact_deletion_reports_partial_bounded_progress() {
    let (_dir, mut store, _, _, _) = crate::admin::tests::setup();
    let maintenance = directory(&store.0).unwrap();
    let id = "aa".repeat(32);
    // Synthetic accumulated crash temporaries for one retired operation.
    for n in 0..300 {
        let extension = format!("restore-{n:064x}");
        private_file(&file(&maintenance, &id, &extension).unwrap()).unwrap();
    }
    assert!(matches!(store.delete_backup(&id), Err(StoreError::Busy)));
    assert_eq!(fs::read_dir(&maintenance).unwrap().count(), 44);
    store.delete_backup(&id).unwrap();
    assert_eq!(fs::read_dir(&maintenance).unwrap().count(), 0);
}
