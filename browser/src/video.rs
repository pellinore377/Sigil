//! Browser call video: camera capture and VP9 WebCodecs over the sealed frame path.
//! Frames carry the Android header (rotation, width, height, big-endian u16) before the
//! encoded frame.
use crate::rtc::{construct, invoke, object};
use crate::{fail, get};
use js_sys::{Array, Function, Reflect, Uint8Array};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::JsFuture;
use web_sys::*;

/// Wire codec identifiers carried in every frame header, so a sender encodes what it can
/// and each receiver decodes per sender without any negotiation.
/// Codec, rotation, width and height, each big endian, ahead of the encoded frame.
const HEADER: u32 = 7;
const CODEC_VP9: u8 = 1;
const CODEC_AV1: u8 = 2;
/// The WebCodecs spelling of a wire codec.
fn codec_name(id: u8) -> Option<&'static str> {
    match id {
        CODEC_VP9 => Some("vp09.00.10.08"),
        CODEC_AV1 => Some("av01.0.08M.08"),
        _ => None,
    }
}
/// Frames per second to ask the camera for; the encoder follows what it actually delivers.
const RATE: u32 = 24;
/// The largest picture a call sends. Frames cross an unordered channel in 1 KB fragments and a
/// frame missing one fragment is discarded whole, so a big frame is a frame that rarely arrives.
const WIDTH: u32 = 640;
const HEIGHT: u32 = 480;
/// Bits per second for a capture, sized so a delta frame is a few fragments and a keyframe tens.
fn bitrate(width: u32, height: u32, _rate: u32) -> u32 {
    if width * height > WIDTH * HEIGHT { 900_000 } else { 600_000 }
}
struct Capture {
    stream: MediaStream,
    video: HtmlVideoElement,
    canvas: HtmlCanvasElement,
    context: CanvasRenderingContext2d,
    encoder: JsValue,
    width: u32,
    height: u32,
    rate: u32,
    frames: u64,
    /// Frames encoded since the last log line, and when that line was written.
    counted: u32,
    since: f64,
    timer: i32,
    _tick: Closure<dyn FnMut()>,
    _step: Option<Closure<dyn FnMut(JsValue, JsValue)>>,
    _output: Closure<dyn FnMut(JsValue, JsValue)>,
    _error: Closure<dyn FnMut(JsValue)>,
}
impl Drop for Capture {
    fn drop(&mut self) {
        if let Some(window) = web_sys::window() {
            window.clear_interval_with_handle(self.timer);
        }
        let _ = invoke(&self.encoder, "close", &[]);
        for track in self.stream.get_tracks() {
            if let Ok(track) = track.dyn_into::<MediaStreamTrack>() {
                track.stop();
            }
        }
        self.video.set_src_object(None);
    }
}
struct Viewer {
    canvas: HtmlCanvasElement,
    context: CanvasRenderingContext2d,
    decoder: Option<JsValue>,
    size: (u32, u32),
    codec: u8,
    rotation: u32,
    last: u64,
    interval: u64,
    awaiting_key: bool,
    counted: u32,
    since: f64,
    _output: Closure<dyn FnMut(JsValue)>,
    _error: Closure<dyn FnMut(JsValue)>,
}
impl Drop for Viewer {
    fn drop(&mut self) {
        if let Some(decoder) = self.decoder.take() {
            let _ = invoke(&decoder, "close", &[]);
        }
    }
}
thread_local! {
    static CAPTURE: RefCell<Option<Capture>> = const { RefCell::new(None) };
    static FORCE_KEY: Cell<bool> = const { Cell::new(false) };
    /// When the last keyframe was asked for after a frame failed to go.
    static RECOVERED: Cell<f64> = const { Cell::new(0.0) };
    static GENERATION: Cell<u32> = const { Cell::new(0) };
    static FAILED: Cell<bool> = const { Cell::new(false) };
    static VIEWERS: RefCell<HashMap<String, Viewer>> = RefCell::new(HashMap::new());
}
fn construct2(name: &str, first: &JsValue, second: &JsValue) -> Result<JsValue, JsValue> {
    let args = Array::new();
    args.push(first);
    args.push(second);
    Reflect::construct(
        &get(&js_sys::global(), name)?.dyn_into::<Function>()?,
        &args,
    )
}
fn number(value: &JsValue, key: &str) -> Option<f64> {
    get(value, key).ok()?.as_f64()
}
#[wasm_bindgen]
pub fn video_supported() -> bool {
    let global = js_sys::global();
    ["VideoEncoder", "VideoDecoder", "VideoFrame", "EncodedVideoChunk"]
        .iter()
        .all(|name| get(&global, name).map(|v| v.is_function()).unwrap_or(false))
}
#[wasm_bindgen]
pub fn video_camera_stop() {
    GENERATION.with(|g| g.set(g.get().wrapping_add(1)));
    CAPTURE.with(|c| c.borrow_mut().take());
}
#[wasm_bindgen]
pub fn video_camera_failed() -> bool {
    FAILED.with(Cell::get)
}
#[wasm_bindgen]
pub async fn video_camera_start(video: HtmlVideoElement, front: bool) -> Result<(), JsValue> {
    if !video_supported() {
        return Err(fail("This browser cannot encode call video"));
    }
    video_camera_stop();
    FAILED.with(|f| f.set(false));
    let generation = GENERATION.with(Cell::get);
    let window = web_sys::window().ok_or_else(|| fail("Missing window"))?;
    let document = window.document().ok_or_else(|| fail("Missing document"))?;
    let constraints = MediaStreamConstraints::new();
    constraints.set_audio(&false.into());
    constraints.set_video(&object(serde_json::json!({
        "facingMode": if front { "user" } else { "environment" },
        "width": {"ideal": WIDTH, "max": 1280}, "height": {"ideal": HEIGHT, "max": 1280},
        "frameRate": {"ideal": RATE, "max": 30}
    }))?);
    let stream = JsFuture::from(
        window
            .navigator()
            .media_devices()?
            .get_user_media_with_constraints(&constraints)?,
    )
    .await?
    .dyn_into::<MediaStream>()?;
    let stop = |stream: &MediaStream| {
        for track in stream.get_tracks() {
            if let Ok(track) = track.dyn_into::<MediaStreamTrack>() {
                track.stop();
            }
        }
    };
    if generation != GENERATION.with(Cell::get) {
        stop(&stream);
        return Err(fail("Camera cancelled"));
    }
    let track = stream
        .get_video_tracks()
        .get(0)
        .dyn_into::<MediaStreamTrack>()
        .map_err(|_| fail("Camera track unavailable"))?;
    let settings = invoke(&track, "getSettings", &[]).unwrap_or(JsValue::UNDEFINED);
    let mut width = number(&settings, "width").unwrap_or(640.0) as u32 & !1;
    let mut height = number(&settings, "height").unwrap_or(480.0) as u32 & !1;
    if !(16..=1920).contains(&width) || !(16..=1920).contains(&height) || width * height > 1920 * 1080 {
        width = 640;
        height = 480;
    }
    let rate = (number(&settings, "frameRate").unwrap_or(30.0).round() as u32).clamp(15, 30);
    video.set_muted(true);
    video.set_attribute("playsinline", "")?;
    video.set_src_object(Some(&stream));
    let played = JsFuture::from(video.play()?).await;
    if generation != GENERATION.with(Cell::get) || played.is_err() {
        stop(&stream);
        video.set_src_object(None);
        return Err(fail("Camera cancelled"));
    }
    let canvas = document
        .create_element("canvas")?
        .dyn_into::<HtmlCanvasElement>()?;
    canvas.set_width(width);
    canvas.set_height(height);
    let context = canvas
        .get_context("2d")?
        .ok_or_else(|| fail("Camera pixels unavailable"))?
        .dyn_into::<CanvasRenderingContext2d>()?;
    let output = Closure::<dyn FnMut(JsValue, JsValue)>::new(move |chunk: JsValue, _| {
        let Some(length) = number(&chunk, "byteLength") else {
            return;
        };
        if length <= 0.0 || length > (1024 * 1024 - HEADER as usize) as f64 {
            return;
        }
        let length = length as u32;
        let bytes = Uint8Array::new_with_length(HEADER + length);
        bytes.set_index(0, CODEC_AV1);
        bytes.set_index(3, (width >> 8) as u8);
        bytes.set_index(4, width as u8);
        bytes.set_index(5, (height >> 8) as u8);
        bytes.set_index(6, height as u8);
        if invoke(&chunk, "copyTo", &[bytes.subarray(HEADER, HEADER + length).into()]).is_err() {
            return;
        }
        let keyframe = get(&chunk, "type").ok().and_then(|v| v.as_string()).as_deref() == Some("key");
        let timestamp = number(&chunk, "timestamp").unwrap_or(-1.0).floor();
        wasm_bindgen_futures::spawn_local(async move {
            // A frame the transport drops leaves the peer decoding against a reference it
            // never received; the next capture becomes a keyframe instead of smearing.
            let sent = crate::rtc::browser_call_send(1, timestamp, keyframe, bytes).await;
            if let Err(error) = &sent {
                web_sys::console::log_1(&JsValue::from_str(&format!(
                    "SigilTiming call send: error {:?}",
                    error.as_string().unwrap_or_default()
                )));
            }
            // A frame that never went leaves the peer decoding against a reference it does not
            // have, and it holds everything until the next keyframe. Ask for one, but no more
            // than twice a second: asking on every shed is what turned the stream into keyframes.
            if !matches!(sent, Ok(true)) {
                let at = js_sys::Date::now();
                RECOVERED.with(|last| {
                    if at - last.get() >= 500.0 {
                        last.set(at);
                        FORCE_KEY.with(|f| f.set(true));
                    }
                });
            }
        });
    });
    let error = Closure::<dyn FnMut(JsValue)>::new(move |_| {
        FAILED.with(|f| f.set(true));
    });
    let options = js_sys::Object::new();
    Reflect::set(&options, &"output".into(), output.as_ref())?;
    Reflect::set(&options, &"error".into(), error.as_ref())?;
    let encoder = match construct("VideoEncoder", &options) {
        Ok(encoder) => encoder,
        Err(e) => {
            stop(&stream);
            return Err(e);
        }
    };
    // The best mode the browser will encode in real time: hardware first, then software at the
    // capture's rate, then down the ladder. Nothing is upscaled past what the camera delivers.
    let supported = |config: &serde_json::Value| -> Option<js_sys::Promise> {
        js_sys::Reflect::get(&js_sys::global(), &"VideoEncoder".into())
            .ok()
            .and_then(|ctor| js_sys::Reflect::get(&ctor, &"isConfigSupported".into()).ok())
            .and_then(|f| f.dyn_into::<js_sys::Function>().ok())
            .and_then(|f| object(config.clone()).ok().and_then(|c| f.call1(&JsValue::UNDEFINED, &c).ok()))
            .map(|p| p.unchecked_into::<js_sys::Promise>())
    };
    let ladder: [(u32, u32, u32, &str); 4] = [
        (WIDTH, HEIGHT, RATE, "prefer-hardware"),
        (WIDTH, HEIGHT, RATE, "no-preference"),
        (480, 360, RATE, "no-preference"),
        (320, 240, 15, "no-preference"),
    ];
    let portrait = height > width;
    let mut chosen = None;
    for (w, h, r, acceleration) in ladder {
        let (w, h) = if portrait { (h, w) } else { (w, h) };
        if w > width.max(WIDTH) || h > height.max(WIDTH) || r > rate.max(RATE) {
            continue;
        }
        let config = serde_json::json!({
            "codec": codec_name(CODEC_AV1).unwrap_or_default(), "width": w, "height": h,
            "bitrate": bitrate(w, h, r), "framerate": r, "latencyMode": "realtime",
            "hardwareAcceleration": acceleration
        });
        let ok = match supported(&config) {
            Some(promise) => match JsFuture::from(promise).await {
                Ok(result) => get(&result, "supported").ok().and_then(|v| v.as_bool()).unwrap_or(false),
                Err(_) => false,
            },
            None => true,
        };
        if ok {
            chosen = Some((w, h, r, acceleration, config));
            break;
        }
    }
    let (width, height, rate, acceleration, config) = chosen.ok_or_else(|| fail("This browser cannot encode call video"))?;
    canvas.set_width(width);
    canvas.set_height(height);
    web_sys::console::log_1(&JsValue::from_str(&format!("SigilTiming video out {width}x{height}@{rate} {}kbps {acceleration}", bitrate(width, height, rate) / 1000)));
    invoke(&encoder, "configure", &[object(config)?])?;
    let tick = Closure::<dyn FnMut()>::new(move || {
        let _ = CAPTURE.with(|slot| -> Result<(), JsValue> {
            let mut slot = slot.borrow_mut();
            let Some(capture) = slot.as_mut() else {
                return Ok(());
            };
            let state = get(&capture.encoder, "state")?.as_string().unwrap_or_default();
            let queue = number(&capture.encoder, "encodeQueueSize").unwrap_or(0.0);
            let ready = capture.video.ready_state();
            if state != "configured" || queue > 2.0 || ready < 2 {
                crate::rtc::note(&format!(
                    "capture: idle state={state} queue={queue} ready={ready}"
                ));
                return Ok(());
            }
            capture
                .context
                .draw_image_with_html_video_element_and_dw_and_dh(
                    &capture.video,
                    0.0,
                    0.0,
                    capture.width as f64,
                    capture.height as f64,
                )?;
            // Counted, not clocked: the capture timer is not punctual, and a wall clock turned
            // every late tick into a step the receiver could not tell from a lost frame.
            let timestamp = (capture.frames * 1_000_000 / u64::from(capture.rate)) as f64;
            let frame = construct2(
                "VideoFrame",
                &capture.canvas,
                &object(serde_json::json!({"timestamp": timestamp}))?,
            )?;
            let keyframe = capture.frames % u64::from(capture.rate) == 0 || FORCE_KEY.with(Cell::take);
            capture.frames += 1;
            capture.counted += 1;
            let at = js_sys::Date::now();
            if at - capture.since >= 5000.0 {
                let fps = f64::from(capture.counted) * 1000.0 / (at - capture.since);
                web_sys::console::log_1(&JsValue::from_str(&format!("SigilTiming video out fps={fps:.0} queue={}", number(&capture.encoder, "encodeQueueSize").unwrap_or(0.0))));
                capture.counted = 0;
                capture.since = at;
            }
            let result = invoke(
                &capture.encoder,
                "encode",
                &[frame.clone(), object(serde_json::json!({"keyFrame": keyframe}))?],
            );
            let _ = invoke(&frame, "close", &[]);
            result.map(|_| ())
        });
    });
    // Encode each camera frame as it lands rather than on a timer, when the browser can say so.
    // The frame callback only fires for an element the page is compositing, and the camera
    // element is not always on screen, so the timer drives capture and the callback does not.
    let paced = false;
    let timer = window.set_interval_with_callback_and_timeout_and_arguments_0(
        tick.as_ref().unchecked_ref(),
        (1000 / rate) as i32,
    )?;
    let step = if paced {
        let generation = GENERATION.with(Cell::get);
        let step: Closure<dyn FnMut(JsValue, JsValue)> = Closure::new(move |_now: JsValue, _meta: JsValue| {
            if GENERATION.with(Cell::get) != generation {
                return;
            }
            CAPTURE.with(|slot| {
                let slot = slot.borrow();
                if let Some(capture) = slot.as_ref() {
                    if let Some(tick) = capture._tick.as_ref().dyn_ref::<js_sys::Function>() {
                        let _ = tick.call0(&JsValue::UNDEFINED);
                    }
                }
            });
            CAPTURE.with(|slot| {
                let slot = slot.borrow();
                if let Some(capture) = slot.as_ref() {
                    if let Some(step) = capture._step.as_ref() {
                        let _ = invoke(&capture.video, "requestVideoFrameCallback", &[step.as_ref().clone()]);
                    }
                }
            });
        });
        Some(step)
    } else {
        None
    };
    CAPTURE.with(|slot| {
        *slot.borrow_mut() = Some(Capture {
            stream,
            video,
            canvas,
            context,
            encoder,
            width,
            height,
            rate,
            frames: 0,
            counted: 0,
            since: js_sys::Date::now(),
            timer,
            _tick: tick,
            _step: step,
            _output: output,
            _error: error,
        })
    });
    CAPTURE.with(|slot| {
        let slot = slot.borrow();
        if let Some(capture) = slot.as_ref() {
            if let Some(step) = capture._step.as_ref() {
                let _ = invoke(&capture.video, "requestVideoFrameCallback", &[step.as_ref().clone()]);
            }
        }
    });
    Ok(())
}
#[wasm_bindgen]
pub fn video_attach(sender: String, canvas: HtmlCanvasElement) -> Result<(), JsValue> {
    let context = canvas
        .get_context("2d")?
        .ok_or_else(|| fail("Video pixels unavailable"))?
        .dyn_into::<CanvasRenderingContext2d>()?;
    let key = sender.clone();
    let output = Closure::<dyn FnMut(JsValue)>::new(move |frame: JsValue| {
        let _ = VIEWERS.with(|viewers| -> Result<(), JsValue> {
            let viewers = viewers.borrow();
            let Some(viewer) = viewers.get(&key) else {
                let _ = invoke(&frame, "close", &[]);
                return Ok(());
            };
            let width = number(&frame, "displayWidth").unwrap_or(0.0);
            let height = number(&frame, "displayHeight").unwrap_or(0.0);
            let turned = viewer.rotation == 90 || viewer.rotation == 270;
            let (cw, ch) = if turned { (height, width) } else { (width, height) };
            if viewer.canvas.width() != cw as u32 || viewer.canvas.height() != ch as u32 {
                viewer.canvas.set_width(cw as u32);
                viewer.canvas.set_height(ch as u32);
            }
            viewer.context.save();
            viewer.context.translate(cw / 2.0, ch / 2.0)?;
            viewer
                .context
                .rotate(viewer.rotation as f64 * std::f64::consts::PI / 180.0)?;
            let result = invoke(
                &viewer.context,
                "drawImage",
                &[
                    frame.clone(),
                    (-width / 2.0).into(),
                    (-height / 2.0).into(),
                    width.into(),
                    height.into(),
                ],
            );
            viewer.context.restore();
            if let Err(error) = &result {
                // A failed paint is invisible otherwise: the tile just stays black.
                web_sys::console::log_1(&JsValue::from_str(&format!(
                    "SigilTiming video draw failed {:?}",
                    error.as_string().unwrap_or_else(|| "unknown".into())
                )));
            }
            let _ = invoke(&frame, "close", &[]);
            drop(viewers);
            VIEWERS.with(|viewers| {
                if let Some(viewer) = viewers.borrow_mut().get_mut(&key) {
                    viewer.counted += 1;
                    let at = js_sys::Date::now();
                    if at - viewer.since >= 5000.0 {
                        let fps = f64::from(viewer.counted) * 1000.0 / (at - viewer.since);
                        web_sys::console::log_1(&JsValue::from_str(&format!("SigilTiming video in {width}x{height} fps={fps:.0}")));
                        viewer.counted = 0;
                        viewer.since = at;
                    }
                }
            });
            result.map(|_| ())
        });
    });
    let key = sender.clone();
    let error = Closure::<dyn FnMut(JsValue)>::new(move |_| {
        VIEWERS.with(|viewers| {
            if let Some(viewer) = viewers.borrow_mut().get_mut(&key) {
                if let Some(decoder) = viewer.decoder.take() {
                    let _ = invoke(&decoder, "close", &[]);
                }
            }
        });
    });
    VIEWERS.with(|viewers| {
        viewers.borrow_mut().insert(
            sender,
            Viewer {
                canvas,
                context,
                decoder: None,
                size: (0, 0),
                codec: 0,
                rotation: 0,
                last: 0,
                interval: 0,
                awaiting_key: false,
                counted: 0,
                since: js_sys::Date::now(),
                _output: output,
                _error: error,
            },
        )
    });
    Ok(())
}
#[wasm_bindgen]
pub fn video_detach(sender: String) {
    VIEWERS.with(|viewers| viewers.borrow_mut().remove(&sender));
}
/// Returns true for video frames (consumed or dropped); audio frames return false.
#[wasm_bindgen]
pub fn video_receive(sender: String, frame: Uint8Array) -> Result<bool, JsValue> {
    let length = frame.length();
    if length < 11 + HEADER || frame.get_index(0) == 0 {
        return Ok(false);
    }
    let keyframe = frame.get_index(1) != 0;
    let mut raw = [0; 8];
    frame.subarray(2, 10).copy_to(&mut raw);
    let timestamp = u64::from_be_bytes(raw);
    let field = |at: u32| (frame.get_index(at) as u32) << 8 | frame.get_index(at + 1) as u32;
    let codec = frame.get_index(10);
    let (rotation, width, height) = (field(11), field(13), field(15));
    if codec_name(codec).is_none()
        || !matches!(rotation, 0 | 90 | 180 | 270)
        || !(16..=1920).contains(&width)
        || !(16..=1920).contains(&height)
        || timestamp > 9_007_199_254_740_991
    {
        return Ok(true);
    }
    VIEWERS.with(|viewers| -> Result<bool, JsValue> {
        let mut viewers = viewers.borrow_mut();
        let Some(viewer) = viewers.get_mut(&sender) else {
            return Ok(true);
        };
        // A lost fragment costs the whole frame, so decoding the next delta against a
        // reference that never arrived smears the picture. Hold for a keyframe instead.
        if keyframe {
            viewer.awaiting_key = false;
        } else if viewer.last > 0 {
            let delta = timestamp.saturating_sub(viewer.last);
            if delta == 0 {
                return Ok(true);
            }
            if viewer.interval == 0 {
                viewer.interval = delta;
            } else if delta * 4 > viewer.interval * 7 {
                viewer.awaiting_key = true;
            } else {
                viewer.interval = (viewer.interval * 7 + delta) / 8;
            }
        }
        viewer.last = timestamp;
        if viewer.awaiting_key {
            return Ok(true);
        }
        let closed = viewer
            .decoder
            .as_ref()
            .map(|d| get(d, "state").ok().and_then(|v| v.as_string()).as_deref() != Some("configured"))
            .unwrap_or(true);
        if closed || viewer.size != (width, height) || viewer.codec != codec {
            if !keyframe {
                return Ok(true);
            }
            if let Some(old) = viewer.decoder.take() {
                let _ = invoke(&old, "close", &[]);
            }
            let options = js_sys::Object::new();
            Reflect::set(&options, &"output".into(), viewer._output.as_ref())?;
            Reflect::set(&options, &"error".into(), viewer._error.as_ref())?;
            let decoder = construct("VideoDecoder", &options)?;
            invoke(
                &decoder,
                "configure",
                &[object(serde_json::json!({
                    "codec": codec_name(codec).unwrap_or_default(), "codedWidth": width, "codedHeight": height,
                    // Software: Chrome's hardware AV1 decode returns black frames, without ever
                    // reporting an error, on drivers we cannot detect cheaply (measured on VA-API at
                    // 640x480 and above, correct only at thumbnail sizes). dav1d keeps up with 1080p.
                    "hardwareAcceleration": "prefer-software", "optimizeForLatency": true
                }))?],
            )?;
            viewer.decoder = Some(decoder);
            viewer.size = (width, height);
            viewer.codec = codec;
            viewer.interval = 0;
        }
        viewer.rotation = rotation;
        let decoder = viewer.decoder.clone().ok_or_else(|| fail("Missing decoder"))?;
        if number(&decoder, "decodeQueueSize").unwrap_or(0.0) > 8.0 && !keyframe {
            return Ok(true);
        }
        let init = js_sys::Object::new();
        Reflect::set(&init, &"type".into(), &if keyframe { "key" } else { "delta" }.into())?;
        Reflect::set(&init, &"timestamp".into(), &(timestamp as f64).into())?;
        Reflect::set(&init, &"data".into(), &frame.subarray(10 + HEADER, length))?;
        let chunk = construct("EncodedVideoChunk", &init)?;
        if invoke(&decoder, "decode", &[chunk]).is_err() {
            let _ = invoke(&decoder, "close", &[]);
            viewer.decoder = None;
        }
        Ok(true)
    })
}
