//! Encoded transforms run in the dedicated media worker, isolated from storage and network waits.
use crate::rtc::invoke;
use crate::{fail, get, set};
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::{spawn_local, JsFuture};

pub(crate) fn timing(message: String) {
    let packet = js_sys::Object::new();
    let worker: web_sys::DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
    if set(&packet, "media_timing", &message.into()).is_ok() {
        let _ = worker.post_message(&packet);
    }
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
    let software = (video && !sealing).then(|| std::rc::Rc::new(crate::camera_decode::Decoder::new(call.clone(),sender.clone(),get(&options,"generation").ok().and_then(|v|v.as_f64()).unwrap_or(0.0) as u64)));
    let mut last_stamp = None::<u32>;
    let mut elapsed = 0u64;
    let reader = invoke(&get(&transformer, "readable")?, "getReader", &[])?;
    let writer = invoke(&get(&transformer, "writable")?, "getWriter", &[])?;
    let mut dropped = 0u64;
    let mut recovery_at = 0.0;
    let mut shape = None;
    let mut sequence = sigil_calls::av1::Sequence::default();
    let mut encoded_size = None;
    let mut numbered = 0u32;
    #[cfg(feature = "video-acceptance")]
    let mut fixture_frame = 0u32;
    let mut video_since = js_sys::Date::now();
    let mut video_last = video_since;
    let (mut video_count, mut video_gap, mut video_open, mut video_ack) = (0u32, 0f64, 0f64, 0f64);
    loop {
        #[cfg(feature = "video-acceptance")]
        if video && !sealing { crate::video_acceptance::receive_burst(&options, fixture_frame).await?; }
        let read = JsFuture::from(invoke(&reader, "read", &[])?.unchecked_into::<js_sys::Promise>())
            .await?;
        if get(&read, "done")?.as_bool() == Some(true) {
            return Ok(());
        }
        let frame = get(&read, "value")?;
        let arrived = js_sys::Date::now();
        let converted = if video {
            let bytes = js_sys::Uint8Array::new(&get(&frame, "data")?).to_vec();
            if sealing {
                let meta = invoke(&frame, "getMetadata", &[])?;
                let dimension = |key| get(&meta, key).ok().and_then(|v| v.as_f64()).unwrap_or(0.0) as u16;
                let stamp = get(&frame, "timestamp")?.as_f64().unwrap_or(0.0) as u32;
                if let Some(last) = last_stamp { elapsed += u64::from(stamp.wrapping_sub(last)); }
                last_stamp = Some(stamp);
                let key = get(&frame, "type")?.as_string().as_deref() == Some("key");
                let size = (dimension("width"), dimension("height"));
                if encoded_size != Some(size) { sequence.clear(); encoded_size = Some(size); }
                sequence.frame(&bytes, key).map_err(|_| fail("Invalid AV1 configuration"))
                    .and_then(|encoded| {
                        let owned;
                        let encoded = match encoded {
                            std::borrow::Cow::Borrowed(bytes) => bytes,
                            std::borrow::Cow::Owned(bytes) => { owned = zeroize::Zeroizing::new(bytes); &owned[..] }
                        };
                        numbered = numbered.wrapping_add(1);
                        crate::media_worker::seal_video(&call, encoded, elapsed * 1000 / 90, key, size.0, size.1, numbered).map(Some)
                    })
            } else { crate::media_worker::open_video(&call, &sender, &bytes).map(Some) }
        } else { convert(&frame, sealing, &call, &sender) };
        #[cfg(feature = "video-acceptance")]
        let converted = {
            fixture_frame += 1;
            if video && !sealing && get(&options, "drop_bursts").ok().and_then(|v|v.as_bool()) == Some(true) && fixture_frame % 600 < 10 {
                Err(fail("Synthetic receive interruption"))
            } else { converted }
        };
        let converted_at = js_sys::Date::now();
        let reason = match &converted {
            Err(error) => Some(error.dyn_ref::<js_sys::Error>().map(|e| String::from(e.message())).or_else(||error.as_string()).unwrap_or_else(|| "unknown".into())),
            Ok(None) => Some("incomplete".into()),
            Ok(Some(_)) => None,
        };
        if let Some(reason) = reason {
            if let Some(decoder)=&software {decoder.discontinuity();}
            dropped += 1;
            if video && arrived - recovery_at >= 250.0 {
                recovery_at = arrived;
                if !sealing { request_keyframe(&call, &sender); }
                else if let Ok(request) = invoke(&transformer, "generateKeyFrame", &[]) {
                    spawn_local(async move { if let Ok(promise) = request.dyn_into::<js_sys::Promise>() { let _ = JsFuture::from(promise).await; } });
                }
            }
            // The reason separates a muted track, whose frames are meant to stop here, from a
            // key or state failure, which is not.
            if dropped % 250 == 1 {
                timing(format!(
                    "SigilTiming call transform dropped={dropped} sealing={sealing} reason={reason}"
                ));
            }
            if video && !sealing { release(&writer, &frame).await?; }
            continue;
        }
        let Ok(Some(payload)) = converted else { continue };
        if video && !sealing {
            let camera = sigil_calls::av1::camera_payload(&payload).map_err(|_| fail("Invalid camera frame"))?;
            let (rotation, width, height) = (camera.rotation, camera.width, camera.height);
            if let Some(decoder) = &software { decoder.receive(payload.clone()); }
            if shape != Some((rotation, width, height)) || payload[1] != 0 {
                let message = crate::rtc::object(serde_json::json!({"video_shape":true,"call":call,"sender":sender,"rotation":rotation,"width":width,"height":height}))?;
                let worker: web_sys::DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
                worker.post_message(&message)?;
                shape = Some((rotation, width, height));
            }
            video_count += 1;
            video_gap = video_gap.max(arrived - video_last);
            video_last = arrived;
            video_open = video_open.max(converted_at - arrived);
            let at = js_sys::Date::now();
            video_ack = video_ack.max(at - converted_at);
            if at - video_since >= 5000.0 {
                timing(format!("SigilTiming video receive frames={video_count} gap_ms={video_gap:.0} open_ms={video_open:.0} write_ms={video_ack:.0} dropped={dropped}"));
                video_since = at;
                (video_count, video_gap, video_open, video_ack) = (0, 0.0, 0.0, 0.0);
            }
            release(&writer, &frame).await?;
            continue;
        }
        let buffer = js_sys::Uint8Array::from(payload.as_slice()).buffer();
        set(&frame, "data", &buffer.into())?;
        JsFuture::from(invoke(&writer, "write", &[frame])?.unchecked_into::<js_sys::Promise>())
            .await?;
    }
}

/// The forwarder ignores RTCP keyframe requests from browsers, which Chrome also sends for
/// tracks it never decodes itself; this receiver's own requests go over the page's channel.
/// Hands Chrome an empty AV1 temporal unit in place of a camera frame we decode ourselves.
/// Chrome frees a frame's packets only once its own pipeline receives the frame; withholding
/// frames filled its 2,048-packet buffer every few seconds, and each overflow discarded a frame.
async fn release(writer: &JsValue, frame: &JsValue) -> Result<(), JsValue> {
    let placeholder = js_sys::Uint8Array::from(&[0x12u8, 0x00][..]).buffer();
    set(frame, "data", &placeholder.into())?;
    JsFuture::from(invoke(writer, "write", &[frame.clone()])?.unchecked_into::<js_sys::Promise>()).await.map(|_| ())
}

pub(crate) fn request_keyframe(call: &str, sender: &str) {
    let worker: web_sys::DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
    if let Ok(message) = crate::rtc::object(serde_json::json!({"video_key_request":true,"call":call,"sender":sender})) {
        let _ = worker.post_message(&message);
    }
}

fn convert(frame: &JsValue, sealing: bool, call: &str, sender: &str) -> Result<Option<Vec<u8>>, JsValue> {
    let bytes = js_sys::Uint8Array::new(&get(frame, "data")?).to_vec();
    if bytes.is_empty() || bytes.len() > 1024 * 1024 {
        return Err(fail("Invalid encoded frame"));
    }
    if sealing {
        // The RTP timestamp counts 48 kHz samples; sealing binds microseconds.
        let stamp = get(frame, "timestamp")?.as_f64().unwrap_or(0.0).max(0.0) as u64;
        Ok(Some(crate::media_worker::seal_audio(call, &bytes, stamp * 1000 / 48)?))
    } else {
        Ok(crate::media_worker::open_audio(call, sender, &bytes)?.map(|frame| frame.to_vec()))
    }
}
