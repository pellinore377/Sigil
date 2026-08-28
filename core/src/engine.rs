//! Engine: owns the Matrix client, sync service and all mutable daemon state.
use std::sync::Arc;

use matrix_sdk::Client;
use matrix_sdk_ui::sync_service::SyncService;
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
    pub client: Option<Client>,
    pub sync: Option<Arc<SyncService>>,
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
    pub login_cancel: Option<tokio::sync::oneshot::Sender<()>>,
    pub verified: bool,
    pub rooms_snapshot: Value,
    pub space_index: crate::sync::rooms::SpaceIndex,
    pub timelines: crate::timeline::OpenTimelines,
    pub rooms_refresh: Option<tokio::sync::mpsc::UnboundedSender<()>>,
    pub focused_room: String,
    pub focused_visible: bool,
    /// Last presence map, replayed to clients on connect so dots are not blank.
    pub presence_snapshot: Value,
    /// MapLibre style URL (MSC3488 or a local override); empty = no maps.
    pub map_style_url: String,
    /// Last position fix, and why we have none if we have none.
    pub position: Option<crate::geo::Fix>,
    pub position_error: String,
    /// The live-location share, if one is running. At most one at a time.
    pub live_share: Option<crate::timeline::beacon::LiveShare>,
}

pub struct Engine {
    pub hub: Hub,
    pub state: Mutex<State>,
    #[cfg(feature = "calls")]
    pub rtc: Arc<crate::rtc::CallManager>,
    /// Local video playback (ffmpeg → shm) for the media viewer.
    pub playback: Mutex<Option<crate::media::player::Playback>>,
    /// Voice-message recorder and voice-note playback.
    pub recording: Mutex<Option<crate::media::voice::Recording>>,
    pub audio_play: Mutex<Option<crate::media::voice::AudioPlayback>>,
}

pub type SharedEngine = Arc<Engine>;

impl Engine {
    pub fn new(hub: Hub) -> SharedEngine {
        let e = Arc::new(Engine {
            hub,
            state: Mutex::new(State { sync_state: "offline".into(), ..Default::default() }),
            #[cfg(feature = "calls")]
            rtc: crate::rtc::CallManager::new(),
            playback: Mutex::new(None),
            recording: Mutex::new(None),
            audio_play: Mutex::new(None),
        });
        #[cfg(feature = "calls")]
        e.rtc.attach(&e);
        e
    }

    pub fn session(&self) -> SessionState {
        self.state.lock().session.unwrap_or(SessionState::LoggedOut)
    }

    pub fn client(&self) -> Option<Client> {
        self.state.lock().client.clone()
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
        })
    }

    pub fn broadcast_status(&self) {
        self.hub.broadcast(self.status_json());
    }

    /// Everything a freshly connected client needs.
    pub fn greeting(&self) -> Vec<Value> {
        let mut v = vec![self.status_json()];
        v.push(crate::session::recovery::status_json(self));
        let s = self.state.lock();
        if !s.rooms_snapshot.is_null() { v.push(s.rooms_snapshot.clone()); }
        if !s.space_index.tree.is_null() { v.push(s.space_index.tree.clone()); }
        drop(s);
        #[cfg(feature = "calls")]
        v.push(self.rtc.state_json());
        let p = self.state.lock().presence_snapshot.clone();
        if !p.is_null() { v.push(p); }
        v.push(crate::geo::position_json(self));
        v.push(crate::timeline::beacon::state_json(self));
        v
    }

    /// Ask the room-list task to rebuild and re-broadcast (e.g. after spaces changed).
    pub fn request_rooms_refresh(&self) {
        if let Some(tx) = &self.state.lock().rooms_refresh { let _ = tx.send(()); }
    }

    fn str_param(p: &serde_json::Map<String, Value>, k: &str) -> String {
        p.get(k).and_then(Value::as_str).unwrap_or("").to_string()
    }

    pub async fn dispatch(self: &Arc<Self>, req: Request) -> Reply {
        let p = &req.params;
        match req.req.as_str() {
            "ping" => Reply::ok(json!({"pong": true})),
            "status" => Reply::ok(self.status_json()),
            "login.start" => {
                let hs = p.get("homeserver").and_then(Value::as_str).unwrap_or("").trim().to_string();
                let open = p.get("openBrowser").and_then(Value::as_bool).unwrap_or(true);
                crate::session::login_start(self.clone(), hs, open).await
            }
            "login.cancel" => crate::session::login_cancel(self).await,
            "login.finish" => {
                let q = p.get("query").and_then(Value::as_str).unwrap_or("").to_string();
                crate::session::login_finish_manual(self.clone(), q).await
            }
            "logout" => {
                let wipe = p.get("wipe").and_then(Value::as_bool).unwrap_or(false);
                crate::session::logout(self.clone(), wipe).await
            }
            "recovery.status" => Reply::ok(crate::session::recovery::status_json(self)),
            "recovery.recover" => {
                let key = p.get("key").and_then(Value::as_str).unwrap_or("").to_string();
                crate::session::recovery::recover(self.clone(), key).await
            }
            "rooms.list" => {
                let snap = self.state.lock().rooms_snapshot.clone();
                if snap.is_null() { Reply::ok(json!({"loaded": false, "rooms": []})) } else { Reply::ok(snap) }
            }
            "spaces.tree" => Reply::ok(self.state.lock().space_index.tree.clone()),
            "room.members" => crate::sync::members::members(self.clone(), Self::str_param(p, "roomId")).await,
            "room.join" => crate::sync::members::join(self.clone(), Self::str_param(p, "roomIdOrAlias")).await,
            "room.leave" => crate::sync::members::leave(self.clone(), Self::str_param(p, "roomId")).await,
            "room.invite" => crate::sync::members::invite(self.clone(), Self::str_param(p, "roomId"), Self::str_param(p, "userId")).await,
            "room.create" => crate::sync::members::create(self.clone(), p).await,
            "dm.create" => crate::sync::members::create_dm(self.clone(), Self::str_param(p, "userId")).await,
            "space.hierarchy" => crate::sync::settings::hierarchy(self.clone(), p).await,
            "room.settings" => crate::sync::settings::settings(self.clone(), Self::str_param(p, "roomId")).await,
            "room.setSettings" => crate::sync::settings::set_settings(self.clone(), p).await,
            "room.setAvatar" => crate::sync::settings::set_avatar(self.clone(), p).await,
            "room.setPowerLevel" => crate::sync::settings::set_power_level(self.clone(), p).await,
            "space.addRoom" => crate::sync::members::space_set_child(self.clone(), p, true).await,
            "space.removeRoom" => crate::sync::members::space_set_child(self.clone(), p, false).await,
            "users.search" => crate::sync::members::search_users(self.clone(), Self::str_param(p, "query"), p.get("limit").and_then(Value::as_u64).unwrap_or(10)).await,
            "room.setFavourite" => crate::sync::members::set_favourite(self.clone(), Self::str_param(p, "roomId"), p.get("favourite").and_then(Value::as_bool).unwrap_or(true)).await,
            "room.setLowPriority" => crate::sync::members::set_low_priority(self.clone(), Self::str_param(p, "roomId"), p.get("lowPriority").and_then(Value::as_bool).unwrap_or(true)).await,
            "room.setUnread" => crate::sync::members::set_unread(self.clone(), Self::str_param(p, "roomId"), p.get("unread").and_then(Value::as_bool).unwrap_or(true)).await,
            "voice.start" => crate::media::voice_start(self.clone(), p).await,
            "voice.stop" => crate::media::voice_stop(self.clone(), p).await,
            "voice.cancel" => crate::media::voice_cancel(self.clone(), p).await,
            "voice.send" => crate::media::voice_send(self.clone(), p).await,
            "audio.info" => crate::media::audio_info(self.clone(), p).await,
            "audio.play" => crate::media::audio_play(self.clone(), p).await,
            "audio.playFile" => crate::media::audio_play_file(self.clone(), p).await,
            "audio.stop" => crate::media::audio_stop(self.clone(), p).await,
            "video.play" => crate::media::video_play(self.clone(), p).await,
            "video.seek" => crate::media::video_seek(self.clone(), p).await,
            "video.stop" => crate::media::video_stop(self.clone(), p).await,
            "link.preview" => crate::media::link_preview(self.clone(), p).await,
            "message.editCaption" => crate::timeline::actions::edit_caption(self.clone(), Self::str_param(p, "roomId"), p).await,
            "message.retry" => crate::timeline::actions::retry(self.clone(), Self::str_param(p, "roomId"), p).await,
            "message.cancelSend" => crate::timeline::actions::cancel_send(self.clone(), Self::str_param(p, "roomId"), p).await,
            "room.markRead" => crate::timeline::actions::mark_read(self.clone(), Self::str_param(p, "roomId")).await,
            "typing" => crate::sync::members::typing(self.clone(), Self::str_param(p, "roomId"), p.get("typing").and_then(Value::as_bool).unwrap_or(false)).await,
            "room.open" => crate::timeline::open(self.clone(), Self::str_param(p, "roomId"), p.get("initialItems").and_then(Value::as_u64).unwrap_or(60) as usize).await,
            "room.close" => crate::timeline::close(self.clone(), Self::str_param(p, "roomId")).await,
            // Threads and pins are keyed by room id plus a focus; the UI passes that key back as `roomId`.
            "thread.open" => crate::timeline::open_thread(self.clone(), Self::str_param(p, "roomId"), Self::str_param(p, "rootId"), p.get("initialItems").and_then(Value::as_u64).unwrap_or(60) as usize).await,
            "threads.list" => crate::timeline::threads::list(self.clone(), Self::str_param(p, "roomId")).await,
            "pins.list" => crate::timeline::pins::list(self.clone(), Self::str_param(p, "roomId")).await,
            "pins.items" => crate::timeline::pins::items(self.clone(), Self::str_param(p, "roomId")).await,
            "message.pin" => crate::timeline::pins::pin(self.clone(), Self::str_param(p, "roomId"), p).await,
            "message.unpin" => crate::timeline::pins::unpin(self.clone(), Self::str_param(p, "roomId"), p).await,
            "timeline.paginate" => crate::timeline::paginate(self.clone(), Self::str_param(p, "roomId"), p.get("count").and_then(Value::as_u64).unwrap_or(50) as u16).await,
            "poll.create" => crate::timeline::extras::create_poll(self.clone(), Self::str_param(p, "roomId"), p).await,
            "doc.preview" => crate::media::doc_preview(self.clone(), p).await,
            "doc.thumb" => crate::media::doc_thumb(self.clone(), p).await,
            "vcard.read" => crate::media::vcard_read(self.clone(), p).await,
            "contact.vcf" => crate::media::contact_vcf(self.clone(), p).await,
            "contact.send" => crate::timeline::actions::send_contact(self.clone(), p).await,
            "contacts.list" => crate::timeline::contacts::list(self.clone()).await,
            "contacts.save" => crate::timeline::contacts::save(self.clone(), p).await,
            "contacts.remove" => crate::timeline::contacts::remove(self.clone(), p).await,
            "doc.page" => crate::media::doc_page(self.clone(), p).await,
            "position.get" => Reply::ok(crate::geo::position_json(self)),
            "position.refresh" => { crate::geo::refresh(self); Reply::ok(crate::geo::position_json(self)) }
            "location.startLive" => crate::timeline::beacon::start(self.clone(), p).await,
            "location.stopLive" => crate::timeline::beacon::stop(self.clone()).await,
            "location.liveState" => Reply::ok(crate::timeline::beacon::state_json(self)),
            "map.config" => Reply::ok(crate::maps::config_json(self)),
            "map.setStyle" => match crate::maps::set_style(self, &Self::str_param(p, "url")).await {
                Ok(()) => Reply::ok(crate::maps::config_json(self)),
                Err(e) => Reply::err("bad_request", e),
            },
            "poll.vote" => crate::timeline::extras::vote_poll(self.clone(), Self::str_param(p, "roomId"), p).await,
            "poll.end" => crate::timeline::extras::end_poll(self.clone(), Self::str_param(p, "roomId"), p).await,
            "location.send" => crate::timeline::extras::send_location(self.clone(), Self::str_param(p, "roomId"), p).await,
            "sticker.send" => crate::timeline::extras::send_sticker(self.clone(), Self::str_param(p, "roomId"), p).await,
            "stickers.list" => crate::timeline::extras::list_stickers(self.clone()).await,
            "message.send" => crate::timeline::actions::send(self.clone(), Self::str_param(p, "roomId"), p).await,
            "message.reply" => crate::timeline::actions::reply(self.clone(), Self::str_param(p, "roomId"), p).await,
            "message.edit" => crate::timeline::actions::edit(self.clone(), Self::str_param(p, "roomId"), p).await,
            "message.react" => crate::timeline::actions::react(self.clone(), Self::str_param(p, "roomId"), p).await,
            "message.redact" => crate::timeline::actions::redact(self.clone(), Self::str_param(p, "roomId"), p).await,
            "readReceipt" => crate::timeline::actions::read_receipt(self.clone(), Self::str_param(p, "roomId"), p).await,
            "sigiltext.motion" => Reply::ok(crate::timeline::motion::all()),
            #[cfg(feature = "calls")]
            r if r.starts_with("call.") => self.rtc.dispatch(r, p).await,
            #[cfg(not(feature = "calls"))]
            r if r.starts_with("call.") => Reply::err("unsupported", "this build has no call support"),
            "media.get" => crate::media::get(self.clone(), p).await,
            "media.saveAs" => crate::media::save_as(self.clone(), p).await,
            "attachment.send" => crate::media::send_attachment(self.clone(), p).await,
            "ui.focus" => {
                let mut s = self.state.lock();
                s.focused_room = Self::str_param(p, "roomId");
                s.focused_visible = p.get("visible").and_then(Value::as_bool).unwrap_or(false);
                Reply::ok(json!({}))
            }
            "notify.settings" => {
                if p.len() > 0 && p.keys().any(|k| ["enabled","dms","mentions","calls"].contains(&k.as_str())) {
                    let mut s = crate::notify::load_settings();
                    if let Some(b) = p.get("enabled").and_then(Value::as_bool) { s.enabled = b; }
                    if let Some(b) = p.get("dms").and_then(Value::as_bool) { s.dms = b; }
                    if let Some(b) = p.get("mentions").and_then(Value::as_bool) { s.mentions = b; }
                    if let Some(b) = p.get("calls").and_then(Value::as_bool) { s.calls = b; }
                    crate::notify::save_settings(&s);
                }
                Reply::ok(crate::notify::settings_json())
            }
            other => Reply::err("bad_request", format!("unknown request '{other}'")),
        }
    }

    pub async fn startup(self: &Arc<Self>) {
        // Retry with backoff: a saved session must survive boot-time DNS hiccups.
        let mut delay = std::time::Duration::from_secs(5);
        loop {
            match crate::session::restore(self.clone()).await {
                Ok(true) => { info!("session restored"); return }
                Ok(false) => { info!("no saved session"); self.set_session(SessionState::LoggedOut); return }
                Err(e) => {
                    self.set_error(format!("session restore failed (retrying in {}s): {e:#}", delay.as_secs()));
                    self.set_session(SessionState::LoggedOut);
                }
            }
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(std::time::Duration::from_secs(300));
            // A login started meanwhile wins.
            if !matches!(self.session(), SessionState::LoggedOut) { return }
        }
    }
}
