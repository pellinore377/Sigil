use std::sync::Arc;
#[allow(dead_code)]
pub async fn proxy(
    sdp: String,
    lossy: bool,
) -> (
    String,
    tokio::task::JoinHandle<()>,
    Arc<std::sync::atomic::AtomicUsize>,
) {
    let first = sdp.lines().find(|l| l.starts_with("a=candidate:")).unwrap();
    let parts: Vec<_> = first.split_whitespace().collect();
    let target: std::net::SocketAddr = format!("{}:{}", parts[4], parts[5]).parse().unwrap();
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let local = socket.local_addr().unwrap();
    let changed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = changed.clone();
    let rewritten = sdp
        .lines()
        .map(|line| {
            if line.starts_with("a=candidate:") {
                let mut parts: Vec<_> = line.split_whitespace().map(str::to_owned).collect();
                parts[4] = local.ip().to_string();
                parts[5] = local.port().to_string();
                parts.join(" ")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\r\n")
        + "\r\n";
    let task = tokio::spawn(async move {
        let mut buf = [0; 2048];
        let mut sender = None;
        let mut n = 0usize;
        let mut held: Option<Vec<u8>> = None;
        loop {
            let (len, from) = socket.recv_from(&mut buf).await.unwrap();
            let to = if from == target {
                let Some(to) = sender else {
                    continue;
                };
                to
            } else {
                sender = Some(from);
                target
            };
            let packet = &buf[..len];
            let rtp = from != target
                && len > 12
                && packet[0] & 0xc0 == 0x80
                && matches!(packet[1] & 0x7f, 96 | 111);
            if lossy && rtp {
                n += 1;
                if n.is_multiple_of(17) {
                    counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    continue;
                }
                if n.is_multiple_of(23) && held.is_none() {
                    held = Some(packet.to_vec());
                    counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    continue;
                }
                if n.is_multiple_of(19) {
                    socket.send_to(packet, to).await.unwrap();
                    counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
            socket.send_to(packet, to).await.unwrap();
            if rtp {
                if let Some(packet) = held.take() {
                    socket.send_to(&packet, to).await.unwrap();
                }
            }
        }
    });
    (rewritten, task, changed)
}
