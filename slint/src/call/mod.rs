//! Calls, on the app's side: the session manager that turns the engine's
//! signalling (`call.start/join/poll/answer/leave/key`, the `call.state`
//! events) and the peer thread's events into the call the pages show, in
//! the shape CallPage.qml read from the QML service.
//!
//! The engine owns the conversation (who is in it, the epoch key); the unit
//! on the server forwards frames it cannot read; this module owns the
//! microphone, the speaker, the codec, the cipher and the state.

pub mod audio;
pub mod crypt;
pub mod peer;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use slint::{ModelRc, VecModel};

use crate::bridge::{on_ui, UiState};
use crate::AppWindow;
use peer::{PeerCmd, PeerEvent, PeerHandle};

/// A participant we have heard from (a `hello` on the channel, or frames).
pub struct Participant {
    pub peer: String,
    pub user: String,
    pub name: String,
    pub muted: bool,
    pub level: f32,
    pub speaking: bool,
    last_voice: Instant,
}

pub struct Session {
    pub room_id: String,
    pub call_id: String,
    pub peer_id: String,
    pub started_by_me: bool,
    /// joining | connected | leaving
    pub state: &'static str,
    handle: PeerHandle,
    pub participants: BTreeMap<String, Participant>,
    pub muted: bool,
    pub local_level: f32,
    pub local_speaking: bool,
    last_local_voice: Instant,
    pub started: Instant,
    pub connected_at: Option<Instant>,
    pub floaters: Vec<(i32, String, String, Instant)>,
    floater_seq: i32,
    pub error: String,
    asked_keys: HashSet<u8>,
    hello_sent: Instant,
}

pub struct Incoming {
    pub room_id: String,
    pub call_id: String,
    pub sender: String,
}

#[derive(Default)]
pub struct Calls {
    pub session: Option<Session>,
    pub incoming: Option<Incoming>,
    /// room id → call id, for every call announced and not yet ended.
    pub active: HashMap<String, String>,
    pub devices: audio::DeviceList,
    pub selected_mic: String,
    pub selected_speaker: String,
    ticking: bool,
}

const SPEAK_LEVEL: f32 = 0.01;
const SPEAK_HANGOVER: Duration = Duration::from_millis(400);
const FLOATER_LIFE: Duration = Duration::from_millis(3200);

fn short(user: &str) -> String {
    crate::rows::localpart(user)
}

// ---------------------------------------------------------------- driving

/// Start a call in a conversation: announce it, then join it.
pub fn start(ui: &mut UiState, win: &AppWindow, room_id: &str) {
    if ui.calls.session.is_some() {
        return;
    }
    let rid = room_id.to_string();
    let rid2 = rid.clone();
    ui.req
        .call("call.start", json!({"roomId": rid}), move |reply| {
            on_ui(move |ui, win| match reply {
                crate::bridge::EngineReply::Ok(v) => {
                    let call_id = v["callId"].as_str().unwrap_or("").to_string();
                    ui.calls.active.insert(rid2.clone(), call_id.clone());
                    join(ui, win, &rid2, &call_id, true);
                }
                crate::bridge::EngineReply::Err(e) => {
                    win.set_call_error(format!("{}", e.message).into());
                    tracing::warn!("call.start: {} {}", e.code, e.message);
                }
            });
        });
    let _ = win;
}

/// Join a call that exists: the peer thread starts, its offer goes to the
/// unit, and the rest follows from events.
pub fn join(ui: &mut UiState, win: &AppWindow, room_id: &str, call_id: &str, started_by_me: bool) {
    if ui.calls.session.is_some() {
        return;
    }
    ui.calls.incoming = None;
    let handle = peer::spawn(Box::new(|ev| {
        on_ui(move |ui, win| on_peer_event(ui, win, ev))
    }));
    ui.calls.session = Some(Session {
        room_id: room_id.to_string(),
        call_id: call_id.to_string(),
        peer_id: String::new(),
        started_by_me,
        state: "joining",
        handle,
        participants: BTreeMap::new(),
        muted: false,
        local_level: 0.0,
        local_speaking: false,
        last_local_voice: Instant::now() - SPEAK_HANGOVER,
        started: Instant::now(),
        connected_at: None,
        floaters: Vec::new(),
        floater_seq: 0,
        error: String::new(),
        asked_keys: HashSet::new(),
        hello_sent: Instant::now() - Duration::from_secs(10),
    });
    ui.calls.devices = audio::devices();
    win.set_call_page_open(true);
    apply(ui, win);
    tick(ui, win);
}

/// Hang up: leave the unit, and end the call for everyone when we started it.
pub fn hangup(ui: &mut UiState, win: &AppWindow) {
    let Some(mut s) = ui.calls.session.take() else {
        return;
    };
    s.state = "leaving";
    let _ = s.handle.cmd.send(PeerCmd::Stop);
    if !s.peer_id.is_empty() {
        ui.req.fire(
            "call.leave",
            json!({"roomId": s.room_id, "callId": s.call_id, "peer": s.peer_id}),
        );
    }
    if s.started_by_me {
        ui.req.fire(
            "call.end",
            json!({"roomId": s.room_id, "callId": s.call_id}),
        );
        ui.calls.active.remove(&s.room_id);
    }
    win.set_call_page_open(false);
    apply(ui, win);
}

/// The other side ended the call, or the connection went.
fn end_locally(ui: &mut UiState, win: &AppWindow, why: &str) {
    if let Some(s) = ui.calls.session.take() {
        let _ = s.handle.cmd.send(PeerCmd::Stop);
        if !s.peer_id.is_empty() {
            ui.req.fire(
                "call.leave",
                json!({"roomId": s.room_id, "callId": s.call_id, "peer": s.peer_id}),
            );
        }
    }
    win.set_call_page_open(false);
    win.set_call_error(why.into());
    apply(ui, win);
}

pub fn decline(ui: &mut UiState, win: &AppWindow) {
    ui.calls.incoming = None;
    apply(ui, win);
}

pub fn set_mic(ui: &mut UiState, win: &AppWindow, muted: bool) {
    if let Some(s) = ui.calls.session.as_mut() {
        s.muted = muted;
        let _ = s.handle.cmd.send(PeerCmd::SetMuted(muted));
        let _ = s.handle.cmd.send(PeerCmd::Send(
            json!({"t": "mute", "muted": muted}).to_string(),
        ));
    }
    apply(ui, win);
}

pub fn react(ui: &mut UiState, win: &AppWindow, emoji: &str) {
    let me = ui.my_user.clone();
    if let Some(s) = ui.calls.session.as_mut() {
        let _ = s.handle.cmd.send(PeerCmd::Send(
            json!({"t": "react", "emoji": emoji}).to_string(),
        ));
        push_floater(s, emoji, &short(&me));
    }
    apply(ui, win);
}

pub fn select_device(ui: &mut UiState, win: &AppWindow, kind: &str, id: &str) {
    if kind == "mic" {
        ui.calls.selected_mic = id.to_string();
    } else {
        ui.calls.selected_speaker = id.to_string();
    }
    if let Some(s) = ui.calls.session.as_ref() {
        let _ = s.handle.cmd.send(PeerCmd::SetDevice {
            kind: kind.to_string(),
            id: id.to_string(),
        });
    }
    apply(ui, win);
}

fn push_floater(s: &mut Session, emoji: &str, who: &str) {
    s.floater_seq += 1;
    s.floaters.push((
        s.floater_seq,
        emoji.to_string(),
        who.to_string(),
        Instant::now(),
    ));
}

// ---------------------------------------------------------------- the engine

/// `call.state` from the engine: a call announced or ended in a conversation.
pub fn on_engine_call_state(ui: &mut UiState, win: &AppWindow, v: &Value) {
    let room = v["roomId"].as_str().unwrap_or("").to_string();
    let call_id = v["callId"].as_str().unwrap_or("").to_string();
    let sender = v["sender"].as_str().unwrap_or("").to_string();
    match v["state"].as_str().unwrap_or("") {
        "started" => {
            ui.calls.active.insert(room.clone(), call_id.clone());
            if sender != ui.my_user && ui.calls.session.is_none() {
                ui.calls.incoming = Some(Incoming {
                    room_id: room,
                    call_id,
                    sender,
                });
            }
        }
        "ended" => {
            ui.calls.active.remove(&room);
            if ui
                .calls
                .incoming
                .as_ref()
                .map(|i| i.call_id == call_id)
                .unwrap_or(false)
            {
                ui.calls.incoming = None;
            }
            let mine = ui
                .calls
                .session
                .as_ref()
                .map(|s| s.call_id == call_id)
                .unwrap_or(false);
            if mine && sender != ui.my_user {
                end_locally(ui, win, "Call ended");
                return;
            }
        }
        _ => {}
    }
    apply(ui, win);
}

fn ask_key(ui: &mut UiState, kid: Option<u8>) {
    let Some(s) = ui.calls.session.as_mut() else {
        return;
    };
    if let Some(k) = kid {
        if !s.asked_keys.insert(k) {
            return;
        }
    }
    let room = s.room_id.clone();
    ui.req
        .call("call.key", json!({"roomId": room}), move |reply| {
            on_ui(move |ui, _win| {
                let crate::bridge::EngineReply::Ok(v) = reply else {
                    return;
                };
                let Some(s) = ui.calls.session.as_mut() else {
                    return;
                };
                let kid = (v["epoch"].as_u64().unwrap_or(0) & 0xff) as u8;
                if let Ok(bytes) = hex::decode(v["key"].as_str().unwrap_or("")) {
                    if let Ok(key) = <[u8; 32]>::try_from(bytes) {
                        let _ = s.handle.cmd.send(PeerCmd::AddKey(kid, key));
                    }
                }
            });
        });
}

fn send_hello(ui: &mut UiState) {
    let me = ui.my_user.clone();
    let Some(s) = ui.calls.session.as_mut() else {
        return;
    };
    s.hello_sent = Instant::now();
    let _ = s.handle.cmd.send(PeerCmd::Send(
        json!({"t": "hello", "user": me, "name": short(&me), "muted": s.muted}).to_string(),
    ));
}

/// Ask the unit for renegotiation offers and the head count, every few
/// seconds while in the call.
fn poll(ui: &mut UiState) {
    let Some(s) = ui.calls.session.as_ref() else {
        return;
    };
    if s.peer_id.is_empty() {
        return;
    }
    let (room, call_id, peer) = (s.room_id.clone(), s.call_id.clone(), s.peer_id.clone());
    let req = ui.req.clone();
    let req2 = req.clone();
    req.call(
        "call.poll",
        json!({"roomId": room, "callId": call_id, "peer": peer}),
        move |reply| {
            on_ui(move |ui, _win| {
                let Some(s) = ui.calls.session.as_ref() else { return };
                if let crate::bridge::EngineReply::Ok(v) = reply {
                    if let Some(offer) = v["offer"].as_str().filter(|o| !o.is_empty()) {
                        tracing::debug!("call.poll: an offer of {} bytes", offer.len());
                        let (room, call_id, peer) = (s.room_id.clone(), s.call_id.clone(), s.peer_id.clone());
                        let req3 = req2.clone();
                        let _ = s.handle.cmd.send(PeerCmd::TakeOffer(
                            offer.to_string(),
                            Box::new(move |answer| {
                                tracing::debug!("call.answer: {} bytes", answer.len());
                                req3.fire(
                                    "call.answer",
                                    json!({"roomId": room, "callId": call_id, "peer": peer, "answer": answer}),
                                );
                            }),
                        ));
                    }
                }
                let req4 = req2.clone();
                crate::actions::after_pub(&req4, 2000, |ui, _win| poll(ui));
            });
        },
    );
}

// ---------------------------------------------------------------- the peer

fn on_peer_event(ui: &mut UiState, win: &AppWindow, ev: PeerEvent) {
    let Some(s) = ui.calls.session.as_mut() else {
        return;
    };
    match ev {
        PeerEvent::Offer(offer) => {
            let (room, call_id) = (s.room_id.clone(), s.call_id.clone());
            ui.req.call(
                "call.join",
                json!({"roomId": room, "callId": call_id, "offer": offer}),
                move |reply| {
                    on_ui(move |ui, win| {
                        match reply {
                            crate::bridge::EngineReply::Ok(v) => {
                                let answer = v["answer"].as_str().unwrap_or("").to_string();
                                let peer = v["peer"].as_str().unwrap_or("").to_string();
                                let Some(s) = ui.calls.session.as_mut() else {
                                    return;
                                };
                                s.peer_id = peer.clone();
                                if let Some(bytes) = hex::decode(&peer)
                                    .ok()
                                    .and_then(|b| <[u8; 16]>::try_from(b).ok())
                                {
                                    let _ = s.handle.cmd.send(PeerCmd::TakeAnswer {
                                        answer,
                                        peer: bytes,
                                    });
                                }
                                ask_key(ui, None);
                                poll(ui);
                            }
                            crate::bridge::EngineReply::Err(e) => {
                                end_locally(ui, win, &format!("Could not join: {}", e.message));
                            }
                        }
                        apply(ui, win);
                    });
                },
            );
        }
        PeerEvent::Connected => {
            s.state = "connected";
            s.connected_at = Some(Instant::now());
        }
        PeerEvent::ChannelOpen => send_hello(ui),
        PeerEvent::Message { from, text } => {
            let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            match v["t"].as_str().unwrap_or("") {
                "hello" => {
                    let user = v["user"].as_str().unwrap_or("").to_string();
                    // frames may have arrived first; a name is what says we
                    // have heard from them
                    let known = s
                        .participants
                        .get(&from)
                        .map(|p| !p.user.is_empty())
                        .unwrap_or(false);
                    let p = s
                        .participants
                        .entry(from.clone())
                        .or_insert_with(|| Participant {
                            peer: from.clone(),
                            user: user.clone(),
                            name: v["name"].as_str().unwrap_or("").to_string(),
                            muted: false,
                            level: 0.0,
                            speaking: false,
                            last_voice: Instant::now() - SPEAK_HANGOVER,
                        });
                    p.user = user;
                    p.name = v["name"].as_str().unwrap_or("").to_string();
                    p.muted = v["muted"].as_bool().unwrap_or(false);
                    // a hello is answered with ours, at most every so often:
                    // a newcomer needs it, and one lost on the way gets asked
                    // again by the tick until everyone has a name
                    let _ = known;
                    if s.hello_sent.elapsed() > Duration::from_millis(1500) {
                        send_hello(ui);
                    }
                }
                "mute" => {
                    if let Some(p) = s.participants.get_mut(&from) {
                        p.muted = v["muted"].as_bool().unwrap_or(false);
                    }
                }
                "react" => {
                    let who = s
                        .participants
                        .get(&from)
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| "someone".into());
                    let emoji = v["emoji"].as_str().unwrap_or("").to_string();
                    if !emoji.is_empty() {
                        push_floater(s, &emoji, &who);
                    }
                }
                _ => {}
            }
        }
        PeerEvent::Levels { local, remotes } => {
            let now = Instant::now();
            s.local_level = local;
            if local > SPEAK_LEVEL && !s.muted {
                s.last_local_voice = now;
            }
            s.local_speaking = now.duration_since(s.last_local_voice) < SPEAK_HANGOVER;
            for (origin, level) in remotes {
                let p = s
                    .participants
                    .entry(origin.clone())
                    .or_insert_with(|| Participant {
                        peer: origin.clone(),
                        user: String::new(),
                        name: String::new(),
                        muted: false,
                        level: 0.0,
                        speaking: false,
                        last_voice: now - SPEAK_HANGOVER,
                    });
                p.level = level;
                if level > SPEAK_LEVEL {
                    p.last_voice = now;
                }
            }
            for p in s.participants.values_mut() {
                p.speaking = now.duration_since(p.last_voice) < SPEAK_HANGOVER;
            }
        }
        PeerEvent::NeedKey(kid) => ask_key(ui, Some(kid)),
        PeerEvent::Disconnected(why) => {
            if s.state != "leaving" {
                end_locally(ui, win, &why);
                return;
            }
        }
    }
    apply(ui, win);
}

/// Once a second while a call is up: the clock, floaters ageing out.
fn tick(ui: &mut UiState, win: &AppWindow) {
    if ui.calls.session.is_none() {
        ui.calls.ticking = false;
        return;
    }
    let mut nameless = false;
    if let Some(s) = ui.calls.session.as_mut() {
        let now = Instant::now();
        s.floaters
            .retain(|(_, _, _, at)| now.duration_since(*at) < FLOATER_LIFE);
        nameless = s.participants.values().any(|p| p.user.is_empty())
            && s.hello_sent.elapsed() > Duration::from_secs(2);
    }
    if nameless {
        send_hello(ui);
    }
    apply(ui, win);
    if !ui.calls.ticking {
        ui.calls.ticking = true;
    }
    let req = ui.req.clone();
    crate::actions::after_pub(&req, 1000, |ui, win| {
        ui.calls.ticking = false;
        tick(ui, win);
    });
}

// ---------------------------------------------------------------- the view

fn mmss(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// The call as the pages show it, in the QML shape: `call.state`,
/// participants, the local side, the incoming banner.
pub fn apply(ui: &mut UiState, win: &AppWindow) {
    let me = ui.my_user.clone();
    let open_room = crate::actions::room_of_key(&ui.open_room);
    let calls = &ui.calls;
    let (state, in_call) = match calls.session.as_ref() {
        Some(s) => (s.state, true),
        None => ("idle", false),
    };
    win.set_call_state(state.into());
    win.set_in_call(in_call);
    win.set_call_here(calls.active.contains_key(&open_room));
    // the banner
    match calls.incoming.as_ref() {
        Some(inc) if !in_call => {
            win.set_call_incoming(true);
            win.set_call_incoming_name(short(&inc.sender).into());
            win.set_call_incoming_tint(crate::rows::tint_for(&inc.sender));
            let name = ui
                .rooms_json
                .iter()
                .find(|r| r["id"].as_str() == Some(inc.room_id.as_str()))
                .and_then(|r| r["name"].as_str())
                .unwrap_or("")
                .to_string();
            win.set_call_room_name(name.into());
        }
        _ => win.set_call_incoming(false),
    }
    let Some(s) = calls.session.as_ref() else {
        win.set_call_tiles(ModelRc::new(VecModel::from(
            Vec::<crate::CallParticipant>::new(),
        )));
        win.set_call_floaters(ModelRc::new(VecModel::from(Vec::<crate::Floater>::new())));
        win.set_call_status("".into());
        return;
    };
    let mut tiles: Vec<crate::CallParticipant> = s
        .participants
        .values()
        .map(|p| {
            let name = if p.name.is_empty() {
                if p.user.is_empty() {
                    "Someone".to_string()
                } else {
                    short(&p.user)
                }
            } else {
                p.name.clone()
            };
            crate::CallParticipant {
                identity: p.user.clone().into(),
                initials: crate::rows::initials(&name).into(),
                display_name: name.into(),
                tint: crate::rows::tint_for(if p.user.is_empty() { &p.peer } else { &p.user }),
                avatar: Default::default(),
                speaking: p.speaking,
                mic_muted: p.muted,
                camera_on: false,
                screen_sharing: false,
                frame: Default::default(),
                has_frame: false,
            }
        })
        .collect();
    // self last, as the QML grid orders it
    tiles.push(crate::CallParticipant {
        identity: me.clone().into(),
        display_name: "You".into(),
        initials: crate::rows::initials(&short(&me)).into(),
        tint: crate::rows::tint_for(&me),
        avatar: Default::default(),
        speaking: s.local_speaking,
        mic_muted: s.muted,
        camera_on: false,
        screen_sharing: false,
        frame: Default::default(),
        has_frame: false,
    });
    let remotes = s.participants.len();
    let status = match (s.state, s.connected_at) {
        ("joining", _) => "Calling…".to_string(),
        (_, Some(t)) if remotes > 0 => mmss(t.elapsed()),
        (_, Some(_)) => "Ringing…".to_string(),
        _ => "Connecting…".to_string(),
    };
    win.set_call_status(status.into());
    win.set_call_error(s.error.as_str().into());
    win.set_call_mic_muted(s.muted);
    win.set_call_group_mode(remotes > 1);
    if let Some(first) = s.participants.values().next() {
        let name = if first.name.is_empty() {
            short(&first.user)
        } else {
            first.name.clone()
        };
        win.set_call_peer_name(name.clone().into());
        win.set_call_peer_initials(crate::rows::initials(&name).into());
        win.set_call_peer_tint(crate::rows::tint_for(if first.user.is_empty() {
            &first.peer
        } else {
            &first.user
        }));
        win.set_call_peer_speaking(first.speaking);
    } else {
        let name = ui
            .rooms_json
            .iter()
            .find(|r| r["id"].as_str() == Some(s.room_id.as_str()))
            .and_then(|r| r["name"].as_str())
            .unwrap_or("")
            .to_string();
        win.set_call_peer_name(name.clone().into());
        win.set_call_peer_initials(crate::rows::initials(&name).into());
        win.set_call_peer_tint(crate::rows::tint_for(&s.room_id));
        win.set_call_peer_speaking(false);
    }
    win.set_call_tiles(ModelRc::new(VecModel::from(tiles)));
    let mics: Vec<crate::DeviceRow> = calls
        .devices
        .mics
        .iter()
        .map(|(id, label)| crate::DeviceRow {
            id: id.clone().into(),
            label: label.clone().into(),
            selected: *id == calls.selected_mic || (calls.selected_mic.is_empty() && false),
        })
        .collect();
    let speakers: Vec<crate::DeviceRow> = calls
        .devices
        .speakers
        .iter()
        .map(|(id, label)| crate::DeviceRow {
            id: id.clone().into(),
            label: label.clone().into(),
            selected: *id == calls.selected_speaker,
        })
        .collect();
    win.set_call_mics(ModelRc::new(VecModel::from(mics)));
    win.set_call_speakers(ModelRc::new(VecModel::from(speakers)));
    let floaters: Vec<crate::Floater> = s
        .floaters
        .iter()
        .map(|(id, emoji, who, _)| crate::Floater {
            fid: *id,
            emoji: emoji.clone().into(),
            who: who.clone().into(),
        })
        .collect();
    win.set_call_floaters(ModelRc::new(VecModel::from(floaters)));
}

/// The `call.state` object as the QML service kept it, for tests and logs.
pub fn state_json(ui: &UiState) -> Value {
    match ui.calls.session.as_ref() {
        None => json!({"state": "idle"}),
        Some(s) => json!({
            "state": s.state,
            "roomId": s.room_id,
            "callId": s.call_id,
            "participants": s.participants.values().map(|p| json!({
                "userId": p.user, "displayName": p.name, "micMuted": p.muted,
                "speaking": p.speaking, "level": p.level,
            })).collect::<Vec<_>>(),
            "local": {"micMuted": s.muted, "speaking": s.local_speaking, "level": s.local_level},
        }),
    }
}
