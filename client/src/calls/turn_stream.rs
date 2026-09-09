//! RFC 8656 stream framing beneath the RTC library's datagram TURN client.
use crate::{ClientStore, Error};
use rtc::ice::url::{ProtoType, SchemeType, Url};
use std::{
    fmt,
    future::Future,
    io::{self, IoSliceMut},
    net::SocketAddr,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::mpsc,
};
use tokio_rustls::{
    rustls::{
        self,
        pki_types::{CertificateDer, ServerName},
    },
    TlsConnector,
};
use webrtc::runtime::{
    AsyncInterval, AsyncTcpListener, AsyncTcpStream, AsyncUdpSocket, JoinHandle, RecvMeta, Runtime,
    TokioRuntime, Transmit,
};

const MAX_PACKET: usize = 8192;
#[derive(Clone)]
struct Route {
    host: String,
    port: u16,
    tls: bool,
    alias: SocketAddr,
}
pub(super) struct RelayRuntime {
    routes: Vec<Route>,
    tls: Arc<rustls::ClientConfig>,
}
impl fmt::Debug for RelayRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RelayRuntime")
    }
}
impl RelayRuntime {
    pub(super) fn configure(store: &ClientStore, urls: &mut [String]) -> Result<Arc<Self>, Error> {
        if urls.len() > 4 {
            return Err(Error::Limit);
        }
        let mut routes = Vec::new();
        for raw in urls {
            sigil_calls::validate_turn_url(raw).map_err(|_| Error::InvalidEvent)?;
            let url = Url::parse_url(raw).map_err(|_| Error::InvalidEvent)?;
            if url.proto == ProtoType::Tcp {
                let alias: SocketAddr = ([127, 0, 0, 1], 10000 + routes.len() as u16).into();
                *raw = format!("turn:{alias}?transport=udp");
                routes.push(Route {
                    host: url.host,
                    port: url.port,
                    tls: url.scheme == SchemeType::Turns,
                    alias,
                });
            }
        }
        let mut roots = rustls::RootCertStore::empty();
        let custom = store.connection_roots()?;
        if custom.is_empty() {
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        } else {
            for root in custom {
                roots
                    .add(CertificateDer::from(root))
                    .map_err(|_| Error::InvalidStore)?;
            }
        }
        let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|_| Error::Unprepared)?
        .with_root_certificates(roots)
        .with_no_client_auth();
        Ok(Arc::new(Self {
            routes,
            tls: Arc::new(tls),
        }))
    }
}
impl Runtime for RelayRuntime {
    fn spawn(&self, future: Pin<Box<dyn Future<Output = ()> + Send>>) -> Box<dyn JoinHandle> {
        TokioRuntime.spawn(future)
    }
    fn wrap_udp_socket(&self, socket: std::net::UdpSocket) -> io::Result<Arc<dyn AsyncUdpSocket>> {
        let udp = TokioRuntime.wrap_udp_socket(socket)?;
        if self.routes.is_empty() || udp.local_addr()?.is_ipv6() {
            return Ok(udp);
        }
        let (receive, incoming) = mpsc::channel(128);
        let mut routes = Vec::new();
        let mut tasks = Vec::new();
        for route in &self.routes {
            let (send, outgoing) = mpsc::channel(64);
            routes.push((route.alias, send));
            let route = route.clone();
            let tls = self.tls.clone();
            let receive = receive.clone();
            tasks.push(tokio::spawn(async move {
                let _ = run(route, tls, outgoing, receive).await;
            }));
        }
        Ok(Arc::new(Socket {
            udp,
            routes,
            incoming: Mutex::new(incoming),
            tasks,
        }))
    }
    fn wrap_tcp_listener(
        &self,
        listener: std::net::TcpListener,
    ) -> io::Result<Arc<dyn AsyncTcpListener>> {
        TokioRuntime.wrap_tcp_listener(listener)
    }
    fn connect_tcp<'a>(
        &'a self,
        remote: SocketAddr,
    ) -> Pin<Box<dyn Future<Output = io::Result<Arc<dyn AsyncTcpStream>>> + Send + 'a>> {
        TokioRuntime.connect_tcp(remote)
    }
    fn resolve_host<'a>(
        &'a self,
        host: &'a str,
    ) -> Pin<Box<dyn Future<Output = io::Result<Vec<SocketAddr>>> + Send + 'a>> {
        TokioRuntime.resolve_host(host)
    }
    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        TokioRuntime.sleep(duration)
    }
    fn interval(&self, duration: Duration) -> Box<dyn AsyncInterval> {
        TokioRuntime.interval(duration)
    }
    fn block_on(&self, future: Pin<Box<dyn Future<Output = ()> + '_>>) {
        TokioRuntime.block_on(future)
    }
}
type Datagram = (SocketAddr, Vec<u8>);
struct Socket {
    udp: Arc<dyn AsyncUdpSocket>,
    routes: Vec<(SocketAddr, mpsc::Sender<Vec<u8>>)>,
    incoming: Mutex<mpsc::Receiver<Datagram>>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}
impl fmt::Debug for Socket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RelaySocket")
    }
}
impl Drop for Socket {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
impl AsyncUdpSocket for Socket {
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.udp.local_addr()
    }
    fn poll_send(&self, cx: &mut Context<'_>, transmit: &Transmit<'_>) -> Poll<io::Result<usize>> {
        if let Some((_, send)) = self
            .routes
            .iter()
            .find(|(addr, _)| *addr == transmit.destination)
        {
            if transmit.contents.len() > MAX_PACKET {
                return Poll::Ready(Err(io::ErrorKind::InvalidData.into()));
            }
            // Congestion and a failed route behave like datagram loss; other routes remain usable.
            let _ = send.try_send(transmit.contents.to_vec());
            Poll::Ready(Ok(transmit.contents.len()))
        } else {
            self.udp.poll_send(cx, transmit)
        }
    }
    fn poll_recv(
        &self,
        cx: &mut Context<'_>,
        bufs: &mut [IoSliceMut<'_>],
        meta: &mut [RecvMeta],
    ) -> Poll<io::Result<usize>> {
        if bufs.is_empty() || meta.is_empty() {
            return Poll::Ready(Err(io::ErrorKind::InvalidInput.into()));
        }
        let mut incoming = self.incoming.lock().map_err(|_| io::ErrorKind::Other)?;
        for index in 0..32 {
            match incoming.poll_recv(cx) {
                Poll::Ready(Some((addr, bytes))) => {
                    if bytes.len() > bufs[0].len() {
                        if index == 31 {
                            cx.waker().wake_by_ref();
                        }
                        continue;
                    }
                    bufs[0][..bytes.len()].copy_from_slice(&bytes);
                    meta[0] = RecvMeta::default();
                    meta[0].addr = addr;
                    meta[0].len = bytes.len();
                    meta[0].stride = bytes.len().max(1);
                    return Poll::Ready(Ok(1));
                }
                _ => break,
            }
        }
        self.udp.poll_recv(cx, bufs, meta)
    }
}
trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}
async fn run(
    route: Route,
    tls: Arc<rustls::ClientConfig>,
    mut outgoing: mpsc::Receiver<Vec<u8>>,
    incoming: mpsc::Sender<Datagram>,
) -> io::Result<()> {
    let stream: Box<dyn Stream> = tokio::time::timeout(Duration::from_secs(8), async {
        let host = route.host.as_str();
        #[cfg(any(test, feature = "test-loopback"))]
        let host = if host == "chat.example" {
            "127.0.0.1"
        } else {
            host
        };
        let tcp = tokio::net::TcpStream::connect((host, route.port)).await?;
        tcp.set_nodelay(true)?;
        if route.tls {
            let name = ServerName::try_from(route.host.clone())
                .map_err(|_| io::ErrorKind::InvalidInput)?;
            Ok::<Box<dyn Stream>, io::Error>(Box::new(
                TlsConnector::from(tls).connect(name, tcp).await?,
            ))
        } else {
            Ok::<Box<dyn Stream>, io::Error>(Box::new(tcp))
        }
    })
    .await
    .map_err(|_| io::ErrorKind::TimedOut)??;
    let (mut read, mut write) = tokio::io::split(stream);
    let receive = async {
        loop {
            let bytes = read_packet(&mut read).await?;
            if incoming.try_send((route.alias, bytes)).is_err() && incoming.is_closed() {
                return Ok::<(), io::Error>(());
            }
        }
    };
    let send = async {
        while let Some(mut bytes) = outgoing.recv().await {
            let (size, padding) = frame_size(
                bytes
                    .get(..4)
                    .ok_or(io::ErrorKind::InvalidData)?
                    .try_into()
                    .unwrap(),
            )?;
            if bytes.len() != size && bytes.len() != size + padding {
                return Err(io::ErrorKind::InvalidData.into());
            }
            bytes.resize(size + padding, 0);
            tokio::time::timeout(Duration::from_secs(5), write.write_all(&bytes))
                .await
                .map_err(|_| io::ErrorKind::TimedOut)??;
        }
        Ok(())
    };
    tokio::select! { result = receive => result, result = send => result }
}
fn frame_size(header: [u8; 4]) -> io::Result<(usize, usize)> {
    let body = u16::from_be_bytes([header[2], header[3]]) as usize;
    let channel = (0x40..0x80).contains(&header[0]);
    let size = if channel {
        4 + body
    } else if header[0] & 0xc0 == 0 && body.is_multiple_of(4) {
        20 + body
    } else {
        return Err(io::ErrorKind::InvalidData.into());
    };
    if size > MAX_PACKET {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok((size, if channel { (4 - size % 4) % 4 } else { 0 }))
}
async fn read_packet(read: &mut (impl AsyncRead + Unpin)) -> io::Result<Vec<u8>> {
    let mut header = [0; 4];
    read.read_exact(&mut header).await?;
    let (size, padding) = frame_size(header)?;
    let mut bytes = vec![0; size + padding];
    bytes[..4].copy_from_slice(&header);
    read.read_exact(&mut bytes[4..]).await?;
    bytes.truncate(size);
    if header[0] & 0xc0 == 0 && bytes[4..8] != [0x21, 0x12, 0xa4, 0x42] {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn discarding_a_full_batch_does_not_strand_the_next_datagram() {
        use std::{
            sync::atomic::{AtomicBool, Ordering},
            task::{Wake, Waker},
        };
        struct Notified(AtomicBool);
        impl Wake for Notified {
            fn wake(self: Arc<Self>) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let udp = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        udp.set_nonblocking(true).unwrap();
        let (send, incoming) = mpsc::channel(40);
        for _ in 0..32 {
            send.try_send((([127, 0, 0, 1], 10000).into(), vec![0; 3]))
                .unwrap();
        }
        send.try_send((([127, 0, 0, 1], 10000).into(), vec![7, 8]))
            .unwrap();
        let socket = Socket {
            udp: TokioRuntime.wrap_udp_socket(udp).unwrap(),
            routes: Vec::new(),
            incoming: Mutex::new(incoming),
            tasks: Vec::new(),
        };
        let notified = Arc::new(Notified(AtomicBool::new(false)));
        let waker = Waker::from(notified.clone());
        let mut cx = Context::from_waker(&waker);
        let mut buffer = [0; 2];
        let mut bufs = [IoSliceMut::new(&mut buffer)];
        let mut meta = [RecvMeta::default()];
        assert!(socket.poll_recv(&mut cx, &mut bufs, &mut meta).is_pending());
        assert!(notified.0.load(Ordering::SeqCst));
        assert!(matches!(
            socket.poll_recv(&mut cx, &mut bufs, &mut meta),
            Poll::Ready(Ok(1))
        ));
        assert_eq!(meta[0].len, 2);
        assert_eq!(buffer, [7, 8]);
    }
    #[tokio::test]
    async fn fragmented_frames_preserve_boundaries_and_reject_oversized_input() {
        let mut stun = vec![0, 1, 0, 0, 0x21, 0x12, 0xa4, 0x42];
        stun.resize(20, 0);
        let channel = vec![0x40, 1, 0, 3, 9, 8, 7];
        let mut wire = channel.clone();
        wire.push(0);
        wire.extend(&stun);
        let (mut write, mut read) = tokio::io::duplex(32);
        let sender = tokio::spawn(async move {
            for byte in wire {
                write.write_all(&[byte]).await.unwrap();
            }
        });
        assert_eq!(read_packet(&mut read).await.unwrap(), channel);
        assert_eq!(read_packet(&mut read).await.unwrap(), stun);
        sender.await.unwrap();
        assert!(read_packet(&mut read).await.is_err());
        assert!(frame_size([0x40, 0, 0xff, 0xff]).is_err());
        assert!(frame_size([0x80, 0, 0, 0]).is_err());
        assert!(frame_size([0, 1, 0, 1]).is_err());
        let bad_cookie = vec![0; 20];
        assert!(read_packet(&mut bad_cookie.as_slice()).await.is_err());
    }
    #[test]
    fn tls_checks_the_configured_hostname_and_trust_roots_before_forwarding() {
        let (_dir, _fixture, alice, _bob, _now) = crate::claims::tests::pair();
        let tls = RelayRuntime::configure(&alice, &mut [])
            .unwrap()
            .tls
            .clone();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            for (host, trusted, expected) in [
                ("chat.example", true, true),
                ("localhost", true, false),
                ("chat.example", false, false),
            ] {
                let provider = Arc::new(rustls::crypto::ring::default_provider());
                let config = rustls::ServerConfig::builder_with_provider(provider.clone())
                    .with_safe_default_protocol_versions()
                    .unwrap()
                    .with_no_client_auth()
                    .with_single_cert(
                        vec![CertificateDer::from(
                            include_bytes!("../../tests/fixtures/synthetic-server.der").to_vec(),
                        )],
                        rustls::pki_types::PrivateKeyDer::try_from(
                            include_bytes!("../../tests/fixtures/synthetic-server-key.der")
                                .to_vec(),
                        )
                        .unwrap(),
                    )
                    .unwrap();
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let port = listener.local_addr().unwrap().port();
                let server = tokio::spawn(async move {
                    let (tcp, _) = listener.accept().await.unwrap();
                    match tokio_rustls::TlsAcceptor::from(Arc::new(config))
                        .accept(tcp)
                        .await
                    {
                        Ok(mut stream) => read_packet(&mut stream).await.is_ok(),
                        Err(_) => false,
                    }
                });
                let tls = if trusted {
                    tls.clone()
                } else {
                    Arc::new(
                        rustls::ClientConfig::builder_with_provider(provider)
                            .with_safe_default_protocol_versions()
                            .unwrap()
                            .with_root_certificates(rustls::RootCertStore::empty())
                            .with_no_client_auth(),
                    )
                };
                let (tx, outgoing) = mpsc::channel(1);
                let (incoming, _rx) = mpsc::channel(1);
                let mut packet = vec![0, 1, 0, 0, 0x21, 0x12, 0xa4, 0x42];
                packet.resize(20, 0);
                tx.send(packet).await.unwrap();
                let task = tokio::spawn(run(
                    Route {
                        host: host.into(),
                        port,
                        tls: true,
                        alias: ([127, 0, 0, 1], 10000).into(),
                    },
                    tls,
                    outgoing,
                    incoming,
                ));
                assert_eq!(
                    tokio::time::timeout(Duration::from_secs(10), server)
                        .await
                        .unwrap()
                        .unwrap(),
                    expected
                );
                drop(tx);
                let _ = task.await;
            }
        });
    }
}
