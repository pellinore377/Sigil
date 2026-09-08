use crate::{
    enrollment::now,
    store::StoreError,
    store_error,
    web_admin::{Claim, Login},
    with_store, AppState,
};
use axum::{
    extract::State,
    http::{header, HeaderMap},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use tower_http::{limit::RequestBodyLimitLayer, services::ServeDir};

const COOKIE: &str = "__Host-sigil-admin";
pub(crate) fn routes() -> Router<AppState> {
    let directory = std::env::var("SIGIL_WEB_DIR")
        .unwrap_or_else(|_| "shared/build/dist/wasmJs/productionExecutable".into());
    Router::new()
        .route(sigil_protocol::discovery::PATH, get(discovery))
        .route("/", get(index))
        .nest_service("/web", ServeDir::new(directory))
        .route("/setup/v0/status", get(status))
        .route("/setup/v0/claim", post(claim))
        .route("/auth/v0/admin/login", post(login))
        .route("/auth/v0/admin/logout", post(logout))
        .route("/auth/v0/admin/finish", post(finish))
        .route("/auth/v0/admin/password-login", post(password_policy))
        .route("/auth/v0/admin/password", post(change_password))
        .route("/auth/v0/admin/avatar", get(avatar))
        .route("/auth/v0/admin/oidc", post(oidc_start))
        .route("/auth/v0/admin/oidc/unlink", post(oidc_unlink))
        .layer(RequestBodyLimitLayer::new(16384))
}
async fn discovery(State(state): State<AppState>) -> Response {
    match with_store(state, |s| {
        let server_name = s
            .configuration()?
            .settings
            .ok_or(StoreError::NotFound)?
            .server_name;
        let api_origin = s
            .administration_policy()?
            .public_origin
            .ok_or(StoreError::NotFound)?;
        Ok(sigil_protocol::discovery::Discovery {
            server_name,
            api_origin,
        })
    })
    .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn index() -> Response {
    let directory = std::env::var("SIGIL_WEB_DIR")
        .unwrap_or_else(|_| "shared/build/dist/wasmJs/productionExecutable".into());
    match tokio::fs::read_to_string(std::path::Path::new(&directory).join("index.html")).await {
        Ok(html) => Html(html).into_response(),
        Err(_) => (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "Admin interface has not been built",
        )
            .into_response(),
    }
}
pub(crate) fn cookie(headers: &HeaderMap) -> Result<String, StoreError> {
    let mut found = None;
    for line in headers.get_all(header::COOKIE) {
        for part in line
            .to_str()
            .map_err(|_| StoreError::Unauthorized)?
            .split(';')
        {
            if let Some(value) = part.trim().strip_prefix(&format!("{COOKIE}=")) {
                if found.is_some() || !sigil_protocol::accounts::valid_credential(value) {
                    return Err(StoreError::Unauthorized);
                }
                found = Some(value.to_owned());
            }
        }
    }
    found.ok_or(StoreError::Unauthorized)
}
fn session_response(token: &str) -> Response {
    let mut response = Json(serde_json::json!({"ok":true})).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        format!("{COOKIE}={token}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=28800")
            .parse()
            .unwrap(),
    );
    response
}
fn origin(headers: &HeaderMap) -> Result<String, StoreError> {
    if headers.get_all(header::ORIGIN).iter().count() != 1
        || headers.get("x-sigil-admin").and_then(|v| v.to_str().ok()) != Some("1")
    {
        return Err(StoreError::Forbidden);
    }
    let value = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .ok_or(StoreError::Forbidden)?;
    crate::admin::origin_url(value)?;
    Ok(value.into())
}
async fn same_origin(state: &AppState, headers: &HeaderMap) -> Result<(), StoreError> {
    let value = origin(headers)?;
    with_store(state.clone(), move |s| {
        if s.administration_policy()?.public_origin.as_deref() != Some(&value) {
            return Err(StoreError::Forbidden);
        }
        Ok(())
    })
    .await
}
pub(crate) async fn authorize(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<sigil_protocol::admin::Role, StoreError> {
    // Fetch does not send Origin on same-origin GET; the custom header forces a preflight cross-origin.
    if headers.contains_key(header::ORIGIN) {
        same_origin(state, headers).await?;
    }
    if headers.get("x-sigil-admin").and_then(|v| v.to_str().ok()) != Some("1") {
        return Err(StoreError::Forbidden);
    }
    let token = cookie(headers)?;
    with_store(state.clone(), move |s| {
        s.web_session(&token, now()?)?;
        Ok(sigil_protocol::admin::Role::Administrator)
    })
    .await
}
async fn status(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let token = if headers.get("x-sigil-admin").and_then(|v| v.to_str().ok()) == Some("1") {
        cookie(&headers).ok()
    } else {
        None
    };
    match with_store(state, move |s| s.web_status(token.as_deref(), now()?)).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn claim(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(value): Json<Claim>,
) -> Response {
    if origin(&headers).as_ref().ok() != Some(&value.public_origin) {
        return store_error(StoreError::Forbidden);
    }
    match with_store(state, move |s| s.web_claim(value, now()?)).await {
        Ok(token) => session_response(&token),
        Err(e) => store_error(e),
    }
}
async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(value): Json<Login>,
) -> Response {
    if let Err(e) = same_origin(&state, &headers).await {
        return store_error(e);
    }
    match with_store(state, move |s| s.web_login(value, now()?)).await {
        Ok(token) => session_response(&token),
        Err(e) => store_error(e),
    }
}
async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(e) = same_origin(&state, &headers).await {
        return store_error(e);
    }
    let Ok(token) = cookie(&headers) else {
        return store_error(StoreError::Unauthorized);
    };
    match with_store(state, move |s| s.web_logout(&token)).await {
        Ok(()) => {
            let mut r = Json(serde_json::json!({"ok":true})).into_response();
            r.headers_mut().insert(
                header::SET_COOKIE,
                format!("{COOKIE}=; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=0")
                    .parse()
                    .unwrap(),
            );
            r
        }
        Err(e) => store_error(e),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Finish {
    username: String,
}
async fn finish(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(value): Json<Finish>,
) -> Response {
    if let Err(e) = same_origin(&state, &headers).await {
        return store_error(e);
    }
    let Ok(token) = cookie(&headers) else {
        return store_error(StoreError::Unauthorized);
    };
    match with_store(state, move |s| {
        s.web_finish_setup(&token, &value.username, now()?)
    })
    .await
    {
        Ok(()) => Json(serde_json::json!({"ok":true})).into_response(),
        Err(e) => store_error(e),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PasswordPolicy {
    enabled: bool,
}
async fn password_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(value): Json<PasswordPolicy>,
) -> Response {
    if let Err(e) = same_origin(&state, &headers).await {
        return store_error(e);
    }
    let Ok(token) = cookie(&headers) else {
        return store_error(StoreError::Unauthorized);
    };
    match with_store(state, move |s| {
        s.web_password_policy(&token, value.enabled, now()?)
    })
    .await
    {
        Ok(()) => Json(serde_json::json!({"ok":true})).into_response(),
        Err(e) => store_error(e),
    }
}
async fn oidc_start(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(e) = same_origin(&state, &headers).await {
        return store_error(e);
    }
    let token = cookie(&headers).ok();
    match with_store(state, move |s| s.web_oidc_start(token.as_deref(), now()?)).await {
        Ok((token, started)) => {
            let mut response = session_response(&token);
            *response.body_mut() = axum::body::Body::from(serde_json::to_vec(&started).unwrap());
            response
        }
        Err(e) => store_error(e),
    }
}
pub(crate) fn oidc_redirect(token: &str) -> Response {
    let mut response = session_response(token);
    *response.status_mut() = axum::http::StatusCode::SEE_OTHER;
    response
        .headers_mut()
        .insert(header::LOCATION, "/".parse().unwrap());
    response
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PasswordChange {
    current: zeroize::Zeroizing<String>,
    replacement: zeroize::Zeroizing<String>,
}
async fn change_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(value): Json<PasswordChange>,
) -> Response {
    if let Err(e) = same_origin(&state, &headers).await {
        return store_error(e);
    }
    let Ok(token) = cookie(&headers) else {
        return store_error(StoreError::Unauthorized);
    };
    match with_store(state, move |s| {
        s.web_change_password(&token, &value.current, &value.replacement, now()?)
    })
    .await
    {
        Ok(()) => Json(serde_json::json!({"ok":true})).into_response(),
        Err(e) => store_error(e),
    }
}
async fn avatar(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(e) = authorize(&state, &headers).await {
        return store_error(e);
    }
    let permit = match state.lookup_slots.clone().try_acquire_owned() {
        Ok(v) => v,
        Err(_) => return store_error(StoreError::Busy),
    };
    let request = match with_store(state, |s| s.web_picture_request()).await {
        Ok(Some(v)) => v,
        Ok(None) => return Json(serde_json::json!({"image":null})).into_response(),
        Err(e) => return store_error(e),
    };
    match tokio::task::spawn_blocking(move || {
        use base64ct::{Base64, Encoding};
        let _permit = permit;
        let response = crate::egress::Policy::new(request.1)
            .map_err(|_| StoreError::Forbidden)?
            .service(
                ureq::http::Request::get(request.0)
                    .body(&[][..])
                    .map_err(|_| StoreError::InvalidData)?,
            )
            .map_err(|_| StoreError::NotFound)?;
        let png = response.body.starts_with(b"\x89PNG\r\n\x1a\n");
        let jpeg = response.body.starts_with(&[0xff, 0xd8, 0xff]);
        if response.status != 200 || response.body.len() > 262144 || !(png || jpeg) {
            return Err(StoreError::NotFound);
        }
        Ok(serde_json::json!({"image":Base64::encode_string(&response.body)}))
    })
    .await
    {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => store_error(e),
        Err(_) => store_error(StoreError::InvalidData),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Unlink {
    password: zeroize::Zeroizing<String>,
}
async fn oidc_unlink(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(value): Json<Unlink>,
) -> Response {
    if let Err(e) = same_origin(&state, &headers).await {
        return store_error(e);
    }
    let Ok(token) = cookie(&headers) else {
        return store_error(StoreError::Unauthorized);
    };
    match with_store(state, move |s| {
        s.web_unlink_oidc(&token, &value.password, now()?)
    })
    .await
    {
        Ok(()) => Json(serde_json::json!({"ok":true})).into_response(),
        Err(e) => store_error(e),
    }
}
