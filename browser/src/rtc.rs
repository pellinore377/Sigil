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
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = set(&self.channel, "onmessage", &JsValue::NULL);
        let _ = invoke(&self.channel, "close", &[]);
        let _ = invoke(&self.pc, "close", &[]);
        self.assembly.clear();
    }
}
thread_local! {static SESSION:RefCell<Option<Session>>=const {RefCell::new(None)};static GENERATION:Cell<u64>=const {Cell::new(0)};}
pub(crate) fn invoke(value: &JsValue, name: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
    let args = args.iter().collect::<Array>();
    get(value, name)?
        .dyn_into::<Function>()?
        .apply(value, &args)
}
pub(crate) fn object(value: serde_json::Value) -> Result<JsValue, JsValue> {
    js_sys::JSON::parse(&value.to_string())
}
fn construct(name: &str, options: &JsValue) -> Result<JsValue, JsValue> {
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
#[wasm_bindgen]
/// The frame callback must copy or decode its bytes before returning.
pub async fn browser_call_connect(id: String, frames: Function) -> Result<(), JsValue> {
    call::id(&id)?;
    browser_call_close();
    let generation = GENERATION.with(Cell::get);
    control(serde_json::json!({"operation":"call_stop"})).await?;
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
                object(serde_json::json!({"ordered":false,"maxRetransmits":0}))?,
            ],
        )?;
        set(&channel, "binaryType", &"arraybuffer".into())?;
        let mut uploads = Vec::new();
        let mut downloads = Vec::new();
        for kind in [MediaKind::Audio, MediaKind::Camera, MediaKind::Screen] {
            let media = if kind == MediaKind::Audio {
                "audio"
            } else {
                "video"
            };
            uploads.push(invoke(
                &pc,
                "addTransceiver",
                &[
                    media.into(),
                    object(serde_json::json!({"direction":"sendonly"}))?,
                ],
            )?);
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
                downloads.push((*member, kind, transceiver));
            }
        }
        let offer = promise(invoke(&pc, "createOffer", &[])?).await?;
        promise(invoke(&pc, "setLocalDescription", &[offer])?).await?;
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
        let answer = control(
            serde_json::json!({"operation":"call_connect","call":id,"sdp":sdp,"layout":layout}),
        )
        .await?;
        if !current(generation) {
            return Err(fail("Call interrupted"));
        }
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
