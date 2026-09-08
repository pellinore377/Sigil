#[path = "fixtures/maps.rs"]
mod fixture;
use axum::{
    body::{to_bytes, Body},
    http::Request,
    Router,
};
use sigil_server::{
    auth::{random_secret, AdminToken},
    maps, router,
    store::Store,
};
use tower::ServiceExt;
async fn request(
    app: &Router,
    method: &str,
    path: &str,
    credential: &str,
    body: serde_json::Value,
) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {credential}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}
#[tokio::test]
async fn map_configuration_serving_restart_and_disable_require_authority() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let mut store = Store::open(&path).unwrap();
    store
        .configure(sigil_protocol::Configure {
            expected_revision: 0,
            settings: serde_json::from_value(serde_json::json!({"server_name":"chat.example"}))
                .unwrap(),
        })
        .unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let invitation = store
        .invite(
            sigil_protocol::accounts::InviteRequest {
                username: "synthetic".into(),
                expires_in_seconds: 60,
            },
            now,
        )
        .unwrap();
    let credential = random_secret().unwrap();
    store
        .enroll(
            sigil_protocol::accounts::Enrollment {
                invitation: invitation.secret,
                device_credential: credential.clone(),
                device_label: "Synthetic".into(),
            },
            now,
        )
        .unwrap();
    let admin = dir.path().join("admin.token");
    let token = AdminToken::load_or_create(&admin).unwrap();
    let secret = std::fs::read_to_string(&admin).unwrap();
    let app = router(store, token);
    let settings = fixture::maps(dir.path());
    let update = serde_json::to_value(maps::Configure {
        expected_revision: 0,
        settings: Some(settings),
    })
    .unwrap();
    assert_eq!(
        request(&app, "PUT", "/admin/v0/maps", &credential, update.clone())
            .await
            .status(),
        403
    );
    assert_eq!(
        request(&app, "PUT", "/admin/v0/maps", &secret, update.clone())
            .await
            .status(),
        200
    );
    assert_eq!(
        request(&app, "PUT", "/admin/v0/maps", &secret, update)
            .await
            .status(),
        200
    );
    for endpoint in [
        "/client/v0/maps",
        "/client/v0/maps/style.json",
        "/client/v0/maps/tiles.json",
        "/client/v0/maps/tiles/0/0/0",
        "/client/v0/maps/assets/sprite.json",
    ] {
        assert_eq!(
            request(&app, "GET", endpoint, "invalid", serde_json::Value::Null)
                .await
                .status(),
            401
        );
        let response = request(&app, "GET", endpoint, &credential, serde_json::Value::Null).await;
        assert_eq!(response.status(), 200);
        let body = to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap();
        assert!(!body
            .windows(dir.path().as_os_str().len())
            .any(|v| v == dir.path().as_os_str().as_encoded_bytes()));
        if endpoint.ends_with("/0/0/0") {
            assert_eq!(&body[..], &[0x1a, 0]);
        }
    }
    drop(app);
    let app = router(
        Store::open(&path).unwrap(),
        AdminToken::load_or_create(&admin).unwrap(),
    );
    assert_eq!(
        request(
            &app,
            "GET",
            "/client/v0/maps/tiles/0/0/0",
            &credential,
            serde_json::Value::Null
        )
        .await
        .status(),
        200
    );
    assert_eq!(
        request(
            &app,
            "PUT",
            "/admin/v0/maps",
            &secret,
            serde_json::json!({"expected_revision":1,"settings":null})
        )
        .await
        .status(),
        200
    );
    assert_eq!(
        request(
            &app,
            "GET",
            "/client/v0/maps/tiles/0/0/0",
            &credential,
            serde_json::Value::Null
        )
        .await
        .status(),
        404
    );
}
