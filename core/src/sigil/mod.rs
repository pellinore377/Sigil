//! The Sigil session inside the engine: an account on a Sigil server and
//! its MLS conversations, presented to frontends through the same `status`,
//! `rooms.list`, `timeline.reset` and `timeline.diff` events the engine has
//! always emitted. Frontends do not know what is underneath, by design.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, Weak};

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
    Conversation(String),
    Requests([u8; 32]),
}

pub struct SigilSession {
    me: OnceLock<Weak<SigilSession>>,
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
}

fn account_path() -> PathBuf {
    crate::paths::state_dir().join("sigil-account.json")
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
    let link = Arc::new(Link::connect(&acct.envoy, &acct.device_id).await?);
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
        me: OnceLock::new(),
        inner: AsyncMutex::new((acct, provider)),
        link,
        history: Mutex::new(history),
        handles: Mutex::new(HashMap::new()),
        open: Mutex::new(HashSet::new()),
        history_path,
        typing_sent: Mutex::new(HashMap::new()),
        username,
        identity_pub,
    });
    let _ = session.me.set(Arc::downgrade(&session));
    *engine.sigil.lock() = Some(session.clone());
    engine.set_session(SessionState::LoggedIn);
    session.subscribe_all().await;
    session.broadcast_rooms(engine).await;
    let (e2, s2) = (engine.clone(), session.clone());
    tokio::spawn(async move { s2.delivery_loop(e2).await });
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
    let result: anyhow::Result<()> = async {
        let mut acct = Account::create(&path, &username, &envoy)?;
        let provider = SigilProvider::open(&acct.mls_path())?;
        let link = Link::connect(&envoy, &acct.device_id).await?;
        account::register(&link, &mut acct, &invite).await?;
        account::publish_key_packages(&link, &mut acct, &provider, 10).await?;
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
        "recovery.status" | "recovery.recover" => return Some(Reply::ok(recovery_status_json())),
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
        "room.join" => s.accept(engine, &param(p, "roomIdOrAlias")).await,
        "room.leave" => s.leave(engine, &param(p, "roomId")).await,
        "room.members" => s.members(&param(p, "roomId")).await,
        "users.search" => s.search(&param(p, "query")).await,
        r if r.starts_with("room.")
            || r.starts_with("message.")
            || r.starts_with("space.")
            || r.starts_with("thread")
            || r.starts_with("pins.")
            || r.starts_with("poll.")
            || r.starts_with("media.")
            || r.starts_with("attachment.")
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

fn room_json(c: &Conversation, hist: &[Value]) -> Value {
    let peer = c.peers.first().cloned().unwrap_or_default();
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
        "name": short_name(&peer),
        "topic": "",
        "avatarUrl": "",
        "avatarPath": "",
        "canonicalAlias": "",
        "isDm": c.peers.len() == 1,
        "dmUserId": if c.peers.len() == 1 { Some(peer.clone()) } else { None },
        "isSpace": false,
        "spaceParents": [],
        "isEncrypted": true,
        "isInvite": false,
        "inviter": Value::Null,
        "isFavourite": false,
        "isLowPriority": false,
        "joinedMembers": c.peers.len() + 1,
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
    fn arc(&self) -> Arc<SigilSession> {
        self.me
            .get()
            .and_then(Weak::upgrade)
            .expect("session is held by the engine")
    }

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
        let mut members: Vec<Value> = c.peers.iter().map(|p| json!({"userId": p, "displayName": short_name(p), "avatarPath": "", "powerLevel": 100, "membership": "join"})).collect();
        members.push(json!({"userId": self.username, "displayName": short_name(&self.username), "avatarPath": "", "powerLevel": 100, "membership": "join"}));
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
        let (seq, address) = match sent {
            Ok(v) => v,
            Err(e) => return Reply::err("network", format!("{e:#}")),
        };
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
        if let Err(e) = sent {
            return Reply::err("network", format!("{e:#}"));
        }
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

    async fn dm_create(&self, engine: &SharedEngine, username: &str) -> Reply {
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
                g.0.conversations.retain(|c| c.group_id != room_id);
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
        let items = {
            let g = self.inner.lock().await;
            conversation::backfill(&self.link, &g.1, &conv, 0).await
        };
        if let Ok(items) = items {
            for (seq, env) in items {
                self.ingest(engine, &conv, seq, &env).await;
            }
        }
        engine.hub.broadcast(
            json!({"event":"timeline.paginationState","roomId":room_id,"state":"timelineStart"}),
        );
        Reply::ok(json!({"hitStart": true}))
    }

    // ------------------------------------------------------------ receiving

    async fn subscribe_all(&self) {
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
        let r = {
            let mut g = self.inner.lock().await;
            let (a, p) = &mut *g;
            conversation::subscribe(&self.link, a, p, c).await
        };
        match r {
            Ok((handle, _)) => {
                self.handles
                    .lock()
                    .insert(handle, Handle::Conversation(c.group_id.clone()));
            }
            Err(e) => warn!("subscribe failed for {}: {e:#}", &c.group_id[..8]),
        }
    }

    /// Process one envelope from a conversation's slot.
    async fn ingest(&self, engine: &SharedEngine, conv: &Conversation, seq: u64, envelope: &[u8]) {
        let (id, incoming) = {
            let g = self.inner.lock().await;
            let (a, p) = &*g;
            let Ok(group) = conversation::load_group(p, conv) else {
                return;
            };
            let Ok(ep) = conversation::epoch_material(&group, p) else {
                return;
            };
            let id = format!("{}:{seq}", hex::encode(ep.address));
            if conversation::own_sent(a, conv, &ep.address, seq).is_some()
                || self.item_by_id(&conv.group_id, &id).is_some()
            {
                return;
            }
            (id, conversation::receive(p, conv, envelope))
        };
        match incoming {
            Ok(conversation::Incoming::Text {
                from_identity,
                ts_ms,
                text,
                reference,
            }) => {
                let sender = self.username_for(conv, &from_identity);
                let reply = if reference.is_empty() {
                    None
                } else {
                    self.item_by_id(&conv.group_id, &reference)
                        .map(|i| reply_summary(&i))
                };
                self.append(
                    engine,
                    &conv.group_id,
                    text_item(&id, &sender, ts_ms as i64, false, &text, reply),
                )
                .await;
            }
            Ok(conversation::Incoming::Event {
                from_identity,
                ts_ms,
                kind,
                reference,
                body,
            }) => {
                let sender = self.username_for(conv, &from_identity);
                self.apply_small(
                    engine,
                    &conv.group_id,
                    kind,
                    &reference,
                    &body,
                    &sender,
                    ts_ms as i64,
                )
                .await;
            }
            Ok(conversation::Incoming::Rotated) => {
                let (me, c2) = (self.arc(), conv.clone());
                tokio::spawn(async move { me.subscribe_conversation(&c2).await });
            }
            Ok(conversation::Incoming::Other { .. }) => {}
            Err(e) => warn!(
                "cannot process envelope {seq} in {}: {e:#}",
                &conv.group_id[..8]
            ),
        }
    }

    fn username_for(&self, conv: &Conversation, identity: &[u8; 32]) -> String {
        if *identity == self.identity_pub {
            return self.username.clone();
        }
        conv.peers
            .first()
            .cloned()
            .unwrap_or_else(|| hex::encode(&identity[..4]))
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
                Some(Handle::Conversation(g)) => Handle::Conversation(g.clone()),
                Some(Handle::Requests(a)) => Handle::Requests(*a),
                None => continue,
            };
            match kind {
                Handle::Conversation(gid) => {
                    if let Some(conv) = self.conversation(&gid).await {
                        self.ingest(&engine, &conv, slot_seq, &envelope).await;
                    }
                }
                Handle::Requests(address) => {
                    let mut g = self.inner.lock().await;
                    match conversation::open_request(&g.0, &address, &envelope) {
                        Ok(r) => {
                            let known = g.0.requests.iter().any(|x| x.welcome == r.welcome)
                                || g.0
                                    .conversations
                                    .iter()
                                    .any(|c| c.peers == [r.from.clone()]);
                            if !known {
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
