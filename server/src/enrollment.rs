use crate::store::StoreError;
use crate::{error, store_error, with_store, AppState};
use axum::{
    extract::{rejection::JsonRejection, Path, RawQuery, Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use sigil_protocol::accounts::{
    Enrollment, InviteRequest, ReauthorizationRequest, RotateCredential,
};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/client/v0/enroll", post(enroll))
        .route("/client/v0/reauthorize", post(reauthorize))
        .route("/client/v0/session", get(session).put(rotate))
        .route("/client/v0/profile", get(profile).put(update_profile))
        .route("/client/v0/devices", get(devices))
        .route("/client/v0/devices/{id}", delete(revoke_device))
        .route("/client/v0/prekeys", get(prekey_inventory))
        .route(
            "/client/v0/prekeys/{id}",
            axum::routing::put(publish_prekey),
        )
        .route("/client/v0/devices/{id}/prekeys/claim", post(claim_prekey))
        .route_layer(middleware::from_fn(native_only))
}
async fn profile(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let token = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    match with_store(state, move |s| s.profile(&token, now()?)).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn update_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(value): Json<sigil_protocol::profile::Profile>,
) -> Response {
    let token = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    match with_store(state, move |s| s.update_profile(&token, value, now()?)).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn devices(
    State(state): State<AppState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Response {
    let credential = match bearer(&headers) {
        Ok(value) => value,
        Err(value) => return store_error(value),
    };
    let after = match query.as_deref() {
        None | Some("") => None,
        Some(value) => match value.strip_prefix("after=") {
            Some(value) if sigil_protocol::accounts::valid_credential(value) => {
                Some(value.to_owned())
            }
            _ => {
                return error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "Invalid device cursor",
                )
            }
        },
    };
    match with_store(state, move |store| {
        store.list_devices(&credential, after.as_deref(), now()?)
    })
    .await
    {
        Ok(value) => Json(value).into_response(),
        Err(value) => store_error(value),
    }
}
pub(crate) async fn native_only(request: Request, next: Next) -> Response {
    if request.headers().contains_key(header::ORIGIN) {
        return error(
            StatusCode::FORBIDDEN,
            "origin_not_allowed",
            "Browser sessions are not enabled",
        );
    }
    next.run(request).await
}
pub(crate) fn now() -> Result<u64, StoreError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_secs())
        .map_err(|_| StoreError::InvalidData)
}
pub(crate) fn bearer(headers: &HeaderMap) -> Result<String, StoreError> {
    if headers.get_all(header::AUTHORIZATION).iter().count() != 1 {
        return Err(StoreError::Unauthorized);
    }
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|v| sigil_protocol::accounts::valid_credential(v))
        .map(str::to_owned)
        .ok_or(StoreError::Unauthorized)
}
fn json<T>(body: Result<Json<T>, JsonRejection>) -> Result<T, StatusCode> {
    body.map(|Json(value)| value)
        .map_err(|failure| failure.status())
}

async fn prekey_inventory(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let credential = match bearer(&headers) {
        Ok(value) => value,
        Err(value) => return store_error(value),
    };
    match with_store(state, move |store| {
        store.prekey_inventory(&credential, now()?)
    })
    .await
    {
        Ok(inventory) => Json(inventory).into_response(),
        Err(value) => store_error(value),
    }
}

async fn publish_prekey(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Result<Json<sigil_protocol::prekeys::PublishPrekey>, JsonRejection>,
) -> Response {
    let credential = match bearer(&headers) {
        Ok(value) => value,
        Err(value) => return store_error(value),
    };
    let request = match json(body) {
        Ok(value) => value,
        Err(status) => return error(status, "invalid_request", "Invalid request JSON"),
    };
    match with_store(state, move |store| {
        store.publish_prekey(&credential, &id, request, now()?)
    })
    .await
    {
        Ok(receipt) => Json(receipt).into_response(),
        Err(value) => store_error(value),
    }
}

async fn claim_prekey(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Result<Json<sigil_protocol::prekeys::ClaimPrekey>, JsonRejection>,
) -> Response {
    let credential = match bearer(&headers) {
        Ok(value) => value,
        Err(value) => return store_error(value),
    };
    let request = match json(body) {
        Ok(value) => value,
        Err(status) => return error(status, "invalid_request", "Invalid request JSON"),
    };
    match with_store(state, move |store| {
        store.claim_prekey(&credential, &id, &request.request_id, now()?)
    })
    .await
    {
        Ok(value) => Json(value).into_response(),
        Err(value) => store_error(value),
    }
}

pub(crate) async fn invite(
    State(state): State<AppState>,
    body: Result<Json<InviteRequest>, JsonRejection>,
) -> Response {
    let request = match json(body) {
        Ok(value) => value,
        Err(status) => return error(status, "invalid_request", "Invalid request JSON"),
    };
    match with_store(state, move |store| store.invite(request, now()?)).await {
        Ok(value) => (StatusCode::CREATED, Json(value)).into_response(),
        Err(value) => store_error(value),
    }
}
pub(crate) async fn revoke_invitation(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    match with_store(state, move |store| store.revoke_invitation(&id)).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(value) => store_error(value),
    }
}
pub(crate) async fn invite_reauthorization(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Result<Json<ReauthorizationRequest>, JsonRejection>,
) -> Response {
    let request = match json(body) {
        Ok(value) => value,
        Err(status) => return error(status, "invalid_request", "Invalid request JSON"),
    };
    match with_store(state, move |store| {
        store.invite_reauthorization(&id, request.expires_in_seconds, now()?)
    })
    .await
    {
        Ok(value) => (StatusCode::CREATED, Json(value)).into_response(),
        Err(value) => store_error(value),
    }
}
pub(crate) async fn disable_account(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    match with_store(state, move |store| store.disable_account(&id)).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(value) => store_error(value),
    }
}
async fn enroll(
    State(state): State<AppState>,
    body: Result<Json<Enrollment>, JsonRejection>,
) -> Response {
    accept_invitation(state, body, false).await
}
async fn reauthorize(
    State(state): State<AppState>,
    body: Result<Json<Enrollment>, JsonRejection>,
) -> Response {
    accept_invitation(state, body, true).await
}
async fn accept_invitation(
    state: AppState,
    body: Result<Json<Enrollment>, JsonRejection>,
    reauthorize: bool,
) -> Response {
    let request = match json(body) {
        Ok(value) => value,
        Err(status) => return error(status, "invalid_request", "Invalid request JSON"),
    };
    match with_store(state, move |store| {
        if reauthorize {
            store.reauthorize(request, now()?)
        } else {
            store.enroll(request, now()?)
        }
    })
    .await
    {
        Ok(value) => (StatusCode::CREATED, Json(value)).into_response(),
        Err(value) => store_error(value),
    }
}
async fn session(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let credential = match bearer(&headers) {
        Ok(value) => value,
        Err(value) => return store_error(value),
    };
    match with_store(state, move |store| store.session(&credential, now()?)).await {
        Ok(value) => Json(value).into_response(),
        Err(value) => store_error(value),
    }
}
async fn rotate(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<RotateCredential>, JsonRejection>,
) -> Response {
    let credential = match bearer(&headers) {
        Ok(value) => value,
        Err(value) => return store_error(value),
    };
    let request = match json(body) {
        Ok(value) => value,
        Err(status) => return error(status, "invalid_request", "Invalid request JSON"),
    };
    match with_store(state, move |store| {
        store.rotate_device(&credential, &request.device_credential, now()?)
    })
    .await
    {
        Ok(value) => Json(value).into_response(),
        Err(value) => store_error(value),
    }
}
async fn revoke_device(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let credential = match bearer(&headers) {
        Ok(value) => value,
        Err(value) => return store_error(value),
    };
    match with_store(state, move |store| {
        store.revoke_device(&credential, &id, now()?)
    })
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(value) => store_error(value),
    }
}
