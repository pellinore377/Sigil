use std::{net::SocketAddr, sync::Arc};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UdpSocket},
    task::JoinHandle,
};

trait Stream: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin> Stream for T {}

// webrtc-rs's test client gathers TURN/UDP only; bridge its datagrams to real TURN/TCP or TLS.
pub async fn bridge(url: &str) -> (String, Option<JoinHandle<()>>) {
    if url.ends_with("?transport=udp") {
        return (url.to_owned(), None);
    }
    let (scheme, rest) = url.split_once(':').unwrap();
    let authority = rest.strip_suffix("?transport=tcp").unwrap();
    let address: SocketAddr = authority.parse().expect("synthetic loopback TURN address");
    assert!(address.ip().is_loopback());
    let tcp = TcpStream::connect(address).await.unwrap();
    tcp.set_nodelay(true).unwrap();
    let stream: Box<dyn Stream> = if scheme == "turns" {
        use tokio_rustls::rustls::{
            self,
            pki_types::{CertificateDer, ServerName},
        };
        let mut roots = rustls::RootCertStore::empty();
        roots
            .add(CertificateDer::from(
                include_bytes!("../../../client/tests/fixtures/synthetic-ca.der").to_vec(),
            ))
            .unwrap();
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
        let tls = tokio_rustls::TlsConnector::from(Arc::new(config))
            .connect(ServerName::try_from("chat.example").unwrap(), tcp)
            .await
            .unwrap();
        Box::new(tls)
    } else {
        assert_eq!(scheme, "turn");
        Box::new(tcp)
    };
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let url = format!("turn:{}?transport=udp", socket.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut read, mut write) = tokio::io::split(stream);
        let mut incoming = Vec::new();
        let mut udp = [0; 2048];
        let mut tcp = [0; 4096];
        let mut client = None;
        loop {
            tokio::select! {
                received = socket.recv_from(&mut udp) => {
                    let (length, source) = received.unwrap();
                    if client.is_some_and(|old| old != source) { continue; }
                    client = Some(source);
                    if write.write_all(&udp[..length]).await.is_err() { return; }
                    if udp[0] & 0xc0 == 0x40 {
                        let padding = (4 - length % 4) % 4;
                        if write.write_all(&[0;3][..padding]).await.is_err() { return; }
                    }
                }
                received = read.read(&mut tcp) => {
                    let Ok(length) = received else { return; };
                    if length == 0 { return; }
                    incoming.extend_from_slice(&tcp[..length]);
                    assert!(incoming.len() < 16384);
                    while incoming.len() >= 4 {
                        let channel = incoming[0] & 0xc0 == 0x40;
                        let length = u16::from_be_bytes([incoming[2],incoming[3]]) as usize + if channel {4} else {20};
                        let padded = if channel { length.next_multiple_of(4) } else {length};
                        if incoming.len() < padded { break; }
                        if let Some(client) = client { socket.send_to(&incoming[..length], client).await.unwrap(); }
                        incoming.drain(..padded);
                    }
                }
            }
        }
    });
    (url, Some(task))
}
