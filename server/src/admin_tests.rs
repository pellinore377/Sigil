use super::*;
use crate::auth::{digest, random_secret, AdminToken};
use sigil_protocol::{
    accounts::{Enrollment, InviteRequest},
    Configure, Settings,
};
pub(crate) fn setup() -> (tempfile::TempDir, Store, String, String, u64) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("sigil.db")).unwrap();
    store
        .configure(Configure {
            expected_revision: 0,
            settings: Settings {
                server_name: "chat.example".into(),
                default_quota_bytes: 16 * 1024 * 1024,
                max_attachment_bytes: 1024 * 1024,
            },
        })
        .unwrap();
    let now = crate::enrollment::now().unwrap();
    let mut credentials = Vec::new();
    for username in ["alice", "bob"] {
        let invitation = store
            .invite(
                InviteRequest {
                    username: username.into(),
                    expires_in_seconds: 600,
                },
                now,
            )
            .unwrap();
        let token = random_secret().unwrap();
        store
            .enroll(
                Enrollment {
                    invitation: invitation.secret,
                    device_credential: token.clone(),
                    device_label: "Synthetic".into(),
                },
                now,
            )
            .unwrap();
        credentials.push(token);
    }
    (
        dir,
        store,
        credentials.remove(0),
        credentials.remove(0),
        now,
    )
}
#[test]
fn policy_roles_discovery_and_quota_are_revisioned_and_do_not_grant_contact_access() {
    let (dir, mut store, alice, bob, now) = setup();
    let a = store.session(&alice, now).unwrap();
    let b = store.session(&bob, now).unwrap();
    let found = store.discover_account(&alice, "bob", now).unwrap();
    assert_eq!(found.account, b.account_id);
    assert_eq!(found.devices, vec![b.device_id.clone()]);
    assert!(crate::admission::check(&store.0, &a.device_id, &b.device_id).is_err());
    assert!(store.discover_account(&alice, "bo", now).is_err());
    let pref = store
        .discovery_preference(
            &bob,
            Some(DiscoveryPreference {
                revision: 0,
                discoverable: false,
            }),
            now,
        )
        .unwrap();
    assert!(store.discover_account(&alice, "bob", now).is_err());
    assert!(store
        .discovery_preference(
            &bob,
            Some(DiscoveryPreference {
                revision: 0,
                discoverable: true
            }),
            now
        )
        .is_err());
    let update = AccountUpdate {
        expected_revision: 0,
        role: Role::Administrator,
        disabled: false,
        quota_bytes: Some(1024 * 1024),
        confirm: true,
    };
    store
        .admin_update_account(&a.account_id, update, now)
        .unwrap();
    assert_eq!(store.admin_role(&alice, now).unwrap(), Role::Administrator);
    let current = store
        .admin_accounts(None, now)
        .unwrap()
        .accounts
        .into_iter()
        .find(|v| v.id == a.account_id)
        .unwrap();
    assert_eq!(current.quota_bytes, 1024 * 1024);
    assert_eq!(
        store.account_storage(&alice, now).unwrap().quota_bytes,
        1024 * 1024
    );
    assert!(store
        .admin_update_account(
            &a.account_id,
            AccountUpdate {
                expected_revision: current.revision,
                role: Role::Member,
                disabled: false,
                quota_bytes: None,
                confirm: true
            },
            now
        )
        .is_err());
    store
        .admin_update_account(
            &b.account_id,
            AccountUpdate {
                expected_revision: pref.revision,
                role: Role::Administrator,
                disabled: false,
                quota_bytes: None,
                confirm: true,
            },
            now,
        )
        .unwrap();
    assert!(store.discover_account(&alice, "bob", now).is_err());
    store.disable_account(&a.account_id).unwrap();
    assert!(store.admin_role(&alice, now).is_err());
    drop(store);
    let mut store = Store::open(&dir.path().join("sigil.db")).unwrap();
    assert_eq!(store.admin_role(&bob, now).unwrap(), Role::Administrator);
    let diagnostics = store.admin_diagnostics(now).unwrap().to_string();
    for sensitive in [
        alice,
        bob,
        a.account_id,
        b.account_id,
        "chat.example".into(),
        "alice".into(),
    ] {
        assert!(!diagnostics.contains(&sensitive));
    }
}
#[test]
fn registration_limits_and_storage_failures_roll_back_and_survive_restart() {
    let (dir, mut store, alice, _, now) = setup();
    let mut policy = store.administration_policy().unwrap();
    policy.registrations_per_day = 3;
    store.configure_administration(policy).unwrap();
    let invitation = store
        .invite(
            InviteRequest {
                username: "carol".into(),
                expires_in_seconds: 600,
            },
            now,
        )
        .unwrap();
    let credential = random_secret().unwrap();
    let enrollment = || Enrollment {
        invitation: invitation.secret.clone(),
        device_credential: credential.clone(),
        device_label: "Synthetic".into(),
    };
    store.0.execute_batch("CREATE TRIGGER synthetic BEFORE INSERT ON devices BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(store.enroll(enrollment(), now).is_err());
    assert_eq!(
        store
            .0
            .query_row("SELECT count FROM registration_usage", [], |r| r
                .get::<_, u32>(0))
            .unwrap(),
        2
    );
    store.0.execute_batch("DROP TRIGGER synthetic").unwrap();
    store.enroll(enrollment(), now).unwrap();
    let invitation = store
        .invite(
            InviteRequest {
                username: "dave".into(),
                expires_in_seconds: 600,
            },
            now,
        )
        .unwrap();
    drop(store);
    let mut store = Store::open(&dir.path().join("sigil.db")).unwrap();
    assert!(matches!(
        store.enroll(
            Enrollment {
                invitation: invitation.secret,
                device_credential: random_secret().unwrap(),
                device_label: "Synthetic".into()
            },
            now
        ),
        Err(StoreError::Busy)
    ));
    let a = store.session(&alice, now).unwrap();
    store
        .admin_update_account(
            &a.account_id,
            AccountUpdate {
                expected_revision: 0,
                role: Role::Member,
                disabled: false,
                quota_bytes: Some(1024 * 1024),
                confirm: true,
            },
            now,
        )
        .unwrap();
    let tx = store.0.transaction().unwrap();
    assert!(crate::storage_budget::reserve(&tx, &a.account_id, 1024 * 1024, now).is_err());
    tx.rollback().unwrap();
    let mut policy = store.administration_policy().unwrap();
    policy.registration = Registration::Closed;
    store.configure_administration(policy).unwrap();
    assert_eq!(store.session(&alice, now).unwrap().account_id, a.account_id);
}
#[tokio::test]
async fn roles_and_origins_are_enforced_on_existing_and_new_routes() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let (dir, mut store, alice, bob, now) = setup();
    let a = store.session(&alice, now).unwrap();
    let b = store.session(&bob, now).unwrap();
    store
        .admin_update_account(
            &a.account_id,
            AccountUpdate {
                expected_revision: 0,
                role: Role::Auditor,
                disabled: false,
                quota_bytes: None,
                confirm: true,
            },
            now,
        )
        .unwrap();
    store
        .admin_update_account(
            &b.account_id,
            AccountUpdate {
                expected_revision: 0,
                role: Role::Operator,
                disabled: false,
                quota_bytes: None,
                confirm: true,
            },
            now,
        )
        .unwrap();
    let mut policy = store.administration_policy().unwrap();
    policy.public_origin = Some("https://chat.example".into());
    store.configure_administration(policy).unwrap();
    let token = AdminToken::load_or_create(&dir.path().join("admin.token")).unwrap();
    let app = crate::router(store, token);
    for (credential, method, path, origin, expected) in [
        (&alice, "GET", "/admin/v0/setup", None, StatusCode::OK),
        (
            &alice,
            "PUT",
            "/admin/v0/configuration",
            None,
            StatusCode::FORBIDDEN,
        ),
        (
            &alice,
            "GET",
            "/admin/v0/maintenance/files/invalid/0",
            None,
            StatusCode::FORBIDDEN,
        ),
        (&bob, "PUT", "/admin/v0/push", None, StatusCode::FORBIDDEN),
        (
            &bob,
            "GET",
            "/admin/v0/diagnostics",
            Some("https://chat.example"),
            StatusCode::OK,
        ),
        (
            &bob,
            "GET",
            "/admin/v0/diagnostics",
            Some("https://evil.example"),
            StatusCode::FORBIDDEN,
        ),
    ] {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("Bearer {credential}"))
            .header("content-type", "application/json");
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::from("{}")).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected, "{method} {path}");
    }
    let db = Connection::open(dir.path().join("sigil.db")).unwrap();
    db.execute(
        "UPDATE devices SET revoked=1 WHERE token_hash=?1",
        [digest(&alice).as_slice()],
    )
    .unwrap();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/admin/v0/diagnostics")
                .header("authorization", format!("Bearer {alice}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
