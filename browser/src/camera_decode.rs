//! Software AV1 decoding and bounded presentation outside the control worker.
use crate::{
    get,
    rtc::{construct, invoke, object},
    set,
};
use js_sys::{Array, Uint8Array};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap, VecDeque},
    rc::{Rc, Weak},
};
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::{spawn_local, JsFuture};

#[path = "camera_pacing.rs"]
mod pacing;
thread_local! {
    static NEXT: Cell<u32> = const {Cell::new(0)};
    static LIVE: RefCell<HashMap<u32, Weak<RefCell<State>>>> = RefCell::new(HashMap::new());
}
struct State {
    decoder: Option<JsValue>,
    pixels: Option<Uint8Array>,
    failed: bool,
    waiting: bool,
    pending: Option<Frame>,
    ready: VecDeque<(f64, Frame)>,
    clock: pacing::Clock,
    timer: Option<i32>,
    tick: Option<js_sys::Function>,
    submitted: BTreeMap<u64, (i32, u16)>,
    last_stamp: Option<u64>,
    call: String,
    sender: String,
    generation: u64,
}
pub(crate) struct Decoder {
    id: u32,
    state: Rc<RefCell<State>>,
    output: Closure<dyn FnMut(JsValue)>,
    error: Closure<dyn FnMut(JsValue)>,
    _tick: Closure<dyn FnMut()>,
}
struct Frame {
    value: JsValue,
    revision: i32,
    rotation: u16,
}
fn close(frame: &JsValue) {
    let _ = invoke(frame, "close", &[]);
}
fn worker() -> web_sys::WorkerGlobalScope {
    js_sys::global().unchecked_into()
}
fn now() -> f64 {
    worker()
        .performance()
        .map(|p| p.now())
        .unwrap_or_else(js_sys::Date::now)
}
fn unqueue(s: &mut State) {
    if let Some(timer) = s.timer.take() {
        worker().clear_timeout_with_handle(timer);
    }
    for (_, frame) in s.ready.drain(..) {
        close(&frame.value);
    }
    s.clock = pacing::Clock::default();
}
fn arm(s: &mut State) {
    if s.timer.is_some() {
        return;
    }
    if let (Some((due, _)), Some(tick)) = (s.ready.front(), s.tick.as_ref()) {
        s.timer = worker()
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                tick,
                (due - now()).ceil().max(0.0) as i32,
            )
            .ok();
        if s.timer.is_none() {
            unqueue(s);
        }
    }
}
impl Drop for Decoder {
    fn drop(&mut self) {
        LIVE.with(|v| v.borrow_mut().remove(&self.id));
        let mut s = self.state.borrow_mut();
        unqueue(&mut s);
        if let Some(d) = s.decoder.take() {
            close(&d);
        }
        if let Some(f) = s.pending.take() {
            close(&f.value);
        }
    }
}
fn post(s: &mut State, id: u32, frame: Frame) {
    if !crate::media_worker::valid_revision(frame.revision) {
        close(&frame.value);
        return;
    }
    if s.waiting {
        if let Some(old) = s.pending.replace(frame) {
            close(&old.value);
        }
        return;
    }
    let message = object(
        serde_json::json!({"video_frame":true,"id":id,"call":s.call,"sender":s.sender,"generation":s.generation,"rotation":frame.rotation,"revision":frame.revision,"until":crate::media_worker::deadline()}),
    );
    let width = get(&frame.value, "visibleRect")
        .and_then(|v| get(&v, "width"))
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as u32;
    let height = get(&frame.value, "visibleRect")
        .and_then(|v| get(&v, "height"))
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as u32;
    if width == 0
        || height == 0
        || width > 3840
        || height > 3840
        || u64::from(width) * u64::from(height) > 3840 * 2160
    {
        close(&frame.value);
        return;
    }
    let pixels = s
        .pixels
        .take()
        .filter(|p| p.length() == width * height * 4)
        .unwrap_or_else(|| Uint8Array::new_with_length(width * height * 4));
    let copy = object(serde_json::json!({"format":"RGBA","colorSpace":"srgb"}))
        .and_then(|options| invoke(&frame.value, "copyTo", &[pixels.clone().into(), options]));
    s.waiting = true;
    // Copy pixels off the page. drawImage(VideoFrame) can synchronously wait for GPU fences,
    // even with a software decoder and an explicitly CPU-backed canvas.
    spawn_local(async move {
        let copied = match copy {
            Ok(promise) => JsFuture::from(js_sys::Promise::resolve(&promise))
                .await
                .map(|_| ()),
            Err(e) => Err(e),
        };
        if let Err(error) = &copied {
            crate::transform::timing(format!(
                "SigilTiming video pixel copy failed {}",
                get(error, "name")
                    .ok()
                    .and_then(|v| v.as_string())
                    .unwrap_or_default()
            ));
        }
        close(&frame.value);
        let state = LIVE.with(|live| live.borrow().get(&id).and_then(Weak::upgrade));
        let Some(state) = state else { return };
        let mut s = state.borrow_mut();
        let sent = copied.and_then(|_| {
            if !crate::media_worker::valid_revision(frame.revision) {
                return Err(crate::fail("Expired video frame"));
            }
            let message = message?;
            let image = object(serde_json::json!({"displayWidth":width,"displayHeight":height}))?;
            set(&image, "pixels", &pixels)?;
            set(&message, "frame", &image)?;
            js_sys::global()
                .unchecked_into::<web_sys::DedicatedWorkerGlobalScope>()
                .post_message_with_transfer(&message, &Array::of1(&pixels.buffer()))
        });
        if sent.is_err() {
            s.waiting = false;
            s.pixels = Some(pixels);
            if let Some(next) = s.pending.take() {
                post(&mut s, id, next);
            }
        }
    });
}
// Decoder output is a live media clock even when browser timers are throttled.
fn present(s: &mut State, id: u32, at: f64) {
    let mut latest: Option<Frame> = None;
    while s.ready.front().is_some_and(|(due, _)| *due <= at) {
        if let Some(old) = latest.take() {
            close(&old.value);
        }
        latest = s.ready.pop_front().map(|(_, frame)| frame);
    }
    if let Some(frame) = latest {
        post(s, id, frame);
    }
}
impl Decoder {
    pub(crate) fn new(call: String, sender: String, generation: u64) -> Self {
        let id = NEXT.with(|n| {
            n.set(n.get().wrapping_add(1));
            n.get()
        });
        let state = Rc::new(RefCell::new(State {
            decoder: None,
            pixels: None,
            failed: false,
            waiting: false,
            pending: None,
            ready: VecDeque::new(),
            clock: pacing::Clock::default(),
            timer: None,
            tick: None,
            submitted: BTreeMap::new(),
            last_stamp: None,
            call,
            sender,
            generation,
        }));
        LIVE.with(|v| v.borrow_mut().insert(id, Rc::downgrade(&state)));
        let weak = Rc::downgrade(&state);
        let output = Closure::<dyn FnMut(JsValue)>::new(move |frame| {
            if let Some(s) = weak.upgrade() {
                let mut s = s.borrow_mut();
                let timestamp = get(&frame, "timestamp")
                    .ok()
                    .and_then(|v| v.as_f64())
                    .unwrap_or(-1.0) as u64;
                if let Some((revision, rotation)) = s.submitted.remove(&timestamp) {
                    let due = s.clock.due(timestamp, now());
                    while s.ready.len() >= 6 {
                        if let Some((_, old)) = s.ready.pop_front() {
                            close(&old.value);
                        }
                    }
                    s.ready.push_back((
                        due,
                        Frame {
                            value: frame,
                            revision,
                            rotation,
                        },
                    ));
                    present(&mut s, id, now());
                    arm(&mut s);
                } else {
                    close(&frame);
                }
            } else {
                close(&frame);
            }
        });
        let weak = Rc::downgrade(&state);
        let error = Closure::<dyn FnMut(JsValue)>::new(move |_| {
            if let Some(s) = weak.upgrade() {
                s.borrow_mut().failed = true;
            }
        });
        let weak = Rc::downgrade(&state);
        let tick = Closure::<dyn FnMut()>::new(move || {
            if let Some(state) = weak.upgrade() {
                let mut s = state.borrow_mut();
                s.timer = None;
                present(&mut s, id, now());
                arm(&mut s);
            }
        });
        state.borrow_mut().tick = Some(tick.as_ref().unchecked_ref::<js_sys::Function>().clone());
        Self {
            id,
            state,
            output,
            error,
            _tick: tick,
        }
    }
    pub(crate) fn discontinuity(&self) {
        let mut s = self.state.borrow_mut();
        unqueue(&mut s);
        if let Some(d) = s.decoder.take() {
            close(&d);
        }
        if let Some(f) = s.pending.take() {
            close(&f.value);
        }
        s.submitted.clear();
        s.last_stamp = None;
    }
    /// Returns false when a fresh keyframe is needed. Decoding never waits on the page.
    pub(crate) fn push(&self, payload: &[u8]) -> Result<bool, JsValue> {
        let (rotation, _, _, encoded) = sigil_calls::av1::camera_payload(payload)
            .map_err(|_| crate::fail("Invalid camera frame"))?;
        let key = payload[1] != 0;
        let timestamp = u64::from_be_bytes(payload[2..10].try_into().unwrap());
        if self
            .state
            .borrow()
            .last_stamp
            .is_some_and(|last| timestamp <= last)
        {
            self.discontinuity();
        }
        let mut s = self.state.borrow_mut();
        if s.failed
            || s.decoder.as_ref().is_some_and(|d| {
                get(d, "decodeQueueSize")
                    .ok()
                    .and_then(|n| n.as_f64())
                    .unwrap_or(0.0)
                    > 3.0
            })
        {
            if let Some(d) = s.decoder.take() {
                close(&d);
            }
            s.submitted.clear();
            s.failed = false;
        }
        if s.decoder.is_none() {
            if !key {
                return Ok(false);
            }
            let options = js_sys::Object::new();
            set(&options, "output", self.output.as_ref())?;
            set(&options, "error", self.error.as_ref())?;
            let configured = (|| -> Result<JsValue, JsValue> {
                let d = construct("VideoDecoder", &options)?;
                if let Err(error) = invoke(
                    &d,
                    "configure",
                    &[object(
                        serde_json::json!({"codec":"av01.0.08M.08","hardwareAcceleration":"prefer-software","optimizeForLatency":true}),
                    )?],
                ) {
                    close(&d);
                    return Err(error);
                }
                Ok(d)
            })();
            let d = match configured {
                Ok(d) => d,
                Err(error) => {
                    crate::transform::timing(
                        "SigilTiming software video decoder unavailable".into(),
                    );
                    return Err(error);
                }
            };
            s.decoder = Some(d);
        }
        s.last_stamp = Some(timestamp);
        s.submitted.insert(
            timestamp,
            (crate::media_worker::revision().unwrap_or(-1), rotation),
        );
        while s.submitted.len() > 8 {
            s.submitted.pop_first();
        }
        let options = object(
            serde_json::json!({"type":if key {"key"} else {"delta"},"timestamp":timestamp}),
        )?;
        set(&options, "data", &Uint8Array::from(encoded))?;
        if invoke(
            s.decoder.as_ref().unwrap(),
            "decode",
            &[construct("EncodedVideoChunk", &options)?],
        )
        .is_err()
        {
            s.failed = true;
            return Ok(false);
        }
        Ok(true)
    }
    pub(crate) fn acknowledged(data: &JsValue) -> bool {
        if get(data, "video_ack").ok().and_then(|v| v.as_bool()) != Some(true) {
            return false;
        }
        let id = get(data, "id").ok().and_then(|v| v.as_f64()).unwrap_or(0.0) as u32;
        LIVE.with(|live| {
            if let Some(state) = live.borrow().get(&id).and_then(Weak::upgrade) {
                let mut s = state.borrow_mut();
                s.waiting = false;
                s.pixels = get(data, "frame")
                    .and_then(|image| get(&image, "pixels"))
                    .ok()
                    .and_then(|v| v.dyn_into().ok());
                if let Some(frame) = s.pending.take() {
                    post(&mut s, id, frame);
                }
            }
        });
        true
    }
}
