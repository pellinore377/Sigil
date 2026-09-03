//! Signing in through the server's identity provider (design B24). The
//! server's card says registration is gated by OIDC and `/oidc` names the
//! issuer and client id; from there the engine runs the authorisation code
//! flow with PKCE as a public client: it opens the provider's login page
//! in the browser (the app does the opening; a phone has no xdg-open),
//! takes the code back on a loopback listener, exchanges it for an ID
//! token, and keeps that token in memory until `account.create` presents
//! it to the server in the invite code's place.
//!
//! The provider learns that a login happened. It never sees a key, a
//! conversation, or the name the person then picks.

use base64::Engine as _;
use serde_json::{json, Value};
use sha2::Digest;
use std::sync::Mutex;
use std::time::Duration;

/// The loopback port the provider redirects to. Registered at the provider
/// as `http://127.0.0.1:44713/callback`; if it is taken the next free one
/// is used, which needs the port wildcarded there (`http://127.0.0.1:*/callback`).
pub const CALLBACK_PORT: u16 = 44713;

/// What a finished login leaves behind for `account.create`.
#[derive(Clone, Debug)]
pub struct Pending {
    pub server: String,
    pub id_token: String,
}

static PENDING: Mutex<Option<Pending>> = Mutex::new(None);

pub fn take(server: &str) -> Option<String> {
    let g = PENDING.lock().unwrap();
    g.as_ref()
        .filter(|p| p.server == server)
        .map(|p| p.id_token.clone())
}

pub fn clear() {
    *PENDING.lock().unwrap() = None;
}

fn http(proxy: Option<&str>) -> anyhow::Result<reqwest::Client> {
    let mut b = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .no_proxy();
    if let Some(px) = proxy.filter(|p| !p.is_empty()) {
        b = b.proxy(reqwest::Proxy::all(format!("socks5h://{px}"))?);
    }
    Ok(b.build()?)
}

/// `{issuer, client_id}` from the server, or None when it has no gate.
pub async fn server_info(base: &str, proxy: Option<&str>) -> anyhow::Result<Option<Value>> {
    let r = http(proxy)?
        .get(format!("{}/oidc", base.trim_end_matches('/')))
        .send()
        .await?;
    if r.status().as_u16() == 404 {
        return Ok(None);
    }
    Ok(Some(r.error_for_status()?.json().await?))
}

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn random_token() -> String {
    let bytes: [u8; 32] = rand::random();
    b64url(&bytes)
}

/// Start the flow. Returns the URL to open; the rest happens in the
/// background and ends in an `oidc.state` event (`done` with the name the
/// provider suggests, or `failed` with a reason).
pub async fn start(
    hub: crate::ipc::hub::Hub,
    server: String,
    issuer: String,
    client_id: String,
    proxy: Option<String>,
) -> anyhow::Result<String> {
    let issuer = issuer.trim_end_matches('/').to_string();
    let client = http(proxy.as_deref())?;
    let disc: Value = client
        .get(format!("{issuer}/.well-known/openid-configuration"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let auth_ep = disc["authorization_endpoint"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("the provider publishes no authorization endpoint"))?
        .to_string();
    let token_ep = disc["token_endpoint"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("the provider publishes no token endpoint"))?
        .to_string();

    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", CALLBACK_PORT)).await {
        Ok(l) => l,
        Err(_) => tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?,
    };
    let port = listener.local_addr()?.port();
    let redirect = format!("http://127.0.0.1:{port}/callback");

    let state = random_token();
    let verifier = format!("{}{}", random_token(), random_token());
    let challenge = b64url(&sha2::Sha256::digest(verifier.as_bytes()));
    let mut url = url::Url::parse(&auth_ep)?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &client_id)
        .append_pair("redirect_uri", &redirect)
        .append_pair("scope", "openid profile")
        .append_pair("state", &state)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");

    clear();
    tokio::spawn(async move {
        let outcome = tokio::time::timeout(
            Duration::from_secs(300),
            finish(listener, &state, &client, &token_ep, &client_id, &redirect, &verifier),
        )
        .await;
        match outcome {
            Ok(Ok((id_token, name_hint))) => {
                *PENDING.lock().unwrap() = Some(Pending { server, id_token });
                hub.broadcast(json!({"event": "oidc.state", "state": "done", "name": name_hint}));
            }
            Ok(Err(e)) => {
                hub.broadcast(json!({"event": "oidc.state", "state": "failed", "error": format!("{e:#}")}));
            }
            Err(_) => {
                hub.broadcast(json!({"event": "oidc.state", "state": "failed", "error": "the sign-in took too long; try again"}));
            }
        }
    });
    Ok(url.to_string())
}

/// Wait for the browser to come back with the code, then trade it for the
/// ID token. Returns the token and the provider's preferred username.
async fn finish(
    listener: tokio::net::TcpListener,
    state: &str,
    client: &reqwest::Client,
    token_ep: &str,
    client_id: &str,
    redirect: &str,
    verifier: &str,
) -> anyhow::Result<(String, String)> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let code = loop {
        let (mut sock, _) = listener.accept().await?;
        let mut buf = vec![0u8; 8192];
        let n = sock.read(&mut buf).await.unwrap_or(0);
        let req = String::from_utf8_lossy(&buf[..n]).to_string();
        let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
        let parsed = url::Url::parse(&format!("http://127.0.0.1{path}")).ok();
        let q = |k: &str| {
            parsed
                .as_ref()
                .and_then(|u| u.query_pairs().find(|(a, _)| a == k).map(|(_, v)| v.to_string()))
        };
        if !path.starts_with("/callback") {
            let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            continue;
        }
        let (title, body) = match (q("code"), q("state"), q("error")) {
            (Some(_), Some(s), _) if s == state => ("Signed in", "You can close this page and go back to Sigil."),
            (_, _, Some(err)) => {
                page(&mut sock, "Sign-in refused", &err).await;
                anyhow::bail!("the provider refused the sign-in: {err}");
            }
            _ => ("Something went wrong", "That sign-in did not match the one Sigil started. Go back to Sigil and try again."),
        };
        page(&mut sock, title, body).await;
        if title == "Signed in" {
            break q("code").unwrap_or_default();
        }
        anyhow::bail!("the sign-in did not match the one started");
    };

    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("grant_type", "authorization_code")
        .append_pair("code", &code)
        .append_pair("redirect_uri", redirect)
        .append_pair("client_id", client_id)
        .append_pair("code_verifier", verifier)
        .finish();
    let resp: Value = client
        .post(token_ep)
        .header("content-type", "application/x-www-form-urlencoded")
        .header("accept", "application/json")
        .body(body)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let id_token = resp["id_token"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("the provider returned no ID token"))?
        .to_string();
    // the name the provider knows the person by, as a suggestion
    let hint = id_token
        .split('.')
        .nth(1)
        .and_then(|p| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(p).ok())
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|c| c["preferred_username"].as_str().map(str::to_string))
        .unwrap_or_default();
    Ok((id_token, hint))
}

async fn page(sock: &mut tokio::net::TcpStream, title: &str, body: &str) {
    use tokio::io::AsyncWriteExt;
    let html = format!(
        "<!doctype html><meta charset=utf-8><meta name=viewport content=\"width=device-width\">\
         <title>{title}</title>\
         <body style=\"font-family:system-ui,sans-serif;background:#111;color:#eee;display:grid;place-items:center;height:100vh;margin:0\">\
         <div style=\"text-align:center;max-width:26em;padding:1em\"><h1 style=\"font-weight:600\">{title}</h1><p>{body}</p></div>"
    );
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
        html.len()
    );
    let _ = sock.write_all(resp.as_bytes()).await;
    let _ = sock.shutdown().await;
}
