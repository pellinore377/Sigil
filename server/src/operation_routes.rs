use crate::{
    enrollment::{bearer, now},
    operations::{Action, Configuration, Upload},
    store::{Store, StoreError},
    store_error, with_store, AppState,
};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/admin/v0/maintenance/configuration",
            get(configuration).put(configure),
        )
        .route("/admin/v0/maintenance/prepare", post(prepare))
        .route("/admin/v0/maintenance/actions", get(actions))
        .route(
            "/admin/v0/maintenance/actions/{id}",
            get(status).post(confirm),
        )
        .route("/admin/v0/maintenance/files", get(files).post(upload))
        .route(
            "/admin/v0/maintenance/files/{id}",
            get(upload_status).delete(delete),
        )
        .route(
            "/admin/v0/maintenance/files/{id}/{offset}",
            get(download).put(chunk),
        )
        .layer(tower_http::limit::RequestBodyLimitLayer::new(
            4 * 1024 * 1024,
        ))
}
pub(crate) fn public() -> Router<AppState> {
    Router::new().route("/setup/v0/probe/{id}", get(probe))
}
async fn probe(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match with_store(state, move |s| s.endpoint_probe(&id, now()?)).await {
        Ok(v) => v.into_response(),
        Err(e) => store_error(e),
    }
}
async fn actions(State(state): State<AppState>) -> Response {
    run(state, |s| s.operations()).await
}
async fn upload_status(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    run(state, move |s| s.backup_upload(&id)).await
}
async fn run<T: serde::Serialize + Send + 'static>(
    state: AppState,
    action: impl FnOnce(&mut Store) -> Result<T, StoreError> + Send + 'static,
) -> Response {
    match with_store(state, action).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn configuration(State(state): State<AppState>) -> Response {
    run(state, |s| s.operation_configuration()).await
}
async fn configure(State(state): State<AppState>, Json(value): Json<Configuration>) -> Response {
    run(state, move |s| s.configure_operations(value)).await
}
async fn prepare(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(action): Json<Action>,
) -> Response {
    let credential = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    let bootstrap = state.token.accepts(&credential);
    run(state, move |s| {
        s.prepare_operation(&credential, bootstrap, action, now()?)
    })
    .await
}
async fn status(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    run(state, move |s| s.operation(&id)).await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Confirmation {
    confirm: bool,
}
async fn confirm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(value): Json<Confirmation>,
) -> Response {
    if !value.confirm {
        return store_error(StoreError::Invalid(
            "confirm the displayed maintenance effect",
        ));
    }
    let credential = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    run(state, move |s| {
        s.confirm_operation(&id, &credential, now()?)
    })
    .await
}
async fn files(State(state): State<AppState>) -> Response {
    run(state, |s| s.backup_files()).await
}
async fn upload(State(state): State<AppState>, Json(value): Json<Upload>) -> Response {
    run(state, move |s| s.prepare_backup_upload(value)).await
}
async fn chunk(
    State(state): State<AppState>,
    Path((id, offset)): Path<(String, u64)>,
    bytes: Bytes,
) -> Response {
    run(state, move |s| s.upload_backup_chunk(&id, offset, &bytes)).await
}
async fn download(
    State(state): State<AppState>,
    Path((id, offset)): Path<(String, u64)>,
) -> Response {
    match with_store(state, move |s| s.backup_chunk(&id, offset)).await {
        Ok(v) => (
            [
                ("content-type", "application/octet-stream"),
                (
                    "content-disposition",
                    "attachment; filename=\"sigil-backup.chunk\"",
                ),
            ],
            v,
        )
            .into_response(),
        Err(e) => store_error(e),
    }
}
async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(value): Json<Confirmation>,
) -> Response {
    if !value.confirm {
        return store_error(StoreError::Invalid("confirm backup deletion"));
    }
    match with_store(state, move |s| s.delete_backup(&id)).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => store_error(e),
    }
}
