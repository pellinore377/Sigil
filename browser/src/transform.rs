//! Encoded transforms. The browser captures, encodes, decodes and plays audio natively on its own
//! threads and hands each encoded frame here, where it is sealed or opened inside the worker that
//! already holds the call's keys. Nothing media related runs on the page's main thread.
use crate::rtc::invoke;
use crate::{fail, get, set};
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::{spawn_local, JsFuture};

thread_local! {
    static ACKS: std::cell::RefCell<std::collections::BTreeMap<u64, futures_channel::oneshot::Sender<()>>> = const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
    static NEXT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}
pub(crate) fn acknowledge(id: u64) {
    ACKS.with(|a| { if let Some(reply) = a.borrow_mut().remove(&id) { let _ = reply.send(()); } });
}

/// The worker's transform entry point; the page attaches a sender or receiver to it by name.
pub(crate) fn install() -> Result<(), JsValue> {
    let handler = Closure::<dyn FnMut(JsValue)>::new(move |event: JsValue| {
        spawn_local(async move {
            let _ = pump(event).await;
        });
    });
    js_sys::Reflect::set(
        &js_sys::global(),
        &"onrtctransform".into(),
        handler.as_ref().unchecked_ref(),
    )?;
    handler.forget();
    Ok(())
}

/// One transform runs for the life of one sender or receiver: read a frame, seal or open it,
/// write it on. A frame that cannot be opened is dropped rather than passed through in the clear.
async fn pump(event: JsValue) -> Result<(), JsValue> {
    let transformer = get(&event, "transformer")?;
    let options = get(&transformer, "options")?;
    let sealing = get(&options, "operation")?.as_string().as_deref() == Some("seal");
    let sender = get(&options, "sender")
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default();
    let video = get(&options, "kind").ok().and_then(|v| v.as_string()).as_deref() == Some("camera");
    let call = get(&options, "call").ok().and_then(|v| v.as_string()).unwrap_or_default();
    let mut last_stamp = None::<u32>;
    let mut elapsed = 0u64;
    let mut key_requested = 0u64;
    let reader = invoke(&get(&transformer, "readable")?, "getReader", &[])?;
    let writer = invoke(&get(&transformer, "writable")?, "getWriter", &[])?;
    let mut dropped = 0u64;
    loop {
        let read = JsFuture::from(invoke(&reader, "read", &[])?.unchecked_into::<js_sys::Promise>())
            .await?;
        if get(&read, "done")?.as_bool() == Some(true) {
            return Ok(());
        }
        let frame = get(&read, "value")?;
        let converted = if video {
            let bytes = js_sys::Uint8Array::new(&get(&frame, "data")?).to_vec();
            if sealing {
                let meta = invoke(&frame, "getMetadata", &[])?;
                let dimension = |key| get(&meta, key).ok().and_then(|v| v.as_f64()).unwrap_or(0.0) as u16;
                let stamp = get(&frame, "timestamp")?.as_f64().unwrap_or(0.0) as u32;
                if let Some(last) = last_stamp { elapsed += u64::from(stamp.wrapping_sub(last)); }
                last_stamp = Some(stamp);
                if elapsed.saturating_sub(key_requested) >= 90_000 {
                    let _ = invoke(&transformer, "generateKeyFrame", &[]); key_requested = elapsed;
                }
                let key = get(&frame, "type")?.as_string().as_deref() == Some("key");
                crate::call::seal_video(&call, &bytes, elapsed * 1000 / 90, key, dimension("width"), dimension("height")).map(Some)
            } else { crate::call::open_video(&call, &sender, &bytes).map(Some) }
        } else { convert(&frame, sealing, &sender) };
        let reason = match &converted {
            Err(error) => Some(error.as_string().unwrap_or_else(|| "unknown".into())),
            Ok(None) => Some("incomplete".into()),
            Ok(Some(_)) => None,
        };
        if let Some(reason) = reason {
            dropped += 1;
            // The reason separates a muted track, whose frames are meant to stop here, from a
            // key or state failure, which is not.
            if dropped % 250 == 1 {
                web_sys::console::log_1(&JsValue::from_str(&format!(
                    "SigilTiming call transform dropped={dropped} sealing={sealing} reason={reason}"
                )));
            }
            continue;
        }
        let Ok(Some(payload)) = converted else { continue };
        let buffer = js_sys::Uint8Array::from(payload.as_slice()).buffer();
        if video && !sealing {
            // Deliver authenticated AV1 to the renderer, preserving its rotation metadata.
            let id = NEXT.with(|n| { let id = n.get().wrapping_add(1); n.set(id); id });
            let (reply, received) = futures_channel::oneshot::channel();
            ACKS.with(|a| a.borrow_mut().insert(id, reply));
            let message = crate::rtc::object(serde_json::json!({"video_frame": true, "call": call, "sender": sender, "ack": id.to_string()}))?;
            set(&message, "data", &buffer)?;
            let worker: web_sys::DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
            if let Err(error) = worker.post_message_with_transfer(&message, &js_sys::Array::of1(&buffer)) { acknowledge(id); return Err(error); }
            // One transferable frame per receiver may await the page; never queue decoded video indefinitely.
            let _ = received.await;
            continue;
        }
        set(&frame, "data", &buffer.into())?;
        JsFuture::from(invoke(&writer, "write", &[frame])?.unchecked_into::<js_sys::Promise>())
            .await?;
    }
}

fn convert(frame: &JsValue, sealing: bool, sender: &str) -> Result<Option<Vec<u8>>, JsValue> {
    let bytes = js_sys::Uint8Array::new(&get(frame, "data")?).to_vec();
    if bytes.is_empty() || bytes.len() > 1024 * 1024 {
        return Err(fail("Invalid encoded frame"));
    }
    if sealing {
        // The RTP timestamp counts 48 kHz samples; sealing binds microseconds.
        let stamp = get(frame, "timestamp")?.as_f64().unwrap_or(0.0).max(0.0) as u64;
        Ok(Some(crate::call::seal_audio(&bytes, stamp * 1000 / 48)?))
    } else {
        Ok(crate::call::open_audio(sender, &bytes)?.map(|frame| frame.to_vec()))
    }
}
