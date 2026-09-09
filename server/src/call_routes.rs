use crate::{
    call_config,
    call_store::{failure, Snapshot},
    enrollment::{bearer, native_only, now},
    store::StoreError,
    store_error, with_store, AppState,
};
use axum::{
    extract::{RawQuery, State},
    http::HeaderMap,
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use sigil_calls::{forwarder::Forwarder, SignedConnect, SignedRoster};
use std::{
    collections::BTreeSet,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{net::UdpSocket, sync::Mutex};
use tower_http::limit::RequestBodyLimitLayer;

#[derive(Default)]
pub(crate) struct Runtime {
    inner: Mutex<Current>,
}
#[derive(Default)]
struct Current {
    revision: Option<u64>,
    socket: Option<Arc<UdpSocket>>,
    forwarder: Option<Forwarder>,
    ready: bool,
}
impl Current {
    async fn synchronize(&mut self, snapshot: &Snapshot) -> Result<(), StoreError> {
        if self.revision != Some(snapshot.configuration.revision) {
            self.ready = false;
            self.forwarder = None;
            let Some(settings) = &snapshot.configuration.settings else {
                self.socket = None;
                self.revision = Some(snapshot.configuration.revision);
                self.ready = true;
                return Ok(());
            };
            if self
                .socket
                .as_ref()
                .is_none_or(|s| s.local_addr().ok() != Some(settings.bind))
            {
                self.socket = None;
                self.socket = Some(Arc::new(
                    UdpSocket::bind(settings.bind)
                        .await
                        .map_err(|_| StoreError::Busy)?,
                ));
            }
            self.forwarder = Some(
                Forwarder::new(settings.advertised, usize::from(settings.max_calls))
                    .map_err(failure)?,
            );
            self.revision = Some(snapshot.configuration.revision);
        }
        if let Some(forwarder) = &mut self.forwarder {
            let ids: BTreeSet<_> = snapshot.rosters.iter().map(|r| r.roster.call).collect();
            forwarder.retain(&ids);
            for roster in &snapshot.rosters {
                forwarder
                    .install(roster.clone(), snapshot.now)
                    .map_err(failure)?;
            }
        }
        self.ready = true;
        Ok(())
    }
    fn fail(&mut self) {
        self.ready = false;
        self.forwarder = None;
        self.revision = None;
    }
}
pub(crate) fn admin() -> Router<AppState> {
    Router::new()
        .route("/admin/v0/calls", get(configuration).put(configure))
        .route("/admin/v0/calls/status", get(status))
        .layer(RequestBodyLimitLayer::new(8192))
}
pub(crate) fn client() -> Router<AppState> {
    Router::new()
        .route("/client/v0/calls", get(availability).put(publish))
        .route_layer(middleware::from_fn(native_only))
        .layer(RequestBodyLimitLayer::new(16384))
}
pub(crate) fn public() -> Router<AppState> {
    Router::new()
        .route("/calls/v0/connect", post(connect))
        .route(
            "/calls/v0/roster",
            post(advance).layer(RequestBodyLimitLayer::new(16384)),
        )
        .route(
            "/calls/v0/relay",
            post(relay_credentials).layer(RequestBodyLimitLayer::new(2048)),
        )
        .route_layer(middleware::from_fn(native_only))
        .layer(RequestBodyLimitLayer::new(100352))
}
async fn configuration(State(state): State<AppState>) -> Response {
    match with_store(state, |s| s.call_configuration()).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn configure(
    State(state): State<AppState>,
    Json(request): Json<call_config::Configure>,
) -> Response {
    let mut current = state.calls.inner.lock().await;
    let result = with_store(state.clone(), move |s| s.configure_calls(request)).await;
    // Reconcile even if a committed write's response was lost.
    let snapshot = with_store(state.clone(), |s| s.call_snapshot(now()?)).await;
    match snapshot {
        Ok(snapshot) => {
            if let Err(e) = current.synchronize(&snapshot).await {
                current.fail();
                return store_error(e);
            }
        }
        Err(e) => {
            current.fail();
            return store_error(e);
        }
    }
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn availability(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let credential = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    match with_store(state,move|s|{crate::prekeys::authorize(&s.0,&credential,now()?)?;s.call_configuration()}).await {
        Ok(v)=>Json(serde_json::json!({"revision":v.revision,"enabled":v.settings.is_some(),"max_participants":8})).into_response(),Err(e)=>store_error(e)
    }
}
async fn publish(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(roster): Json<SignedRoster>,
) -> Response {
    let credential = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    let mut current = state.calls.inner.lock().await;
    let result = with_store(state.clone(), move |s| {
        s.publish_call(&credential, roster, now()?)
    })
    .await;
    match with_store(state.clone(), |s| s.call_snapshot(now()?)).await {
        Ok(snapshot) => {
            if let Err(e) = current.synchronize(&snapshot).await {
                current.fail();
                return store_error(e);
            }
        }
        Err(e) => {
            current.fail();
            return store_error(e);
        }
    }
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn connect(
    State(state): State<AppState>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    Json(proof): Json<SignedConnect>,
) -> Response {
    if query.is_some() || headers.contains_key(axum::http::header::AUTHORIZATION) {
        return store_error(StoreError::Invalid(
            "call proofs must not include account credentials or queries",
        ));
    }
    match answer(state, proof).await {
        Ok(value) => Json(value).into_response(),
        Err(e) => store_error(e),
    }
}
async fn advance(
    State(state): State<AppState>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    Json(roster): Json<SignedRoster>,
) -> Response {
    if query.is_some() || headers.contains_key(axum::http::header::AUTHORIZATION) {
        return store_error(StoreError::Invalid(
            "call proofs must not include account credentials or queries",
        ));
    }
    match advance_roster(state, roster).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => store_error(error),
    }
}
async fn advance_roster(state: AppState, roster: SignedRoster) -> Result<SignedRoster, StoreError> {
    let mut current = state.calls.inner.lock().await;
    let result = with_store(state.clone(), move |store| {
        store.advance_call(roster, now()?)
    })
    .await;
    let snapshot = match with_store(state.clone(), |store| store.call_snapshot(now()?)).await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            current.fail();
            return Err(error);
        }
    };
    if let Err(error) = current.synchronize(&snapshot).await {
        current.fail();
        return Err(error);
    }
    result
}
async fn answer(state: AppState, proof: SignedConnect) -> Result<sigil_calls::Answer, StoreError> {
    let mut current = state.calls.inner.lock().await;
    let copied = proof.clone();
    let admission = with_store(state.clone(), move |s| s.admit_call(&copied, now()?)).await?;
    if let Err(e) = current.synchronize(&admission.snapshot).await {
        current.fail();
        return Err(e);
    }
    let forwarder = current.forwarder.as_mut().ok_or(StoreError::NotFound)?;
    if !admission.fresh && !forwarder.connected(proof.request.call, proof.request.participant) {
        return Err(StoreError::Conflict);
    }
    forwarder
        .connect(&proof, admission.snapshot.now, Instant::now())
        .map_err(failure)
}
async fn relay_credentials(
    State(state): State<AppState>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    Json(proof): Json<sigil_calls::RelayRequest>,
) -> Response {
    if query.is_some() || headers.contains_key(axum::http::header::AUTHORIZATION) {
        return store_error(StoreError::Invalid(
            "relay proofs must not include account credentials or queries",
        ));
    }
    let result = issue_relay(state, proof).await;
    match result {
        Ok(value) => Json(value).into_response(),
        Err(e) => store_error(e),
    }
}
async fn issue_relay(
    state: AppState,
    proof: sigil_calls::RelayRequest,
) -> Result<Option<sigil_calls::Relay>, StoreError> {
    let participant = proof.participant;
    with_store(state, move |s| s.call_relay(&proof, now()?))
        .await
        .and_then(|(config, expiry, now)| relay(&config, participant, expiry, now))
}
pub(crate) async fn federated(state: AppState, headers: HeaderMap, body: Vec<u8>) -> Response {
    use sigil_protocol::federation::{LookupReply, LookupValue, Service};
    let (hash, service) = match with_store(state.clone(), move |s| {
        s.admit_federation_call(&body, &headers, now()?)
    })
    .await
    {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    let result = match service {
        Service::CallConnect { request } => match serde_json::from_str(&request) {
            Ok(proof) => answer(state, proof)
                .await
                .and_then(|v| serde_json::to_string(&v).map_err(|_| StoreError::InvalidData)),
            Err(_) => Err(StoreError::Invalid("invalid call proof")),
        },
        Service::CallRelay { request } => match serde_json::from_str(&request) {
            Ok(proof) => issue_relay(state, proof)
                .await
                .and_then(|v| serde_json::to_string(&v).map_err(|_| StoreError::InvalidData)),
            Err(_) => Err(StoreError::Invalid("invalid relay proof")),
        },
        Service::CallUpdate { request } => match serde_json::from_str(&request) {
            Ok(proof) => advance_roster(state, proof).await.and_then(|value| {
                serde_json::to_string(&value).map_err(|_| StoreError::InvalidData)
            }),
            Err(_) => Err(StoreError::Invalid("invalid call roster")),
        },
        _ => Err(StoreError::InvalidData),
    };
    match result {
        Ok(value) => Json(LookupReply {
            request_hash: hash,
            value: LookupValue::Service(value),
        })
        .into_response(),
        Err(e) => store_error(e),
    }
}
fn relay(
    config: &call_config::Stored,
    participant: sigil_calls::Id,
    expiry: u64,
    now: u64,
) -> Result<Option<sigil_calls::Relay>, StoreError> {
    use base64ct::{Base64, Encoding};
    let Some(secret) = &config.secret else {
        return Ok(None);
    };
    let settings = config.settings.as_ref().ok_or(StoreError::InvalidData)?;
    let expires = expiry.min(now.checked_add(600).ok_or(StoreError::InvalidData)?);
    let username = format!("{expires}:{}", crate::federation_auth::hex(&participant));
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, secret.as_bytes());
    let credential = Base64::encode_string(ring::hmac::sign(&key, username.as_bytes()).as_ref());
    Ok(Some(sigil_calls::Relay {
        urls: settings.turn_urls.clone(),
        username,
        credential: zeroize::Zeroizing::new(credential),
        expires,
    }))
}
async fn status(State(state): State<AppState>) -> Response {
    let current = state.calls.inner.lock().await;
    let (calls, participants) = current.forwarder.as_ref().map_or((0, 0), Forwarder::counts);
    let dropped = current
        .forwarder
        .as_ref()
        .map_or(0, Forwarder::dropped_packets);
    Json(serde_json::json!({"ready":current.ready,"calls":calls,"participants":participants,"dropped_packets":dropped}))
        .into_response()
}
pub(crate) async fn run(state: AppState) {
    let mut refreshed = Instant::now() - Duration::from_secs(1);
    let mut bytes = [0; 2049];
    loop {
        if refreshed.elapsed() >= Duration::from_millis(250) {
            let mut current = state.calls.inner.lock().await;
            match with_store(state.clone(), |s| s.call_snapshot(now()?)).await {
                Ok(snapshot) => {
                    if current.synchronize(&snapshot).await.is_err() {
                        current.fail();
                    }
                }
                Err(_) => current.fail(),
            }
            refreshed = Instant::now();
        }
        let (socket, tick) = {
            let mut current = state.calls.inner.lock().await;
            let instant = Instant::now();
            let wall = now().unwrap_or(u64::MAX);
            let tick = current.forwarder.as_mut().map(|f| f.tick(wall, instant));
            (current.socket.clone(), tick)
        };
        let (Some(socket), Some(tick)) = (socket, tick) else {
            tokio::time::sleep(Duration::from_millis(50)).await;
            continue;
        };
        for packet in tick.datagrams {
            let _ = socket.send_to(&packet.contents, packet.destination).await;
        }
        tokio::select! {
            received=socket.recv_from(&mut bytes)=>if let Ok((n,source))=received {
                let mut current=state.calls.inner.lock().await;
                if current.socket.as_ref().is_some_and(|s|Arc::ptr_eq(s,&socket)) {
                    if let Some(forwarder)=&mut current.forwarder {forwarder.receive(source,&bytes[..n],Instant::now());}
                }
            },
            _=tokio::time::sleep_until(tick.next.into())=>(),
        }
    }
}
