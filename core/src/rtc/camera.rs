//! V4L2 camera capture (nokhwa) → I420 into a LiveKit NativeVideoSource + RGBA self-view shm.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use livekit::webrtc::video_frame::{I420Buffer, VideoFrame, VideoRotation};
use livekit::webrtc::video_source::native::NativeVideoSource;
use livekit::webrtc::native::yuv_helper;
use nokhwa::pixel_format::RgbAFormat;
use nokhwa::utils::{CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType};
use nokhwa::Camera;
use parking_lot::Mutex;
use tracing::{info, warn};

use super::shm::ShmWriter;

/// Truncated-MJPEG detector: the bottom band's mean and spread jump while the mid band stays still.
#[derive(Default, Clone, Copy)]
struct BandStats { bm: f32, bs: f32, mm: f32, ms: f32, valid: bool }

fn band(rgba: &[u8], w: usize, y0: usize, rows: usize) -> (f32, f32) {
    let mut n = 0f32;
    let mut sum = 0f32;
    let mut sum2 = 0f32;
    for y in y0..y0 + rows {
        let row = &rgba[y * w * 4..(y + 1) * w * 4];
        for px in row.chunks_exact(32) {
            let l = px[0] as f32 * 0.299 + px[1] as f32 * 0.587 + px[2] as f32 * 0.114;
            sum += l; sum2 += l * l; n += 1.0;
        }
    }
    if n < 1.0 { return (0.0, 0.0) }
    let mean = sum / n;
    ((mean), (sum2 / n - mean * mean).max(0.0).sqrt())
}

fn frame_stats(rgba: &[u8], w: usize, h: usize) -> BandStats {
    if h < 32 { return BandStats::default() }
    let (bm, bs) = band(rgba, w, h - 10, 8);
    let (mm, ms) = band(rgba, w, h / 2, 8);
    BandStats { bm, bs, mm, ms, valid: true }
}

fn bottom_corrupt(prev: &BandStats, cur: &BandStats) -> bool {
    if !prev.valid || !cur.valid { return false }
    let bottom_jump = (cur.bm - prev.bm).abs() > 6.0 || (cur.bs - prev.bs).abs() > 6.0;
    let mid_still = (cur.mm - prev.mm).abs() < 4.0 && (cur.ms - prev.ms).abs() < 4.0;
    bottom_jump && mid_still
}

pub struct CameraHandle {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl CameraHandle {
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() { let _ = t.join(); }
    }
}

pub fn list() -> Vec<serde_json::Value> {
    match nokhwa::query(nokhwa::utils::ApiBackend::Video4Linux) {
        Ok(list) => list.into_iter().map(|c| serde_json::json!({"id": c.index().to_string(), "name": c.human_name(), "description": c.description()})).collect(),
        Err(e) => { warn!("camera query failed: {e}"); Vec::new() }
    }
}

fn index_of(id: &str) -> CameraIndex {
    if let Ok(n) = id.parse::<u32>() { CameraIndex::Index(n) } else if id.is_empty() { CameraIndex::Index(0) } else { CameraIndex::String(id.to_string()) }
}

/// Start capturing; frames go to `source` (I420) and `preview` (RGBA, mirrored hint).
pub fn start(device: &str, width: u32, height: u32, fps: u32, source: NativeVideoSource, preview: Arc<Mutex<Option<ShmWriter>>>, on_error: impl Fn(String) + Send + 'static) -> CameraHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let idx = index_of(device);
    let thread = std::thread::Builder::new().name("sigil-camera".into()).spawn(move || {
        let fmt = RequestedFormat::new::<RgbAFormat>(RequestedFormatType::Closest(CameraFormat::new_from(width, height, FrameFormat::MJPEG, fps)));
        let mut cam = match Camera::new(idx, fmt) {
            Ok(c) => c,
            Err(e) => { on_error(format!("camera open failed: {e}")); return; }
        };
        if let Err(e) = cam.open_stream() { on_error(format!("camera stream failed: {e}")); return; }
        let cf = cam.camera_format();
        info!("camera: {}x{} @{} {:?}", cf.width(), cf.height(), cf.frame_rate(), cf.format());
        let mut frames: u64 = 0;
        let mut drops: u64 = 0;
        let mut prev_stats = BandStats::default();
        while !stop2.load(Ordering::Relaxed) {
            let frame = match cam.frame() { Ok(f) => f, Err(e) => { warn!("camera frame: {e}"); std::thread::sleep(std::time::Duration::from_millis(50)); continue } };
            let img = match frame.decode_image::<RgbAFormat>() { Ok(i) => i, Err(e) => { warn!("camera decode: {e}"); continue } };
            let (w, h) = (img.width(), img.height());
            let rgba = img.into_raw();
            if w == 0 || h == 0 || rgba.len() != (w as usize) * (h as usize) * 4 {
                drops += 1;
                if drops % 30 == 1 { warn!("camera: dropped {drops} invalid frames (last {w}x{h}, {} bytes)", rgba.len()); }
                continue;
            }
            let stats = frame_stats(&rgba, w as usize, h as usize);
            if bottom_corrupt(&prev_stats, &stats) {
                drops += 1;
                if drops % 5 == 1 { warn!("camera: dropped {drops} corrupt frames (bottom-band jump)"); }
                // Keep prev_stats: a corrupt band must not become the baseline.
                continue;
            }
            prev_stats = stats;
            {
                let mut g = preview.lock();
                if let Some(wr) = g.as_mut() {
                    if wr.ensure_capacity(w, h).is_ok() {
                        wr.write_with(w, h, true, |dst, stride| {
                            for y in 0..h as usize { dst[y * stride..y * stride + w as usize * 4].copy_from_slice(&rgba[y * w as usize * 4..(y + 1) * w as usize * 4]); }
                        });
                    }
                }
            }
            let mut buf = I420Buffer::new(w, h);
            {
                let (sy, su, sv) = buf.strides();
                let (dy, du, dv) = buf.data_mut();
                yuv_helper::abgr_to_i420(&rgba, w * 4, dy, sy, du, su, dv, sv, w as i32, h as i32);
            }
            let vf = VideoFrame { rotation: VideoRotation::VideoRotation0, timestamp_us: super::shm::monotonic_us() as i64, frame_metadata: None, buffer: buf };
            source.capture_frame(&vf);
            frames += 1;
            if frames % 300 == 0 { info!("camera: {frames} frames"); }
        }
        let _ = cam.stop_stream();
        info!("camera: stopped");
    }).expect("spawn camera thread");
    CameraHandle { stop, thread: Some(thread) }
}
