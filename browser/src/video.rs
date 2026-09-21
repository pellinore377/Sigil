//! AV1 camera capture uses native WebRTC encoding and encrypted RTP; playback uses WebCodecs.
use crate::rtc::{construct, invoke, object};
use crate::{fail, get};
use js_sys::{Reflect, Uint8Array};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::JsFuture;
use web_sys::*;
const HEADER: u32 = 7;
const CODEC_AV1: u8 = 2;
fn codec_name(id: u8) -> Option<&'static str> { (id == CODEC_AV1).then_some("av01.0.08M.08") }
fn number(value: &JsValue, key: &str) -> Option<f64> { get(value, key).ok()?.as_f64() }
struct Capture { stream: MediaStream, video: HtmlVideoElement }
#[derive(Default)]
struct NativeViewer {
    track: Option<MediaStreamTrack>, video: Option<HtmlVideoElement>, shape: Option<(u16,u16,u16)>,
    canvas: Option<HtmlCanvasElement>, probe: Option<HtmlCanvasElement>, good: u8, token: u32, failures:u8, probe_at:f64,
}
thread_local! { static NATIVE: RefCell<HashMap<String, NativeViewer>> = RefCell::new(HashMap::new()); }
impl NativeViewer {
    fn attach(&mut self) -> Result<(), JsValue> {
        let Some(video) = &self.video else { return Ok(()) };
        if self.canvas.is_none() {
            let document=web_sys::window().ok_or_else(||fail("Missing window"))?.document().ok_or_else(||fail("Missing document"))?;
            let canvas=document.create_element("canvas")?.dyn_into::<HtmlCanvasElement>()?;
            canvas.set_attribute("hidden","")?;
            canvas.set_attribute("style","position:absolute;inset:0;width:100%;height:100%;object-fit:contain;background:#000")?;
            if let Some(parent)=video.parent_node(){parent.append_child(&canvas)?;self.canvas=Some(canvas);}
            let probe=document.create_element("canvas")?.dyn_into::<HtmlCanvasElement>()?;probe.set_width(16);probe.set_height(16);self.probe=Some(probe);
        }
        if let Some(track) = self.track.as_ref().filter(|_| video.src_object().is_none()) {
            let stream = MediaStream::new()?;
            stream.add_track(track);
            video.set_src_object(Some(&stream));
            let _ = video.play();
        }
        if let Some((rotation, width, height)) = self.shape {
            video.set_attribute("data-rotation", &rotation.to_string())?;
            video.set_attribute("data-width", &width.to_string())?;
            video.set_attribute("data-height", &height.to_string())?;
        }
        Ok(())
    }
}
pub(crate) fn native_track(sender: String, track: MediaStreamTrack) -> Result<(), JsValue> {
    NATIVE.with(|n| { let mut n = n.borrow_mut(); let v = n.entry(sender).or_default(); if let Some(video) = &v.video { video.set_src_object(None); } v.track = Some(track); v.good=0; v.token=0; if let Some(c)=v.canvas.take(){c.remove();} v.attach() })
}
pub(crate) fn native_shape(sender: String, rotation: u16, width: u16, height: u16) -> Result<(), JsValue> {
    NATIVE.with(|n| { let mut n = n.borrow_mut(); let v = n.entry(sender).or_default(); v.shape = Some((rotation, width, height)); v.attach() })
}
pub(crate) fn native_clear() {
    NATIVE.with(|n| { for v in n.borrow_mut().values_mut() { v.track = None; v.shape = None; v.good=0; v.token=0; if let Some(c)=v.canvas.take(){c.remove();} if let Some(video) = &v.video { video.set_src_object(None); } } });
}
#[wasm_bindgen]
pub fn video_native_attach(sender: String, video: HtmlVideoElement) -> Result<(), JsValue> {
    video.set_muted(true); video.set_autoplay(true); video.set_attribute("playsinline", "")?;
    NATIVE.with(|n| { let mut n = n.borrow_mut(); let v = n.entry(sender).or_default(); v.video = Some(video); v.attach() })
}
#[wasm_bindgen]
pub fn video_native_detach(sender: String) {
    NATIVE.with(|n| { if let Some(v) = n.borrow_mut().get_mut(&sender) { if let Some(video) = v.video.take() { video.set_src_object(None); } if let Some(c)=v.canvas.take(){c.remove();} v.probe=None; v.token=0; v.good=0; } });
}
/// The first decoded images verify native presentation. Blank native output keeps software active.
pub(crate) fn native_frame(sender:&str, frame:&JsValue, rotation:u16, token:u32) -> Result<bool,JsValue> {
    NATIVE.with(|n| {
        let mut n=n.borrow_mut();let Some(v)=n.get_mut(sender) else{return Ok(false)};
        v.attach()?;
        let (Some(canvas),Some(video),Some(probe))=(&v.canvas,&v.video,&v.probe) else{return Ok(false)};
        if v.token!=token {v.token=token;v.good=0;v.failures=0;v.probe_at=0.0;canvas.remove_attribute("hidden")?;}
        let width=number(frame,"displayWidth").unwrap_or(0.0);let height=number(frame,"displayHeight").unwrap_or(0.0);
        let turned=rotation==90||rotation==270;let (cw,ch)=if turned{(height,width)}else{(width,height)};
        if canvas.width()!=cw as u32{canvas.set_width(cw as u32);}if canvas.height()!=ch as u32{canvas.set_height(ch as u32);}
        let ctx=canvas.get_context("2d")?.unwrap().dyn_into::<CanvasRenderingContext2d>()?;
        ctx.save();ctx.translate(cw/2.0,ch/2.0)?;ctx.rotate(f64::from(rotation)*std::f64::consts::PI/180.0)?;
        let painted=invoke(&ctx,"drawImage",&[frame.clone(),(-width/2.0).into(),(-height/2.0).into(),width.into(),height.into()]);ctx.restore();painted?;
        let ctx=probe.get_context("2d")?.unwrap().dyn_into::<CanvasRenderingContext2d>()?;
        let sample=|image:&JsValue|->Result<([f64;3],u8),JsValue>{
            invoke(&ctx,"drawImage",&[image.clone(),0.into(),0.into(),16.into(),16.into()])?;
            let data=ctx.get_image_data(0.0,0.0,16.0,16.0)?.data();let mut rgb=[0.0;3];let(mut low,mut high)=(255,0);
            for p in data.chunks_exact(4){for i in 0..3{rgb[i]+=f64::from(p[i])/256.0;low=low.min(p[i]);high=high.max(p[i]);}}
            Ok((rgb,high-low))
        };
        if video.ready_state()>=2 && js_sys::Date::now()>=v.probe_at {
            v.probe_at=js_sys::Date::now()+50.0;
            let (soft,contrast)=sample(frame)?;let(native,_)=sample(video.as_ref())?;
            let difference=(0..3).map(|i|(soft[i]-native[i]).abs()).fold(0.0,f64::max);
            if contrast>16&&difference<12.0 {v.good=v.good.saturating_add(1);}else{v.good=0;v.failures=v.failures.saturating_add(1);if v.failures>=12 {v.probe_at=js_sys::Date::now()+5000.0;}}
            if v.good>=12 {canvas.set_attribute("hidden","")?;return Ok(true);}
        }
        Ok(false)
    })
}
impl Drop for Capture {
    fn drop(&mut self) {
        for track in self.stream.get_tracks() { if let Ok(t) = track.dyn_into::<MediaStreamTrack>() { t.stop(); } }
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
    static GENERATION: Cell<u32> = const { Cell::new(0) };
    static VIEWERS: RefCell<HashMap<String, Viewer>> = RefCell::new(HashMap::new());
}
#[wasm_bindgen]
pub fn video_supported() -> bool { crate::rtc::browser_call_supported() }
#[wasm_bindgen]
pub fn video_camera_stop() {
    GENERATION.with(|g| g.set(g.get().wrapping_add(1)));
    CAPTURE.with(|c| c.borrow_mut().take());
    // A stopped track sends no media. Replacement on the next start is serialized by the caller.
}
#[wasm_bindgen]
pub fn video_camera_failed() -> bool {
    CAPTURE.with(|c| c.borrow().as_ref().is_some_and(|c| c.stream.get_video_tracks().iter().all(|t| get(&t, "readyState").ok().and_then(|v|v.as_string()).as_deref() == Some("ended"))))
}
pub(crate) fn camera_track() -> Option<MediaStreamTrack> {
    CAPTURE.with(|c| c.borrow().as_ref().and_then(|c| c.stream.get_video_tracks().get(0).dyn_into().ok()))
}
#[wasm_bindgen]
pub async fn video_camera_start(video: HtmlVideoElement, front: bool) -> Result<(), JsValue> {
    video_camera_stop(); let generation = GENERATION.with(Cell::get);
    let constraints = MediaStreamConstraints::new(); constraints.set_audio(&false.into());
    constraints.set_video(&object(serde_json::json!({"facingMode":if front {"user"} else {"environment"},"width":{"ideal":1920,"max":1920},"height":{"ideal":1080,"max":1080},"frameRate":{"ideal":60,"max":60}}))?);
    let window = web_sys::window().ok_or_else(|| fail("Missing window"))?;
    let stream = JsFuture::from(window.navigator().media_devices()?.get_user_media_with_constraints(&constraints)?).await?.dyn_into::<MediaStream>()?;
    let capture = Capture { stream, video };
    if generation != GENERATION.with(Cell::get) { return Err(fail("Camera cancelled")); }
    let track = capture.stream.get_video_tracks().get(0).dyn_into::<MediaStreamTrack>()?;
    capture.video.set_muted(true); capture.video.set_attribute("playsinline", "")?; capture.video.set_src_object(Some(&capture.stream));
    JsFuture::from(capture.video.play()?).await?;
    if generation != GENERATION.with(Cell::get) { return Err(fail("Camera cancelled")); }
    crate::rtc::camera_track(Some(&track)).await?;
    if generation != GENERATION.with(Cell::get) { return Err(fail("Camera cancelled")); }
    let settings = invoke(&track,"getSettings",&[])?;
    web_sys::console::log_1(&format!("SigilTiming video capture {}x{}@{} AV1 RTP", number(&settings,"width").unwrap_or(0.0),number(&settings,"height").unwrap_or(0.0),number(&settings,"frameRate").unwrap_or(0.0)).into());
    CAPTURE.with(|c| *c.borrow_mut() = Some(capture)); Ok(())
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
    let error = Closure::<dyn FnMut(JsValue)>::new(move |error: JsValue| {
        let name = get(&error, "name").ok().and_then(|v| v.as_string()).unwrap_or_default();
        web_sys::console::log_1(&format!("SigilTiming video decode failed {name}").into());
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
        if timestamp <= viewer.last && viewer.last != 0 {
            if !keyframe { return Ok(true); }
            if let Some(decoder) = viewer.decoder.take() { let _ = invoke(&decoder, "close", &[]); }
        }
        viewer.last = timestamp;
        if keyframe { viewer.awaiting_key = false; }
        if viewer.awaiting_key { return Ok(true); }
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
                    "hardwareAcceleration": "no-preference", "optimizeForLatency": true
                }))?],
            )?;
            viewer.decoder = Some(decoder);
            viewer.size = (width, height);
            viewer.codec = codec;
        }
        viewer.rotation = rotation;
        let decoder = viewer.decoder.clone().ok_or_else(|| fail("Missing decoder"))?;
        if number(&decoder, "decodeQueueSize").unwrap_or(0.0) > 3.0 {
            let _ = invoke(&decoder, "close", &[]); viewer.decoder = None;
            viewer.awaiting_key = true; return Ok(true);
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
