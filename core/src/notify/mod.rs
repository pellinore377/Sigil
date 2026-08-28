//! Desktop notifications for new messages (push rules → notify-send) and `notify` pushes.
use std::sync::atomic::{AtomicI64, Ordering};

use matrix_sdk::{Client, Room};
use ruma::events::room::message::{MessageType, SyncRoomMessageEvent};
use ruma::push::Action;
use serde_json::{json, Value};
use tracing::{debug, warn};

use crate::engine::SharedEngine;

pub static START_TS_MS: AtomicI64 = AtomicI64::new(0);

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub enabled: bool,
    pub dms: bool,
    pub mentions: bool,
    pub calls: bool,
}

impl Default for Settings {
    fn default() -> Self { Settings { enabled: true, dms: true, mentions: true, calls: true } }
}

pub fn settings_path() -> std::path::PathBuf { crate::paths::state_dir().join("settings.json") }

pub fn load_settings() -> Settings {
    std::fs::read(settings_path()).ok().and_then(|d| serde_json::from_slice::<Value>(&d).ok())
        .and_then(|v| serde_json::from_value(v.get("notify").cloned()?).ok()).unwrap_or_default()
}

pub fn save_settings(s: &Settings) {
    let mut v: Value = std::fs::read(settings_path()).ok().and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or_else(|| json!({}));
    v["notify"] = serde_json::to_value(s).unwrap_or(json!({}));
    let _ = std::fs::write(settings_path(), serde_json::to_vec_pretty(&v).unwrap_or_default());
}

pub fn install(engine: SharedEngine, client: Client) {
    START_TS_MS.store(now_ms(), Ordering::Relaxed);
    client.add_event_handler(move |ev: SyncRoomMessageEvent, room: Room, actions: Vec<Action>| {
        let engine = engine.clone();
        async move {
            if let Err(e) = handle(engine, room, ev, actions).await { warn!("notify handler: {e:#}"); }
        }
    });
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

async fn handle(engine: SharedEngine, room: Room, ev: SyncRoomMessageEvent, actions: Vec<Action>) -> anyhow::Result<()> {
    let Some(orig) = ev.as_original() else { return Ok(()) };
    let own = room.own_user_id();
    if orig.sender == own { return Ok(()); }
    let ts = u64::from(orig.origin_server_ts.0) as i64;
    if ts < START_TS_MS.load(Ordering::Relaxed) - 60_000 { return Ok(()); } // initial-sync replay
    let notify = actions.iter().any(|a| a.should_notify());
    if !notify { return Ok(()); }
    let highlight = actions.iter().any(|a| a.is_highlight());
    let is_dm = room.is_direct().await.unwrap_or(false);
    let settings = load_settings();
    if !settings.enabled { return Ok(()); }
    let kind = if highlight { "mention" } else if is_dm { "dm" } else { "message" };
    if kind == "dm" && !settings.dms { return Ok(()); }
    if kind == "mention" && !settings.mentions { return Ok(()); }
    let room_id = room.room_id().to_string();
    let focused = {
        let s = engine.state.lock();
        s.focused_room == room_id && s.focused_visible
    };
    let sender_name = room.get_member_no_sync(&orig.sender).await.ok().flatten().and_then(|m| m.display_name().map(|s| s.to_string())).unwrap_or_else(|| orig.sender.localpart().to_string());
    let room_name = room.cached_display_name().map(|n| n.to_string()).unwrap_or_else(|| room_id.clone());
    let body = match &orig.content.msgtype {
        MessageType::Image(_) => "📷 Image".to_string(),
        MessageType::File(f) => format!("📎 {}", f.body),
        MessageType::Video(_) => "🎬 Video".to_string(),
        MessageType::Audio(_) => "🎤 Audio".to_string(),
        other => other.body().to_string(),
    };
    let summary = if is_dm { sender_name.clone() } else { format!("{sender_name} in {room_name}") };
    let event_id = orig.event_id.to_string();
    engine.hub.broadcast(json!({"event":"notify","roomId":room_id,"eventId":event_id,"kind":kind,"summary":summary,"body":body,"sender":orig.sender.to_string(),"senderName":sender_name,"focused":focused}));
    if focused { debug!("suppressed notification for focused room"); return Ok(()); }
    let avatar = room.avatar_url().map(|u| u.to_string()).unwrap_or_default();
    let icon = crate::media::cached_avatar_path(&engine, &avatar).await;
    let mut cmd = std::process::Command::new("notify-send");
    cmd.arg("-a").arg("Matrix");
    if !icon.is_empty() { cmd.arg("-i").arg(&icon); } else { cmd.arg("-i").arg("chat"); }
    cmd.arg("-h").arg(format!("string:x-omarchy-room:{room_id}"));
    if highlight { cmd.arg("-u").arg("normal"); }
    // `--` first: remote text beginning with "-" would otherwise be read as options.
    cmd.arg("--").arg(strip_controls(&summary)).arg(strip_controls(&body));
    cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    if let Err(e) = cmd.spawn() { warn!("notify-send failed: {e}"); }
    Ok(())
}

pub fn settings_json() -> Value {
    serde_json::to_value(load_settings()).unwrap_or(json!({}))
}

/// Remote text: drop control characters so it cannot rewrite the notification.
fn strip_controls(s: &str) -> String {
    s.chars().filter(|c| !c.is_control() || *c == ' ').take(512).collect()
}
