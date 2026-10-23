//! Page op-log routes over the validated HTTPS client.
use super::*;
use crate::page_sync::{NetError, PageServer};
use sigil_protocol::pages::{self as wire, PageId};

const OPS_RESPONSE: usize = wire::MAX_OPS_PAGE * (2 * wire::MAX_OP_BYTES + 512) + 1024;
const SNAPSHOT_RESPONSE: usize = 2 * wire::MAX_SNAPSHOT_BYTES + 1024;
const PRESENCE_RESPONSE: usize = 64 * (2 * wire::MAX_PRESENCE_BYTES + 256) + 64;

fn net(error: Error) -> NetError {
    match error {
        Error::Transport => NetError::Unreachable,
        Error::Status { code, .. } => NetError::Refused {
            status: code,
            code: String::new(),
        },
        _ => NetError::Invalid,
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn path(id: &PageId, rest: &str) -> String {
    format!("/client/v0/pages/{}{rest}", id.hex())
}
/// A refusal keeps the server's error code so callers can tell `base_before_snapshot` from `wrong_epoch`.
fn refused(mut response: Response<Body>) -> Result<Response<Body>, NetError> {
    if response.status().is_success() {
        return Ok(response);
    }
    #[derive(serde::Deserialize)]
    struct Failure {
        code: String,
    }
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .with_config()
        .limit(SMALL as u64)
        .read_to_vec()
        .unwrap_or_default();
    let code = serde_json::from_slice::<Failure>(&body)
        .map(|f| f.code)
        .unwrap_or_default();
    Err(NetError::Refused { status, code })
}
/// The server's presence body; the protocol's epoch field is not read there yet.
#[derive(serde::Serialize)]
struct ServerPresence<'a> {
    slot: &'a wire::Slot,
    body: &'a wire::Sealed,
}

impl HttpsClient {
    fn page_call(
        &self,
        method: Method,
        path: &str,
        body: Option<Vec<u8>>,
        access: Option<&[u8; 32]>,
        wait: u64,
    ) -> Result<Response<Body>, NetError> {
        let bytes = Zeroizing::new(body.unwrap_or_default());
        let authorization = Zeroizing::new(format!("Bearer {}", self.credential.as_str()));
        let mut value = HeaderValue::from_str(&authorization).map_err(|_| NetError::Invalid)?;
        value.set_sensitive(true);
        let origin = self.api_origin().map_err(net)?;
        let mut builder = Request::builder()
            .method(method)
            .uri(format!("{origin}{path}"))
            .header(header::AUTHORIZATION, value)
            .header(header::ACCEPT, "application/json")
            .header(header::ACCEPT_ENCODING, "identity");
        if !bytes.is_empty() {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
        }
        if let Some(token) = access {
            let token = Zeroizing::new(hex(token));
            let mut value = HeaderValue::from_str(&token).map_err(|_| NetError::Invalid)?;
            value.set_sensitive(true);
            builder = builder.header(wire::ACCESS_HEADER, value);
        }
        let request = builder
            .body(bytes.as_slice())
            .map_err(|_| NetError::Invalid)?;
        self.page_send(request, wait)
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn page_send(&self, request: Request<&[u8]>, wait: u64) -> Result<Response<Body>, NetError> {
        let label = format!("http {} {}", request.method(), request.uri().path());
        let started = crate::clock::Instant::now();
        let outcome = if wait > 0 {
            let extended = Some(Duration::from_secs(wait + 10));
            let request = self
                .agent
                .configure_request(request)
                .timeout_global(extended)
                .timeout_recv_response(extended)
                .build();
            self.agent.run(request)
        } else {
            self.agent.run(request)
        };
        crate::perf::mark(&label, started);
        refused(outcome.map_err(|_| NetError::Unreachable)?)
    }
    #[cfg(target_arch = "wasm32")]
    fn page_send(&self, request: Request<&[u8]>, _wait: u64) -> Result<Response<Body>, NetError> {
        refused(crate::browser_transport::send(request).map_err(net)?)
    }
    fn page_json<T: DeserializeOwned>(
        &self,
        response: Response<Body>,
        limit: usize,
    ) -> Result<T, NetError> {
        self.json(response, 200, limit).map_err(net)
    }
    fn encode(value: &impl Serialize) -> Result<Option<Vec<u8>>, NetError> {
        serde_json::to_vec(value)
            .map(Some)
            .map_err(|_| NetError::Invalid)
    }
}

impl PageServer for HttpsClient {
    fn create(&self, request: &wire::Create) -> Result<wire::Status, NetError> {
        request.validate().map_err(|_| NetError::Invalid)?;
        let response = self.page_call(
            Method::POST,
            "/client/v0/pages",
            Self::encode(request)?,
            None,
            0,
        )?;
        self.page_json(response, SMALL)
    }
    fn append(
        &self,
        id: &PageId,
        access: &[u8; 32],
        request: &wire::Append,
    ) -> Result<wire::Appended, NetError> {
        request.validate().map_err(|_| NetError::Invalid)?;
        let response = self.page_call(
            Method::POST,
            &path(id, "/ops"),
            Self::encode(request)?,
            Some(access),
            0,
        )?;
        self.page_json(response, OPS_RESPONSE)
    }
    fn ops(
        &self,
        id: &PageId,
        access: &[u8; 32],
        after: u64,
        wait: u64,
    ) -> Result<wire::Ops, NetError> {
        let wait = wait.min(wire::MAX_WAIT_SECONDS);
        let query = format!("/ops?after={after}&wait={wait}");
        let response = self.page_call(Method::GET, &path(id, &query), None, Some(access), wait)?;
        self.page_json(response, OPS_RESPONSE)
    }
    fn snapshot(&self, id: &PageId, access: &[u8; 32]) -> Result<wire::Snapshot, NetError> {
        let response =
            self.page_call(Method::GET, &path(id, "/snapshot"), None, Some(access), 0)?;
        self.page_json(response, SNAPSHOT_RESPONSE)
    }
    fn put_snapshot(
        &self,
        id: &PageId,
        access: &[u8; 32],
        request: &wire::PutSnapshot,
    ) -> Result<bool, NetError> {
        request.validate().map_err(|_| NetError::Invalid)?;
        let response = self.page_call(
            Method::PUT,
            &path(id, "/snapshot"),
            Self::encode(request)?,
            Some(access),
            0,
        )?;
        if response.status().as_u16() == 204 {
            self.empty(response).map_err(net)?;
            return Ok(false);
        }
        self.page_json::<wire::Version>(response, SMALL)?;
        Ok(true)
    }
    fn put_presence(
        &self,
        id: &PageId,
        access: &[u8; 32],
        request: &wire::PutPresence,
    ) -> Result<(), NetError> {
        request.validate().map_err(|_| NetError::Invalid)?;
        let body = ServerPresence {
            slot: &request.slot,
            body: &request.body,
        };
        let response = self.page_call(
            Method::PUT,
            &path(id, "/presence"),
            Self::encode(&body)?,
            Some(access),
            0,
        )?;
        self.empty(response).map_err(net)
    }
    fn presence(
        &self,
        id: &PageId,
        access: &[u8; 32],
    ) -> Result<Vec<wire::PresenceEntry>, NetError> {
        let response =
            self.page_call(Method::GET, &path(id, "/presence"), None, Some(access), 0)?;
        self.page_json(response, PRESENCE_RESPONSE)
    }
}
