//! Encoded transforms. The browser captures, encodes, decodes and plays audio natively on its own
//! threads and hands each encoded frame here, where it is sealed or opened inside the worker that
//! already holds the call's keys. Nothing media related runs on the page's main thread.
use crate::rtc::invoke;
use crate::{fail, get, set};
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::{spawn_local, JsFuture};

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
        let converted = convert(&frame, sealing, &sender);
        if converted.is_err() || matches!(converted, Ok(None)) {
            dropped += 1;
            if dropped % 50 == 1 {
                web_sys::console::log_1(&JsValue::from_str(&format!(
                    "SigilTiming call transform dropped={dropped} sealing={sealing}"
                )));
            }
            continue;
        }
        let Ok(Some(payload)) = converted else { continue };
        let buffer = js_sys::Uint8Array::from(payload.as_slice()).buffer();
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
