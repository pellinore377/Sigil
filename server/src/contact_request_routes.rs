use crate::{
    enrollment::{bearer, native_only, now},
    store::StoreError,
    store_error, with_store, AppState,
};
use axum::{
    extract::{Path, RawQuery, State},
    http::{HeaderMap, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, put},
    Json, Router,
};
use sigil_protocol::contacts::*;

struct Failure(StoreError);
impl From<StoreError> for Failure {
    fn from(error: StoreError) -> Self {
        Self(error)
    }
}
impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        store_error(self.0)
    }
}
type Reply<T> = Result<Json<T>, Failure>;
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/client/v0/contact-requests", get(list).post(create))
        .route(
            "/client/v0/contact-requests/policy",
            get(policy).put(set_policy),
        )
        .route("/client/v0/contact-requests/blocked", put(block))
        .route(
            "/client/v0/contact-requests/outgoing/{recipient}",
            get(status),
        )
        .route(
            "/client/v0/contact-requests/{id}",
            get(incoming_status).put(resolve),
        )
        .route_layer(middleware::from_fn(native_only))
        .layer(tower_http::limit::RequestBodyLimitLayer::new(4096))
}
async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Reply<RequestPage> {
    let token = bearer(&headers)?;
    let after = query
        .map(|value| {
            value
                .strip_prefix("after=")
                .filter(|value| sigil_protocol::accounts::valid_credential(value))
                .map(str::to_owned)
                .ok_or(StoreError::Invalid("invalid request cursor"))
        })
        .transpose()?;
    Ok(Json(
        with_store(state, move |s| {
            s.contact_requests(&token, after.as_deref(), now()?)
        })
        .await?,
    ))
}
async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<RequestContact>,
) -> Reply<RequestReceipt> {
    let token = bearer(&headers)?;
    Ok(Json(
        with_store(state, move |s| s.request_contact(&token, request, now()?)).await?,
    ))
}
async fn status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(recipient): Path<String>,
) -> Reply<RequestReceipt> {
    let token = bearer(&headers)?;
    Ok(Json(
        with_store(state, move |s| {
            s.contact_request_status(&token, &recipient, now()?)
        })
        .await?,
    ))
}
async fn resolve(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<ResolveRequest>,
) -> Reply<RequestReceipt> {
    let token = bearer(&headers)?;
    Ok(Json(
        with_store(state, move |s| {
            s.resolve_contact_request(&token, &id, request.state, &request.signature, now()?)
        })
        .await?,
    ))
}
async fn policy(State(state): State<AppState>, headers: HeaderMap) -> Reply<RequestPolicy> {
    let token = bearer(&headers)?;
    Ok(Json(
        with_store(state, move |s| s.contact_request_policy(&token, now()?)).await?,
    ))
}
async fn set_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<RequestPolicy>,
) -> Reply<RequestPolicy> {
    let token = bearer(&headers)?;
    Ok(Json(
        with_store(state, move |s| {
            s.set_contact_request_policy(&token, request, now()?)
        })
        .await?,
    ))
}
async fn block(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<BlockContact>,
) -> Result<StatusCode, Failure> {
    let token = bearer(&headers)?;
    with_store(state, move |s| {
        s.block_contact_requests(&token, request, now()?)
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn incoming_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
) -> Reply<RequestReceipt> {
    let token = bearer(&headers)?;
    let signature = query
        .and_then(|q| q.strip_prefix("signature=").map(str::to_owned))
        .ok_or(StoreError::Invalid("missing request signature"))?;
    Ok(Json(
        with_store(state, move |s| {
            s.incoming_contact_status(&token, &id, &signature, now()?)
        })
        .await?,
    ))
}
