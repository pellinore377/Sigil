//! What is inside an audio or video file, and a still from it. Everything comes from
//! ffprobe/ffmpeg, never the sender's `info` block, which is routinely absent or wrong.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use tracing::debug;

#[derive(Debug, Default, Clone)]
pub struct Probe {
    pub width: u32,
    pub height: u32,
    pub duration_ms: u64,
    pub video_codec: String,
    pub audio_codec: String,
    pub has_video: bool,
    pub has_audio: bool,
    /// An embedded picture (album art). Not a video stream.
    pub has_cover: bool,
}

impl Probe {
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "width": self.width,
            "height": self.height,
            "duration": self.duration_ms,
            "videoCodec": self.video_codec,
            "audioCodec": self.audio_codec,
            "hasVideo": self.has_video,
            "hasAudio": self.has_audio,
        })
    }
}

/// One ffprobe call for everything, as JSON.
pub fn probe(file: &Path) -> Option<Probe> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(file)
        .output()
        .ok()?;
    if !out.status.success() { return None }
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    let mut p = Probe::default();
    if let Some(d) = v.pointer("/format/duration").and_then(Value::as_str).and_then(|s| s.parse::<f64>().ok()) {
        p.duration_ms = (d.max(0.0) * 1000.0) as u64;
    }
    for s in v.get("streams").and_then(Value::as_array).into_iter().flatten() {
        let kind = s.get("codec_type").and_then(Value::as_str).unwrap_or("");
        let codec = s.get("codec_name").and_then(Value::as_str).unwrap_or("").to_string();
        match kind {
            "video" => {
                // Cover art in an MP3 is a one-frame video stream; it does not make the file a video.
                let is_cover = s.pointer("/disposition/attached_pic").and_then(Value::as_i64).unwrap_or(0) == 1;
                if p.video_codec.is_empty() { p.video_codec = codec }
                if is_cover { p.has_cover = true }
                if !is_cover {
                    p.has_video = true;
                    let (w, h) = (
                        s.get("width").and_then(Value::as_u64).unwrap_or(0) as u32,
                        s.get("height").and_then(Value::as_u64).unwrap_or(0) as u32,
                    );
                    // Rotated phone video reports the pre-rotation size.
                    let rot = rotation_of(s);
                    let (w, h) = if rot == 90 || rot == 270 { (h, w) } else { (w, h) };
                    if p.width == 0 { p.width = w; p.height = h }
                }
            }
            "audio" => {
                p.has_audio = true;
                if p.audio_codec.is_empty() { p.audio_codec = codec }
            }
            _ => {}
        }
    }
    Some(p)
}

fn rotation_of(stream: &Value) -> i32 {
    let from_side = stream
        .get("side_data_list")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find_map(|d| d.get("rotation").and_then(Value::as_f64));
    let from_tag = stream
        .pointer("/tags/rotate")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<f64>().ok());
    let raw = from_side.or(from_tag).unwrap_or(0.0);
    ((raw.round() as i32) % 360 + 360) % 360
}

/// A still from `file` as `<stem>.poster.png`, taken a second in — a first frame is often black.
pub fn poster(file: &Path, max: (u32, u32)) -> Option<PathBuf> {
    poster_to(file, max, &file.with_extension("poster.png"))
}

/// The embedded picture from a music file, written where told.
pub fn cover(file: &Path, max: (u32, u32), out_path: &Path) -> Option<PathBuf> {
    let out_path = out_path.to_path_buf();
    if out_path.exists() { return Some(out_path) }
    if !probe(file)?.has_cover { return None }
    let scale = format!("scale='min({},iw)':'min({},ih)':force_original_aspect_ratio=decrease", max.0, max.1);
    let tmp = out_path.with_extension("part.png");
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .arg("-i").arg(file)
        // Only the attached picture; `-an` keeps a stray audio stream out of the image container.
        .args(["-an", "-map", "0:v:0", "-frames:v", "1", "-vf", &scale, "-f", "image2", "-c:v", "png"])
        .arg(&tmp)
        .status()
        .ok()?;
    if !status.success() || !tmp.exists() {
        let _ = std::fs::remove_file(&tmp);
        return None
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, &out_path).ok()?;
    debug!("av: pulled the cover art off {}", file.display());
    Some(out_path)
}

/// The same still in the cache: an outgoing attachment is the user's own file, so nothing goes beside it.
pub fn poster_to(file: &Path, max: (u32, u32), out_path: &Path) -> Option<PathBuf> {
    let out_path = out_path.to_path_buf();
    if out_path.exists() { return Some(out_path) }
    let p = probe(file)?;
    if !p.has_video { return None }
    let seek = if p.duration_ms > 2_000 { "1" } else { "0" };
    let scale = format!("scale='min({},iw)':'min({},ih)':force_original_aspect_ratio=decrease", max.0, max.1);
    let tmp = out_path.with_extension("part.png");
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-ss", seek])
        .arg("-i").arg(file)
        .args(["-frames:v", "1", "-vf", &scale, "-f", "image2", "-c:v", "png"])
        .arg(&tmp)
        .status()
        .ok()?;
    if !status.success() || !tmp.exists() {
        let _ = std::fs::remove_file(&tmp);
        return None
    }
    // 0600 to match everything else in the cache.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, &out_path).ok()?;
    debug!("av: made a poster for {}", file.display());
    Some(out_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A two-second ffmpeg test clip; skips rather than fails without ffmpeg.
    fn make_clip(dir: &Path, name: &str, args: &[&str]) -> Option<PathBuf> {
        let path = dir.join(name);
        let mut cmd = Command::new("ffmpeg");
        cmd.args(["-hide_banner", "-loglevel", "error", "-y",
                  "-f", "lavfi", "-i", "testsrc=size=320x240:rate=10:duration=2",
                  "-f", "lavfi", "-i", "sine=frequency=440:duration=2"]);
        cmd.args(args);
        cmd.arg(&path);
        let ok = cmd.status().ok()?.success();
        if ok && path.exists() { Some(path) } else { None }
    }

    fn tmpdir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("sigil-av-test-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_poster_goes_where_it_is_told_and_leaves_the_source_alone() {
        // A still for the user's own file must not be scattered beside their video.
        let dir = tmpdir();
        let Some(clip) = make_clip(&dir, "elsewhere.mp4", &["-c:v", "libx264", "-an", "-shortest"]) else {
            eprintln!("skipping: no ffmpeg");
            return
        };
        let out = dir.join("chosen-name.png");
        let made = poster_to(&clip, (320, 240), &out).expect("a clip with video in it has a first frame");
        assert_eq!(made, out);
        assert!(out.exists());
        assert!(!clip.with_extension("poster.png").exists(), "nothing beside the source");
        let (w, h) = image::image_dimensions(&out).expect("a readable PNG");
        assert!(w > 0 && h > 0 && w <= 320 && h <= 240, "scaled into the box, got {w}x{h}");
        let _ = std::fs::remove_file(&out);
        let _ = std::fs::remove_file(&clip);
    }

    #[test]
    fn probes_dimensions_duration_and_codecs() {
        let dir = tmpdir();
        let Some(clip) = make_clip(&dir, "probe.mp4", &["-c:v", "libx264", "-c:a", "aac", "-shortest"]) else {
            eprintln!("no ffmpeg/x264; skipping");
            return
        };
        let p = probe(&clip).expect("probes");
        assert_eq!((p.width, p.height), (320, 240));
        assert!(p.has_video && p.has_audio);
        assert_eq!(p.video_codec, "h264");
        assert_eq!(p.audio_codec, "aac");
        assert!(p.duration_ms >= 1_800 && p.duration_ms <= 2_400, "duration was {}", p.duration_ms);
        let _ = std::fs::remove_file(clip);
    }

    #[test]
    fn poster_is_a_real_png_of_the_first_frames() {
        let dir = tmpdir();
        let Some(clip) = make_clip(&dir, "poster.mp4", &["-c:v", "libx264", "-an", "-shortest"]) else {
            eprintln!("no ffmpeg/x264; skipping");
            return
        };
        let poster = poster(&clip, (800, 600)).expect("makes a poster");
        let bytes = std::fs::read(&poster).unwrap();
        let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).expect("a PNG");
        assert_eq!((img.width(), img.height()), (320, 240));
        let _ = std::fs::remove_file(clip);
        let _ = std::fs::remove_file(poster);
    }

    #[test]
    fn audio_only_files_get_no_poster_and_report_no_video() {
        let dir = tmpdir();
        let Some(clip) = make_clip(&dir, "audio.opus", &["-map", "1:a", "-c:a", "libopus", "-shortest"]) else {
            eprintln!("no ffmpeg/opus; skipping");
            return
        };
        let p = probe(&clip).expect("probes");
        assert!(p.has_audio && !p.has_video);
        assert_eq!(p.audio_codec, "opus");
        assert!(poster(&clip, (800, 600)).is_none(), "audio has no poster to take");
        let _ = std::fs::remove_file(clip);
    }

    #[test]
    fn cover_art_does_not_make_an_mp3_into_a_video() {
        let dir = tmpdir();
        // ffprobe reports embedded artwork as a video stream.
        let cover = dir.join("cover.png");
        image::DynamicImage::new_rgb8(64, 64).save(&cover).unwrap();
        let path = dir.join("withcover.mp3");
        let ok = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y",
                   "-f", "lavfi", "-i", "sine=frequency=440:duration=1"])
            .arg("-i").arg(&cover)
            .args(["-map", "0:a", "-map", "1:v", "-c:a", "libmp3lame", "-c:v", "copy",
                   "-id3v2_version", "3", "-disposition:v", "attached_pic"])
            .arg(&path)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok || !path.exists() { eprintln!("no ffmpeg/lame; skipping"); return }
        let p = probe(&path).expect("probes");
        assert!(p.has_audio, "it is audio");
        assert!(!p.has_video, "cover art is not a video stream");
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(cover);
    }
}
