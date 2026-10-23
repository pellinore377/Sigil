use crate::{
    enrollment::{bearer, native_only, now},
    error,
    pages::{AppendOp, CreatePage, Label, PageError, Presence, PutSnapshot, Rotate, Token},
    rate::Bucket,
    store::Store,
    store_error, with_store, AppState,
};
use axum::{
    extract::{DefaultBodyLimit, Path, RawQuery, Request, State},
    handler::Handler,
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};
use tokio::sync::broadcast::{self, error::RecvError};
use tower_http::limit::RequestBodyLimitLayer;

pub(crate) const ACCESS: &str = "sigil-page-access";
pub(crate) const MANAGE: &str = "sigil-page-manage";
pub(crate) const MAX_WAIT: u64 = 25;
const SMALL: usize = 2 * crate::pages::MAX_OP + 4096;
const LARGE: usize = 2 * crate::pages::MAX_SNAPSHOT + 4096;
const MAX_BUCKETS: usize = 4096;
const IDLE: Duration = Duration::from_secs(60);

pub(crate) struct Runtime {
    pub(crate) wake: broadcast::Sender<String>,
    appends: Mutex<HashMap<(String, String), (Bucket, Instant)>>,
}
impl Default for Runtime {
    fn default() -> Self {
        Self { wake: broadcast::channel(256).0, appends: Mutex::new(HashMap::new()) }
    }
}
impl Runtime {
    /// Burst 40, then 10 appends a second per page and device.
    pub(crate) fn admit(&self, page: &str, device: &str, now: Instant) -> bool {
        let Ok(mut buckets) = self.appends.lock() else { return false };
        let key = (page.to_owned(), device.to_owned());
        if !buckets.contains_key(&key) && buckets.len() >= MAX_BUCKETS {
            buckets.retain(|_, (_, last)| now.saturating_duration_since(*last) < IDLE);
            if buckets.len() >= MAX_BUCKETS {
                return false;
            }
        }
        let (bucket, last) = buckets.entry(key).or_insert_with(|| (Bucket::new(40, 100, now), now));
        *last = (*last).max(now);
        bucket.take(now).is_ok()
    }
}

/// Page long-polls hold a wait slot and the longer mailbox deadline.
pub(crate) fn long_poll(request: &Request) -> bool {
    request.method() == Method::GET
        && request.uri().path().strip_prefix("/client/v0/pages/").is_some_and(|rest| rest.ends_with("/ops"))
        && request.uri().query().is_some_and(|q| q.split('&').any(|p| p.starts_with("wait=") && p != "wait=0"))
}

pub(crate) struct Failure(PageError);
impl From<PageError> for Failure {
    fn from(error: PageError) -> Self {
        Self(error)
    }
}
impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let conflict = StatusCode::CONFLICT;
        match self.0 {
            PageError::Store(e) => store_error(e),
            PageError::NotFound => error(StatusCode::NOT_FOUND, "not_found", "Page not found"),
            PageError::Removed => error(StatusCode::GONE, "page_removed", "Page was removed"),
            PageError::Denied => error(StatusCode::FORBIDDEN, "page_access_denied", "Page token is not current"),
            PageError::Exists => error(conflict, "already_exists", "Page ID is taken"),
            PageError::WrongEpoch => error(conflict, "wrong_epoch", "Page key epoch is not current"),
            PageError::BaseAhead => error(conflict, "base_beyond_head", "Sequence is beyond the head"),
            PageError::BaseBeforeSnapshot => error(conflict, "base_before_snapshot", "Pull and rebase on the newest snapshot"),
            PageError::Orphans => error(conflict, "snapshot_orphans_op", "A later op has an older base"),
            PageError::HeadMoved => error(conflict, "head_moved", "Head moved; seal the new head"),
            PageError::Compacted => error(StatusCode::GONE, "ops_compacted", "Ops after this point were compacted"),
            PageError::RequestReused => error(conflict, "request_reused", "Request name already holds another batch"),
            PageError::Quota => error(StatusCode::INSUFFICIENT_STORAGE, "quota_exceeded", "Page owner's storage is full"),
            PageError::PageLimit => error(StatusCode::INSUFFICIENT_STORAGE, "page_limit", "Too many live pages"),
            PageError::Busy => error(StatusCode::SERVICE_UNAVAILABLE, "busy", "Too many editors present; retry later"),
            PageError::RateLimited => {
                let mut response = error(StatusCode::TOO_MANY_REQUESTS, "rate_limited", "Page append rate exceeded; retry later");
                response.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
                response
            }
            PageError::Invalid(message) => error(StatusCode::UNPROCESSABLE_ENTITY, "invalid_request", message),
        }
    }
}
type Reply<T> = Result<T, Failure>;

fn token(headers: &HeaderMap, name: &str) -> Reply<Token> {
    if headers.get_all(name).iter().count() != 1 {
        return Err(PageError::Denied.into());
    }
    headers.get(name).and_then(|v| v.to_str().ok()).and_then(Token::parse).ok_or(PageError::Denied.into())
}
fn credential(headers: &HeaderMap) -> Reply<String> {
    bearer(headers).map_err(|e| PageError::Store(e).into())
}
async fn run<T: Send + 'static>(
    state: AppState,
    operation: impl FnOnce(&mut Store, u64) -> Result<T, PageError> + Send + 'static,
) -> Reply<T> {
    match with_store(state, move |s| Ok(now().map(|now| operation(s, now)))).await {
        Ok(Ok(result)) => result.map_err(Failure),
        Ok(Err(e)) | Err(e) => Err(PageError::Store(e).into()),
    }
}

pub(crate) fn routes() -> Router<AppState> {
    let small = || DefaultBodyLimit::max(SMALL);
    Router::new()
        .route("/client/v0/pages", post(create))
        .route("/client/v0/pages/{id}", get(info).delete(remove.layer(small())))
        .route("/client/v0/pages/{id}/ops", get(ops).post(append.layer(small())))
        .route("/client/v0/pages/{id}/snapshot", get(snapshot).put(put_snapshot))
        .route("/client/v0/pages/{id}/snapshots", get(versions))
        .route("/client/v0/pages/{id}/snapshots/{version}", get(version))
        .route("/client/v0/pages/{id}/snapshots/{version}/label", post(label.layer(small())))
        .route("/client/v0/pages/{id}/presence", get(presence).put(put_presence.layer(small())))
        .route("/client/v0/pages/{id}/rotate", post(rotate))
        .layer(DefaultBodyLimit::max(LARGE))
        .layer(RequestBodyLimitLayer::new(LARGE))
        .route_layer(middleware::from_fn(native_only))
}

async fn create(State(state): State<AppState>, headers: HeaderMap, Json(request): Json<CreatePage>) -> Reply<Json<crate::pages::PageInfo>> {
    let credential = credential(&headers)?;
    Ok(Json(run(state, move |s, now| s.page_create(&credential, request, now)).await?))
}
async fn info(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Reply<Json<crate::pages::PageInfo>> {
    let (credential, token) = (credential(&headers)?, token(&headers, ACCESS)?);
    Ok(Json(run(state, move |s, now| s.page_info(&credential, &id, &token, now)).await?))
}
async fn append(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Json(request): Json<AppendOp>) -> Reply<Json<crate::pages::Appended>> {
    let (credential, token) = (credential(&headers)?, token(&headers, ACCESS)?);
    let (page, runtime) = (id.clone(), state.pages.clone());
    let appended = run(state.clone(), move |s, now| {
        s.page_append(&credential, &page, &token, request, now, |device| runtime.admit(&page, device, Instant::now()))
    })
    .await?;
    let _ = state.pages.wake.send(id);
    Ok(Json(appended))
}
/// Answers at once when ops exist after `after`; otherwise waits up to `wait` seconds for one.
async fn ops(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, RawQuery(query): RawQuery) -> Response {
    match wait_ops(state, headers, id, query).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => e.into_response(),
    }
}
async fn wait_ops(state: AppState, headers: HeaderMap, id: String, query: Option<String>) -> Reply<crate::pages::Ops> {
    let (credential, token) = (credential(&headers)?, token(&headers, ACCESS)?);
    let (mut after, mut wait) = (0u64, 0u64);
    for pair in query.as_deref().unwrap_or("").split('&').filter(|v| !v.is_empty()) {
        let number = |v: &str| (!v.is_empty() && v.len() <= 19 && v.bytes().all(|b| b.is_ascii_digit())).then(|| v.parse::<u64>().ok()).flatten();
        match pair.split_once('=') {
            Some(("after", v)) if number(v).is_some() => after = number(v).unwrap_or(0),
            Some(("wait", v)) if number(v).is_some_and(|t| t <= MAX_WAIT) => wait = number(v).unwrap_or(0),
            _ => return Err(PageError::Invalid("invalid ops query").into()),
        }
    }
    let mut wake = state.pages.wake.subscribe();
    // Out of shared wait slots, the reply comes at once.
    let _slot = if wait > 0 { state.wait_slots.clone().try_acquire_owned().ok() } else { None };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(if _slot.is_some() { wait } else { 0 });
    let token = std::sync::Arc::new(token);
    loop {
        let (credential, page, token) = (credential.clone(), id.clone(), token.clone());
        let found = run(state.clone(), move |s, now| s.page_ops(&credential, &page, &token, after, now)).await?;
        if !found.ops.is_empty() || tokio::time::Instant::now() >= deadline {
            return Ok(found);
        }
        loop {
            match tokio::time::timeout_at(deadline, wake.recv()).await {
                Ok(Ok(changed)) if changed == id => break,
                Ok(Ok(_)) => {}
                Ok(Err(RecvError::Lagged(_))) => break,
                Ok(Err(RecvError::Closed)) | Err(_) => return Ok(found),
            }
        }
    }
}
async fn put_snapshot(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Json(request): Json<PutSnapshot>) -> Reply<Response> {
    let (credential, token) = (credential(&headers)?, token(&headers, ACCESS)?);
    Ok(match run(state, move |s, now| s.page_put_snapshot(&credential, &id, &token, request, now)).await? {
        Some(version) => Json(version).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    })
}
async fn snapshot(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Reply<Json<crate::pages::Snapshot>> {
    let (credential, token) = (credential(&headers)?, token(&headers, ACCESS)?);
    Ok(Json(run(state, move |s, now| s.page_snapshot(&credential, &id, &token, None, now)).await?))
}
async fn version(State(state): State<AppState>, headers: HeaderMap, Path((id, version)): Path<(String, u64)>) -> Reply<Json<crate::pages::Snapshot>> {
    let (credential, token) = (credential(&headers)?, token(&headers, ACCESS)?);
    Ok(Json(run(state, move |s, now| s.page_snapshot(&credential, &id, &token, Some(version), now)).await?))
}
async fn versions(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Reply<Json<Vec<crate::pages::Version>>> {
    let (credential, token) = (credential(&headers)?, token(&headers, ACCESS)?);
    Ok(Json(run(state, move |s, now| s.page_versions(&credential, &id, &token, now)).await?))
}
async fn label(State(state): State<AppState>, headers: HeaderMap, Path((id, version)): Path<(String, u64)>, Json(request): Json<Label>) -> Reply<StatusCode> {
    let (credential, token) = (credential(&headers)?, token(&headers, ACCESS)?);
    run(state, move |s, now| s.page_label(&credential, &id, &token, version, request, now)).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn put_presence(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Json(request): Json<Presence>) -> Reply<StatusCode> {
    let (credential, token) = (credential(&headers)?, token(&headers, ACCESS)?);
    run(state, move |s, now| s.page_put_presence(&credential, &id, &token, request, now)).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn presence(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Reply<Json<Vec<crate::pages::PresenceEntry>>> {
    let (credential, token) = (credential(&headers)?, token(&headers, ACCESS)?);
    Ok(Json(run(state, move |s, now| s.page_presence(&credential, &id, &token, now)).await?))
}
async fn rotate(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Json(request): Json<Rotate>) -> Reply<Json<crate::pages::PageInfo>> {
    let (credential, token) = (credential(&headers)?, token(&headers, MANAGE)?);
    let page = id.clone();
    let value = run(state.clone(), move |s, now| s.page_rotate(&credential, &page, &token, request, now)).await?;
    let _ = state.pages.wake.send(id);
    Ok(Json(value))
}
async fn remove(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Reply<StatusCode> {
    let (credential, token) = (credential(&headers)?, token(&headers, MANAGE)?);
    let page = id.clone();
    run(state.clone(), move |s, now| s.page_delete(&credential, &page, &token, now)).await?;
    let _ = state.pages.wake.send(id);
    Ok(StatusCode::NO_CONTENT)
}
