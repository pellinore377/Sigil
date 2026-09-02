//! Engine: owns the Sigil session and all mutable daemon state, and turns
//! frontend requests into replies and pushed events.

use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::ipc::hub::Hub;
use crate::ipc::wire::{Reply, Request};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    LoggedOut,
    LoginPending,
    Restoring,
    LoggedIn,
}

impl SessionState {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionState::LoggedOut => "loggedOut",
            SessionState::LoginPending => "loginPending",
            SessionState::Restoring => "restoring",
            SessionState::LoggedIn => "loggedIn",
        }
    }
}

#[derive(Default)]
pub struct State {
    pub session: Option<SessionState>,
    /// The Sigil server the account lives on (kept under the old name so
    /// the status event's shape does not change).
    pub homeserver: String,
    pub server_name: String,
    pub user_id: String,
    pub device_id: String,
    pub display_name: String,
    pub avatar_path: String,
    pub sync_state: String,
    pub sync_error: String,
    pub last_error: String,
    pub login_url: String,
    pub verified: bool,
    pub rooms_snapshot: Value,
    pub focused_room: String,
    pub focused_visible: bool,
    /// MapLibre style URL (a local override); empty = no maps.
    pub map_style_url: String,
    /// Last position fix, and why we have none if we have none.
    pub position: Option<crate::geo::Fix>,
    pub position_error: String,
}

pub struct Engine {
    pub hub: Hub,
    pub state: Mutex<State>,
    /// The active Sigil session, if any.
    pub sigil: Mutex<Option<Arc<crate::sigil::SigilSession>>>,
    /// Local video playback (ffmpeg → shm) for the media viewer.
    pub playback: Mutex<Option<crate::media::player::Playback>>,
    /// Voice-message recorder and voice-note playback.
    pub recording: Mutex<Option<crate::media::voice::Recording>>,
    pub audio_play: Mutex<Option<crate::media::voice::AudioPlayback>>,
}

pub type SharedEngine = Arc<Engine>;

impl Engine {
    pub fn new(hub: Hub) -> SharedEngine {
        Arc::new(Engine {
            hub,
            state: Mutex::new(State {
                sync_state: "offline".into(),
                ..Default::default()
            }),
            sigil: Mutex::new(None),
            playback: Mutex::new(None),
            recording: Mutex::new(None),
            audio_play: Mutex::new(None),
        })
    }

    pub fn session(&self) -> SessionState {
        self.state.lock().session.unwrap_or(SessionState::LoggedOut)
    }

    pub fn set_session(&self, s: SessionState) {
        self.state.lock().session = Some(s);
        self.broadcast_status();
    }

    pub fn set_error(&self, msg: impl Into<String>) {
        let msg = msg.into();
        if !msg.is_empty() {
            warn!("{msg}");
        }
        self.state.lock().last_error = msg;
        self.broadcast_status();
    }

    pub fn status_json(&self) -> Value {
        let s = self.state.lock();
        json!({
            "event": "status",
            "backend": "sigil",
            "session": s.session.unwrap_or(SessionState::LoggedOut).as_str(),
            "homeserver": s.homeserver,
            "serverName": s.server_name,
            "userId": s.user_id,
            "deviceId": s.device_id,
            "displayName": s.display_name,
            "avatarPath": s.avatar_path,
            "sync": s.sync_state,
            "syncError": s.sync_error,
            "verified": s.verified,
            "login": { "url": s.login_url },
            "lastError": s.last_error,
            "mapStyleUrl": s.map_style_url,
            "accountSaved": crate::sigil::has_account(),
        })
    }

    pub fn broadcast_status(&self) {
        self.hub.broadcast(self.status_json());
    }

    /// Everything a freshly connected client needs.
    pub fn greeting(&self) -> Vec<Value> {
        let mut v = vec![self.status_json(), crate::sigil::recovery_status_json()];
        let s = self.state.lock();
        if !s.rooms_snapshot.is_null() {
            v.push(s.rooms_snapshot.clone());
        }
        drop(s);
        v.push(crate::geo::position_json(self));
        v
    }

    fn str_param(p: &serde_json::Map<String, Value>, k: &str) -> String {
        p.get(k).and_then(Value::as_str).unwrap_or("").to_string()
    }

    pub async fn dispatch(self: &Arc<Self>, req: Request) -> Reply {
        // Account and conversation requests belong to the Sigil session.
        if let Some(reply) = crate::sigil::dispatch(self, &req).await {
            return reply;
        }
        let p = &req.params;
        match req.req.as_str() {
            "ping" => Reply::ok(json!({"pong": true})),
            "status" => Reply::ok(self.status_json()),
            "voice.start" => crate::media::voice_start(self.clone(), p).await,
            "voice.stop" => crate::media::voice_stop(self.clone(), p).await,
            "voice.cancel" => crate::media::voice_cancel(self.clone(), p).await,
            "audio.play" => crate::media::audio_play(self.clone(), p).await,
            "audio.playFile" => crate::media::audio_play_file(self.clone(), p).await,
            "audio.stop" => crate::media::audio_stop(self.clone(), p).await,
            "video.play" => crate::media::video_play(self.clone(), p).await,
            "video.seek" => crate::media::video_seek(self.clone(), p).await,
            "video.stop" => crate::media::video_stop(self.clone(), p).await,
            "position.get" => Reply::ok(crate::geo::position_json(self)),
            "position.refresh" => {
                crate::geo::refresh(self);
                Reply::ok(crate::geo::position_json(self))
            }
            "map.config" => Reply::ok(crate::maps::config_json(self)),
            "map.setStyle" => {
                match crate::maps::set_style(self, &Self::str_param(p, "url")).await {
                    Ok(()) => Reply::ok(crate::maps::config_json(self)),
                    Err(e) => Reply::err("bad_request", e),
                }
            }
            "sigiltext.motion" => Reply::ok(crate::timeline::motion::all()),
            "ui.focus" => {
                let mut s = self.state.lock();
                s.focused_room = Self::str_param(p, "roomId");
                s.focused_visible = p.get("visible").and_then(Value::as_bool).unwrap_or(false);
                Reply::ok(json!({}))
            }
            "notify.settings" => {
                if p.len() > 0
                    && p.keys()
                        .any(|k| ["enabled", "dms", "mentions", "calls"].contains(&k.as_str()))
                {
                    let mut s = crate::notify::load_settings();
                    if let Some(b) = p.get("enabled").and_then(Value::as_bool) {
                        s.enabled = b;
                    }
                    if let Some(b) = p.get("dms").and_then(Value::as_bool) {
                        s.dms = b;
                    }
                    if let Some(b) = p.get("mentions").and_then(Value::as_bool) {
                        s.mentions = b;
                    }
                    if let Some(b) = p.get("calls").and_then(Value::as_bool) {
                        s.calls = b;
                    }
                    crate::notify::save_settings(&s);
                }
                Reply::ok(crate::notify::settings_json())
            }
            r if r.starts_with("call.") => Reply::err("unsupported", "calls arrive with Phase 7"),
            r if r.starts_with("login.") => Reply::err(
                "unsupported",
                "use account.create (or account.link, account.recover when they land)",
            ),
            other => Reply::err("bad_request", format!("unknown request '{other}'")),
        }
    }

    pub async fn startup(self: &Arc<Self>) {
        if crate::sigil::restore(self).await {
            info!("sigil session restored");
        } else {
            info!("no saved account");
            self.set_session(SessionState::LoggedOut);
        }
    }
}
