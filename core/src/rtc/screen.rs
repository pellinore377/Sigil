//! Screen share via libwebrtc's desktop capturer (xdg portal + its internal PipeWire).
//! The system `pipewire` crate must NOT be used here — its libpipewire collides with
//! libwebrtc's copy and pw_init segfaults; `glib-main-loop` is required for the portal.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use livekit::webrtc::desktop_capturer::{CaptureError, DesktopCaptureSourceType, DesktopCapturer, DesktopCapturerOptions, DesktopFrame};
use livekit::webrtc::native::yuv_helper;
use livekit::webrtc::video_frame::{I420Buffer, VideoFrame, VideoRotation};
use livekit::webrtc::video_source::native::NativeVideoSource;
use parking_lot::Mutex;
use tracing::{info, warn};

use crate::shm::ShmWriter;

pub struct ScreenHandle {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ScreenHandle {
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let (done_tx, done_rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || { let _ = t.join(); let _ = done_tx.send(()); });
            let _ = done_rx.recv_timeout(std::time::Duration::from_secs(3));
        }
    }
}

/// Start capturing; returns after the first frame, or an error if the picker was cancelled.
pub async fn start(source: NativeVideoSource, preview: Arc<Mutex<Option<ShmWriter>>>, on_error: impl Fn(String) + Send + 'static) -> anyhow::Result<ScreenHandle> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let (started_tx, started_rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let thread = std::thread::Builder::new().name("sigil-screen".into()).spawn(move || {
        let mut opts = DesktopCapturerOptions::new(DesktopCaptureSourceType::Screen);
        opts.set_include_cursor(true);
        let Some(mut cap) = DesktopCapturer::new(opts) else {
            let _ = started_tx.send(Err("desktop capturer unavailable (libwebrtc returned null)".into()));
            return;
        };
        info!("screen: capturer created");
        let cb_stop = stop2.clone();
        let cb_tx = started_tx.clone();
        let mut announced = false;
        let mut frames: u64 = 0;
        cap.start_capture(None, move |result: Result<DesktopFrame, CaptureError>| {
            match result {
                Ok(frame) => {
                    let (w, h, stride) = (frame.width(), frame.height(), frame.stride() as usize);
                    if w <= 0 || h <= 0 || stride < w as usize * 4 { return; }
                    let (w, h) = (w as u32, h as u32);
                    let data = frame.data();
                    if data.len() < stride * h as usize { return; }
                    // DesktopFrame pixels are BGRA in memory (libyuv "ARGB").
                    let mut buf = I420Buffer::new(w, h);
                    {
                        let (sy, su, sv) = buf.strides();
                        let (dy, du, dv) = buf.data_mut();
                        yuv_helper::argb_to_i420(data, stride as u32, dy, sy, du, su, dv, sv, w as i32, h as i32);
                    }
                    let vf = VideoFrame { rotation: VideoRotation::VideoRotation0, timestamp_us: crate::shm::monotonic_us() as i64, frame_metadata: None, buffer: buf };
                    source.capture_frame(&vf);
                    if !announced {
                        announced = true;
                        let _ = cb_tx.send(Ok(()));
                        info!("screen: first frame {w}x{h}");
                    }
                    frames += 1;
                    if frames % 3 == 0 {
                        let mut g = preview.lock();
                        if let Some(wr) = g.as_mut() {
                            if wr.ensure_capacity(w, h).is_ok() {
                                wr.write_with(w, h, false, |dst, dstride| {
                                    for y in 0..h as usize {
                                        let src = &data[y * stride..y * stride + w as usize * 4];
                                        let row = &mut dst[y * dstride..y * dstride + w as usize * 4];
                                        for (s, o) in src.chunks_exact(4).zip(row.chunks_exact_mut(4)) {
                                            o[0] = s[2]; o[1] = s[1]; o[2] = s[0]; o[3] = 255;
                                        }
                                    }
                                });
                            }
                        }
                    }
                }
                Err(CaptureError::Temporary) => {}
                Err(CaptureError::Permanent) => {
                    warn!("screen: permanent capture error (cancelled or revoked)");
                    let _ = cb_tx.send(Err("screen capture ended (picker cancelled or session revoked)".into()));
                    cb_stop.store(true, Ordering::Relaxed);
                }
                #[allow(unreachable_patterns)]
                Err(_) => {}
            }
        });
        info!("screen: capture started; pumping");
        let mut ticks: u64 = 0;
        while !stop2.load(Ordering::Relaxed) {
            cap.capture_frame();
            ticks += 1;
            if ticks == 1 || ticks == 60 || ticks % 1800 == 0 { info!("screen: pump tick {ticks}"); }
            std::thread::sleep(std::time::Duration::from_millis(33));
        }
        info!("screen: stopped");
    })?;
    let handle = ScreenHandle { stop: stop.clone(), thread: Some(thread) };
    let res = tokio::task::spawn_blocking(move || started_rx.recv_timeout(std::time::Duration::from_secs(120))).await;
    match res {
        Ok(Ok(Ok(()))) => Ok(handle),
        Ok(Ok(Err(msg))) => { handle.stop(); anyhow::bail!(msg) }
        _ => { handle.stop(); on_error("screen share timed out waiting for the picker".into()); anyhow::bail!("screen share timed out") }
    }
}
