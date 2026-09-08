use crate::{
    enrollment::{bearer, native_only, now},
    store::{Store, StoreError},
    store_error, with_store, AppState,
};
use axum::{
    extract::{Path, RawQuery, State},
    http::HeaderMap,
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sigil_protocol::admin::*;

pub(crate) fn admin() -> Router<AppState> {
    Router::new()
        .route("/admin/v0/policy", get(policy).put(configure))
        .route("/admin/v0/accounts", get(accounts))
        .route("/admin/v0/accounts/{id}", axum::routing::put(update))
        .route("/admin/v0/invitations", get(invitations))
        .route("/admin/v0/accounts/{id}/devices", get(devices))
        .route(
            "/admin/v0/accounts/{id}/devices/{device}",
            axum::routing::delete(revoke_device),
        )
        .route("/admin/v0/diagnostics", get(diagnostics))
        .route("/admin/v0/setup", get(setup))
        .layer(tower_http::limit::RequestBodyLimitLayer::new(8192))
}
fn cursor(query: Option<&str>) -> Result<Option<String>, StoreError> {
    match query {
        None | Some("") => Ok(None),
        Some(q) => q
            .strip_prefix("after=")
            .filter(|v| sigil_protocol::accounts::valid_credential(v))
            .map(|s| Some(s.to_owned()))
            .ok_or(StoreError::Invalid("invalid cursor")),
    }
}
async fn invitations(State(state): State<AppState>, RawQuery(query): RawQuery) -> Response {
    let after = match cursor(query.as_deref()) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    run(state, move |s| {
        s.admin_invitations(after.as_deref(), now()?)
    })
    .await
}
async fn devices(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
) -> Response {
    let after = match cursor(query.as_deref()) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    run(state, move |s| s.admin_devices(&id, after.as_deref())).await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Confirmation {
    confirm: bool,
}
async fn revoke_device(
    State(state): State<AppState>,
    Path((id, device)): Path<(String, String)>,
    Json(value): Json<Confirmation>,
) -> Response {
    if !value.confirm {
        return store_error(StoreError::Invalid("confirm device revocation"));
    }
    run(state, move |s| s.admin_revoke_device(&id, &device)).await
}
pub(crate) fn client() -> Router<AppState> {
    Router::new()
        .route("/client/v0/discovery", post(discover))
        .route(
            "/client/v0/discovery/preference",
            get(preference).put(set_preference),
        )
        .route_layer(middleware::from_fn(native_only))
        .layer(tower_http::limit::RequestBodyLimitLayer::new(8192))
}
async fn run<T: Serialize + Send + 'static>(
    state: AppState,
    action: impl FnOnce(&mut Store) -> Result<T, StoreError> + Send + 'static,
) -> Response {
    match with_store(state, action).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn policy(State(state): State<AppState>) -> Response {
    run(state, |s| s.administration_policy()).await
}
async fn configure(State(state): State<AppState>, Json(value): Json<Policy>) -> Response {
    run(state, move |s| s.configure_administration(value)).await
}
async fn accounts(State(state): State<AppState>, RawQuery(query): RawQuery) -> Response {
    let after = match cursor(query.as_deref()) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    run(state, move |s| s.admin_accounts(after.as_deref(), now()?)).await
}
async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(value): Json<AccountUpdate>,
) -> Response {
    run(state, move |s| s.admin_update_account(&id, value, now()?)).await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Lookup {
    username: String,
}
async fn discover(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(value): Json<Lookup>,
) -> Response {
    let token = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    run(state, move |s| {
        s.discover_account(&token, &value.username, now()?)
    })
    .await
}
async fn preference(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let token = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    run(state, move |s| s.discovery_preference(&token, None, now()?)).await
}
async fn set_preference(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(value): Json<DiscoveryPreference>,
) -> Response {
    let token = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    run(state, move |s| {
        s.discovery_preference(&token, Some(value), now()?)
    })
    .await
}
async fn diagnostics(State(state): State<AppState>) -> Response {
    run(state, |s| s.admin_diagnostics(now()?)).await
}
async fn setup(State(state): State<AppState>) -> Response {
    run(state,|s| Ok(serde_json::json!({
        "configuration":s.configuration()?, "policy":s.administration_policy()?,
        "push":s.push_configuration()?, "federation":s.federation_configuration()?,
        "calls":s.call_configuration()?, "groups":s.group_configuration()?,
        "maps":s.map_configuration()?, "services":s.service_configuration()?,
        "oidc":s.oidc_configuration()?, "maintenance":s.operation_configuration()?,
        "storage_path_change":"mount a private persistent data directory and restart",
        "https":"configure public_origin, terminate TLS at the reverse proxy, then run the endpoint check"
    }))).await
}
