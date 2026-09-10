use axum::{
    body::{to_bytes, Body},
    http::Request,
};
use sha2::{Digest, Sha256};
use sigil_protocol::{
    accounts::{Enrollment, Invitation, InviteRequest, Session},
    Configure, Settings,
};
use sigil_server::{
    auth::{random_secret, AdminToken},
    router,
    store::{Store, StoreError},
};
use tower::ServiceExt;

const NOW: u64 = 1000;
#[tokio::test]
async fn android_push_configuration_keeps_admin_and_device_authority_separate() {
    let dir=tempfile::tempdir().unwrap();
    let path=dir.path().join("admin.token");
    let admin=AdminToken::load_or_create(&path).unwrap();
    let secret=std::fs::read_to_string(path).unwrap();
    let mut store=configured(&dir.path().join("server.db"));
    let now=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let invitation=store.invite(InviteRequest {username:"alice".into(),expires_in_seconds:60},now).unwrap();
    let token=random_secret().unwrap();
    enroll(&mut store,&invitation.secret,&token,now).unwrap();
    let app=router(store,admin);
    for (route,credential,expected) in [
        ("/admin/v0/push/android",None,401),
        ("/admin/v0/push/android",Some(token.as_str()),403),
        ("/admin/v0/push/android",Some(secret.as_str()),200),
        ("/client/v0/push/android",None,401),
        ("/client/v0/push/android",Some(secret.as_str()),401),
        ("/client/v0/push/android",Some(token.as_str()),200),
    ] {
        assert_eq!(request(&app,"GET",route,credential,serde_json::Value::Null).await.0,expected, "{route}");
    }
    assert_eq!(request(&app,"PUT","/admin/v0/push/android",Some(&token),serde_json::json!({"expected_revision":0,"expected_push_revision":0,"android":null})).await.0,403);
}
fn configured(path: &std::path::Path) -> Store {
    let mut store = Store::open(path).unwrap();
    if store.configuration().unwrap().settings.is_none() {
        store
            .configure(Configure {
                expected_revision: 0,
                settings: serde_json::from_str::<Settings>(r#"{"server_name":"chat.example"}"#)
                    .unwrap(),
            })
            .unwrap();
    }
    store
}
fn invite(store: &mut Store, username: &str) -> Invitation {
    store
        .invite(
            InviteRequest {
                username: username.into(),
                expires_in_seconds: 60,
            },
            NOW,
        )
        .unwrap()
}
fn enroll(
    store: &mut Store,
    invitation: &str,
    token: &str,
    now: u64,
) -> Result<Session, StoreError> {
    store.enroll(
        Enrollment {
            invitation: invitation.into(),
            device_credential: token.into(),
            device_label: "Synthetic device".into(),
        },
        now,
    )
}

#[test]
fn invitation_is_single_use_and_lost_enrollment_response_is_recoverable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let mut store = configured(&path);
    let invite = invite(&mut store, "alice");
    let token = random_secret().unwrap();
    let account = enroll(&mut store, &invite.secret, &token, NOW).unwrap();
    assert!(enroll(&mut store, &invite.secret, &random_secret().unwrap(), NOW).is_err());
    assert_eq!(store.session(&token, NOW).unwrap(), account);
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.session(&token, NOW).unwrap(), account);
    assert_eq!(account.address, "@alice:chat.example");
    assert_ne!(account.account_id, account.device_id);
}

#[test]
fn expired_and_revoked_invitations_cannot_enroll() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = configured(&dir.path().join("sigil.db"));
    let expired = invite(&mut store, "alice");
    assert!(enroll(
        &mut store,
        &expired.secret,
        &random_secret().unwrap(),
        expired.expires_at
    )
    .is_err());
    let revoked = invite(&mut store, "bob");
    store.revoke_invitation(&revoked.id).unwrap();
    assert!(enroll(&mut store, &revoked.secret, &random_secret().unwrap(), NOW).is_err());
}

#[test]
fn failed_enrollment_does_not_consume_invitation() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = configured(&dir.path().join("sigil.db"));
    let first = invite(&mut store, "alice");
    let token = random_secret().unwrap();
    enroll(&mut store, &first.secret, &token, NOW).unwrap();
    let second = invite(&mut store, "bob");
    assert!(enroll(&mut store, &second.secret, &token, NOW).is_err());
    assert!(enroll(&mut store, &second.secret, &random_secret().unwrap(), NOW).is_ok());
}

#[test]
fn rotation_expires_old_token_and_preserves_identity() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = configured(&dir.path().join("sigil.db"));
    let invitation = invite(&mut store, "alice");
    let old = random_secret().unwrap();
    let new = random_secret().unwrap();
    let before = enroll(&mut store, &invitation.secret, &old, NOW).unwrap();
    let after = store.rotate_device(&old, &new, NOW + 10).unwrap();
    assert_eq!(before.account_id, after.account_id);
    assert_eq!(before.device_id, after.device_id);
    assert!(store.session(&old, NOW + 10).is_err());
    assert!(store
        .rotate_device(&old, &random_secret().unwrap(), NOW + 10)
        .is_err());
    assert!(store.session(&new, after.expires_at - 1).is_ok());
    assert!(store.session(&new, after.expires_at).is_err());
    assert!(store
        .rotate_device(&new, &random_secret().unwrap(), after.expires_at)
        .is_err());
}

#[test]
fn device_ownership_and_account_disabling_are_enforced() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = configured(&dir.path().join("sigil.db"));
    let first = invite(&mut store, "alice");
    let alice_token = random_secret().unwrap();
    let alice = enroll(&mut store, &first.secret, &alice_token, NOW).unwrap();
    let second = invite(&mut store, "bob");
    let bob_token = random_secret().unwrap();
    let bob = enroll(&mut store, &second.secret, &bob_token, NOW).unwrap();
    assert!(store
        .revoke_device(&alice_token, &bob.device_id, NOW)
        .is_err());
    assert!(store.session(&bob_token, NOW).is_ok());
    store
        .revoke_device(&alice_token, &alice.device_id, NOW)
        .unwrap();
    assert!(store.session(&alice_token, NOW).is_err());
    store.disable_account(&bob.account_id).unwrap();
    assert!(store.session(&bob_token, NOW).is_err());
    assert!(store
        .invite(
            InviteRequest {
                username: "bob".into(),
                expires_in_seconds: 60
            },
            NOW
        )
        .is_err());
}

#[test]
fn backup_restore_invalidates_all_credentials_and_invitations() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = configured(&dir.path().join("sigil.db"));
    let first = invite(&mut store, "alice");
    let token = random_secret().unwrap();
    enroll(&mut store, &first.secret, &token, NOW).unwrap();
    let pending = invite(&mut store, "bob");
    let backup = dir.path().join("backup.db");
    store.backup(&backup).unwrap();
    let destination = dir.path().join("restored.db");
    Store::restore(&backup, &destination).unwrap();
    let mut restored = Store::open(&destination).unwrap();
    assert!(restored.session(&token, NOW).is_err());
    assert!(enroll(
        &mut restored,
        &pending.secret,
        &random_secret().unwrap(),
        NOW
    )
    .is_err());
    assert!(store.session(&token, NOW).is_ok());
}

#[test]
fn schema_one_migrates_and_credentials_are_never_stored_in_plaintext() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v1.db");
    sigil_server::store::private_file(&path).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("PRAGMA application_id=1397311308; PRAGMA user_version=1; CREATE TABLE configuration(id INTEGER PRIMARY KEY, revision INTEGER NOT NULL, settings TEXT); INSERT INTO configuration VALUES(1,0,NULL);").unwrap();
    drop(db);
    let mut store = configured(&path);
    let pending = invite(&mut store, "alice");
    let db = rusqlite::Connection::open(&path).unwrap();
    let hash: Vec<u8> = db
        .query_row("SELECT token_hash FROM invitations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(hash.len(), 32);
    assert!(hash.as_slice() != pending.secret.as_bytes());
    let token = random_secret().unwrap();
    enroll(&mut store, &pending.secret, &token, NOW).unwrap();
    let hash: Vec<u8> = db
        .query_row("SELECT token_hash FROM devices", [], |r| r.get(0))
        .unwrap();
    assert_eq!(hash.len(), 32);
    assert!(hash.as_slice() != token.as_bytes());
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        sigil_server::store::SCHEMA_VERSION
    );
}

async fn request(
    app: &axum::Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    value: serde_json::Value,
) -> (u16, serde_json::Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(value.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = to_bytes(response.into_body(), 8192).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

#[tokio::test]
async fn http_enrollment_and_admin_authorization_stay_separate() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("admin.token");
    let admin = AdminToken::load_or_create(&path).unwrap();
    let secret = std::fs::read_to_string(path).unwrap();
    let app = router(configured(&dir.path().join("sigil.db")), admin);
    assert_eq!(
        request(
            &app,
            "POST",
            "/admin/v0/invitations",
            None,
            serde_json::json!({"username":"alice","expires_in_seconds":60})
        )
        .await
        .0,
        401
    );
    let (status, invitation) = request(
        &app,
        "POST",
        "/admin/v0/invitations",
        Some(&secret),
        serde_json::json!({"username":"alice","expires_in_seconds":60}),
    )
    .await;
    assert_eq!(status, 201);
    let token = random_secret().unwrap();
    let (status, session) = request(&app, "POST", "/client/v0/enroll", None, serde_json::json!({"invitation":invitation["secret"],"device_credential":token,"device_label":"Synthetic device"})).await;
    assert_eq!(status, 201);
    assert_eq!(
        request(
            &app,
            "GET",
            "/client/v0/session",
            Some(&token),
            serde_json::Value::Null
        )
        .await
        .0,
        200
    );
    assert_eq!(
        request(
            &app,
            "GET",
            "/client/v0/session",
            Some(&secret),
            serde_json::Value::Null
        )
        .await
        .0,
        401
    );
    assert_eq!(
        request(
            &app,
            "GET",
            "/admin/v0/configuration",
            Some(&token),
            serde_json::Value::Null
        )
        .await
        .0,
        403
    );
    assert_eq!(
        request(
            &app,
            "DELETE",
            &format!(
                "/admin/v0/accounts/{}",
                session["account_id"].as_str().unwrap()
            ),
            Some(&secret),
            serde_json::Value::Null
        )
        .await
        .0,
        204
    );
    assert_eq!(
        request(
            &app,
            "GET",
            "/client/v0/session",
            Some(&token),
            serde_json::Value::Null
        )
        .await
        .0,
        401
    );
}

#[test]
fn simultaneous_redemption_creates_exactly_one_account() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("sigil.db");
    let mut store = configured(&path);
    let invitation = invite(&mut store, "alice");
    let barrier = std::sync::Barrier::new(2);
    let results = std::thread::scope(|scope| {
        let run = || {
            let mut connection = Store::open(&path).unwrap();
            barrier.wait();
            enroll(
                &mut connection,
                &invitation.secret,
                &random_secret().unwrap(),
                NOW,
            )
            .is_ok()
        };
        let first = scope.spawn(run);
        let second = scope.spawn(run);
        [first.join().unwrap(), second.join().unwrap()]
    });
    assert_eq!(results.into_iter().filter(|value| *value).count(), 1);
}

#[test]
fn restore_never_publishes_a_database_if_credential_invalidation_fails() {
    let directory = tempfile::tempdir().unwrap();
    let backup = directory.path().join("backup.db");
    let store = configured(&directory.path().join("source.db"));
    store.backup(&backup).unwrap();
    let malformed = rusqlite::Connection::open(&backup).unwrap();
    malformed.execute_batch("DROP TABLE devices;").unwrap();
    drop(malformed);
    let destination = directory.path().join("restored.db");
    assert!(Store::restore(&backup, &destination).is_err());
    assert!(!destination.exists());
    assert!(!std::fs::read_dir(directory.path())
        .unwrap()
        .any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("restore-")));
}

#[test]
fn restore_rejects_a_trigger_that_silently_skips_credential_revocation() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.db");
    let backup = directory.path().join("backup.db");
    let destination = directory.path().join("restored.db");
    let mut store = configured(&source);
    let invitation = invite(&mut store, "alice");
    let token = random_secret().unwrap();
    enroll(&mut store, &invitation.secret, &token, NOW).unwrap();
    store.backup(&backup).unwrap();
    let modified = rusqlite::Connection::open(&backup).unwrap();
    modified.execute_batch("CREATE TRIGGER preserve_credentials BEFORE UPDATE ON devices BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    drop(modified);
    assert!(Store::restore(&backup, &destination).is_err());
    assert!(!destination.exists());
    assert!(Store::open(&backup).is_err());
    // Rejection never mutates the source or revokes the live installation.
    assert!(store.session(&token, NOW).is_ok());
    let source = rusqlite::Connection::open(&backup).unwrap();
    assert_eq!(
        source
            .query_row(
                "SELECT count(*) FROM devices WHERE revoked=0 AND token_hash IS NOT NULL",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert!(!std::fs::read_dir(directory.path())
        .unwrap()
        .any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("restore-")));
}

#[test]
fn unrepresentable_session_times_fail_closed_without_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = configured(&dir.path().join("sigil.db"));
    let invitation = invite(&mut store, "alice");
    let token = random_secret().unwrap();
    let session = enroll(&mut store, &invitation.secret, &token, NOW).unwrap();
    for now in [i64::MAX as u64, i64::MAX as u64 + 1, u64::MAX] {
        assert!(matches!(
            store.session(&token, now),
            Err(StoreError::Unauthorized)
        ));
        assert!(store
            .rotate_device(&token, &random_secret().unwrap(), now)
            .is_err());
        assert!(matches!(
            store.revoke_device(&token, &session.device_id, now),
            Err(StoreError::Unauthorized)
        ));
    }
    assert_eq!(store.session(&token, NOW).unwrap(), session);
}

#[test]
fn revocation_retries_require_a_still_authorized_caller() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let mut store = configured(&path);
    let first = invite(&mut store, "alice");
    let token = random_secret().unwrap();
    let alice = enroll(&mut store, &first.secret, &token, NOW).unwrap();
    // Synthetic second-device fixture; authenticated linking remains unimplemented.
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute("INSERT INTO devices(id, account_id, label, expires_at) VALUES('synthetic-second', ?1, 'Synthetic second device', ?2)", (&alice.account_id, alice.expires_at as i64)).unwrap();
    store
        .revoke_device(&token, "synthetic-second", NOW)
        .unwrap();
    store
        .revoke_device(&token, "synthetic-second", NOW)
        .unwrap();
    assert!(matches!(
        store.revoke_device(&token, "missing", NOW),
        Err(StoreError::NotFound)
    ));
    store.revoke_device(&token, &alice.device_id, NOW).unwrap();
    assert!(matches!(
        store.revoke_device(&token, "synthetic-second", NOW),
        Err(StoreError::Unauthorized)
    ));
}

#[test]
fn competing_devices_cannot_both_revoke_each_other() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let mut store = configured(&path);
    let first = invite(&mut store, "alice");
    let alice_token = random_secret().unwrap();
    let alice = enroll(&mut store, &first.secret, &alice_token, NOW).unwrap();
    let second = invite(&mut store, "bob");
    let bob_token = random_secret().unwrap();
    let bob = enroll(&mut store, &second.secret, &bob_token, NOW).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    // Put two synthetic devices on one account without implementing device linking.
    db.execute(
        "UPDATE devices SET account_id=?1 WHERE id=?2",
        (&alice.account_id, &bob.device_id),
    )
    .unwrap();
    for _ in 0..32 {
        for (id, token) in [
            (&alice.device_id, &alice_token),
            (&bob.device_id, &bob_token),
        ] {
            db.execute(
                "UPDATE devices SET revoked=0, token_hash=?1 WHERE id=?2",
                (Sha256::digest(token.as_bytes()).as_slice(), id),
            )
            .unwrap();
        }
        let barrier = std::sync::Barrier::new(2);
        let run = |credential: &str, target: &str| {
            let mut connection = Store::open(&path).unwrap();
            barrier.wait();
            connection.revoke_device(credential, target, NOW)
        };
        let results = std::thread::scope(|scope| {
            let first = scope.spawn(|| run(&alice_token, &bob.device_id));
            let second = scope.spawn(|| run(&bob_token, &alice.device_id));
            [first.join().unwrap(), second.join().unwrap()]
        });
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(StoreError::Unauthorized)))
                .count(),
            1
        );
        assert_eq!(
            [&alice_token, &bob_token]
                .into_iter()
                .filter(|token| store.session(token, NOW).is_ok())
                .count(),
            1
        );
    }
}

fn recover(store: &mut Store, secret: &str, token: &str, now: u64) -> Result<Session, StoreError> {
    store.reauthorize(
        Enrollment {
            invitation: secret.into(),
            device_credential: token.into(),
            device_label: "Synthetic replacement".into(),
        },
        now,
    )
}

#[test]
fn reauthorization_preserves_account_but_replaces_device_and_authorization() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let mut store = configured(&path);
    let first = invite(&mut store, "alice");
    let old = random_secret().unwrap();
    let alice = enroll(&mut store, &first.secret, &old, NOW).unwrap();
    let other = invite(&mut store, "bob");
    let bob_token = random_secret().unwrap();
    let bob = enroll(&mut store, &other.secret, &bob_token, NOW).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    let second_token = random_secret().unwrap();
    db.execute("INSERT INTO devices(id,account_id,label,token_hash,expires_at) VALUES('synthetic-linked',?1,'Synthetic linked',?2,?3)", (&alice.account_id, Sha256::digest(second_token.as_bytes()).as_slice(), alice.expires_at as i64)).unwrap();
    db.execute(
        "INSERT INTO encryption_identities VALUES(?1,zeroblob(32))",
        [&alice.device_id],
    )
    .unwrap();
    db.execute(
        "INSERT INTO allowed_senders VALUES(?1,?2)",
        (&bob.device_id, &alice.device_id),
    )
    .unwrap();
    let invitation = store
        .invite_reauthorization(&alice.account_id, 60, NOW)
        .unwrap();
    assert!(store.session(&old, NOW).is_ok());
    assert!(enroll(
        &mut store,
        &invitation.secret,
        &random_secret().unwrap(),
        NOW
    )
    .is_err());
    let replacement = random_secret().unwrap();
    let restored = recover(&mut store, &invitation.secret, &replacement, NOW).unwrap();
    assert_eq!(restored.account_id, alice.account_id);
    assert_eq!(restored.address, alice.address);
    assert_ne!(restored.device_id, alice.device_id);
    assert!(store.session(&old, NOW).is_err());
    assert!(store.session(&second_token, NOW).is_err());
    assert!(store.session(&bob_token, NOW).is_ok());
    assert!(recover(
        &mut store,
        &invitation.secret,
        &random_secret().unwrap(),
        NOW
    )
    .is_err());
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM encryption_identities WHERE device_id=?1",
            [&restored.device_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM allowed_senders WHERE recipient=?1 OR sender=?1",
            [&restored.device_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(store);
    assert_eq!(
        Store::open(&path)
            .unwrap()
            .session(&replacement, NOW)
            .unwrap(),
        restored
    );
}

#[test]
fn reauthorization_rejects_wrong_invitation_expiry_cancellation_and_disabled_accounts() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = configured(&dir.path().join("sigil.db"));
    let first = invite(&mut store, "alice");
    assert!(recover(&mut store, &first.secret, &random_secret().unwrap(), NOW).is_err());
    let token = random_secret().unwrap();
    let alice = enroll(&mut store, &first.secret, &token, NOW).unwrap();
    assert!(store.invite_reauthorization("missing", 60, NOW).is_err());
    assert!(store
        .invite_reauthorization(&alice.account_id, 59, NOW)
        .is_err());
    assert!(store
        .invite_reauthorization(&alice.account_id, 60, u64::MAX)
        .is_err());
    let expired = store
        .invite_reauthorization(&alice.account_id, 60, NOW)
        .unwrap();
    assert!(store
        .invite_reauthorization(&alice.account_id, 60, NOW)
        .is_err());
    assert!(recover(
        &mut store,
        &expired.secret,
        &random_secret().unwrap(),
        expired.expires_at
    )
    .is_err());
    store.revoke_invitation(&expired.id).unwrap();
    assert!(recover(&mut store, &expired.secret, &random_secret().unwrap(), NOW).is_err());
    let disabled = store
        .invite_reauthorization(&alice.account_id, 60, NOW)
        .unwrap();
    store.disable_account(&alice.account_id).unwrap();
    assert!(recover(&mut store, &disabled.secret, &random_secret().unwrap(), NOW).is_err());
    assert!(store
        .invite_reauthorization(&alice.account_id, 60, NOW)
        .is_err());
}

#[test]
fn reauthorization_failure_rolls_back_revocations_and_preserves_invitation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let mut store = configured(&path);
    let first = invite(&mut store, "alice");
    let token = random_secret().unwrap();
    let alice = enroll(&mut store, &first.secret, &token, NOW).unwrap();
    let invitation = store
        .invite_reauthorization(&alice.account_id, 60, NOW)
        .unwrap();
    assert!(recover(&mut store, &invitation.secret, &token, NOW).is_err());
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_replacement BEFORE INSERT ON devices BEGIN SELECT RAISE(ABORT,'synthetic storage failure'); END;").unwrap();
    let replacement = random_secret().unwrap();
    assert!(recover(&mut store, &invitation.secret, &replacement, NOW).is_err());
    assert!(store.session(&token, NOW).is_ok());
    db.execute_batch("DROP TRIGGER fail_replacement;").unwrap();
    assert!(recover(&mut store, &invitation.secret, &replacement, NOW).is_ok());
}

#[test]
fn backup_restore_requires_a_new_reauthorization_invitation() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = configured(&dir.path().join("sigil.db"));
    let first = invite(&mut store, "alice");
    let token = random_secret().unwrap();
    let alice = enroll(&mut store, &first.secret, &token, NOW).unwrap();
    let stale = store
        .invite_reauthorization(&alice.account_id, 60, NOW)
        .unwrap();
    let backup = dir.path().join("backup.db");
    store.backup(&backup).unwrap();
    let path = dir.path().join("restored.db");
    Store::restore(&backup, &path).unwrap();
    let mut restored = Store::open(&path).unwrap();
    assert!(restored.session(&token, NOW).is_err());
    assert!(recover(&mut restored, &stale.secret, &random_secret().unwrap(), NOW).is_err());
    let fresh = restored
        .invite_reauthorization(&alice.account_id, 60, NOW)
        .unwrap();
    let new = recover(&mut restored, &fresh.secret, &random_secret().unwrap(), NOW).unwrap();
    assert_eq!(new.account_id, alice.account_id);
    assert_ne!(new.device_id, alice.device_id);
}

#[test]
fn concurrent_reauthorization_consumes_invitation_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let mut store = configured(&path);
    let first = invite(&mut store, "alice");
    let alice = enroll(&mut store, &first.secret, &random_secret().unwrap(), NOW).unwrap();
    let invitation = store
        .invite_reauthorization(&alice.account_id, 60, NOW)
        .unwrap();
    let barrier = std::sync::Barrier::new(2);
    let run = || {
        let mut connection = Store::open(&path).unwrap();
        barrier.wait();
        recover(
            &mut connection,
            &invitation.secret,
            &random_secret().unwrap(),
            NOW,
        )
    };
    let results = std::thread::scope(|scope| {
        let first = scope.spawn(run);
        let second = scope.spawn(run);
        [first.join().unwrap(), second.join().unwrap()]
    });
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    let db = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM devices WHERE revoked=0", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn reauthorization_crosses_retained_device_history_without_revalidating_old_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let mut store = configured(&path);
    let first = invite(&mut store, "alice");
    let token = random_secret().unwrap();
    let alice = enroll(&mut store, &first.secret, &token, NOW).unwrap();
    let invitation = store
        .invite_reauthorization(&alice.account_id, 60, NOW)
        .unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    for n in 0..255 {
        db.execute("INSERT INTO devices(id,account_id,label,expires_at,revoked) VALUES(?1,?2,'Synthetic retired',0,1)", (format!("retired-{n}"), &alice.account_id)).unwrap();
    }
    let replacement_token = random_secret().unwrap();
    let replacement = recover(&mut store, &invitation.secret, &replacement_token, NOW).unwrap();
    assert_eq!(replacement.account_id, alice.account_id);
    assert!(store.session(&token, NOW).is_err());
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.session(&replacement_token, NOW).unwrap(), replacement);
    assert!(store.session(&token, NOW).is_err());
    assert_eq!(
        db.query_row("SELECT count(*) FROM devices", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        257
    );
}

#[tokio::test]
async fn http_reauthorization_requires_admin_and_shares_enrollment_rate_budget() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = configured(&dir.path().join("sigil.db"));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let first = store
        .invite(
            InviteRequest {
                username: "alice".into(),
                expires_in_seconds: 60,
            },
            now,
        )
        .unwrap();
    let old = random_secret().unwrap();
    let alice = enroll(&mut store, &first.secret, &old, now).unwrap();
    let token_path = dir.path().join("admin.token");
    let admin = AdminToken::load_or_create(&token_path).unwrap();
    let admin_secret = std::fs::read_to_string(token_path).unwrap();
    let app = router(store, admin);
    let endpoint = format!(
        "/admin/v0/accounts/{}/reauthorization-invitations",
        alice.account_id
    );
    let lifetime = serde_json::json!({"expires_in_seconds":60});
    for token in [None, Some(old.as_str())] {
        assert_eq!(
            request(&app, "POST", &endpoint, token, lifetime.clone())
                .await
                .0,
            if token.is_some() { 403 } else { 401 }
        );
    }
    let (status, invitation) =
        request(&app, "POST", &endpoint, Some(&admin_secret), lifetime).await;
    assert_eq!(status, 201);
    let replacement = random_secret().unwrap();
    let body = serde_json::json!({"invitation":invitation["secret"],"device_credential":replacement,"device_label":"Synthetic replacement"});
    let blocked = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/client/v0/reauthorize")
                .header("origin", "https://chat.example")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(blocked.status(), 403);
    let (status, session) = request(&app, "POST", "/client/v0/reauthorize", None, body).await;
    assert_eq!(status, 201);
    assert_eq!(session["account_id"], alice.account_id);
    assert_eq!(
        request(
            &app,
            "GET",
            "/client/v0/session",
            Some(&old),
            serde_json::Value::Null
        )
        .await
        .0,
        401
    );
    assert_eq!(
        request(
            &app,
            "GET",
            "/client/v0/session",
            Some(&replacement),
            serde_json::Value::Null
        )
        .await
        .0,
        200
    );
    for path in [
        "/client/v0/enroll",
        "/client/v0/reauthorize",
        "/client/v0/enroll",
        "/client/v0/reauthorize",
    ] {
        assert_eq!(
            request(&app, "POST", path, None, serde_json::json!({}))
                .await
                .0,
            422
        );
    }
    assert_eq!(
        request(
            &app,
            "POST",
            "/client/v0/reauthorize",
            None,
            serde_json::json!({})
        )
        .await
        .0,
        429
    );
}

#[test]
fn device_inventory_is_account_scoped_paginated_and_retains_revocation_history() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = configured(&dir.path().join("sigil.db"));
    let invitation = invite(&mut store, "alice");
    let old_token = random_secret().unwrap();
    let alice = enroll(&mut store, &invitation.secret, &old_token, NOW).unwrap();
    let invitation = invite(&mut store, "bob");
    let bob_token = random_secret().unwrap();
    let bob = enroll(&mut store, &invitation.secret, &bob_token, NOW).unwrap();
    let mut token = old_token.clone();
    let mut current = alice.device_id.clone();
    for _ in 0..33 {
        let invitation = store
            .invite_reauthorization(&alice.account_id, 60, NOW)
            .unwrap();
        token = random_secret().unwrap();
        current = recover(&mut store, &invitation.secret, &token, NOW)
            .unwrap()
            .device_id;
    }
    assert!(matches!(
        store.list_devices(&old_token, None, NOW),
        Err(StoreError::Unauthorized)
    ));
    let first = store.list_devices(&token, None, NOW).unwrap();
    assert_eq!(first.account_id, alice.account_id);
    assert_eq!(first.devices.len(), 32);
    let second = store
        .list_devices(&token, first.next_after.as_deref(), NOW)
        .unwrap();
    assert_eq!(second.devices.len(), 2);
    assert!(second.next_after.is_none());
    let devices: Vec<_> = first.devices.into_iter().chain(second.devices).collect();
    assert!(devices.windows(2).all(|pair| pair[0].id < pair[1].id));
    assert!(devices.iter().all(|device| device.id != bob.device_id));
    assert_eq!(devices.iter().filter(|device| !device.revoked).count(), 1);
    assert!(devices
        .iter()
        .any(|device| device.id == current && !device.revoked));
    let other = store.list_devices(&bob_token, None, NOW).unwrap();
    assert_eq!(other.devices.len(), 1);
    assert_eq!(other.devices[0].id, bob.device_id);
    assert!(store.list_devices(&token, Some("bad"), NOW).is_err());
    assert!(matches!(
        store.list_devices(&token, None, devices[0].expires_at),
        Err(StoreError::Unauthorized)
    ));
}

#[tokio::test]
async fn device_inventory_http_rejects_unauthenticated_browser_and_invalid_queries() {
    let dir = tempfile::tempdir().unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut store = configured(&dir.path().join("sigil.db"));
    let invitation = store
        .invite(
            InviteRequest {
                username: "alice".into(),
                expires_in_seconds: 60,
            },
            now,
        )
        .unwrap();
    let token = random_secret().unwrap();
    let session = enroll(&mut store, &invitation.secret, &token, now).unwrap();
    let app = router(
        store,
        AdminToken::load_or_create(&dir.path().join("admin.token")).unwrap(),
    );
    let (status, page) = request(
        &app,
        "GET",
        "/client/v0/devices",
        Some(&token),
        serde_json::Value::Null,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(page["account_id"], session.account_id);
    assert_eq!(page["devices"][0]["id"], session.device_id);
    assert_eq!(
        request(
            &app,
            "GET",
            "/client/v0/devices",
            None,
            serde_json::Value::Null
        )
        .await
        .0,
        401
    );
    for query in [
        "after=",
        "after=bad",
        "account_id=alice",
        "after=bad&after=bad",
    ] {
        assert_eq!(
            request(
                &app,
                "GET",
                &format!("/client/v0/devices?{query}"),
                Some(&token),
                serde_json::Value::Null
            )
            .await
            .0,
            400
        );
    }
    let response = app
        .oneshot(
            Request::builder()
                .uri("/client/v0/devices")
                .header("authorization", format!("Bearer {token}"))
                .header("origin", "https://chat.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
}

#[test]
fn device_inventory_migrates_schema_nine_without_changing_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let mut store = configured(&path);
    let invitation = invite(&mut store, "alice");
    let token = random_secret().unwrap();
    let session = enroll(&mut store, &invitation.secret, &token, NOW).unwrap();
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("DROP TABLE profile_shares; DROP TABLE profile_photos; DROP TABLE contact_requests; DROP TABLE contact_request_policy; DROP TABLE account_passwords; DROP TABLE password_policy; DROP TABLE web_oidc; DROP TABLE web_sessions; DROP TABLE oidc_fallback_ack; DROP TABLE oidc_transition; DROP TABLE account_profiles; DROP TABLE web_owner; DROP TABLE deleted_accounts; DROP TABLE operation_uploads; DROP TABLE operations; DROP TABLE operation_configuration; DROP TABLE oidc_grants; DROP TABLE oidc_bindings; DROP TABLE oidc_flows; DROP TABLE oidc_configuration; DROP TABLE registration_usage; DROP TABLE account_policy; DROP TABLE admin_policy; DROP TABLE call_connections; DROP TABLE calls; DROP TABLE call_configuration; DROP TABLE service_budgets; DROP TABLE service_configuration; DROP TABLE map_configuration; DROP TABLE private_group_invitations; DROP TABLE private_group_proposals; DROP TABLE private_group_nonces; DROP TABLE private_group_commits; DROP TABLE private_group_members; DROP TABLE private_groups; DROP TABLE group_credential_uids; DROP TABLE group_authority; DROP INDEX prekeys_available; DROP INDEX prekeys_remote_claim; ALTER TABLE prekeys DROP COLUMN remote_request; ALTER TABLE prekeys DROP COLUMN remote_device; ALTER TABLE prekeys DROP COLUMN remote_account; ALTER TABLE prekeys DROP COLUMN remote_server; CREATE INDEX prekeys_available ON prekeys(device_id,expires_at) WHERE bundle IS NOT NULL AND claimant IS NULL; DROP TABLE federation_outbox; DROP TABLE federation_revocations; DROP TABLE federation_senders; DROP TABLE federation_nonces; DROP TABLE federation_admission; DROP TABLE federation_usage; DROP TABLE federation_peers; DROP TABLE federation_configuration; DROP TABLE push_jobs; DROP TABLE push_channels; DROP TABLE push_configuration; DROP TABLE attachment_chunks; DROP TABLE attachments; DROP TABLE retained_storage; DROP INDEX mailbox_live_recipient; DROP INDEX prekeys_available; DROP INDEX devices_active_account; DROP TABLE cancelled_device_links; DROP TABLE device_links; DROP INDEX devices_account_id; DROP TABLE IF EXISTS push_android; PRAGMA user_version=9;")
        .unwrap();
    drop(db);
    let mut store = Store::open(&path).unwrap();
    assert_eq!(store.session(&token, NOW).unwrap(), session);
    assert_eq!(
        store.list_devices(&token, None, NOW).unwrap().devices[0].id,
        session.device_id
    );
    let db = rusqlite::Connection::open(&path).unwrap();
    let columns: Vec<String> = db
        .prepare("PRAGMA index_info(devices_account_id)")
        .unwrap()
        .query_map([], |r| r.get(2))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(columns, ["account_id", "id"]);
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        sigil_server::store::SCHEMA_VERSION
    );
}
