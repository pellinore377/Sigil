//! CallManager: one MatrixRTC call at a time (state machine, signalling, keys, LiveKit session).
pub mod camera;
pub mod e2ee;
pub mod livekit_room;
pub mod screen;
pub mod signaling;
pub mod transport;

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use matrix_sdk::{Client, Room};
use parking_lot::Mutex;
use ruma::events::rtc::notification::{NotificationType, RtcNotificationEventContent};
use ruma::events::{SyncMessageLikeEvent, ToDeviceEvent};
use ruma::RoomId;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;
use tracing::{debug, info, warn};

use crate::engine::{Engine, SharedEngine};
use crate::ipc::wire::Reply;
use livekit_room::{LkSession, SessionEvent};
use signaling::{Msc4075NotificationContent, RtcEncryptionKeyEventContent};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallState { Idle, Ringing, Joining, Connected, Reconnecting, Leaving, Failed }

impl CallState {
    fn as_str(&self) -> &'static str {
        match self { CallState::Idle => "idle", CallState::Ringing => "ringing", CallState::Joining => "joining", CallState::Connected => "connected", CallState::Reconnecting => "reconnecting", CallState::Leaving => "leaving", CallState::Failed => "failed" }
    }
}

#[derive(Clone, Debug, Default)]
pub struct TrackInfo { pub key: String, pub kind: String, pub path: String, pub width: u32, pub height: u32 }

#[derive(Clone, Debug, Default)]
pub struct Participant {
    pub identity: String, pub user_id: String, pub device_id: String, pub display_name: String, pub avatar_path: String,
    pub mic_muted: bool, pub camera_on: bool, pub screen_sharing: bool, pub speaking: bool, pub level: f32, pub quality: String,
    pub tracks: Vec<TrackInfo>,
}

#[derive(Default)]
struct Inner {
    state: Option<CallState>,
    step: String,
    room_id: String,
    video: bool,
    since_ms: u64,
    encrypted: bool,
    error: String,
    incoming: Option<Value>,
    session: Option<Arc<LkSession>>,
    keys: Option<e2ee::KeyState>,
    participants: HashMap<String, Participant>,
    local_speaking: bool,
    local_level: f32,
    local_tracks: Vec<TrackInfo>,
    tasks: Vec<AbortHandle>,
    service_url: String,
    selected_mic: String,
    selected_speaker: String,
    selected_camera: String,
    pending_keys: Vec<(u64, String, String, i32, Vec<u8>)>, // (ts, room_id, sender, index, key)
    rotate_pending: bool,
    outgoing_notification: String,
}

pub struct CallManager {
    inner: Mutex<Inner>,
    http: reqwest::Client,
    engine: Mutex<Weak<Engine>>,
    delayed_events: std::sync::atomic::AtomicBool,
}

impl CallManager {
    pub fn new() -> Arc<Self> {
        Arc::new(CallManager { inner: Mutex::new(Inner { state: Some(CallState::Idle), ..Default::default() }), http: reqwest::Client::new(), engine: Mutex::new(Weak::new()), delayed_events: std::sync::atomic::AtomicBool::new(false) })
    }
    pub fn attach(&self, engine: &SharedEngine) { *self.engine.lock() = Arc::downgrade(engine); }
    fn engine(&self) -> Option<SharedEngine> { self.engine.lock().upgrade() }
    fn state(&self) -> CallState { self.inner.lock().state.clone().unwrap_or(CallState::Idle) }

    /// In a call? A call merely ringing at us does not count.
    pub fn in_call(&self) -> bool {
        matches!(self.state(), CallState::Joining | CallState::Connected | CallState::Reconnecting | CallState::Leaving)
    }

    fn broadcast(&self) {
        if let Some(e) = self.engine() { e.hub.broadcast(self.state_json()); }
    }

    pub fn state_json(&self) -> Value {
        let s = self.inner.lock();
        let (mic_muted, camera_on, screen) = match &s.session { Some(l) => (l.mic_muted(), l.camera_on(), l.screen_on()), None => (false, false, false) };
        let mut parts: Vec<Value> = s.participants.values().map(|p| json!({
            "participantId": p.identity, "userId": p.user_id, "deviceId": p.device_id, "displayName": p.display_name, "avatarPath": p.avatar_path,
            "micMuted": p.mic_muted, "cameraOn": p.camera_on, "screenSharing": p.screen_sharing, "speaking": p.speaking, "level": p.level, "quality": p.quality,
            "tracks": p.tracks.iter().map(|t| json!({"key": t.key, "kind": t.kind, "shmPath": t.path, "width": t.width, "height": t.height})).collect::<Vec<_>>(),
        })).collect();
        parts.sort_by(|a, b| a["participantId"].as_str().cmp(&b["participantId"].as_str()));
        json!({
            "event": "call.state",
            "state": s.state.clone().unwrap_or(CallState::Idle).as_str(),
            "step": s.step,
            "roomId": s.room_id,
            "intent": if s.video { "video" } else { "audio" },
            "since": s.since_ms,
            "encrypted": s.encrypted,
            "error": s.error,
            "local": {
                "participantId": s.session.as_ref().map(|l| l.identity.clone()).unwrap_or_default(),
                "micMuted": mic_muted, "cameraOn": camera_on, "screenSharing": screen, "speaking": s.local_speaking, "level": s.local_level,
                "tracks": s.local_tracks.iter().map(|t| json!({"key": t.key, "kind": t.kind, "shmPath": t.path, "width": t.width, "height": t.height})).collect::<Vec<_>>(),
            },
            "incoming": s.incoming,
            "participants": parts,
            "delayedEvents": self.delayed_events.load(std::sync::atomic::Ordering::Relaxed),
        })
    }

    pub async fn dispatch(self: &Arc<Self>, req: &str, p: &serde_json::Map<String, Value>) -> Reply {
        let room_id = p.get("roomId").and_then(Value::as_str).unwrap_or("").to_string();
        match req {
            "call.devices" => self.devices().await,
            "call.setDevice" => {
                let kind = p.get("kind").and_then(Value::as_str).unwrap_or("");
                let id = p.get("id_").or_else(|| p.get("id")).and_then(Value::as_str).unwrap_or("").to_string();
                self.set_device(kind, id).await
            }
            "call.start" | "call.join" => {
                let video = p.get("video").and_then(Value::as_bool).unwrap_or(false);
                self.start(room_id, video).await
            }
            "call.decline" => self.decline(room_id).await,
            "call.leave" => { self.leave("hangup").await; Reply::ok(json!({})) }
            "call.mute" => {
                let muted = p.get("muted").and_then(Value::as_bool).unwrap_or(true);
                let sess = self.inner.lock().session.clone();
                match sess { Some(l) => { l.set_mic_muted(muted); self.broadcast(); Reply::ok(json!({})) } None => Reply::err("no_call", "no active call") }
            }
            "call.camera" => {
                let on = p.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                self.set_camera(on).await
            }
            "call.screenshare" => {
                let on = p.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                self.set_screen(on).await
            }
            "call.react" => {
                let emoji = p.get("emoji").and_then(Value::as_str).unwrap_or("").to_string();
                if emoji.is_empty() { return Reply::err("bad_request", "which emoji?") }
                let sess = self.inner.lock().session.clone();
                match sess {
                    Some(l) => match l.send_reaction(&emoji).await {
                        // LiveKit does not loop data packets back to the publisher.
                        Ok(()) => {
                            if let Some(e) = self.engine() {
                                let me = l.identity.clone();
                                let (user_id, _) = split_identity(&me);
                                e.hub.broadcast(json!({
                                    "event": "call.reaction", "identity": me, "userId": user_id,
                                    "displayName": "You", "emoji": emoji, "own": true
                                }));
                            }
                            Reply::ok(json!({}))
                        }
                        Err(e) => Reply::err("network", e.to_string()),
                    },
                    None => Reply::err("no_call", "no active call"),
                }
            }
            "call.state" => Reply::ok(self.state_json()),
            other => Reply::err("bad_request", format!("unknown request '{other}'")),
        }
    }

    /// Device ids are PipeWire node ids; the libwebrtc ADM follows the PipeWire default.
    async fn devices(self: &Arc<Self>) -> Reply {
        let (mics, speakers) = tokio::task::spawn_blocking(pw_audio_nodes).await.unwrap_or((vec![], vec![]));
        let cameras = camera::list();
        let sel = |list: &Vec<Value>| -> String {
            list.iter().find(|e| e.get("default").and_then(Value::as_bool).unwrap_or(false)).and_then(|e| e.get("id").and_then(Value::as_str)).unwrap_or("").to_string()
        };
        let s = self.inner.lock();
        Reply::ok(json!({"mics": mics, "speakers": speakers, "cameras": cameras, "selected": {"mic": sel(&mics), "speaker": sel(&speakers), "camera": s.selected_camera}}))
    }

    async fn set_device(self: &Arc<Self>, kind: &str, id: String) -> Reply {
        let sess = {
            let mut s = self.inner.lock();
            match kind { "mic" => s.selected_mic = id.clone(), "speaker" => s.selected_speaker = id.clone(), "camera" => s.selected_camera = id.clone(), _ => return Reply::err("bad_request", "kind must be mic|speaker|camera") }
            s.session.clone()
        };
        match kind {
            "mic" | "speaker" => {
                // Moving the PipeWire default also moves live streams.
                let kind2 = kind.to_string();
                let id2 = id.clone();
                let node_name = tokio::task::spawn_blocking(move || {
                    let (mics, speakers) = pw_audio_nodes();
                    let list = if kind2 == "mic" { mics } else { speakers };
                    list.iter().find(|e| e.get("id").and_then(Value::as_str) == Some(id2.as_str())).and_then(|e| e.get("nodeName").and_then(Value::as_str).map(|s| s.to_string()))
                }).await.ok().flatten();
                let _ = &node_name;
                let out = tokio::process::Command::new("wpctl").args(["set-default", &id]).output().await;
                match out {
                    Ok(o) if o.status.success() => {}
                    Ok(o) => return Reply::err("device_error", String::from_utf8_lossy(&o.stderr).trim().to_string()),
                    Err(e) => return Reply::err("device_error", format!("wpctl: {e}")),
                }
            }
            "camera" => {
                if let Some(l) = sess {
                    if l.camera_on() { let _ = self.set_camera(false).await; let _ = self.set_camera(true).await; }
                }
            }
            _ => {}
        }
        Reply::ok(json!({}))
    }

    async fn set_camera(self: &Arc<Self>, on: bool) -> Reply {
        let (sess, dev) = { let s = self.inner.lock(); (s.session.clone(), s.selected_camera.clone()) };
        let Some(l) = sess else { return Reply::err("no_call", "no active call") };
        let me = self.clone();
        let r = l.set_camera(on, &dev, move |err| { me.inner.lock().error = err; me.broadcast(); }).await;
        {
            let mut s = self.inner.lock();
            s.local_tracks.retain(|t| t.kind != "camera");
            if on { if let Some(w) = l.preview.lock().as_ref() { let (mw, mh) = w.max_size(); s.local_tracks.push(TrackInfo { key: "local-camera".into(), kind: "camera".into(), path: w.path().to_string_lossy().into_owned(), width: mw, height: mh }); } }
        }
        self.broadcast();
        match r { Ok(()) => Reply::ok(json!({})), Err(e) => Reply::err("device_error", format!("{e:#}")) }
    }

    async fn set_screen(self: &Arc<Self>, on: bool) -> Reply {
        let sess = self.inner.lock().session.clone();
        let Some(l) = sess else { return Reply::err("no_call", "no active call") };
        let me = self.clone();
        let r = l.set_screen(on, move |err| { me.inner.lock().error = err; me.broadcast(); }).await;
        {
            let mut s = self.inner.lock();
            s.local_tracks.retain(|t| t.kind != "screen");
            if on && r.is_ok() { if let Some(w) = l.screen_preview.lock().as_ref() { let (mw, mh) = w.max_size(); s.local_tracks.push(TrackInfo { key: "local-screen".into(), kind: "screen".into(), path: w.path().to_string_lossy().into_owned(), width: mw, height: mh }); } }
            if let Err(e) = &r { s.error = format!("screen share: {e:#}"); }
        }
        self.broadcast();
        match r { Ok(()) => Reply::ok(json!({})), Err(e) => Reply::err("device_error", format!("{e:#}")) }
    }

    async fn decline(self: &Arc<Self>, room_id: String) -> Reply {
        let incoming = self.inner.lock().incoming.take();
        let Some(inc) = incoming else { return Reply::err("no_call", "no incoming call") };
        if self.state() == CallState::Ringing { self.inner.lock().state = Some(CallState::Idle); }
        self.broadcast();
        let notif = inc.get("notificationEventId").and_then(Value::as_str).unwrap_or("").to_string();
        let rid = if room_id.is_empty() { inc.get("roomId").and_then(Value::as_str).unwrap_or("").to_string() } else { room_id };
        if let (Some(e), Ok(rid)) = (self.engine(), RoomId::parse(&rid)) {
            if let Some(room) = e.client().and_then(|c| c.get_room(&rid)) {
                if !notif.is_empty() { let _ = signaling::send_decline(&room, &notif).await; }
            }
        }
        Reply::ok(json!({}))
    }

    async fn start(self: &Arc<Self>, room_id: String, video: bool) -> Reply {
        let Some(engine) = self.engine() else { return Reply::err("internal", "engine gone") };
        let Some(client) = engine.client() else { return Reply::err("not_logged_in", "not logged in") };
        let Ok(rid) = RoomId::parse(&room_id) else { return Reply::err("bad_request", "invalid roomId") };
        let Some(room) = client.get_room(&rid) else { return Reply::err("unknown_room", "unknown room") };
        match self.state() {
            CallState::Idle | CallState::Ringing | CallState::Failed => {}
            _ => return Reply::err("call_busy", "already in a call"),
        }
        {
            let mut s = self.inner.lock();
            s.state = Some(CallState::Joining);
            s.step = "discover".into();
            s.room_id = room_id.clone();
            s.video = video;
            s.error.clear();
            s.participants.clear();
            s.local_tracks.clear();
            s.since_ms = now_ms();
            s.incoming = None;
            s.encrypted = room.encryption_state().is_encrypted();
            for t in s.tasks.drain(..) { t.abort(); }
        }
        self.broadcast();
        let me = self.clone();
        tokio::spawn(async move {
            if let Err(e) = me.join_flow(client, room, video).await {
                warn!("call failed: {e:#}");
                me.fail(format!("{e:#}")).await;
            }
        });
        Reply::ok(json!({}))
    }

    async fn fail(self: &Arc<Self>, msg: String) {
        self.leave_inner(false).await;
        { let mut s = self.inner.lock(); s.state = Some(CallState::Failed); s.error = msg; }
        self.broadcast();
        let me = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(6)).await;
            let mut s = me.inner.lock();
            if s.state == Some(CallState::Failed) { s.state = Some(CallState::Idle); s.error.clear(); drop(s); me.broadcast(); }
        });
    }

    fn set_step(&self, step: &str) { self.inner.lock().step = step.into(); self.broadcast(); }

    async fn join_flow(self: &Arc<Self>, client: Client, room: Room, video: bool) -> anyhow::Result<()> {
        let device_id = client.device_id().map(|d| d.to_string()).unwrap_or_default();
        let service_url = match self.inner.lock().service_url.clone() { s if !s.is_empty() => s, _ => String::new() };
        let service_url = if service_url.is_empty() { transport::discover_service_url(&client, &self.http).await? } else { service_url };
        self.inner.lock().service_url = service_url.clone();

        self.set_step("token");
        let openid = transport::openid_token(&client).await?;
        let lk = transport::fetch_jwt(&self.http, &service_url, room.room_id().as_str(), &openid, &device_id).await?;
        let identity = transport::jwt_sub(&lk.jwt).unwrap_or_else(|| format!("{}:{}", room.own_user_id(), device_id));
        debug!("rtc: identity {identity}, sfu {}", lk.server_url);

        // Signal membership BEFORE joining the SFU so Element can map our tracks.
        self.set_step("signal");
        let others = signaling::other_active_members(&room).await;
        let _ = signaling::send_call_open(&room).await;
        let expires = Some(std::time::Duration::from_secs(4 * 3600));
        // created_ts must stay stable: a differing refresh rotates media keys.
        let created_ts = ruma::MilliSecondsSinceUnixEpoch::now();
        signaling::send_member_join(&room, &device_id, &identity, &service_url, video, expires, created_ts).await?;
        if others.is_empty() {
            match signaling::send_ring(&room, video, &device_id).await {
                Ok(eid) => self.inner.lock().outgoing_notification = eid,
                Err(e) => warn!("ring failed: {e:#}"),
            }
        }
        // Keep the membership alive: delayed leave (MSC4140) if supported, plus periodic refresh.
        {
            let client2 = client.clone();
            let room2 = room.clone();
            let dev2 = device_id.clone();
            let (svc2, id2) = (service_url.clone(), identity.clone());
            let created_ts = created_ts;
            let me = self.clone();
            let h = tokio::spawn(async move {
                let mut delay_id: Option<String> = None;
                if me.delayed_events.load(std::sync::atomic::Ordering::Relaxed) {
                    match signaling::schedule_delayed_leave(&client2, &room2, &dev2, std::time::Duration::from_secs(8)).await {
                        Ok(id) => { info!("rtc: delayed leave scheduled {id}"); delay_id = Some(id) }
                        Err(e) => warn!("rtc: delayed leave unavailable: {e:#}"),
                    }
                }
                let mut tick = 0u32;
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    if let Some(id) = &delay_id {
                        if let Err(e) = signaling::update_delayed(&client2, id, ruma::api::client::delayed_events::update_delayed_event::unstable::UpdateAction::Restart).await { warn!("delayed restart: {e:#}"); }
                    }
                    tick += 1;
                    // Refresh well inside the 4 h expiry, with identical content.
                    if tick % 48 == 0 {
                        let _ = signaling::send_member_join(&room2, &dev2, &id2, &svc2, video, expires, created_ts).await;
                    }
                }
            });
            self.inner.lock().tasks.push(h.abort_handle());
        }

        self.set_step("connect");
        let encrypted = room.encryption_state().is_encrypted();
        let (tx, rx) = mpsc::unbounded_channel::<SessionEvent>();
        let (mic_name, speaker_name) = audio_settings();
        let session = LkSession::connect(&lk.server_url, &lk.jwt, encrypted, (Some(mic_name), Some(speaker_name)), tx).await?;
        if session.identity != identity {
            debug!("rtc: identity mismatch {identity} vs {}", session.identity);
            signaling::send_member_join(&room, &device_id, &session.identity, &service_url, video, expires, created_ts).await?;
        }
        let identity = session.identity.clone();
        {
            let mut s = self.inner.lock();
            s.session = Some(session.clone());
            s.state = Some(CallState::Connected);
            s.step = "publish".into();
        }
        if encrypted {
            let mut keys = e2ee::KeyState::new();
            session.set_key(&identity, 0, keys.own_key.clone());
            let b64 = keys.own_b64();
            e2ee::broadcast_key(&client, &room, &identity, &b64, keys.own_index).await;
            let pending: Vec<_> = { let mut s = self.inner.lock(); let rid = room.room_id().to_string(); let now = now_ms(); s.pending_keys.retain(|(ts, _, _, _, _)| now - *ts < 120_000); s.pending_keys.iter().filter(|(_, r, _, _, _)| *r == rid).cloned().collect() };
            for (_, _, sender, index, key) in pending {
                keys.peer.entry(sender.clone()).or_default().insert(index, key.clone());
                session.set_key_for_user(&sender, index, &key);
            }
            self.inner.lock().keys = Some(keys);
        }
        if video {
            let dev = self.inner.lock().selected_camera.clone();
            let me = self.clone();
            if let Err(e) = session.set_camera(true, &dev, move |err| { me.inner.lock().error = err; me.broadcast(); }).await {
                warn!("camera: {e:#}");
                self.inner.lock().error = format!("camera unavailable: {e}");
            } else if let Some(w) = session.preview.lock().as_ref() {
                let (mw, mh) = w.max_size();
                self.inner.lock().local_tracks.push(TrackInfo { key: "local-camera".into(), kind: "camera".into(), path: w.path().to_string_lossy().into_owned(), width: mw, height: mh });
            }
        }
        self.broadcast();
        let me = self.clone();
        let h = tokio::spawn(async move { me.session_events(client, room, rx).await });
        self.inner.lock().tasks.push(h.abort_handle());
        Ok(())
    }

    async fn session_events(self: &Arc<Self>, client: Client, room: Room, mut rx: mpsc::UnboundedReceiver<SessionEvent>) {
        while let Some(ev) = rx.recv().await {
            tracing::debug!("rtc event: {ev:?}");
            match ev {
                SessionEvent::Reaction { identity, emoji } => {
                    let (user_id, _) = split_identity(&identity);
                    let name = {
                        let s = self.inner.lock();
                        s.participants.get(&identity).map(|p| p.display_name.clone()).unwrap_or_default()
                    };
                    if let Some(e) = self.engine() {
                        e.hub.broadcast(json!({
                            "event": "call.reaction",
                            "identity": identity,
                            "userId": user_id,
                            "displayName": name,
                            "emoji": emoji,
                        }));
                    }
                }
                SessionEvent::ParticipantJoined { identity } => {
                    debug!("rtc: participant joined {identity}");
                    let (user_id, device_id) = split_identity(&identity);
                    let (name, avatar) = member_profile(&room, &user_id).await;
                    let avatar_path = match self.engine() { Some(e) => crate::media::cached_avatar_path(&e, &avatar).await, None => String::new() };
                    let (session, keys) = {
                        let mut s = self.inner.lock();
                        s.participants.insert(identity.clone(), Participant { identity: identity.clone(), user_id: user_id.clone(), device_id, display_name: name, avatar_path, quality: "good".into(), ..Default::default() });
                        (s.session.clone(), s.keys.as_ref().map(|k| (k.own_b64(), k.own_index, k.peer.get(&user_id).cloned())))
                    };
                    if let (Some(sess), Some((b64, idx, peer))) = (session, keys) {
                        let _ = e2ee::send_key_to_user(&client, room.room_id().as_str(), &sess.identity, &b64, idx, &ruma::OwnedUserId::try_from(user_id.as_str()).unwrap_or_else(|_| room.own_user_id().to_owned())).await;
                        if let Some(map) = peer { for (i, k) in map { sess.set_key(&identity, i, k); } }
                    }
                    self.broadcast();
                }
                SessionEvent::ParticipantLeft { identity } => {
                    debug!("rtc: participant left {identity}");
                    self.inner.lock().participants.remove(&identity);
                    self.broadcast();
                    self.schedule_rotation(client.clone(), room.clone());
                }
                SessionEvent::TrackAdded { identity, kind, key, path, width, height } => {
                    let mut s = self.inner.lock();
                    if let Some(p) = s.participants.get_mut(&identity) {
                        p.tracks.retain(|t| t.key != key);
                        p.tracks.push(TrackInfo { key, kind: kind.into(), path, width, height });
                        if kind == "camera" { p.camera_on = true } else if kind == "screen" { p.screen_sharing = true }
                    }
                    drop(s);
                    self.broadcast();
                }
                SessionEvent::TrackRemoved { identity, key } => {
                    let mut s = self.inner.lock();
                    if let Some(p) = s.participants.get_mut(&identity) {
                        p.tracks.retain(|t| t.key != key);
                        p.camera_on = p.tracks.iter().any(|t| t.kind == "camera");
                        p.screen_sharing = p.tracks.iter().any(|t| t.kind == "screen");
                    }
                    drop(s);
                    self.broadcast();
                }
                SessionEvent::Muted { identity, kind, muted } => {
                    let mut s = self.inner.lock();
                    if let Some(p) = s.participants.get_mut(&identity) {
                        match kind { "mic" => p.mic_muted = muted, "camera" => p.camera_on = !muted, "screen" => p.screen_sharing = !muted, _ => {} }
                    }
                    drop(s);
                    self.broadcast();
                }
                SessionEvent::Speaking { levels } => {
                    let mut s = self.inner.lock();
                    let me = s.session.as_ref().map(|l| l.identity.clone()).unwrap_or_default();
                    for p in s.participants.values_mut() {
                        match levels.iter().find(|(id, _)| id == &p.identity) {
                            Some((_, lvl)) => { p.speaking = true; p.level = *lvl; }
                            None => { p.speaking = false; p.level = 0.0; }
                        }
                    }
                    match levels.iter().find(|(id, _)| id == &me) {
                        Some((_, lvl)) => { s.local_speaking = true; s.local_level = *lvl; }
                        None => { s.local_speaking = false; s.local_level = 0.0; }
                    }
                    drop(s);
                    self.broadcast();
                }
                SessionEvent::Quality { identity, quality } => {
                    let mut s = self.inner.lock();
                    if let Some(p) = s.participants.get_mut(&identity) { p.quality = quality; }
                }
                SessionEvent::Reconnecting => { warn!("rtc: livekit reconnecting"); self.inner.lock().state = Some(CallState::Reconnecting); self.broadcast(); }
                SessionEvent::Reconnected => { info!("rtc: livekit reconnected"); self.inner.lock().state = Some(CallState::Connected); self.broadcast(); }
                SessionEvent::Disconnected { reason } => {
                    info!("rtc: disconnected: {reason}");
                    self.leave("disconnected").await;
                    break;
                }
            }
        }
    }

    fn schedule_rotation(self: &Arc<Self>, client: Client, room: Room) {
        {
            let mut s = self.inner.lock();
            if s.keys.is_none() || s.rotate_pending { return; }
            s.rotate_pending = true;
        }
        let me = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            let (sess, next) = {
                let mut s = me.inner.lock();
                s.rotate_pending = false;
                let sess = s.session.clone();
                let Some(k) = s.keys.as_mut() else { return };
                let (idx, key) = k.rotate();
                let b64 = k.own_b64();
                (sess, (idx, key, b64))
            };
            if let Some(sess) = sess {
                sess.set_key(&sess.identity, next.0 as i32, next.1);
                e2ee::broadcast_key(&client, &room, &sess.identity, &next.2, next.0).await;
                info!("rtc: rotated frame key to index {}", next.0);
            }
        });
    }

    pub async fn leave(self: &Arc<Self>, reason: &str) {
        if matches!(self.state(), CallState::Idle | CallState::Ringing) { return; }
        self.inner.lock().state = Some(CallState::Leaving);
        self.broadcast();
        self.leave_inner(true).await;
        { let mut s = self.inner.lock(); s.state = Some(CallState::Idle); s.step.clear(); if reason != "hangup" { s.error = reason.into(); } }
        self.broadcast();
    }

    async fn leave_inner(self: &Arc<Self>, send_leave: bool) {
        let (session, tasks, room_id) = {
            let mut s = self.inner.lock();
            (s.session.take(), std::mem::take(&mut s.tasks), s.room_id.clone())
        };
        for t in tasks { t.abort(); }
        if let Some(l) = session { l.disconnect().await; }
        { let mut s = self.inner.lock(); s.participants.clear(); s.local_tracks.clear(); s.keys = None; }
        if send_leave {
            if let (Some(e), Ok(rid)) = (self.engine(), RoomId::parse(&room_id)) {
                if let Some(client) = e.client() {
                    if let Some(room) = client.get_room(&rid) {
                        let dev = client.device_id().map(|d| d.to_string()).unwrap_or_default();
                        if let Err(err) = signaling::send_member_leave(&room, &dev).await { warn!("leave event: {err:#}"); }
                    }
                }
            }
        }
    }

    fn on_key(&self, room_id: String, sender: String, index: i32, key: Vec<u8>) {
        let mut s = self.inner.lock();
        let active = s.room_id == room_id && s.session.is_some();
        if active {
            if let Some(k) = s.keys.as_mut() { k.peer.entry(sender.clone()).or_default().insert(index, key.clone()); }
            if let Some(sess) = s.session.clone() {
                let n = sess.set_key_for_user(&sender, index, &key);
                debug!("e2ee: key from {sender} index {index} applied to {n} participant(s)");
            }
        } else {
            s.pending_keys.push((now_ms(), room_id, sender, index, key));
        }
    }

    async fn on_ring(self: &Arc<Self>, room: Room, sender: String, video: Option<bool>, remaining_ms: u64, notif_event_id: String) {
        if !matches!(self.state(), CallState::Idle | CallState::Failed) { return; }
        let (name, _) = member_profile(&room, &sender).await;
        let room_name = room.cached_display_name().map(|n| n.to_string()).unwrap_or_else(|| room.room_id().to_string());
        let inc = json!({
            "roomId": room.room_id().to_string(), "roomName": room_name, "callerId": sender, "callerName": name,
            "intent": match video { Some(true) => "video", Some(false) => "audio", None => "audio" },
            "expiresAt": now_ms() + remaining_ms, "notificationEventId": notif_event_id,
        });
        { let mut s = self.inner.lock(); s.state = Some(CallState::Ringing); s.incoming = Some(inc.clone()); }
        if let Some(e) = self.engine() {
            let mut ev = inc.clone(); ev["event"] = json!("call.incoming");
            e.hub.broadcast(ev);
            let settings = crate::notify::load_settings();
            if settings.enabled && settings.calls {
                let _ = std::process::Command::new("notify-send").args(["-a", "Sigil", "-u", "critical", "-t", "30000", "-i", "call-start"]).arg(format!("Incoming {} call", inc["intent"].as_str().unwrap_or("audio"))).arg(format!("{} · {}", inc["callerName"].as_str().unwrap_or(""), inc["roomName"].as_str().unwrap_or(""))).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
            }
        }
        self.broadcast();
        let me = self.clone();
        let rid = room.room_id().to_string();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(remaining_ms)).await;
            let mut s = me.inner.lock();
            if s.state == Some(CallState::Ringing) && s.incoming.as_ref().and_then(|i| i["roomId"].as_str()) == Some(rid.as_str()) {
                s.state = Some(CallState::Idle); s.incoming = None; drop(s); me.broadcast();
            }
        });
    }
}

fn now_ms() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64 }

/// (mic node.name, speaker node.name) remembered in settings.json under "audio".
fn audio_settings() -> (String, String) {
    let v: Value = std::fs::read(crate::notify::settings_path()).ok().and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or(Value::Null);
    (
        v.pointer("/audio/mic").and_then(Value::as_str).unwrap_or("").to_string(),
        v.pointer("/audio/speaker").and_then(Value::as_str).unwrap_or("").to_string(),
    )
}

fn save_audio_setting(kind: &str, node_name: &str) {
    let path = crate::notify::settings_path();
    let mut v: Value = std::fs::read(&path).ok().and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or_else(|| json!({}));
    if !v.is_object() { v = json!({}); }
    if v.get("audio").map(|a| !a.is_object()).unwrap_or(true) { v["audio"] = json!({}); }
    v["audio"][if kind == "mic" { "mic" } else { "speaker" }] = json!(node_name);
    let _ = std::fs::write(&path, serde_json::to_vec_pretty(&v).unwrap_or_default());
}

/// (mics, speakers) from `pw-dump`; ids are usable with `wpctl set-default`.
fn pw_audio_nodes() -> (Vec<Value>, Vec<Value>) {
    let out = std::process::Command::new("pw-dump").output();
    let Ok(out) = out else { return (vec![], vec![]) };
    let Ok(v) = serde_json::from_slice::<Value>(&out.stdout) else { return (vec![], vec![]) };
    let Some(arr) = v.as_array() else { return (vec![], vec![]) };
    let mut default_source = String::new();
    let mut default_sink = String::new();
    for o in arr {
        if o.get("type").and_then(Value::as_str) == Some("PipeWire:Interface:Metadata") {
            for m in o.get("metadata").and_then(Value::as_array).unwrap_or(&vec![]) {
                let key = m.get("key").and_then(Value::as_str).unwrap_or("");
                let name = m.get("value").and_then(|x| x.get("name")).and_then(Value::as_str).unwrap_or("");
                if key == "default.audio.source" { default_source = name.to_string(); }
                if key == "default.audio.sink" { default_sink = name.to_string(); }
            }
        }
    }
    let mut mics = Vec::new();
    let mut speakers = Vec::new();
    for o in arr {
        if o.get("type").and_then(Value::as_str) != Some("PipeWire:Interface:Node") { continue; }
        let props = o.pointer("/info/props").cloned().unwrap_or(Value::Null);
        let class = props.get("media.class").and_then(Value::as_str).unwrap_or("");
        if class != "Audio/Source" && class != "Audio/Sink" { continue; }
        let id = o.get("id").and_then(Value::as_u64).unwrap_or(0);
        let node_name = props.get("node.name").and_then(Value::as_str).unwrap_or("");
        let desc = props.get("node.description").and_then(Value::as_str).or_else(|| props.get("node.nick").and_then(Value::as_str)).unwrap_or(node_name);
        let entry = json!({"id": id.to_string(), "name": desc, "nodeName": node_name, "default": node_name == if class == "Audio/Source" { default_source.as_str() } else { default_sink.as_str() }});
        if class == "Audio/Source" { mics.push(entry) } else { speakers.push(entry) }
    }
    (mics, speakers)
}

fn split_identity(identity: &str) -> (String, String) {
    // "@user:server:DEVICE" → user id keeps its own colon; device is the last segment.
    match identity.rfind(':') {
        Some(i) if identity.starts_with('@') && identity[..i].contains(':') => (identity[..i].to_string(), identity[i + 1..].to_string()),
        _ => (identity.to_string(), String::new()),
    }
}

async fn member_profile(room: &Room, user_id: &str) -> (String, String) {
    let Ok(uid) = ruma::OwnedUserId::try_from(user_id) else { return (user_id.to_string(), String::new()) };
    match room.get_member_no_sync(&uid).await {
        Ok(Some(m)) => (m.display_name().map(|s| s.to_string()).unwrap_or_else(|| uid.localpart().to_string()), m.avatar_url().map(|u| u.to_string()).unwrap_or_default()),
        _ => (uid.localpart().to_string(), String::new()),
    }
}

/// Install inbound handlers (once per login).
pub fn install(engine: SharedEngine, client: Client) {
    let cm = engine.rtc.clone();
    cm.attach(&engine);
    crate::shm::sweep();
    {
        let cm2 = cm.clone();
        let c2 = client.clone();
        tokio::spawn(async move {
            if let Ok(f) = c2.unstable_features().await {
                let on = f.iter().any(|x| x.as_str() == "org.matrix.msc4140");
                cm2.delayed_events.store(on, std::sync::atomic::Ordering::Relaxed);
                info!("rtc: delayed events supported: {on}");
            }
        });
    }
    {
        let cm2 = cm.clone();
        // Element's call-reaction wire format; `debug` because these lines carry room ids and MXIDs.
        client.add_event_handler(|ev: ruma::events::AnyToDeviceEvent| async move {
            let t = ev.event_type().to_string();
            if t.contains("call") || t.contains("rtc") || t.contains("reaction") {
                debug!(target: "sigil_engine::capture", "to-device: type={t}");
            }
        });
        client.add_event_handler(|ev: ruma::events::AnySyncMessageLikeEvent, room: Room| async move {
            let t = ev.event_type().to_string();
            if t.contains("call") || t.contains("rtc") || t.contains("reaction") {
                debug!(target: "sigil_engine::capture",
                       "room event: type={t} room={} sender={}", room.room_id(), ev.sender());
            }
        });

        client.add_event_handler(move |ev: ToDeviceEvent<RtcEncryptionKeyEventContent>| {
            let cm = cm2.clone();
            async move {
                if let Some(key) = e2ee::decode_key(&ev.content.keys.key) {
                    cm.on_key(ev.content.room_id.clone(), ev.sender.to_string(), ev.content.keys.index as i32, key);
                } else {
                    debug!("e2ee: undecodable key from {}", ev.sender);
                }
            }
        });
    }
    {
        let cm2 = cm.clone();
        let c2 = client.clone();
        client.add_event_handler(move |ev: SyncMessageLikeEvent<RtcNotificationEventContent>, room: Room| {
            let cm = cm2.clone();
            let client = c2.clone();
            async move {
                let SyncMessageLikeEvent::Original(o) = ev else { return };
                if Some(o.sender.as_ref()) == client.user_id() { return; }
                if o.content.notification_type != NotificationType::Ring { return; }
                let exp = u64::from(o.content.expiration_ts(o.origin_server_ts, None).get());
                let remaining = exp.saturating_sub(now_ms()).min(120_000);
                if remaining == 0 { return; }
                let video = o.content.call_intent.as_ref().map(|i| matches!(i, ruma::events::rtc::notification::CallIntent::Video));
                cm.on_ring(room, o.sender.to_string(), video, remaining, o.event_id.to_string()).await;
            }
        });
        let cm3 = cm.clone();
        let c3 = client.clone();
        client.add_event_handler(move |ev: SyncMessageLikeEvent<Msc4075NotificationContent>, room: Room| {
            let cm = cm3.clone();
            let client = c3.clone();
            async move {
                let SyncMessageLikeEvent::Original(o) = ev else { return };
                if Some(o.sender.as_ref()) == client.user_id() { return; }
                if o.content.notification_type != "ring" { return; }
                let exp = o.content.sender_ts.saturating_add(o.content.lifetime);
                let remaining = exp.saturating_sub(now_ms()).min(120_000);
                if remaining == 0 { return; }
                let video = o.content.call_intent.as_deref().map(|i| i == "video");
                cm.on_ring(room, o.sender.to_string(), video, remaining, o.event_id.to_string()).await;
            }
        });
    }
    // Membership changes: end ringing when the caller gives up; refresh room list badges.
    {
        let cm2 = cm.clone();
        let e2 = engine.clone();
        client.add_event_handler(move |_ev: ruma::events::SyncStateEvent<ruma::events::call::member::CallMemberEventContent>, room: Room| {
            let cm = cm2.clone();
            let engine = e2.clone();
            async move {
                engine.request_rooms_refresh();
                let ringing_here = { let s = cm.inner.lock(); s.state == Some(CallState::Ringing) && s.incoming.as_ref().and_then(|i| i["roomId"].as_str()) == Some(room.room_id().as_str()) };
                if ringing_here && signaling::other_active_members(&room).await.is_empty() {
                    { let mut s = cm.inner.lock(); s.state = Some(CallState::Idle); s.incoming = None; }
                    cm.broadcast();
                }
            }
        });
    }
}
