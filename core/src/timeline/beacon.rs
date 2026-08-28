//! Live location sharing (MSC3489): a `beacon_info` **state** event declares that we are
//! sharing and for how long, `beacon` **messages** then carry positions. Stopping is not a
//! redaction — the state event is re-sent with `live: false`.

use std::time::Duration;

use serde_json::{json, Value};
use tracing::{debug, warn};

use crate::engine::SharedEngine;
use crate::ipc::wire::Reply;

/// How often to publish while sharing. More often mostly burns battery.
const BEACON_EVERY: Duration = Duration::from_secs(15);
/// Retry budget for the first beacon while the state event syncs back.
const ESTABLISH_RETRY: Duration = Duration::from_secs(2);
const MAX_ESTABLISH_ATTEMPTS: u32 = 15;

/// Refuse absurd durations rather than pass them to the server.
const MIN_DURATION_MS: u64 = 60 * 1000;
const MAX_DURATION_MS: u64 = 8 * 60 * 60 * 1000;

/// Where the running share is noted so a restart can clean up after itself. See [`reap`].
fn note_path() -> std::path::PathBuf { crate::paths::state_dir().join("live-share.json") }

fn remember(room_id: &str, until: u64) {
    let _ = std::fs::write(note_path(), json!({"roomId": room_id, "until": until}).to_string());
}

fn forget() { let _ = std::fs::remove_file(note_path()); }

fn remembered() -> Option<(String, u64)> {
    let v: Value = serde_json::from_slice(&std::fs::read(note_path()).ok()?).ok()?;
    Some((v.get("roomId")?.as_str()?.to_string(), v.get("until")?.as_u64()?))
}

/// End any share left by a previous process: the publish timer does not survive a restart
/// but the server's `beacon_info` does, so other clients keep drawing a dead share.
pub async fn reap(engine: &SharedEngine) {
    let Some((room_id, until)) = remembered() else { return };
    forget();
    // Already over on its own; the timeout did the work.
    if now_ms() >= until { return }
    let Some(client) = engine.client() else { return };
    let Some(rid) = crate::sync::members::parse_room_id(&room_id) else { return };
    let Some(room) = client.get_room(&rid) else { return };
    match room.stop_live_location_share().await {
        Ok(_) => debug!("beacon: ended a live share left over from the last run in {room_id}"),
        Err(e) => warn!("beacon: could not end the leftover share: {e}"),
    }
}

pub struct LiveShare {
    pub room_id: String,
    pub until_ms: u64,
    task: tokio::task::JoinHandle<()>,
}

impl LiveShare {
    pub fn abort(&self) { self.task.abort() }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn state_json(engine: &crate::engine::Engine) -> Value {
    let s = engine.state.lock();
    match &s.live_share {
        Some(l) => json!({
            "event": "location.live",
            "sharing": true,
            "roomId": l.room_id,
            "until": l.until_ms,
        }),
        None => json!({"event": "location.live", "sharing": false, "roomId": "", "until": 0}),
    }
}

fn broadcast(engine: &SharedEngine) {
    engine.hub.broadcast(state_json(engine));
}

/// `location.startLive {roomId, durationMs, description?}`
pub async fn start(engine: SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    let room_id = p.get("roomId").and_then(Value::as_str).unwrap_or("").to_string();
    let duration = p
        .get("durationMs")
        .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
        .unwrap_or(15 * 60 * 1000)
        .clamp(MIN_DURATION_MS, MAX_DURATION_MS);
    let description = p.get("description").and_then(Value::as_str).unwrap_or("").to_string();

    let Some(client) = engine.client() else { return Reply::err("not_logged_in", "not logged in") };
    let Some(rid) = crate::sync::members::parse_room_id(&room_id) else {
        return Reply::err("bad_request", "invalid roomId")
    };
    let Some(room) = client.get_room(&rid) else {
        return Reply::err("unknown_room", format!("unknown room {room_id}"))
    };

    // Sharing from two rooms at once is not something the UI offers.
    stop_inner(&engine).await;

    if crate::geo::fresh_fix(&engine).is_none() {
        return Reply::err(
            "no_position",
            "no position yet — GeoClue has not returned a fix (see `position.get` for why)",
        )
    }

    let desc = if description.is_empty() { None } else { Some(description) };
    if let Err(e) = room.start_live_location_share(duration, desc).await {
        return Reply::err("send_failed", format!("could not start sharing: {e}"))
    }

    let until = now_ms() + duration;
    let engine2 = engine.clone();
    let room2 = room.clone();
    let room_id2 = room_id.clone();
    let task = tokio::spawn(async move {
        // `send_location_beacon` reads `beacon_info` from the local store, which the one
        // just sent has not synced back into yet, so retry fast until the first lands.
        let mut established = false;
        let mut attempts = 0u32;
        loop {
            if now_ms() >= until { break }
            let wait = match crate::geo::fresh_fix(&engine2) {
                Some(f) => {
                    let uri = format!("geo:{},{}", f.lat, f.lon);
                    match room2.send_location_beacon(uri).await {
                        Ok(_) => { established = true; BEACON_EVERY }
                        Err(e) if !established && attempts < MAX_ESTABLISH_ATTEMPTS => {
                            attempts += 1;
                            debug!("beacon: waiting for the beacon_info to sync back ({attempts}): {e}");
                            ESTABLISH_RETRY
                        }
                        Err(e) => {
                            // Another client or the server ended it; stop rather than retry.
                            warn!("beacon: publish failed, ending the share: {e}");
                            break
                        }
                    }
                }
                // A stale fix is not worth publishing; keep the share alive.
                None => { warn!("beacon: no fresh fix this tick"); BEACON_EVERY }
            };
            tokio::time::sleep(wait).await;
        }
        debug!("beacon: live share in {room_id2} finished");
        forget();
        // Best effort: mark it not-live so no client thinks it still runs.
        let _ = room2.stop_live_location_share().await;
        engine2.state.lock().live_share = None;
        broadcast(&engine2);
    });

    remember(&room_id, until);
    engine.state.lock().live_share = Some(LiveShare { room_id: room_id.clone(), until_ms: until, task });
    broadcast(&engine);
    debug!("beacon: sharing live location in {room_id} for {}s", duration / 1000);
    Reply::ok(json!({"until": until}))
}

async fn stop_inner(engine: &SharedEngine) {
    let existing = engine.state.lock().live_share.take();
    let Some(share) = existing else { return };
    forget();
    share.abort();
    if let Some(client) = engine.client() {
        if let Some(rid) = crate::sync::members::parse_room_id(&share.room_id) {
            if let Some(room) = client.get_room(&rid) {
                if let Err(e) = room.stop_live_location_share().await {
                    warn!("beacon: could not mark the share stopped: {e}");
                }
            }
        }
    }
}

/// `location.stopLive {}` — ends whichever share is running.
pub async fn stop(engine: SharedEngine) -> Reply {
    stop_inner(&engine).await;
    broadcast(&engine);
    Reply::ok(json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(v: Value) -> serde_json::Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    fn duration_of(p: &serde_json::Map<String, Value>) -> u64 {
        p.get("durationMs")
            .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
            .unwrap_or(15 * 60 * 1000)
            .clamp(MIN_DURATION_MS, MAX_DURATION_MS)
    }

    #[test]
    fn the_ui_durations_survive_unchanged() {
        for ms in [15 * 60 * 1000u64, 60 * 60 * 1000, 8 * 60 * 60 * 1000] {
            assert_eq!(duration_of(&params(json!({"durationMs": ms}))), ms);
        }
    }

    #[test]
    fn absurd_durations_are_clamped_rather_than_sent() {
        assert_eq!(duration_of(&params(json!({"durationMs": 1}))), MIN_DURATION_MS);
        assert_eq!(
            duration_of(&params(json!({"durationMs": 30u64 * 24 * 60 * 60 * 1000}))),
            MAX_DURATION_MS
        );
    }

    #[test]
    fn the_cli_passes_numbers_as_strings_and_that_still_works() {
        assert_eq!(duration_of(&params(json!({"durationMs": "3600000"}))), 60 * 60 * 1000);
    }

    #[test]
    fn a_missing_duration_defaults_to_the_shortest_offer() {
        assert_eq!(duration_of(&params(json!({}))), 15 * 60 * 1000);
    }
}
