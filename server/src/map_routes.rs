use crate::{
    enrollment::{bearer, native_only, now},
    maps,
    store::StoreError,
    store_error, with_store, AppState,
};
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore};
use tower_http::limit::RequestBodyLimitLayer;
struct Catalog {
    tiles: sigil_maps::Tiles,
    assets: sigil_maps::Assets,
}
pub(crate) struct Runtime {
    current: Mutex<Option<(u64, Arc<Catalog>)>>,
    slots: Arc<Semaphore>,
}
impl Default for Runtime {
    fn default() -> Self {
        Self {
            current: Mutex::new(None),
            slots: Arc::new(Semaphore::new(4)),
        }
    }
}
impl Catalog {
    async fn open(
        settings: &maps::Settings,
        slots: Arc<Semaphore>,
    ) -> Result<Arc<Self>, StoreError> {
        let permit = slots.try_acquire_owned().map_err(|_| StoreError::Busy)?;
        let assets = settings.assets.clone();
        let assets = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            sigil_maps::Assets::open(std::path::Path::new(&assets))
        })
        .await
        .map_err(|_| StoreError::Busy)?
        .map_err(|_| StoreError::Invalid("invalid local map assets"))?;
        let tiles = sigil_maps::Tiles::open(std::path::Path::new(&settings.archive))
            .await
            .map_err(|_| StoreError::Invalid("invalid local PMTiles archive"))?;
        Ok(Arc::new(Self { tiles, assets }))
    }
}
impl Runtime {
    async fn catalog(&self, config: maps::Configuration) -> Result<Arc<Catalog>, StoreError> {
        let settings = config.settings.ok_or(StoreError::NotFound)?;
        let mut current = self.current.lock().await;
        if let Some((revision, catalog)) = &*current {
            if *revision == config.revision {
                return Ok(catalog.clone());
            }
        }
        let catalog = Catalog::open(&settings, self.slots.clone()).await?;
        *current = Some((config.revision, catalog.clone()));
        Ok(catalog)
    }
}
pub(crate) fn admin() -> Router<AppState> {
    Router::new()
        .route("/admin/v0/maps", get(configuration).put(configure))
        .layer(RequestBodyLimitLayer::new(8192))
}
pub(crate) fn client() -> Router<AppState> {
    Router::new()
        .route("/client/v0/maps", get(availability))
        .route("/client/v0/maps/style.json", get(style))
        .route("/client/v0/maps/tiles.json", get(metadata))
        .route("/client/v0/maps/tiles/{z}/{x}/{y}", get(tile))
        .route("/client/v0/maps/assets/{*path}", get(asset))
        .route_layer(middleware::from_fn(native_only))
}
async fn configuration(State(state): State<AppState>) -> Response {
    match with_store(state, |store| store.map_configuration()).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn configure(
    State(state): State<AppState>,
    Json(request): Json<maps::Configure>,
) -> Response {
    let catalog = match &request.settings {
        Some(settings) => match Catalog::open(settings, state.maps.slots.clone()).await {
            Ok(v) => Some(v),
            Err(e) => return store_error(e),
        },
        None => None,
    };
    match with_store(state.clone(), move |store| store.configure_maps(request)).await {
        Ok(config) => {
            *state.maps.current.lock().await = catalog.map(|catalog| (config.revision, catalog));
            Json(config).into_response()
        }
        Err(e) => store_error(e),
    }
}
async fn config(state: AppState, headers: &HeaderMap) -> Result<maps::Configuration, StoreError> {
    let credential = bearer(headers)?;
    with_store(state, move |store| {
        store.available_maps(&credential, now()?)
    })
    .await
}
async fn catalog(state: AppState, headers: &HeaderMap) -> Result<Arc<Catalog>, StoreError> {
    let config = config(state.clone(), headers).await?;
    state.maps.catalog(config).await
}
async fn availability(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match config(state, &headers).await {
        Ok(v) => Json(serde_json::json!({"revision":v.revision,"enabled":v.settings.is_some()}))
            .into_response(),
        Err(e) => store_error(e),
    }
}
async fn style(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match catalog(state, &headers).await {
        Ok(v) => (
            [(header::CONTENT_TYPE, "application/json")],
            v.assets.style().to_vec(),
        )
            .into_response(),
        Err(e) => store_error(e),
    }
}
async fn metadata(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let catalog = match catalog(state, &headers).await {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    match catalog.tiles.metadata() {
        Ok(raw) => {
            let mut value = serde_json::json!({"tilejson":"3.0.0","tiles":["/client/v0/maps/tiles/{z}/{x}/{y}"]});
            for key in [
                "minzoom",
                "maxzoom",
                "bounds",
                "center",
                "vector_layers",
                "attribution",
                "name",
                "description",
            ] {
                if let Some(v) = raw.get(key) {
                    value[key] = v.clone();
                }
            }
            Json(value).into_response()
        }
        Err(_) => store_error(StoreError::InvalidData),
    }
}
async fn tile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((z, x, y)): Path<(u8, u32, u32)>,
) -> Response {
    let catalog = match catalog(state, &headers).await {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    match catalog.tiles.tile(z, x, y).await {
        Ok(bytes) if bytes.is_empty() => StatusCode::NO_CONTENT.into_response(),
        Ok(bytes) => {
            let mut response = (
                [(header::CONTENT_TYPE, catalog.tiles.content_type())],
                bytes,
            )
                .into_response();
            if let Some(encoding) = catalog.tiles.encoding() {
                response
                    .headers_mut()
                    .insert(header::CONTENT_ENCODING, encoding.parse().unwrap());
            }
            response
        }
        Err(sigil_maps::Error::Invalid) => {
            store_error(StoreError::Invalid("invalid map coordinate"))
        }
        Err(_) => store_error(StoreError::Busy),
    }
}
async fn asset(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(path): Path<String>,
) -> Response {
    let catalog = match catalog(state.clone(), &headers).await {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    let permit = match state.maps.slots.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => return store_error(StoreError::Busy),
    };
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        catalog.assets.asset(&path)
    })
    .await
    {
        Ok(Ok((bytes, kind))) => ([(header::CONTENT_TYPE, kind)], bytes).into_response(),
        _ => store_error(StoreError::NotFound),
    }
}
