//! Local video playback: ffmpeg decodes into the same OMV1 shared-memory surface
//! the call tiles use, so QML reuses VideoSurface. Audio is a parallel ffplay.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Context;
use tracing::{info, warn};

use crate::rtc::shm::ShmWriter;

pub struct Playback {
    stop: Arc<AtomicBool>,
    audio: Option<Child>,
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub start_at: f64,
    pub file: String,
}

impl Playback {
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(mut a) = self.audio.take() { let _ = a.kill(); let _ = a.wait(); }
    }
}

/// Total duration in seconds (0 when unknown) — drives the scrubber.
fn duration(file: &str) -> f64 {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "default=nw=1:nk=1", file])
        .output();
    if let Ok(o) = out {
        if let Ok(v) = String::from_utf8_lossy(&o.stdout).trim().parse::<f64>() { return v.max(0.0); }
    }
    0.0
}

/// Rotation ffmpeg applies on decode. ffprobe reports the *pre*-rotation size, so 90/270 must be swapped.
fn rotation(file: &str) -> i32 {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries",
               "stream_side_data=rotation:stream_tags=rotate", "-of", "default=nw=1:nk=1", file])
        .output();
    if let Ok(o) = out {
        let s = String::from_utf8_lossy(&o.stdout);
        for line in s.lines() {
            if let Ok(v) = line.trim().parse::<f64>() {
                let r = ((v.round() as i32) % 360 + 360) % 360;
                if r != 0 { return r; }
            }
        }
    }
    0
}

/// Probe WxH and fps with ffprobe (falls back to 1280x720@30).
fn probe(file: &str) -> (u32, u32, f64) {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries",
               "stream=width,height,avg_frame_rate", "-of", "csv=p=0:s=x", file])
        .output();
    if let Ok(o) = out {
        let s = String::from_utf8_lossy(&o.stdout);
        let parts: Vec<&str> = s.trim().split('x').collect();
        if parts.len() >= 3 {
            let w = parts[0].parse::<u32>().unwrap_or(1280);
            let h = parts[1].parse::<u32>().unwrap_or(720);
            let fps = parts[2].split('/').collect::<Vec<_>>();
            let f = if fps.len() == 2 {
                let n = fps[0].parse::<f64>().unwrap_or(30.0);
                let d = fps[1].parse::<f64>().unwrap_or(1.0);
                if d > 0.0 { n / d } else { 30.0 }
            } else { 30.0 };
            return (w.max(2), h.max(2), if f > 0.5 { f } else { 30.0 });
        }
    }
    (1280, 720, 30.0)
}

/// Start decoding `file` into shm surface `name`; returns the surface path.
pub fn start(name: &str, file: &str, with_audio: bool) -> anyhow::Result<Playback> {
    start_at(name, file, with_audio, 0.0)
}

/// Start (or resume at `seek` seconds); seeking restarts both pipes.
pub fn start_at(name: &str, file: &str, with_audio: bool, seek: f64) -> anyhow::Result<Playback> {
    let (pw, ph, fps) = probe(file);
    let rot = rotation(file);
    let (w, h) = if rot == 90 || rot == 270 { (ph, pw) } else { (pw, ph) };
    let mut writer = ShmWriter::create(name, w, h).context("shm create")?;
    let path = writer.path().to_string_lossy().into_owned();
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let file2 = file.to_string();
    let seek2 = seek;

    std::thread::spawn(move || {
        let seek = seek2;
        let seek_s = format!("{seek:.3}");
        let mut child = match Command::new("ffmpeg")
            // Force the geometry we allocated: any mismatch would shear every frame.
            .args(["-hide_banner", "-loglevel", "error", "-ss", &seek_s, "-re", "-i", &file2,
                   "-vf", &format!("scale={w}:{h}"),
                   "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
            .stdout(Stdio::piped()).stderr(Stdio::null()).spawn() {
            Ok(c) => c,
            Err(e) => { warn!("video: ffmpeg spawn failed: {e}"); return }
        };
        let mut out = match child.stdout.take() { Some(o) => o, None => return };
        let frame_len = (w as usize) * (h as usize) * 4;
        let mut buf = vec![0u8; frame_len];
        let mut n: u64 = 0;
        while !stop2.load(Ordering::Relaxed) {
            match out.read_exact(&mut buf) {
                Ok(()) => {}
                Err(_) => break,
            }
            writer.write_with(w, h, false, |dst, stride| {
                let row = (w as usize) * 4;
                for y in 0..(h as usize) {
                    let s = y * row;
                    let d = y * stride;
                    dst[d..d + row].copy_from_slice(&buf[s..s + row]);
                }
            });
            n += 1;
        }
        let _ = child.kill();
        let _ = child.wait();
        info!("video: playback ended after {n} frames ({}x{} @{:.1})", w, h, fps);
    });

    let audio = if with_audio {
        Command::new("ffplay")
            .args(["-hide_banner", "-loglevel", "error", "-nodisp", "-autoexit", "-vn",
                   "-ss", &format!("{seek:.3}"), file])
            .stdout(Stdio::null()).stderr(Stdio::null()).spawn().ok()
    } else { None };

    Ok(Playback { stop, audio, path, width: w, height: h, duration: duration(file), start_at: seek, file: file.to_string() })
}
