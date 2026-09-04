//! Voice messages: record with ffmpeg via PipeWire's Pulse layer, stream `voice.level`, send as MSC3245.
//! On Android the same entry points ride voice_android (AAudio/AMediaCodec);
//! the reply shapes match, so nothing above this module differs by platform.

#[cfg(not(target_os = "android"))]
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
#[cfg(not(target_os = "android"))]
use std::process::{Child, Command, Stdio};
#[cfg(not(target_os = "android"))]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(not(target_os = "android"))]
use std::sync::Arc;

#[cfg(not(target_os = "android"))]
use anyhow::Context;
#[cfg(not(target_os = "android"))]
use serde_json::json;
use tracing::info;
#[cfg(not(target_os = "android"))]
use tracing::warn;

use crate::engine::SharedEngine;

#[cfg(not(target_os = "android"))]
pub struct Recording {
    child: Child,
    stop: Arc<AtomicBool>,
    pub path: PathBuf,
    pub started: std::time::Instant,
}

#[cfg(not(target_os = "android"))]
impl Recording {
    fn finish(mut self) -> PathBuf {
        self.stop.store(true, Ordering::Relaxed);
        // ffmpeg finalises the container on SIGINT (kill would truncate it)
        unsafe { libc::kill(self.child.id() as i32, libc::SIGINT); }
        let _ = self.child.wait();
        self.path
    }
}

// Unique per take, so a new recording cannot overwrite the clip still in the composer.
fn take_path(ext: &str) -> PathBuf {
    let dir = crate::paths::cache_dir().join("voice");
    std::fs::create_dir_all(&dir).ok();
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis()).unwrap_or(0);
    dir.join(format!("rec-{stamp}.{ext}"))
}

/// Start recording; pushes `voice.level` events ~10x/second while it runs.
#[cfg(not(target_os = "android"))]
pub fn start(engine: &SharedEngine) -> anyhow::Result<Recording> {
    let path = take_path("ogg");

    // ebur128's momentary loudness goes to stderr: ffmpeg block-buffers stdout.
    let mut child = Command::new("ffmpeg")
        .args([
            "-hide_banner", "-loglevel", "verbose", "-nostats",
            "-f", "pulse", "-i", "default",
            "-map", "0:a", "-ac", "1", "-ar", "48000", "-c:a", "libopus", "-b:a", "32k",
            "-y", path.to_string_lossy().as_ref(),
            "-map", "0:a", "-af", "ebur128=metadata=1:framelog=verbose", "-f", "null", "-",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("ffmpeg record")?;

    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    if let Some(err) = child.stderr.take() {
        let eng = engine.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(err);
            for line in reader.lines() {
                if stop2.load(Ordering::Relaxed) { break; }
                let Ok(line) = line else { break };
                // "[Parsed_ebur128_0 @ …] t: 0.4  TARGET:-23 LUFS  M: -21.9 S:…"
                let Some(idx) = line.find("M:") else { continue };
                let tok: String = line[idx + 2..]
                    .trim_start()
                    .chars()
                    .take_while(|c| !c.is_whitespace())
                    .collect();
                let Ok(lufs) = tok.parse::<f64>() else { continue };
                if lufs < -100.0 {
                    eng.hub.broadcast(json!({"event": "voice.level", "level": 0.0}));
                    continue;
                }
                // speech sits around -35..-8 LUFS momentary
                let level = ((lufs + 40.0) / 32.0).clamp(0.0, 1.0).powf(0.8);
                eng.hub.broadcast(json!({"event": "voice.level", "level": level}));
            }
        });
    }
    Ok(Recording { child, stop, path, started: std::time::Instant::now() })
}

#[cfg(not(target_os = "android"))]
fn duration_secs(path: &std::path::Path) -> f64 {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "default=nw=1:nk=1"])
        .arg(path)
        .output();
    if let Ok(o) = out {
        if let Ok(v) = String::from_utf8_lossy(&o.stdout).trim().parse::<f64>() { return v.max(0.0); }
    }
    0.0
}

/// `buckets` RMS amplitudes in 0..1, decoded straight from the file.
#[cfg(not(target_os = "android"))]
pub fn waveform(path: &std::path::Path, buckets: usize) -> Vec<f32> {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-ac", "1", "-ar", "8000", "-f", "s16le", "-"])
        .output();
    let Ok(o) = out else { return vec![] };
    let samples: Vec<i16> = o.stdout
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect();
    wave_buckets(&samples, buckets)
}

/// The bucket shape both platforms share: per-chunk RMS, peak-normalised so
/// quiet recordings still show a readable shape.
fn wave_buckets(samples: &[i16], buckets: usize) -> Vec<f32> {
    if samples.is_empty() || buckets == 0 { return vec![]; }
    let per = (samples.len() / buckets).max(1);
    let mut out_v: Vec<f32> = samples
        .chunks(per)
        .take(buckets)
        .map(|c| {
            let sum: f64 = c.iter().map(|s| (*s as f64 / 32768.0).powi(2)).sum();
            ((sum / c.len() as f64).sqrt() as f32).clamp(0.0, 1.0)
        })
        .collect();
    // normalise so quiet recordings still show a readable shape
    let peak = out_v.iter().cloned().fold(0.0_f32, f32::max);
    if peak > 0.001 { for v in out_v.iter_mut() { *v = (*v / peak).clamp(0.0, 1.0); } }
    out_v
}

#[cfg(not(target_os = "android"))]
pub fn stop(rec: Recording) -> (PathBuf, f64, Vec<f32>) {
    let path = rec.finish();
    let secs = duration_secs(&path);
    let wave = waveform(&path, 60);
    info!("voice: recorded {:.1}s -> {}", secs, path.display());
    (path, secs, wave)
}

#[cfg(not(target_os = "android"))]
pub fn cancel(rec: Recording) {
    let path = rec.finish();
    let _ = std::fs::remove_file(&path);
}

/// Simple audio playback (voice notes): ffplay from an offset.
#[cfg(not(target_os = "android"))]
pub struct AudioPlayback {
    child: Child,
    pub event_id: String,
    pub start_at: f64,
    started: std::time::Instant,
}

#[cfg(not(target_os = "android"))]
impl AudioPlayback {
    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// Seconds into the clip. ffplay reports nothing back, so this is the
    /// wall clock since it started, offset by wherever the seek put it.
    pub fn position(&self) -> f64 {
        self.start_at + self.started.elapsed().as_secs_f64()
    }

    /// True once the player has run out (ffplay carries `-autoexit`).
    pub fn finished(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }
}

#[cfg(not(target_os = "android"))]
pub fn play(file: &std::path::Path, event_id: &str, seek: f64) -> anyhow::Result<AudioPlayback> {
    let child = Command::new("ffplay")
        .args(["-hide_banner", "-loglevel", "error", "-nodisp", "-autoexit", "-ss", &format!("{seek:.3}")])
        .arg(file)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("ffplay")?;
    if false { warn!("unreachable"); }
    Ok(AudioPlayback {
        child,
        event_id: event_id.to_string(),
        start_at: seek,
        started: std::time::Instant::now(),
    })
}

// ---- Android: the same four entry points over voice_android --------------
// AAudio records straight PCM, so duration and waveform come from the
// captured samples rather than a probe of the file.

#[cfg(target_os = "android")]
pub struct Recording {
    rec: super::voice_android::Recorder,
    pub started: std::time::Instant,
}

#[cfg(target_os = "android")]
pub fn start(engine: &SharedEngine) -> anyhow::Result<Recording> {
    let rec = super::voice_android::start(engine, take_path("wav"))?;
    Ok(Recording { rec, started: std::time::Instant::now() })
}

#[cfg(target_os = "android")]
pub fn stop(rec: Recording) -> (PathBuf, f64, Vec<f32>) {
    let (path, samples) = rec.rec.finish();
    let secs = samples.len() as f64 / super::voice_android::SAMPLE_RATE as f64;
    let wave = wave_buckets(&samples, 60);
    info!("voice: recorded {:.1}s -> {}", secs, path.display());
    (path, secs, wave)
}

#[cfg(target_os = "android")]
pub fn cancel(rec: Recording) {
    let (path, _) = rec.rec.finish();
    let _ = std::fs::remove_file(&path);
}

#[cfg(target_os = "android")]
pub struct AudioPlayback {
    inner: super::voice_android::Playback,
    pub event_id: String,
    pub start_at: f64,
}

#[cfg(target_os = "android")]
impl AudioPlayback {
    pub fn stop(&mut self) {
        self.inner.stop();
    }

    /// Seconds into the clip, counted by the frames handed to AAudio.
    pub fn position(&self) -> f64 {
        self.inner.position()
    }

    /// True once the decode/write loop has run off the end of the clip.
    pub fn finished(&mut self) -> bool {
        self.inner.finished()
    }
}

#[cfg(target_os = "android")]
pub fn play(file: &std::path::Path, event_id: &str, seek: f64) -> anyhow::Result<AudioPlayback> {
    let inner = super::voice_android::play(file, seek)?;
    Ok(AudioPlayback { inner, event_id: event_id.to_string(), start_at: seek })
}
