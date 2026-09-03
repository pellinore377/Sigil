//! A stand-in identity provider for tests: the four endpoints Sigil's OIDC
//! gate touches, one fixed Ed25519 key, and a login page that agrees at
//! once. Anyone who asks is signed in as `--user` (Pocket ID would show a
//! passkey prompt here). Test only: the key is public knowledge.
//!
//!   fake-issuer --listen 127.0.0.1:18470 --client-id sigil --user marlowe
//!   fake-issuer --mint wren        # print an ID token for wren and exit
//!
//! The token is minted for `--client-id` with the running issuer URL as
//! `iss`, so a token from `--mint` verifies against the same process
//! started with the same `--listen`.

use axum::extract::{Form, Query, State};
use axum::response::{IntoResponse, Redirect};
use axum::routing::{get, post};
use axum::Router;
use base64::Engine as _;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const SEED: [u8; 32] = [0x42; 32];

#[derive(Clone)]
struct App {
    issuer: String,
    client_id: String,
    user: String,
    codes: Arc<Mutex<HashMap<String, (String, String)>>>, // code → (challenge, user)
}

fn b64url(b: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

fn signing_key() -> jsonwebtoken::EncodingKey {
    // PKCS#8 v1 wrapping of a raw Ed25519 seed
    let mut der = hex::decode("302e020100300506032b657004220420").unwrap();
    der.extend_from_slice(&SEED);
    jsonwebtoken::EncodingKey::from_ed_der(&der)
}

fn public_jwk() -> serde_json::Value {
    let sk = ed25519_dalek::SigningKey::from_bytes(&SEED);
    serde_json::json!({
        "kty": "OKP", "crv": "Ed25519", "kid": "test-1", "alg": "EdDSA", "use": "sig",
        "x": b64url(sk.verifying_key().as_bytes()),
    })
}

fn mint(issuer: &str, client_id: &str, user: &str) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let claims = serde_json::json!({
        "iss": issuer, "aud": client_id, "sub": format!("sub-{user}"),
        "preferred_username": user, "name": user,
        "iat": now, "exp": now + 600,
    });
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::EdDSA);
    header.kid = Some("test-1".into());
    jsonwebtoken::encode(&header, &claims, &signing_key()).unwrap()
}

async fn discovery(State(app): State<App>) -> impl IntoResponse {
    axum::Json(serde_json::json!({
        "issuer": app.issuer,
        "authorization_endpoint": format!("{}/authorize", app.issuer),
        "token_endpoint": format!("{}/token", app.issuer),
        "jwks_uri": format!("{}/jwks", app.issuer),
        "response_types_supported": ["code"],
        "code_challenge_methods_supported": ["S256"],
        "id_token_signing_alg_values_supported": ["EdDSA"],
    }))
}

async fn jwks() -> impl IntoResponse {
    axum::Json(serde_json::json!({"keys": [public_jwk()]}))
}

async fn authorize(
    State(app): State<App>,
    Query(q): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let get = |k: &str| q.get(k).cloned().unwrap_or_default();
    if get("client_id") != app.client_id || get("code_challenge_method") != "S256" {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "unknown client or no PKCE",
        )
            .into_response();
    }
    let code = b64url(&rand::random::<[u8; 16]>());
    app.codes
        .lock()
        .unwrap()
        .insert(code.clone(), (get("code_challenge"), app.user.clone()));
    let mut to = url::Url::parse(&get("redirect_uri")).expect("redirect_uri");
    to.query_pairs_mut()
        .append_pair("code", &code)
        .append_pair("state", &get("state"));
    Redirect::to(to.as_str()).into_response()
}

async fn token(
    State(app): State<App>,
    Form(f): Form<HashMap<String, String>>,
) -> impl IntoResponse {
    let get = |k: &str| f.get(k).cloned().unwrap_or_default();
    let Some((challenge, user)) = app.codes.lock().unwrap().remove(&get("code")) else {
        return (axum::http::StatusCode::BAD_REQUEST, "unknown code").into_response();
    };
    let expect = b64url(&<sha2::Sha256 as sha2::Digest>::digest(
        get("code_verifier").as_bytes(),
    ));
    if expect != challenge || get("client_id") != app.client_id {
        return (axum::http::StatusCode::BAD_REQUEST, "PKCE mismatch").into_response();
    }
    axum::Json(serde_json::json!({
        "access_token": "unused", "token_type": "Bearer", "expires_in": 600,
        "id_token": mint(&app.issuer, &app.client_id, &user),
    }))
    .into_response()
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |k: &str, d: &str| {
        args.iter()
            .position(|a| a == k)
            .and_then(|i| args.get(i + 1).cloned())
            .unwrap_or_else(|| d.to_string())
    };
    let listen = arg("--listen", "127.0.0.1:18470");
    let issuer = format!("http://{listen}");
    let client_id = arg("--client-id", "sigil");
    if let Some(i) = args.iter().position(|a| a == "--mint") {
        let user = args.get(i + 1).cloned().unwrap_or_else(|| "wren".into());
        println!("{}", mint(&issuer, &client_id, &user));
        return;
    }
    let app = App {
        issuer: issuer.clone(),
        client_id,
        user: arg("--user", "marlowe"),
        codes: Arc::new(Mutex::new(HashMap::new())),
    };
    let router = Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/jwks", get(jwks))
        .route("/authorize", get(authorize))
        .route("/token", post(token))
        .with_state(app);
    let l = tokio::net::TcpListener::bind(&listen).await.expect("bind");
    println!("fake issuer at {issuer}");
    axum::serve(l, router).await.unwrap();
}
