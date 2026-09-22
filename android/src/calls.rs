use super::*;
use jni::sys::jlong;
use sigil_client::calls::{GateCrypto, MediaGate, RtcCall, RtcReceive, RtcSend};
use std::sync::{
    atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering},
    Arc, Mutex, OnceLock, Weak,
};
#[path = "call_lanes.rs"]
mod call_lanes;
use call_lanes::SendLanes;
/// Storage and authority live on the control thread; frames only take the gate for crypto.
struct NativeCall {
    control: Mutex<Control>,
    gate: Mutex<MediaGate>,
    send: Mutex<RtcSend>,
    receive: Mutex<RtcReceive>,
    camera: Mutex<(sigil_calls::av1::Sequence, Option<[u8; 4]>)>,
    requests: Arc<AtomicU8>,
    state: AtomicI32,
    stopped: AtomicBool,
    send_lanes: SendLanes,
}
struct Control {
    store: sigil_client::ClientStore,
    call: RtcCall,
}
/// Authority outlives a publish by this much; a stalled owner stops media within it.
const AUTHORITY: std::time::Duration = std::time::Duration::from_secs(2);
const REFRESH: std::time::Duration = std::time::Duration::from_millis(250);
type Handle = Arc<NativeCall>;
static ACTIVE: Mutex<Option<(i64, Option<Handle>)>> = Mutex::new(None);
/// The media handle and sender of the last closed transport, kept for the rebuild after a roster change.
static PARKED: Mutex<Option<(sigil_client::calls::Media, MediaGate)>> = Mutex::new(None);
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
fn state_code(state: Result<&'static str, sigil_client::Error>) -> i32 {
    match state {
        Ok("connected") => 1,
        Ok("disconnected") => 2,
        Ok("reconnect") => 4,
        Ok("securing") => 5,
        Ok("failed") => 3,
        Ok(_) => 0,
        Err(_) => -1,
    }
}
impl NativeCall {
    fn publish(&self) {
        let _timing = Timing("control", std::time::Instant::now());
        let Ok(mut control) = self.control.lock() else { return self.state.store(-1, Ordering::Relaxed) };
        let Control { store, call } = &mut *control;
        self.state.store(state_code(store.rtc_publish(call, &self.gate, AUTHORITY, now())), Ordering::Relaxed);
    }
}
/// Refreshes authority off the frame path until the call closes or its handle is dropped.
fn control(call: Weak<NativeCall>) {
    loop {
        let Some(call) = call.upgrade() else { return };
        if call.stopped.load(Ordering::Relaxed) { return; }
        call.publish();
        drop(call);
        std::thread::sleep(REFRESH);
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
        handle.stopped.store(true, Ordering::Relaxed);
        if let Ok(mut gate) = handle.gate.lock() { gate.revoke(); }
        // The control thread holds only a weak reference, so this is the last owner once in-flight frames finish.
        for _ in 0..200 {
            if Arc::strong_count(&handle) == 1 { break; }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if let Ok(native) = Arc::try_unwrap(handle) {
            if let (Ok(control), Ok(gate)) = (native.control.into_inner(), native.gate.into_inner()) {
                if let Ok(mut parked) = PARKED.lock() {
                    *parked = Some((control.call.into_media(), gate));
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
        let parked = PARKED.lock().ok().and_then(|mut parked| parked.take()).filter(|(media, _)| media.call() == id);
        let (reuse, gate) = match parked {
            Some((media, gate)) => (Some(media), gate),
            None => (None, MediaGate::default()),
        };
        let mut value = match runtime()?.block_on(store.connect_rtc_call_with(id, enabled, reuse, now())) {
            Ok(value) => value,
            Err(sigil_client::Error::Network(sigil_client::network::Error::Status {
                retry_after_seconds: Some(delay),
                ..
            })) => return Some(-i64::try_from(delay.max(1)).ok()?),
            Err(_) => return None,
        };
        let (send, receive) = value.detach()?;
        let native = Arc::new(NativeCall {
            requests: value.video_requests(),
            control: Mutex::new(Control { store: store.into_inner(), call: value }),
            gate: Mutex::new(gate),
            send: Mutex::new(send),
            receive: Mutex::new(receive),
            camera: Mutex::new(Default::default()),
            state: AtomicI32::new(0),
            stopped: AtomicBool::new(false),
            send_lanes: SendLanes::default(),
        });
        native.publish();
        {
            let mut active = ACTIVE.lock().ok()?;
            let slot = active.as_mut().filter(|(id, _)| *id == token)?;
            slot.1 = Some(native.clone());
        }
        let weak = Arc::downgrade(&native);
        std::thread::Builder::new().name("Sigil call control".into()).spawn(move || control(weak)).ok()?;
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
    handle(token).map(|call| call.state.load(Ordering::Relaxed)).unwrap_or(-1)
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
        {
            let mut control = handle.control.lock().ok()?;
            let Control { store, call } = &mut *control;
            store.rtc_set_tracks(call, tracks(enabled)?, now()).ok()?;
        }
        handle.publish();
        Some(())
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
        // The wire flag promises a decoder can start here; encoders also flag intra-only frames.
        let flagged = keyframe != 0;
        let header = bytes.first().and_then(|codec| sigil_calls::av1::camera_header(*codec));
        let keyframe = flagged && (media == sigil_calls::MediaKind::Audio
            || header.is_some_and(|h| bytes.len() > h && sigil_calls::av1::random_access(&bytes[h..])));
        if media == sigil_calls::MediaKind::Camera && flagged {
            drop(Timing(if keyframe { "send_key" } else { "send_sync_not_key" }, std::time::Instant::now()));
        }
        let current = handle(token)?;
        current.send_lanes.run(media as usize, || {
            let transmission = {
                let _timing = Timing("prepare", std::time::Instant::now());
                let normalized;
                let encoded = if media == sigil_calls::MediaKind::Camera {
                    let header = header.filter(|h| bytes.len() > *h)?;
                    let size: [u8; 4] = bytes[3..7].try_into().ok()?;
                    let mut camera = current.camera.lock().ok()?;
                    let (sequence, camera_size) = &mut *camera;
                    if *camera_size != Some(size) { sequence.clear(); *camera_size = Some(size); }
                    let frame = sequence.frame(&bytes[header..], keyframe).map_err(|_| drop(Timing("send_refused_config", std::time::Instant::now()))).ok()?;
                    match frame {
                        std::borrow::Cow::Borrowed(_) => &bytes[..],
                        std::borrow::Cow::Owned(frame) => {
                            let frame = Zeroizing::new(frame);
                            normalized = Zeroizing::new([&bytes[..header], &frame[..]].concat());
                            &normalized[..]
                        }
                    }
                } else { &bytes[..] };
                let mut crypto = GateCrypto { gate: &current.gate, now: now() };
                current.send.lock().ok()?
                    .prepare(&mut crypto, media, timestamp as u64, keyframe, encoded)
                    .map_err(|_| drop(Timing("send_refused_authority", std::time::Instant::now()))).ok()?
            };
            let _timing = Timing("wire", std::time::Instant::now());
            runtime()?.block_on(transmission.send()).map_err(|_| drop(Timing("send_refused_wire", std::time::Instant::now()))).ok()
        })?
    }));
    if result.ok().flatten().is_some() {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}
/// Blocks up to `wait_ms` for media, so the caller needs no polling schedule.
#[no_mangle]
pub extern "system" fn Java_org_sigil_storage_NativeStorage_receiveCallFrames(
    env: JNIEnv,
    _: JObject,
    token: jlong,
    wait_ms: jint,
) -> jbyteArray {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> Option<Zeroizing<Vec<u8>>> {
            let handle = handle(token)?;
            let mut receive = handle.receive.lock().ok()?;
            if !receive.wait(std::time::Duration::from_millis(wait_ms.clamp(0, 1000) as u64)) {
                return Some(Zeroizing::new(Vec::new()));
            }
            let _timing = Timing("receive", std::time::Instant::now());
            let frames = receive.receive(&mut GateCrypto { gate: &handle.gate, now: now() }).ok()?;
            drop(receive);
            let mut bytes = Zeroizing::new(Vec::with_capacity(
                frames.iter().map(|v| 46 + v.frame.data.len()).sum(),
            ));
            for value in frames {
                if value.frame.timestamp > i64::MAX as u64 {
                    continue;
                }
                bytes.extend_from_slice(&value.sender);
                bytes.push(value.frame.kind as u8);
                let data = &value.frame.data;
                let random = value.frame.kind == sigil_calls::MediaKind::Audio
                    || data.first().and_then(|c| sigil_calls::av1::camera_header(*c))
                        .is_some_and(|h| data.len() > h && sigil_calls::av1::random_access(&data[h..]));
                bytes.push(u8::from(value.frame.keyframe && random));
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
        handle.receive.lock().ok()?.request_video_keyframe(sender, kind(media)?);
        Some(())
    }));
}

#[no_mangle]
pub extern "system" fn Java_org_sigil_storage_NativeStorage_takeCallVideoRequests(
    _: JNIEnv, _: JObject, token: jlong,
) -> jint {
    handle(token).map(|call| i32::from(call.requests.swap(0, Ordering::Relaxed))).unwrap_or(0)
}
