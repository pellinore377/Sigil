use super::*;
pub(super) struct PooledAgent {
    pub(super) id: [u8; 32],
    pub(super) created: crate::clock::Instant,
    pub(super) agent: Agent,
    pub(super) resolved: DiscoveryCache,
}
pub(super) static AGENTS: std::sync::Mutex<Vec<PooledAgent>> = std::sync::Mutex::new(Vec::new());
pub(super) static DNS_JOBS: AtomicUsize = AtomicUsize::new(0);
static DNS_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());
static DNS_READY: std::sync::Condvar = std::sync::Condvar::new();
struct DnsPermit;
impl Drop for DnsPermit {
    fn drop(&mut self) {
        let _guard = DNS_GATE.lock().unwrap_or_else(|e| e.into_inner());
        DNS_JOBS.fetch_sub(1, Ordering::AcqRel);
        DNS_READY.notify_one();
    }
}
#[derive(Debug)]
pub(super) struct BoundedResolver;
pub(super) fn lookup(
    timeout: NextTimeout,
    resolve: impl FnOnce() -> std::io::Result<Vec<SocketAddr>> + Send + 'static,
) -> Result<Vec<SocketAddr>, ureq::Error> {
    // A burst waits for a slot within the same deadline; timed-out lookups keep their slot.
    let deadline = std::time::Instant::now() + (*timeout.after).min(Duration::from_secs(5));
    let mut guard = DNS_GATE.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        if std::time::Instant::now() >= deadline {
            return Err(ureq::Error::Timeout(timeout.reason));
        }
        if DNS_JOBS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < 4).then_some(count + 1)
            })
            .is_ok()
        {
            break;
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(ureq::Error::Timeout(timeout.reason));
        }
        guard = DNS_READY
            .wait_timeout(guard, remaining)
            .unwrap_or_else(|e| e.into_inner())
            .0;
    }
    drop(guard);
    let permit = DnsPermit;
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("sigil-dns".into())
        .spawn(move || {
            let _permit = permit;
            let _ = sender.send(resolve());
        })
        .map_err(ureq::Error::Io)?;
    match receiver.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
        Ok(result) => result.map_err(ureq::Error::Io),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(ureq::Error::Timeout(timeout.reason)),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(ureq::Error::Io(std::io::Error::from(
            std::io::ErrorKind::Other,
        ))),
    }
}
impl Resolver for BoundedResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        _: &ureq::config::Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let host = uri.host().ok_or(ureq::Error::HostNotFound)?.to_owned();
        let port = uri.port_u16().unwrap_or(443);
        #[cfg(feature = "test-loopback")]
        if host == "chat.example" {
            let mut result = self.empty();
            result.push(SocketAddr::from(([127, 0, 0, 1], port)));
            return Ok(result);
        }
        let addresses = lookup(timeout, move || {
            (host.as_str(), port)
                .to_socket_addrs()
                .map(|values| values.take(16).collect())
        })?;
        let mut result = self.empty();
        for address in addresses {
            result.push(address);
        }
        if result.is_empty() {
            Err(ureq::Error::HostNotFound)
        } else {
            Ok(result)
        }
    }
}
