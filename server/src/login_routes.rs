use crate::{
    enrollment::{native_only, now},
    password_login,
    store::{Store, StoreError},
    store_error, with_store, AppState,
};
use axum::{
    extract::{Path, State},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use sigil_protocol::login::PasswordPolicy;
pub(crate) fn public() -> Router<AppState> {
    Router::new()
        .route("/client/v0/login", get(methods))
        .route("/client/v0/login/password", post(login))
        .route_layer(middleware::from_fn(native_only))
        .layer(tower_http::limit::RequestBodyLimitLayer::new(8192))
}
pub(crate) fn admin() -> Router<AppState> {
    Router::new()
        .route("/admin/v0/password-login", get(policy).put(configure))
        .route("/admin/v0/accounts/{id}/password", put(set_password))
        .layer(tower_http::limit::RequestBodyLimitLayer::new(8192))
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
async fn methods(State(state): State<AppState>) -> Response {
    run(state, |s| s.login_methods()).await
}
async fn policy(State(state): State<AppState>) -> Response {
    run(state, |s| s.user_password_policy()).await
}
async fn configure(State(state): State<AppState>, Json(value): Json<PasswordPolicy>) -> Response {
    run(state, move |s| s.configure_user_passwords(value)).await
}
async fn login(
    State(state): State<AppState>,
    Json(value): Json<password_login::Login>,
) -> Response {
    run(state, move |s| s.password_sign_in(value, now()?)).await
}
async fn set_password(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(value): Json<password_login::SetPassword>,
) -> Response {
    run(state, move |s| s.set_user_password(&id, value)).await
}
