use super::*;
fn config(revision: u64) -> Configure {
    Configure {
        expected_revision: revision,
        enabled: true,
        exceptions: vec![],
        peer_quota_bytes: 1024 * 1024,
        rotate_key: false,
    }
}
fn setup(path: &std::path::Path) -> Store {
    let mut s = Store::open(path).unwrap();
    s.configure(sigil_protocol::Configure {
        expected_revision: 0,
        settings: sigil_protocol::Settings {
            server_name: "chat.example".into(),
            default_quota_bytes: sigil_protocol::DEFAULT_QUOTA,
            max_attachment_bytes: sigil_protocol::DEFAULT_ATTACHMENT_LIMIT,
        },
    })
    .unwrap();
    s
}
fn allow(revision: u64) -> ConfigurePeer {
    ConfigurePeer {
        expected_revision: revision,
        allowed: true,
        port: 443,
        approve_key: None,
    }
}
fn discovery(key: &SigningKey) -> Discovery {
    Discovery {
        version: 0,
        server: "remote.example".into(),
        current: key.descriptor().clone(),
        rotation: None,
    }
}
#[test]
fn key_lifecycle_cas_restart_and_restore() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    let mut s = setup(&path);
    assert!(!s.federation_configuration().unwrap().enabled);
    assert!(matches!(
        s.federation_discovery(),
        Err(StoreError::Forbidden)
    ));
    let first = s.configure_federation(config(0), 1000).unwrap();
    let d = first.discovery.unwrap();
    assert_eq!(s.configure_federation(config(0), 1001).unwrap().revision, 1);
    assert!(
        !serde_json::to_string(&s.federation_configuration().unwrap())
            .unwrap()
            .contains("seed")
    );
    let mut rotate = config(1);
    rotate.rotate_key = true;
    assert!(s.configure_federation(rotate.clone(), 1000).is_err());
    assert_eq!(s.federation_configuration().unwrap().revision, 1);
    let next = s
        .configure_federation(rotate.clone(), 1001)
        .unwrap()
        .discovery
        .unwrap();
    auth::follows(&d.current, &next, "chat.example", 1001).unwrap();
    assert_eq!(
        s.configure_federation(rotate, 1002)
            .unwrap()
            .discovery
            .unwrap(),
        next
    );
    drop(s);
    let s = Store::open(&path).unwrap();
    assert_eq!(s.federation_discovery().unwrap(), next);
    let backup = dir.path().join("backup.db");
    s.backup(&backup).unwrap();
    let restored = dir.path().join("restored.db");
    Store::restore(&backup, &restored).unwrap();
    let mut restored = Store::open(&restored).unwrap();
    let c = restored.federation_configuration().unwrap();
    assert!(!c.enabled);
    assert!(c.discovery.is_none());
    let fresh = restored
        .configure_federation(config(c.revision), 1100)
        .unwrap()
        .discovery
        .unwrap();
    assert_ne!(fresh.current.id, next.current.id);
    assert_eq!(fresh.current.generation, 1);
}
#[test]
fn peer_refresh_pins_rotation_and_explicit_replacement() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = setup(&dir.path().join("s.db"));
    s.configure_federation(config(0), 1000).unwrap();
    s.configure_federation_peer("remote.example", allow(0))
        .unwrap();
    assert!(matches!(
        s.begin_federation_refresh("unknown.example", 1000),
        Err(StoreError::Forbidden)
    ));
    let a = SigningKey::generate(1, 900).unwrap();
    let b = SigningKey::generate(2, 1001).unwrap();
    let replacement = SigningKey::generate(1, 1001).unwrap();
    let (c, p) = s.begin_federation_refresh("remote.example", 1000).unwrap();
    assert!(matches!(
        s.begin_federation_refresh("remote.example", 1000),
        Err(StoreError::Busy)
    ));
    let p = s
        .finish_federation_refresh(c.revision, &p, Some(discovery(&a)), 1000)
        .unwrap();
    assert_eq!(p.pinned.unwrap().current, a.descriptor().clone());
    assert_eq!(p.not_before, 4600);
    s.configure_federation_peer("remote.example", allow(1))
        .unwrap();
    let (c, p) = s.begin_federation_refresh("remote.example", 1001).unwrap();
    let rotated = Discovery {
        version: 0,
        server: "remote.example".into(),
        current: b.descriptor().clone(),
        rotation: Some(Rotation {
            previous: a.descriptor().clone(),
            signature: a.transition("remote.example", b.descriptor()).unwrap(),
        }),
    };
    let p = s
        .finish_federation_refresh(c.revision, &p, Some(rotated.clone()), 1001)
        .unwrap();
    assert_eq!(p.pinned, Some(rotated.clone()));
    s.configure_federation_peer("remote.example", allow(2))
        .unwrap();
    let (c, p) = s.begin_federation_refresh("remote.example", 1002).unwrap();
    let p = s
        .finish_federation_refresh(c.revision, &p, Some(discovery(&replacement)), 1002)
        .unwrap();
    assert_eq!(p.checked_at, 0);
    assert_eq!(p.error.as_deref(), Some("key_change_requires_approval"));
    assert_eq!(p.pinned, Some(rotated.clone()));
    let mut approve = allow(3);
    approve.approve_key = Some(replacement.descriptor().id.clone());
    s.configure_federation_peer("remote.example", approve)
        .unwrap();
    let (c, p) = s.begin_federation_refresh("remote.example", 1003).unwrap();
    let p = s
        .finish_federation_refresh(c.revision, &p, Some(rotated), 1003)
        .unwrap();
    assert!(p.pending_approval.is_some());
    assert_eq!(p.checked_at, 0);
    let (c, p) = s.begin_federation_refresh("remote.example", 1063).unwrap();
    let p = s
        .finish_federation_refresh(c.revision, &p, Some(discovery(&replacement)), 1063)
        .unwrap();
    assert!(p.pending_approval.is_none());
    assert_eq!(p.pinned.unwrap().current, replacement.descriptor().clone());
}
#[test]
fn stale_network_results_cannot_override_policy_or_peer_changes() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = setup(&dir.path().join("s.db"));
    s.configure_federation(config(0), 1000).unwrap();
    s.configure_federation_peer("remote.example", allow(0))
        .unwrap();
    let a = SigningKey::generate(1, 900).unwrap();
    let (c, p) = s.begin_federation_refresh("remote.example", 1000).unwrap();
    s.configure_federation(config(1), 1001).unwrap();
    assert!(matches!(
        s.finish_federation_refresh(c.revision, &p, Some(discovery(&a)), 1001),
        Err(StoreError::Conflict)
    ));
    let (c, p) = s.begin_federation_refresh("remote.example", 1060).unwrap();
    let mut disabled = allow(1);
    disabled.allowed = false;
    s.configure_federation_peer("remote.example", disabled)
        .unwrap();
    assert!(matches!(
        s.finish_federation_refresh(c.revision, &p, Some(discovery(&a)), 1061),
        Err(StoreError::Conflict)
    ));
    assert!(s
        .federation_peer("remote.example")
        .unwrap()
        .pinned
        .is_none());
    s.0.execute("UPDATE federation_peers SET pinned=?1", ["x".repeat(16385)])
        .unwrap();
    assert!(matches!(
        s.federation_peer("remote.example"),
        Err(StoreError::InvalidData)
    ));
}

fn signed_ping(key: &SigningKey, now: u64, nonce: u8) -> axum::http::HeaderMap {
    auth::sign(
        key,
        &auth::Request {
            origin: "remote.example",
            destination: "chat.example",
            path: sigil_protocol::federation::PING_PATH,
            body: b"{}",
        },
        now,
        [nonce; 32],
    )
    .unwrap()
}
pub(crate) fn trusted(path: &std::path::Path) -> (Store, SigningKey) {
    let mut s = setup(path);
    s.configure_federation(config(0), 1000).unwrap();
    s.configure_federation_peer("remote.example", allow(0))
        .unwrap();
    let key = SigningKey::generate(1, 900).unwrap();
    let (c, p) = s.begin_federation_refresh("remote.example", 1000).unwrap();
    s.finish_federation_refresh(c.revision, &p, Some(discovery(&key)), 1000)
        .unwrap();
    (s, key)
}
#[test]
fn authenticated_admission_is_durable_bounded_and_atomic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    let (mut s, key) = trusted(&path);
    let headers = signed_ping(&key, 1000, 1);
    s.0.execute_batch("CREATE TRIGGER synthetic_failure BEFORE INSERT ON federation_nonces BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(s.federation_ping(b"{}", &headers, 1000).is_err());
    assert_eq!(
        s.0.query_row("SELECT credit FROM federation_admission", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        20
    );
    s.0.execute_batch("DROP TRIGGER synthetic_failure").unwrap();
    s.federation_ping(b"{}", &headers, 1000).unwrap();
    assert!(matches!(
        s.federation_ping(b"{}", &headers, 1000),
        Err(StoreError::Conflict)
    ));
    drop(s);
    let mut s = Store::open(&path).unwrap();
    assert!(matches!(
        s.federation_ping(b"{}", &headers, 1000),
        Err(StoreError::Conflict)
    ));
    for nonce in 2..=20 {
        s.federation_ping(b"{}", &signed_ping(&key, 1000, nonce), 1000)
            .unwrap();
    }
    assert!(matches!(
        s.federation_ping(b"{}", &signed_ping(&key, 1000, 21), 1000),
        Err(StoreError::Busy)
    ));
    s.federation_ping(b"{}", &signed_ping(&key, 1001, 21), 1001)
        .unwrap();
    assert_eq!(
        s.0.query_row("SELECT nonce_bytes FROM federation_usage", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        21 * 1024
    );
    // Backward wall-clock movement cannot refill admission credit.
    assert!(matches!(
        s.federation_ping(b"{}", &signed_ping(&key, 1000, 22), 1000),
        Err(StoreError::Busy)
    ));
    assert_eq!(s.expire_batch(1120).unwrap(), 20);
    assert_eq!(
        s.0.query_row("SELECT nonce_bytes FROM federation_usage", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1024
    );
    assert!(matches!(
        s.federation_ping(b"{}", &headers, 1120),
        Err(StoreError::Unauthorized)
    ));
    assert_eq!(s.expire_batch(1121).unwrap(), 1);
    assert_eq!(
        s.0.query_row("SELECT nonce_bytes FROM federation_admission", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    // An operation failing after admission must roll back the nonce too.
    let headers = signed_ping(&key, 1200, 23);
    {
        let tx =
            s.0.transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
        crate::federation_admission::admit(
            &tx,
            sigil_protocol::federation::PING_PATH,
            b"{}",
            &headers,
            1200,
        )
        .unwrap();
    }
    s.federation_ping(b"{}", &headers, 1200).unwrap();
}
#[test]
fn key_cutover_rejects_post_rotation_old_signatures_and_disabled_peers() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, old) = trusted(&dir.path().join("s.db"));
    let new = SigningKey::generate(2, 1001).unwrap();
    let d = Discovery {
        version: 0,
        server: "remote.example".into(),
        current: new.descriptor().clone(),
        rotation: Some(Rotation {
            previous: old.descriptor().clone(),
            signature: old.transition("remote.example", new.descriptor()).unwrap(),
        }),
    };
    s.configure_federation_peer("remote.example", allow(1))
        .unwrap();
    let (c, p) = s.begin_federation_refresh("remote.example", 1001).unwrap();
    s.finish_federation_refresh(c.revision, &p, Some(d), 1001)
        .unwrap();
    s.federation_ping(b"{}", &signed_ping(&old, 1000, 1), 1001)
        .unwrap();
    assert!(matches!(
        s.federation_ping(b"{}", &signed_ping(&old, 1001, 2), 1001),
        Err(StoreError::Unauthorized)
    ));
    s.federation_ping(b"{}", &signed_ping(&new, 1001, 2), 1001)
        .unwrap();
    assert!(matches!(
        s.federation_ping(b"{}", &signed_ping(&new, 4602, 3), 4602),
        Err(StoreError::Forbidden)
    ));
    let mut denied = allow(2);
    denied.allowed = false;
    s.configure_federation_peer("remote.example", denied)
        .unwrap();
    assert!(matches!(
        s.federation_ping(b"{}", &signed_ping(&new, 1002, 4), 1002),
        Err(StoreError::Forbidden)
    ));
}

#[test]
fn real_https_discovery_and_authenticated_ping_enforce_route_boundaries() {
    use axum::http::{Method, Request};
    let _network = crate::egress::tests::NETWORK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let (mut store, key) = trusted(&dir.path().join("s.db"));
    let now = crate::enrollment::now().unwrap();
    // Refresh the synthetic peer at real wall time for the actual HTTP handler.
    let (c, p) = store
        .begin_federation_refresh("remote.example", now)
        .unwrap();
    store
        .finish_federation_refresh(c.revision, &p, Some(discovery(&key)), now)
        .unwrap();
    let token = crate::auth::AdminToken::load_or_create(&dir.path().join("token")).unwrap();
    let fixture = crate::egress::tests::Fixture::new(crate::router(store, token));
    let response = fixture
        .federation(
            Request::get(fixture.uri("chat.example", sigil_protocol::federation::DISCOVERY_PATH))
                .body(&[][..])
                .unwrap(),
        )
        .unwrap();
    assert_eq!(response.status, 200);
    let d: Discovery = serde_json::from_slice(&response.body).unwrap();
    auth::validate_discovery(&d, "chat.example", now).unwrap();
    assert!(!std::str::from_utf8(&response.body)
        .unwrap()
        .contains("seed"));
    for method in [Method::GET, Method::POST] {
        let request = Request::builder()
            .method(method)
            .uri(fixture.uri("chat.example", "/admin/v0/federation"))
            .body(&[][..])
            .unwrap();
        let expected = if request.method() == Method::GET {
            401
        } else {
            405
        };
        assert_eq!(fixture.federation(request).unwrap().status, expected);
    }
    let send = |path: &str, method: Method, nonce: u8, origin: bool| {
        let mut request = Request::builder()
            .method(method)
            .uri(fixture.uri("chat.example", path))
            .body(b"{}".as_slice())
            .unwrap();
        *request.headers_mut() = signed_ping(&key, now, nonce);
        if origin {
            request.headers_mut().insert(
                "sigil-destination",
                axum::http::HeaderValue::from_static("wrong.example"),
            );
        }
        fixture.federation(request).unwrap()
    };
    assert_eq!(
        send(
            sigil_protocol::federation::PING_PATH,
            Method::POST,
            1,
            false
        )
        .status,
        200
    );
    assert_eq!(
        send(
            sigil_protocol::federation::PING_PATH,
            Method::POST,
            1,
            false
        )
        .status,
        409
    );
    assert_eq!(
        send("/federation/v0/ping?x=1", Method::POST, 2, false).status,
        422
    );
    assert_eq!(
        send(sigil_protocol::federation::PING_PATH, Method::POST, 2, true).status,
        403
    );
    assert_eq!(
        send(
            sigil_protocol::federation::PING_PATH,
            Method::POST,
            2,
            false
        )
        .status,
        200
    );
    let request = Request::get(fixture.uri("chat.example", sigil_protocol::federation::PING_PATH))
        .body(&[][..])
        .unwrap();
    assert_eq!(fixture.federation(request).unwrap().status, 405);
    let oversized = vec![0; sigil_protocol::federation::MAX_BODY + 1];
    assert!(matches!(
        fixture.federation(
            Request::post(fixture.uri("chat.example", sigil_protocol::federation::PING_PATH))
                .body(oversized.as_slice())
                .unwrap()
        ),
        Err(crate::egress::Error::Limit)
    ));
}
