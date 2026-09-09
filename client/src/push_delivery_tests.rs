use super::*;
use crate::{
    claims::tests::pair,
    incoming::tests::{start, trust},
};
use axum::{
    body::Bytes,
    http::{HeaderMap, StatusCode},
    routing::post,
    Router,
};
use sigil_server::{
    auth::AdminToken,
    push_config::{Configure, FcmUpdate},
    store::Store,
};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc,
    },
    time::Duration,
};

fn maintained(dir: &Path, port: u16) -> crate::network::tests::Fixture {
    let (app, maintenance) = sigil_server::router_with_maintenance(
        Store::open(&dir.join("server.db")).unwrap(),
        AdminToken::load_or_create(&dir.join("admin.token")).unwrap(),
    );
    crate::network::tests::Fixture::maintained_at(app, maintenance, port)
}

#[test]
fn background_delivery_retries_after_restart_and_wakes_authenticated_decryption() {
    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    let (sent, received) = mpsc::channel();
    let (failed, failure) = mpsc::channel();
    let provider = crate::network::tests::Fixture::local_provider(Router::new().route(
        "/push",
        post(move |headers: HeaderMap, bytes: Bytes| {
            let seen = seen.clone();
            let sent = sent.clone();
            let failed = failed.clone();
            async move {
                assert_eq!(headers["content-encoding"], "aes128gcm");
                assert!(headers["authorization"]
                    .to_str()
                    .unwrap()
                    .starts_with("vapid t="));
                assert!(!bytes.windows(9).any(|w| w == b"synthetic"));
                if seen.fetch_add(1, Ordering::SeqCst) == 0 {
                    failed.send(()).unwrap();
                    (StatusCode::SERVICE_UNAVAILABLE, [("retry-after", "2")])
                } else {
                    sent.send((headers, bytes.to_vec())).unwrap();
                    (StatusCode::CREATED, [("retry-after", "0")])
                }
            }
        }),
    ));
    let (dir, fixture, mut alice, mut bob, _) = pair();
    let (_, peer) = trust(&mut alice, &mut bob);
    let port = fixture.port();
    let mut server = Store::open(&dir.path().join("server.db")).unwrap();
    let configured = server
        .configure_push(Configure {
            expected_revision: 0,
            unified_push: true,
            contact: Some("mailto:operator@example.com".into()),
            exceptions: vec![sigil_server::egress::Exception {
                host: "127.0.0.1".into(),
                port: provider.port(),
                networks: vec!["127.0.0.1/32".into()],
                root_ca: Some(
                    include_bytes!("../../server/tests/fixtures/provider-ca.der").to_vec(),
                ),
            }],
            rotate_vapid: false,
            fcm: FcmUpdate::Disable,
        })
        .unwrap();
    drop(server);
    drop(fixture);
    let fixture = maintained(dir.path(), port);
    fn mobile(client: &mut ClientStore, value: serde_json::Value) -> serde_json::Value {
        let result: serde_json::Value =
            serde_json::from_str(&client.mobile_command(&value.to_string())).unwrap();
        assert_eq!(result["ok"], true, "{result}");
        result["value"].clone()
    }
    let registration = mobile(
        &mut bob,
        serde_json::json!({"command":"push","action":"prepare"}),
    );
    assert_eq!(registration["vapid"], configured.vapid_public_key.unwrap());
    assert_eq!(registration["pending"], true);
    let connector = bob.unified_push_registration().unwrap().unwrap();
    mobile(
        &mut bob,
        serde_json::json!({"command":"push","action":"endpoint","connection":connector.connection,"endpoint":format!("https://127.0.0.1:{}/push", provider.port())}),
    );
    let now = crate::schedule::clock().unwrap();
    assert!(matches!(bob.push_step(now).unwrap(), Progress::Reconciled));
    assert!(
        matches!(bob.push_step(now).unwrap(), Progress::Updated(s) if s.state == RemoteState::Pending)
    );
    failure.recv_timeout(Duration::from_secs(15)).unwrap();
    let db = Connection::open(dir.path().join("server.db")).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let retry: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM push_jobs WHERE attempts=1 AND lease IS NULL AND due_at>0)", [], |r| r.get(0)).unwrap();
        if retry {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "retry was not persisted"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(fixture);
    drop(bob);
    let _fixture = maintained(dir.path(), port);
    let mut bob = ClientStore::open(
        &dir.path().join("bob.db"),
        StorageKey::new(sigil_crypto::Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap();
    let (headers, challenge) = received.recv_timeout(Duration::from_secs(15)).unwrap();
    assert!(!headers.contains_key("topic"));
    let now = crate::schedule::clock().unwrap();
    assert_eq!(
        mobile(
            &mut bob,
            serde_json::json!({"command":"push","action":"receive","connection":connector.connection,"payload":B64::encode_string(&challenge)})
        )["accepted"],
        true
    );
    let files = mobile(&mut bob, serde_json::json!({"command":"file_work"}));
    assert!(files["issue"].is_null(), "{files}");
    assert_eq!(
        mobile(
            &mut bob,
            serde_json::json!({"command":"push","action":"status"})
        )["remote"],
        "active"
    );
    let mut cache = bob
        .open_attachment_cache(&dir.path().join("cache"), 8 * 1024 * 1024)
        .unwrap();
    let work = bob.sync_backend_due_online(&mut cache);
    assert!(work.push.unwrap().scheduling_error.is_none());
    let messaging = work.messaging.unwrap();
    assert!(messaging.scheduling_error.is_none());
    assert!(messaging.step.unwrap().failure.is_none());
    start(&mut alice, peer, now);
    let (headers, wake) = received.recv_timeout(Duration::from_secs(15)).unwrap();
    assert_eq!(headers["topic"], "sigil-wake-v0");
    assert_eq!(
        mobile(
            &mut bob,
            serde_json::json!({"command":"push","action":"receive","connection":connector.connection,"payload":B64::encode_string(&wake)})
        )["accepted"],
        true
    );
    // Receipt of a hint neither imports plaintext nor acknowledges the mailbox.
    assert_eq!(bob.connected_client().unwrap().mailbox().unwrap().len(), 1);
    assert_eq!(
        bob.db
            .query_row("SELECT count(*) FROM inbox", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        let work = bob.sync_backend_due_online(&mut cache);
        let messaging = work.messaging.unwrap();
        assert!(messaging.scheduling_error.is_none());
        if let Some(step) = messaging.step {
            assert!(step.failure.is_none());
            if !step.incoming.is_empty() {
                let MailboxEvent::Text(message) =
                    step.incoming.into_iter().next().unwrap().result.unwrap()
                else {
                    panic!("expected text");
                };
                assert_eq!(message.text().unwrap().body, "synthetic initial");
                break;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "message was not fetched"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(bob
        .connected_client()
        .unwrap()
        .mailbox()
        .unwrap()
        .is_empty());
    mobile(
        &mut bob,
        serde_json::json!({"command":"push","action":"disable"}),
    );
    assert!(
        matches!(bob.sync_backend_due_online(&mut cache).push.unwrap().progress.unwrap().unwrap(), Progress::Updated(s) if s.state == RemoteState::Disabled)
    );
    assert_eq!(
        bob.receive_unified_push(
            &connector.connection,
            &wake,
            crate::schedule::clock().unwrap()
        )
        .unwrap(),
        ReceivedHint::Ignored
    );
}
