use crate::{
    enrollment::{bearer, native_only, now},
    oidc,
    store::{Store, StoreError},
    store_error, with_store, AppState,
};
use axum::{
    extract::{RawQuery, State},
    http::HeaderMap,
    middleware,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use sigil_protocol::oidc::{Finish, Start};
pub(crate) fn admin() -> Router<AppState> {
    Router::new()
        .route("/admin/v0/oidc", get(configuration).put(configure))
        .layer(tower_http::limit::RequestBodyLimitLayer::new(65536))
}
pub(crate) fn client() -> Router<AppState> {
    Router::new()
        .route("/client/v0/oidc/start", post(start))
        .route("/client/v0/oidc/link", post(link))
        .route("/client/v0/oidc/finish", post(finish))
        .route("/client/v0/oidc/bindings", get(bindings).delete(unlink))
        .route_layer(middleware::from_fn(native_only))
        .layer(tower_http::limit::RequestBodyLimitLayer::new(8192))
}
pub(crate) fn callback() -> Router<AppState> {
    Router::new().route("/auth/v0/oidc/callback", get(complete))
}
async fn run<T: serde::Serialize + Send + 'static>(
    state: AppState,
    f: impl FnOnce(&mut Store) -> Result<T, StoreError> + Send + 'static,
) -> Response {
    match with_store(state, f).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn configuration(State(state): State<AppState>) -> Response {
    run(state, |s| s.oidc_configuration()).await
}
async fn configure(State(state): State<AppState>, Json(update): Json<oidc::Configure>) -> Response {
    let permit = match state.lookup_slots.clone().try_acquire_owned() {
        Ok(v) => v,
        Err(_) => return store_error(StoreError::Busy),
    };
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let metadata = oidc::check(&update)?;
        Ok::<_, StoreError>((update, metadata))
    })
    .await;
    match result {
        Ok(Ok((update, metadata))) => run(state, move |s| s.oidc_install(update, metadata)).await,
        Ok(Err(e)) => store_error(e),
        Err(_) => store_error(StoreError::InvalidData),
    }
}
async fn start(State(state): State<AppState>, Json(request): Json<Start>) -> Response {
    run(state, move |s| s.oidc_start(request, None, now()?)).await
}
async fn link(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<Start>,
) -> Response {
    let token = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    run(state, move |s| s.oidc_start(request, Some(&token), now()?)).await
}
async fn finish(State(state): State<AppState>, Json(request): Json<Finish>) -> Response {
    run(state, move |s| s.oidc_finish(request, now()?)).await
}
async fn bindings(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let token = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    run(state, move |s| s.oidc_bindings(&token, now()?)).await
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Unlink {
    issuer: String,
    confirm: bool,
}
async fn unlink(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<Unlink>,
) -> Response {
    if !request.confirm {
        return store_error(StoreError::Invalid("confirm identity provider unlink"));
    }
    let token = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    run(state, move |s| {
        s.unlink_oidc(&token, &request.issuer, now()?)
    })
    .await
}
async fn complete(State(state): State<AppState>, RawQuery(query): RawQuery) -> Response {
    let Some(query) = query.filter(|v| v.len() <= 8192) else {
        return store_error(StoreError::Unauthorized);
    };
    let mut code = None;
    let mut csrf = None;
    let mut failure = false;
    let mut issuer = None;
    let mut seen = std::collections::BTreeSet::new();
    for (key, value) in openidconnect::url::form_urlencoded::parse(query.as_bytes()) {
        if !seen.insert(key.to_string()) {
            return store_error(StoreError::Unauthorized);
        }
        match key.as_ref() {
            "code" if code.is_none() => code = Some(value.into_owned()),
            "state" if csrf.is_none() => csrf = Some(value.into_owned()),
            "error" if !failure => failure = true,
            "iss" => issuer = Some(value.into_owned()),
            "error_description" | "error_uri" => {}
            _ => return store_error(StoreError::Unauthorized),
        }
    }
    let Some(csrf) = csrf else {
        return store_error(StoreError::Unauthorized);
    };
    let permit = match state.lookup_slots.clone().try_acquire_owned() {
        Ok(v) => v,
        Err(_) => return store_error(StoreError::Busy),
    };
    let callback = match with_store(state.clone(), move |s| s.oidc_claim(&csrf, now()?)).await {
        Ok(Some(v)) => v,
        Ok(None) => return finished(None),
        Err(e) => return store_error(e),
    };
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let result = if failure || issuer.as_deref().is_some_and(|i| i != callback.issuer()) {
            Err(StoreError::Unauthorized)
        } else {
            callback.verify(code.as_deref().unwrap_or(""))
        };
        (callback, result)
    })
    .await;
    match result {
        Ok((callback, result)) => {
            match with_store(state, move |s| s.oidc_verified(callback, result, now()?)).await {
                Ok(completion) => finished(completion),
                Err(e) => store_error(e),
            }
        }
        Err(_) => store_error(StoreError::InvalidData),
    }
}
fn finished(completion: Option<oidc::Completion>) -> Response {
    let body=match completion {
        Some(value)=>format!("<!doctype html><title>Sigil</title><p>Continue only on the device where you started signing in.</p><a href=\"sigil://oidc/{}/{}\">Return to Sigil</a><p>If this page is closed before returning, start sign-in again.</p>",value.request_id,value.secret),
        None=>"<!doctype html><title>Sigil</title><p>Authentication was not completed or this callback was already used. Return to Sigil to restart sign-in.</p>".into()
    };
    let mut response = Html(body).into_response();
    response.headers_mut().insert(
        "content-security-policy",
        "default-src 'none'; frame-ancestors 'none'; base-uri 'none'"
            .parse()
            .unwrap(),
    );
    response
        .headers_mut()
        .insert("referrer-policy", "no-referrer".parse().unwrap());
    response
}
