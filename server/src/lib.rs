#![forbid(unsafe_code)]

mod accounts;
pub mod admin;
mod admin_routes;
mod admin_storage;
mod admission;
mod attachments;
pub mod auth;
pub mod call_config;
mod call_routes;
mod call_store;
mod contacts;
mod device;
pub mod egress;
mod enrollment;
mod federation_admission;
pub mod federation_auth;
pub mod federation_config;
mod federation_lookup;
mod federation_mailbox;
mod federation_outbox;
mod federation_routes;
pub mod federation_status;
mod group_authority;
mod group_operations;
mod group_routes;
mod link;
mod mailbox;
mod maintenance;
mod map_routes;
pub mod maps;
pub mod oidc;
mod oidc_routes;
mod operation_routes;
pub mod operations;
mod prekeys;
mod push;
pub mod push_config;
mod push_delivery;
pub mod push_provider;
mod push_routes;
mod rate;
mod recovery;
pub mod service_config;
mod service_provider;
mod service_routes;
mod storage_budget;
pub mod store;
pub mod web_admin;
mod web_routes;

use auth::AdminToken;
use axum::{
    extract::{rejection::JsonRejection, Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use sigil_protocol::{ApiError, Configure, MAX_ADMIN_BODY};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use store::{Store, StoreError};
use tokio::sync::Semaphore;
use tower_http::{limit::RequestBodyLimitLayer, timeout::TimeoutLayer};

#[derive(Clone)]
struct AppState {
    federation_wake: Arc<tokio::sync::Notify>,
    calls: Arc<call_routes::Runtime>,
    services: Arc<service_routes::Runtime>,
    maps: Arc<map_routes::Runtime>,
    store: Arc<Mutex<Store>>,
    token: AdminToken,
    slots: Arc<Semaphore>,
    database_slot: Arc<Semaphore>,
    lookup_slots: Arc<Semaphore>,
    rate: Arc<Mutex<rate::Limiter>>,
}

pub fn router(store: Store, token: AdminToken) -> Router {
    application(store, token).0
}

/// The serving process polls this maintenance future alongside HTTP serving.
/// Keeping it explicit avoids spawning hidden tasks when constructing a test router.
pub fn router_with_maintenance(
    store: Store,
    token: AdminToken,
) -> (Router, impl std::future::Future<Output = ()>) {
    let (router, state) = application(store, token);
    (router, async move {
        tokio::join!(
            maintenance::run(state.clone()),
            operations::run(state.clone()),
            push_delivery::run(state.clone()),
            federation_routes::run(state.clone()),
            call_routes::run(state.clone()),
            federation_outbox::run(state)
        );
    })
}

fn application(store: Store, token: AdminToken) -> (Router, AppState) {
    let state = AppState {
        federation_wake: Arc::new(tokio::sync::Notify::new()),
        calls: Arc::new(call_routes::Runtime::default()),
        services: Arc::new(service_routes::Runtime::default()),
        maps: Arc::new(map_routes::Runtime::default()),
        store: Arc::new(Mutex::new(store)),
        token,
        slots: Arc::new(Semaphore::new(32)),
        database_slot: Arc::new(Semaphore::new(1)),
        lookup_slots: Arc::new(Semaphore::new(4)),
        rate: Arc::new(Mutex::new(rate::Limiter::new(std::time::Instant::now()))),
    };
    let admin = Router::new()
        .route("/admin/v0/configuration", get(configuration).put(configure))
        .route(
            "/admin/v0/invitations",
            axum::routing::post(enrollment::invite),
        )
        .route(
            "/admin/v0/invitations/{id}",
            axum::routing::delete(enrollment::revoke_invitation),
        )
        .route(
            "/admin/v0/accounts/{id}/reauthorization-invitations",
            axum::routing::post(enrollment::invite_reauthorization),
        )
        .route(
            "/admin/v0/accounts/{id}",
            axum::routing::delete(enrollment::disable_account),
        )
        .route_layer(middleware::from_fn_with_state(state.clone(), authenticate));
    let router = Router::new()
        .merge(web_routes::routes())
        .merge(
            admin_routes::admin()
                .route_layer(middleware::from_fn_with_state(state.clone(), authenticate)),
        )
        .merge(
            oidc_routes::admin()
                .route_layer(middleware::from_fn_with_state(state.clone(), authenticate)),
        )
        .merge(
            operation_routes::routes()
                .route_layer(middleware::from_fn_with_state(state.clone(), authenticate)),
        )
        .merge(
            call_routes::admin()
                .route_layer(middleware::from_fn_with_state(state.clone(), authenticate)),
        )
        .merge(call_routes::client())
        .merge(call_routes::public())
        .merge(
            service_routes::admin()
                .route_layer(middleware::from_fn_with_state(state.clone(), authenticate)),
        )
        .merge(service_routes::client())
        .merge(
            map_routes::admin()
                .route_layer(middleware::from_fn_with_state(state.clone(), authenticate)),
        )
        .merge(map_routes::client())
        .merge(
            group_routes::admin()
                .route_layer(middleware::from_fn_with_state(state.clone(), authenticate)),
        )
        .merge(group_routes::client())
        .merge(group_routes::public())
        .merge(
            federation_routes::admin()
                .route_layer(middleware::from_fn_with_state(state.clone(), authenticate)),
        )
        .merge(federation_routes::public())
        .merge(federation_routes::client())
        .merge(
            push_routes::admin()
                .route_layer(middleware::from_fn_with_state(state.clone(), authenticate)),
        )
        .merge(push_routes::client())
        .merge(link::routes())
        .merge(admin.layer(RequestBodyLimitLayer::new(MAX_ADMIN_BODY)))
        .merge(enrollment::routes().layer(RequestBodyLimitLayer::new(MAX_ADMIN_BODY)))
        .merge(mailbox::routes())
        .merge(admission::routes())
        .merge(contacts::routes())
        .merge(admin_routes::client())
        .merge(oidc_routes::client())
        .merge(oidc_routes::callback())
        .merge(operation_routes::public())
        .merge(device::routes())
        .merge(recovery::routes())
        .merge(attachments::routes())
        .route(
            "/healthz",
            get(|| async { Json(serde_json::json!({"status": "ok"})) }),
        )
        .route("/readyz", get(readiness))
        .route(
            "/versions",
            get(|| async { Json(&sigil_protocol::CAPABILITIES) }),
        )
        .fallback(|| async {
            error(
                StatusCode::NOT_FOUND,
                "not_found",
                "Unknown route or unsupported API version",
            )
        })
        .method_not_allowed_fallback(|| async {
            error(
                StatusCode::METHOD_NOT_ALLOWED,
                "method_not_allowed",
                "Unsupported method",
            )
        })
        .layer(middleware::from_fn_with_state(state.clone(), rate::limit))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(5),
        ))
        .layer(middleware::from_fn_with_state(state.clone(), bounded))
        .layer(middleware::from_fn(security_headers))
        .with_state(state.clone());
    (router, state)
}

async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        header::HeaderValue::from_static("nosniff"),
    );
    response.headers_mut().insert(header::CONTENT_SECURITY_POLICY, header::HeaderValue::from_static("default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self'; worker-src 'self' blob:; object-src 'none'; base-uri 'self'; frame-ancestors 'none'; form-action 'self'"));
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        header::HeaderValue::from_static("no-referrer"),
    );
    response
}

async fn bounded(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let Ok(_permit) = state.slots.try_acquire() else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "busy",
            "Server is busy; retry later",
        );
    };
    next.run(request).await
}

async fn authenticate(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let bootstrap_origin = request.uri().path() == "/admin/v0/policy"
        && enrollment::bearer(request.headers()).is_ok_and(|v| state.token.accepts(&v));
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    if request.headers().contains_key(header::ORIGIN)
        && (origin.is_none()
            || request.headers().get_all(header::ORIGIN).iter().count() != 1
            || !matches!(
                with_store(state.clone(), move |s| {
                    let configured = s.administration_policy()?.public_origin;
                    Ok(origin.as_ref().is_some_and(|v| {
                        configured.as_ref() == Some(v)
                            || (bootstrap_origin
                                && configured.is_none()
                                && admin::origin_url(v).is_ok())
                    }))
                })
                .await,
                Ok(true)
            ))
    {
        return error(
            StatusCode::FORBIDDEN,
            "origin_not_allowed",
            "Browser Admin access is not enabled",
        );
    }
    let credential = request
        .headers()
        .get_all(header::AUTHORIZATION)
        .iter()
        .count()
        == 1;
    let credential = credential
        .then(|| {
            request
                .headers()
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
                .map(str::to_owned)
        })
        .flatten();
    let role = match credential {
        Some(value) if state.token.accepts(&value) => {
            Ok(sigil_protocol::admin::Role::Administrator)
        }
        Some(value) => {
            with_store(state.clone(), move |s| {
                s.admin_role(&value, enrollment::now()?)
            })
            .await
        }
        None => web_routes::authorize(&state, request.headers()).await,
    };
    let Ok(role) = role else {
        let mut response = error(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Admin authentication required",
        );
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            header::HeaderValue::from_static("Bearer"),
        );
        return response;
    };
    if !admin::allowed(role, request.method(), request.uri().path()) {
        return error(
            StatusCode::FORBIDDEN,
            "permission_denied",
            "Administrator permission required",
        );
    }
    next.run(request).await
}

async fn with_store<T: Send + 'static>(
    state: AppState,
    operation: impl FnOnce(&mut Store) -> Result<T, StoreError> + Send + 'static,
) -> Result<T, StoreError> {
    let permit = state
        .database_slot
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| StoreError::Busy)?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let mut store = state.store.lock().map_err(|_| StoreError::InvalidData)?;
        operation(&mut store)
    })
    .await
    .map_err(|_| StoreError::InvalidData)?
}

async fn configuration(State(state): State<AppState>) -> Response {
    match with_store(state, |store| store.configuration()).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => store_error(error),
    }
}

async fn configure(
    State(state): State<AppState>,
    body: Result<Json<Configure>, JsonRejection>,
) -> Response {
    let update = match body {
        Ok(Json(update)) => update,
        Err(rejection) => {
            return error(
                rejection.status(),
                "invalid_request",
                "Expected a valid configuration JSON object within the body limit",
            )
        }
    };
    match with_store(state, |store| store.configure(update)).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => store_error(error),
    }
}

async fn readiness(State(state): State<AppState>) -> Response {
    match with_store(state, |store| store.configuration()).await {
        Ok(configuration) if configuration.settings.is_some() => {
            Json(serde_json::json!({"status": "ready", "scope": "control_plane"})).into_response()
        }
        Ok(_) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "setup_required",
            "Server configuration is required",
        ),
        Err(error) => store_error(error),
    }
}

fn store_error(value: StoreError) -> Response {
    match value {
        StoreError::Forbidden => error(
            StatusCode::FORBIDDEN,
            "sender_not_allowed",
            "Recipient has not authorized this sender",
        ),
        StoreError::Unauthorized => error(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Invalid or expired credential",
        ),
        StoreError::AlreadyExists => error(
            StatusCode::CONFLICT,
            "already_exists",
            "The requested resource already exists",
        ),
        StoreError::NotFound => error(StatusCode::NOT_FOUND, "not_found", "Resource not found"),
        StoreError::Busy => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "busy",
            "Storage is busy; retry later",
        ),
        StoreError::Conflict => error(
            StatusCode::CONFLICT,
            "revision_conflict",
            "Reload configuration before retrying",
        ),
        StoreError::Invalid(message) => error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_configuration",
            message,
        ),
        StoreError::Database(_) | StoreError::InvalidData => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "storage_unavailable",
            "Storage operation failed",
        ),
    }
}

fn error(status: StatusCode, code: &'static str, message: &'static str) -> Response {
    (status, Json(ApiError { code, message })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cancelled_request_keeps_its_blocking_work_bounded() {
        let directory = tempfile::tempdir().unwrap();
        let state = AppState {
            federation_wake: Arc::new(tokio::sync::Notify::new()),
            calls: Arc::new(call_routes::Runtime::default()),
            services: Arc::new(service_routes::Runtime::default()),
            maps: Arc::new(map_routes::Runtime::default()),
            store: Arc::new(Mutex::new(
                Store::open(&directory.path().join("sigil.db")).unwrap(),
            )),
            token: AdminToken::load_or_create(&directory.path().join("admin.token")).unwrap(),
            slots: Arc::new(Semaphore::new(32)),
            lookup_slots: Arc::new(Semaphore::new(4)),
            database_slot: Arc::new(Semaphore::new(1)),
            rate: Arc::new(Mutex::new(rate::Limiter::new(std::time::Instant::now()))),
        };
        let (started, waiting) = tokio::sync::oneshot::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let first_state = state.clone();
        let request = tokio::spawn(async move {
            with_store(first_state, move |_| {
                started.send(()).unwrap();
                blocked.recv().unwrap();
                Ok(())
            })
            .await
        });
        waiting.await.unwrap();
        request.abort();
        assert!(tokio::time::timeout(
            Duration::from_millis(20),
            with_store::<()>(state.clone(), |_| panic!("cancelled waiter ran")),
        )
        .await
        .is_err());
        release.send(()).unwrap();
        let permit = tokio::time::timeout(Duration::from_secs(2), state.database_slot.acquire())
            .await
            .unwrap()
            .unwrap();
        drop(permit);
        assert!(with_store(state, |_| Ok(())).await.is_ok());
    }
}
