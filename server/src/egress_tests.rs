use super::*;
use axum::{routing::post, Router};
use std::{
    net::TcpListener,
    sync::{Condvar, Mutex},
    time::Instant,
};
use tokio_rustls::{
    rustls::{
        pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
        ServerConfig,
    },
    TlsAcceptor,
};
const CA: &[u8] = include_bytes!("../../client/tests/fixtures/synthetic-ca.der");
const CERT: &[u8] = include_bytes!("../../client/tests/fixtures/synthetic-server.der");
const KEY: &[u8] = include_bytes!("../../client/tests/fixtures/synthetic-server-key.der");
pub(crate) static NETWORK: Mutex<()> = Mutex::new(());
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
pub(crate) struct Fixture {
    ca: &'static [u8],
    pub(super) address: SocketAddr,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Fixture {
    pub(crate) fn new(router: Router) -> Self {
        Self::start(router, CERT, KEY, CA)
    }
    pub(crate) fn local(router: Router) -> Self {
        Self::start(
            router,
            include_bytes!("../tests/fixtures/provider-server.der"),
            include_bytes!("../tests/fixtures/provider-key.der"),
            include_bytes!("../tests/fixtures/provider-ca.der"),
        )
    }
    fn start(router: Router, cert: &'static [u8], key: &'static [u8], ca: &'static [u8]) -> Self {
        let socket = TcpListener::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let address = socket.local_addr().unwrap();
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
                axum::serve(listener, router)
                    .with_graceful_shutdown(async {
                        let _ = stopped.await;
                    })
                    .await
                    .unwrap();
            });
        });
        Self {
            ca,
            address,
            stop: Some(stop),
            worker: Some(worker),
        }
    }
    pub(super) fn policy(&self, host: &str) -> Policy {
        Policy::new(vec![self.exception(host)]).unwrap()
    }
    pub(crate) fn exception(&self, host: &str) -> Exception {
        Exception {
            host: host.into(),
            port: self.address.port(),
            networks: vec!["127.0.0.1/32".into()],
            root_ca: Some(self.ca.to_vec()),
        }
    }
    pub(crate) fn uri(&self, host: &str, path: &str) -> String {
        format!("https://{host}:{}{path}", self.address.port())
    }
    pub(crate) fn send(&self, request: Request<&[u8]>) -> Result<Response, Error> {
        let address = self.address;
        self.policy("chat.example")
            .send_with(request, move |_, _| Ok(vec![address]))
    }
    pub(crate) fn federation(&self, request: Request<&[u8]>) -> Result<Response, Error> {
        let address = self.address;
        self.policy("chat.example")
            .federation_with(request, move |_, _| Ok(vec![address]))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.stop.take().unwrap().send(());
        self.worker.take().unwrap().join().unwrap();
    }
}
fn request(uri: &str) -> Request<&'static [u8]> {
    Request::post(uri).body(b"synthetic".as_slice()).unwrap()
}
#[test]
fn urls_and_all_resolved_addresses_obey_exact_operator_exceptions() {
    for value in [
        "http://push.example/p",
        "https://user:pass@push.example/p",
        "https://push.example/p#secret",
        "https://localhost/p",
        "https://UPPER.example/p",
        "https://push.example:0/p",
        "https://[fe80::1%25lo]/p",
    ] {
        assert!(endpoint(value).is_err(), "{value}");
    }
    assert!(endpoint("https://push.example/path@value?token=a%40b").is_ok());
    let policy = Policy::default();
    let uri = endpoint("https://push.example/p").unwrap();
    for ip in [
        "0.0.0.0",
        "10.0.0.1",
        "100.64.0.1",
        "127.0.0.1",
        "169.254.169.254",
        "172.31.255.255",
        "192.0.0.9",
        "192.0.2.1",
        "192.88.99.1",
        "192.168.1.1",
        "198.18.0.1",
        "198.51.100.1",
        "203.0.113.1",
        "224.0.0.1",
        "255.255.255.255",
        "::",
        "::1",
        "::ffff:127.0.0.1",
        "64:ff9b::7f00:1",
        "2001::1",
        "2001:db8::1",
        "2002:7f00:1::",
        "3fff::1",
        "fc00::1",
        "fe80::1",
        "ff02::1",
    ] {
        let address = SocketAddr::new(ip.parse().unwrap(), 443);
        assert_eq!(policy.check(&uri, &[address]), Err(Error::Policy), "{ip}");
    }
    let public: SocketAddr = "8.8.8.8:443".parse().unwrap();
    assert!(policy.check(&uri, &[public]).is_ok());
    assert!(policy
        .check(&uri, &["[2606:4700:4700::1111]:443".parse().unwrap()])
        .is_ok());
    assert_eq!(
        policy.check(&uri, &[public, "127.0.0.1:443".parse().unwrap()]),
        Err(Error::Policy)
    );
    assert_eq!(policy.check(&uri, &vec![public; 17]), Err(Error::Policy));
    let rule = Exception {
        host: "push.example".into(),
        port: 8443,
        networks: vec!["192.168.1.0/24".into()],
        root_ca: None,
    };
    let private = Policy::new(vec![rule.clone()]).unwrap();
    let uri = endpoint("https://push.example:8443/p").unwrap();
    assert!(private
        .check(&uri, &["192.168.1.10:8443".parse().unwrap()])
        .is_ok());
    for address in ["192.168.2.10:8443", "192.168.1.10:443", "8.8.8.8:8443"] {
        assert_eq!(
            private.check(&uri, &[address.parse().unwrap()]),
            Err(Error::Policy)
        );
    }
    for cidr in [
        "192.168.1.1/24",
        "192.168.1.0/024",
        "192.168.1.0/33",
        "::1/64",
        "::/129",
        "not-network",
    ] {
        assert!(Policy::new(vec![Exception {
            networks: vec![cidr.into()],
            ..rule.clone()
        }])
        .is_err());
    }
    assert!(Policy::new(vec![rule.clone(), rule]).is_err());
}
#[test]
fn oidc_private_addresses_follow_only_the_configured_origin() {
    let uri = endpoint("https://idp.example:9443/token").unwrap();
    let policy = Policy::oidc("https://idp.example:9443/tenant", vec![]).unwrap();
    for ip in [
        "10.0.0.2",
        "172.20.0.4",
        "172.20.0.7",
        "192.168.1.2",
        "fd00::2",
        "8.8.8.8",
    ] {
        let address = SocketAddr::new(ip.parse().unwrap(), 9443);
        assert!(policy.check(&uri, &[address]).is_ok(), "{ip}");
        assert_eq!(
            Policy::default().check(&uri, &[address]),
            Err(Error::Policy)
        );
    }
    for ip in [
        "127.0.0.1",
        "0.0.0.0",
        "169.254.169.254",
        "100.100.100.200",
        "224.0.0.1",
        "::1",
        "::",
        "fe80::1",
        "ff02::1",
        "::ffff:172.20.0.4",
    ] {
        let address = SocketAddr::new(ip.parse().unwrap(), 9443);
        assert_eq!(policy.check(&uri, &[address]), Err(Error::Policy), "{ip}");
        assert_eq!(
            policy.check(&uri, &["172.20.0.4:9443".parse().unwrap(), address]),
            Err(Error::Policy),
            "mixed DNS: {ip}"
        );
    }
    for url in [
        "https://other.example:9443/token",
        "https://idp.example/token",
        "https://idp.example.evil.test:9443/token",
    ] {
        let other = endpoint(url).unwrap();
        let address = SocketAddr::new(
            "172.20.0.4".parse().unwrap(),
            other.port_u16().unwrap_or(443),
        );
        assert_eq!(
            policy.check(&other, &[address]),
            Err(Error::Policy),
            "{url}"
        );
    }
    assert_eq!(
        policy.check(&uri, &["172.20.0.4:443".parse().unwrap()]),
        Err(Error::Policy)
    );
    let pinned = Policy::oidc(
        "https://idp.example:9443",
        vec![Exception {
            host: "idp.example".into(),
            port: 9443,
            networks: vec!["172.20.0.4/32".into()],
            root_ca: None,
        }],
    )
    .unwrap();
    assert_eq!(
        pinned.check(&uri, &["172.20.0.7:9443".parse().unwrap()]),
        Err(Error::Policy)
    );
}
#[test]
fn real_tls_revalidates_dns_rejects_wrong_names_and_never_follows_redirects() {
    let _serial = NETWORK.lock().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let fixture = Fixture::new(
        Router::new()
            .route(
                "/push",
                post(move || {
                    let count = count.clone();
                    async move {
                        count.fetch_add(1, Ordering::SeqCst);
                        ([(header::RETRY_AFTER, "120")], "accepted")
                    }
                }),
            )
            .route(
                "/redirect",
                post(|| async {
                    (
                        axum::http::StatusCode::FOUND,
                        [(header::LOCATION, "https://127.0.0.1/private")],
                        "",
                    )
                }),
            )
            .route("/large", post(|| async { vec![0; LIMIT + 1] }))
            .route(
                "/compressed",
                post(|| async { ([(header::CONTENT_ENCODING, "gzip")], "not decoded") }),
            ),
    );
    let policy = fixture.policy("chat.example");
    let address = fixture.address;
    let result = policy
        .send_with(
            request(&fixture.uri("chat.example", "/push")),
            move |_, _| Ok(vec![address]),
        )
        .unwrap();
    assert_eq!(result.status, 200);
    assert_eq!(result.body.as_slice(), b"accepted");
    assert_eq!(result.retry_after.as_deref(), Some("120"));
    assert!(matches!(
        policy.send_with(
            request(&fixture.uri("chat.example", "/push")),
            move |_, port| Ok(vec![SocketAddr::new(
                "169.254.169.254".parse().unwrap(),
                port
            )])
        ),
        Err(Error::Policy)
    ));
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert!(matches!(
        fixture.policy("other.example").send_with(
            request(&fixture.uri("other.example", "/push")),
            move |_, _| Ok(vec![address])
        ),
        Err(Error::Transport)
    ));
    let response = policy
        .send_with(
            request(&fixture.uri("chat.example", "/redirect")),
            move |_, _| Ok(vec![address]),
        )
        .unwrap();
    assert_eq!(response.status, 302);
    assert!(matches!(
        policy.send_with(
            request(&fixture.uri("chat.example", "/large")),
            move |_, _| Ok(vec![address])
        ),
        Err(Error::Limit)
    ));
    assert!(matches!(
        policy.send_with(
            request(&fixture.uri("chat.example", "/compressed")),
            move |_, _| Ok(vec![address])
        ),
        Err(Error::InvalidResponse)
    ));
    assert!(matches!(
        Policy::default().send(request(&fixture.uri("127.0.0.1", "/push"))),
        Err(Error::Policy)
    ));
}
#[test]
fn timed_out_dns_jobs_keep_their_capacity_until_the_actual_lookup_finishes() {
    let _serial = NETWORK.lock().unwrap();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let started = Arc::new(AtomicUsize::new(0));
    let mut workers = Vec::new();
    for _ in 0..4 {
        let (gate, started) = (gate.clone(), started.clone());
        workers.push(std::thread::spawn(move || {
            Policy::default().send_with(request("https://push.example/p"), move |_, _| {
                started.fetch_add(1, Ordering::SeqCst);
                let (lock, ready) = &*gate;
                let mut released = lock.lock().unwrap();
                while !*released {
                    released = ready.wait(released).unwrap();
                }
                Ok(vec!["8.8.8.8:443".parse().unwrap()])
            })
        }));
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while started.load(Ordering::SeqCst) != 4 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    for worker in workers {
        assert!(matches!(worker.join().unwrap(), Err(Error::Transport)));
    }
    assert_eq!(DNS_JOBS.load(Ordering::Acquire), 4);
    let called = Arc::new(AtomicBool::new(false));
    let flag = called.clone();
    assert!(matches!(
        Policy::default().send_with(request("https://push.example/p"), move |_, _| {
            flag.store(true, Ordering::Release);
            Ok(vec![])
        }),
        Err(Error::Transport)
    ));
    assert!(!called.load(Ordering::Acquire));
    let (lock, ready) = &*gate;
    *lock.lock().unwrap() = true;
    ready.notify_all();
    let deadline = Instant::now() + Duration::from_secs(2);
    while DNS_JOBS.load(Ordering::Acquire) != 0 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn only_lookup_responses_receive_the_bounded_chunk_body_allowance() {
    let _serial = NETWORK.lock().unwrap();
    let size = Arc::new(std::sync::atomic::AtomicUsize::new(LIMIT + 1));
    let read = size.clone();
    let fixture = Fixture::new(Router::new().fallback(post(move || {
        let length = read.load(Ordering::Relaxed);
        async move { vec![0u8; length] }
    })));
    let call = |path| {
        fixture.federation(
            Request::post(fixture.uri("chat.example", path))
                .body(b"{}".as_slice())
                .unwrap(),
        )
    };
    assert_eq!(
        call(sigil_protocol::federation::LOOKUP_PATH)
            .unwrap()
            .body
            .len(),
        LIMIT + 1
    );
    assert!(matches!(
        call(sigil_protocol::federation::PING_PATH),
        Err(Error::Limit)
    ));
    size.store(
        sigil_protocol::federation::MAX_LOOKUP_RESPONSE + 8192 + 1,
        Ordering::Relaxed,
    );
    assert!(matches!(
        call(sigil_protocol::federation::LOOKUP_PATH),
        Err(Error::Limit)
    ));
}
