//! Notification settings. Delivery of desktop notifications for the Sigil
//! backend lands with Phase 3 (devices and push).

use serde_json::{json, Value};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub enabled: bool,
    pub dms: bool,
    pub mentions: bool,
    pub calls: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            enabled: true,
            dms: true,
            mentions: true,
            calls: true,
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
    serde_json::to_value(load_settings()).unwrap_or(json!({}))
}
