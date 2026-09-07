use crate::{enrollment::now, federation_config, store_error, with_store, AppState};
use axum::{
    extract::{Path, RawQuery, State},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use tower_http::limit::RequestBodyLimitLayer;
pub(crate) fn admin() -> Router<AppState> {
    Router::new()
        .route("/admin/v0/federation", get(configuration).put(configure))
        .route("/admin/v0/federation/status", get(status))
        .route(
            "/admin/v0/federation/peers/{server}/status",
            get(peer_status),
        )
        .route("/admin/v0/federation/peers", get(peers))
        .route(
            "/admin/v0/federation/peers/{server}",
            get(peer).put(configure_peer),
        )
        .layer(RequestBodyLimitLayer::new(federation_config::MAX_BODY))
}
async fn status(State(state): State<AppState>) -> Response {
    match with_store(state, |s| s.federation_status()).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => store_error(error),
    }
}
async fn peer_status(State(state): State<AppState>, Path(server): Path<String>) -> Response {
    match with_store(state, move |s| s.federation_peer_status(&server, now()?)).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => store_error(error),
    }
}
pub(crate) fn public() -> Router<AppState> {
    Router::new()
        .route(sigil_protocol::federation::DISCOVERY_PATH, get(discovery))
        .route(
            sigil_protocol::federation::PING_PATH,
            axum::routing::post(ping),
        )
        .route(
            sigil_protocol::federation::DELIVER_PATH,
            axum::routing::post(deliver),
        )
        .route(
            sigil_protocol::federation::LOOKUP_PATH,
            axum::routing::post(lookup).layer(RequestBodyLimitLayer::new(
                sigil_protocol::federation::MAX_LOOKUP_BODY,
            )),
        )
        .layer(RequestBodyLimitLayer::new(
            sigil_protocol::federation::MAX_BODY,
        ))
}
pub(crate) fn client() -> Router<AppState> {
    Router::new()
        .route(
            "/client/v0/federation/senders/{server}/{device}",
            get(sender_permission),
        )
        .route(
            "/client/v0/federation/senders",
            get(senders).put(configure_sender),
        )
        .route("/client/v0/federation/mailbox", get(mailbox))
        .route(
            "/client/v0/federation/lookup",
            axum::routing::post(proxy_lookup).layer(RequestBodyLimitLayer::new(
                sigil_protocol::federation::MAX_LOOKUP_BODY,
            )),
        )
        .route("/client/v0/federation/messages", axum::routing::post(queue))
        .route("/client/v0/federation/messages/{id}", get(outbound))
        .route_layer(axum::middleware::from_fn(crate::enrollment::native_only))
        .layer(RequestBodyLimitLayer::new(
            sigil_protocol::federation::MAX_BODY,
        ))
}
async fn deliver(
    State(state): State<AppState>,
    uri: axum::http::Uri,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if uri.query().is_some() {
        return store_error(crate::store::StoreError::Invalid(
            "federation query is not supported",
        ));
    }
    match with_store(state, move |s| {
        s.receive_federated_message(&body, &headers, now()?)
    })
    .await
    {
        Ok(v) => (axum::http::StatusCode::ACCEPTED, Json(v)).into_response(),
        Err(e) => store_error(e),
    }
}
async fn senders(State(state): State<AppState>, headers: axum::http::HeaderMap) -> Response {
    let token = match crate::enrollment::bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    match with_store(state, move |s| s.federated_senders(&token, now()?)).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn mailbox(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    RawQuery(query): RawQuery,
) -> Response {
    let token = match crate::enrollment::bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    let after = match query.as_deref() {
        None | Some("") => Some(0),
        Some(q) => q
            .strip_prefix("after=")
            .filter(|v| !v.is_empty() && v.len() <= 19 && v.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|v| v.parse::<i64>().ok()),
    };
    let Some(after) = after else {
        return store_error(crate::store::StoreError::Invalid("invalid mailbox cursor"));
    };
    match with_store(state, move |s| s.federated_mailbox(&token, after, now()?)).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn ping(
    State(state): State<AppState>,
    uri: axum::http::Uri,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if uri.query().is_some() {
        return store_error(crate::store::StoreError::Invalid(
            "federation query is not supported",
        ));
    }
    match with_store(state, move |s| s.federation_ping(&body, &headers, now()?)).await {
        Ok(hash) => Json(serde_json::json!({"request_hash":hash})).into_response(),
        Err(e) => store_error(e),
    }
}
async fn configuration(State(state): State<AppState>) -> Response {
    match with_store(state, |s| s.federation_configuration()).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn configure(
    State(state): State<AppState>,
    Json(request): Json<federation_config::Configure>,
) -> Response {
    match with_store(state, move |s| s.configure_federation(request, now()?)).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn peers(State(state): State<AppState>, RawQuery(query): RawQuery) -> Response {
    let after = match query {
        None => None,
        Some(q) => match q.strip_prefix("after=") {
            Some(value) if sigil_protocol::valid_server_name(value) => Some(value.to_owned()),
            _ => return store_error(crate::store::StoreError::Invalid("invalid peer cursor")),
        },
    };
    match with_store(state, move |s| s.federation_peers(after.as_deref())).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn peer(State(state): State<AppState>, Path(server): Path<String>) -> Response {
    match with_store(state, move |s| s.federation_peer(&server)).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn configure_peer(
    State(state): State<AppState>,
    Path(server): Path<String>,
    Json(request): Json<federation_config::ConfigurePeer>,
) -> Response {
    match with_store(state, move |s| {
        s.configure_federation_peer(&server, request)
    })
    .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn discovery(State(state): State<AppState>) -> Response {
    match with_store(state, |s| s.federation_discovery()).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}

/// Four bounded discovery workers. A request cannot trigger DNS for an unknown
/// peer; only operator-allowed peers enter this durable refresh schedule.
pub(crate) async fn run(state: AppState) {
    tokio::join!(
        worker(state.clone()),
        worker(state.clone()),
        worker(state.clone()),
        worker(state)
    );
}
async fn worker(state: AppState) {
    loop {
        let job = with_store(state.clone(), |s| s.claim_federation_refresh(now()?)).await;
        let Ok(Some((config, peer))) = job else {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            continue;
        };
        let previous = peer.clone();
        let revision = config.revision;
        let observed = tokio::task::spawn_blocking(move || {
            let url = format!(
                "https://{}:{}{}",
                peer.server,
                peer.port,
                sigil_protocol::federation::DISCOVERY_PATH
            );
            let request = ureq::http::Request::get(url).body(&[][..]).ok()?;
            let response = config.policy.federation(request).ok()?;
            if response.status != 200
                || response.content_type.as_deref() != Some("application/json")
            {
                return None;
            }
            let value = serde_json::from_slice(&response.body).ok()?;
            crate::federation_auth::validate_discovery(&value, &peer.server, now().ok()?).ok()?;
            Some(value)
        })
        .await
        .ok()
        .flatten();
        let _ = with_store(state.clone(), move |s| {
            s.finish_federation_refresh(revision, &previous, observed, now()?)
        })
        .await;
    }
}
async fn configure_sender(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(request): Json<sigil_protocol::federation::ConfigureSender>,
) -> Response {
    let token = match crate::enrollment::bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    match with_store(state, move |s| {
        s.configure_federation_sender(&token, request, now()?)
    })
    .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn sender_permission(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((server, device)): Path<(String, String)>,
) -> Response {
    let token = match crate::enrollment::bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    match with_store(state, move |s| {
        s.federation_sender_permission(&token, &server, &device, now()?)
    })
    .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn queue(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(request): Json<sigil_protocol::federation::Queue>,
) -> Response {
    let token = match crate::enrollment::bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    match with_store(state, move |s| {
        s.queue_federated_message(&token, request, now()?)
    })
    .await
    {
        Ok(v) => (axum::http::StatusCode::ACCEPTED, Json(v)).into_response(),
        Err(e) => store_error(e),
    }
}
async fn outbound(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let token = match crate::enrollment::bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    match with_store(state, move |s| s.federated_outbound(&token, &id, now()?)).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn lookup(
    State(state): State<AppState>,
    uri: axum::http::Uri,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if uri.query().is_some() {
        return store_error(crate::store::StoreError::Invalid(
            "federation query is not supported",
        ));
    }
    match with_store(state, move |s| {
        s.receive_federation_lookup(&body, &headers, now()?)
    })
    .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn proxy_lookup(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(request): Json<sigil_protocol::federation::ProxyLookup>,
) -> Response {
    let token = match crate::enrollment::bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    let permit = match state.lookup_slots.clone().try_acquire_owned() {
        Ok(v) => v,
        Err(_) => return store_error(crate::store::StoreError::Busy),
    };
    let proxy = match with_store(state.clone(), move |s| {
        s.prepare_federation_lookup(&token, request, now()?)
    })
    .await
    {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    // The permit stays with the actual blocking HTTPS operation after an HTTP
    // timeout/cancellation. Retry uses the client's persisted claim identifier.
    let outcome = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let result = proxy.send();
        (proxy, result)
    })
    .await;
    let Ok((proxy, result)) = outcome else {
        return crate::error(
            axum::http::StatusCode::BAD_GATEWAY,
            "lookup_failed",
            "Remote lookup failed",
        );
    };
    match result {
        Ok(value) => match with_store(state, move |s| {
            s.finish_federation_lookup(&proxy, value, now()?)
        })
        .await
        {
            Ok(v) => Json(v).into_response(),
            Err(e) => store_error(e),
        },
        Err(failure) => {
            use crate::federation_lookup::Failure;
            let (status, retry) = match failure {
                Failure::Transport => (502, 0),
                Failure::Remote { status, retry_at } => (
                    match status {
                        400 | 404 | 409 | 413 | 422 | 429 | 503 => status,
                        401 | 403 => 403,
                        _ => 502,
                    },
                    retry_at,
                ),
            };
            let status = axum::http::StatusCode::from_u16(status)
                .unwrap_or(axum::http::StatusCode::BAD_GATEWAY);
            let mut response = crate::error(status, "lookup_failed", "Remote lookup failed");
            if status == axum::http::StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                let seconds = retry.saturating_sub(now().unwrap_or(0)).max(
                    if status == axum::http::StatusCode::TOO_MANY_REQUESTS {
                        60
                    } else {
                        5
                    },
                );
                if let Ok(value) = axum::http::HeaderValue::from_str(&seconds.to_string()) {
                    response
                        .headers_mut()
                        .insert(axum::http::header::RETRY_AFTER, value);
                }
            }
            response
        }
    }
}
