//! Local media: video playback into shared memory, voice recording, audio
//! playback of local files, and the media cache directory. Fetching and
//! sending media over the Sigil backend arrives with Phase 5.

use std::path::PathBuf;

pub mod audio;
pub mod emoji;
pub mod av;
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

/// `video.play {path, audio?}` — play a local file into the shm viewer.
pub async fn video_play(engine: SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    let file = p
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if file.is_empty() || !std::path::Path::new(&file).is_file() {
        return Reply::err(
            "bad_request",
            "video.play needs a local `path` on the Sigil backend",
        );
    }
    let with_audio = p.get("audio").and_then(Value::as_bool).unwrap_or(true);
    video_stop(engine.clone(), p).await;
    match player::start("video-play", &file, with_audio) {
        Ok(pb) => {
            let out = json!({"path": pb.path, "width": pb.width, "height": pb.height, "duration": pb.duration, "startAt": pb.start_at});
            *engine.playback.lock() = Some(pb);
            Reply::ok(out)
        }
        Err(e) => Reply::err("internal", e.to_string()),
    }
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
    let file = { engine.playback.lock().as_ref().map(|pb| pb.file.clone()) };
    let Some(file) = file else {
        return Reply::err("bad_request", "nothing playing");
    };
    if let Some(mut pb) = engine.playback.lock().take() {
        pb.stop();
    }
    match player::start_at("video-play", &file, true, secs) {
        Ok(pb) => {
            let out = json!({"path": pb.path, "width": pb.width, "height": pb.height, "duration": pb.duration, "startAt": pb.start_at});
            *engine.playback.lock() = Some(pb);
            Reply::ok(out)
        }
        Err(e) => Reply::err("internal", e.to_string()),
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
