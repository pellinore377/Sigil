//! Reading the engine's video frames into a Slint `Image`.
//!
//! The engine decodes into an OMV1 shared-memory surface (core/src/shm.rs,
//! contract in video/omv_shm.h) — three slots written under a seq-lock, the
//! newest named by one atomic in the file header. The QML build had a
//! compositor plugin that mapped that file and painted it; Slint has no such
//! thing, so the mapping happens here and each frame is copied into a
//! `SharedPixelBuffer`.
//!
//! The copy is the whole cost: one `memcpy` of w×h×4 per frame, measured at
//! 0.77 ms for 720p in release on this machine (`cargo test --release --lib
//! video:: -- --nocapture`), against a 33 ms budget at 30 fps. That is what
//! makes this approach fine and why nothing cleverer is warranted. A debug
//! build is ~20× slower, which is the build, not the design.
//!
//! The surface belongs to the decoder: it is unlinked when playback ends or
//! pauses, and a fresh one appears at the same path on resume. The reader
//! therefore tolerates a vanished file (the last frame stays on screen) and
//! notices a replacement by its inode.

use std::cell::RefCell;

use memmap2::Mmap;

// OMV1 header (core/src/shm.rs writes these offsets).
const MAGIC: u32 = 0x3156_4D4F;
const HDR_SIZE: usize = 4096;
const SLOT_HDR: usize = 4096;
const OFF_HDR_SIZE: usize = 0x08;
const OFF_SLOTS: usize = 0x0C;
const OFF_SLOT_STRIDE: usize = 0x10;
const OFF_LATEST: usize = 0x28;

/// A mapped surface, and the last frame the view took from it.
struct Surface {
    path: String,
    ino: (u64, u64),
    map: Mmap,
    last_seq: u64,
}

thread_local! {
    static SURFACE: RefCell<Option<Surface>> = const { RefCell::new(None) };
}

/// Drop the mapping (playback ended, or the viewer closed).
pub fn release() {
    SURFACE.with(|s| *s.borrow_mut() = None);
}

/// The newest frame on the surface at `path`, if one has arrived since the
/// last call. `None` means "nothing new" — the view keeps what it has.
pub fn next_frame(path: &str) -> Option<slint::Image> {
    SURFACE.with(|cell| {
        let mut slot = cell.borrow_mut();
        // A pause unlinks the surface and a resume makes a new one at the same
        // name, so identity is the inode, not the path.
        let id = file_id(path);
        let stale = match slot.as_ref() {
            Some(s) => s.path != path || Some(s.ino) != id,
            None => true,
        };
        if stale {
            *slot = open(path);
        }
        let s = slot.as_mut()?;
        read_newest(s)
    })
}

fn file_id(path: &str) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::metadata(path).ok()?;
    Some((m.dev(), m.ino()))
}

fn open(path: &str) -> Option<Surface> {
    let ino = file_id(path)?;
    let file = std::fs::File::open(path).ok()?;
    // SAFETY: the engine only ever appends frames into a fixed-size mapping
    // of its own making; a truncation would be its own bug, not a race we
    // can lose here.
    let map = unsafe { Mmap::map(&file) }.ok()?;
    if map.len() < HDR_SIZE || get32(&map, 0)? != MAGIC {
        return None;
    }
    Some(Surface {
        path: path.to_string(),
        ino,
        map,
        last_seq: 0,
    })
}

fn get32(m: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(m.get(off..off + 4)?.try_into().ok()?))
}

/// Copy the newest slot out under the writer's seq-lock: the slot's own
/// counter is odd while it is being filled and must match before and after.
fn read_newest(s: &mut Surface) -> Option<slint::Image> {
    let m = &s.map;
    let hdr = get32(m, OFF_HDR_SIZE)? as usize;
    let slots = get32(m, OFF_SLOTS)? as usize;
    let slot_stride = get32(m, OFF_SLOT_STRIDE)? as usize;
    if slots == 0 || slot_stride == 0 {
        return None;
    }
    let latest = u64::from_le_bytes(m.get(OFF_LATEST..OFF_LATEST + 8)?.try_into().ok()?);
    let (seq, idx) = (latest >> 8, (latest & 0xff) as usize);
    if seq == 0 || seq == s.last_seq || idx >= slots {
        return None;
    }
    let base = hdr + idx * slot_stride;

    for _ in 0..3 {
        let before = get32(m, base)?;
        if before & 1 != 0 {
            continue; // mid-write
        }
        let w = get32(m, base + 4)? as usize;
        let h = get32(m, base + 8)? as usize;
        let stride = get32(m, base + 12)? as usize;
        if w == 0 || h == 0 || stride < w * 4 {
            return None;
        }
        let px = base + SLOT_HDR;
        let need = stride * h;
        let src = m.get(px..px + need)?;

        let mut buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(w as u32, h as u32);
        {
            let dst = buf.make_mut_bytes();
            let row = w * 4;
            if stride == row {
                // The common case: the writer's 64-byte alignment already
                // lands on the row, so the frame is one memcpy.
                dst.copy_from_slice(&src[..row * h]);
            } else {
                for y in 0..h {
                    dst[y * row..(y + 1) * row]
                        .copy_from_slice(&src[y * stride..y * stride + row]);
                }
            }
        }
        // Torn read: the writer came round to this slot mid-copy.
        if get32(m, base)? != before {
            continue;
        }
        s.last_seq = seq;
        return Some(slint::Image::from_rgba8(buf));
    }
    None
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    /// Read real frames off a real decode, and time the copy — the number the
    /// module comment quotes, and the reason this approach was taken.
    #[test]
    fn reads_decoded_frames_off_the_surface_and_says_what_it_costs() {
        if !sigil_engine::media::player::available() {
            eprintln!("no ffmpeg; skipping");
            return;
        }
        let dir = std::env::temp_dir().join(format!("sigil-vshm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("XDG_STATE_HOME", &dir);
        std::env::set_var("XDG_RUNTIME_DIR", &dir);
        let clip = dir.join("clip.mp4");
        let made = std::process::Command::new("ffmpeg")
            .args([
                "-hide_banner", "-loglevel", "error", "-y",
                "-f", "lavfi", "-i", "testsrc=size=1280x720:rate=30:duration=4",
                "-c:v", "libx264", "-pix_fmt", "yuv420p",
            ])
            .arg(&clip)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !made {
            eprintln!("no x264; skipping");
            return;
        }

        let mut pb = sigil_engine::media::player::start(
            "vshm-test",
            &clip.to_string_lossy(),
            false,
        )
        .expect("the decoder starts");
        assert_eq!((pb.width, pb.height), (1280, 720));

        let mut got = 0usize;
        let mut nanos = 0u128;
        let mut last: Option<slint::Image> = None;
        let start = Instant::now();
        while got < 20 && start.elapsed() < Duration::from_secs(20) {
            let t = Instant::now();
            match super::next_frame(&pb.path) {
                Some(img) => {
                    nanos += t.elapsed().as_nanos();
                    assert_eq!(img.size().width, 1280);
                    assert_eq!(img.size().height, 720);
                    got += 1;
                    last = Some(img);
                }
                None => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        pb.stop();
        assert!(got >= 20, "only {got} frames arrived off the surface");
        assert!(last.is_some());
        let per = nanos as f64 / got as f64 / 1e6;
        println!("shm → slint::Image at 1280x720: {per:.3} ms per frame over {got} frames");
        // A 720p frame is 3.5 MB. Release is the number that matters — 33 ms
        // is the whole budget at 30 fps — and an unoptimised copy is ~20×
        // slower, so the debug bound only catches something gone badly wrong.
        let bound = if cfg!(debug_assertions) { 60.0 } else { 5.0 };
        assert!(per < bound, "{per:.3} ms per frame is too slow to draw at 30 fps");
        super::release();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_surface_that_is_not_there_yields_nothing() {
        assert!(super::next_frame("/nonexistent/sigil/video.shm").is_none());
        super::release();
    }
}
