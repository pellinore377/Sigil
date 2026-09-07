use super::*;
use crate::{
    enrollment::{bearer, native_only, now},
    error, store_error, with_store, AppState,
};
use axum::{
    body::Bytes,
    extract::{rejection::JsonRejection, Path, State as App},
    http::{HeaderMap, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use sigil_protocol::attachments::ACCESS_HEADER;
use tower_http::limit::RequestBodyLimitLayer;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/client/v0/attachments/{id}",
            get(status).put(begin).delete(remove),
        )
        .route("/client/v0/attachments/{id}/publish", post(publish))
        .route("/client/v0/attachments/{id}/parts", get(first_parts))
        .route("/client/v0/attachments/{id}/parts/{after}", get(next_parts))
        .layer(RequestBodyLimitLayer::new(sigil_protocol::MAX_ADMIN_BODY))
        .merge(
            Router::new()
                .route(
                    "/client/v0/attachments/{id}/chunks/{index}",
                    get(chunk).put(put_chunk),
                )
                .layer(RequestBodyLimitLayer::new(
                    sigil_crypto::attachment::CHUNK_SIZE + CHUNK_OVERHEAD,
                )),
        )
        .route_layer(middleware::from_fn(native_only))
}
async fn begin(
    App(state): App<AppState>,
    headers: HeaderMap,
    Path(file): Path<String>,
    body: Result<Json<Begin>, JsonRejection>,
) -> Response {
    let credential = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    let request = match body {
        Ok(Json(v)) => v,
        Err(e) => return error(e.status(), "invalid_request", "Invalid attachment request"),
    };
    match with_store(state, move |store| {
        store.begin_attachment(&credential, &file, request, now()?)
    })
    .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn status(
    App(state): App<AppState>,
    headers: HeaderMap,
    Path(file): Path<String>,
) -> Response {
    let credential = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    match with_store(state, move |store| {
        store.attachment_status(&credential, &file, now()?)
    })
    .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn put_chunk(
    App(state): App<AppState>,
    headers: HeaderMap,
    Path((file, index)): Path<(String, u32)>,
    body: Bytes,
) -> Response {
    let credential = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    if headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        != Some("application/octet-stream")
    {
        return error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "invalid_request",
            "Expected binary attachment ciphertext",
        );
    }
    match with_store(state, move |store| {
        store.put_attachment_chunk(&credential, &file, index, &body, now()?)
    })
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => store_error(e),
    }
}
async fn chunk(
    App(state): App<AppState>,
    headers: HeaderMap,
    Path((file, index)): Path<(String, u32)>,
) -> Response {
    let credential = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    if headers.get_all(ACCESS_HEADER).iter().count() != 1 {
        return store_error(StoreError::NotFound);
    }
    let access = match headers.get(ACCESS_HEADER).and_then(|v| v.to_str().ok()) {
        Some(v) => v.to_owned(),
        None => return store_error(StoreError::NotFound),
    };
    match with_store(state, move |store| {
        store.attachment_chunk(&credential, &file, index, &access, now()?)
    })
    .await
    {
        Ok(v) => (
            [(axum::http::header::CONTENT_TYPE, "application/octet-stream")],
            v,
        )
            .into_response(),
        Err(e) => store_error(e),
    }
}
async fn publish(
    App(state): App<AppState>,
    headers: HeaderMap,
    Path(file): Path<String>,
    body: Result<Json<Publish>, JsonRejection>,
) -> Response {
    let credential = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    let request = match body {
        Ok(Json(v)) => v,
        Err(e) => {
            return error(
                e.status(),
                "invalid_request",
                "Invalid attachment publication",
            )
        }
    };
    match with_store(state, move |store| {
        store.publish_attachment(&credential, &file, request, now()?)
    })
    .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn remove(
    App(state): App<AppState>,
    headers: HeaderMap,
    Path(file): Path<String>,
) -> Response {
    let credential = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    match with_store(state, move |store| {
        store.remove_attachment(&credential, &file, now()?)
    })
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => store_error(e),
    }
}
async fn first_parts(
    App(state): App<AppState>,
    headers: HeaderMap,
    Path(file): Path<String>,
) -> Response {
    list(state, headers, file, None).await
}
async fn next_parts(
    App(state): App<AppState>,
    headers: HeaderMap,
    Path((file, after)): Path<(String, u32)>,
) -> Response {
    list(state, headers, file, Some(after)).await
}
async fn list(state: AppState, headers: HeaderMap, file: String, after: Option<u32>) -> Response {
    let credential = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    match with_store(state, move |store| {
        store.attachment_parts(&credential, &file, after, now()?)
    })
    .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
