use crate::{
    enrollment::{bearer, native_only, now},
    store::StoreError,
    store_error, with_store, AppState,
};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, put},
    Json, Router,
};
use sigil_protocol::profile::{ContactProfile, Photo, SetPhoto, ShareProfile};
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
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/client/v0/profile/photo", get(own).put(update))
        .route("/client/v0/profile/shares", put(share))
        .route("/client/v0/profiles/{account}", get(contact))
        .route("/client/v0/profiles/{account}/photo/{hash}", get(photo))
        .route_layer(middleware::from_fn(native_only))
        .layer(tower_http::limit::RequestBodyLimitLayer::new(
            2 * sigil_protocol::profile::MAX_PHOTO + 1024,
        ))
}
async fn own(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Photo>, Failure> {
    let token = bearer(&headers)?;
    Ok(Json(
        with_store(state, move |s| s.profile_photo(&token, now()?)).await?,
    ))
}
async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SetPhoto>,
) -> Result<Json<Photo>, Failure> {
    let token = bearer(&headers)?;
    Ok(Json(
        with_store(state, move |s| s.set_profile_photo(&token, request, now()?)).await?,
    ))
}
async fn share(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ShareProfile>,
) -> Result<StatusCode, Failure> {
    let token = bearer(&headers)?;
    with_store(state, move |s| s.share_profile(&token, request, now()?)).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn contact(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(account): Path<String>,
) -> Result<Json<ContactProfile>, Failure> {
    let token = bearer(&headers)?;
    Ok(Json(
        with_store(state, move |s| s.contact_profile(&token, &account, now()?)).await?,
    ))
}
async fn photo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((account, hash)): Path<(String, String)>,
) -> Result<Response, Failure> {
    let token = bearer(&headers)?;
    let bytes = with_store(state, move |s| {
        s.contact_photo(&token, &account, &hash, now()?)
    })
    .await?;
    Ok((
        [
            ("content-type", "image/jpeg"),
            ("cache-control", "private, no-store"),
            ("x-content-type-options", "nosniff"),
        ],
        bytes,
    )
        .into_response())
}
