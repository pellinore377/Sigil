//! Animated GIF → a strip of PNG frames on disk.
//!
//! Neither Slint nor the software renderer has an animated image, so the view
//! cycles stills on a timer and needs the frames as files. Decoding happens
//! once per source file: the strip lands in its own directory under the media
//! cache, keyed on the file's identity, so opening the same GIF a second time
//! costs a `stat` and nothing else.
//!
//! Everything here is capped, because a GIF arrives from a stranger: a phone
//! must not be talked into decoding four hundred 4K frames into memory.

use std::io::BufReader;
use std::path::{Path, PathBuf};

use anyhow::{ensure, Context};
use image::AnimationDecoder;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tracing::debug;

use crate::engine::SharedEngine;
use crate::ipc::wire::Reply;

/// Frames kept from one GIF. Past this the strip loops early — a deliberate
/// trade against holding hundreds of textures on a phone.
pub const MAX_FRAMES: usize = 64;
/// Long edge of a kept frame. Bubbles are ~300 logical px and the viewer is a
/// phone screen; 480 covers both without paying for a poster-sized animation.
pub const MAX_EDGE: u32 = 480;
/// Refuse an absurd source outright rather than stream it frame by frame.
const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
/// One frame's canvas, matching the still-image decoder's cap (images.rs).
const MAX_PIXELS: u64 = 50_000_000;
/// How much of the media cache the decoded strips may hold between them.
const CACHE_BUDGET: u64 = 64 * 1024 * 1024;

/// A decoded strip: the frame files in order, and how long each is shown.
#[derive(Debug, Clone)]
pub struct Strip {
    pub dir: PathBuf,
    pub frames: Vec<PathBuf>,
    /// Per-frame display time in ms, same length as `frames`.
    pub delays: Vec<u32>,
    pub width: u32,
    pub height: u32,
    /// The source had more frames than `MAX_FRAMES`: the strip stops early.
    pub truncated: bool,
}

impl Strip {
    pub fn to_json(&self) -> Value {
        json!({
            "frames": self.frames.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>(),
            "delays": self.delays,
            "width": self.width,
            "height": self.height,
            "truncated": self.truncated,
        })
    }
}

/// `media.gifFrames {roomId, eventId} | {path}` →
/// `{frames: [path], delays: [ms], width, height, truncated}`.
///
/// The timeline names an event, as `audio.play` does, and the session finds
/// (downloading if it must) the file behind it; a local `path` is the door for
/// tools and tests. A still image answers `bad_media`, which the view caches as
/// "not animated" and stops asking.
pub async fn frames(engine: SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    let given = p.get("path").and_then(Value::as_str).unwrap_or("").to_string();
    // A caller that already holds the file says so and saves the lookup; a
    // path that has since been swept falls back to the session.
    let src = if !given.is_empty() && std::path::Path::new(&given).is_file() {
        PathBuf::from(given)
    } else {
        let room = p.get("roomId").and_then(Value::as_str).unwrap_or("").to_string();
        let event = p.get("eventId").and_then(Value::as_str).unwrap_or("").to_string();
        if room.is_empty() || event.is_empty() {
            return Reply::err(
                "bad_request",
                "media.gifFrames needs roomId and eventId (or a local path)",
            );
        }
        let Some(session) = engine.sigil.lock().clone() else {
            return Reply::err("bad_request", "no session");
        };
        match session.media_get(&room, &event).await {
            Reply::Ok(v) => PathBuf::from(v.get("path").and_then(Value::as_str).unwrap_or("")),
            other => return other,
        }
    };
    if !src.is_file() {
        return Reply::err("bad_request", "no such file");
    }
    // Only a fresh decode can push the cache over its budget; a repeat view
    // must not pay for a sweep of every strip on disk.
    let fresh = !strip_dir(&src).join("strip.json").is_file();
    match tokio::task::spawn_blocking(move || strip(&src)).await {
        Ok(Ok(s)) => {
            if fresh {
                gc(CACHE_BUDGET);
            }
            Reply::ok(s.to_json())
        }
        Ok(Err(e)) => Reply::err("bad_media", format!("{e:#}")),
        Err(e) => Reply::err("internal", e.to_string()),
    }
}

/// Where a source file's strip lives. The name carries the file's identity
/// (path, length, mtime), so an edited or replaced file gets a fresh strip.
pub fn strip_dir(src: &Path) -> PathBuf {
    let mut h = Sha256::new();
    h.update(src.to_string_lossy().as_bytes());
    if let Ok(m) = std::fs::metadata(src) {
        h.update(m.len().to_le_bytes());
        if let Ok(t) = m.modified() {
            if let Ok(d) = t.duration_since(std::time::UNIX_EPOCH) {
                h.update(d.as_secs().to_le_bytes());
            }
        }
    }
    crate::media::media_dir().join(format!("gif-{:x}", h.finalize()))
}

/// The strip for `src`, decoding it only if the cache has none.
pub fn strip(src: &Path) -> anyhow::Result<Strip> {
    let dir = strip_dir(src);
    if let Some(s) = cached(&dir) {
        debug!("gif: strip for {} was already decoded", src.display());
        return Ok(s);
    }
    let _ = std::fs::remove_dir_all(&dir);
    match decode(src, &dir) {
        Ok(s) => Ok(s),
        Err(e) => {
            // A half-written strip must never be mistaken for a cached one.
            let _ = std::fs::remove_dir_all(&dir);
            Err(e)
        }
    }
}

/// Read back a strip written by an earlier call, if all of it is still there.
fn cached(dir: &Path) -> Option<Strip> {
    let manifest = std::fs::read_to_string(dir.join("strip.json")).ok()?;
    let v: Value = serde_json::from_str(&manifest).ok()?;
    let frames: Vec<PathBuf> = v["frames"]
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .map(PathBuf::from)
        .collect();
    let delays: Vec<u32> = v["delays"]
        .as_array()?
        .iter()
        .filter_map(Value::as_u64)
        .map(|d| d as u32)
        .collect();
    if frames.len() < 2 || frames.len() != delays.len() {
        return None;
    }
    // The cache is swept from under us; one missing frame invalidates the lot.
    if !frames.iter().all(|f| f.is_file()) {
        return None;
    }
    Some(Strip {
        dir: dir.to_path_buf(),
        frames,
        delays,
        width: v["width"].as_u64().unwrap_or(0) as u32,
        height: v["height"].as_u64().unwrap_or(0) as u32,
        truncated: v["truncated"].as_bool().unwrap_or(false),
    })
}

/// Decode `src` into `dir` as `000.png`, `001.png`, … plus `strip.json`.
/// The manifest is written last: it is the commit point a cache read trusts.
fn decode(src: &Path, dir: &Path) -> anyhow::Result<Strip> {
    let file = std::fs::File::open(src).with_context(|| format!("open {}", src.display()))?;
    let len = file.metadata()?.len();
    ensure!(len <= MAX_SOURCE_BYTES, "gif is {len} bytes, past the cap");
    ensure!(is_gif(src), "not a GIF");

    let decoder = image::codecs::gif::GifDecoder::new(BufReader::new(file)).context("gif decoder")?;
    let (sw, sh) = image::ImageDecoder::dimensions(&decoder);
    ensure!(sw > 0 && sh > 0, "gif has no canvas");
    ensure!(
        (sw as u64) * (sh as u64) <= MAX_PIXELS,
        "gif canvas is {sw}x{sh}, past the cap"
    );
    // One scale for the whole strip: `image` composites every frame onto the
    // logical screen, so they all share the canvas size and stay in register.
    let (tw, th) = fit(sw, sh);

    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut delays: Vec<u32> = Vec::new();
    let mut truncated = false;
    for (i, frame) in decoder.into_frames().enumerate() {
        if i >= MAX_FRAMES {
            truncated = true;
            break;
        }
        let frame = frame.with_context(|| format!("frame {i}"))?;
        let (num, den) = frame.delay().numer_denom_ms();
        let img = image::DynamicImage::ImageRgba8(frame.into_buffer());
        let img = if (img.width(), img.height()) != (tw, th) {
            img.resize_exact(tw, th, image::imageops::FilterType::Triangle)
        } else {
            img
        };
        let path = dir.join(format!("{i:03}.png"));
        img.save_with_format(&path, image::ImageFormat::Png)
            .with_context(|| format!("write {}", path.display()))?;
        private(&path);
        paths.push(path);
        delays.push(delay_ms(num, den));
    }
    ensure!(paths.len() > 1, "not an animated GIF");

    let s = Strip {
        dir: dir.to_path_buf(),
        frames: paths,
        delays,
        width: tw,
        height: th,
        truncated,
    };
    let manifest = dir.join("strip.json");
    std::fs::write(&manifest, serde_json::to_vec(&s.to_json())?)?;
    private(&manifest);
    debug!(
        "gif: {} → {} frames at {tw}x{th}{}",
        src.display(),
        s.frames.len(),
        if truncated { " (truncated)" } else { "" }
    );
    Ok(s)
}

/// GIF87a / GIF89a. Extension and MIME both come from the sender.
fn is_gif(src: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 6];
    std::fs::File::open(src)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok()
        && (&head == b"GIF87a" || &head == b"GIF89a")
}

/// The canvas scaled into the `MAX_EDGE` box, never enlarged.
fn fit(w: u32, h: u32) -> (u32, u32) {
    if w.max(h) <= MAX_EDGE {
        return (w, h);
    }
    let s = MAX_EDGE as f64 / w.max(h) as f64;
    (
        ((w as f64 * s).round() as u32).max(1),
        ((h as f64 * s).round() as u32).max(1),
    )
}

/// A frame's display time. GIF stores hundredths, and 0 (or one hundredth)
/// means "as fast as you can" — which every renderer since Netscape has read
/// as 100ms, so the strip does too.
fn delay_ms(num: u32, den: u32) -> u32 {
    let ms = if den == 0 { 0 } else { num / den };
    if ms <= 10 {
        100
    } else {
        ms.min(10_000)
    }
}

#[cfg(unix)]
fn private(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}
#[cfg(not(unix))]
fn private(_path: &Path) {}

/// Keep the decoded strips under a budget, oldest first. `media::gc` only
/// reaps files, so the strip directories are swept here.
pub fn gc(max_bytes: u64) {
    let Ok(rd) = std::fs::read_dir(crate::media::media_dir()) else {
        return;
    };
    let mut dirs: Vec<(std::time::SystemTime, u64, PathBuf)> = rd
        .flatten()
        .filter(|e| {
            e.file_name().to_string_lossy().starts_with("gif-")
                && e.file_type().map(|t| t.is_dir()).unwrap_or(false)
        })
        .filter_map(|e| {
            let path = e.path();
            let mut bytes = 0u64;
            let mut newest = std::time::SystemTime::UNIX_EPOCH;
            for f in std::fs::read_dir(&path).ok()?.flatten() {
                if let Ok(m) = f.metadata() {
                    bytes += m.len();
                    if let Ok(t) = m.modified() {
                        newest = newest.max(t);
                    }
                }
            }
            Some((newest, bytes, path))
        })
        .collect();
    let total: u64 = dirs.iter().map(|d| d.1).sum();
    if total <= max_bytes {
        return;
    }
    dirs.sort_by_key(|d| d.0);
    let mut freed = 0;
    for d in dirs {
        if total - freed <= max_bytes {
            break;
        }
        if std::fs::remove_dir_all(&d.2).is_ok() {
            freed += d.1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "sigil-gif-test-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// An animated GIF of `n` frames, each a flat colour, `w`×`h`.
    fn make_gif(path: &Path, n: u32, w: u32, h: u32, delay_cs: u16) {
        let file = std::fs::File::create(path).unwrap();
        let mut enc = image::codecs::gif::GifEncoder::new(std::io::BufWriter::new(file));
        enc.set_repeat(image::codecs::gif::Repeat::Infinite).unwrap();
        for i in 0..n {
            let shade = (i * 255 / n.max(1)) as u8;
            let buf =
                image::RgbaImage::from_pixel(w, h, image::Rgba([shade, 40, 255 - shade, 255]));
            enc.encode_frame(image::Frame::from_parts(
                buf,
                0,
                0,
                image::Delay::from_numer_denom_ms(delay_cs as u32 * 10, 1),
            ))
            .unwrap();
        }
    }

    #[test]
    fn decodes_every_frame_with_its_delay() {
        let dir = tmpdir("basic");
        let src = dir.join("party.gif");
        make_gif(&src, 5, 32, 24, 8);
        let out = dir.join("strip");
        let s = decode(&src, &out).expect("an animated gif decodes");
        assert_eq!(s.frames.len(), 5);
        assert_eq!(s.delays.len(), 5);
        assert_eq!((s.width, s.height), (32, 24));
        assert!(!s.truncated);
        // 8 hundredths on the wire, 80ms on the clock.
        assert!(s.delays.iter().all(|d| *d == 80), "delays were {:?}", s.delays);
        for f in &s.frames {
            let img = image::open(f).expect("a readable PNG frame");
            assert_eq!((img.width(), img.height()), (32, 24));
        }
        // The frames really differ — a strip of one repeated still would
        // animate nothing.
        let a = std::fs::read(&s.frames[0]).unwrap();
        let b = std::fs::read(&s.frames[4]).unwrap();
        assert_ne!(a, b, "the frames must not all be the same picture");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_long_gif_stops_at_the_cap() {
        let dir = tmpdir("long");
        let src = dir.join("long.gif");
        make_gif(&src, (MAX_FRAMES + 20) as u32, 16, 16, 4);
        let s = decode(&src, &dir.join("strip")).expect("decodes");
        assert_eq!(s.frames.len(), MAX_FRAMES);
        assert!(s.truncated, "the strip says it stopped early");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_big_gif_is_scaled_into_the_box() {
        let dir = tmpdir("big");
        let src = dir.join("big.gif");
        make_gif(&src, 3, 1600, 900, 5);
        let s = decode(&src, &dir.join("strip")).expect("decodes");
        assert_eq!(s.width, MAX_EDGE);
        assert_eq!(s.height, 270, "aspect kept: 900 * 480 / 1600");
        for f in &s.frames {
            let (w, h) = image::image_dimensions(f).unwrap();
            assert_eq!((w, h), (MAX_EDGE, 270));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_still_is_not_a_strip() {
        let dir = tmpdir("still");
        let src = dir.join("still.gif");
        make_gif(&src, 1, 24, 24, 10);
        let err = decode(&src, &dir.join("strip")).expect_err("one frame is not an animation");
        assert!(format!("{err:#}").contains("not an animated"), "{err:#}");

        // A PNG wearing a .gif name is refused before the decoder sees it.
        let png = dir.join("liar.gif");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([1, 2, 3, 255]))
            .save_with_format(&png, image::ImageFormat::Png)
            .unwrap();
        let err = decode(&png, &dir.join("strip2")).expect_err("not a GIF");
        assert!(format!("{err:#}").contains("not a GIF"), "{err:#}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn zero_delay_frames_run_at_the_conventional_tenth_of_a_second() {
        assert_eq!(delay_ms(0, 1), 100);
        assert_eq!(delay_ms(10, 1), 100);
        assert_eq!(delay_ms(20, 1), 20);
        assert_eq!(delay_ms(30, 1), 30);
        // A slideshow frame is kept, not clamped down to a flicker.
        assert_eq!(delay_ms(5_000, 1), 5_000);
        assert_eq!(delay_ms(90_000, 1), 10_000);
        assert_eq!(delay_ms(100, 0), 100, "a zero denominator is not a panic");
    }

    #[test]
    fn a_second_look_reads_the_cache_and_a_changed_file_does_not() {
        let dir = tmpdir("cache");
        let src = dir.join("cached.gif");
        make_gif(&src, 4, 20, 20, 6);
        let first = strip(&src).expect("decodes");
        let marker = first.frames[0].clone();
        let stamp = std::fs::metadata(&marker).unwrap().modified().unwrap();
        let again = strip(&src).expect("comes back from the cache");
        assert_eq!(again.frames, first.frames);
        assert_eq!(
            std::fs::metadata(&marker).unwrap().modified().unwrap(),
            stamp,
            "the cached frames were not rewritten"
        );

        // A different file at the same path is a different strip.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        make_gif(&src, 6, 20, 20, 6);
        let fresh = strip(&src).expect("decodes the replacement");
        assert_eq!(fresh.frames.len(), 6);
        assert_ne!(fresh.dir, first.dir, "a new file gets a new strip");
        let _ = std::fs::remove_dir_all(&first.dir);
        let _ = std::fs::remove_dir_all(&fresh.dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_frame_invalidates_the_cache() {
        let dir = tmpdir("holes");
        let src = dir.join("holey.gif");
        make_gif(&src, 4, 20, 20, 6);
        let first = strip(&src).expect("decodes");
        std::fs::remove_file(&first.frames[2]).unwrap();
        assert!(cached(&first.dir).is_none(), "a swept frame invalidates it");
        let again = strip(&src).expect("decodes again");
        assert_eq!(again.frames.len(), 4);
        assert!(again.frames.iter().all(|f| f.is_file()));
        let _ = std::fs::remove_dir_all(&again.dir);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
