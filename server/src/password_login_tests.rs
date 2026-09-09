use super::*;
fn request(username: &str, password: &str, credential: &str) -> Login {
    Login {
        username: username.into(),
        password: Zeroizing::new(password.into()),
        device_credential: credential.into(),
        device_label: "Test phone".into(),
    }
}
const PASSWORD: &str = "a synthetic long account password";
#[test]
fn policy_password_checks_and_retry_preserve_account_and_devices() {
    let (dir, mut s, alice, _, now) = crate::admin::tests::setup();
    let old = s.session(&alice, now).unwrap();
    let credential = "aa".repeat(32);
    s.set_user_password(
        &old.account_id,
        SetPassword {
            password: Zeroizing::new(PASSWORD.into()),
        },
    )
    .unwrap();
    assert!(!s.login_methods().unwrap().password);
    assert!(s
        .password_sign_in(request("alice", PASSWORD, &credential), now)
        .is_err());
    let policy = s
        .configure_user_passwords(PasswordPolicy {
            revision: 0,
            enabled: true,
        })
        .unwrap();
    assert!(s
        .configure_user_passwords(PasswordPolicy {
            revision: 0,
            enabled: false
        })
        .is_err());
    assert!(s
        .password_sign_in(request("alice", "wrong", &credential), now + 1)
        .is_err());
    assert!(s
        .password_sign_in(request("unknown", PASSWORD, &credential), now + 2)
        .is_err());
    assert!(s
        .password_sign_in(request("alice", PASSWORD, &credential), now + 3)
        .is_err());
    assert_eq!(s.session(&alice, now + 3).unwrap(), old);
    s.0.execute(
        "UPDATE devices SET revoked=1,token_hash=NULL WHERE account_id=?1",
        [&old.account_id],
    )
    .unwrap();
    let session = s
        .password_sign_in(request("alice", PASSWORD, &credential), now + 4)
        .unwrap();
    assert_eq!(session.account_id, old.account_id);
    assert_eq!(session.address, old.address);
    drop(s);
    let mut s = Store::open(&dir.path().join("sigil.db")).unwrap();
    assert_eq!(
        s.password_sign_in(request("alice", PASSWORD, &credential), now + 5)
            .unwrap(),
        session
    );
    assert!(s
        .password_sign_in(request("alice", PASSWORD, &"bb".repeat(32)), now + 6)
        .is_err());
    s.configure_user_passwords(PasswordPolicy {
        enabled: false,
        ..policy
    })
    .unwrap();
    assert!(s
        .password_sign_in(request("alice", PASSWORD, &credential), now + 7)
        .is_err());
    assert_eq!(s.session(&credential, now + 7).unwrap(), session);
    s.0.execute(
        "UPDATE accounts SET disabled=1 WHERE id=?1",
        [&old.account_id],
    )
    .unwrap();
    let policy = s.user_password_policy().unwrap();
    s.configure_user_passwords(PasswordPolicy {
        enabled: true,
        ..policy
    })
    .unwrap();
    assert!(s
        .password_sign_in(request("alice", PASSWORD, &credential), now + 8)
        .is_err());
}
#[test]
fn migration_defaults_closed_and_keeps_existing_sessions() {
    let (dir, s, alice, _, now) = crate::admin::tests::setup();
    let prior = s.session(&alice, now).unwrap();
    s.0.execute_batch("DROP TABLE contact_requests; DROP TABLE contact_request_policy; DROP TABLE account_passwords; DROP TABLE password_policy; ALTER TABLE oidc_grants DROP COLUMN replace_devices; PRAGMA user_version=29;").unwrap();
    drop(s);
    let s = Store::open(&dir.path().join("sigil.db")).unwrap();
    assert!(!s.user_password_policy().unwrap().enabled);
    assert_eq!(s.session(&alice, now).unwrap(), prior);
}
#[tokio::test]
async fn routes_reject_unauthorized_admin_changes_and_browser_password_requests() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let (dir, s, _, _, _) = crate::admin::tests::setup();
    let token = crate::auth::AdminToken::load_or_create(&dir.path().join("admin.token")).unwrap();
    let app = crate::router(s, token);
    let response = app
        .clone()
        .oneshot(
            Request::get("/client/v0/login")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    for path in [
        "/admin/v0/password-login",
        "/admin/v0/accounts/alice/password",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::put(path)
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    let response = app
        .oneshot(
            Request::post("/client/v0/login/password")
                .header("origin", "https://example.test")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[test]
fn passwords_have_independent_salts_and_restore_does_not_revive_old_passwords() {
    let (dir, mut s, alice, bob, now) = crate::admin::tests::setup();
    for credential in [&alice, &bob] {
        let account = s.session(credential, now).unwrap().account_id;
        s.set_user_password(
            &account,
            SetPassword {
                password: Zeroizing::new(PASSWORD.into()),
            },
        )
        .unwrap();
    }
    let hashes: Vec<String> =
        s.0.prepare("SELECT hash FROM account_passwords ORDER BY account")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
    assert_ne!(hashes[0], hashes[1]);
    assert!(hashes
        .iter()
        .all(|h| h.starts_with("$argon2id$v=19$m=65536,t=3,p=1$")
            && verify_password(h, PASSWORD).unwrap()));
    s.configure_user_passwords(PasswordPolicy {
        revision: 0,
        enabled: true,
    })
    .unwrap();
    let backup = dir.path().join("backup.db");
    s.backup(&backup).unwrap();
    let restored = dir.path().join("restored.db");
    Store::restore(&backup, &restored).unwrap();
    let restored = Store::open(&restored).unwrap();
    assert!(!restored.user_password_policy().unwrap().enabled);
    assert_eq!(
        restored
            .0
            .query_row("SELECT count(*) FROM account_passwords", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
