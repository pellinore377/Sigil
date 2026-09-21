//! Software fallback for native decoder failures. The native RTP pipeline retains its feedback.
use crate::{
    get,
    rtc::{construct, invoke, object},
    set,
};
use js_sys::{Array, Uint8Array};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap},
    rc::{Rc, Weak},
};
use wasm_bindgen::{JsCast, prelude::*};

thread_local! {
    static NEXT: Cell<u32> = const {Cell::new(0)};
    static LIVE: RefCell<HashMap<u32, Weak<RefCell<State>>>> = RefCell::new(HashMap::new());
}
struct State {
    decoder: Option<JsValue>,
    native: bool,
    failed: bool,
    waiting: bool,
    pending: Option<Frame>,
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
}
struct Frame {
    value: JsValue,
    revision: i32,
    rotation: u16,
}
fn close(frame: &JsValue) {
    let _ = invoke(frame, "close", &[]);
}
impl Drop for Decoder {
    fn drop(&mut self) {
        LIVE.with(|v| v.borrow_mut().remove(&self.id));
        let mut s = self.state.borrow_mut();
        if let Some(d) = s.decoder.take() {
            close(&d);
        }
        if let Some(f) = s.pending.take() {
            close(&f.value);
        }
    }
}
fn post(s: &mut State, id: u32, frame: Frame) {
    if s.native || !crate::media_worker::valid_revision(frame.revision) {
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
    let result = message.and_then(|m| {
        set(&m, "frame", &frame.value)?;
        js_sys::global()
            .unchecked_into::<web_sys::DedicatedWorkerGlobalScope>()
            .post_message_with_transfer(&m, &Array::of1(&frame.value))
    });
    if result.is_ok() {
        s.waiting = true;
    } else {
        close(&frame.value);
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
            native: false,
            failed: false,
            waiting: false,
            pending: None,
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
                    post(
                        &mut s,
                        id,
                        Frame {
                            value: frame,
                            revision,
                            rotation,
                        },
                    );
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
        Self {
            id,
            state,
            output,
            error,
        }
    }
    pub(crate) fn discontinuity(&self) {
        let mut s = self.state.borrow_mut();
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
        if s.native {
            return Ok(true);
        }
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
                Err(_) => {
                    s.native = true;
                    crate::transform::timing(
                        "SigilTiming software video fallback unavailable".into(),
                    );
                    return Ok(true);
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
                if get(data, "native").ok().and_then(|v| v.as_bool()) == Some(true) {
                    s.native = true;
                    if let Some(d) = s.decoder.take() {
                        close(&d);
                    }
                    if let Some(f) = s.pending.take() {
                        close(&f.value);
                    }
                } else if let Some(frame) = s.pending.take() {
                    post(&mut s, id, frame);
                }
            }
        });
        true
    }
}
