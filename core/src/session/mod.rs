//! Login (OAuth 2.0 / MSC3861 via the system browser), session persistence, restore, logout.
pub mod recovery;
pub mod store;

use anyhow::Context;
use matrix_sdk::{
    authentication::oauth::{
        registration::{ApplicationType, ClientMetadata, Localized, OAuthGrantType},
        ClientRegistrationData,
    },
    encryption::{BackupDownloadStrategy, EncryptionSettings},
    sliding_sync::VersionBuilder,
    store::RoomLoadSettings,
    utils::{
        local_server::{LocalServerBuilder, LocalServerIpAddress, LocalServerResponse},
        UrlOrQuery,
    },
    Client, SessionChange,
};
use ruma::serde::Raw;
use serde_json::{json, Value};
use tracing::{info, warn};
use url::Url;

use crate::engine::{SessionState, SharedEngine};
use crate::ipc::wire::Reply;
use crate::paths;

const DONE_PAGE: &str = r#"<!doctype html><html><head><meta charset="utf-8"><title>Omarchy Matrix</title>
<style>body{margin:0;height:100vh;display:flex;align-items:center;justify-content:center;font-family:sans-serif;background:#1a1a1a;color:#dcdcdc}
div{padding:2rem 3rem;border:1px solid #444;border-radius:14px;background:rgba(75,75,75,.55)}</style></head>
<body><div><h2>Signed in</h2><p>You can close this tab and return to Omarchy.</p></div></body></html>"#;

pub async fn build_client(homeserver: &str) -> anyhow::Result<Client> {
    let state = paths::state_dir();
    let cache = paths::cache_dir();
    paths::ensure_private_dir(&state)?;
    paths::ensure_private_dir(&cache)?;
    let key = store::store_key(&state)?;
    let client = Client::builder()
        .server_name_or_homeserver_url(homeserver)
        .sqlite_store_with_cache_path(state.join("store"), cache.join("store"), Some(&key))
        .handle_refresh_tokens()
        .with_encryption_settings(EncryptionSettings {
            auto_enable_cross_signing: true,
            backup_download_strategy: BackupDownloadStrategy::AfterDecryptionFailure,
            auto_enable_backups: true,
        })
        .sliding_sync_version_builder(VersionBuilder::Native)
        .build()
        .await
        .context("building client")?;
    Ok(client)
}

fn registration_data() -> anyhow::Result<ClientRegistrationData> {
    let client_uri = Url::parse("https://github.com/pellinore377/Sigil")?;
    let mut meta = ClientMetadata::new(
        ApplicationType::Native,
        vec![
            OAuthGrantType::AuthorizationCode { redirect_uris: vec![Url::parse("http://localhost/")?, Url::parse("http://127.0.0.1/")?, Url::parse("http://[::1]/")?] },
        ],
        Localized::new(client_uri, []),
    );
    meta.client_name = Some(Localized::new("Sigil".to_owned(), []));
    Ok(ClientRegistrationData::new(Raw::new(&meta)?))
}

fn device_name() -> String {
    let host = hostname::get().ok().and_then(|h| h.into_string().ok()).unwrap_or_else(|| "this machine".into());
    format!("Sigil on {host}")
}

pub async fn login_start(engine: SharedEngine, homeserver: String, open_browser: bool) -> Reply {
    if homeserver.is_empty() {
        return Reply::err("bad_request", "homeserver is required");
    }
    // Check and claim in ONE lock scope. Checking then awaiting lets a second
    // login.start through before the first sets pending, and two build_client
    // calls race their migrations over one sqlite store (seen on Android,
    // where a debug build made the window seconds wide).
    {
        let mut s = engine.state.lock();
        match s.session.unwrap_or(SessionState::LoggedOut) {
            SessionState::LoggedIn => return Reply::err("bad_request", "already logged in"),
            SessionState::LoginPending => return Reply::err("login_in_progress", "a login is already in progress"),
            _ => s.session = Some(SessionState::LoginPending),
        }
    }
    engine.broadcast_status();
    let reply = login_begin(engine.clone(), homeserver, open_browser).await;
    // Every error return in login_begin happens before its flow task spawns,
    // so an Err here means nothing is running: release the claim. Failures
    // after the spawn go through finish_failed instead.
    if matches!(reply, Reply::Err(_)) {
        engine.set_session(SessionState::LoggedOut);
    }
    reply
}

async fn login_begin(engine: SharedEngine, homeserver: String, open_browser: bool) -> Reply {
    let client = match build_client(&homeserver).await {
        Ok(c) => c,
        Err(e) => return Reply::err("network", format!("{e:#}")),
    };
    let oauth = client.oauth();
    if let Err(e) = oauth.server_metadata().await {
        return Reply::err("oidc_unsupported", format!("homeserver does not support OAuth 2.0 login: {e}"));
    }
    let (redirect_url, handle) = match LocalServerBuilder::new()
        .ip_address(LocalServerIpAddress::Localhostv4)
        .port_range(20000..30000)
        .response(LocalServerResponse::Html(DONE_PAGE.to_owned()))
        .spawn()
        .await
    {
        Ok(v) => v,
        Err(e) => return Reply::err("internal", format!("cannot start local redirect server: {e}")),
    };
    let reg = match registration_data() {
        Ok(r) => r,
        Err(e) => return Reply::err("internal", format!("{e:#}")),
    };
    // Reuse the previous device id so the crypto identity survives a re-login.
    let previous_device = store::load_session(&paths::state_dir()).ok().flatten().map(|s| s.user.meta.device_id);
    let auth = match oauth.login(redirect_url, previous_device, Some(reg), None).build().await {
        Ok(a) => a,
        Err(e) => return Reply::err("network", format!("authorization request failed: {e}")),
    };
    let url = auth.url.to_string();
    info!("login: redirect {} → auth url ready", client.homeserver());
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
    {
        let mut s = engine.state.lock();
        s.session = Some(SessionState::LoginPending);
        s.homeserver = client.homeserver().to_string();
        s.login_url = url.clone();
        s.login_cancel = Some(cancel_tx);
        s.last_error.clear();
        s.client = Some(client.clone());
    }
    engine.broadcast_status();
    if open_browser {
        open_in_browser(&url);
    }
    let engine2 = engine.clone();
    let state_token = auth.state.clone();
    tokio::spawn(async move {
        let timeout = tokio::time::sleep(std::time::Duration::from_secs(600));
        tokio::pin!(timeout);
        let query = tokio::select! {
            q = handle => q,
            _ = &mut timeout => { finish_failed(&engine2, &client, &state_token, "login timed out").await; return; }
            _ = cancel_rx => { finish_failed(&engine2, &client, &state_token, "login cancelled").await; return; }
        };
        let Some(query) = query else {
            finish_failed(&engine2, &client, &state_token, "browser redirect carried no data").await;
            return;
        };
        match client.oauth().finish_login(UrlOrQuery::Query(query.0)).await {
            Ok(()) => finish_ok(engine2, client).await,
            Err(e) => { tracing::error!("finish_login: {e:#}"); finish_failed(&engine2, &client, &state_token, &format!("login failed: {e}")).await },
        }
    });
    Reply::ok(json!({"url": url}))
}

/// Manual fallback: the user pastes the URL/query the browser was redirected to.
pub async fn login_finish_manual(engine: SharedEngine, query: String) -> Reply {
    if engine.session() != SessionState::LoginPending {
        return Reply::err("bad_request", "no login in progress");
    }
    let Some(client) = engine.client() else { return Reply::err("internal", "no client") };
    let q = query.trim().to_string();
    let uq = match Url::parse(&q) {
        Ok(u) => UrlOrQuery::Url(u),
        Err(_) => UrlOrQuery::Query(q.trim_start_matches('?').to_string()),
    };
    match client.oauth().finish_login(uq).await {
        Ok(()) => {
            finish_ok(engine, client).await;
            Reply::ok(json!({}))
        }
        Err(e) => Reply::err("network", format!("login failed: {e}")),
    }
}

pub async fn login_cancel(engine: &SharedEngine) -> Reply {
    let tx = engine.state.lock().login_cancel.take();
    match tx {
        Some(tx) => {
            let _ = tx.send(());
            Reply::ok(json!({}))
        }
        None => Reply::err("bad_request", "no login in progress"),
    }
}

async fn finish_failed(engine: &SharedEngine, client: &Client, state: &matrix_sdk::authentication::oauth::CsrfToken, msg: &str) {
    client.oauth().abort_login(state).await;
    {
        let mut s = engine.state.lock();
        s.session = Some(SessionState::LoggedOut);
        s.login_url.clear();
        s.login_cancel = None;
        s.client = None;
        s.last_error = msg.to_string();
    }
    engine.hub.broadcast(json!({"event":"login.failed","error":{"code":"login_failed","message":msg}}));
    engine.broadcast_status();
}

async fn finish_ok(engine: SharedEngine, client: Client) {
    let Some(session) = client.oauth().full_session() else {
        finish_failed(&engine, &client, &matrix_sdk::authentication::oauth::CsrfToken::new(String::new()), "no session after login").await;
        return;
    };
    if let Err(e) = store::save_session(&paths::state_dir(), &store::SavedSession::from_oauth(client.homeserver().to_string(), session.clone())) {
        warn!("failed to persist session: {e:#}");
    }
    if let Some(dev) = client.device_id() {
        if let Err(e) = client.rename_device(dev, &device_name()).await {
            warn!("rename_device failed: {e}");
        }
    }
    {
        let mut s = engine.state.lock();
        s.login_url.clear();
        s.login_cancel = None;
    }
    activate(engine.clone(), client.clone()).await;
    engine.hub.broadcast(json!({"event":"login.finished","userId":session.user.meta.user_id,"deviceId":session.user.meta.device_id}));
}

/// Common path after login or restore: fill identity, watch tokens, start sync.
async fn activate(engine: SharedEngine, client: Client) {
    {
        let mut s = engine.state.lock();
        s.client = Some(client.clone());
        s.session = Some(SessionState::LoggedIn);
        s.homeserver = client.homeserver().to_string();
        s.server_name = client.user_id().map(|u| u.server_name().to_string()).unwrap_or_default();
        s.user_id = client.user_id().map(|u| u.to_string()).unwrap_or_default();
        s.device_id = client.device_id().map(|d| d.to_string()).unwrap_or_default();
        s.last_error.clear();
    }
    engine.broadcast_status();

    {
        let engine = engine.clone();
        let client = client.clone();
        let mut rx = client.subscribe_to_session_changes();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(SessionChange::TokensRefreshed) => {
                        if let Some(session) = client.oauth().full_session() {
                            if let Err(e) = store::save_session(&paths::state_dir(), &store::SavedSession::from_oauth(client.homeserver().to_string(), session)) {
                                warn!("failed to persist refreshed tokens: {e:#}");
                            } else {
                                info!("tokens refreshed and persisted");
                            }
                        }
                    }
                    Ok(SessionChange::UnknownToken(data)) => {
                        let msg = if data.soft_logout { "session expired (soft logout)" } else { "session was invalidated by the server" };
                        engine.set_error(msg);
                        let _ = crate::sync::stop(&engine).await;
                        {
                            let mut s = engine.state.lock();
                            s.session = Some(SessionState::LoggedOut);
                            s.client = None;
                        }
                        engine.broadcast_status();
                        break;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        });
    }

    crate::notify::install(engine.clone(), client.clone());
    #[cfg(feature = "calls")]
    crate::rtc::install(engine.clone(), client.clone());
    crate::sync::start(engine.clone(), client.clone()).await;
    crate::session::recovery::watch(engine.clone(), client.clone());
    tokio::spawn(async { crate::media::gc(2 * 1024 * 1024 * 1024) });
    tokio::spawn(async move { crate::session::profile::refresh(engine, client).await });
}

pub async fn restore(engine: SharedEngine) -> anyhow::Result<bool> {
    let Some(saved) = store::load_session(&paths::state_dir())? else { return Ok(false) };
    engine.set_session(SessionState::Restoring);
    let client = build_client(&saved.homeserver).await?;
    client
        .oauth()
        .restore_session(saved.into_oauth(), RoomLoadSettings::default())
        .await
        .context("restoring OAuth session")?;
    activate(engine, client).await;
    Ok(true)
}

pub async fn logout(engine: SharedEngine, wipe: bool) -> Reply {
    #[cfg(feature = "calls")]
    engine.rtc.leave("logout").await;
    let client = engine.client();
    let _ = crate::sync::stop(&engine).await;
    if let Some(client) = client {
        if let Err(e) = client.logout().await {
            warn!("server logout failed: {e}");
        }
    }
    store::remove_session(&paths::state_dir());
    {
        let mut s = engine.state.lock();
        *s = crate::engine::State { session: Some(SessionState::LoggedOut), sync_state: "offline".into(), ..Default::default() };
    }
    if wipe {
        let _ = std::fs::remove_dir_all(paths::state_dir().join("store"));
        let _ = std::fs::remove_dir_all(paths::cache_dir());
    }
    engine.broadcast_status();
    engine.hub.broadcast(crate::session::recovery::status_json(&engine));
    Reply::ok(json!({}))
}

fn open_in_browser(url: &str) {
    let cmds: [&[&str]; 2] = [&["omarchy-launch-browser", url], &["xdg-open", url]];
    for c in cmds {
        if std::process::Command::new(c[0]).args(&c[1..]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().is_ok() {
            return;
        }
    }
    warn!("could not open a browser for {url}");
}

pub mod profile {
    use super::*;
    pub async fn refresh(engine: SharedEngine, client: Client) {
        let Some(user_id) = client.user_id().map(|u| u.to_owned()) else { return };
        match client.account().get_display_name().await {
            Ok(p) => {
                let name = p.unwrap_or_else(|| user_id.localpart().to_string());
                engine.state.lock().display_name = name;
                engine.broadcast_status();
            }
            Err(e) => warn!("get_display_name failed: {e}"),
        }
        match client.account().get_avatar_url().await {
            Ok(Some(url)) => {
                let path = crate::media::cached_avatar_path(&engine, url.as_str()).await;
                engine.state.lock().avatar_path = path;
                engine.broadcast_status();
            }
            Ok(None) => {}
            Err(e) => warn!("get_avatar_url failed: {e}"),
        }
    }
}

#[allow(dead_code)]
pub fn value_str(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}
