//! The store and media workers communicate directly. A shared revision revokes old authority
//! synchronously, including while the control worker is blocked on its next network request.
use crate::{fail, get, set};
use js_sys::{Int32Array, Uint8Array};
use sigil_calls::{Id, MediaKind};
use sigil_client::calls::{MediaProcessor, MediaUpdate};
use std::cell::RefCell;
use wasm_bindgen::{JsCast, prelude::*};

struct Link {
    port: web_sys::MessagePort,
    gate: Int32Array,
}
thread_local! {
    static LINK: RefCell<Option<Link>> = const { RefCell::new(None) };
    static PROCESSING: std::cell::Cell<bool> = const {std::cell::Cell::new(false)};
    static PROCESSOR: RefCell<Processor> = RefCell::new(Processor::default());
}
#[derive(Default)]
struct Processor {
    media: MediaProcessor,
    revision: Option<i32>,
    until: f64,
    call: Option<Id>,
    assembly: sigil_calls::Assembly,
    renewing: bool,
}
pub(crate) fn processing() -> bool {
    PROCESSING.with(std::cell::Cell::get)
}
pub(crate) fn linked() -> bool {
    LINK.with(|v| v.borrow().is_some())
}
pub(crate) fn invalidate() {
    LINK.with(|v| {
        if let Some(link) = v.borrow().as_ref() {
            let _ = js_sys::Atomics::add(&link.gate, 0, 1);
        }
    });
}
pub(crate) fn counts() -> (u64, u64) {
    LINK.with(|v| {
        v.borrow()
            .as_ref()
            .map(|l| {
                (
                    js_sys::Atomics::load(&l.gate, 1).unwrap_or(0) as u32 as u64,
                    js_sys::Atomics::load(&l.gate, 2).unwrap_or(0) as u32 as u64,
                )
            })
            .unwrap_or_default()
    })
}
pub(crate) fn publish(update: MediaUpdate) -> Result<(), JsValue> {
    let bytes = zeroize::Zeroizing::new(
        serde_json::to_vec(&update).map_err(|_| fail("Invalid media update"))?,
    );
    LINK.with(|v| {
        let v = v.borrow();
        let link = v.as_ref().ok_or_else(|| fail("Media worker unavailable"))?;
        let message = js_sys::Object::new();
        set(
            &message,
            "revision",
            &js_sys::Atomics::load(&link.gate, 0)?.into(),
        )?;
        set(&message, "until", &(js_sys::Date::now() + 2000.0).into())?;
        let data = Uint8Array::from(bytes.as_slice());
        set(&message, "data", &data)?;
        link.port
            .post_message_with_transferable(&message, &js_sys::Array::of1(&data.buffer()))
    })
}
pub(crate) fn control_receive(event: &web_sys::MessageEvent) -> bool {
    let data = event.data();
    if get(&data, "media_link").ok().and_then(|v| v.as_bool()) != Some(true) {
        return false;
    }
    let result = (|| -> Result<(), JsValue> {
        let port = get(&data, "port")?.dyn_into::<web_sys::MessagePort>()?;
        let gate = Int32Array::new(&get(&data, "gate")?);
        let handler =
            Closure::<dyn FnMut(web_sys::MessageEvent)>::new(|event: web_sys::MessageEvent| {
                if let Some(raw) = event.data().as_string().filter(|s| s.len() < 2048) {
                    if let Ok(context) = serde_json::from_str(&raw) {
                        crate::call::renew_media(context);
                    }
                }
            });
        port.set_onmessage(Some(handler.as_ref().unchecked_ref()));
        handler.forget();
        port.start();
        LINK.with(|v| *v.borrow_mut() = Some(Link { port, gate }));
        let timer = Closure::<dyn FnMut()>::new(crate::call::publish_media);
        js_sys::global()
            .unchecked_into::<web_sys::DedicatedWorkerGlobalScope>()
            .set_interval_with_callback_and_timeout_and_arguments_0(
                timer.as_ref().unchecked_ref(),
                100,
            )?;
        timer.forget();
        crate::call::publish_media();
        Ok(())
    })();
    if result.is_err() {
        invalidate();
    }
    true
}
#[wasm_bindgen]
pub fn media_worker_start(port: web_sys::MessagePort, buffer: JsValue) -> Result<(), JsValue> {
    PROCESSING.with(|v| v.set(true));
    let messages =
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(|event: web_sys::MessageEvent| {
            if !crate::camera_decode::Decoder::acknowledged(&event.data()) {
                crate::call::receive(&event);
            }
        });
    js_sys::global()
        .unchecked_into::<web_sys::DedicatedWorkerGlobalScope>()
        .set_onmessage(Some(messages.as_ref().unchecked_ref()));
    messages.forget();
    let gate = Int32Array::new(&buffer);
    let handler =
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(|event: web_sys::MessageEvent| {
            let result = (|| -> Result<(), JsValue> {
                let data = event.data();
                let revision = get(&data, "revision")?
                    .as_f64()
                    .ok_or_else(|| fail("Invalid revision"))? as i32;
                let until = get(&data, "until")?
                    .as_f64()
                    .ok_or_else(|| fail("Invalid deadline"))?;
                let array = get(&data, "data")?.dyn_into::<Uint8Array>()?;
                if array.length() > 65536 {
                    array.fill(0, 0, array.length());
                    return Err(fail("Media update too large"));
                }
                let bytes = zeroize::Zeroizing::new(array.to_vec());
                array.fill(0, 0, array.length());
                let update: MediaUpdate =
                    serde_json::from_slice(&bytes).map_err(|_| fail("Invalid media update"))?;
                // Apply even an overtaken update: a one-use sender handoff cannot be dropped.
                // Its authority remains unusable until a matching revision arrives.
                PROCESSOR.with(|v| {
                    let mut p = v.borrow_mut();
                    if p.call != Some(update.call) {
                        p.assembly.clear();
                    }
                    p.call = Some(update.call);
                    p.media
                        .apply(update)
                        .map_err(|_| fail("Media update rejected"))?;
                    p.revision = Some(revision);
                    p.until = until;
                    p.renewing = false;
                    Ok(())
                })
            })();
            if result.is_err() {
                PROCESSOR.with(|v| v.borrow_mut().revision = None);
            }
        });
    port.set_onmessage(Some(handler.as_ref().unchecked_ref()));
    handler.forget();
    port.start();
    LINK.with(|v| *v.borrow_mut() = Some(Link { port, gate }));
    crate::transform::install()
}
fn process<T>(
    call: Option<Id>,
    sealing: bool,
    f: impl FnOnce(&mut Processor, Id, u64) -> Result<T, sigil_client::Error>,
) -> Result<T, JsValue> {
    LINK.with(|link| {
        let link = link.borrow();
        let link = link
            .as_ref()
            .ok_or_else(|| fail("Media worker unavailable"))?;
        PROCESSOR.with(|slot| {
            let mut p = slot.borrow_mut();
            let revision = js_sys::Atomics::load(&link.gate, 0)?;
            if p.revision != Some(revision) || js_sys::Date::now() >= p.until {
                return Err(fail("Media authority pending"));
            }
            let active = p.call.ok_or_else(|| fail("No media session"))?;
            if call.is_some_and(|call| call != active) {
                return Err(fail("Call changed"));
            }
            let result = f(&mut p, active, (js_sys::Date::now() / 1000.0) as u64);
            if matches!(result, Err(sigil_client::Error::Expired)) && sealing && !p.renewing {
                if let Some(context) = p.media.context() {
                    link.port.post_message(
                        &serde_json::to_string(&context)
                            .map_err(|_| fail("Invalid context"))?
                            .into(),
                    )?;
                    p.renewing = true;
                }
            }
            let output = result.map_err(|e| fail(&format!("Media rejected: {e:?}")))?;
            if js_sys::Atomics::load(&link.gate, 0)? != revision {
                return Err(fail("Media authority changed"));
            }
            let _ = js_sys::Atomics::add(&link.gate, if sealing { 1 } else { 2 }, 1);
            Ok(output)
        })
    })
}
pub(crate) fn seal_audio(call: &str, bytes: &[u8], timestamp: u64) -> Result<Vec<u8>, JsValue> {
    let call = crate::call::id(call)?;
    process(Some(call), true, |p, call, now| {
        let sealed = p
            .media
            .seal(call, MediaKind::Audio, timestamp, false, bytes, now)?;
        let packets = sigil_calls::packetize(MediaKind::Audio, &sealed)
            .map_err(|_| sigil_client::Error::InvalidEvent)?;
        if packets.len() != 1 {
            return Err(sigil_client::Error::Limit);
        }
        Ok(packets.into_iter().next().unwrap())
    })
}
pub(crate) fn open_audio(
    call: &str,
    sender: &str,
    bytes: &[u8],
) -> Result<Option<Vec<u8>>, JsValue> {
    let sender = crate::call::id(sender)?;
    let call = crate::call::id(call)?;
    process(Some(call), false, |p, call, now| {
        p.assembly
            .push(sender, MediaKind::Audio, bytes, web_time::Instant::now())
            .map_err(|_| sigil_client::Error::InvalidEvent)?
            .map(|bytes| {
                p.media
                    .open(call, sender, MediaKind::Audio, &bytes, now)
                    .map(|f| f.data.to_vec())
            })
            .transpose()
    })
}
pub(crate) fn seal_video(
    call: &str,
    bytes: &[u8],
    timestamp: u64,
    keyframe: bool,
    width: u16,
    height: u16,
) -> Result<Vec<u8>, JsValue> {
    if !sigil_calls::av1::camera_size(width, height) {
        return Err(fail("Invalid video dimensions"));
    }
    let call = crate::call::id(call)?;
    let mut body = zeroize::Zeroizing::new(vec![2, 0, 0]);
    body.extend_from_slice(&width.to_be_bytes());
    body.extend_from_slice(&height.to_be_bytes());
    body.extend_from_slice(bytes);
    process(Some(call), true, |p, call, now| {
        let sealed = p
            .media
            .seal(call, MediaKind::Camera, timestamp, keyframe, &body, now)?;
        sigil_calls::av1::wrap(&sealed, keyframe).map_err(|_| sigil_client::Error::InvalidEvent)
    })
}
pub(crate) fn open_video(call: &str, sender: &str, bytes: &[u8]) -> Result<Vec<u8>, JsValue> {
    let call = crate::call::id(call)?;
    let sender = crate::call::id(sender)?;
    let encrypted = sigil_calls::av1::unwrap(bytes).map_err(|_| fail("Invalid video envelope"))?;
    process(Some(call), false, |p, call, now| {
        let frame = p
            .media
            .open(call, sender, MediaKind::Camera, encrypted, now)?;
        let mut bytes = vec![1, u8::from(frame.keyframe)];
        bytes.extend_from_slice(&frame.timestamp.to_be_bytes());
        bytes.extend_from_slice(&frame.data);
        Ok(bytes)
    })
}

pub(crate) fn seal_frame(
    call: Id,
    kind: MediaKind,
    timestamp: u64,
    keyframe: bool,
    bytes: &[u8],
) -> Result<Vec<u8>, JsValue> {
    process(Some(call), true, |p, call, now| {
        p.media.seal(call, kind, timestamp, keyframe, bytes, now)
    })
}
pub(crate) fn open_frame(
    call: Id,
    sender: Id,
    kind: MediaKind,
    bytes: &[u8],
) -> Result<Vec<u8>, JsValue> {
    process(Some(call), false, |p, call, now| {
        let frame = p.media.open(call, sender, kind, bytes, now)?;
        let mut bytes = vec![frame.kind as u8, u8::from(frame.keyframe)];
        bytes.extend_from_slice(&frame.timestamp.to_be_bytes());
        bytes.extend_from_slice(&frame.data);
        Ok(bytes)
    })
}

pub(crate) fn revision() -> Option<i32> {
    PROCESSOR.with(|p| p.borrow().revision)
}
pub(crate) fn deadline() -> f64 {
    PROCESSOR.with(|p| {
        let p = p.borrow();
        p.until.min(p.media.expires_at().unwrap_or(0) as f64 * 1000.0)
    })
}
pub(crate) fn valid_revision(revision: i32) -> bool {
    LINK.with(|l| {
        l.borrow()
            .as_ref()
            .is_some_and(|l| js_sys::Atomics::load(&l.gate, 0).ok() == Some(revision))
    }) && PROCESSOR.with(|p| {
        let p = p.borrow();
        p.revision == Some(revision)
            && js_sys::Date::now() < p.until
            && p.media.is_live((js_sys::Date::now() / 1000.0) as u64)
    })
}
