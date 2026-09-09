//! Native HTTPS transport. Blocking calls belong on a dedicated worker, never UI.
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};
use sigil_crypto::recovery::Object;
use sigil_protocol::{accounts, mailbox, prekeys, recovery};
use std::time::Duration;
use std::{
    net::{SocketAddr, ToSocketAddrs},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
};
use ureq::unversioned::{
    resolver::{ResolvedSocketAddrs, Resolver},
    transport::{DefaultConnector, NextTimeout},
};
use ureq::{
    http::{header, HeaderValue, Method, Request, Response},
    tls::{Certificate, RootCerts, TlsConfig},
    Agent, Body,
};
use zeroize::Zeroizing;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Configuration,
    Transport,
    InvalidResponse,
    Limit,
    Status {
        code: u16,
        retry_after_seconds: Option<u64>,
    },
}

pub struct HttpsClient {
    agent: Agent,
    origin: String,
    server: String,
    credential: Zeroizing<String>,
    discover: bool,
    resolved: std::sync::OnceLock<String>,
}
const SMALL: usize = 8192;
#[path = "admin_network.rs"]
mod admin;
const MAILBOX_RESPONSE: usize = 16 * (mailbox::MAX_PAYLOAD_HEX + 1024);
#[path = "attachment_network.rs"]
mod attachments;
#[path = "call_network.rs"]
mod calls;
#[path = "federation_network.rs"]
mod federation;
#[path = "group_network.rs"]
mod groups;
#[path = "push_network.rs"]
mod push;
#[path = "service_network.rs"]
mod services;
pub use calls::CallAvailability;
pub(crate) use push::{target_fields as push_target_fields, valid_status as valid_push_status};
pub use services::{MapAvailability, MapTile};
static DNS_JOBS: AtomicUsize = AtomicUsize::new(0);
struct DnsPermit;
impl Drop for DnsPermit {
    fn drop(&mut self) {
        DNS_JOBS.fetch_sub(1, Ordering::AcqRel);
    }
}
#[derive(Debug)]
struct BoundedResolver;
fn lookup(
    timeout: NextTimeout,
    resolve: impl FnOnce() -> std::io::Result<Vec<SocketAddr>> + Send + 'static,
) -> Result<Vec<SocketAddr>, ureq::Error> {
    DNS_JOBS
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
            (count < 4).then_some(count + 1)
        })
        .map_err(|_| ureq::Error::Io(std::io::Error::from(std::io::ErrorKind::WouldBlock)))?;
    let permit = DnsPermit;
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("sigil-dns".into())
        .spawn(move || {
            let _permit = permit;
            let _ = sender.send(resolve());
        })
        .map_err(ureq::Error::Io)?;
    match receiver.recv_timeout((*timeout.after).min(Duration::from_secs(5))) {
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

#[cfg(test)]
#[path = "network_tests.rs"]
pub(crate) mod tests;

pub(super) fn valid_hex(value: &str, minimum: usize, maximum: usize) -> bool {
    (minimum..=maximum).contains(&value.len())
        && value.len().is_multiple_of(2)
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn valid_time(value: u64) -> bool {
    value > 0 && value <= i64::MAX as u64
}
fn valid_head(head: &recovery::Head) -> bool {
    match &head.manifest {
        None => head.generation == 0 && !head.restored_checkpoint,
        Some(id) => accounts::valid_credential(id) && valid_time(head.generation),
    }
}
fn retry_after(response: &Response<Body>) -> Option<u64> {
    let values = response.headers().get_all(header::RETRY_AFTER);
    if values.iter().count() != 1 {
        return None;
    }
    let value = values.iter().next()?.to_str().ok()?;
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse::<u64>().ok().filter(|value| *value <= 86400)
}

impl HttpsClient {
    pub fn login_methods(
        server: &str,
        port: u16,
        roots: &[Vec<u8>],
    ) -> Result<sigil_protocol::login::Methods, Error> {
        let client = Self::new(server, port, &"0".repeat(64), roots)?;
        let response = client.send(
            Request::get(format!(
                "{}{}",
                client.origin,
                sigil_protocol::discovery::PATH
            ))
            .body(&[][..])
            .map_err(|_| Error::Configuration)?,
        )?;
        let discovered: sigil_protocol::discovery::Discovery =
            client.json(response, 200, sigil_protocol::discovery::MAX_BODY)?;
        if !discovered.valid_for(&discovered.server_name) {
            return Err(Error::InvalidResponse);
        }
        // A service address may suggest an identity domain; that domain must independently delegate back.
        let canonical = Self::discover(
            &discovered.server_name,
            if discovered.server_name == server {
                port
            } else {
                443
            },
            &"0".repeat(64),
            roots,
        )?;
        if canonical.api_origin()? != discovered.api_origin {
            return Err(Error::InvalidResponse);
        }
        if server != discovered.server_name && client.origin != discovered.api_origin {
            return Err(Error::InvalidResponse);
        }
        let response = canonical.request(Method::GET, "/client/v0/login", None::<&()>)?;
        let methods: sigil_protocol::login::Methods = canonical.json(response, 200, SMALL)?;
        if methods.server_name != discovered.server_name {
            return Err(Error::InvalidResponse);
        }
        Ok(methods)
    }
    pub fn password_sign_in(
        &self,
        username: &str,
        password: &str,
        label: &str,
    ) -> Result<accounts::Session, Error> {
        let bytes = Zeroizing::new(serde_json::to_vec(&serde_json::json!({"username":username,"password":password,"device_credential":self.credential.as_str(),"device_label":label})).map_err(|_|Error::Configuration)?);
        let response = self.request_bytes(
            Method::POST,
            "/client/v0/login/password",
            &bytes,
            Some("application/json"),
            None,
        )?;
        self.json(response, 200, SMALL)
    }
    pub fn discover(
        server: &str,
        port: u16,
        credential: &str,
        roots: &[Vec<u8>],
    ) -> Result<Self, Error> {
        let mut client = Self::new(server, port, credential, roots)?;
        client.discover = true;
        Ok(client)
    }
    fn api_origin(&self) -> Result<&str, Error> {
        if !self.discover {
            return Ok(&self.origin);
        }
        if self.resolved.get().is_none() {
            let _ = self.resolved.set(self.resolve_origin()?);
        }
        self.resolved
            .get()
            .map(String::as_str)
            .ok_or(Error::Configuration)
    }
    fn resolve_origin(&self) -> Result<String, Error> {
        let request = Request::get(format!(
            "{}{}",
            self.origin,
            sigil_protocol::discovery::PATH
        ))
        .header(header::ACCEPT, "application/json")
        .body(&[][..])
        .map_err(|_| Error::Configuration)?;
        let response = match self.send(request) {
            Err(Error::Status { code: 404, .. }) => return Ok(self.origin.clone()),
            other => other?,
        };
        let discovery: sigil_protocol::discovery::Discovery =
            self.json(response, 200, sigil_protocol::discovery::MAX_BODY)?;
        if !discovery.valid_for(&self.server) {
            return Err(Error::InvalidResponse);
        }
        Ok(discovery.api_origin)
    }
    /// Names are canonical homeserver DNS names; schemes, paths, credentials and
    /// IP literals are rejected. Empty roots uses bundled WebPKI roots. Explicit
    /// DER roots replace them for a privately administered CA; verification stays on.
    pub fn new(
        server: &str,
        port: u16,
        credential: &str,
        roots: &[Vec<u8>],
    ) -> Result<Self, Error> {
        super::recovery::account_scope(server, [0; 32]).map_err(|_| Error::Configuration)?;
        if port == 0
            || !accounts::valid_credential(credential)
            || roots.len() > 16
            || roots
                .iter()
                .any(|cert| cert.is_empty() || cert.len() > 16384)
            || roots.iter().map(Vec::len).sum::<usize>() > 65536
        {
            return Err(Error::Configuration);
        }
        let root_certs = if roots.is_empty() {
            RootCerts::WebPki
        } else {
            RootCerts::new_with_certs(
                &roots
                    .iter()
                    .map(|bytes| Certificate::from_der(bytes).to_owned())
                    .collect::<Vec<_>>(),
            )
        };
        let config = Agent::config_builder()
            .https_only(true)
            .proxy(None)
            .max_redirects(0)
            .http_status_as_error(false)
            .tls_config(TlsConfig::builder().root_certs(root_certs).build())
            .timeout_global(Some(Duration::from_secs(20)))
            .timeout_resolve(Some(Duration::from_secs(5)))
            .timeout_connect(Some(Duration::from_secs(5)))
            .timeout_send_body(Some(Duration::from_secs(10)))
            .timeout_recv_response(Some(Duration::from_secs(10)))
            .timeout_recv_body(Some(Duration::from_secs(10)))
            .max_response_header_size(SMALL)
            .input_buffer_size(16384)
            .output_buffer_size(16384)
            .max_idle_connections(1)
            .max_idle_connections_per_host(1)
            .user_agent("Sigil/experimental-v0")
            .build();
        let agent = Agent::with_parts(config, DefaultConnector::default(), BoundedResolver);
        #[cfg(test)]
        let agent = tests::agent(agent.config().clone());
        Ok(Self {
            agent,
            origin: format!("https://{server}:{port}"),
            server: server.into(),
            credential: Zeroizing::new(credential.into()),
            discover: false,
            resolved: std::sync::OnceLock::new(),
        })
    }

    fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<&impl Serialize>,
    ) -> Result<Response<Body>, Error> {
        let bytes = Zeroizing::new(match body {
            Some(value) => serde_json::to_vec(value).map_err(|_| Error::Configuration)?,
            None => Vec::new(),
        });
        if bytes.len() > recovery::MAX_BODY {
            return Err(Error::Limit);
        }
        self.request_bytes(method, path, &bytes, body.map(|_| "application/json"), None)
    }
    fn request_bytes(
        &self,
        method: Method,
        path: &str,
        bytes: &[u8],
        content_type: Option<&str>,
        access: Option<&str>,
    ) -> Result<Response<Body>, Error> {
        if bytes.len()
            > sigil_crypto::attachment::CHUNK_SIZE + sigil_crypto::attachment::CHUNK_OVERHEAD
        {
            return Err(Error::Limit);
        }
        let authorization = Zeroizing::new(format!("Bearer {}", self.credential.as_str()));
        let mut value = HeaderValue::from_str(&authorization).map_err(|_| Error::Configuration)?;
        value.set_sensitive(true);
        let mut builder = Request::builder()
            .method(method)
            .uri(format!("{}{path}", self.api_origin()?))
            .header(header::AUTHORIZATION, value)
            .header(
                header::ACCEPT,
                if access.is_some() {
                    "application/octet-stream"
                } else {
                    "application/json"
                },
            )
            .header(header::ACCEPT_ENCODING, "identity");
        if let Some(content_type) = content_type {
            builder = builder.header(header::CONTENT_TYPE, content_type);
        }
        if let Some(access) = access {
            if !accounts::valid_credential(access) {
                return Err(Error::Configuration);
            }
            let mut value = HeaderValue::from_str(access).map_err(|_| Error::Configuration)?;
            value.set_sensitive(true);
            builder = builder.header(sigil_protocol::attachments::ACCESS_HEADER, value);
        }
        let request = builder.body(bytes).map_err(|_| Error::Configuration)?;
        self.send(request)
    }
    fn send(&self, request: Request<&[u8]>) -> Result<Response<Body>, Error> {
        let response = self.agent.run(request).map_err(|_| Error::Transport)?;
        if !response.status().is_success() {
            return Err(Error::Status {
                code: response.status().as_u16(),
                retry_after_seconds: retry_after(&response),
            });
        }
        Ok(response)
    }
    fn json<T: DeserializeOwned>(
        &self,
        response: Response<Body>,
        status: u16,
        limit: usize,
    ) -> Result<T, Error> {
        let bytes = self.response_bytes(response, status, limit, "application/json")?;
        serde_json::from_slice(&bytes).map_err(|_| Error::InvalidResponse)
    }
    fn response_bytes(
        &self,
        mut response: Response<Body>,
        status: u16,
        limit: usize,
        content_type: &str,
    ) -> Result<Zeroizing<Vec<u8>>, Error> {
        if response.status().as_u16() != status {
            return Err(Error::InvalidResponse);
        }
        let types = response.headers().get_all(header::CONTENT_TYPE);
        if types.iter().count() != 1
            || !types
                .iter()
                .next()
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| {
                    value
                        .split(';')
                        .next()
                        .is_some_and(|kind| kind.trim().eq_ignore_ascii_case(content_type))
                })
        {
            return Err(Error::InvalidResponse);
        }
        let encodings = response.headers().get_all(header::CONTENT_ENCODING);
        if encodings.iter().count() > 1
            || encodings
                .iter()
                .any(|value| value.as_bytes() != b"identity")
        {
            return Err(Error::InvalidResponse);
        }
        if response
            .headers()
            .get(header::CONTENT_LENGTH)
            .is_some_and(|value| {
                value
                    .to_str()
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .is_none_or(|length| length > limit as u64)
            })
        {
            return Err(Error::Limit);
        }
        let bytes = Zeroizing::new(
            response
                .body_mut()
                .with_config()
                .limit(limit as u64 + 1)
                .read_to_vec()
                .map_err(|_| Error::Transport)?,
        );
        if bytes.len() > limit {
            return Err(Error::Limit);
        }
        Ok(bytes)
    }
    fn empty(&self, mut response: Response<Body>) -> Result<(), Error> {
        if response.status().as_u16() != 204 {
            return Err(Error::InvalidResponse);
        }
        if !response
            .body_mut()
            .with_config()
            .limit(1)
            .read_to_vec()
            .map_err(|_| Error::Transport)?
            .is_empty()
        {
            return Err(Error::InvalidResponse);
        }
        Ok(())
    }
    fn session_response(
        &self,
        response: Response<Body>,
        status: u16,
    ) -> Result<accounts::Session, Error> {
        let session: accounts::Session = self.json(response, status, SMALL)?;
        let address = session
            .address
            .strip_prefix('@')
            .and_then(|s| s.split_once(':'));
        if !accounts::valid_credential(&session.account_id)
            || !accounts::valid_credential(&session.device_id)
            || !valid_time(session.expires_at)
            || session.device_label.is_empty()
            || session.device_label.len() > 80
            || session.device_label.chars().any(char::is_control)
            || !address.is_some_and(|(name, server)| {
                accounts::valid_username(name) && server == self.server
            })
        {
            return Err(Error::InvalidResponse);
        }
        Ok(session)
    }
    pub fn session(&self) -> Result<accounts::Session, Error> {
        self.session_response(
            self.request(Method::GET, "/client/v0/session", None::<&()>)?,
            200,
        )
    }
    pub fn authorize_device_link(
        &self,
        proof: &sigil_protocol::link::Proof,
    ) -> Result<accounts::Session, Error> {
        let request = sigil_protocol::link::Authorization {
            proof: super::transport::hex(&proof.to_bytes().map_err(|_| Error::Configuration)?),
        };
        let session = self.session_response(
            self.request(Method::POST, "/client/v0/device-links", Some(&request))?,
            201,
        )?;
        let target = &proof.joining.binding;
        if session.account_id != super::transport::hex(&target.account)
            || session.device_id != super::transport::hex(&target.device)
            || session.address != format!("@{}:{}", target.username, target.server)
            || session.device_label != "Linked device"
        {
            return Err(Error::InvalidResponse);
        }
        Ok(session)
    }
    pub fn own_device_link(&self) -> Result<sigil_protocol::link::Proof, Error> {
        let request: sigil_protocol::link::Authorization = self.json(
            self.request(Method::GET, "/client/v0/device-link", None::<&()>)?,
            200,
            4096,
        )?;
        request.parse().map_err(|_| Error::InvalidResponse)
    }
    pub fn cancel_device_link(&self, challenge: &[u8; 32]) -> Result<(), Error> {
        self.empty(self.request(
            Method::DELETE,
            &format!(
                "/client/v0/device-links/{}",
                super::transport::hex(challenge)
            ),
            None::<&()>,
        )?)
    }
    /// Server-reported account inventory; this does not verify encryption keys.
    pub fn devices(&self, after: Option<&str>) -> Result<accounts::DevicePage, Error> {
        if after.is_some_and(|id| !accounts::valid_credential(id)) {
            return Err(Error::Configuration);
        }
        let path = after.map_or_else(
            || "/client/v0/devices".to_owned(),
            |id| format!("/client/v0/devices?after={id}"),
        );
        let page: accounts::DevicePage =
            self.json(self.request(Method::GET, &path, None::<&()>)?, 200, 16384)?;
        if !accounts::valid_credential(&page.account_id)
            || page.devices.len() > accounts::DEVICE_PAGE_SIZE
        {
            return Err(Error::InvalidResponse);
        }
        let mut previous = after.unwrap_or("");
        for device in &page.devices {
            if !accounts::valid_credential(&device.id)
                || device.id.as_str() <= previous
                || device.label.is_empty()
                || device.label.len() > 80
                || device.label.chars().any(char::is_control)
                || !valid_time(device.expires_at)
            {
                return Err(Error::InvalidResponse);
            }
            previous = &device.id;
        }
        if page.next_after.as_ref().is_some_and(|id| {
            page.devices.len() != accounts::DEVICE_PAGE_SIZE
                || page.devices.last().is_none_or(|device| &device.id != id)
        }) {
            return Err(Error::InvalidResponse);
        }
        Ok(page)
    }
    /// Revoke server authorization. A lost self-revocation response cannot be
    /// confirmed by retrying with the now-invalid credential.
    pub fn revoke_device(&self, device: &str) -> Result<(), Error> {
        if !accounts::valid_credential(device) {
            return Err(Error::Configuration);
        }
        self.empty(self.request(
            Method::DELETE,
            &format!("/client/v0/devices/{device}"),
            None::<&()>,
        )?)
    }
    pub fn rotate_credential(&self, credential: &str) -> Result<accounts::Session, Error> {
        if !accounts::valid_credential(credential) || credential == self.credential.as_str() {
            return Err(Error::Configuration);
        }
        let request = accounts::RotateCredential {
            device_credential: credential.into(),
        };
        let result = self.request(Method::PUT, "/client/v0/session", Some(&request));
        drop(Zeroizing::new(request.device_credential));
        self.session_response(result?, 200)
    }
    /// Persist this client's fresh credential and invitation before enrollment.
    pub fn enroll(
        &self,
        invitation: &str,
        label: &str,
        reauthorize: bool,
    ) -> Result<accounts::Session, Error> {
        if !accounts::valid_credential(invitation)
            || label.is_empty()
            || label.len() > 80
            || label.chars().any(char::is_control)
        {
            return Err(Error::Configuration);
        }
        let request = accounts::Enrollment {
            invitation: invitation.into(),
            device_credential: self.credential.to_string(),
            device_label: label.into(),
        };
        let path = if reauthorize {
            "/client/v0/reauthorize"
        } else {
            "/client/v0/enroll"
        };
        // These wire fields contain credentials; zero their owned allocations as well.
        let result = self.request(Method::POST, path, Some(&request));
        let accounts::Enrollment {
            invitation,
            device_credential,
            ..
        } = request;
        drop(Zeroizing::new(invitation));
        drop(Zeroizing::new(device_credential));
        self.session_response(result?, 201)
    }
    pub fn submit(&self, request: &mailbox::Submit) -> Result<mailbox::Receipt, Error> {
        if !accounts::valid_credential(&request.recipient_device)
            || !accounts::valid_credential(&request.message_id)
            || !valid_hex(&request.payload, 32, mailbox::MAX_PAYLOAD_HEX)
            || !valid_time(request.expires_at)
        {
            return Err(Error::Configuration);
        }
        let receipt: mailbox::Receipt = self.json(
            self.request(Method::POST, "/client/v0/messages", Some(request))?,
            202,
            SMALL,
        )?;
        if receipt.sequence <= 0 || receipt.expires_at != request.expires_at {
            return Err(Error::InvalidResponse);
        }
        Ok(receipt)
    }
    pub fn mailbox(&self) -> Result<Vec<mailbox::Delivery>, Error> {
        self.mailbox_after(0)
    }
    pub fn mailbox_after(&self, after: i64) -> Result<Vec<mailbox::Delivery>, Error> {
        if after < 0 {
            return Err(Error::Configuration);
        }
        let path = if after == 0 {
            "/client/v0/mailbox".into()
        } else {
            format!("/client/v0/mailbox?after={after}")
        };
        let deliveries: Vec<mailbox::Delivery> = self.json(
            self.request(Method::GET, &path, None::<&()>)?,
            200,
            MAILBOX_RESPONSE,
        )?;
        if deliveries.len() > 16
            || deliveries
                .windows(2)
                .any(|pair| pair[0].sequence >= pair[1].sequence)
            || deliveries.iter().any(|d| {
                d.sequence <= after
                    || d.origin.as_ref().is_some_and(|origin| {
                        !federation::sender_valid(origin, &self.server)
                            || origin.device != d.sender_device
                    })
                    || !accounts::valid_credential(&d.sender_device)
                    || !accounts::valid_credential(&d.message_id)
                    || !valid_hex(&d.payload, 32, mailbox::MAX_PAYLOAD_HEX)
                    || !valid_time(d.expires_at)
            })
        {
            return Err(Error::InvalidResponse);
        }
        Ok(deliveries)
    }
    pub fn acknowledge_delivery(&self, sequence: i64) -> Result<(), Error> {
        if sequence <= 0 {
            return Err(Error::Configuration);
        }
        self.empty(self.request(
            Method::DELETE,
            &format!("/client/v0/mailbox/{sequence}"),
            None::<&()>,
        )?)
    }
    pub fn allow_mailbox_sender(&self, device: &str) -> Result<(), Error> {
        if !accounts::valid_credential(device) {
            return Err(Error::Configuration);
        }
        self.empty(self.request(
            Method::PUT,
            &format!("/client/v0/mailbox/senders/{device}"),
            None::<&()>,
        )?)
    }
    pub fn prekey_inventory(&self) -> Result<prekeys::PrekeyInventory, Error> {
        let inventory: prekeys::PrekeyInventory = self.json(
            self.request(Method::GET, "/client/v0/prekeys", None::<&()>)?,
            200,
            SMALL,
        )?;
        if inventory.available > 64 {
            return Err(Error::InvalidResponse);
        }
        Ok(inventory)
    }
    pub fn publish_prekey(
        &self,
        id: &str,
        request: &prekeys::PublishPrekey,
    ) -> Result<prekeys::PublishedPrekey, Error> {
        if !accounts::valid_credential(id)
            || !valid_hex(&request.bundle, 3544, 3610)
            || !(3600..=604800).contains(&request.expires_in_seconds)
        {
            return Err(Error::Configuration);
        }
        let receipt: prekeys::PublishedPrekey = self.json(
            self.request(
                Method::PUT,
                &format!("/client/v0/prekeys/{id}"),
                Some(request),
            )?,
            200,
            SMALL,
        )?;
        if receipt.prekey_id != id || !valid_time(receipt.expires_at) {
            return Err(Error::InvalidResponse);
        }
        Ok(receipt)
    }
    pub fn publish_device_binding(
        &self,
        statement: &sigil_protocol::device::Statement,
    ) -> Result<(), Error> {
        let bytes = statement.bytes().map_err(|_| Error::Configuration)?;
        sigil_protocol::device::SignedBinding::from_bytes(&bytes)
            .map_err(|_| Error::Configuration)?;
        self.empty(self.request(Method::PUT, "/client/v0/device-binding", Some(statement))?)
    }
    pub fn device_binding(&self, device: &str) -> Result<sigil_protocol::device::Statement, Error> {
        if !accounts::valid_credential(device) {
            return Err(Error::Configuration);
        }
        let statement: sigil_protocol::device::Statement = self.json(
            self.request(
                Method::GET,
                &format!("/client/v0/devices/{device}/binding"),
                None::<&()>,
            )?,
            200,
            2048,
        )?;
        let bytes = statement.bytes().map_err(|_| Error::InvalidResponse)?;
        let parsed = sigil_protocol::device::SignedBinding::from_bytes(&bytes)
            .map_err(|_| Error::InvalidResponse)?;
        if super::transport::hex(&parsed.binding.device) != device {
            return Err(Error::InvalidResponse);
        }
        Ok(statement)
    }
    pub fn claim_prekey(
        &self,
        device: &str,
        request_id: &str,
    ) -> Result<prekeys::ClaimedPrekey, Error> {
        if !accounts::valid_credential(device) || !accounts::valid_credential(request_id) {
            return Err(Error::Configuration);
        }
        let request = prekeys::ClaimPrekey {
            request_id: request_id.into(),
        };
        let claimed: prekeys::ClaimedPrekey = self.json(
            self.request(
                Method::POST,
                &format!("/client/v0/devices/{device}/prekeys/claim"),
                Some(&request),
            )?,
            200,
            SMALL,
        )?;
        if claimed.device_id != device
            || !accounts::valid_credential(&claimed.prekey_id)
            || !valid_hex(&claimed.bundle, 3544, 3610)
            || !valid_time(claimed.expires_at)
        {
            return Err(Error::InvalidResponse);
        }
        // Signature, expected identity and KEM-key identifier verification remain
        // mandatory at durable handshake acceptance; transport metadata is not trust.
        Ok(claimed)
    }
    pub fn upload_recovery_object(&self, object: &Object) -> Result<(), Error> {
        let id = super::transport::hex(&object.id());
        self.empty(self.request(
            Method::PUT,
            &format!("/client/v0/recovery/objects/{id}"),
            Some(&recovery::PutObject {
                ciphertext: super::transport::hex(object.bytes()),
            }),
        )?)
    }
    pub fn download_recovery_object(&self, id: super::Id) -> Result<Object, Error> {
        let encoded = super::transport::hex(&id);
        let object: recovery::PutObject = self.json(
            self.request(
                Method::GET,
                &format!("/client/v0/recovery/objects/{encoded}"),
                None::<&()>,
            )?,
            200,
            recovery::MAX_BODY,
        )?;
        if !valid_hex(&object.ciphertext, 72, recovery::MAX_OBJECT_BYTES * 2) {
            return Err(Error::InvalidResponse);
        }
        let bytes: Vec<u8> = object
            .ciphertext
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                u8::from_str_radix(
                    std::str::from_utf8(pair).map_err(|_| Error::InvalidResponse)?,
                    16,
                )
                .map_err(|_| Error::InvalidResponse)
            })
            .collect::<Result<_, _>>()?;
        if Sha256::digest(&bytes).as_slice() != id {
            return Err(Error::InvalidResponse);
        }
        Object::from_bytes(bytes).map_err(|_| Error::InvalidResponse)
    }
    pub fn recovery_head(&self) -> Result<recovery::Head, Error> {
        let head: recovery::Head = self.json(
            self.request(Method::GET, "/client/v0/recovery/head", None::<&()>)?,
            200,
            SMALL,
        )?;
        if !valid_head(&head) {
            return Err(Error::InvalidResponse);
        }
        Ok(head)
    }
    pub fn account_storage(&self) -> Result<recovery::StorageStatus, Error> {
        let status: recovery::StorageStatus = self.json(
            self.request(Method::GET, "/client/v0/storage", None::<&()>)?,
            200,
            SMALL,
        )?;
        if status.quota_bytes == 0
            || status.recovery_object_limit == 0
            || status.recovery_objects > status.recovery_object_limit
        {
            return Err(Error::InvalidResponse);
        }
        Ok(status)
    }
    pub fn delete_recovery_objects(&self, request: &recovery::DeleteObjects) -> Result<(), Error> {
        if request.expected_generation == 0
            || request.expected_generation > i64::MAX as u64
            || !accounts::valid_credential(&request.expected_manifest)
            || request.objects.is_empty()
            || request.objects.len() > recovery::MAX_DELETE_OBJECTS
            || request
                .objects
                .iter()
                .any(|id| !accounts::valid_credential(id) || *id == request.expected_manifest)
        {
            return Err(Error::Configuration);
        }
        self.empty(self.request(
            Method::POST,
            "/client/v0/recovery/objects/delete",
            Some(request),
        )?)
    }
    pub fn publish_recovery_head(
        &self,
        request: &recovery::PublishHead,
    ) -> Result<recovery::Head, Error> {
        let generation = request.generation().ok_or(Error::Configuration)?;
        if !accounts::valid_credential(&request.manifest)
            || request.expected_generation >= i64::MAX as u64
            || request
                .expected_manifest
                .as_ref()
                .is_some_and(|id| !accounts::valid_credential(id))
            || (request.expected_generation == 0) != request.expected_manifest.is_none()
        {
            return Err(Error::Configuration);
        }
        let head: recovery::Head = self.json(
            self.request(Method::PUT, "/client/v0/recovery/head", Some(request))?,
            200,
            SMALL,
        )?;
        if !valid_head(&head)
            || head.generation != generation
            || head.manifest.as_deref() != Some(&request.manifest)
            || head.restored_checkpoint
        {
            return Err(Error::InvalidResponse);
        }
        Ok(head)
    }
}
