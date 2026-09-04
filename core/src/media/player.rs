//! Local video playback: ffmpeg decodes into the same OMV1 shared-memory surface
//! the call tiles use, so a view reuses one frame reader. Audio is a parallel ffplay.
//!
//! The decoder is a child process, which is why this is the desktop path only:
//! there is no ffmpeg on a phone (see `video_play` in mod.rs, which turns a
//! missing decoder into an error instead of a surface that never fills).

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::Context;
use tracing::{info, warn};

use crate::shm::ShmWriter;

pub struct Playback {
    stop: Arc<AtomicBool>,
    audio: Option<Child>,
    /// Frames the decoder has published. With `fps` this is the media clock:
    /// ffmpeg runs `-re`, so the count is the wall clock too, and it does not
    /// drift when the pipe stalls.
    frames: Arc<AtomicU64>,
    /// Set when the decoder thread runs out of pipe (the clip ended).
    ended: Arc<AtomicBool>,
    fps: f64,
    /// Where the clock stopped when `pause` was called; `None` while running.
    paused_at: Option<f64>,
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub start_at: f64,
    pub file: String,
    /// The timeline event this clip came from, so a poll can say what plays.
    pub event_id: String,
}

impl Playback {
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(mut a) = self.audio.take() {
            let _ = a.kill();
            let _ = a.wait();
        }
    }

    /// Seconds into the clip. Paused playback holds where it stopped.
    pub fn position(&self) -> f64 {
        if let Some(p) = self.paused_at {
            return p;
        }
        let played = self.frames.load(Ordering::Relaxed) as f64 / self.fps.max(0.1);
        let pos = self.start_at + played;
        if self.duration > 0.0 {
            pos.min(self.duration)
        } else {
            pos
        }
    }

    pub fn paused(&self) -> bool {
        self.paused_at.is_some()
    }

    /// The clip ran to its end (the decoder's pipe closed).
    pub fn finished(&self) -> bool {
        self.ended.load(Ordering::Relaxed)
    }

    /// Freeze: the decoder and its audio stop, the clock keeps its place. The
    /// surface goes with the decoder thread, so the view keeps the last frame
    /// it copied and a resume brings a fresh surface at the same path.
    pub fn pause(&mut self) {
        if self.paused_at.is_some() {
            return;
        }
        let at = self.position();
        self.stop();
        self.paused_at = Some(at);
    }
}

/// Whether this machine can decode at all. Android has neither binary.
pub fn available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
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
    let seek_s = format!("{seek:.3}");
    // Spawned here, not on the decoder thread: a machine with no ffmpeg has to
    // fail the request, not hand back a surface that never fills.
    let mut child = Command::new("ffmpeg")
        // Force the geometry we allocated: any mismatch would shear every frame.
        .args(["-hide_banner", "-loglevel", "error", "-ss", &seek_s, "-re", "-i", file,
               "-vf", &format!("scale={w}:{h}"),
               "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .stdout(Stdio::piped()).stderr(Stdio::null()).spawn()
        .context("ffmpeg could not start — no video decoder on this platform")?;
    let mut out = child.stdout.take().context("ffmpeg gave no pipe")?;

    let mut writer = ShmWriter::create(name, w, h).context("shm create")?;
    let path = writer.path().to_string_lossy().into_owned();
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let frames = Arc::new(AtomicU64::new(0));
    let frames2 = frames.clone();
    let ended = Arc::new(AtomicBool::new(false));
    let ended2 = ended.clone();

    std::thread::spawn(move || {
        let frame_len = (w as usize) * (h as usize) * 4;
        let mut buf = vec![0u8; frame_len];
        let mut n: u64 = 0;
        let mut ran_out = false;
        while !stop2.load(Ordering::Relaxed) {
            match out.read_exact(&mut buf) {
                Ok(()) => {}
                Err(_) => { ran_out = true; break }
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
            frames2.store(n, Ordering::Relaxed);
        }
        if ran_out {
            ended2.store(true, Ordering::Relaxed);
        }
        let _ = child.kill();
        let _ = child.wait();
        info!("video: playback ended after {n} frames ({}x{} @{:.1})", w, h, fps);
    });

    let audio = if with_audio {
        match Command::new("ffplay")
            .args(["-hide_banner", "-loglevel", "error", "-nodisp", "-autoexit", "-vn",
                   "-ss", &seek_s, file])
            .stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            Ok(c) => Some(c),
            // Sound is not worth failing playback over.
            Err(e) => { warn!("video: no ffplay, playing silently: {e}"); None }
        }
    } else { None };

    Ok(Playback {
        stop,
        audio,
        frames,
        ended,
        fps,
        paused_at: None,
        path,
        width: w,
        height: h,
        duration: duration(file),
        start_at: seek,
        file: file.to_string(),
        event_id: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(dir: &std::path::Path) -> Option<String> {
        let path = dir.join("clip.mp4");
        let ok = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y",
                   "-f", "lavfi", "-i", "testsrc=size=160x120:rate=10:duration=3",
                   "-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .arg(&path)
            .status().ok()?.success();
        (ok && path.exists()).then(|| path.to_string_lossy().into_owned())
    }

    #[test]
    fn the_clock_follows_the_frames_and_a_pause_holds_it() {
        if !available() {
            eprintln!("no ffmpeg; skipping");
            return;
        }
        let dir = std::env::temp_dir().join(format!("sigil-player-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("XDG_STATE_HOME", &dir);
        std::env::set_var("XDG_RUNTIME_DIR", &dir);
        let Some(file) = clip(&dir) else {
            eprintln!("no x264; skipping");
            return;
        };
        let mut pb = start("player-test", &file, false).expect("starts");
        assert!(pb.duration > 2.0 && pb.duration < 4.0, "duration {}", pb.duration);
        assert_eq!((pb.width, pb.height), (160, 120));
        assert_eq!(pb.position(), 0.0, "nothing decoded yet");
        assert!(
            std::path::Path::new(&pb.path).is_file(),
            "the surface exists before the first frame"
        );
        // ffmpeg takes about a second to hand over its first frame; the view
        // shows the poster until then, so the test waits the same way.
        let start = std::time::Instant::now();
        while pb.position() <= 0.2 && start.elapsed() < std::time::Duration::from_secs(10) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let running = pb.position();
        assert!(running > 0.2, "the clock moved with the frames, got {running}");
        pb.pause();
        assert!(pb.paused());
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert_eq!(pb.position(), running, "a paused clip does not advance");
        pb.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_is_not_there_fails_the_request() {
        if !available() {
            eprintln!("no ffmpeg; skipping");
            return;
        }
        let dir = std::env::temp_dir().join(format!("sigil-player-missing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("XDG_STATE_HOME", &dir);
        std::env::set_var("XDG_RUNTIME_DIR", &dir);
        // ffmpeg starts and then exits: the surface exists but nothing arrives.
        let mut pb = start("player-missing", &dir.join("nope.mp4").to_string_lossy(), false)
            .expect("ffmpeg spawns even for a missing file");
        std::thread::sleep(std::time::Duration::from_millis(400));
        assert!(pb.finished(), "the decoder reports the clip is over");
        assert_eq!(pb.duration, 0.0, "nothing to probe");
        pb.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
