use crate::{
    enrollment::{bearer, native_only, now},
    push_config, store_error, with_store, AppState,
};
use axum::{
    extract::State,
    http::HeaderMap,
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use sigil_protocol::push::{Confirm, Disable, Register, MAX_BODY};
use tower_http::limit::RequestBodyLimitLayer;

pub(crate) fn admin() -> Router<AppState> {
    Router::new()
        .route("/admin/v0/push", get(configuration).put(configure))
        .layer(RequestBodyLimitLayer::new(push_config::MAX_BODY))
}
pub(crate) fn client() -> Router<AppState> {
    Router::new()
        .route("/client/v0/push/providers", get(providers))
        .route("/client/v0/push", get(status).put(register).delete(disable))
        .route("/client/v0/push/confirm", post(confirm))
        .route_layer(middleware::from_fn(native_only))
        .layer(RequestBodyLimitLayer::new(MAX_BODY))
}
async fn configuration(State(state): State<AppState>) -> Response {
    match with_store(state, |store| store.push_configuration()).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn configure(
    State(state): State<AppState>,
    Json(request): Json<push_config::Configure>,
) -> Response {
    match with_store(state, move |store| store.configure_push(request)).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn providers(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let credential = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    match with_store(state, move |store| {
        store.push_providers(&credential, now()?)
    })
    .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn status(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let credential = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    match with_store(state, move |store| store.push_status(&credential, now()?)).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<Register>,
) -> Response {
    let credential = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    match with_store(state, move |store| {
        store.register_push(&credential, request, now()?)
    })
    .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn confirm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<Confirm>,
) -> Response {
    let credential = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    match with_store(state, move |store| {
        store.confirm_push(&credential, request, now()?)
    })
    .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn disable(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<Disable>,
) -> Response {
    let credential = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    match with_store(state, move |store| {
        store.disable_push(&credential, request, now()?)
    })
    .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
