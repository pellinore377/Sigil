//! MapLibre renders maps but does not host them, so it needs a style URL: `mapStyleUrl`
//! in `settings.json` first, then the homeserver's MSC3488 `m.tile_server.map_style_url`
//! from `.well-known/matrix/client`. With neither, nothing is requested from anyone.

use serde_json::{json, Value};
use tracing::{debug, info};

use crate::engine::SharedEngine;

fn settings_override() -> String {
    std::fs::read(crate::notify::settings_path())
        .ok()
        .and_then(|d| serde_json::from_slice::<Value>(&d).ok())
        .and_then(|v| v.get("mapStyleUrl").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default()
}

/// Only http(s): the URL is fed straight to a renderer that will fetch it.
fn sane(url: &str) -> bool {
    let u = url.trim();
    (u.starts_with("https://") || u.starts_with("http://")) && u.len() < 2048
}

/// `.well-known` lookup order: the server-name domain, then the homeserver's own host.
fn well_known_urls(server_name: &str, homeserver: &str) -> Vec<String> {
    let mut out = Vec::new();
    let name = server_name.trim().trim_end_matches('/');
    if !name.is_empty() {
        out.push(format!("https://{name}/.well-known/matrix/client"));
    }
    let hs = homeserver.trim().trim_end_matches('/');
    if !hs.is_empty() {
        let url = format!("{hs}/.well-known/matrix/client");
        if !out.contains(&url) { out.push(url) }
    }
    out
}

fn style_from(v: &Value) -> String {
    // MSC3488 is still unstable, so accept both spellings.
    for key in ["m.tile_server", "org.matrix.msc3488.tile_server"] {
        if let Some(s) = v.get(key).and_then(|t| t.get("map_style_url")).and_then(Value::as_str) {
            if sane(s) { return s.to_string() }
        }
    }
    String::new()
}

async fn well_known(server_name: &str, homeserver: &str) -> String {
    let client = match crate::net::http_builder().timeout(std::time::Duration::from_secs(8)).build() {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    for url in well_known_urls(server_name, homeserver) {
        let Ok(resp) = client.get(&url).send().await else { continue };
        let Ok(v) = resp.json::<Value>().await else { continue };
        let found = style_from(&v);
        if !found.is_empty() {
            debug!("maps: found a style URL in {url}");
            return found
        }
    }
    String::new()
}

/// Resolve the style URL and stash it in state; the view reads it from `status`.
pub async fn refresh(engine: &SharedEngine) {
    let local = settings_override();
    let url = if sane(&local) {
        info!("maps: using the style URL from settings.json");
        local
    } else {
        let (hs, name) = {
            let s = engine.state.lock();
            (s.homeserver.clone(), s.server_name.clone())
        };
        let found = well_known(&name, &hs).await;
        if found.is_empty() {
            debug!("maps: no m.tile_server in .well-known and no local override");
        } else {
            info!("maps: using the homeserver's m.tile_server style URL");
        }
        found
    };
    engine.state.lock().map_style_url = url;
    engine.broadcast_status();
}

/// `map.config` → what the view needs to decide whether it can draw a map.
pub fn config_json(engine: &SharedEngine) -> Value {
    let url = engine.state.lock().map_style_url.clone();
    json!({ "mapStyleUrl": url, "configured": !url.is_empty() })
}

/// `map.setStyle {url}` — write the local override; empty clears it.
pub async fn set_style(engine: &SharedEngine, url: &str) -> Result<(), String> {
    let url = url.trim();
    if !url.is_empty() && !sane(url) {
        return Err("a map style URL must be http(s)".into())
    }
    let path = crate::notify::settings_path();
    let mut v: Value = std::fs::read(&path)
        .ok()
        .and_then(|d| serde_json::from_slice(&d).ok())
        .unwrap_or_else(|| json!({}));
    v["mapStyleUrl"] = json!(url);
    std::fs::write(&path, serde_json::to_vec_pretty(&v).unwrap_or_default())
        .map_err(|e| format!("could not save settings: {e}"))?;
    refresh(engine).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_server_name_domain_is_asked_first() {
        let urls = well_known_urls("example.com", "https://matrix.example.com/");
        assert_eq!(urls[0], "https://example.com/.well-known/matrix/client");
        assert_eq!(urls[1], "https://matrix.example.com/.well-known/matrix/client");
    }

    #[test]
    fn one_url_when_both_are_the_same_host() {
        let urls = well_known_urls("example.org", "https://example.org");
        assert_eq!(urls.len(), 1);
    }

    #[test]
    fn missing_pieces_do_not_produce_nonsense_urls() {
        assert!(well_known_urls("", "").is_empty());
        assert_eq!(well_known_urls("", "https://hs.example").len(), 1);
    }

    #[test]
    fn both_msc3488_spellings_are_read_and_only_http_is_accepted() {
        let stable = serde_json::json!({"m.tile_server": {"map_style_url": "https://a/style.json"}});
        assert_eq!(style_from(&stable), "https://a/style.json");
        let unstable = serde_json::json!({"org.matrix.msc3488.tile_server": {"map_style_url": "https://b/s.json"}});
        assert_eq!(style_from(&unstable), "https://b/s.json");
        let nasty = serde_json::json!({"m.tile_server": {"map_style_url": "file:///etc/passwd"}});
        assert_eq!(style_from(&nasty), "");
    }
}
