use super::*;
use jni::sys::jlong;
use sigil_client::calls::RtcCall;
use std::sync::{Arc, Mutex, OnceLock};
#[path = "call_lanes.rs"]
mod call_lanes;
use call_lanes::SendLanes;
struct NativeCall {
    state: Mutex<NativeMedia>,
    send_lanes: SendLanes,
}
struct NativeMedia {
    store: sigil_client::ClientStore,
    call: RtcCall,
}
type Handle = Arc<NativeCall>;
static ACTIVE: Mutex<Option<(i64, Option<Handle>)>> = Mutex::new(None);
/// The media handle of the last closed transport, kept for the rebuild that follows a roster change.
static PARKED: Mutex<Option<sigil_client::calls::Media>> = Mutex::new(None);
static RUNTIME: OnceLock<Option<tokio::runtime::Runtime>> = OnceLock::new();
struct Timing(&'static str, std::time::Instant);
impl Drop for Timing {
    fn drop(&mut self) {
        static SAMPLES: OnceLock<Mutex<std::collections::BTreeMap<&'static str, (std::time::Instant, u64, u128)>>> = OnceLock::new();
        let Ok(mut samples) = SAMPLES.get_or_init(|| Mutex::new(Default::default())).lock() else { return };
        let entry = samples.entry(self.0).or_insert((self.1, 0, 0));
        entry.1 += 1;
        entry.2 = entry.2.max(self.1.elapsed().as_millis());
        if entry.0.elapsed() >= std::time::Duration::from_secs(5) {
            if let (Ok(tag), Ok(line)) = (std::ffi::CString::new("SigilTiming"), std::ffi::CString::new(format!("call media {} count={} max_ms={}", self.0, entry.1, entry.2))) {
                unsafe { super::__android_log_write(4, tag.as_ptr(), line.as_ptr()); }
            }
            *entry = (std::time::Instant::now(), 0, 0);
        }
    }
}
fn runtime() -> Option<&'static tokio::runtime::Runtime> {
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .ok()
        })
        .as_ref()
}
fn handle(id: i64) -> Option<Handle> {
    ACTIVE
        .lock()
        .ok()?
        .as_ref()
        .filter(|(token, _)| *token == id)?
        .1
        .clone()
}
fn remove(id: i64) {
    let taken = match ACTIVE.lock() {
        Ok(mut active) if active.as_ref().is_some_and(|(token, _)| *token == id) => active.take(),
        _ => None,
    };
    if let Some((_, Some(handle))) = taken {
        if let Ok(handle) = Arc::try_unwrap(handle) {
            if let Ok(native) = handle.state.into_inner() {
                if let Ok(mut parked) = PARKED.lock() {
                    *parked = Some(native.call.into_media());
                }
            }
        }
    }
}
fn tracks(bits: jint) -> Option<sigil_calls::Tracks> {
    if !(0..8).contains(&bits) {
        return None;
    }
    Some(sigil_calls::Tracks {
        audio: bits & 1 != 0,
        camera: bits & 2 != 0,
        screen: bits & 4 != 0,
    })
}
fn kind(value: jint) -> Option<sigil_calls::MediaKind> {
    match value {
        0 => Some(sigil_calls::MediaKind::Audio),
        1 => Some(sigil_calls::MediaKind::Camera),
        2 => Some(sigil_calls::MediaKind::Screen),
        _ => None,
    }
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|v| v.as_secs())
        .unwrap_or(0)
}
struct Reservation(i64);
impl Drop for Reservation {
    fn drop(&mut self) {
        remove(self.0);
    }
}
#[no_mangle]
pub extern "system" fn Java_org_sigil_storage_NativeStorage_openCall(
    mut env: JNIEnv,
    _: JObject,
    directory: JString,
    key: JByteArray,
    call: JByteArray,
    enabled: jint,
) -> jlong {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Option<i64> {
        if env.get_array_length(&call).ok()? != 32 {
            return None;
        }
        let id = env.convert_byte_array(&call).ok()?.try_into().ok()?;
        let enabled = tracks(enabled)?;
        let mut token = [0; 8];
        getrandom::fill(&mut token).ok()?;
        let token = i64::from_be_bytes(token) & i64::MAX;
        if token == 0 {
            return None;
        }
        {
            let mut active = ACTIVE.lock().ok()?;
            if active.is_some() {
                return None;
            }
            *active = Some((token, None));
        }
        let reservation = Reservation(token);
        let mut store = open(&mut env, &directory, &key)?;
        let reuse = PARKED.lock().ok().and_then(|mut parked| parked.take()).filter(|media| media.call() == id);
        let value = match runtime()?.block_on(store.connect_rtc_call_with(id, enabled, reuse, now())) {
            Ok(value) => value,
            Err(sigil_client::Error::Network(sigil_client::network::Error::Status {
                retry_after_seconds: Some(delay),
                ..
            })) => return Some(-i64::try_from(delay.max(1)).ok()?),
            Err(_) => return None,
        };
        {
            let mut active = ACTIVE.lock().ok()?;
            let slot = active.as_mut().filter(|(id, _)| *id == token)?;
            slot.1 = Some(Arc::new(NativeCall {
                state: Mutex::new(NativeMedia { store: store.into_inner(), call: value }),
                send_lanes: SendLanes::default(),
            }));
        }
        std::mem::forget(reservation);
        Some(token)
    }))
    .ok()
    .flatten()
    .unwrap_or(0)
}
#[no_mangle]
pub extern "system" fn Java_org_sigil_storage_NativeStorage_closeCall(
    _: JNIEnv,
    _: JObject,
    token: jlong,
) {
    let _ = std::panic::catch_unwind(|| remove(token));
}
#[no_mangle]
pub extern "system" fn Java_org_sigil_storage_NativeStorage_callState(
    _: JNIEnv,
    _: JObject,
    token: jlong,
) -> jint {
    let _timing = Timing("state", std::time::Instant::now());
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Option<jint> {
        let handle = handle(token)?;
        let mut active = handle.state.lock().ok()?;
        let NativeMedia { store, call } = &mut *active;
        Some(match store.rtc_media_state(call, now()).ok()? {
            "connected" => 1,
            "disconnected" => 2,
            "reconnect" => 4,
            "securing" => 5,
            "failed" => 3,
            _ => 0,
        })
    }))
    .ok()
    .flatten()
    .unwrap_or(-1)
}
#[no_mangle]
pub extern "system" fn Java_org_sigil_storage_NativeStorage_callTracks(
    _: JNIEnv,
    _: JObject,
    token: jlong,
    enabled: jint,
) -> jboolean {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Option<()> {
        let handle = handle(token)?;
        let mut active = handle.state.lock().ok()?;
        let NativeMedia { store, call } = &mut *active;
        store.rtc_set_tracks(call, tracks(enabled)?, now()).ok()
    }));
    if result.ok().flatten().is_some() {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}
#[no_mangle]
pub extern "system" fn Java_org_sigil_storage_NativeStorage_sendCallFrame(
    env: JNIEnv,
    _: JObject,
    token: jlong,
    media: jint,
    timestamp: jlong,
    keyframe: jboolean,
    data: JByteArray,
) -> jboolean {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Option<()> {
        if timestamp < 0 || !(1..=1024 * 1024).contains(&env.get_array_length(&data).ok()?) {
            return None;
        }
        let bytes = Zeroizing::new(env.convert_byte_array(&data).ok()?);
        let media = kind(media)?;
        let current = handle(token)?;
        current.send_lanes.run(media as usize, || {
            let handle = handle(token)?;
            if !Arc::ptr_eq(&current, &handle) {
                return None;
            }
            let transmission = {
                let _timing = Timing("prepare", std::time::Instant::now());
                let mut active = handle.state.lock().ok()?;
                let NativeMedia { store, call } = &mut *active;
                store
                    .rtc_prepare_send(call, media, timestamp as u64, keyframe != 0, &bytes, now())
                    .ok()?
            };
            let _timing = Timing("wire", std::time::Instant::now());
            runtime()?.block_on(transmission.send()).ok()
        })?
    }));
    if result.ok().flatten().is_some() {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}
#[no_mangle]
pub extern "system" fn Java_org_sigil_storage_NativeStorage_receiveCallFrames(
    env: JNIEnv,
    _: JObject,
    token: jlong,
) -> jbyteArray {
    let _timing = Timing("receive", std::time::Instant::now());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> Option<Zeroizing<Vec<u8>>> {
            let handle = handle(token)?;
            let mut active = handle.state.lock().ok()?;
            let NativeMedia { store, call } = &mut *active;
            let frames = store.rtc_receive(call, now()).ok()?;
            let mut bytes = Zeroizing::new(Vec::with_capacity(
                frames.iter().map(|v| 46 + v.frame.data.len()).sum(),
            ));
            for value in frames {
                if value.frame.timestamp > i64::MAX as u64 {
                    continue;
                }
                bytes.extend_from_slice(&value.sender);
                bytes.push(value.frame.kind as u8);
                bytes.push(u8::from(value.frame.keyframe));
                bytes.extend_from_slice(&value.frame.timestamp.to_be_bytes());
                bytes.extend_from_slice(&(value.frame.data.len() as u32).to_be_bytes());
                bytes.extend_from_slice(&value.frame.data);
            }
            Some(bytes)
        },
    ));
    result
        .ok()
        .flatten()
        .and_then(|bytes| env.byte_array_from_slice(&bytes).ok())
        .map(|v| v.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
pub extern "system" fn Java_org_sigil_storage_NativeStorage_requestCallVideoKeyframe(
    env: JNIEnv, _: JObject, token: jlong, sender: JByteArray, media: jint,
) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Option<()> {
        if env.get_array_length(&sender).ok()? != 32 { return None; }
        let sender = env.convert_byte_array(&sender).ok()?.try_into().ok()?;
        let handle = handle(token)?;
        handle.state.lock().ok()?.call.request_video_keyframe(sender, kind(media)?);
        Some(())
    }));
}
