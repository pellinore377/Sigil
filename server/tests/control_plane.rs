use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use sigil_protocol::{Configuration, Configure, Settings, DEFAULT_ATTACHMENT_LIMIT, DEFAULT_QUOTA};
use sigil_server::{
    auth::AdminToken,
    router,
    store::{lock_directory, Store, StoreError},
};
use std::{
    fs,
    os::unix::fs::{symlink, PermissionsExt},
};
use tower::ServiceExt;

fn settings() -> Settings {
    Settings {
        server_name: "chat.example".into(),
        default_quota_bytes: DEFAULT_QUOTA,
        max_attachment_bytes: DEFAULT_ATTACHMENT_LIMIT,
    }
}
fn update(revision: u64) -> Configure {
    Configure {
        expected_revision: revision,
        settings: settings(),
    }
}
struct Fixture {
    _dir: tempfile::TempDir,
    app: Router,
    token: String,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("admin.token");
        let token = AdminToken::load_or_create(&path).unwrap();
        Self {
            app: router(Store::open(&dir.path().join("sigil.db")).unwrap(), token),
            token: fs::read_to_string(path).unwrap(),
            _dir: dir,
        }
    }
    async fn request(
        &self,
        method: &str,
        path: &str,
        body: String,
        authorized: bool,
    ) -> axum::response::Response {
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");
        if authorized {
            builder = builder.header("authorization", format!("Bearer {}", self.token));
        }
        self.app
            .clone()
            .oneshot(builder.body(Body::from(body)).unwrap())
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn admin_requires_authentication_for_reads_and_writes() {
    let fixture = Fixture::new();
    for method in ["GET", "PUT"] {
        let response = fixture
            .request(method, "/admin/v0/configuration", "{}".into(), false)
            .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(response.headers()["www-authenticate"], "Bearer");
    }
    let mut request = Request::builder()
        .uri("/admin/v0/configuration")
        .header("authorization", format!("Bearer {}", fixture.token))
        .header("authorization", "Bearer invalid")
        .body(Body::empty())
        .unwrap();
    let response = fixture.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    request = Request::builder()
        .uri("/admin/v0/configuration")
        .header("authorization", format!("Bearer {}", fixture.token))
        .header("origin", "https://untrusted.example")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        fixture.app.oneshot(request).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn setup_readiness_and_revision_conflicts_round_trip() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture
            .request("GET", "/healthz", String::new(), false)
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        fixture
            .request("GET", "/readyz", String::new(), false)
            .await
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    let response = fixture
        .request(
            "PUT",
            "/admin/v0/configuration",
            serde_json::to_string(&update(0)).unwrap(),
            true,
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let value: Configuration =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    assert_eq!(value.revision, 1);
    assert_eq!(value.settings, Some(settings()));
    assert_eq!(
        fixture
            .request("GET", "/readyz", String::new(), false)
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        fixture
            .request(
                "PUT",
                "/admin/v0/configuration",
                serde_json::to_string(&update(0)).unwrap(),
                true
            )
            .await
            .status(),
        StatusCode::CONFLICT
    );
    let response = fixture
        .request("GET", "/admin/v0/configuration", String::new(), true)
        .await;
    let current: Configuration =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    assert_eq!(current, value);
}

#[tokio::test]
async fn malformed_unknown_and_oversized_input_cannot_change_configuration() {
    let fixture = Fixture::new();
    for body in [
        "{".to_owned(),
        "{\"expected_revision\":0,\"settings\":{},\"unknown\":true}".into(),
        "x".repeat(8193),
    ] {
        assert!(fixture
            .request("PUT", "/admin/v0/configuration", body, true)
            .await
            .status()
            .is_client_error());
    }
    let mut invalid = update(0);
    invalid.settings.server_name = "https://chat.example".into();
    assert_eq!(
        fixture
            .request(
                "PUT",
                "/admin/v0/configuration",
                serde_json::to_string(&invalid).unwrap(),
                true
            )
            .await
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        fixture
            .request("GET", "/readyz", String::new(), false)
            .await
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn protocol_versions_do_not_advertise_unimplemented_security() {
    let fixture = Fixture::new();
    let response = fixture
        .request("GET", "/versions", String::new(), false)
        .await;
    let value: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    assert_eq!(value["admin"], serde_json::json!([0]));
    assert_eq!(value["client"], serde_json::json!([0]));
    assert_eq!(value["enrollment"], serde_json::json!([0]));
    assert_eq!(value["reauthorization"], serde_json::json!([0]));
    assert_eq!(value["recovery_storage"], serde_json::json!([0]));
    for protocol in ["federation", "encrypted_event", "recovery"] {
        assert_eq!(value[protocol], serde_json::json!([]));
    }
    assert_eq!(
        fixture
            .request("PUT", "/admin/v1/configuration", "{}".into(), true)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        fixture
            .request("POST", "/admin/v0/configuration", "{}".into(), true)
            .await
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
}

#[test]
fn configuration_survives_restart_and_cannot_rename_the_homeserver() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let mut store = Store::open(&path).unwrap();
    store.configure(update(0)).unwrap();
    drop(store);
    let mut store = Store::open(&path).unwrap();
    assert_eq!(store.configuration().unwrap().settings, Some(settings()));
    let mut renamed = update(1);
    renamed.settings.server_name = "elsewhere.example".into();
    assert!(matches!(
        store.configure(renamed),
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(store.configuration().unwrap().revision, 1);
}

#[test]
fn stale_writer_cannot_overwrite_a_committed_update() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let mut first = Store::open(&path).unwrap();
    let mut second = Store::open(&path).unwrap();
    first.configure(update(0)).unwrap();
    assert!(matches!(
        second.configure(update(0)),
        Err(StoreError::Conflict)
    ));
    assert_eq!(second.configuration().unwrap().revision, 1);
}

#[test]
fn backup_restores_configuration_without_overwriting_existing_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let backup = dir.path().join("backup.db");
    let restored = dir.path().join("restored.db");
    let mut store = Store::open(&path).unwrap();
    let expected = store.configure(update(0)).unwrap();
    store.backup(&backup).unwrap();
    let header = fs::read(&backup).unwrap();
    assert_eq!(
        &header[18..20],
        &[1, 1],
        "backup must not require WAL sidecars"
    );
    assert!(!backup.with_extension("db-wal").exists());
    assert!(!backup.with_extension("db-shm").exists());
    assert!(store.backup(&backup).is_err());
    Store::restore(&backup, &restored).unwrap();
    assert_eq!(
        Store::open(&restored).unwrap().configuration().unwrap(),
        expected
    );
    assert!(Store::restore(&backup, &restored).is_err());
    assert_eq!(
        fs::metadata(&backup).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn future_and_foreign_databases_are_refused_without_rewriting_them() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("future.db");
    drop(Store::open(&path).unwrap());
    let db = rusqlite::Connection::open(&path).unwrap();
    db.pragma_update(None, "user_version", 999).unwrap();
    drop(db);
    assert!(Store::open(&path).is_err());
    assert!(Store::restore(&path, &dir.path().join("restored.db")).is_err());
    let db = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        999
    );
    let foreign = dir.path().join("foreign.db");
    let db = rusqlite::Connection::open(&foreign).unwrap();
    db.execute_batch("CREATE TABLE unrelated(value TEXT);")
        .unwrap();
    drop(db);
    assert!(Store::open(&foreign).is_err());
}

#[test]
fn invalid_backup_never_creates_a_destination() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("invalid");
    let destination = dir.path().join("restore.db");
    fs::write(&source, b"not a sqlite database").unwrap();
    assert!(Store::restore(&source, &destination).is_err());
    assert!(!destination.exists());
}

#[test]
fn schema_views_cannot_masquerade_as_configuration_during_open_or_backup() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.db");
    let destination = dir.path().join("backup.db");
    let mut store = Store::open(&source).unwrap();
    store.configure(update(0)).unwrap();
    let db = rusqlite::Connection::open(&source).unwrap();
    db.execute_batch("ALTER TABLE configuration RENAME TO configuration_data; CREATE VIEW configuration AS SELECT * FROM configuration_data;").unwrap();
    assert!(Store::open(&source).is_err());
    assert!(store.backup(&destination).is_err());
    assert!(Store::restore(&source, &destination).is_err());
    assert!(!destination.exists());
    // Rejection preserves the original schema and stored configuration.
    assert_eq!(
        db.query_row("SELECT revision FROM configuration_data", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn token_rotation_invalidates_the_previous_credential_on_reload() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("admin.token");
    let first = AdminToken::load_or_create(&path).unwrap();
    let previous = fs::read_to_string(&path).unwrap();
    assert!(first.accepts(&previous));
    assert!(!first.accepts("invalid"));
    AdminToken::rotate(&path).unwrap();
    let current = AdminToken::load_or_create(&path).unwrap();
    assert!(!current.accepts(&previous));
    assert!(current.accepts(&fs::read_to_string(&path).unwrap()));
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn private_storage_rejects_shared_directories_symlinks_and_parallel_processes() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("private");
    let lock = lock_directory(&data).unwrap();
    assert!(lock_directory(&data).is_err());
    drop(lock);
    drop(lock_directory(&data).unwrap());
    fs::set_permissions(&data, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(lock_directory(&data).is_err());
    let link = dir.path().join("link");
    symlink(&data, &link).unwrap();
    assert!(lock_directory(&link).is_err());
    let token = dir.path().join("admin.token");
    fs::write(&token, "a".repeat(64)).unwrap();
    fs::set_permissions(&token, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(AdminToken::load_or_create(&token).is_err());
    let database = dir.path().join("sigil.db");
    drop(Store::open(&database).unwrap());
    fs::set_permissions(&database, fs::Permissions::from_mode(0o644)).unwrap();
    let before = fs::read(&database).unwrap();
    assert!(Store::open(&database).is_err());
    assert_eq!(fs::read(&database).unwrap(), before);
    fs::set_permissions(&database, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(Store::open(&database).is_ok());
    assert!(Store::open(dir.path()).is_err());
}

#[test]
fn storage_defaults_are_applied_and_attachment_limit_is_configurable() {
    let default: Settings = serde_json::from_str(r#"{"server_name":"chat.example"}"#).unwrap();
    assert_eq!(default.default_quota_bytes, DEFAULT_QUOTA);
    assert_eq!(default.max_attachment_bytes, DEFAULT_ATTACHMENT_LIMIT);
    let larger = Settings {
        max_attachment_bytes: 2 * DEFAULT_ATTACHMENT_LIMIT,
        ..default
    };
    assert!(larger.validate().is_ok());
}
