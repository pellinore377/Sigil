use crate::{
    enrollment::{bearer, native_only, now},
    service_config, service_provider,
    store::StoreError,
    store_error, with_store, AppState,
};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use sha2::{Digest, Sha256};
use sigil_protocol::services::{Query, Resolve, Resolved, MAX_BODY};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tokio::sync::Semaphore;
use tower_http::limit::RequestBodyLimitLayer;
use zeroize::Zeroizing;
type Cache = BTreeMap<[u8; 32], (u64, Zeroizing<Vec<u8>>)>;
pub(crate) struct Runtime {
    slots: Arc<Semaphore>,
    cache: Mutex<Cache>,
}
impl Default for Runtime {
    fn default() -> Self {
        Self {
            slots: Arc::new(Semaphore::new(4)),
            cache: Mutex::new(BTreeMap::new()),
        }
    }
}
pub(crate) fn admin() -> Router<AppState> {
    Router::new()
        .route("/admin/v0/services", get(configuration).put(configure))
        .layer(RequestBodyLimitLayer::new(MAX_BODY))
}
pub(crate) fn client() -> Router<AppState> {
    Router::new()
        .route("/client/v0/services", get(catalog))
        .route("/client/v0/services/resolve", post(resolve))
        .route_layer(middleware::from_fn(native_only))
        .layer(RequestBodyLimitLayer::new(MAX_BODY))
}
async fn configuration(State(state): State<AppState>) -> Response {
    match with_store(state, |store| store.service_configuration()).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn configure(
    State(state): State<AppState>,
    Json(request): Json<service_config::Configure>,
) -> Response {
    match with_store(state.clone(), move |store| {
        store.configure_services(request)
    })
    .await
    {
        Ok(v) => {
            if let Ok(mut cache) = state.services.cache.lock() {
                cache.clear();
            }
            Json(v).into_response()
        }
        Err(e) => store_error(e),
    }
}
async fn catalog(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let credential = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    match with_store(state, move |store| {
        store.service_catalog(&credential, now()?)
    })
    .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn resolve(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<Resolve>,
) -> Response {
    let credential = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    let now = match now() {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    let permit = match state.services.slots.clone().try_acquire_owned() {
        Ok(v) => v,
        Err(_) => return store_error(StoreError::Busy),
    };
    let check = request.clone();
    let token = credential.clone();
    let entry = match with_store(state.clone(), move |store| {
        store.prepare_service(&token, &check, now)
    })
    .await
    {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    let cached = matches!(request.query, Query::Define { .. });
    let hash: [u8; 32] =
        match serde_json::to_vec(&(request.revision, &request.provider, &request.query)) {
            Ok(v) => Sha256::digest(v).into(),
            Err(_) => return store_error(StoreError::InvalidData),
        };
    if cached && !request.refresh {
        let bytes = state.services.cache.lock().ok().and_then(|cache| {
            cache
                .get(&hash)
                .filter(|(until, _)| now < *until)
                .map(|(_, bytes)| bytes.clone())
        });
        if let Some(bytes) = bytes {
            return (
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                bytes.to_vec(),
            )
                .into_response();
        }
    }
    let query = request.query;
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        service_provider::resolve(&entry, &query, now)
    })
    .await;
    let resolved = match result {
        Ok(Ok(v)) => v,
        Ok(Err(service_provider::Error::Configuration)) => {
            return crate::error(
                StatusCode::SERVICE_UNAVAILABLE,
                "provider_configuration",
                "Service provider configuration needs attention",
            )
        }
        Ok(Err(service_provider::Error::Limit | service_provider::Error::Invalid)) => {
            return crate::error(
                StatusCode::BAD_GATEWAY,
                "provider_response",
                "Provider returned unsupported content",
            )
        }
        _ => {
            return crate::error(
                StatusCode::BAD_GATEWAY,
                "provider_unavailable",
                "Provider unavailable; retry or send literal text",
            )
        }
    };
    if let Err(e) = with_store(state.clone(), move |store| {
        crate::prekeys::authorize(&store.0, &credential, crate::enrollment::now()?)?;
        service_config::current(&store.0, request.revision)
    })
    .await
    {
        return store_error(e);
    }
    if cached {
        if let (Ok(bytes), Ok(mut cache)) =
            (serde_json::to_vec(&resolved), state.services.cache.lock())
        {
            if bytes.len() <= 64 * 1024 {
                cache.retain(|_, (until, _)| now < *until);
                if cache.len() >= 256 {
                    if let Some(key) = cache
                        .iter()
                        .min_by_key(|(_, entry)| entry.0)
                        .map(|(key, _)| *key)
                    {
                        cache.remove(&key);
                    }
                }
                cache.insert(hash, (now.saturating_add(7 * 86400), Zeroizing::new(bytes)));
            }
        }
    }
    Json::<Resolved>(resolved).into_response()
}
