//! Notification settings. Delivery of desktop notifications for the Sigil
//! backend lands with Phase 3 (devices and push).

use serde_json::{json, Value};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub enabled: bool,
    pub dms: bool,
    pub mentions: bool,
    pub calls: bool,
    /// Per-conversation modes, `all | mentions | mute`; absent = the account
    /// default above. Local to this device.
    #[serde(default)]
    pub rooms: std::collections::BTreeMap<String, String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            enabled: true,
            dms: true,
            mentions: true,
            calls: true,
            rooms: Default::default(),
        }
    }
}

pub fn settings_path() -> std::path::PathBuf {
    crate::paths::state_dir().join("settings.json")
}

pub fn load_settings() -> Settings {
    std::fs::read(settings_path())
        .ok()
        .and_then(|d| serde_json::from_slice::<Value>(&d).ok())
        .and_then(|v| serde_json::from_value(v.get("notify").cloned()?).ok())
        .unwrap_or_default()
}

pub fn save_settings(s: &Settings) {
    let mut v: Value = std::fs::read(settings_path())
        .ok()
        .and_then(|d| serde_json::from_slice(&d).ok())
        .unwrap_or_else(|| json!({}));
    v["notify"] = serde_json::to_value(s).unwrap_or(json!({}));
    let _ = std::fs::write(
        settings_path(),
        serde_json::to_vec_pretty(&v).unwrap_or_default(),
    );
}

pub fn settings_json() -> Value {
    let s = load_settings();
    json!({"enabled": s.enabled, "dms": s.dms, "mentions": s.mentions, "calls": s.calls})
}

/// This conversation's mode, or "" when it follows the account default.
pub fn room_mode(room_id: &str) -> String {
    load_settings().rooms.get(room_id).cloned().unwrap_or_default()
}

/// `default` (or empty) forgets the per-conversation mode.
pub fn set_room_mode(room_id: &str, mode: &str) {
    let mut s = load_settings();
    if mode.is_empty() || mode == "default" {
        s.rooms.remove(room_id);
    } else {
        s.rooms.insert(room_id.to_string(), mode.to_string());
    }
    save_settings(&s);
}
