//! Local media: video playback into shared memory, voice recording, audio
//! playback of local files, and the media cache directory. Fetching and
//! sending media over the Sigil backend arrives with Phase 5.

use std::path::PathBuf;

pub mod audio;
pub mod emoji;
#[cfg(target_os = "android")]
pub mod emoji_android;
pub mod av;
pub mod gif;
pub mod images;
pub mod player;
pub mod voice;
#[cfg(target_os = "android")]
pub mod voice_android;

use serde_json::{json, Value};

use crate::engine::SharedEngine;
use crate::ipc::wire::Reply;

pub fn media_dir() -> PathBuf {
    let d = crate::paths::cache_dir().join("media");
    let _ = crate::paths::ensure_private_dir(&d);
    d
}

/// What a started (or resumed) clip tells the view about itself.
fn playback_json(pb: &player::Playback) -> Value {
    json!({
        "path": pb.path,
        "width": pb.width,
        "height": pb.height,
        "duration": pb.duration,
        "startAt": pb.start_at,
        "eventId": pb.event_id,
    })
}

/// `video.play {roomId, eventId} | {path}` → {path, width, height, duration, startAt, eventId}
///
/// `path` is the surface, not the clip: the view maps that shared-memory file
/// and draws the frames the decoder writes into it. The timeline names an
/// event, as `audio.play` does, and the session finds the file behind it.
pub async fn video_play(engine: SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    let mut file = p
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let event = p
        .get("eventId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    // A caller that already holds the file says so and saves the lookup; a
    // path that has since been swept falls back to the session.
    if !std::path::Path::new(&file).is_file() {
        file.clear();
        let room = p
            .get("roomId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if room.is_empty() || event.is_empty() {
            return Reply::err(
                "bad_request",
                "video.play needs roomId and eventId (or a local path)",
            );
        }
        let Some(session) = engine.sigil.lock().clone() else {
            return Reply::err("bad_request", "no session");
        };
        file = match session.media_get(&room, &event).await {
            Reply::Ok(v) => v
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            other => return other,
        };
    }
    if file.is_empty() || !std::path::Path::new(&file).is_file() {
        return Reply::err("bad_request", "no such file");
    }
    let with_audio = p.get("audio").and_then(Value::as_bool).unwrap_or(true);
    let seek = p.get("seek").and_then(Value::as_f64).unwrap_or(0.0).max(0.0);
    video_stop(engine.clone(), p).await;
    match player::start_at("video-play", &file, with_audio, seek) {
        Ok(mut pb) => {
            pb.event_id = event;
            let out = playback_json(&pb);
            *engine.playback.lock() = Some(pb);
            Reply::ok(out)
        }
        // No decoder on this platform (a phone has no ffmpeg): the view must
        // hear that rather than sit in front of a surface nothing fills.
        Err(e) => Reply::err("unsupported", format!("{e:#}")),
    }
}

/// `video.pause` — freeze where it is; the clock holds and the last frame the
/// view copied stays on screen. Resuming is `video.play` with `seek`.
pub async fn video_pause(engine: SharedEngine, _p: &serde_json::Map<String, Value>) -> Reply {
    let mut slot = engine.playback.lock();
    let Some(pb) = slot.as_mut() else {
        return Reply::err("bad_request", "nothing playing");
    };
    pb.pause();
    Reply::ok(json!({"position": pb.position(), "eventId": pb.event_id}))
}

/// `video.position` → {playing, paused, position, duration, eventId, path}
/// — the media clock, polled while the scrubber is on screen. A clip that has
/// run out is reported once and then dropped, as `audio.position` does.
pub async fn video_position(engine: SharedEngine, _p: &serde_json::Map<String, Value>) -> Reply {
    let mut slot = engine.playback.lock();
    let out = match slot.as_ref() {
        None => json!({"playing": false, "paused": false, "position": 0.0, "duration": 0.0}),
        Some(pb) => json!({
            "playing": !pb.paused() && !pb.finished(),
            "paused": pb.paused(),
            "ended": pb.finished(),
            "position": pb.position(),
            "duration": pb.duration,
            "eventId": pb.event_id,
            "path": pb.path,
            "width": pb.width,
            "height": pb.height,
        }),
    };
    if slot.as_ref().is_some_and(|pb| pb.finished()) {
        if let Some(mut pb) = slot.take() {
            pb.stop();
        }
    }
    Reply::ok(out)
}

/// `audio.play {path, seek?}` — play a local file.
pub async fn audio_play(engine: SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    // The timeline names the event, not the file: the session knows where
    // the media lives (and fetches it first when it is not here yet).
    let has_path = p.get("path").and_then(Value::as_str).is_some_and(|s| !s.is_empty());
    if has_path {
        return audio_play_file(engine, p).await;
    }
    let (room, event) = (
        p.get("roomId").and_then(Value::as_str).unwrap_or("").to_string(),
        p.get("eventId").and_then(Value::as_str).unwrap_or("").to_string(),
    );
    let Some(session) = engine.sigil.lock().clone() else {
        return Reply::err("bad_request", "no session");
    };
    let got = session.media_get(&room, &event).await;
    let path = match got {
        Reply::Ok(v) => v.get("path").and_then(Value::as_str).unwrap_or("").to_string(),
        other => return other,
    };
    let mut q = p.clone();
    q.insert("path".into(), json!(path));
    audio_play_file(engine, &q).await
}

/// `audio.position` → {playing, position, eventId} — where the one running
/// player has reached, in seconds. The UI polls this to advance a voice
/// note's waveform; a player that has run out is dropped here, so the poll
/// that sees the end also clears the engine's slot.
pub async fn audio_position(engine: SharedEngine, _p: &serde_json::Map<String, Value>) -> Reply {
    let out = {
        let mut slot = engine.audio_play.lock();
        match slot.as_mut() {
            None => json!({"playing": false, "position": 0.0}),
            Some(a) => {
                if a.finished() {
                    *slot = None;
                    json!({"playing": false, "position": 0.0})
                } else {
                    json!({"playing": true, "position": a.position(), "eventId": a.event_id})
                }
            }
        }
    };
    Reply::ok(out)
}

/// `voice.start` — begin recording; `voice.level` events stream while it runs.
pub async fn voice_start(engine: SharedEngine, _p: &serde_json::Map<String, Value>) -> Reply {
    if engine.recording.lock().is_some() {
        return Reply::err("bad_request", "already recording");
    }
    match voice::start(&engine) {
        Ok(rec) => {
            *engine.recording.lock() = Some(rec);
            Reply::ok(json!({}))
        }
        Err(e) => Reply::err("internal", format!("{e:#}")),
    }
}

/// `voice.stop` → {path, duration, waveform}
pub async fn voice_stop(engine: SharedEngine, _p: &serde_json::Map<String, Value>) -> Reply {
    let Some(rec) = engine.recording.lock().take() else {
        return Reply::err("bad_request", "not recording");
    };
    let (path, secs, wave) = voice::stop(rec);
    Reply::ok(json!({"path": path.to_string_lossy(), "duration": secs, "waveform": wave}))
}

/// `voice.cancel` — stop and discard.
pub async fn voice_cancel(engine: SharedEngine, _p: &serde_json::Map<String, Value>) -> Reply {
    if let Some(rec) = engine.recording.lock().take() {
        voice::cancel(rec);
    }
    Reply::ok(json!({}))
}

/// `audio.playFile {path, seek?}` — preview a local clip (pending recording).
pub async fn audio_play_file(engine: SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    let path = PathBuf::from(p.get("path").and_then(Value::as_str).unwrap_or(""));
    let seek = p
        .get("seek")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        .max(0.0);
    if !path.is_file() {
        return Reply::err("bad_request", "file not found");
    }
    if let Some(mut a) = engine.audio_play.lock().take() {
        a.stop();
    }
    // The timeline's event id rides along so a poll can say what is playing;
    // a composer preview has none and answers to "local".
    let event = match p.get("eventId").and_then(Value::as_str) {
        Some(e) if !e.is_empty() => e,
        _ => "local",
    };
    match voice::play(&path, event, seek) {
        Ok(a) => {
            *engine.audio_play.lock() = Some(a);
            Reply::ok(json!({}))
        }
        Err(e) => Reply::err("internal", format!("{e:#}")),
    }
}

pub async fn audio_stop(engine: SharedEngine, _p: &serde_json::Map<String, Value>) -> Reply {
    if let Some(mut a) = engine.audio_play.lock().take() {
        a.stop();
    }
    Reply::ok(json!({}))
}

/// `video.seek {seconds}` — restart the decoder at the requested offset.
pub async fn video_seek(engine: SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    let secs = p
        .get("seconds")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        .max(0.0);
    let was = {
        engine
            .playback
            .lock()
            .as_ref()
            .map(|pb| (pb.file.clone(), pb.event_id.clone()))
    };
    let Some((file, event)) = was else {
        return Reply::err("bad_request", "nothing playing");
    };
    if let Some(mut pb) = engine.playback.lock().take() {
        pb.stop();
    }
    match player::start_at("video-play", &file, true, secs) {
        Ok(mut pb) => {
            pb.event_id = event;
            let out = playback_json(&pb);
            *engine.playback.lock() = Some(pb);
            Reply::ok(out)
        }
        Err(e) => Reply::err("unsupported", format!("{e:#}")),
    }
}

pub async fn video_stop(engine: SharedEngine, _p: &serde_json::Map<String, Value>) -> Reply {
    if let Some(mut pb) = engine.playback.lock().take() {
        pb.stop();
    }
    Reply::ok(json!({}))
}

/// Keep the cache under a size budget (oldest first).
pub fn gc(max_bytes: u64) {
    let dir = media_dir();
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((m.modified().ok()?, m.len(), e.path()))
        })
        .collect();
    let total: u64 = files.iter().map(|f| f.1).sum();
    if total <= max_bytes {
        return;
    }
    files.sort_by_key(|f| f.0);
    let mut freed = 0;
    for f in files {
        if total - freed <= max_bytes {
            break;
        }
        if std::fs::remove_file(&f.2).is_ok() {
            freed += f.1;
        }
    }
}
