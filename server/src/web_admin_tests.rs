use super::*;
use crate::{auth::AdminToken, router};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::ServiceExt;

fn setup(store: &mut Store, now: u64) -> String {
    let code = store.web_setup_code().unwrap().unwrap();
    store
        .web_claim(
            Claim {
                code: Zeroizing::new(code),
                password: Zeroizing::new("a unique lengthy test passphrase".into()),
                server_name: "example.test".into(),
                public_origin: "https://sigil.example.test".into(),
            },
            now,
        )
        .unwrap()
}
#[test]
fn ownership_is_atomic_and_password_sessions_expire() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("sigil.db")).unwrap();
    let initial = s.web_setup_code().unwrap().unwrap();
    let invalid = s.web_claim(
        Claim {
            code: Zeroizing::new(initial.clone()),
            password: Zeroizing::new("short".into()),
            server_name: "example.test".into(),
            public_origin: "https://sigil.example.test".into(),
        },
        10000,
    );
    assert!(invalid.is_err());
    assert!(s.configuration().unwrap().settings.is_none());
    let session = s
        .web_claim(
            Claim {
                code: Zeroizing::new(initial.clone()),
                password: Zeroizing::new("a unique lengthy test passphrase".into()),
                server_name: "example.test".into(),
                public_origin: "https://sigil.example.test".into(),
            },
            10000,
        )
        .unwrap();
    assert!(s.web_setup_code().unwrap().is_none());
    assert!(s
        .web_claim(
            Claim {
                code: Zeroizing::new(initial),
                password: Zeroizing::new("another lengthy test passphrase".into()),
                server_name: "example.test".into(),
                public_origin: "https://sigil.example.test".into()
            },
            10001
        )
        .is_err());
    let hash: String =
        s.0.query_row("SELECT password FROM web_owner", [], |r| r.get(0))
            .unwrap();
    assert!(hash.starts_with("$argon2id$v=19$m=65536,t=3,p=1$"));
    assert!(!hash.contains("passphrase"));
    s.web_finish_setup(&session, "admin", 10002).unwrap();
    assert_eq!(
        s.web_status(Some(&session), 10003)
            .unwrap()
            .username
            .as_deref(),
        Some("admin")
    );
    assert!(s.web_password_policy(&session, false, 10004).is_err());
    assert!(s
        .web_login(
            Login {
                username: "other".into(),
                password: Zeroizing::new("a unique lengthy test passphrase".into())
            },
            10005
        )
        .is_err());
    assert!(s
        .web_login(
            Login {
                username: "admin".into(),
                password: Zeroizing::new("a unique lengthy test passphrase".into())
            },
            10006
        )
        .is_err());
    let token = s
        .web_login(
            Login {
                username: "admin".into(),
                password: Zeroizing::new("a unique lengthy test passphrase".into()),
            },
            10008,
        )
        .unwrap();
    assert!(s.web_session(&token, 10009).is_ok());
    s.web_logout(&token).unwrap();
    assert!(s.web_session(&token, 10010).is_err());
    assert!(s.web_session(&session, 10004 + 1800).is_err());
}
#[test]
fn restore_revokes_browser_access_and_reopens_local_claim() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("source.db")).unwrap();
    let token = setup(&mut s, 10000);
    s.web_finish_setup(&token, "admin", 10001).unwrap();
    s.backup(&dir.path().join("backup.db")).unwrap();
    Store::restore(
        &dir.path().join("backup.db"),
        &dir.path().join("restored.db"),
    )
    .unwrap();
    let mut restored = Store::open(&dir.path().join("restored.db")).unwrap();
    assert!(restored.web_session(&token, 10002).is_err());
    assert!(!restored.web_status(None, 10002).unwrap().claimed);
    assert!(restored.web_setup_code().unwrap().is_some());
}
#[tokio::test]
async fn browser_routes_require_ownership_origin_and_session_proof() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("sigil.db")).unwrap();
    let code = store.web_setup_code().unwrap().unwrap();
    let token = AdminToken::load_or_create(&dir.path().join("admin.token")).unwrap();
    let app = router(store, token);
    for path in ["/", "/web/index.html", "/setup/v0/status"] {
        let response = app
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert!(response.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("frame-ancestors 'none'"));
        assert_eq!(response.headers()["referrer-policy"], "no-referrer");
    }
    let body = serde_json::json!({"code":code,"password":"long enough synthetic passphrase","server_name":"example.test","public_origin":"https://sigil.example.test"});
    for origin in [None, Some("https://attacker.example")] {
        let mut req = Request::post("/setup/v0/claim")
            .header("content-type", "application/json")
            .header("x-sigil-admin", "1");
        if let Some(o) = origin {
            req = req.header("origin", o);
        }
        assert_eq!(
            app.clone()
                .oneshot(req.body(Body::from(body.to_string())).unwrap())
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    let response = app
        .clone()
        .oneshot(
            Request::post("/setup/v0/claim")
                .header("content-type", "application/json")
                .header("x-sigil-admin", "1")
                .header("origin", "https://sigil.example.test")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let set_cookie = response.headers()["set-cookie"].to_str().unwrap();
    for flag in ["Secure", "HttpOnly", "SameSite=Lax", "Path=/"] {
        assert!(set_cookie.contains(flag));
    }
    let cookie = set_cookie.split(';').next().unwrap();
    for (proof, origin, expected) in [
        (false, None, StatusCode::UNAUTHORIZED),
        (
            true,
            Some("https://attacker.example"),
            StatusCode::FORBIDDEN,
        ),
        (true, None, StatusCode::OK),
    ] {
        let mut req = Request::get("/admin/v0/configuration").header("cookie", cookie);
        if proof {
            req = req.header("x-sigil-admin", "1");
        }
        if let Some(o) = origin {
            req = req.header("origin", o);
        }
        assert_eq!(
            app.clone()
                .oneshot(req.body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
            expected
        );
    }
    let duplicate = format!("{cookie}; {cookie}");
    assert_eq!(
        app.clone()
            .oneshot(
                Request::get("/admin/v0/configuration")
                    .header("cookie", duplicate)
                    .header("x-sigil-admin", "1")
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
}
