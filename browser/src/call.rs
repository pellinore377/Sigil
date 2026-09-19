use crate::{ERASING, STORE, fail, get, host, set};
use js_sys::Uint8Array;
use serde::Deserialize;
use sigil_calls::{Id, Layout, MediaKind, Tracks};
use std::cell::RefCell;
use wasm_bindgen::{JsCast, prelude::*};
use zeroize::Zeroizing;

struct Active {
    id: Id,
    roster: Id,
    media: sigil_client::calls::Media,
}
thread_local! { static ACTIVE:RefCell<Option<Active>>=const {RefCell::new(None)}; }
pub(crate) fn clear() {
    ACTIVE.with(|active| active.borrow_mut().take());
}
#[derive(Deserialize)]
#[expect(
    clippy::enum_variant_names,
    reason = "Binary commands share a call namespace"
)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Operation {
    CallInfo {
        call: String,
    },
    CallConnect {
        call: String,
        sdp: String,
        layout: Layout,
    },
    CallStart {
        call: String,
        tracks: Tracks,
    },
    CallTracks {
        call: String,
        tracks: Tracks,
    },
    CallRefresh {
        call: String,
    },
    CallSeal {
        call: String,
        kind: MediaKind,
        timestamp: u64,
        keyframe: bool,
    },
    CallOpen {
        call: String,
        sender: String,
        kind: MediaKind,
    },
    CallStop {
        call: Option<String>,
    },
}
pub(crate) fn id(value: &str) -> Result<Id, JsValue> {
    if value.len() != 64 {
        return Err(fail("Invalid call identifier"));
    }
    let mut bytes = [0; 32];
    for (i, pair) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let digit = |byte: u8| match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            _ => None,
        };
        bytes[i] = digit(pair[0])
            .zip(digit(pair[1]))
            .map(|(a, b)| a * 16 + b)
            .ok_or_else(|| fail("Invalid call identifier"))?;
    }
    if bytes == [0; 32] {
        return Err(fail("Invalid call identifier"));
    }
    Ok(bytes)
}
pub(crate) fn hex(value: Id) -> String {
    value.iter().map(|v| format!("{v:02x}")).collect()
}
fn json(value: serde_json::Value) -> Zeroizing<Vec<u8>> {
    Zeroizing::new(value.to_string().into_bytes())
}
fn execute(operation: Operation, bytes: Zeroizing<Vec<u8>>) -> Result<Zeroizing<Vec<u8>>, JsValue> {
    let now = (js_sys::Date::now() / 1000.0) as u64;
    STORE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let store = slot.as_mut().ok_or_else(|| fail("Browser is locked"))?;
        let error = |_| fail("Call state is unavailable or changed");
        match operation {
            Operation::CallInfo { call } if bytes.is_empty() => {
                let call = id(&call)?;
                let (roster, own) = store.call_transport_roster(call, now).map_err(error)?;
                let relay = store.call_relay_online(call, now).map_err(error)?;
                Ok(json(
                    serde_json::json!({"roster":roster,"own":hex(own),"relay":relay}),
                ))
            }
            Operation::CallConnect { call, sdp, layout }
                if bytes.is_empty() && sdp.len() <= 65536 =>
            {
                let proof = store
                    .prepare_call_connection(id(&call)?, sdp, layout, now)
                    .map_err(error)?;
                let answer = store.connect_call_online(&proof, now).map_err(error)?;
                Ok(json(serde_json::json!(answer)))
            }
            Operation::CallStart { call, tracks } if bytes.is_empty() => {
                let call = id(&call)?;
                if ACTIVE.with(|slot| {
                    slot.borrow()
                        .as_ref()
                        .is_some_and(|active| active.id != call)
                }) {
                    return Err(fail("Another call is active"));
                }
                let (roster, _) = store.call_transport_roster(call, now).map_err(error)?;
                let digest = roster
                    .roster
                    .digest()
                    .map_err(|_| fail("Invalid call roster"))?;
                // A rebuild for the same call keeps its media: same lease, same keys, nothing to re-declare.
                let kept = ACTIVE.with(|slot| {
                    let mut slot = slot.borrow_mut();
                    match slot.as_mut() {
                        Some(active) if active.id == call => match store.refresh_call_media(&mut active.media, now) {
                            Ok(_) | Err(sigil_client::Error::Unprepared) => {
                                active.roster = digest;
                                true
                            }
                            Err(_) => false,
                        },
                        _ => false,
                    }
                });
                if kept {
                    return Ok(json(serde_json::json!({"started":true,"kept":true})));
                }
                let media = store.start_call_media(call, tracks, now).map_err(error)?;
                ACTIVE.with(|slot| {
                    *slot.borrow_mut() = Some(Active {
                        id: call,
                        roster: digest,
                        media,
                    })
                });
                Ok(json(serde_json::json!({"started":true})))
            }
            Operation::CallStop { call } if bytes.is_empty() => {
                let call = call.as_deref().map(id).transpose()?;
                ACTIVE.with(|slot| {
                    let mut slot = slot.borrow_mut();
                    if call.is_none() || slot.as_ref().is_some_and(|active| Some(active.id) == call)
                    {
                        slot.take();
                    }
                });
                Ok(json(serde_json::json!({"stopped":true})))
            }
            operation => ACTIVE.with(|slot| {
                let mut slot = slot.borrow_mut();
                let active = slot.as_mut().ok_or_else(|| fail("No call is active"))?;
                let call = match &operation {
                    Operation::CallTracks { call, .. }
                    | Operation::CallRefresh { call }
                    | Operation::CallSeal { call, .. }
                    | Operation::CallOpen { call, .. } => id(call)?,
                    _ => return Err(fail("Invalid call payload")),
                };
                if active.id != call {
                    return Err(fail("Call changed"));
                }
                let (roster, _) = store.call_transport_roster(call, now).map_err(error)?;
                if roster
                    .roster
                    .digest()
                    .map_err(|_| fail("Invalid call roster"))?
                    != active.roster
                {
                    slot.take();
                    return Err(fail("Call membership changed; reconnect"));
                }
                match operation {
                    Operation::CallTracks { tracks, .. } if bytes.is_empty() => {
                        store
                            .set_call_tracks(&mut active.media, tracks, now)
                            .map_err(error)?;
                        Ok(json(serde_json::json!({"updated":true})))
                    }
                    Operation::CallRefresh { .. } if bytes.is_empty() => {
                        let receivers = store
                            .refresh_call_media(&mut active.media, now)
                            .map_err(error)?;
                        Ok(json(serde_json::json!({"receivers":receivers})))
                    }
                    Operation::CallSeal {
                        kind,
                        timestamp,
                        keyframe,
                        ..
                    } if !bytes.is_empty()
                        && bytes.len() <= 1024 * 1024
                        && timestamp <= i64::MAX as u64 =>
                    {
                        store
                            .seal_call_frame(
                                &mut active.media,
                                kind,
                                timestamp,
                                keyframe,
                                &bytes,
                                now,
                            )
                            .map(Zeroizing::new)
                            .map_err(error)
                    }
                    Operation::CallOpen { sender, kind, .. } if !bytes.is_empty() => {
                        let frame = store
                            .open_call_frame(&mut active.media, id(&sender)?, kind, &bytes, now)
                            .map_err(error)?;
                        let mut value = Zeroizing::new(Vec::with_capacity(10 + frame.data.len()));
                        value.push(frame.kind as u8);
                        value.push(u8::from(frame.keyframe));
                        value.extend_from_slice(&frame.timestamp.to_be_bytes());
                        value.extend_from_slice(&frame.data);
                        Ok(value)
                    }
                    _ => Err(fail("Invalid call payload")),
                }
            }),
        }
    })
}
/// Seal one encoded audio frame and return the single RTP payload that carries it. Audio frames
/// are well under a fragment, so the packetizer always yields exactly one.
pub(crate) fn seal_audio(bytes: &[u8], timestamp: u64) -> Result<Vec<u8>, JsValue> {
    let now = (js_sys::Date::now() / 1000.0) as u64;
    STORE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let store = slot.as_mut().ok_or_else(|| fail("Browser is locked"))?;
        ACTIVE.with(|active| -> Result<Vec<u8>, JsValue> {
            let mut active = active.borrow_mut();
            let active = active.as_mut().ok_or_else(|| fail("No call is active"))?;
            let sealed = store
                .seal_call_frame(&mut active.media, MediaKind::Audio, timestamp, false, bytes, now)
                .map_err(|_| fail("Call state is unavailable or changed"))?;
            let mut packets = sigil_calls::packetize(MediaKind::Audio, &sealed)
                .map_err(|_| fail("Invalid audio frame"))?;
            if packets.len() != 1 {
                return Err(fail("Audio frame does not fit one packet"));
            }
            Ok(packets.remove(0))
        })
    })
}
/// Open one received audio payload; None means the frame is not yet complete or was rejected.
pub(crate) fn open_audio(sender: &str, bytes: &[u8]) -> Result<Option<Zeroizing<Vec<u8>>>, JsValue> {
    let now = (js_sys::Date::now() / 1000.0) as u64;
    let sender = id(sender)?;
    STORE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let store = slot.as_mut().ok_or_else(|| fail("Browser is locked"))?;
        ACTIVE.with(|active| -> Result<Option<Zeroizing<Vec<u8>>>, JsValue> {
            let mut active = active.borrow_mut();
            let active = active.as_mut().ok_or_else(|| fail("No call is active"))?;
            Ok(store
                .open_call_packet(&mut active.media, sender, MediaKind::Audio, bytes, now)
                .map_err(|_| fail("Call state is unavailable or changed"))?
                .map(|frame| frame.data))
        })
    })
}
pub(crate) fn receive(event: &web_sys::MessageEvent) -> bool {
    let data = event.data();
    if get(&data, "binary").ok().and_then(|v| v.as_bool()) != Some(true) {
        return false;
    }
    let Some(raw) = get(&data, "request").ok().and_then(|v| v.as_string()) else {
        return false;
    };
    if !raw.contains("\"call_") {
        return false;
    }
    let Ok(reply_id) = get(&data, "id") else {
        return true;
    };
    if !reply_id
        .as_string()
        .is_some_and(|v| v.parse::<u64>().is_ok())
    {
        return true;
    }
    let result = (|| {
        let bytes = get(&data, "data")?.dyn_into::<Uint8Array>()?;
        if bytes.length() > 1024 * 1024 + 256
            || raw.len() > 80 * 1024
            || ERASING.with(std::cell::Cell::get)
        {
            return Err(fail("Invalid call payload"));
        }
        let operation = serde_json::from_str(&raw).map_err(|_| fail("Invalid call command"))?;
        let plain = Zeroizing::new(bytes.to_vec());
        bytes.fill(0, 0, bytes.length());
        execute(operation, plain)
    })();
    let reply = js_sys::Object::new();
    let _ = set(&reply, "binary", &true.into());
    let _ = set(&reply, "id", &reply_id);
    let _ = set(&reply, "ok", &result.is_ok().into());
    if let Err(error) = &result {
        // The message names the failed step; it carries no call content.
        let text = error.dyn_ref::<js_sys::Error>().map(|e| String::from(e.message())).or_else(|| error.as_string()).unwrap_or_default();
        let _ = set(&reply, "error", &text.into());
    }
    let output = result
        .map(|bytes| Uint8Array::from(bytes.as_slice()))
        .unwrap_or_else(|_| Uint8Array::new_with_length(0));
    let _ = set(&reply, "data", &output);
    let transfers = js_sys::Array::new();
    transfers.push(&output.buffer());
    let worker = js_sys::global().unchecked_into::<web_sys::DedicatedWorkerGlobalScope>();
    let _ = worker.post_message_with_transfer(&reply, &transfers);
    true
}
#[wasm_bindgen]
pub async fn call_command(request: String, bytes: Uint8Array) -> Result<Uint8Array, JsValue> {
    if request.len() > 80 * 1024
        || bytes.length() > 1024 * 1024 + 256
        || !request.contains("\"call_")
    {
        return Err(fail("Invalid call payload"));
    }
    host::rpc(request, Some(bytes)).await?.dyn_into()
}
