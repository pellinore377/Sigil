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
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_futures::{JsFuture, spawn_local};

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
    last_number: Option<u32>,
    last_input: (bool, usize, i64),
    recent: VecDeque<(bool, usize, i64, Vec<u8>)>,
    output_wait: Option<f64>,
    produced_output: bool,
    report_at: f64,
    outputs: u32,
    matched: u32,
    posted: u32,
    acknowledged: u32,
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
fn queued(s: &State) -> f64 {
    s.decoder
        .as_ref()
        .and_then(|d| get(d, "decodeQueueSize").ok())
        .and_then(|n| n.as_f64())
        .unwrap_or(0.0)
}
async fn dequeue(decoder: &JsValue, timeout: i32) -> Result<(), JsValue> {
    let mut timer = None;
    let next = js_sys::Promise::new(&mut |resolve, reject| {
        let installed = set(decoder, "ondequeue", &resolve).and_then(|_| {
            worker().set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, timeout)
        });
        match installed {
            Ok(id) => timer = Some(id),
            Err(error) => {
                let _ = reject.call1(&JsValue::UNDEFINED, &error);
            }
        }
    });
    let result = JsFuture::from(next).await.map(|_| ());
    let _ = set(decoder, "ondequeue", &JsValue::NULL);
    if let Some(timer) = timer {
        worker().clear_timeout_with_handle(timer);
    }
    result
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
        if sent.is_ok() {
            s.posted += 1;
        }
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
            last_number: None,
            last_input: (false, 0, 0),
            recent: VecDeque::new(),
            output_wait: None,
            produced_output: false,
            report_at: now(),
            outputs: 0,
            matched: 0,
            posted: 0,
            acknowledged: 0,
            call,
            sender,
            generation,
        }));
        LIVE.with(|v| v.borrow_mut().insert(id, Rc::downgrade(&state)));
        let weak = Rc::downgrade(&state);
        let output = Closure::<dyn FnMut(JsValue)>::new(move |frame| {
            if let Some(s) = weak.upgrade() {
                let mut s = s.borrow_mut();
                s.output_wait = None;
                s.produced_output = true;
                s.outputs += 1;
                let timestamp = get(&frame, "timestamp")
                    .ok()
                    .and_then(|v| v.as_f64())
                    .unwrap_or(-1.0) as u64;
                if let Some((revision, rotation)) = s.submitted.remove(&timestamp) {
                    s.matched += 1;
                    let due = s.clock.due(timestamp, now());
                    while s.ready.len() >= pacing::READY_FRAMES {
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
        let error = Closure::<dyn FnMut(JsValue)>::new(move |error: JsValue| {
            let name = get(&error, "name")
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_default();
            let message = get(&error, "message")
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_default();
            crate::transform::timing(format!("SigilTiming video decoder error={name}: {message}"));
            if let Some(s) = weak.upgrade() {
                let mut s = s.borrow_mut();
                crate::transform::timing(format!("SigilTiming video decoder input key={} bytes={} stamp_gap_us={}", s.last_input.0, s.last_input.1, s.last_input.2));
                crate::transform::timing(format!("SigilTiming video decoder recent={:?}",s.recent));
                s.failed = true;
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
        s.output_wait = None;
        s.produced_output = false;
    }
    /// Returns false when a fresh keyframe is needed. Decoding never waits on the page.
    pub(crate) async fn push(&self, payload: &[u8]) -> Result<bool, JsValue> {
        let revision = crate::media_worker::revision().unwrap_or(-1);
        // Preserve reference frames during receive bursts. Wait for decoder capacity,
        // not a larger playback buffer; abort sustained overload after the stall budget.
        // Initial codec configuration is asynchronous and can outlast a frame stall.
        let budget = if self.state.borrow().produced_output {
            250.0
        } else {
            1000.0
        };
        let until = now() + budget;
        loop {
            let decoder = {
                let s = self.state.borrow();
                if s.failed || queued(&s) <= 3.0 {
                    None
                } else {
                    s.decoder.clone()
                }
            };
            let Some(decoder) = decoder else { break };
            let remaining = until - now();
            if remaining <= 0.0 {
                break;
            }
            dequeue(&decoder, remaining.ceil() as i32).await?;
        }
        // The input was authenticated before yielding; never relabel it with newer authority.
        if !crate::media_worker::valid_revision(revision) {
            self.discontinuity();
            crate::transform::timing("SigilTiming video decoder recovery=authority changed".into());
            return Ok(false);
        }
        let camera = sigil_calls::av1::camera_payload(payload)
            .map_err(|_| crate::fail("Invalid camera frame"))?;
        let (rotation, encoded) = (camera.rotation, camera.encoded);
        // Only a shown KEY_FRAME can restart a decoder; senders may flag intra-only frames.
        let key = payload[1] != 0 && sigil_calls::av1::random_access(encoded);
        // Chrome hands over only complete frames, so a lost one shows up as a numbering gap.
        // Decoding past it paints against a missing reference until the next keyframe.
        let previous = std::mem::replace(&mut self.state.borrow_mut().last_number, camera.number);
        if !key && camera.number.is_some_and(|n| previous.is_some_and(|p| n != p.wrapping_add(1))) {
            self.discontinuity();
            crate::transform::timing(format!("SigilTiming video decoder recovery=lost frames={}", camera.number.unwrap_or(0).wrapping_sub(previous.unwrap_or(0)).wrapping_sub(1)));
            return Ok(false);
        }
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
        if now() - s.report_at >= 5000.0 {
            crate::transform::timing(format!(
                "SigilTiming video decode outputs={} matched={} posted={} ack={} waiting={} ready={} pending={}",
                s.outputs,
                s.matched,
                s.posted,
                s.acknowledged,
                s.waiting,
                s.ready.len(),
                s.pending.is_some()
            ));
            s.outputs = 0;
            s.matched = 0;
            s.posted = 0;
            s.acknowledged = 0;
            s.report_at = now();
        }
        let stalled = s.output_wait.is_some_and(|since| now() - since >= budget);
        if stalled {
            crate::transform::timing("SigilTiming video decoder recovery=stalled output".into());
        }
        let queued = queued(&s);
        if queued > 3.0 || s.failed {
            crate::transform::timing(format!(
                "SigilTiming video decoder recovery=reset queued={queued} failed={}",
                s.failed
            ));
        }
        if stalled || s.failed || queued > 3.0 {
            if let Some(d) = s.decoder.take() {
                close(&d);
            }
            s.submitted.clear();
            s.failed = false;
            s.output_wait = None;
            s.produced_output = false;
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
        s.output_wait.get_or_insert_with(now);
        s.last_input = (key, encoded.len(), s.last_stamp.map_or(0, |last| timestamp as i64 - last as i64));
        let diagnostic = frame_headers(encoded);
        let input = s.last_input;
        s.recent.push_back((input.0, input.1, input.2, diagnostic));
        if s.recent.len() > 8 { s.recent.pop_front(); }
        s.last_stamp = Some(timestamp);
        s.submitted.insert(timestamp, (revision, rotation));
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
                s.acknowledged += 1;
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

fn frame_headers(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new(); let mut at = 0;
    while at < bytes.len() {
        let header=bytes[at]; at+=1;
        if header & 4 != 0 { at+=1; }
        if header & 2 == 0 {break;}
        let mut size=0usize; let mut shift=0;
        loop { let Some(&b)=bytes.get(at) else {return out}; at+=1; size|=usize::from(b&127)<<shift; if b&128==0 {break;} shift+=7;if shift>28{return out;} }
        if matches!((header>>3)&15,3|6) {if let Some(&b)=bytes.get(at) {out.push(b>>3);}}
        let Some(end)=at.checked_add(size) else {break}; at=end;
    }
    out
}
