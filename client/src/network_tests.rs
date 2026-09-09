use super::*;
use axum::{body::Body as AxumBody, http::StatusCode, routing::get, Router};
use std::{
    net::{SocketAddr, TcpListener},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tokio_rustls::{
    rustls::{
        pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
        ServerConfig,
    },
    TlsAcceptor,
};
use ureq::unversioned::{
    resolver::{ResolvedSocketAddrs, Resolver},
    transport::{DefaultConnector, NextTimeout},
};
pub(crate) const CA: &[u8] = include_bytes!("../tests/fixtures/synthetic-ca.der");
const CERT: &[u8] = include_bytes!("../tests/fixtures/synthetic-server.der");
const KEY: &[u8] = include_bytes!("../tests/fixtures/synthetic-server-key.der");
fn token() -> String {
    "ab".repeat(32)
}

struct Listener {
    socket: tokio::net::TcpListener,
    tls: TlsAcceptor,
}
impl axum::serve::Listener for Listener {
    type Io = tokio_rustls::server::TlsStream<tokio::net::TcpStream>;
    type Addr = SocketAddr;
    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let (socket, address) = self.socket.accept().await.unwrap();
            if let Ok(Ok(stream)) =
                tokio::time::timeout(Duration::from_secs(3), self.tls.accept(socket)).await
            {
                return (stream, address);
            }
        }
    }
    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.socket.local_addr()
    }
}
#[derive(Debug)]
struct LocalResolver;
static ENDPOINTS: std::sync::Mutex<std::collections::BTreeMap<u16, SocketAddr>> =
    std::sync::Mutex::new(std::collections::BTreeMap::new());
pub(crate) fn agent(config: ureq::config::Config) -> Agent {
    Agent::with_parts(config, DefaultConnector::default(), LocalResolver)
}
impl Resolver for LocalResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        _: &ureq::config::Config,
        _: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        if !matches!(
            uri.host(),
            Some("chat.example" | "other.example" | "federated.example")
        ) {
            return Err(ureq::Error::HostNotFound);
        }
        let address = *ENDPOINTS
            .lock()
            .unwrap()
            .get(&uri.port_u16().ok_or(ureq::Error::HostNotFound)?)
            .ok_or(ureq::Error::HostNotFound)?;
        let mut values = self.empty();
        values.push(address);
        Ok(values)
    }
}
pub(crate) struct Fixture {
    address: SocketAddr,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Fixture {
    pub(crate) fn local_provider(app: Router) -> Self {
        Self::start(
            app,
            None,
            include_bytes!("../../server/tests/fixtures/provider-server.der"),
            include_bytes!("../../server/tests/fixtures/provider-key.der"),
            0,
        )
    }
    pub(crate) fn new(app: Router) -> Self {
        Self::start(app, None, CERT, KEY, 0)
    }
    pub(crate) fn maintained_at(
        app: Router,
        maintenance: impl std::future::Future<Output = ()> + Send + 'static,
        port: u16,
    ) -> Self {
        Self::start(app, Some(Box::pin(maintenance)), CERT, KEY, port)
    }
    pub(crate) fn federation(
        app: Router,
        maintenance: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> Self {
        Self::federation_at(app, maintenance, 0)
    }
    pub(crate) fn federation_at(
        app: Router,
        maintenance: impl std::future::Future<Output = ()> + Send + 'static,
        port: u16,
    ) -> Self {
        Self::start(
            app,
            Some(Box::pin(maintenance)),
            include_bytes!("../tests/fixtures/federation-server.der"),
            include_bytes!("../tests/fixtures/federation-server-key.der"),
            port,
        )
    }
    fn start(
        app: Router,
        maintenance: Option<std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>>,
        cert: &'static [u8],
        key: &'static [u8],
        port: u16,
    ) -> Self {
        let socket = TcpListener::bind(("127.0.0.1", port)).unwrap();
        socket.set_nonblocking(true).unwrap();
        let address = socket.local_addr().unwrap();
        ENDPOINTS.lock().unwrap().insert(address.port(), address);
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let worker = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let config = ServerConfig::builder_with_provider(Arc::new(
                    tokio_rustls::rustls::crypto::ring::default_provider(),
                ))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(
                    vec![CertificateDer::from(cert.to_vec())],
                    PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.to_vec())),
                )
                .unwrap();
                let listener = Listener {
                    socket: tokio::net::TcpListener::from_std(socket).unwrap(),
                    tls: TlsAcceptor::from(Arc::new(config)),
                };
                if let Some(maintenance) = maintenance {
                    tokio::spawn(maintenance);
                }
                axum::serve(listener, app)
                    .with_graceful_shutdown(async {
                        let _ = stopped.await;
                    })
                    .await
                    .unwrap();
            });
        });
        Self {
            address,
            stop: Some(stop),
            worker: Some(worker),
        }
    }
    fn client(&self, server: &str, credential: &str, roots: &[Vec<u8>]) -> HttpsClient {
        HttpsClient::new(server, self.address.port(), credential, roots).unwrap()
    }
    pub(crate) fn port(&self) -> u16 {
        self.address.port()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
        ENDPOINTS.lock().unwrap().remove(&self.address.port());
    }
}
fn session_json() -> serde_json::Value {
    serde_json::json!({"account_id":"01".repeat(32),"address":"@alice:chat.example","device_id":"02".repeat(32),"device_label":"Synthetic","expires_at":2000000000_u64})
}

#[test]
fn certificates_and_names_are_verified_before_http_credentials_are_sent() {
    let requests = Arc::new(AtomicUsize::new(0));
    let observed = requests.clone();
    let app = Router::new().route(
        "/client/v0/session",
        get(move |headers: axum::http::HeaderMap| {
            let observed = observed.clone();
            async move {
                observed.fetch_add(1, Ordering::SeqCst);
                assert_eq!(
                    headers[header::AUTHORIZATION],
                    format!("Bearer {}", token())
                );
                axum::Json(session_json())
            }
        }),
    );
    let fixture = Fixture::new(app);
    for (name, roots) in [
        ("chat.example", Vec::new()),
        ("other.example", vec![CA.to_vec()]),
    ] {
        let client = fixture.client(name, &token(), &roots);
        assert_eq!(client.session(), Err(Error::Transport));
    }
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    let client = fixture.client("chat.example", &token(), &[CA.to_vec()]);
    assert_eq!(client.session().unwrap().address, "@alice:chat.example");
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    for name in [
        "http://chat.example",
        "chat.example/path",
        "alice@chat.example",
        "127.0.0.1",
        "CHAT.example",
        "chat.example:443",
        "chat.example?query",
    ] {
        assert!(matches!(
            HttpsClient::new(name, 443, &token(), &[]),
            Err(Error::Configuration)
        ));
    }
}

#[test]
fn redirects_and_rate_errors_are_returned_without_following_or_echoing_bodies() {
    for (status, retry, expected) in [
        (302, "1", Some(1)),
        (429, "5", Some(5)),
        (503, "999999999999999999999999999999", None),
        (429, "-1", None),
    ] {
        let app = Router::new().route(
            "/client/v0/session",
            get(move || async move {
                Response::builder()
                    .status(status)
                    .header(header::LOCATION, "https://other.example/steal")
                    .header(header::RETRY_AFTER, retry)
                    .body(AxumBody::from("untrusted error text must not be surfaced"))
                    .unwrap()
            }),
        );
        let fixture = Fixture::new(app);
        let client = fixture.client("chat.example", &token(), &[CA.to_vec()]);
        let error = client.session().unwrap_err();
        assert_eq!(
            error,
            Error::Status {
                code: status,
                retry_after_seconds: expected
            }
        );
        assert!(!format!("{error:?}").contains("untrusted"));
    }
}

#[test]
fn response_framing_and_semantics_are_bounded_before_use() {
    let valid = session_json().to_string();
    let mut bad_domain = session_json();
    bad_domain["address"] = "@alice:other.example".into();
    let mut bad_id = session_json();
    bad_id["device_id"] = "bad".into();
    for (status, kind, encoding, body) in [
        (201, "application/json", "identity", valid.clone()),
        (200, "text/html", "identity", valid.clone()),
        (200, "application/json", "gzip", valid.clone()),
        (200, "application/json", "identity", bad_domain.to_string()),
        (200, "application/json", "identity", bad_id.to_string()),
        (200, "application/json", "identity", "x".repeat(SMALL + 1)),
        (
            200,
            "application/json",
            "identity",
            format!("{valid} trailing"),
        ),
    ] {
        let app = Router::new().route(
            "/client/v0/session",
            get(move || {
                let body = body.clone();
                async move {
                    Response::builder()
                        .status(status)
                        .header(header::CONTENT_TYPE, kind)
                        .header(header::CONTENT_ENCODING, encoding)
                        .body(AxumBody::from(body))
                        .unwrap()
                }
            }),
        );
        let fixture = Fixture::new(app);
        let client = fixture.client("chat.example", &token(), &[CA.to_vec()]);
        assert!(matches!(
            client.session(),
            Err(Error::InvalidResponse | Error::Limit)
        ));
    }
    let app = Router::new().route(
        "/client/v0/session",
        get(|| async {
            Response::builder()
                .header(header::CONTENT_TYPE, "application/json")
                .header("x-oversized", "x".repeat(SMALL + 1))
                .body(AxumBody::from(session_json().to_string()))
                .unwrap()
        }),
    );
    let fixture = Fixture::new(app);
    assert_eq!(
        fixture
            .client("chat.example", &token(), &[CA.to_vec()])
            .session(),
        Err(Error::Transport)
    );
}

#[test]
fn receipt_and_mailbox_validation_rejects_mismatches() {
    let app = Router::new().route("/client/v0/messages", axum::routing::post(|| async {
        (StatusCode::ACCEPTED, axum::Json(serde_json::json!({"sequence":1,"expires_at":1001})))
    })).route("/client/v0/mailbox", get(|| async {
        let entry = serde_json::json!({"sequence":1,"sender_device":"01".repeat(32),"message_id":"02".repeat(32),"payload":"ab".repeat(32),"expires_at":1000});
        axum::Json(vec![entry.clone(), entry])
    }));
    let fixture = Fixture::new(app);
    let client = fixture.client("chat.example", &token(), &[CA.to_vec()]);
    let request = mailbox::Submit {
        recipient_device: "01".repeat(32),
        message_id: "02".repeat(32),
        payload: "ab".repeat(32),
        expires_at: 1000,
    };
    assert_eq!(client.submit(&request), Err(Error::InvalidResponse));
    assert!(matches!(client.mailbox(), Err(Error::InvalidResponse)));
    assert!(matches!(
        client.claim_prekey("../session", &token()),
        Err(Error::Configuration)
    ));
}

#[test]
fn actual_server_enrollment_delivery_and_recovery_work_over_tls() {
    use sigil_server::{auth::AdminToken, store::Store};
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("server.db")).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    store
        .configure(sigil_protocol::Configure {
            expected_revision: 0,
            settings: sigil_protocol::Settings {
                server_name: "chat.example".into(),
                default_quota_bytes: sigil_protocol::DEFAULT_QUOTA,
                max_attachment_bytes: sigil_protocol::DEFAULT_ATTACHMENT_LIMIT,
            },
        })
        .unwrap();
    let invite = store
        .invite(
            accounts::InviteRequest {
                username: "alice".into(),
                expires_in_seconds: 60,
            },
            now,
        )
        .unwrap();
    let fixture = Fixture::new(sigil_server::router(
        store,
        AdminToken::load_or_create(&dir.path().join("admin.token")).unwrap(),
    ));
    let client = fixture.client("chat.example", &token(), &[CA.to_vec()]);
    let session = client.enroll(&invite.secret, "Synthetic", false).unwrap();
    assert!(matches!(
        client.enroll(&invite.secret, "Synthetic", false),
        Err(Error::Status { code: 401, .. })
    ));
    assert_eq!(client.session().unwrap(), session);
    let request = mailbox::Submit {
        recipient_device: session.device_id,
        message_id: "03".repeat(32),
        payload: "ab".repeat(32),
        expires_at: now + 60,
    };
    let receipt = client.submit(&request).unwrap();
    assert_eq!(client.submit(&request).unwrap(), receipt);
    let deliveries = client.mailbox().unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].payload, request.payload);
    client.acknowledge_delivery(receipt.sequence).unwrap();
    assert!(client.mailbox().unwrap().is_empty());
    let key = sigil_crypto::recovery::RecoveryKey::generate([5; 32]).unwrap();
    let (head, object) = key.seal_manifest(None, now, &[]).unwrap();
    client.upload_recovery_object(&object).unwrap();
    assert_eq!(
        client
            .download_recovery_object(object.id())
            .unwrap()
            .bytes(),
        object.bytes()
    );
    let published = client
        .publish_recovery_head(&recovery::PublishHead {
            expected_generation: 0,
            expected_manifest: None,
            manifest: super::super::transport::hex(&head.manifest),
            acknowledge_restored_checkpoint: false,
            restore_generation: None,
        })
        .unwrap();
    assert_eq!(client.recovery_head().unwrap(), published);
    assert_eq!(published.generation, 1);
}

#[test]
fn dns_timeouts_retain_worker_slots_until_the_lookup_finishes() {
    let barrier = Arc::new(std::sync::Barrier::new(5));
    let timeout = || NextTimeout {
        after: ureq::unversioned::transport::time::Duration::from_millis(5),
        reason: ureq::Timeout::Resolve,
    };
    for _ in 0..4 {
        let barrier = barrier.clone();
        assert!(matches!(
            lookup(timeout(), move || {
                barrier.wait();
                Ok(Vec::new())
            }),
            Err(ureq::Error::Timeout(_))
        ));
    }
    assert_eq!(DNS_JOBS.load(Ordering::Acquire), 4);
    assert!(
        matches!(lookup(timeout(), || panic!("bounded lookup must not start")), Err(ureq::Error::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock)
    );
    barrier.wait();
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while DNS_JOBS.load(Ordering::Acquire) != 0 && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(DNS_JOBS.load(Ordering::Acquire), 0);
}

#[test]
fn revocation_requires_exact_success_and_does_not_hide_authorization_failure() {
    for status in [
        StatusCode::NO_CONTENT,
        StatusCode::OK,
        StatusCode::ACCEPTED,
        StatusCode::UNAUTHORIZED,
        StatusCode::NOT_FOUND,
    ] {
        let fixture = Fixture::new(Router::new().route(
            "/client/v0/devices/{id}",
            axum::routing::delete(move || async move { status }),
        ));
        let client =
            HttpsClient::new("chat.example", fixture.port(), &token(), &[CA.to_vec()]).unwrap();
        let result = client.revoke_device(&"02".repeat(32));
        match status {
            StatusCode::NO_CONTENT => assert_eq!(result, Ok(())),
            StatusCode::OK | StatusCode::ACCEPTED => {
                assert_eq!(result, Err(Error::InvalidResponse))
            }
            _ => assert!(
                matches!(result, Err(Error::Status { code, .. }) if code == status.as_u16())
            ),
        }
    }
}

#[test]
fn delegation_resolves_without_disclosing_credentials_and_keeps_identity_domain() {
    let target = Fixture::new(Router::new().route(
        "/client/v0/session",
        get(|| async { (StatusCode::UNAUTHORIZED, "synthetic") }),
    ));
    let origin = format!("https://chat.example:{}", target.port());
    let authority = Fixture::new(Router::new().route(
        sigil_protocol::discovery::PATH,
        get(move |headers: axum::http::HeaderMap| {
            let origin = origin.clone();
            async move {
                assert!(!headers.contains_key(header::AUTHORIZATION));
                axum::Json(sigil_protocol::discovery::Discovery {
                    server_name: "chat.example".into(),
                    api_origin: origin,
                })
            }
        }),
    ));
    let client =
        HttpsClient::discover("chat.example", authority.port(), &token(), &[CA.to_vec()]).unwrap();
    assert_eq!(client.server, "chat.example");
    assert_eq!(
        client.api_origin().unwrap(),
        format!("https://chat.example:{}", target.port())
    );
    assert!(matches!(
        client.request(Method::GET, "/client/v0/session", None::<&()>),
        Err(Error::Status { code: 401, .. })
    ));
    let wrong = Fixture::new(Router::new().route(
        sigil_protocol::discovery::PATH,
        get(|| async {
            axum::Json(sigil_protocol::discovery::Discovery {
                server_name: "other.example".into(),
                api_origin: "https://chat.example".into(),
            })
        }),
    ));
    assert!(matches!(
        HttpsClient::discover("chat.example", wrong.port(), &token(), &[CA.to_vec()])
            .unwrap()
            .api_origin(),
        Err(Error::InvalidResponse)
    ));
    let absent = Fixture::new(Router::new());
    assert!(
        HttpsClient::discover("chat.example", absent.port(), &token(), &[CA.to_vec()])
            .unwrap()
            .api_origin()
            .is_ok()
    );
}

#[test]
fn login_discovery_uses_delegated_methods_and_rejects_wrong_identity() {
    let target = Fixture::new(Router::new().route(
        "/client/v0/login",
        get(|| async {
            axum::Json(sigil_protocol::login::Methods {
                server_name: "chat.example".into(),
                sso: true,
                password: false,
                invitation: false,
            })
        }),
    ));
    let origin = format!("https://chat.example:{}", target.port());
    let authority = Fixture::new(Router::new().route(
        sigil_protocol::discovery::PATH,
        get(move |headers: axum::http::HeaderMap| {
            let origin = origin.clone();
            async move {
                assert!(!headers.contains_key(header::AUTHORIZATION));
                axum::Json(sigil_protocol::discovery::Discovery {
                    server_name: "chat.example".into(),
                    api_origin: origin,
                })
            }
        }),
    ));
    let methods =
        HttpsClient::login_methods("chat.example", authority.port(), &[CA.to_vec()]).unwrap();
    assert!(methods.sso && !methods.password);
    let wrong = Fixture::new(Router::new().route(
        sigil_protocol::discovery::PATH,
        get(|| async {
            axum::Json(sigil_protocol::discovery::Discovery {
                server_name: "other.example".into(),
                api_origin: "https://attacker.example".into(),
            })
        }),
    ));
    assert!(HttpsClient::login_methods("chat.example", wrong.port(), &[CA.to_vec()]).is_err());
}
