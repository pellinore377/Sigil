use crate::{call, fail, get, set};
use js_sys::{Array, Function, Reflect, Uint8Array};
use sigil_calls::{Assembly, Id, MediaKind, channel::Packet};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
};
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_futures::JsFuture;

struct Outgoing {
    timestamp: f64,
    keyframe: bool,
    bytes: zeroize::Zeroizing<Vec<u8>>,
    queued: web_time::Instant,
    reply: futures_channel::oneshot::Sender<Result<bool, JsValue>>,
}
struct Session {
    generation: u64,
    call: String,
    own: Id,
    members: Vec<Id>,
    pc: JsValue,
    channel: JsValue,
    sequence: [u64; 3],
    sending: [bool; 3],
    outgoing: [VecDeque<Outgoing>; 3],
    assembly: Assembly,
    incoming: VecDeque<Packet>,
    pending: usize,
    decoding: bool,
    frames: Function,
    _message: Closure<dyn FnMut(web_sys::MessageEvent)>,
    _track: Closure<dyn FnMut(JsValue)>,
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = set(&self.pc, "ontrack", &JsValue::NULL);
        stop_playback();
        let _ = set(&self.channel, "onmessage", &JsValue::NULL);
        let _ = invoke(&self.channel, "close", &[]);
        let _ = invoke(&self.pc, "close", &[]);
        self.assembly.clear();
    }
}
/// Retransmission deadline for media fragments, in milliseconds.
const CHANNEL_LIFETIME: u16 = 120;
thread_local! {static SESSION:RefCell<Option<Session>>=const {RefCell::new(None)};static GENERATION:Cell<u64>=const {Cell::new(0)};static MICROPHONE:RefCell<Option<JsValue>>=const {RefCell::new(None)};static MUTED:Cell<bool>=const {Cell::new(false)};static PLAYBACK:RefCell<Vec<JsValue>>=const {RefCell::new(Vec::new())};}
pub(crate) fn invoke(value: &JsValue, name: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
    let args = args.iter().collect::<Array>();
    get(value, name)?
        .dyn_into::<Function>()?
        .apply(value, &args)
}
pub(crate) fn object(value: serde_json::Value) -> Result<JsValue, JsValue> {
    js_sys::JSON::parse(&value.to_string())
}
pub(crate) fn construct(name: &str, options: &JsValue) -> Result<JsValue, JsValue> {
    let args = Array::new();
    args.push(options);
    Reflect::construct(
        &get(&js_sys::global(), name)?.dyn_into::<Function>()?,
        &args,
    )
}
async fn promise(value: JsValue) -> Result<JsValue, JsValue> {
    JsFuture::from(value.dyn_into::<js_sys::Promise>()?).await
}
async fn command(value: serde_json::Value, bytes: Uint8Array) -> Result<Uint8Array, JsValue> {
    call::call_command(value.to_string(), bytes).await
}
async fn control(value: serde_json::Value) -> Result<serde_json::Value, JsValue> {
    let bytes = command(value, Uint8Array::new_with_length(0)).await?;
    serde_json::from_slice(&bytes.to_vec()).map_err(|_| fail("Invalid call response"))
}
#[wasm_bindgen]
pub async fn browser_call_control(request: String) -> Result<String, JsValue> {
    if request.len() > 80 * 1024 {
        return Err(fail("Invalid call command"));
    }
    let value = serde_json::from_str(&request).map_err(|_| fail("Invalid call command"))?;
    Ok(control(value).await?.to_string())
}
async fn delay() -> Result<(), JsValue> {
    let (send, receive) = futures_channel::oneshot::channel();
    let callback = Closure::once(move || {
        let _ = send.send(());
    });
    web_sys::window()
        .ok_or_else(|| fail("Missing window"))?
        .set_timeout_with_callback_and_timeout_and_arguments_0(
            callback.as_ref().unchecked_ref(),
            50,
        )?;
    receive.await.map_err(|_| fail("Call interrupted"))
}
fn current(generation: u64) -> bool {
    GENERATION.with(|value| value.get() == generation)
}
#[wasm_bindgen]
pub fn browser_call_close() {
    GENERATION.with(|value| value.set(value.get().wrapping_add(1)));
    SESSION.with(|slot| slot.borrow_mut().take());
}
#[wasm_bindgen]
pub fn browser_call_transport_state() -> String {
    SESSION.with(|slot| {
        slot.borrow()
            .as_ref()
            .and_then(|s| get(&s.pc, "connectionState").ok()?.as_string())
            .unwrap_or_else(|| "closed".into())
    })
}
/// Runs one sender or receiver through the worker that holds the call keys.
fn attach_transform(target: &JsValue, options: serde_json::Value) -> Result<(), JsValue> {
    let Some(worker) = crate::host::worker() else {
        return Err(fail("Browser client stopped"));
    };
    let constructor: js_sys::Function = Reflect::get(&js_sys::global(), &"RTCRtpScriptTransform".into())?
        .dyn_into()
        .map_err(|_| fail("This browser cannot encrypt call media in a worker"))?;
    let transform = Reflect::construct(&constructor, &Array::of2(&worker.into(), &object(options)?))?;
    set(target, "transform", &transform)?;
    // A silently ignored assignment would send media in the clear, so refuse to continue.
    if get(target, "transform")?.is_undefined() || get(target, "transform")?.is_null() {
        return Err(fail("This browser did not accept the call media transform"));
    }
    Ok(())
}
/// The microphone, with the browser's own echo cancellation and noise suppression. Held open for
/// the whole call so a reconnect never prompts again.
async fn microphone() -> Option<JsValue> {
    if let Some(track) = MICROPHONE.with(|slot| slot.borrow().clone()) {
        return Some(track);
    }
    let devices = web_sys::window()?.navigator().media_devices().ok()?;
    let constraints = web_sys::MediaStreamConstraints::new();
    constraints.set_audio(
        &object(serde_json::json!({
            "echoCancellation": true, "noiseSuppression": true, "autoGainControl": true
        }))
        .ok()?,
    );
    constraints.set_video(&false.into());
    let stream = JsFuture::from(devices.get_user_media_with_constraints(&constraints).ok()?)
        .await
        .ok()?;
    let tracks = invoke(&stream, "getAudioTracks", &[]).ok()?;
    let track = Reflect::get(&tracks, &0.into())
        .ok()
        .filter(|t| !t.is_undefined())?;
    let _ = set(&track, "enabled", &(!MUTED.with(Cell::get)).into());
    MICROPHONE.with(|slot| *slot.borrow_mut() = Some(track.clone()));
    Some(track)
}
/// Opens the microphone before the call is placed, so permission is settled first.
#[wasm_bindgen]
pub async fn browser_call_microphone() -> Result<(), JsValue> {
    microphone()
        .await
        .map(|_| ())
        .ok_or_else(|| fail("Microphone unavailable"))
}
/// Muting stops the browser encoding anything at all, rather than sealing silence.
#[wasm_bindgen]
pub fn browser_call_mute(muted: bool) {
    MUTED.with(|value| value.set(muted));
    MICROPHONE.with(|slot| {
        if let Some(track) = slot.borrow().as_ref() {
            let _ = set(track, "enabled", &(!muted).into());
        }
    });
}
/// Releases the microphone when the call is over.
#[wasm_bindgen]
pub fn browser_call_release() {
    MUTED.with(|value| value.set(false));
    if let Some(track) = MICROPHONE.with(|slot| slot.borrow_mut().take()) {
        let _ = invoke(&track, "stop", &[]);
    }
}
/// Counts only: how much audio the browser's own pipeline has actually sent and received.
#[wasm_bindgen]
pub async fn browser_call_audio_stats() -> String {
    async fn read() -> Result<String, JsValue> {
        let pc = SESSION
            .with(|slot| slot.borrow().as_ref().map(|s| s.pc.clone()))
            .ok_or_else(|| fail("No call is active"))?;
        let report = promise(invoke(&pc, "getStats", &[])?).await?;
        let counts = std::rc::Rc::new(Cell::new([0.0f64; 3]));
        let sink = counts.clone();
        let visit = Closure::<dyn FnMut(JsValue)>::new(move |entry: JsValue| {
            if get(&entry, "kind").ok().and_then(|v| v.as_string()).as_deref() != Some("audio") {
                return;
            }
            let number = |name: &str| get(&entry, name).ok().and_then(|v| v.as_f64()).unwrap_or(0.0);
            let mut totals = sink.get();
            match get(&entry, "type").ok().and_then(|v| v.as_string()).as_deref() {
                Some("outbound-rtp") => totals[0] += number("packetsSent"),
                Some("inbound-rtp") => {
                    totals[1] += number("packetsReceived");
                    totals[2] += number("packetsLost");
                }
                _ => return,
            }
            sink.set(totals);
        });
        invoke(&report, "forEach", &[visit.as_ref().clone()])?;
        drop(visit);
        let totals = counts.get();
        Ok(format!(
            "sent={:.0} received={:.0} lost={:.0}",
            totals[0], totals[1], totals[2]
        ))
    }
    read().await.unwrap_or_else(|_| "unavailable".into())
}
/// Calls need a peer connection, a worker transform for the keys, and a capture device.
#[wasm_bindgen]
pub fn browser_call_supported() -> bool {
    let global = js_sys::global();
    let present = |name: &str| {
        Reflect::get(&global, &name.into())
            .map(|value| !value.is_undefined() && !value.is_null())
            .unwrap_or(false)
    };
    present("RTCPeerConnection")
        && present("RTCRtpScriptTransform")
        && web_sys::window()
            .map(|w| w.navigator().media_devices().is_ok())
            .unwrap_or(false)
}
/// Plays every remote audio track the browser hands us, natively. The answer comes from an SFU
/// and carries no stream identity, so each track is wrapped in a stream of its own rather than
/// taken from the event.
fn play_remote(pc: &JsValue) -> Result<Closure<dyn FnMut(JsValue)>, JsValue> {
    let handler = Closure::<dyn FnMut(JsValue)>::new(move |event: JsValue| {
        let attach = || -> Result<(), JsValue> {
            let track = get(&event, "track")?;
            if get(&track, "kind")?.as_string().as_deref() != Some("audio") {
                return Ok(());
            }
            let document = web_sys::window()
                .and_then(|w| w.document())
                .ok_or_else(|| fail("Missing document"))?;
            let element = document.create_element("audio")?;
            set(&element, "autoplay", &true.into())?;
            let stream = Reflect::construct(
                &get(&js_sys::global(), "MediaStream")?.dyn_into::<Function>()?,
                &Array::of1(&Array::of1(&track)),
            )?;
            set(&element, "srcObject", &stream)?;
            document
                .body()
                .ok_or_else(|| fail("Missing body"))?
                .append_child(&element)?;
            let _ = invoke(&element, "play", &[]);
            PLAYBACK.with(|slot| slot.borrow_mut().push(element.into()));
            Ok(())
        };
        if attach().is_err() {
            web_sys::console::log_1(&JsValue::from_str(
                "SigilTiming call playback attach failed",
            ));
        }
    });
    set(pc, "ontrack", handler.as_ref())?;
    Ok(handler)
}
/// Stops and removes every playback element the call created.
fn stop_playback() {
    PLAYBACK.with(|slot| {
        for element in slot.borrow_mut().drain(..) {
            let _ = invoke(&element, "pause", &[]);
            let _ = set(&element, "srcObject", &JsValue::NULL);
            let _ = invoke(&element, "remove", &[]);
        }
    });
}
#[wasm_bindgen]
/// The frame callback must copy or decode its bytes before returning.
pub async fn browser_call_connect(id: String, frames: Function) -> Result<(), JsValue> {
    call::id(&id)?;
    browser_call_close();
    let generation = GENERATION.with(Cell::get);
    // The page has already started this call's media so readiness overlaps gathering; stopping here would discard it.
    let info = control(serde_json::json!({"operation":"call_info","call":id})).await?;
    if !current(generation) {
        return Err(fail("Call interrupted"));
    }
    let own = call::id(
        info["own"]
            .as_str()
            .ok_or_else(|| fail("Missing participant"))?,
    )?;
    let roster: sigil_calls::SignedRoster =
        serde_json::from_value(info["roster"].clone()).map_err(|_| fail("Invalid roster"))?;
    let members = roster
        .roster
        .members
        .iter()
        .filter(|m| m.id != own)
        .map(|m| m.id)
        .collect::<Vec<_>>();
    let relay = &info["relay"];
    let servers = if relay.is_null() {
        serde_json::json!([])
    } else {
        serde_json::json!([{"urls":relay["urls"],"username":relay["username"],"credential":relay["credential"]}])
    };
    let pc = construct(
        "RTCPeerConnection",
        &object(serde_json::json!({"iceServers":servers,"bundlePolicy":"max-bundle"}))?,
    )?;
    let result = async {
        let channel = invoke(
            &pc,
            "createDataChannel",
            &[
                sigil_calls::channel::LABEL.into(),
                // Unordered, but a lost fragment discards its whole frame, so allow
                // retransmission inside a deadline well under the keyframe interval.
                object(serde_json::json!({"ordered":false,"maxPacketLifeTime":CHANNEL_LIFETIME}))?,
            ],
        )?;
        set(&channel, "binaryType", &"arraybuffer".into())?;
        let microphone = microphone().await;
        let track_handler = play_remote(&pc)?;
        let mut uploads = Vec::new();
        let mut downloads = Vec::new();
        for kind in [MediaKind::Audio, MediaKind::Camera, MediaKind::Screen] {
            let media = if kind == MediaKind::Audio {
                "audio"
            } else {
                "video"
            };
            let transceiver = invoke(
                &pc,
                "addTransceiver",
                &[
                    media.into(),
                    object(serde_json::json!({"direction":"sendonly"}))?,
                ],
            )?;
            // The browser captures and encodes audio itself; the worker seals each encoded frame.
            if kind == MediaKind::Audio {
                let sender = get(&transceiver, "sender")?;
                attach_transform(&sender, serde_json::json!({"operation":"seal"}))?;
                if let Some(track) = microphone.as_ref() {
                    let _ = JsFuture::from(
                        invoke(&sender, "replaceTrack", &[track.clone()])?
                            .unchecked_into::<js_sys::Promise>(),
                    )
                    .await;
                }
            }
            uploads.push(transceiver);
        }
        for member in &members {
            for kind in [MediaKind::Audio, MediaKind::Camera, MediaKind::Screen] {
                let media = if kind == MediaKind::Audio {
                    "audio"
                } else {
                    "video"
                };
                let transceiver = invoke(
                    &pc,
                    "addTransceiver",
                    &[
                        media.into(),
                        object(serde_json::json!({"direction":"recvonly"}))?,
                    ],
                )?;
                if kind == MediaKind::Audio {
                    attach_transform(
                        &get(&transceiver, "receiver")?,
                        serde_json::json!({"operation":"open","sender":call::hex(*member)}),
                    )?;
                }
                downloads.push((*member, kind, transceiver));
            }
        }
        let offer = promise(invoke(&pc, "createOffer", &[])?).await?;
        promise(invoke(&pc, "setLocalDescription", &[offer])?).await?;
        let gathering_from = js_sys::Date::now();
        let mut complete = false;
        for _ in 0..200 {
            if !current(generation) {
                return Err(fail("Call interrupted"));
            }
            if get(&pc, "iceGatheringState")?.as_string().as_deref() == Some("complete") {
                complete = true;
                break;
            }
            delay().await?;
        }
        if !complete {
            return Err(fail("Call connectivity discovery timed out"));
        }
        let sdp = get(&get(&pc, "localDescription")?, "sdp")?
            .as_string()
            .ok_or_else(|| fail("Missing call offer"))?;
        let mid = |transceiver: &JsValue| {
            get(transceiver, "mid")?
                .as_string()
                .ok_or_else(|| fail("Missing media identifier"))
        };
        let layout = sigil_calls::Layout {
            uploads: uploads
                .iter()
                .map(mid)
                .collect::<Result<Vec<_>, _>>()?
                .try_into()
                .map_err(|_| fail("Invalid uploads"))?,
            downloads: downloads
                .iter()
                .map(|(sender, kind, transceiver)| {
                    Ok(sigil_calls::Track {
                        sender: *sender,
                        kind: *kind,
                        mid: mid(transceiver)?,
                    })
                })
                .collect::<Result<Vec<_>, JsValue>>()?,
        };
        let gathered_at = js_sys::Date::now();
        let answer = control(
            serde_json::json!({"operation":"call_connect","call":id,"sdp":sdp,"layout":layout}),
        )
        .await?;
        if !current(generation) {
            return Err(fail("Call interrupted"));
        }
        // Durations only, and the candidate kinds the offer carried.
        let kinds: Vec<&str> = ["typ host", "typ srflx", "typ relay"].into_iter().filter(|k| sdp.contains(k)).collect();
        web_sys::console::log_1(&JsValue::from_str(&format!(
            "SigilTiming call transport gather={:.0}ms connect={:.0}ms candidates={}",
            gathered_at - gathering_from,
            js_sys::Date::now() - gathered_at,
            kinds.join("+")
        )));
        let message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event| {
            incoming(generation, event)
        });
        set(&channel, "onmessage", message.as_ref())?;
        SESSION.with(|slot| {
            *slot.borrow_mut() = Some(Session {
                generation,
                call: id.clone(),
                own,
                members,
                pc: pc.clone(),
                channel,
                sequence: [0; 3],
                sending: [false; 3],
                outgoing: std::array::from_fn(|_| VecDeque::new()),
                assembly: Assembly::default(),
                incoming: VecDeque::new(),
                pending: 0,
                decoding: false,
                frames,
                _message: message,
                _track: track_handler,
            })
        });
        promise(invoke(
            &pc,
            "setRemoteDescription",
            &[object(
                serde_json::json!({"type":"answer","sdp":answer["sdp"]}),
            )?],
        )?)
        .await?;
        if !current(generation) {
            return Err(fail("Call interrupted"));
        }
        Ok(())
    }
    .await;
    if result.is_err() {
        let _ = invoke(&pc, "close", &[]);
        if current(generation) {
            browser_call_close();
        }
    }
    result
}
fn kind(value: u8) -> Result<MediaKind, JsValue> {
    match value {
        0 => Ok(MediaKind::Audio),
        1 => Ok(MediaKind::Camera),
        2 => Ok(MediaKind::Screen),
        _ => Err(fail("Invalid media kind")),
    }
}
#[wasm_bindgen]
pub async fn browser_call_send(
    media: u8,
    timestamp: f64,
    keyframe: bool,
    bytes: Uint8Array,
) -> Result<bool, JsValue> {
    let kind = kind(media)?;
    if !timestamp.is_finite()
        || timestamp < 0.0
        || timestamp.fract() != 0.0
        || timestamp > 9_007_199_254_740_991.0
        || media == 0 && keyframe
        || bytes.length() == 0
        || bytes.length() > 1024 * 1024
    {
        return Err(fail("Invalid encoded frame"));
    }

    let (reply, receive) = futures_channel::oneshot::channel();
    let context = SESSION.with(|slot| {
        let mut slot = slot.borrow_mut();
        let session = slot.as_mut()?;
        if session.outgoing[media as usize].len() >= if media == 0 { 4 } else { 2 }
            || get(&session.channel, "readyState")
                .ok()?
                .as_string()
                .as_deref()
                != Some("open")
            || get(&session.channel, "bufferedAmount").ok()?.as_f64()? > 192.0 * 1024.0
        {
            return None;
        }
        session.outgoing[media as usize].push_back(Outgoing {
            timestamp,
            keyframe,
            bytes: zeroize::Zeroizing::new(bytes.to_vec()),
            queued: web_time::Instant::now(),
            reply,
        });
        let start = !session.sending[media as usize];
        session.sending[media as usize] = true;
        Some((session.generation, start))
    });
    let Some((generation, start)) = context else {
        return Ok(false);
    };
    if start {
        wasm_bindgen_futures::spawn_local(async move {
            send_queued(generation, kind).await;
        });
    }
    receive.await.map_err(|_| fail("Call changed"))?
}
async fn send_queued(generation: u64, kind: MediaKind) {
    let media = kind as usize;
    loop {
        let next = SESSION.with(|slot| {
            let mut slot = slot.borrow_mut();
            let s = slot.as_mut().filter(|s| s.generation == generation)?;
            if let Some(frame) = s.outgoing[media].pop_front() {
                Some((s.call.clone(), frame))
            } else {
                s.sending[media] = false;
                None
            }
        });
        let Some((id, frame)) = next else {
            return;
        };
        let result=async{
   if frame.queued.elapsed()>std::time::Duration::from_millis(80){return Ok(false);}
   let data=Uint8Array::from(frame.bytes.as_slice());
   let result=command(serde_json::json!({"operation":"call_seal","call":id,"kind":kind,"timestamp":frame.timestamp as u64,"keyframe":frame.keyframe}),data.clone()).await;
   data.fill(0,0,data.length());let encrypted=result?;
   if frame.queued.elapsed()>std::time::Duration::from_millis(80){return Ok(false);}
   let fragments=sigil_calls::packetize(kind,&encrypted.to_vec()).map_err(|_|fail("Invalid encrypted frame"))?;
   SESSION.with(|slot|{
    let mut slot=slot.borrow_mut();let session=slot.as_mut().filter(|s|s.generation==generation).ok_or_else(||fail("Call changed"))?;
    let total=fragments.iter().map(|p|p.len()+51).sum::<usize>();
    if get(&session.channel,"bufferedAmount")?.as_f64().unwrap_or(f64::INFINITY)+total as f64>256.0*1024.0{return Ok(false);}
    let count=fragments.len();
    for(index,payload)in fragments.into_iter().enumerate(){
     session.sequence[media]=session.sequence[media].checked_add(1).ok_or_else(||fail("Call sequence exhausted"))?;
     let packet=Packet{sender:session.own,kind,sequence:session.sequence[media],timestamp:(frame.timestamp*(if media==0{48000.0}else{90000.0})/1_000_000.0) as u64 as u32,marker:index+1==count,payload:payload.into()}.encode().map_err(|_|fail("Invalid encrypted packet"))?;
     invoke(&session.channel,"send",&[Uint8Array::from(packet.as_slice()).into()])?;
    }
    Ok(true)
   })
  }.await;
        let _ = frame.reply.send(result);
    }
}

fn incoming(generation: u64, event: web_sys::MessageEvent) {
    let Ok(buffer) = event.data().dyn_into::<js_sys::ArrayBuffer>() else {
        return;
    };
    if buffer.byte_length() > 1551 {
        return;
    }
    let Ok(mut packet) = Packet::decode(&Uint8Array::new(&buffer).to_vec()) else {
        return;
    };
    let start = SESSION.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(s) = slot.as_mut().filter(|s| s.generation == generation) else {
            return false;
        };
        if !s.members.contains(&packet.sender) {
            return false;
        }
        let Ok(Some(frame)) = s.assembly.push(
            packet.sender,
            packet.kind,
            &packet.payload,
            web_time::Instant::now(),
        ) else {
            return false;
        };
        if s.incoming.len() >= 32 || s.pending + frame.len() > 2 * 1024 * 1024 {
            return false;
        }
        s.pending += frame.len();
        packet.payload = frame.into();
        s.incoming.push_back(packet);
        if s.decoding {
            return false;
        }
        s.decoding = true;
        true
    });
    if start {
        wasm_bindgen_futures::spawn_local(async move {
            drain(generation).await;
        });
    }
}
async fn drain(generation: u64) {
    loop {
        let next = SESSION.with(|slot| {
            let mut slot = slot.borrow_mut();
            let s = slot.as_mut().filter(|s| s.generation == generation)?;
            if let Some(packet) = s.incoming.pop_front() {
                s.pending -= packet.payload.len();
                Some((s.call.clone(), packet))
            } else {
                s.decoding = false;
                None
            }
        });
        let Some((id, packet)) = next else {
            return;
        };
        let result=command(serde_json::json!({"operation":"call_open","call":id,"sender":call::hex(packet.sender),"kind":packet.kind}),Uint8Array::from(packet.payload.as_ref())).await;
        if let Ok(bytes) = result {
            let callback = SESSION.with(|slot| {
                slot.borrow()
                    .as_ref()
                    .filter(|s| s.generation == generation)
                    .map(|s| s.frames.clone())
            });
            if let Some(callback) = callback {
                let _ = callback.call2(&JsValue::NULL, &call::hex(packet.sender).into(), &bytes);
            }
            bytes.fill(0, 0, bytes.length());
        }
    }
}
