use crate::{
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
use sigil_protocol::groups::{Configure, CredentialRequest, Request, MAX_BODY};
use tower_http::limit::RequestBodyLimitLayer;

pub(crate) fn admin() -> Router<AppState> {
    Router::new()
        .route("/admin/v0/groups", get(configuration).put(configure))
        .layer(RequestBodyLimitLayer::new(4096))
}
pub(crate) fn client() -> Router<AppState> {
    Router::new()
        .route("/client/v0/groups/credential", post(issue))
        .route_layer(middleware::from_fn(native_only))
        .layer(RequestBodyLimitLayer::new(4096))
}
pub(crate) fn public() -> Router<AppState> {
    Router::new()
        .route("/groups/v0/authority", get(authority))
        .route("/groups/v0/request", post(request))
        .route_layer(middleware::from_fn(native_only))
        .layer(RequestBodyLimitLayer::new(MAX_BODY))
}

async fn configuration(State(state): State<AppState>) -> Response {
    match with_store(state, |s| s.group_configuration()).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => store_error(error),
    }
}
async fn configure(State(state): State<AppState>, Json(value): Json<Configure>) -> Response {
    match with_store(state, move |s| s.configure_groups(value)).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => store_error(error),
    }
}
fn anonymous(headers: &HeaderMap, query: Option<String>) -> Result<(), StoreError> {
    if headers.contains_key(axum::http::header::AUTHORIZATION)
        || headers.contains_key(axum::http::header::COOKIE)
        || query.is_some()
    {
        return Err(StoreError::Invalid(
            "anonymous group requests cannot carry account credentials or queries",
        ));
    }
    Ok(())
}
async fn authority(
    State(state): State<AppState>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    if let Err(error) = anonymous(&headers, query) {
        return store_error(error);
    }
    match with_store(state, |s| s.group_authority()).await {
        Ok(value) => Json(crate::federation_auth::hex(&value)).into_response(),
        Err(error) => store_error(error),
    }
}
async fn issue(
    State(state): State<AppState>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    Json(value): Json<CredentialRequest>,
) -> Response {
    if query.is_some() {
        return store_error(StoreError::Invalid("group queries are not supported"));
    }
    let credential = match bearer(&headers) {
        Ok(value) => value,
        Err(error) => return store_error(error),
    };
    match with_store(state, move |s| {
        s.issue_group_credential(&credential, value, now()?)
    })
    .await
    {
        Ok(value) => Json(value).into_response(),
        Err(error) => store_error(error),
    }
}
async fn request(
    State(state): State<AppState>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    Json(value): Json<Request>,
) -> Response {
    if let Err(error) = anonymous(&headers, query) {
        return store_error(error);
    }
    match with_store(state, move |s| s.group_request(value, now()?)).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => store_error(error),
    }
}
