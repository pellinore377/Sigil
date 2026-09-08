//! Bounded HTTPS for untrusted provider endpoints. No endpoint or response body
//! is included in an error; capability URLs and OAuth responses are sensitive.
use serde::{Deserialize, Serialize};
use std::{
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Arc,
    },
    time::Duration,
};
use ureq::{
    http::{header, Request, Uri},
    tls::{Certificate, RootCerts, TlsConfig},
    unversioned::{
        resolver::{ResolvedSocketAddrs, Resolver},
        transport::{DefaultConnector, NextTimeout},
    },
    Agent,
};
use zeroize::Zeroizing;
const LIMIT: usize = 16 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidEndpoint,
    Policy,
    Transport,
    InvalidResponse,
    Limit,
}
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Exception {
    pub host: String,
    pub port: u16,
    pub networks: Vec<String>,
    /// Optional DER CA for this exact host/port; ordinary certificate and name
    /// validation remain required. This is operator configuration, never client input.
    pub root_ca: Option<Vec<u8>>,
}
#[derive(Clone)]
struct Network {
    address: IpAddr,
    prefix: u8,
}
impl Network {
    fn parse(value: &str) -> Result<Self, Error> {
        let (ip, prefix) = value.split_once('/').ok_or(Error::Policy)?;
        let address: IpAddr = ip.parse().map_err(|_| Error::Policy)?;
        let prefix: u8 = prefix.parse().map_err(|_| Error::Policy)?;
        let network = Self { address, prefix };
        let bits = if address.is_ipv4() { 32 } else { 128 };
        if prefix > bits || value != format!("{address}/{prefix}") || !network.canonical() {
            return Err(Error::Policy);
        }
        Ok(network)
    }
    fn canonical(&self) -> bool {
        match self.address {
            IpAddr::V4(ip) => self.prefix <= 32 && (u32::from(ip) & !mask32(self.prefix)) == 0,
            IpAddr::V6(ip) => self.prefix <= 128 && (u128::from(ip) & !mask128(self.prefix)) == 0,
        }
    }
    fn contains(&self, ip: IpAddr) -> bool {
        match (self.address, ip) {
            (IpAddr::V4(a), IpAddr::V4(b)) => {
                u32::from(a) & mask32(self.prefix) == u32::from(b) & mask32(self.prefix)
            }
            (IpAddr::V6(a), IpAddr::V6(b)) => {
                u128::from(a) & mask128(self.prefix) == u128::from(b) & mask128(self.prefix)
            }
            _ => false,
        }
    }
}
fn mask32(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    }
}
fn mask128(prefix: u8) -> u128 {
    if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - prefix)
    }
}
fn host(uri: &Uri) -> Result<String, Error> {
    let host = uri
        .host()
        .ok_or(Error::InvalidEndpoint)?
        .trim_start_matches('[')
        .trim_end_matches(']');
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(ip.to_string());
    }
    if !sigil_protocol::valid_server_name(host) {
        return Err(Error::InvalidEndpoint);
    }
    Ok(host.to_owned())
}
pub fn endpoint(value: &str) -> Result<Uri, Error> {
    if value.is_empty() || value.len() > 1000 || !value.is_ascii() || value.contains('#') {
        return Err(Error::InvalidEndpoint);
    }
    let uri: Uri = value.parse().map_err(|_| Error::InvalidEndpoint)?;
    if uri.scheme_str() != Some("https")
        || uri.port_u16() == Some(0)
        || uri
            .authority()
            .is_none_or(|authority| authority.as_str().contains('@'))
    {
        return Err(Error::InvalidEndpoint);
    }
    host(&uri)?;
    Ok(uri)
}
// Conservative ordinary-unicast policy. Reserved/protocol/documentation and
// translation ranges require the same explicit operator exception as private IPs.
fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(matches!(a, 0 | 10 | 127 | 224..=255)
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192
                    && ((b == 0 && matches!(c, 0 | 2)) || (b == 88 && c == 99) || b == 168))
                || (a == 198 && (matches!(b, 18 | 19) || (b == 51 && c == 100)))
                || (a == 203 && b == 0 && c == 113))
        }
        IpAddr::V6(ip) => {
            let p = ip.segments();
            (p[0] & 0xe000) == 0x2000
                && !(p[0] == 0x2001 && (p[1] < 0x0200 || p[1] == 0x0db8))
                && p[0] != 0x2002
                && !(p[0] == 0x3fff && (p[1] & 0xf000) == 0)
        }
    }
}
#[derive(Clone, Default)]
pub struct Policy {
    exceptions: Vec<(Exception, Vec<Network>)>,
}
impl Policy {
    pub fn new(exceptions: Vec<Exception>) -> Result<Self, Error> {
        if exceptions.len() > 8 {
            return Err(Error::Policy);
        }
        let mut parsed = Vec::new();
        for exception in exceptions {
            if exception.port == 0
                || exception
                    .host
                    .parse::<IpAddr>()
                    .is_ok_and(|ip| ip.to_string() != exception.host)
                || exception.networks.is_empty()
                || exception.networks.len() > 4
                || exception
                    .root_ca
                    .as_ref()
                    .is_some_and(|ca| ca.is_empty() || ca.len() > 4096)
                || !(sigil_protocol::valid_server_name(&exception.host)
                    || exception.host.parse::<IpAddr>().is_ok())
                || parsed.iter().any(|(old, _): &(Exception, Vec<Network>)| {
                    old.host == exception.host && old.port == exception.port
                })
            {
                return Err(Error::Policy);
            }
            let networks = exception
                .networks
                .iter()
                .map(|value| Network::parse(value))
                .collect::<Result<Vec<_>, _>>()?;
            parsed.push((exception, networks));
        }
        Ok(Self { exceptions: parsed })
    }
    fn exception(&self, uri: &Uri) -> Result<Option<&(Exception, Vec<Network>)>, Error> {
        let host = host(uri)?;
        let port = uri.port_u16().unwrap_or(443);
        Ok(self
            .exceptions
            .iter()
            .find(|(rule, _)| rule.host == host && rule.port == port))
    }
    fn check(&self, uri: &Uri, addresses: &[SocketAddr]) -> Result<(), Error> {
        if addresses.is_empty() || addresses.len() > 16 {
            return Err(Error::Policy);
        }
        let port = uri.port_u16().unwrap_or(443);
        let rule = self.exception(uri)?;
        if addresses.iter().any(|address| {
            address.port() != port
                || match rule {
                    Some((_, networks)) => !networks
                        .iter()
                        .any(|network| network.contains(address.ip())),
                    None => port != 443 || !public(address.ip()),
                }
        }) {
            return Err(Error::Policy);
        }
        Ok(())
    }
    pub fn send(&self, request: Request<&[u8]>) -> Result<Response, Error> {
        self.send_with(request, |host, port| {
            (host.as_str(), port)
                .to_socket_addrs()
                .map(|v| v.take(17).collect())
        })
    }
    fn send_with(
        &self,
        request: Request<&[u8]>,
        lookup: impl Fn(String, u16) -> std::io::Result<Vec<SocketAddr>> + Send + Sync + 'static,
    ) -> Result<Response, Error> {
        if request.body().len() > LIMIT || request.method() != ureq::http::Method::POST {
            return Err(Error::Limit);
        }
        self.exchange_with(request, lookup, LIMIT, Duration::from_secs(15))
    }
    pub(crate) fn federation(&self, mut request: Request<&[u8]>) -> Result<Response, Error> {
        let uri = request.uri();
        let server = host(uri)?;
        let url = format!(
            "https://{}:{}{}",
            server,
            uri.port_u16().unwrap_or(443),
            sigil_protocol::discovery::PATH
        );
        let discovery =
            self.federation_direct(Request::get(url).body(&[][..]).map_err(|_| Error::Policy)?)?;
        if discovery.status != 404 {
            if discovery.status != 200
                || discovery.body.len() > sigil_protocol::discovery::MAX_BODY
                || discovery.content_type.as_deref() != Some("application/json")
            {
                return Err(Error::Policy);
            }
            let discovered: sigil_protocol::discovery::Discovery =
                serde_json::from_slice(&discovery.body).map_err(|_| Error::Policy)?;
            if !discovered.valid_for(&server) {
                return Err(Error::Policy);
            }
            let path = uri.path_and_query().ok_or(Error::Policy)?;
            *request.uri_mut() = format!("{}{path}", discovered.api_origin)
                .parse()
                .map_err(|_| Error::Policy)?;
        }
        self.federation_direct(request)
    }
    fn federation_direct(&self, request: Request<&[u8]>) -> Result<Response, Error> {
        self.federation_with(request, |host, port| {
            (host.as_str(), port)
                .to_socket_addrs()
                .map(|v| v.take(17).collect())
        })
    }
    fn federation_with(
        &self,
        request: Request<&[u8]>,
        lookup: impl Fn(String, u16) -> std::io::Result<Vec<SocketAddr>> + Send + Sync + 'static,
    ) -> Result<Response, Error> {
        let method = request.method();
        if request.body().len()
            > if request.uri().path() == sigil_protocol::federation::LOOKUP_PATH {
                sigil_protocol::federation::MAX_LOOKUP_BODY
            } else {
                sigil_protocol::federation::MAX_BODY
            }
            || !((method == ureq::http::Method::GET && request.body().is_empty())
                || method == ureq::http::Method::POST)
        {
            return Err(Error::Limit);
        }
        let limit = if request.uri().path() == sigil_protocol::federation::LOOKUP_PATH {
            sigil_protocol::federation::MAX_LOOKUP_RESPONSE + 8192
        } else {
            LIMIT
        };
        self.exchange_with(request, lookup, limit, Duration::from_secs(15))
    }
    pub(crate) fn service(&self, request: Request<&[u8]>) -> Result<Response, Error> {
        if request.body().len() > sigil_protocol::services::MAX_BODY
            || !matches!(
                *request.method(),
                ureq::http::Method::GET | ureq::http::Method::POST
            )
        {
            return Err(Error::Limit);
        }
        self.exchange_with(
            request,
            |host, port| {
                (host.as_str(), port)
                    .to_socket_addrs()
                    .map(|v| v.take(17).collect())
            },
            sigil_protocol::services::MAX_RESPONSE,
            Duration::from_secs(3),
        )
    }
    fn exchange_with(
        &self,
        request: Request<&[u8]>,
        lookup: impl Fn(String, u16) -> std::io::Result<Vec<SocketAddr>> + Send + Sync + 'static,
        response_limit: usize,
        timeout: Duration,
    ) -> Result<Response, Error> {
        let uri = endpoint(&request.uri().to_string())?;
        let roots = match self
            .exception(&uri)?
            .and_then(|(rule, _)| rule.root_ca.as_ref())
        {
            Some(ca) => RootCerts::new_with_certs(&[Certificate::from_der(ca).to_owned()]),
            None => RootCerts::WebPki,
        };
        let config = Agent::config_builder()
            .https_only(true)
            .proxy(None)
            .max_redirects(0)
            .http_status_as_error(false)
            .tls_config(TlsConfig::builder().root_certs(roots).build())
            .timeout_global(Some(timeout))
            .timeout_resolve(Some(Duration::from_secs(5)))
            .timeout_connect(Some(Duration::from_secs(5)))
            .timeout_recv_body(Some(Duration::from_secs(5)))
            .max_response_header_size(8192)
            .max_idle_connections(0)
            .max_idle_connections_per_host(0)
            .user_agent("Sigil/experimental-v0")
            .build();
        let denied = Arc::new(AtomicBool::new(false));
        let resolver = CheckedResolver {
            policy: self.clone(),
            denied: denied.clone(),
            lookup: Arc::new(lookup),
        };
        let agent = Agent::with_parts(config, DefaultConnector::default(), resolver);
        let mut response = agent.run(request).map_err(|_| {
            if denied.load(Ordering::Acquire) {
                Error::Policy
            } else {
                Error::Transport
            }
        })?;
        if response
            .headers()
            .get_all(header::CONTENT_ENCODING)
            .iter()
            .any(|v| v.as_bytes() != b"identity")
        {
            return Err(Error::InvalidResponse);
        }
        if response
            .headers()
            .get(header::CONTENT_LENGTH)
            .is_some_and(|v| {
                v.to_str()
                    .ok()
                    .and_then(|s| s.parse::<u64>().ok())
                    .is_none_or(|len| len > response_limit as u64)
            })
        {
            return Err(Error::Limit);
        }
        let status = response.status().as_u16();
        let retry_after = response.headers().get_all(header::RETRY_AFTER);
        let retry_after = if retry_after.iter().count() == 1 {
            retry_after
                .iter()
                .next()
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        } else {
            None
        };
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let body = Zeroizing::new(
            response
                .body_mut()
                .with_config()
                .limit(response_limit as u64)
                .read_to_vec()
                .map_err(|_| Error::InvalidResponse)?,
        );
        Ok(Response {
            status,
            retry_after,
            content_type,
            body,
        })
    }
}
pub struct Response {
    pub status: u16,
    pub retry_after: Option<String>,
    pub content_type: Option<String>,
    pub body: Zeroizing<Vec<u8>>,
}
static DNS_JOBS: AtomicUsize = AtomicUsize::new(0);
struct DnsPermit;
impl Drop for DnsPermit {
    fn drop(&mut self) {
        DNS_JOBS.fetch_sub(1, Ordering::AcqRel);
    }
}
type Lookup = dyn Fn(String, u16) -> std::io::Result<Vec<SocketAddr>> + Send + Sync;
struct CheckedResolver {
    policy: Policy,
    denied: Arc<AtomicBool>,
    lookup: Arc<Lookup>,
}
impl std::fmt::Debug for CheckedResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CheckedResolver")
    }
}
impl Resolver for CheckedResolver {
    fn resolve(
        &self,
        uri: &Uri,
        _: &ureq::config::Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let permission = || ureq::Error::Io(std::io::ErrorKind::PermissionDenied.into());
        let name = host(uri).map_err(|_| permission())?;
        let port = uri.port_u16().unwrap_or(443);
        DNS_JOBS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 4).then_some(n + 1)
            })
            .map_err(|_| ureq::Error::Io(std::io::ErrorKind::WouldBlock.into()))?;
        let permit = DnsPermit;
        let lookup = self.lookup.clone();
        let (send, recv) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("sigil-provider-dns".into())
            .spawn(move || {
                let _permit = permit;
                let _ = send.send(lookup(name, port));
            })
            .map_err(ureq::Error::Io)?;
        let addresses = recv
            .recv_timeout((*timeout.after).min(Duration::from_secs(5)))
            .map_err(|_| ureq::Error::Timeout(timeout.reason))?
            .map_err(ureq::Error::Io)?;
        if self.policy.check(uri, &addresses).is_err() {
            self.denied.store(true, Ordering::Release);
            return Err(permission());
        }
        let mut result = self.empty();
        for address in addresses {
            result.push(address);
        }
        Ok(result)
    }
}

#[cfg(test)]
#[path = "egress_tests.rs"]
pub(crate) mod tests;
