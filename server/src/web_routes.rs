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
/// Lets the Android app use passkeys for this host. `SIGIL_ANDROID_APPS` holds
/// `package=SHA256:FINGERPRINT` pairs separated by commas; unset serves nothing.
async fn asset_links() -> Response {
    let Ok(apps) = std::env::var("SIGIL_ANDROID_APPS") else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    let targets: Vec<_> = apps
        .split(',')
        .filter_map(|pair| pair.trim().split_once('='))
        .filter(|(package, fingerprint)| {
            !package.is_empty()
                && package.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_')
                && fingerprint.bytes().all(|b| b.is_ascii_hexdigit() || b == b':')
        })
        .map(|(package, fingerprint)| {
            serde_json::json!({
                "relation": ["delegate_permission/common.handle_all_urls", "delegate_permission/common.get_login_creds"],
                "target": {"namespace": "android_app", "package_name": package, "sha256_cert_fingerprints": [fingerprint.to_ascii_uppercase()]}
            })
        })
        .collect();
    Json(targets).into_response()
}
pub(crate) fn routes() -> Router<AppState> {
    let directory = std::env::var("SIGIL_WEB_DIR")
        .unwrap_or_else(|_| "shared/build/dist/wasmJs/productionExecutable".into());
    Router::new()
        .route(sigil_protocol::discovery::PATH, get(discovery))
        .route("/.well-known/assetlinks.json", get(asset_links))
        .route("/", get(index))
        .route("/admin", get(index))
        .route("/preview", get(index))
        .route("/messenger", get(index))
        .route("/auth/browser",get(browser_callback))
        .route("/inactive",get(inactive))
        .route("/passkey",get(passkey_page))
        .nest_service("/web", ServeDir::new(directory).precompressed_gzip())
        .route("/setup/v0/status", get(status))
        .route("/setup/v0/claim", post(claim))
        .route("/auth/v0/admin/login", post(login))
        .route("/auth/v0/admin/logout", post(logout))
        .route("/auth/v0/admin/finish", post(finish))
        .route("/auth/v0/admin/password-login", post(password_policy))
        .route("/auth/v0/admin/password", post(change_password))
        .route("/auth/v0/admin/avatar", get(avatar))
        .route("/auth/v0/admin/profile", get(profile).put(update_profile))
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
        Ok(v) => {
            let mut response=Json(v).into_response();
            response.headers_mut().insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*".parse().unwrap());
            response
        },
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
            "Web interface has not been built",
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
    #[serde(default)]
    display_name: Option<String>,
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
        s.web_finish_setup(
            &token,
            &value.username,
            value.display_name.as_deref(),
            now()?,
        )
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
async fn profile(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(e) = authorize(&state, &headers).await {
        return store_error(e);
    }
    let token = match cookie(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    match with_store(state, move |s| s.web_profile(&token, now()?)).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn update_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(value): Json<sigil_protocol::profile::Profile>,
) -> Response {
    if let Err(e) = same_origin(&state, &headers).await {
        return store_error(e);
    }
    let token = match cookie(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    match with_store(state, move |s| s.web_update_profile(&token, value, now()?)).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
pub(crate) fn oidc_redirect(token: &str) -> Response {
    let mut response = session_response(token);
    *response.status_mut() = axum::http::StatusCode::SEE_OTHER;
    response
        .headers_mut()
        .insert(header::LOCATION, "/admin".parse().unwrap());
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
        let response = request
            .1
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

async fn passkey_page()->Html<&'static str> {
    Html(r#"<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"><title>Sigil passkey</title><style>
body{margin:0;min-height:100svh;display:grid;place-items:center;padding:24px;background:#f4f4f4;color:#222;font-family:Georgia,serif}main{max-width:26rem;text-align:center}h1{font-size:2.2rem;font-weight:400;margin:12px 0}p{font-size:1.1rem;line-height:1.5}button{font:inherit;font-size:1.05rem;padding:12px 22px;border:0;border-radius:16px;background:#222;color:#fff;cursor:pointer}@media(prefers-color-scheme:dark){body{background:#141414;color:#eee}button{background:#eee;color:#141414}}
</style></head><body><main><h1>Sigil</h1><p id="passkey-status" role="status">Preparing…</p><button id="passkey-continue" hidden>Continue</button></main><script type="module" src="/web/sigil-passkey.mjs"></script></body></html>"#)
}
async fn inactive()->Html<&'static str> {
    Html(r#"<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"><title>Sigil</title><style>
body{margin:0;min-height:100svh;display:grid;place-items:center;padding:24px;background:#f4f4f4;color:#222;font-family:Georgia,serif}main{max-width:28rem;text-align:center}h1{font-size:2.5rem;font-weight:400;margin:16px 0}p{font-size:1.15rem;line-height:1.5}a{display:inline-block;padding:12px 20px;border-radius:16px;background:#dedede;color:inherit;text-decoration:none}@media(prefers-color-scheme:dark){body{background:#141414;color:#eee}a{background:#333}}
</style></head><body><main><h1>Sigil</h1><p>Sigil is open in another tab.</p><a href="/">Use Sigil here</a></main></body></html>"#)
}
async fn browser_callback()->Html<&'static str> {
    Html(r#"<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"><title>Sigil</title><style>
@font-face{font-family:Sigil;src:url('/web/composeResources/sigil.shared.generated.resources/font/newsreader.ttf')}*{box-sizing:border-box}body{margin:0;min-height:100svh;display:grid;place-items:center;padding:24px;background:#f4f4f4;color:#222;font-family:Sigil,Georgia,serif}main{max-width:28rem;text-align:center}img{width:48px;height:80px;object-fit:contain}h1{font-size:2.5rem;font-weight:400;margin:16px 0}p{font-size:1.15rem;line-height:1.5}a{display:inline-block;padding:12px 20px;border-radius:16px;background:#dedede;color:inherit;text-decoration:none}a:hover{background:#ccc}@media(prefers-color-scheme:dark){body{background:#141414;color:#eee}img{filter:invert(1)}a{background:#333}a:hover{background:#444}}
</style></head><body><main><img src="/web/composeResources/sigil.shared.generated.resources/drawable/sigil_light.svg" alt=""><h1>Sigil</h1><p id="auth-status" role="status">Completing sign-in…</p><a href="/" target="_self">Return to Sigil</a></main><script type="module" src="/web/sigil-callback.mjs"></script></body></html>"#)
}
