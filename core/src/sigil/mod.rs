//! The Sigil session inside the engine: an account on a Sigil server and
//! its MLS conversations, presented to frontends through the same `status`,
//! `rooms.list`, `timeline.reset` and `timeline.diff` events the engine has
//! always emitted. Frontends do not know what is underneath, by design.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::{json, Value};
use sigil_client::provider::SigilProvider;
use sigil_client::state::{Conversation, PendingRequest};
use sigil_client::{account, conversation, Link, State as Account};
use sigil_protocol::envelope::Kind;
use sigil_protocol::wire::Frame;
use tokio::sync::Mutex as AsyncMutex;
use tracing::{info, warn};

use crate::engine::{SessionState, SharedEngine};
use crate::ipc::wire::{Reply, Request};

enum Handle {
    /// A conversation, and the address this handle was subscribed for.
    Conversation(String, [u8; 32]),
    Requests([u8; 32]),
}

pub struct SigilSession {
    /// Account state and the MLS store, locked together: every MLS
    /// operation spends a token and records what it sent.
    inner: AsyncMutex<(Account, SigilProvider)>,
    link: Arc<Link>,
    /// group id → timeline items, oldest first. Persisted.
    history: Mutex<HashMap<String, Vec<Value>>>,
    handles: Mutex<HashMap<[u8; 32], Handle>>,
    open: Mutex<HashSet<String>>,
    history_path: PathBuf,
    /// Last typing notice sent per room (ms), for the 5 s rate limit.
    typing_sent: Mutex<HashMap<String, i64>>,
    username: String,
    identity_pub: [u8; 32],
    /// A scanned link offer awaiting the user's emoji confirmation.
    pending_scan: Mutex<Option<sigil_client::linking::Scanned>>,
    /// Set when something changed since the last backup upload.
    dirty: std::sync::atomic::AtomicBool,
    /// Whether the last backup upload succeeded, for `recovery.status`.
    backed_up: std::sync::atomic::AtomicBool,
}

fn account_path() -> PathBuf {
    crate::paths::state_dir().join("sigil-account.json")
}

/// The paranoid tier's switches, in `settings.json` under `shape`.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Shape {
    /// Seconds between bags to the Envoy whether or not there is anything
    /// to say; 0 = off.
    #[serde(default)]
    pub clocked_seconds: u64,
    /// SOCKS5 proxy `host:port` for every connection, for example a local
    /// Tor daemon at 127.0.0.1:9050; empty = direct.
    #[serde(default)]
    pub socks_proxy: String,
}

pub fn load_shape() -> Shape {
    std::fs::read(crate::notify::settings_path())
        .ok()
        .and_then(|d| serde_json::from_slice::<Value>(&d).ok())
        .and_then(|v| serde_json::from_value(v.get("shape").cloned()?).ok())
        .unwrap_or_default()
}

pub fn save_shape(sh: &Shape) {
    let path = crate::notify::settings_path();
    let mut v: Value = std::fs::read(&path).ok().and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or_else(|| json!({}));
    v["shape"] = serde_json::to_value(sh).unwrap_or(json!({}));
    let _ = std::fs::write(&path, serde_json::to_vec_pretty(&v).unwrap_or_default());
}

/// Connect to an Envoy honouring the shape settings.
async fn connect(envoy: &str, device_id: &str) -> anyhow::Result<Link> {
    let sh = load_shape();
    let proxy = if sh.socks_proxy.is_empty() { None } else { Some(sh.socks_proxy.as_str()) };
    Link::connect_with(envoy, device_id, proxy).await
}

/// A saved account exists on disk.
pub fn has_account() -> bool {
    account_path().exists()
}

/// The recovery status event. Recovery arrives with Phase 4; until then
/// the account is "backed up" only by the device that holds it.
pub fn recovery_status_json() -> Value {
    json!({"event":"recovery.status","recovery":"unknown","backup":"unknown","verified":true})
}

// ---------------------------------------------------------------- lifecycle

/// Restore a saved account at startup. Returns true if a Sigil account
/// exists on this device (whether or not it could connect).
pub async fn restore(engine: &SharedEngine) -> bool {
    if !has_account() {
        return false;
    }
    engine.set_session(SessionState::Restoring);
    let acct = match Account::load(&account_path()) {
        Ok(a) => a,
        Err(e) => {
            engine.set_error(format!("sigil account unreadable: {e:#}"));
            engine.set_session(SessionState::LoggedOut);
            return true;
        }
    };
    if let Err(e) = start(engine, acct).await {
        engine.set_error(format!("sigil session restore failed: {e:#}"));
        engine.set_session(SessionState::LoggedOut);
    }
    true
}

async fn start(engine: &SharedEngine, acct: Account) -> anyhow::Result<()> {
    let provider = SigilProvider::open(&acct.mls_path())?;
    let link = Arc::new(connect(&acct.envoy, &acct.device_id).await?);
    {
        let sh = load_shape();
        if sh.clocked_seconds > 0 {
            link.start_clock(acct.server(), std::time::Duration::from_secs(sh.clocked_seconds));
        }
    }
    let history_path = crate::paths::state_dir().join("sigil-history.json");
    let history: HashMap<String, Vec<Value>> = std::fs::read(&history_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let username = acct.username.clone();
    let identity_pub = acct.identity().public();
    let (local, server) = sigil_protocol::names::parse_username(&username)
        .map_err(|_| anyhow::anyhow!("bad username"))?;
    {
        let mut s = engine.state.lock();
        s.homeserver = server.to_string();
        s.server_name = server.to_string();
        s.user_id = username.clone();
        s.device_id = acct.device_id[..8].to_string();
        s.display_name = local.to_string();
        s.sync_state = "online".into();
        s.sync_error.clear();
        s.verified = true;
        s.last_error.clear();
    }
    let session = Arc::new(SigilSession {
        inner: AsyncMutex::new((acct, provider)),
        link,
        history: Mutex::new(history),
        handles: Mutex::new(HashMap::new()),
        open: Mutex::new(HashSet::new()),
        history_path,
        typing_sent: Mutex::new(HashMap::new()),
        username,
        identity_pub,
        pending_scan: Mutex::new(None),
        dirty: std::sync::atomic::AtomicBool::new(false),
        backed_up: std::sync::atomic::AtomicBool::new(false),
    });
    *engine.sigil.lock() = Some(session.clone());
    engine.set_session(SessionState::LoggedIn);
    session.subscribe_all().await;
    session.broadcast_rooms(engine).await;
    let (e2, s2) = (engine.clone(), session.clone());
    tokio::spawn(async move { s2.delivery_loop(e2).await });
    let (e4, s4) = (engine.clone(), session.clone());
    tokio::spawn(async move { s4.backup_loop(e4).await });
    info!("sigil session active as {}", session.username);
    Ok(())
}

async fn create(engine: &SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    let username = p
        .get("username")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_lowercase();
    let invite = p
        .get("invite")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let Ok((_, server)) = sigil_protocol::names::parse_username(&username) else {
        return Reply::err("bad_request", "username must look like @name:server");
    };
    let envoy = p
        .get("envoy")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("wss://{server}/envoy"));
    if has_account() {
        return Reply::err(
            "bad_request",
            "an account already exists on this device; log out first",
        );
    }
    engine.set_session(SessionState::LoginPending);
    let path = account_path();
    let _ = crate::paths::ensure_private_dir(path.parent().unwrap());
    let password = p.get("password").and_then(Value::as_str).unwrap_or("").to_string();
    let result: anyhow::Result<()> = async {
        let mut acct = Account::create(&path, &username, &envoy)?;
        let provider = SigilProvider::open(&acct.mls_path())?;
        let link = connect(&envoy, &acct.device_id).await?;
        account::register(&link, &mut acct, &invite).await?;
        account::publish_key_packages(&link, &mut acct, &provider, 10).await?;
        if !password.is_empty() {
            sigil_client::backup::enable(&link, &mut acct, &password).await?;
        }
        drop(link);
        start(engine, acct).await
    }
    .await;
    match result {
        Ok(()) => Reply::ok(json!({"userId": username})),
        Err(e) => {
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(path.with_extension("mls.json"));
            engine.set_error(format!("account creation failed: {e:#}"));
            engine.set_session(SessionState::LoggedOut);
            Reply::err("network", format!("{e:#}"))
        }
    }
}

async fn logout(engine: &SharedEngine, wipe: bool) -> Reply {
    *engine.sigil.lock() = None;
    {
        let mut s = engine.state.lock();
        s.user_id.clear();
        s.homeserver.clear();
        s.server_name.clear();
        s.display_name.clear();
        s.sync_state = "offline".into();
        s.rooms_snapshot = Value::Null;
    }
    if wipe {
        let p = account_path();
        let _ = std::fs::remove_file(&p);
        let _ = std::fs::remove_file(p.with_extension("mls.json"));
        let _ = std::fs::remove_file(crate::paths::state_dir().join("sigil-history.json"));
    }
    engine.set_session(SessionState::LoggedOut);
    Reply::ok(json!({}))
}

/// New device: show an offer, wait for an existing device, then start.
/// Progress arrives as `link.state` events: offer, sas, joining, done, failed.
async fn link_offer(engine: &SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    if has_account() {
        return Reply::err("bad_request", "an account already exists on this device; log out first");
    }
    let username = p.get("username").and_then(Value::as_str).unwrap_or("").trim().to_lowercase();
    let Ok((_, server)) = sigil_protocol::names::parse_username(&username) else {
        return Reply::err("bad_request", "username must look like @name:server");
    };
    let server = server.to_string();
    let envoy = p.get("envoy").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("wss://{server}/envoy"));
    let offer = sigil_client::linking::Offer::new();
    let text = offer.text();
    engine.set_session(SessionState::LoginPending);
    engine.hub.broadcast(json!({"event":"link.state","state":"offer","offer":text,"sas":""}));
    let e2 = engine.clone();
    tokio::spawn(async move {
        let path = account_path();
        let _ = crate::paths::ensure_private_dir(path.parent().unwrap());
        let e3 = e2.clone();
        let result = sigil_client::linking::wait_for_link(&path, &server, &envoy, &offer, move |pr| match pr {
            sigil_client::linking::Progress::Sas(sas) => e3.hub.broadcast(json!({"event":"link.state","state":"sas","sas":sas})),
            sigil_client::linking::Progress::Welcomed(w) => e3.hub.broadcast(json!({"event":"link.state","state":"joining","with":w})),
            sigil_client::linking::Progress::Done => {}
        })
        .await;
        match result {
            Ok((acct, extra)) => {
                if !extra.is_empty() {
                    let _ = std::fs::write(crate::paths::state_dir().join("sigil-history.json"), &extra);
                }
                match start(&e2, acct).await {
                    Ok(()) => e2.hub.broadcast(json!({"event":"link.state","state":"done"})),
                    Err(err) => {
                        e2.set_error(format!("link failed: {err:#}"));
                        e2.set_session(SessionState::LoggedOut);
                        e2.hub.broadcast(json!({"event":"link.state","state":"failed","error":format!("{err:#}")}));
                    }
                }
            }
            Err(err) => {
                let _ = std::fs::remove_file(&path);
                let _ = std::fs::remove_file(path.with_extension("mls.json"));
                e2.set_error(format!("link failed: {err:#}"));
                e2.set_session(SessionState::LoggedOut);
                e2.hub.broadcast(json!({"event":"link.state","state":"failed","error":format!("{err:#}")}));
            }
        }
    });
    Reply::ok(json!({"offer": text}))
}

/// Fresh device: restore from username, password and the recovery code.
async fn recover(engine: &SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    if has_account() {
        return Reply::err("bad_request", "an account already exists on this device; log out first");
    }
    let username = p.get("username").and_then(Value::as_str).unwrap_or("").trim().to_lowercase();
    let password = p.get("password").and_then(Value::as_str).unwrap_or("").to_string();
    let code = p.get("code").and_then(Value::as_str).unwrap_or("").to_string();
    let Ok((_, server)) = sigil_protocol::names::parse_username(&username) else {
        return Reply::err("bad_request", "username must look like @name:server");
    };
    let envoy = p.get("envoy").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("wss://{server}/envoy"));
    let Ok(key) = sigil_protocol::recovery::parse_recovery_code(&code) else {
        return Reply::err("bad_request", "that is not a valid recovery code");
    };
    engine.set_session(SessionState::Restoring);
    let path = account_path();
    let _ = crate::paths::ensure_private_dir(path.parent().unwrap());
    match sigil_client::backup::restore(&path, &envoy, &username, &password, &key).await {
        Ok((acct, extra)) => {
            if !extra.is_empty() {
                let _ = std::fs::write(crate::paths::state_dir().join("sigil-history.json"), &extra);
            }
            match start(engine, acct).await {
                Ok(()) => Reply::ok(json!({"userId": username})),
                Err(e) => {
                    engine.set_error(format!("recovered but could not start: {e:#}"));
                    engine.set_session(SessionState::LoggedOut);
                    Reply::err("network", format!("{e:#}"))
                }
            }
        }
        Err(e) => {
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(path.with_extension("mls.json"));
            engine.set_session(SessionState::LoggedOut);
            Reply::err("bad_request", format!("{e:#}"))
        }
    }
}

async fn self_ref_hack(s: &Arc<SigilSession>) -> tokio::sync::MutexGuard<'_, (Account, SigilProvider)> {
    s.inner.lock().await
}

// ---------------------------------------------------------------- dispatch

fn param(p: &serde_json::Map<String, Value>, k: &str) -> String {
    p.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

/// First chance at every request. `None` means "not mine, carry on".
pub async fn dispatch(engine: &SharedEngine, req: &Request) -> Option<Reply> {
    let p = &req.params;
    match req.req.as_str() {
        "account.create" => return Some(create(engine, p).await),
        "account.status" => {
            return Some(Reply::ok(
                json!({"exists": has_account(), "active": engine.sigil.lock().is_some()}),
            ))
        }
        "shape.settings" => {
            let mut sh = load_shape();
            let mut changed = false;
            if let Some(n) = p.get("clockedSeconds").and_then(Value::as_u64) {
                sh.clocked_seconds = n;
                changed = true;
            }
            if let Some(px) = p.get("socksProxy").and_then(Value::as_str) {
                sh.socks_proxy = px.trim().to_string();
                changed = true;
            }
            if changed {
                save_shape(&sh);
            }
            return Some(Reply::ok(json!({"clockedSeconds": sh.clocked_seconds, "socksProxy": sh.socks_proxy, "appliesOn": "next connection"})));
        }
        "recovery.status" => {
            let active = engine.sigil.lock().clone();
            return Some(Reply::ok(match active {
                Some(s) => s.recovery_status().await,
                None => recovery_status_json(),
            }));
        }
        "recovery.recover" => return Some(Reply::err("bad_request", "use account.recover on a fresh device")),
        "link.offer" => return Some(link_offer(engine, p).await),
        "account.recover" => return Some(recover(engine, p).await),
        "logout" => {
            return Some(
                logout(
                    engine,
                    p.get("wipe").and_then(Value::as_bool).unwrap_or(false),
                )
                .await,
            )
        }
        _ => {}
    }
    let s = engine.sigil.lock().clone()?;
    Some(match req.req.as_str() {
        "rooms.list" => Reply::ok(s.rooms_snapshot().await),
        "spaces.tree" => Reply::ok(json!({"event":"spaces.tree","spaces":[]})),
        "room.open" => s.room_open(engine, &param(p, "roomId")).await,
        "room.close" => {
            s.open.lock().remove(&param(p, "roomId"));
            Reply::ok(json!({}))
        }
        "timeline.paginate" => s.paginate(engine, &param(p, "roomId")).await,
        "message.send" => {
            s.send_text(engine, &param(p, "roomId"), &param(p, "body"), None)
                .await
        }
        "message.reply" => {
            s.send_text(
                engine,
                &param(p, "roomId"),
                &param(p, "body"),
                Some(param(p, "eventId")),
            )
            .await
        }
        "message.react" => {
            s.send_small(
                engine,
                &param(p, "roomId"),
                Kind::Reaction,
                &param(p, "eventId"),
                &param(p, "key"),
            )
            .await
        }
        "readReceipt" => {
            s.send_small(
                engine,
                &param(p, "roomId"),
                Kind::Receipt,
                &param(p, "eventId"),
                "",
            )
            .await
        }
        "room.markRead" => {
            s.mark_read(engine, &param(p, "roomId")).await;
            Reply::ok(json!({}))
        }
        "typing" => {
            s.typing(
                engine,
                &param(p, "roomId"),
                p.get("typing").and_then(Value::as_bool).unwrap_or(false),
            )
            .await
        }
        "dm.create" => s.dm_create(engine, &param(p, "userId")).await,
        "room.create" => s.room_create(engine, p).await,
        "room.invite" => s.room_invite(engine, &param(p, "roomId"), &param(p, "userId")).await,
        "room.setSettings" => s.room_set_settings(engine, &param(p, "roomId"), p).await,
        "attachment.send" => s.attachment_send(engine, p).await,
        "call.start" => s.call_start(engine, &param(p, "roomId")).await,
        "call.end" => s.call_end(engine, &param(p, "roomId"), &param(p, "callId")).await,
        "call.join" | "call.poll" | "call.answer" | "call.leave" => s.call_signal(p, &req.req[5..]).await,
        "media.get" => s.media_get(&param(p, "roomId"), &param(p, "eventId")).await,
        "room.join" => s.accept(engine, &param(p, "roomIdOrAlias")).await,
        "room.leave" => s.leave(engine, &param(p, "roomId")).await,
        "room.members" => s.members(&param(p, "roomId")).await,
        "users.search" => s.search(&param(p, "query")).await,
        "link.scan" => s.link_scan(engine, &param(p, "offer")).await,
        "recovery.code" => match sigil_client::backup::code(&s.inner.lock().await.0) {
            Some(code) => Reply::ok(json!({"code": code})),
            None => Reply::err("bad_request", "no password set on this account"),
        },
        "account.setPassword" => {
            let pw = param(p, "password");
            if pw.is_empty() {
                Reply::err("bad_request", "password is empty")
            } else {
                let r = {
                    let mut g = self_ref_hack(&s).await;
                    let (a, _) = &mut *g;
                    if a.recovery.is_none() {
                        sigil_client::backup::enable(&s.link, a, &pw).await
                    } else {
                        sigil_client::backup::set_password(&s.link, a, &pw).await
                    }
                };
                match r {
                    Ok(()) => {
                        s.mark_dirty();
                        engine.hub.broadcast(s.recovery_status().await);
                        Reply::ok(json!({}))
                    }
                    Err(e) => Reply::err("network", format!("{e:#}")),
                }
            }
        }
        "link.confirm" => s.link_confirm(engine, p.get("ok").and_then(Value::as_bool).unwrap_or(false)).await,
        r if r.starts_with("room.")
            || r.starts_with("message.")
            || r.starts_with("space.")
            || r.starts_with("thread")
            || r.starts_with("pins.")
            || r.starts_with("poll.")
            || r.starts_with("sticker")
            || r.starts_with("contact")
            || r.starts_with("location.")
            || r.starts_with("link.")
            || r == "voice.send"
            || r.starts_with("doc.")
            || r.starts_with("vcard.") =>
        {
            Reply::err(
                "unsupported",
                format!("'{r}' is not on the Sigil backend yet"),
            )
        }
        _ => return None,
    })
}

// ---------------------------------------------------------------- helpers

fn now_ms() -> i64 {
    crate::timeline::fmt::now_ms()
}

fn short_name(username: &str) -> String {
    username
        .trim_start_matches('@')
        .split(':')
        .next()
        .unwrap_or(username)
        .to_string()
}

fn period_now() -> u32 {
    (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        / 2_592_000) as u32
}

fn reply_summary(item: &Value) -> Value {
    json!({
        "eventId": item.get("eventId").cloned().unwrap_or(json!("")),
        "sender": item.get("sender").cloned().unwrap_or(json!("")),
        "senderName": item.get("senderName").cloned().unwrap_or(json!("")),
        "kind": item.get("kind").cloned().unwrap_or(json!("text")),
        "body": item.get("body").cloned().unwrap_or(json!("")),
    })
}

/// A text item in the engine's timeline shape. `src` is SigilText source;
/// it is composed here so every device renders it identically.
fn text_item(
    id: &str,
    sender: &str,
    ts: i64,
    is_own: bool,
    src: &str,
    reply_to: Option<Value>,
) -> Value {
    let composed = crate::timeline::effects::compose(src);
    let mut obj = json!({
        "id": id,
        "kind": "text",
        "eventId": id,
        "txnId": Value::Null,
        "sender": sender,
        "senderName": short_name(sender),
        "senderAvatarPath": "",
        "ts": ts,
        "isOwn": is_own,
        "isHighlighted": false,
        "body": composed.body,
        "html": crate::timeline::html::to_rich_text(&composed.html),
        "isEdited": false,
        "reactions": [],
        "sendState": "sent",
        "sendError": "",
        "readBy": [],
        "can": {"edit": false, "reply": true, "redact": false, "react": true},
    });
    let o = obj.as_object_mut().unwrap();
    if let Some(parts) = crate::timeline::html::to_parts(&composed.html) {
        o.insert("parts".into(), json!(parts));
    }
    if !composed.effects.is_empty() {
        o.insert(
            "effects".into(),
            crate::timeline::effects::to_json(&composed.effects),
        );
    }
    if let Some(r) = reply_to {
        o.insert("replyTo".into(), r);
    }
    obj
}

/// A media item: image, video, audio or file, with the manifest kept on the
/// item for `media.get`.
fn media_item(id: &str, sender: &str, ts: i64, is_own: bool, m: &sigil_client::media::Manifest, local_path: &str) -> Value {
    let kind = if m.mime.starts_with("image/") {
        "image"
    } else if m.mime.starts_with("video/") {
        "video"
    } else if m.mime.starts_with("audio/") {
        "audio"
    } else {
        "file"
    };
    json!({
        "id": id,
        "kind": kind,
        "eventId": id,
        "txnId": Value::Null,
        "sender": sender,
        "senderName": short_name(sender),
        "senderAvatarPath": "",
        "ts": ts,
        "isOwn": is_own,
        "isHighlighted": false,
        "body": if m.caption.is_empty() { m.filename.clone() } else { m.caption.clone() },
        "isEdited": false,
        "reactions": [],
        "sendState": "sent",
        "sendError": "",
        "readBy": [],
        "can": {"edit": false, "reply": true, "redact": false, "react": true},
        "media": {
            "mxc": format!("sigil:{}", m.chunks.first().cloned().unwrap_or_default()),
            "encrypted": true,
            "filename": m.filename,
            "mime": m.mime,
            "size": m.size,
            "width": m.width,
            "height": m.height,
            "path": local_path,
            "thumbnailPath": if kind == "image" { local_path } else { "" },
        },
        "manifest": serde_json::to_value(m).unwrap_or(Value::Null),
    })
}

fn room_json(c: &Conversation, hist: &[Value]) -> Value {
    let peer = c.peers.first().cloned().unwrap_or_default();
    let is_dm = c.name.is_empty() && c.peers.len() <= 1;
    let name = if c.name.is_empty() {
        if is_dm { short_name(&peer) } else { c.peers.iter().map(|p| short_name(p)).collect::<Vec<_>>().join(", ") }
    } else {
        c.name.clone()
    };
    let last = hist.last();
    let last_ts = last
        .and_then(|v| v.get("ts"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let unread = hist
        .iter()
        .rev()
        .take_while(|v| {
            !v.get("isOwn").and_then(Value::as_bool).unwrap_or(false)
                && !v.get("read").and_then(Value::as_bool).unwrap_or(false)
        })
        .count();
    json!({
        "id": c.group_id,
        "name": name,
        "topic": "",
        "avatarUrl": "",
        "avatarPath": "",
        "canonicalAlias": "",
        "isDm": is_dm,
        "dmUserId": if is_dm { Some(peer.clone()) } else { None },
        "isSpace": false,
        "spaceParents": [],
        "isEncrypted": true,
        "isInvite": false,
        "inviter": Value::Null,
        "isFavourite": false,
        "isLowPriority": false,
        "joinedMembers": c.members.len().max(c.peers.len() + 1),
        "unread": unread,
        "highlights": 0,
        "unreadMessages": unread,
        "markedUnread": false,
        "lastMessage": last.map(|v| json!({"kind": v.get("kind").cloned().unwrap_or(json!("text")), "sender": v.get("sender").cloned(), "senderName": v.get("senderName").cloned(), "body": v.get("body").cloned().unwrap_or(json!(""))})).unwrap_or(Value::Null),
        "lastActivityTs": last_ts,
        "stamp": crate::timeline::fmt::short(last_ts, now_ms()),
        "hasActiveCall": false,
        "callParticipants": [],
    })
}

fn parse_room(hex_id: &str) -> Option<[u8; 32]> {
    hex::decode(hex_id).ok()?.try_into().ok()
}

fn request_room_json(r: &PendingRequest) -> Value {
    json!({
        "id": format!("req:{}", &r.welcome[..16]),
        "name": short_name(&r.from),
        "topic": "",
        "avatarUrl": "", "avatarPath": "", "canonicalAlias": "",
        "isDm": true, "dmUserId": r.from, "isSpace": false, "spaceParents": [],
        "isEncrypted": true, "isInvite": true, "inviter": r.from,
        "isFavourite": false, "isLowPriority": false, "joinedMembers": 1,
        "unread": 1, "highlights": 1, "unreadMessages": 1, "markedUnread": false,
        "lastMessage": {"kind":"invite","sender": r.from, "body": if r.first_message.is_empty() { "Invitation".to_string() } else { r.first_message.clone() }},
        "lastActivityTs": now_ms(), "stamp": "", "hasActiveCall": false, "callParticipants": [],
    })
}

// ---------------------------------------------------------------- session

impl SigilSession {
    fn save_history(&self) {
        let h = self.history.lock();
        if let Ok(b) = serde_json::to_vec(&*h) {
            let _ = std::fs::write(&self.history_path, b);
        }
    }

    async fn conversation(&self, room_id: &str) -> Option<Conversation> {
        self.inner
            .lock()
            .await
            .0
            .conversations
            .iter()
            .find(|c| c.group_id == room_id)
            .cloned()
    }

    async fn rooms_snapshot(&self) -> Value {
        let (convs, reqs) = {
            let g = self.inner.lock().await;
            (g.0.conversations.clone(), g.0.requests.clone())
        };
        let h = self.history.lock();
        let mut rooms: Vec<Value> = convs
            .iter()
            .map(|c| room_json(c, h.get(&c.group_id).map(Vec::as_slice).unwrap_or(&[])))
            .collect();
        rooms.extend(reqs.iter().map(request_room_json));
        rooms.sort_by_key(|r| -r.get("lastActivityTs").and_then(Value::as_i64).unwrap_or(0));
        json!({"event":"rooms.list","loaded":true,"rooms":rooms})
    }

    async fn broadcast_rooms(&self, engine: &SharedEngine) {
        let snap = self.rooms_snapshot().await;
        engine.state.lock().rooms_snapshot = snap.clone();
        engine.hub.broadcast(snap);
    }

    async fn room_open(&self, engine: &SharedEngine, room_id: &str) -> Reply {
        if let Some(rid) = room_id.strip_prefix("req:") {
            let g = self.inner.lock().await;
            let Some(r) = g.0.requests.iter().find(|r| r.welcome.starts_with(rid)) else {
                return Reply::err("unknown_room", "no such request");
            };
            let item = text_item(
                &format!("req:{rid}"),
                &r.from,
                now_ms(),
                false,
                &r.first_message,
                None,
            );
            engine.hub.broadcast(
                json!({"event":"timeline.reset","roomId":room_id,"items":[item],"len":1}),
            );
            return Reply::ok(json!({"key": room_id}));
        }
        if self.conversation(room_id).await.is_none() {
            return Reply::err("unknown_room", format!("unknown room {room_id}"));
        }
        self.open.lock().insert(room_id.to_string());
        let items = self
            .history
            .lock()
            .get(room_id)
            .cloned()
            .unwrap_or_default();
        engine.hub.broadcast(
            json!({"event":"timeline.reset","roomId":room_id,"items":items,"len":items.len()}),
        );
        engine.hub.broadcast(
            json!({"event":"timeline.paginationState","roomId":room_id,"state":"timelineStart"}),
        );
        Reply::ok(json!({"key": room_id}))
    }

    async fn mark_read(&self, engine: &SharedEngine, room_id: &str) {
        {
            let mut h = self.history.lock();
            if let Some(items) = h.get_mut(room_id) {
                for it in items.iter_mut() {
                    if let Some(o) = it.as_object_mut() {
                        o.insert("read".into(), json!(true));
                    }
                }
            }
        }
        self.save_history();
        self.broadcast_rooms(engine).await;
    }

    async fn members(&self, room_id: &str) -> Reply {
        let Some(c) = self.conversation(room_id).await else {
            return Reply::err("unknown_room", "unknown room");
        };
        let members: Vec<Value> = if c.members.is_empty() {
            let mut v: Vec<Value> = c.peers.iter().map(|p| json!({"userId": p, "displayName": short_name(p), "avatarPath": "", "powerLevel": 100, "membership": "join"})).collect();
            v.push(json!({"userId": self.username, "displayName": short_name(&self.username), "avatarPath": "", "powerLevel": 100, "membership": "join"}));
            v
        } else {
            c.members.iter().map(|m| json!({"userId": m.username, "displayName": short_name(&m.username), "avatarPath": "", "powerLevel": if c.admins.contains(&m.identity) { 100 } else { 0 }, "membership": "join"})).collect()
        };
        Reply::ok(json!({"members": members}))
    }

    async fn search(&self, query: &str) -> Reply {
        let q = query.trim().to_lowercase();
        let q = if q.starts_with('@') {
            q
        } else {
            format!("@{q}")
        };
        if sigil_protocol::names::parse_username(&q).is_err() {
            return Reply::ok(json!({"results": []}));
        }
        match account::lookup(&self.link, &q).await {
            Ok(c) => Reply::ok(
                json!({"results": [{"userId": c.username, "displayName": short_name(&c.username), "avatarPath": ""}]}),
            ),
            Err(_) => Reply::ok(json!({"results": []})),
        }
    }

    async fn recovery_status(&self) -> Value {
        let enabled = self.inner.lock().await.0.recovery.is_some();
        json!({
            "event": "recovery.status",
            "recovery": if enabled { "enabled" } else { "disabled" },
            "backup": if !enabled { "disabled" } else if self.backed_up.load(std::sync::atomic::Ordering::Relaxed) { "enabled" } else { "pending" },
            "verified": true,
        })
    }

    fn mark_dirty(&self) {
        self.dirty.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Upload a backup whenever something changed, at most every few
    /// seconds, and once at start.
    async fn backup_loop(self: Arc<Self>, engine: SharedEngine) {
        self.mark_dirty();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            if engine.sigil.lock().as_ref().map(|s| !Arc::ptr_eq(s, &self)).unwrap_or(true) {
                return;
            }
            if !self.dirty.swap(false, std::sync::atomic::Ordering::Relaxed) {
                continue;
            }
            let extra = std::fs::read(&self.history_path).unwrap_or_default();
            self.top_up().await;
            let r = {
                let mut g = self.inner.lock().await;
                if g.0.recovery.is_none() {
                    continue;
                }
                let (a, p) = &mut *g;
                sigil_client::backup::upload(&self.link, a, p, &extra).await
            };
            match r {
                Ok(_) => {
                    if !self.backed_up.swap(true, std::sync::atomic::Ordering::Relaxed) {
                        engine.hub.broadcast(self.recovery_status().await);
                    }
                }
                Err(e) => {
                    warn!("backup upload failed: {e:#}");
                    self.mark_dirty();
                }
            }
        }
    }

    // ------------------------------------------------------------ timeline

    /// Append an item to a room's history and push it to open timelines.
    async fn append(&self, engine: &SharedEngine, room_id: &str, item: Value) {
        let len = {
            let mut h = self.history.lock();
            let items = h.entry(room_id.to_string()).or_default();
            let id = item
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if items
                .iter()
                .any(|i| i.get("id").and_then(Value::as_str) == Some(id.as_str()))
            {
                return;
            }
            items.push(item.clone());
            items.len()
        };
        self.save_history();
        self.mark_dirty();
        if self.open.lock().contains(room_id) {
            engine.hub.broadcast(json!({"event":"timeline.diff","roomId":room_id,"ops":[{"op":"pushBack","item":item}],"len":len}));
        }
        self.broadcast_rooms(engine).await;
    }

    fn item_by_id(&self, room_id: &str, event_id: &str) -> Option<Value> {
        self.history
            .lock()
            .get(room_id)?
            .iter()
            .find(|i| i.get("eventId").and_then(Value::as_str) == Some(event_id))
            .cloned()
    }

    async fn send_text(
        &self,
        engine: &SharedEngine,
        room_id: &str,
        body: &str,
        reply_to: Option<String>,
    ) -> Reply {
        self.top_up().await;
        if body.trim().is_empty() {
            return Reply::err("bad_request", "empty message");
        }
        let Some(conv) = self.conversation(room_id).await else {
            return Reply::err("unknown_room", format!("unknown room {room_id}"));
        };
        let reference = reply_to.clone().unwrap_or_default();
        let sent = {
            let mut g = self.inner.lock().await;
            let (a, p) = &mut *g;
            conversation::send_event(
                &self.link,
                a,
                p,
                &conv,
                Kind::Text,
                reference.as_bytes(),
                body.as_bytes(),
            )
            .await
        };
        let sent = match sent {
            Ok(v) => v,
            Err(e) => return Reply::err("network", format!("{e:#}")),
        };
        self.ingest_caught(engine, &conv, sent.caught_up).await;
        let (seq, address) = (sent.seq, sent.address);
        let id = format!("{}:{seq}", hex::encode(address));
        let reply_json = reply_to
            .as_deref()
            .and_then(|eid| self.item_by_id(room_id, eid))
            .map(|i| reply_summary(&i));
        self.append(
            engine,
            room_id,
            text_item(&id, &self.username, now_ms(), true, body, reply_json),
        )
        .await;
        Reply::ok(json!({}))
    }

    async fn send_small(
        &self,
        engine: &SharedEngine,
        room_id: &str,
        kind: Kind,
        reference: &str,
        body: &str,
    ) -> Reply {
        self.top_up().await;
        let Some(conv) = self.conversation(room_id).await else {
            return Reply::err("unknown_room", format!("unknown room {room_id}"));
        };
        let sent = {
            let mut g = self.inner.lock().await;
            let (a, p) = &mut *g;
            conversation::send_event(
                &self.link,
                a,
                p,
                &conv,
                kind,
                reference.as_bytes(),
                body.as_bytes(),
            )
            .await
        };
        let sent = match sent {
            Ok(v) => v,
            Err(e) => return Reply::err("network", format!("{e:#}")),
        };
        self.ingest_caught(engine, &conv, sent.caught_up).await;
        self.apply_small(
            engine,
            room_id,
            kind as u16,
            reference,
            body,
            &self.username.clone(),
            now_ms(),
        )
        .await;
        Reply::ok(json!({}))
    }

    async fn typing(&self, engine: &SharedEngine, room_id: &str, typing: bool) -> Reply {
        if !typing {
            return Reply::ok(json!({}));
        }
        let now = now_ms();
        {
            let mut t = self.typing_sent.lock();
            if t.get(room_id)
                .map(|&last| now - last < 5000)
                .unwrap_or(false)
            {
                return Reply::ok(json!({}));
            }
            t.insert(room_id.to_string(), now);
        }
        self.send_small(engine, room_id, Kind::Typing, "", "").await
    }

    /// A reaction, receipt or typing notice, from us or from a peer.
    async fn apply_small(
        &self,
        engine: &SharedEngine,
        room_id: &str,
        kind: u16,
        reference: &str,
        body: &str,
        sender: &str,
        ts: i64,
    ) {
        if kind == Kind::Typing as u16 {
            if sender != self.username {
                engine.hub.broadcast(json!({"event":"room.typing","roomId":room_id,"users":[{"userId": sender, "displayName": short_name(sender), "avatarPath": ""}]}));
                let (e2, rid) = (engine.clone(), room_id.to_string());
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
                    e2.hub
                        .broadcast(json!({"event":"room.typing","roomId":rid,"users":[]}));
                });
            }
            return;
        }
        let mut changed: Option<(usize, Value, usize)> = None;
        {
            let mut h = self.history.lock();
            if let Some(items) = h.get_mut(room_id) {
                let len = items.len();
                if let Some(idx) = items
                    .iter()
                    .position(|i| i.get("eventId").and_then(Value::as_str) == Some(reference))
                {
                    let it = items[idx].as_object_mut().unwrap();
                    if kind == Kind::Reaction as u16 {
                        let mut reactions = it
                            .get("reactions")
                            .and_then(Value::as_array)
                            .cloned()
                            .unwrap_or_default();
                        let mut found = false;
                        for r in reactions.iter_mut() {
                            if r.get("key").and_then(Value::as_str) == Some(body) {
                                let mut senders: Vec<Value> = r
                                    .get("senders")
                                    .and_then(Value::as_array)
                                    .cloned()
                                    .unwrap_or_default();
                                if senders.iter().any(|s| s.as_str() == Some(sender)) {
                                    senders.retain(|s| s.as_str() != Some(sender));
                                } else {
                                    senders.push(json!(sender));
                                }
                                r["count"] = json!(senders.len());
                                r["senders"] = json!(senders);
                                found = true;
                            }
                        }
                        if !found {
                            reactions.push(json!({"key": body, "count": 1, "senders": [sender]}));
                        }
                        reactions
                            .retain(|r| r.get("count").and_then(Value::as_u64).unwrap_or(0) > 0);
                        it.insert("reactions".into(), json!(reactions));
                    } else if kind == Kind::Receipt as u16 && sender != self.username {
                        let mut read_by: Vec<Value> = it
                            .get("readBy")
                            .and_then(Value::as_array)
                            .cloned()
                            .unwrap_or_default();
                        if !read_by
                            .iter()
                            .any(|r| r.get("userId").and_then(Value::as_str) == Some(sender))
                        {
                            read_by.push(json!({"userId": sender, "ts": ts}));
                        }
                        it.insert("readBy".into(), json!(read_by));
                    }
                    changed = Some((idx, Value::Object(it.clone()), len));
                }
            }
        }
        if let Some((idx, item, len)) = changed {
            self.save_history();
            if self.open.lock().contains(room_id) {
                engine.hub.broadcast(json!({"event":"timeline.diff","roomId":room_id,"ops":[{"op":"set","index":idx,"item":item}],"len":len}));
            }
        }
    }

    /// Existing device: scan an offer; the reply and a `link.state` event
    /// carry the emoji the user must compare with the new device.
    async fn link_scan(&self, engine: &SharedEngine, offer: &str) -> Reply {
        self.top_up().await;
        let scanned = {
            let mut g = self.inner.lock().await;
            sigil_client::linking::scan(&self.link, &mut g.0, offer).await
        };
        match scanned {
            Ok(sc) => {
                let sas = sc.sas.clone();
                *self.pending_scan.lock() = Some(sc);
                engine.hub.broadcast(json!({"event":"link.state","state":"sas","sas":sas}));
                Reply::ok(json!({"sas": sas}))
            }
            Err(e) => Reply::err("network", format!("{e:#}")),
        }
    }

    /// The user compared the emoji. `ok` runs the transfer, which adds the
    /// new device to every conversation and rotates every address.
    async fn link_confirm(&self, engine: &SharedEngine, ok: bool) -> Reply {
        let Some(sc) = self.pending_scan.lock().take() else { return Reply::err("bad_request", "nothing to confirm") };
        if !ok {
            engine.hub.broadcast(json!({"event":"link.state","state":"cancelled"}));
            return Reply::ok(json!({}));
        }
        let extra = std::fs::read(&self.history_path).unwrap_or_default();
        let e2 = engine.clone();
        let result = {
            let mut g = self.inner.lock().await;
            let (a, p) = &mut *g;
            sigil_client::linking::transfer(&self.link, a, p, sc, extra, move |pr| {
                if let sigil_client::linking::Progress::Welcomed(w) = pr {
                    e2.hub.broadcast(json!({"event":"link.state","state":"joining","with":w}));
                }
            })
            .await
        };
        match result {
            Ok(()) => {
                let convs = self.inner.lock().await.0.conversations.clone();
                for c in &convs {
                    self.subscribe_conversation(c).await;
                }
                engine.hub.broadcast(json!({"event":"link.state","state":"done"}));
                Reply::ok(json!({}))
            }
            Err(e) => {
                engine.hub.broadcast(json!({"event":"link.state","state":"failed","error":format!("{e:#}")}));
                Reply::err("network", format!("{e:#}"))
            }
        }
    }

    async fn room_create(&self, engine: &SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
        self.top_up().await;
        let name = param(p, "name");
        let invite: Vec<String> = p.get("invite").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(|u| u.trim().to_lowercase()).collect()).unwrap_or_default();
        for u in &invite {
            if sigil_protocol::names::parse_username(u).is_err() {
                return Reply::err("bad_request", format!("{u} is not a username"));
            }
        }
        let created = {
            let mut g = self.inner.lock().await;
            let (a, pr) = &mut *g;
            sigil_client::group::create(&self.link, a, pr, &name, &invite, "").await
        };
        match created {
            Ok(conv) => {
                self.subscribe_conversation(&conv).await;
                self.mark_dirty();
                self.broadcast_rooms(engine).await;
                Reply::ok(json!({"roomId": conv.group_id}))
            }
            Err(e) => Reply::err("network", format!("{e:#}")),
        }
    }

    async fn room_invite(&self, engine: &SharedEngine, room_id: &str, user_id: &str) -> Reply {
        self.top_up().await;
        let Some(conv) = self.conversation(room_id).await else { return Reply::err("unknown_room", "unknown room") };
        let user = user_id.trim().to_lowercase();
        let r = {
            let mut g = self.inner.lock().await;
            let (a, pr) = &mut *g;
            sigil_client::group::invite(&self.link, a, pr, &conv, &user).await
        };
        match r {
            Ok(()) => {
                if let Some(c) = self.conversation(room_id).await {
                    self.subscribe_conversation(&c).await;
                }
                self.mark_dirty();
                self.broadcast_rooms(engine).await;
                Reply::ok(json!({}))
            }
            Err(e) => Reply::err("network", format!("{e:#}")),
        }
    }

    async fn room_set_settings(&self, engine: &SharedEngine, room_id: &str, p: &serde_json::Map<String, Value>) -> Reply {
        let Some(conv) = self.conversation(room_id).await else { return Reply::err("unknown_room", "unknown room") };
        if let Some(name) = p.get("name").and_then(Value::as_str) {
            let r = {
                let mut g = self.inner.lock().await;
                let (a, pr) = &mut *g;
                sigil_client::group::rename(&self.link, a, pr, &conv, name).await
            };
            if let Err(e) = r {
                return Reply::err("network", format!("{e:#}"));
            }
            self.mark_dirty();
            self.broadcast_rooms(engine).await;
        }
        Reply::ok(json!({}))
    }

    // ------------------------------------------------------------ calls

    /// Pick a room for a call and announce it in the conversation.
    async fn call_start(&self, engine: &SharedEngine, room_id: &str) -> Reply {
        self.top_up().await;
        let Some(conv) = self.conversation(room_id).await else { return Reply::err("unknown_room", "unknown room") };
        let r = {
            let mut g = self.inner.lock().await;
            let (a, pr) = &mut *g;
            sigil_client::call::start(&self.link, a, pr, &conv).await
        };
        match r {
            Ok(room) => {
                let call_id = hex::encode(room);
                self.mark_dirty();
                engine.hub.broadcast(json!({"event":"call.state","roomId":room_id,"callId":call_id,"state":"started","sender":self.username}));
                Reply::ok(json!({"callId": call_id}))
            }
            Err(e) => Reply::err("network", format!("{e:#}")),
        }
    }

    async fn call_end(&self, engine: &SharedEngine, room_id: &str, call_id: &str) -> Reply {
        self.top_up().await;
        let Some(conv) = self.conversation(room_id).await else { return Reply::err("unknown_room", "unknown room") };
        let Some(room) = parse_room(call_id) else { return Reply::err("bad_request", "callId is 32 bytes of hex") };
        let r = {
            let mut g = self.inner.lock().await;
            let (a, pr) = &mut *g;
            sigil_client::call::end(&self.link, a, pr, &conv, &room).await
        };
        match r {
            Ok(()) => {
                self.mark_dirty();
                engine.hub.broadcast(json!({"event":"call.state","roomId":room_id,"callId":call_id,"state":"ended","sender":self.username}));
                Reply::ok(json!({}))
            }
            Err(e) => Reply::err("network", format!("{e:#}")),
        }
    }

    /// One signalling message to the forwarding unit: `join{offer}`,
    /// `poll{peer}`, `answer{peer, answer}`, `leave{peer}`. The unit's JSON
    /// reply is the result.
    async fn call_signal(&self, p: &serde_json::Map<String, Value>, kind: &str) -> Reply {
        if kind == "join" {
            self.top_up().await;
        }
        let room_id = param(p, "roomId");
        let Some(conv) = self.conversation(&room_id).await else { return Reply::err("unknown_room", "unknown room") };
        let Some(room) = parse_room(&param(p, "callId")) else { return Reply::err("bad_request", "callId is 32 bytes of hex") };
        let mut body = serde_json::Map::new();
        body.insert("kind".into(), json!(kind));
        for k in ["offer", "peer", "answer"] {
            if let Some(v) = p.get(k).and_then(Value::as_str) {
                body.insert(k.into(), json!(v));
            }
        }
        let r = {
            let mut g = self.inner.lock().await;
            sigil_client::call::signal(&self.link, &mut g.0, &conv.slot_server, &room, Value::Object(body)).await
        };
        match r {
            Ok(v) => Reply::ok(v),
            Err(e) => Reply::err("network", format!("{e:#}")),
        }
    }

    async fn attachment_send(&self, engine: &SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
        self.top_up().await;
        let room_id = param(p, "roomId");
        let path = std::path::PathBuf::from(param(p, "path"));
        let caption = param(p, "caption");
        let Some(conv) = self.conversation(&room_id).await else { return Reply::err("unknown_room", "unknown room") };
        if !path.is_file() {
            return Reply::err("bad_request", "file not found");
        }
        let r = {
            let mut g = self.inner.lock().await;
            let (a, pr) = &mut *g;
            match sigil_client::media::upload(&self.link, a, &path, &caption).await {
                Ok(m) => conversation::send_event(&self.link, a, pr, &conv, Kind::Media, &[], &serde_json::to_vec(&m).unwrap()).await.map(|s| (s, m)),
                Err(e) => Err(e),
            }
        };
        match r {
            Ok((sent, m)) => {
                self.ingest_caught(engine, &conv, sent.caught_up).await;
                let id = format!("{}:{}", hex::encode(sent.address), sent.seq);
                self.append(engine, &room_id, media_item(&id, &self.username, now_ms(), true, &m, &path.to_string_lossy())).await;
                Reply::ok(json!({}))
            }
            Err(e) => Reply::err("network", format!("{e:#}")),
        }
    }

    /// Where a downloaded file lives in the cache.
    fn media_path(m: &sigil_client::media::Manifest) -> std::path::PathBuf {
        let dir = crate::media::media_dir();
        let stem = m.chunks.first().map(|c| c[..16].to_string()).unwrap_or_default();
        dir.join(format!("{stem}-{}", m.filename.replace('/', "_")))
    }

    async fn media_get(&self, room_id: &str, event_id: &str) -> Reply {
        let Some(item) = self.item_by_id(room_id, event_id) else { return Reply::err("unknown_event", "no such event") };
        let Some(m) = item.get("manifest").and_then(|v| serde_json::from_value::<sigil_client::media::Manifest>(v.clone()).ok()) else {
            return Reply::err("bad_request", "not a media event");
        };
        let path = Self::media_path(&m);
        if !path.is_file() {
            let server = self.conversation(room_id).await.map(|c| c.slot_server).unwrap_or_default();
            if let Err(e) = sigil_client::media::download(&self.link, &server, &m, &path).await {
                return Reply::err("network", format!("{e:#}"));
            }
        }
        Reply::ok(json!({"path": path.to_string_lossy(), "filename": m.filename, "mime": m.mime}))
    }

    /// Download in the background and update the item with the local path.
    fn fetch_media_later(&self, engine: &SharedEngine, room_id: String, event_id: String, m: sigil_client::media::Manifest) {
        let Some(me) = engine.sigil.lock().clone() else { return };
        let e2 = engine.clone();
        tokio::spawn(async move {
            let path = SigilSession::media_path(&m);
            let server = me.conversation(&room_id).await.map(|c| c.slot_server).unwrap_or_default();
            if !path.is_file() {
                if let Err(e) = sigil_client::media::download(&me.link, &server, &m, &path).await {
                    warn!("media download failed: {e:#}");
                    return;
                }
            }
            let updated = {
                let mut h = me.history.lock();
                let Some(items) = h.get_mut(&room_id) else { return };
                let Some(idx) = items.iter().position(|i| i.get("eventId").and_then(Value::as_str) == Some(event_id.as_str())) else { return };
                let it = items[idx].as_object_mut().unwrap();
                if let Some(media) = it.get_mut("media").and_then(Value::as_object_mut) {
                    media.insert("path".into(), json!(path.to_string_lossy()));
                    if m.mime.starts_with("image/") {
                        media.insert("thumbnailPath".into(), json!(path.to_string_lossy()));
                    }
                }
                Some((idx, Value::Object(it.clone()), items.len()))
            };
            if let Some((idx, item, len)) = updated {
                me.save_history();
                if me.open.lock().contains(&room_id) {
                    e2.hub.broadcast(json!({"event":"timeline.diff","roomId":room_id,"ops":[{"op":"set","index":idx,"item":item}],"len":len}));
                }
            }
        });
    }

    /// Draw tokens when the wallet runs low. Called before anything that
    /// spends, so a device never fails for want of a token.
    async fn top_up(&self) {
        let mut g = self.inner.lock().await;
        if let Err(e) = account::ensure_tokens(&self.link, &mut g.0, 20, 60).await {
            warn!("token top-up failed: {e:#}");
        }
    }

    /// Apply what catch-up processed.
    async fn ingest_caught(&self, engine: &SharedEngine, conv: &Conversation, caught: Vec<conversation::Caught>) {
        for c in caught {
            Box::pin(self.apply_incoming(engine, conv, &c.address, c.seq, c.incoming)).await;
        }
    }

    async fn dm_create(&self, engine: &SharedEngine, username: &str) -> Reply {
        self.top_up().await;
        let username = username.trim().to_lowercase();
        if sigil_protocol::names::parse_username(&username).is_err() {
            return Reply::err("bad_request", "userId must look like @name:server");
        }
        if username == self.username {
            return Reply::err("bad_request", "that is you");
        }
        let started = {
            let mut g = self.inner.lock().await;
            if let Some(c) =
                g.0.conversations
                    .iter()
                    .find(|c| c.peers == [username.clone()])
            {
                return Reply::ok(json!({"roomId": c.group_id}));
            }
            let (a, p) = &mut *g;
            conversation::start_dm(&self.link, a, p, &username, "").await
        };
        let conv = match started {
            Ok(c) => c,
            Err(e) => return Reply::err("network", format!("{e:#}")),
        };
        self.subscribe_conversation(&conv).await;
        self.broadcast_rooms(engine).await;
        Reply::ok(json!({"roomId": conv.group_id}))
    }

    async fn accept(&self, engine: &SharedEngine, room_id: &str) -> Reply {
        let Some(rid) = room_id.strip_prefix("req:") else {
            return Reply::err(
                "bad_request",
                "only requests can be joined on the Sigil backend",
            );
        };
        let accepted = {
            let mut g = self.inner.lock().await;
            let Some(req) =
                g.0.requests
                    .iter()
                    .find(|r| r.welcome.starts_with(rid))
                    .cloned()
            else {
                return Reply::err("unknown_room", "no such request");
            };
            let (a, p) = &mut *g;
            conversation::accept(a, p, &req).map(|c| (c, req))
        };
        let (conv, req) = match accepted {
            Ok(v) => v,
            Err(e) => return Reply::err("bad_request", format!("{e:#}")),
        };
        if !req.first_message.is_empty() {
            let id = format!("welcome:{}", &req.welcome[..16]);
            self.append(
                engine,
                &conv.group_id,
                text_item(&id, &req.from, now_ms(), false, &req.first_message, None),
            )
            .await;
        }
        self.subscribe_conversation(&conv).await;
        self.broadcast_rooms(engine).await;
        Reply::ok(json!({"roomId": conv.group_id}))
    }

    async fn leave(&self, engine: &SharedEngine, room_id: &str) -> Reply {
        {
            let mut g = self.inner.lock().await;
            if let Some(rid) = room_id.strip_prefix("req:") {
                g.0.requests.retain(|r| !r.welcome.starts_with(rid));
            } else {
                let (a, p) = &mut *g;
                if let Some(c) = a.conversations.iter().find(|c| c.group_id == room_id).cloned() {
                    let _ = sigil_client::group::leave(&self.link, a, p, &c).await;
                }
                a.conversations.retain(|c| c.group_id != room_id);
                self.history.lock().remove(room_id);
            }
            let _ = g.0.save();
        }
        self.save_history();
        self.broadcast_rooms(engine).await;
        Reply::ok(json!({}))
    }

    async fn paginate(&self, engine: &SharedEngine, room_id: &str) -> Reply {
        let Some(conv) = self.conversation(room_id).await else {
            return Reply::err("unknown_room", "unknown room");
        };
        engine.hub.broadcast(
            json!({"event":"timeline.paginationState","roomId":room_id,"state":"paginating"}),
        );
        let caught = {
            let mut g = self.inner.lock().await;
            let (a, p) = &mut *g;
            conversation::catch_up(&self.link, a, p, &conv).await
        };
        if let Ok(caught) = caught {
            self.ingest_caught(engine, &conv, caught).await;
        }
        engine.hub.broadcast(
            json!({"event":"timeline.paginationState","roomId":room_id,"state":"timelineStart"}),
        );
        Reply::ok(json!({"hitStart": true}))
    }

    // ------------------------------------------------------------ receiving

    async fn subscribe_all(&self) {
        self.top_up().await;
        let convs = self.inner.lock().await.0.conversations.clone();
        for c in &convs {
            self.subscribe_conversation(c).await;
        }
        let handles = {
            let mut g = self.inner.lock().await;
            account::subscribe_requests(&self.link, &mut g.0).await
        };
        match handles {
            Ok(hs) => {
                let period = period_now();
                let mut h = self.handles.lock();
                for (i, handle) in hs.into_iter().enumerate() {
                    h.insert(
                        handle,
                        Handle::Requests(sigil_protocol::names::requests_address(
                            &self.identity_pub,
                            period + i as u32,
                        )),
                    );
                }
            }
            Err(e) => warn!("requests slot subscription failed: {e:#}"),
        }
    }

    async fn subscribe_conversation(&self, c: &Conversation) {
        self.top_up().await;
        let r = {
            let mut g = self.inner.lock().await;
            let (a, p) = &mut *g;
            conversation::subscribe(&self.link, a, p, c).await
        };
        match r {
            Ok((handle, ep)) => {
                self.handles
                    .lock()
                    .insert(handle, Handle::Conversation(c.group_id.clone(), ep.address));
            }
            Err(e) => warn!("subscribe failed for {}: {e:#}", &c.group_id[..8]),
        }
    }

    /// Process one envelope delivered live for `address`, which is the
    /// conversation's current slot or one it recently rotated away from.
    async fn ingest(&self, engine: &SharedEngine, conv: &Conversation, address: &[u8; 32], seq: u64, envelope: &[u8]) {
        let incoming = {
            let mut g = self.inner.lock().await;
            let (a, p) = &mut *g;
            if seq <= conversation::cursor(a, conv, address) {
                return; // already processed by a catch-up
            }
            conversation::set_cursor(a, conv, address, seq);
            let _ = a.save();
            if conversation::own_sent(a, conv, address, seq).is_some() {
                return;
            }
            conversation::receive_at(a, p, conv, address, envelope)
        };
        match incoming {
            Ok(inc) => self.apply_incoming(engine, conv, address, seq, inc).await,
            Err(e) => warn!("cannot process envelope {seq} in {}: {e:#}", &conv.group_id[..8]),
        }
    }

    async fn apply_incoming(&self, engine: &SharedEngine, conv: &Conversation, address: &[u8; 32], seq: u64, incoming: conversation::Incoming) {
        let id = format!("{}:{seq}", hex::encode(address));
        match incoming {
            conversation::Incoming::Text { from_identity, ts_ms, text, reference } => {
                let sender = self.username_for(conv, &from_identity);
                let is_own = from_identity == self.identity_pub;
                let reply = if reference.is_empty() { None } else { self.item_by_id(&conv.group_id, &reference).map(|i| reply_summary(&i)) };
                self.append(engine, &conv.group_id, text_item(&id, &sender, ts_ms as i64, is_own, &text, reply)).await;
            }
            conversation::Incoming::Event { from_identity, ts_ms, kind, reference, body } => {
                let sender = self.username_for(conv, &from_identity);
                if kind == Kind::Media as u16 {
                    if let Ok(m) = serde_json::from_str::<sigil_client::media::Manifest>(&body) {
                        let is_own = from_identity == self.identity_pub;
                        self.append(engine, &conv.group_id, media_item(&id, &sender, ts_ms as i64, is_own, &m, "")).await;
                        self.fetch_media_later(engine, conv.group_id.clone(), id.clone(), m);
                    }
                } else if kind == Kind::Policy as u16 || kind == Kind::Membership as u16 {
                    let change = {
                        let mut g = self.inner.lock().await;
                        let (a, p) = &mut *g;
                        sigil_client::group::apply_control(&self.link, a, p, conv, kind, &from_identity, &body).await
                    };
                    match change {
                        Ok(sigil_client::group::Change::Left { username, .. }) => {
                            let text = format!("{} left", short_name(&username));
                            let item = json!({"id": id, "kind": "membership", "eventId": id, "sender": sender, "senderName": short_name(&sender), "senderAvatarPath": "", "ts": ts_ms, "isOwn": false, "isHighlighted": false, "body": text, "stateText": text, "sendState": "sent", "sendError": "", "readBy": [], "reactions": [], "can": {"edit": false, "reply": false, "redact": false, "react": false}});
                            self.append(engine, &conv.group_id, item).await;
                        }
                        Ok(sigil_client::group::Change::Policy) => self.broadcast_rooms(engine).await,
                        Ok(sigil_client::group::Change::None) => {}
                        Err(e) => warn!("control event ignored: {e:#}"),
                    }
                    // a removal may have rotated the address; catch up and follow
                    if let Some(c) = self.conversation(&conv.group_id).await {
                        let caught = {
                            let mut g = self.inner.lock().await;
                            let (a, p) = &mut *g;
                            conversation::catch_up(&self.link, a, p, &c).await
                        };
                        if let Ok(caught) = caught {
                            if !caught.is_empty() {
                                Box::pin(self.ingest_caught(engine, &c, caught)).await;
                            }
                        }
                        self.subscribe_conversation(&c).await;
                    }
                } else if kind == Kind::Call as u16 {
                    if let Ok(ev) = serde_json::from_str::<sigil_client::call::CallEvent>(&body) {
                        let is_own = from_identity == self.identity_pub;
                        let state = if ev.action == "start" { "started" } else { "ended" };
                        engine.hub.broadcast(json!({"event":"call.state","roomId":conv.group_id,"callId":ev.room,"state":state,"sender":sender}));
                        let text = format!("{} {} a call", short_name(&sender), state);
                        let item = json!({"id": id, "kind": "call", "eventId": id, "sender": sender, "senderName": short_name(&sender), "senderAvatarPath": "", "ts": ts_ms, "isOwn": is_own, "isHighlighted": false, "body": text, "stateText": text, "callId": ev.room, "callState": state, "sendState": "sent", "sendError": "", "readBy": [], "reactions": [], "can": {"edit": false, "reply": false, "redact": false, "react": false}});
                        self.append(engine, &conv.group_id, item).await;
                    }
                } else {
                    self.apply_small(engine, &conv.group_id, kind, &reference, &body, &sender, ts_ms as i64).await;
                }
            }
            conversation::Incoming::Rotated => {
                // Catch up on the new address and follow it.
                let caught = {
                    let mut g = self.inner.lock().await;
                    let (a, p) = &mut *g;
                    conversation::catch_up(&self.link, a, p, conv).await
                };
                if let Ok(caught) = caught {
                    Box::pin(self.ingest_caught(engine, conv, caught)).await;
                }
                self.subscribe_conversation(conv).await;
            }
            conversation::Incoming::Other { .. } => {}
        }
    }

    fn username_for(&self, conv: &Conversation, identity: &[u8; 32]) -> String {
        if *identity == self.identity_pub {
            return self.username.clone();
        }
        let h = hex::encode(identity);
        conv.members
            .iter()
            .find(|m| m.identity == h)
            .map(|m| m.username.clone())
            .or_else(|| conv.peers.first().cloned())
            .unwrap_or_else(|| h[..8].to_string())
    }

    async fn delivery_loop(self: Arc<Self>, engine: SharedEngine) {
        loop {
            let frame = {
                let mut rx = self.link.deliveries.lock().await;
                rx.recv().await
            };
            let Some(Frame::Deliver {
                wake_handle,
                queue_seq,
                slot_seq,
                envelope,
            }) = frame
            else {
                if engine
                    .sigil
                    .lock()
                    .as_ref()
                    .map(|s| !Arc::ptr_eq(s, &self))
                    .unwrap_or(true)
                {
                    return;
                }
                continue;
            };
            let _ = self
                .link
                .tx
                .send(Frame::Ack {
                    wake_handle,
                    queue_seq,
                })
                .await;
            let kind = match self.handles.lock().get(&wake_handle) {
                Some(Handle::Conversation(g, a)) => Handle::Conversation(g.clone(), *a),
                Some(Handle::Requests(a)) => Handle::Requests(*a),
                None => continue,
            };
            match kind {
                Handle::Conversation(gid, address) => {
                    if let Some(conv) = self.conversation(&gid).await {
                        self.ingest(&engine, &conv, &address, slot_seq, &envelope).await;
                    }
                }
                Handle::Requests(address) => {
                    let mut g = self.inner.lock().await;
                    match conversation::open_request(&g.0, &address, &envelope) {
                        Ok(r) => {
                            let mark = hex::encode(sigil_protocol::kdf::hash(r.welcome.as_bytes()));
                            if !g.0.seen_requests.contains(&mark) {
                                g.0.seen_requests.push(mark);
                                g.0.requests.push(r);
                                let _ = g.0.save();
                            }
                            drop(g);
                            self.broadcast_rooms(&engine).await;
                        }
                        Err(e) => warn!("unreadable request: {e:#}"),
                    }
                }
            }
        }
    }
}
