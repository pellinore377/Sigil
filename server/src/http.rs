//! Routes. `/info`, `/bag` and `/stream` belong to the home role; `/envoy`
//! to the Envoy role. Nothing here logs a client address.

use crate::envoy::Envoy;
use crate::home::Home;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use ed25519_dalek::Verifier as _;
use futures_util::{SinkExt, StreamExt};
use sigil_protocol::wire::Frame;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone)]
pub struct App {
    pub home: Option<Arc<Home>>,
    pub envoy: Option<Arc<Envoy>>,
}

pub fn router(app: App) -> Router {
    Router::new()
        .route("/info", get(info))
        .route("/bag", post(bag))
        .route("/stream", get(stream))
        .route("/envoy", get(envoy_ws))
        .route("/info/{server}", get(info_via_envoy))
        .route("/credential-key", get(credential_key))
        .route("/admin/invite", post(admin_invite))
        .route(
            "/info/{server}/credential-key",
            get(credential_key_via_envoy),
        )
        .with_state(app)
}

async fn info(State(app): State<App>) -> impl IntoResponse {
    match &app.home {
        Some(h) => (StatusCode::OK, h.card.clone()),
        None => (StatusCode::NOT_FOUND, Vec::new()),
    }
}

/// The Envoy fetches a server's card on the client's behalf, so a first
/// contact never reveals the client's address to the server.
async fn info_via_envoy(
    State(app): State<App>,
    axum::extract::Path(server): axum::extract::Path<String>,
) -> impl IntoResponse {
    let Some(envoy) = &app.envoy else {
        return (StatusCode::NOT_FOUND, Vec::new());
    };
    if let Some(home) = &envoy.local_home {
        if home.cfg.hostname == server {
            return (StatusCode::OK, home.card.clone());
        }
    }
    let url = format!("{}/info", envoy.cfg.base_url(&server));
    match reqwest::get(&url).await {
        Ok(r) if r.status().is_success() => (
            StatusCode::OK,
            r.bytes().await.map(|b| b.to_vec()).unwrap_or_default(),
        ),
        _ => (StatusCode::BAD_GATEWAY, Vec::new()),
    }
}

async fn credential_key(State(app): State<App>) -> impl IntoResponse {
    match &app.home {
        Some(h) => match h.tokens.current(&h.store, "credential") {
            Ok(i) => (StatusCode::OK, i.spki.clone()),
            Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, Vec::new()),
        },
        None => (StatusCode::NOT_FOUND, Vec::new()),
    }
}

async fn credential_key_via_envoy(
    State(app): State<App>,
    axum::extract::Path(server): axum::extract::Path<String>,
) -> impl IntoResponse {
    let Some(envoy) = &app.envoy else {
        return (StatusCode::NOT_FOUND, Vec::new());
    };
    if let Some(home) = &envoy.local_home {
        if home.cfg.hostname == server {
            return match home.tokens.current(&home.store, "credential") {
                Ok(i) => (StatusCode::OK, i.spki.clone()),
                Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, Vec::new()),
            };
        }
    }
    let url = format!("{}/credential-key", envoy.cfg.base_url(&server));
    match reqwest::get(&url).await {
        Ok(r) if r.status().is_success() => (
            StatusCode::OK,
            r.bytes().await.map(|b| b.to_vec()).unwrap_or_default(),
        ),
        _ => (StatusCode::BAD_GATEWAY, Vec::new()),
    }
}

/// Operator-only: mint an invite code. Requires the admin token from
/// `<data_dir>/admin.token`, which only the operator can read.
async fn admin_invite(State(app): State<App>, headers: HeaderMap) -> impl IntoResponse {
    let Some(home) = &app.home else {
        return (StatusCode::NOT_FOUND, String::new());
    };
    let expected =
        std::fs::read_to_string(home.cfg.data_dir.join("admin.token")).unwrap_or_default();
    let given = headers
        .get("x-sigil-admin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if expected.trim().is_empty() || given != expected.trim() {
        return (StatusCode::UNAUTHORIZED, String::new());
    }
    let code = hex::encode(rand::random::<[u8; 8]>());
    let r = (|| -> anyhow::Result<()> {
        let w = home.store.db.begin_write()?;
        w.open_table(crate::store::INVITES)?
            .insert(code.as_str(), ())?;
        w.commit()?;
        Ok(())
    })();
    match r {
        Ok(()) => (StatusCode::OK, code),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
    }
}

async fn bag(
    State(app): State<App>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let Some(home) = &app.home else {
        return (StatusCode::NOT_FOUND, Vec::new());
    };
    let envoy: [u8; 32] = headers
        .get("x-sigil-envoy")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| hex::decode(s).ok())
        .and_then(|b| b.try_into().ok())
        .unwrap_or([0; 32]);
    match home.handle_bag(&body, &envoy).await {
        Some(resp) => (StatusCode::OK, resp),
        None => (StatusCode::BAD_REQUEST, Vec::new()),
    }
}

/// Envoy → home delivery stream. Handshake: 32-byte id, 32-byte challenge,
/// 64-byte signature; then frames.
async fn stream(State(app): State<App>, ws: WebSocketUpgrade) -> impl IntoResponse {
    let Some(home) = app.home.clone() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    ws.on_upgrade(move |socket| serve_stream(home, socket))
        .into_response()
}

async fn serve_stream(home: Arc<Home>, socket: WebSocket) {
    let (mut sink, mut source) = socket.split();
    let id: [u8; 32] = match source.next().await {
        Some(Ok(Message::Binary(b))) if b.len() == 32 => b.as_ref().try_into().unwrap(),
        _ => return,
    };
    let challenge: [u8; 32] = rand::random();
    if sink
        .send(Message::Binary(challenge.to_vec().into()))
        .await
        .is_err()
    {
        return;
    }
    let sig: [u8; 64] = match source.next().await {
        Some(Ok(Message::Binary(b))) if b.len() == 64 => b.as_ref().try_into().unwrap(),
        _ => return,
    };
    let Ok(vk) = ed25519_dalek::VerifyingKey::from_bytes(&id) else {
        return;
    };
    if vk
        .verify(&challenge, &ed25519_dalek::Signature::from_bytes(&sig))
        .is_err()
    {
        return;
    }
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Frame>(1024);
    if home.delivery.attach(&home.store, id, tx).is_err() {
        return;
    }
    let first = home.delivery.new_nonce(&id);
    let _ = sink
        .send(Message::Binary(
            Frame::Keepalive { nonce: first }.encode().into(),
        ))
        .await;
    let mut tick = tokio::time::interval(Duration::from_secs(30));
    tick.tick().await;
    loop {
        tokio::select! {
            Some(f) = rx.recv() => {
                if sink.send(Message::Binary(f.encode().into())).await.is_err() { break; }
            }
            _ = tick.tick() => {
                let n = home.delivery.new_nonce(&id);
                if sink.send(Message::Binary(Frame::Keepalive { nonce: n }.encode().into())).await.is_err() { break; }
            }
            msg = source.next() => {
                match msg { Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break, _ => {} }
            }
        }
    }
    home.delivery.detach(&id);
}

async fn envoy_ws(
    State(app): State<App>,
    Query(q): Query<HashMap<String, String>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let Some(envoy) = app.envoy.clone() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let device: Option<[u8; 32]> = q
        .get("device")
        .and_then(|s| hex::decode(s).ok())
        .and_then(|b| b.try_into().ok());
    let Some(device) = device else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    ws.on_upgrade(move |socket| envoy.serve_device(device, socket))
        .into_response()
}
