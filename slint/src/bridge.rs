//! The engine, linked in-process: a tokio runtime hosts `Engine`, the UI
//! thread owns every model, and the two meet only through
//! `slint::invoke_from_event_loop`. This file is the whole transport — the
//! JSON protocol is unchanged from the socket (core/docs/protocol.md), there
//! is just no socket.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use serde_json::{json, Map, Value};
use sigil_engine::engine::Engine;
use sigil_engine::ipc::hub::Hub;
use sigil_engine::ipc::wire::{Reply, Request};

use crate::rows::{self, IconSet, RowShape};
use crate::{AppWindow, RoomRow, TimelineRow};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

/// Fire requests at the engine from the UI thread.
#[derive(Clone)]
pub struct Requester {
    handle: tokio::runtime::Handle,
    engine: Arc<Engine>,
}

impl Requester {
    pub fn fire(&self, req: &str, params: Value) {
        self.call(req, params, |_| {});
    }

    pub fn call(&self, req: &str, params: Value, done: impl FnOnce(Reply) + Send + 'static) {
        let engine = self.engine.clone();
        let request = Request {
            req: req.to_string(),
            id: None,
            params: match params {
                Value::Object(m) => m,
                _ => Map::new(),
            },
        };
        let name = request.req.clone();
        self.handle.spawn(async move {
            let reply = engine.dispatch(request).await;
            if let Reply::Err(e) = &reply {
                tracing::warn!("{name}: {} {}", e.code, e.message);
            }
            done(reply);
        });
    }
}

/// Everything the event handlers touch. Lives on the UI thread only.
pub struct UiState {
    pub win: slint::Weak<AppWindow>,
    pub req: Requester,
    pub icons: IconSet,
    /// Last rooms.list, unfiltered: search re-projects from here.
    pub rooms_json: Vec<Value>,
    pub search: String,
    /// The open room and the full item list its diffs index into.
    pub open_room: String,
    pub shadow: Vec<Value>,
    pub typing: HashMap<String, Vec<Value>>,
    pub my_user: String,
    pub login_url: String,
    /// Avatar images by path; a cache because rooms.list re-arrives constantly.
    pub avatars: HashMap<String, slint::Image>,
    /// Whether the last typing notice we sent said "typing".
    pub typing_sent: bool,
}

thread_local! {
    static UI: RefCell<Option<Rc<RefCell<UiState>>>> = const { RefCell::new(None) };
}

pub fn with_ui(f: impl FnOnce(&mut UiState)) {
    UI.with(|ui| {
        if let Some(state) = ui.borrow().as_ref() {
            f(&mut state.borrow_mut());
        }
    });
}

/// Boot the engine and wire the event stream into the UI. Call once, on the
/// UI thread, after the window exists.
pub fn start(win: &AppWindow, rt: &tokio::runtime::Runtime, icons: IconSet) -> Requester {
    sigil_engine::init_crypto();

    let engine = Engine::new(Hub::new());
    let req = Requester { handle: rt.handle().clone(), engine: engine.clone() };

    let state = Rc::new(RefCell::new(UiState {
        win: win.as_weak(),
        req: req.clone(),
        icons,
        rooms_json: Vec::new(),
        search: String::new(),
        open_room: String::new(),
        shadow: Vec::new(),
        typing: HashMap::new(),
        my_user: String::new(),
        login_url: String::new(),
        avatars: HashMap::new(),
        typing_sent: false,
    }));
    UI.with(|ui| *ui.borrow_mut() = Some(state));

    // Event pump: engine broadcasts → parse off-thread → apply on the UI thread.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(1024);
    let sub = engine.hub.subscribe(tx.clone());
    std::mem::forget(sub); // the one subscriber lives as long as the process
    for ev in engine.greeting() {
        let _ = tx.try_send(ev.to_string());
    }
    rt.spawn(async move {
        while let Some(line) = rx.recv().await {
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            let _ = slint::invoke_from_event_loop(move || with_ui(|ui| handle_event(ui, &v)));
        }
    });
    {
        let engine = engine.clone();
        rt.spawn(async move { engine.startup().await });
    }
    wire_callbacks(win, req.clone());
    req
}

/// UI actions → engine requests. Every handler runs on the UI thread; the
/// Requester hops to the runtime.
fn wire_callbacks(win: &AppWindow, req: Requester) {
    win.on_sign_in({
        let req = req.clone();
        move |hs| {
            let hs = hs.trim().to_string();
            // Show pending before the engine says so: building the client can
            // take seconds, and an idle-looking button collects extra taps.
            with_ui(|ui| {
                if let Some(win) = ui.win.upgrade() {
                    win.set_session("loginPending".into());
                    win.set_login_error(SharedString::new());
                }
            });
            req.call("login.start", json!({"homeserver": hs, "openBrowser": false}), |reply| {
                match reply {
                    Reply::Ok(v) => {
                        if let Some(url) = v["url"].as_str() {
                            crate::platform::open_url(url);
                        }
                    }
                    // Request errors never arrive as login.failed events;
                    // surface them or the button just looks dead.
                    Reply::Err(e) => {
                        let msg = e.message.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            with_ui(|ui| {
                                if let Some(win) = ui.win.upgrade() {
                                    win.set_login_error(msg.as_str().into());
                                }
                            });
                        });
                    }
                }
            });
        }
    });
    win.on_open_again(|| {
        with_ui(|ui| {
            if !ui.login_url.is_empty() {
                crate::platform::open_url(&ui.login_url);
            }
        });
    });
    win.on_cancel_login({
        let req = req.clone();
        move || req.fire("login.cancel", json!({}))
    });
    win.on_room_clicked(|id| {
        with_ui(|ui| {
            let Some(win) = ui.win.upgrade() else { return };
            ui.open_room = id.to_string();
            ui.shadow.clear();
            ui.typing_sent = false;
            win.set_items(ModelRc::new(VecModel::from(Vec::<TimelineRow>::new())));
            win.set_typing_line(typing_line(ui).into());
            set_chat_header(ui, &win);
            win.set_in_chat(true);
            ui.req.fire("room.open", json!({"roomId": ui.open_room, "initialItems": 60}));
            ui.req.fire("ui.focus", json!({"roomId": ui.open_room, "visible": true}));
            ui.req.fire("room.markRead", json!({"roomId": ui.open_room}));
        });
    });
    win.on_back_to_home(|| {
        with_ui(|ui| {
            let Some(win) = ui.win.upgrade() else { return };
            if ui.typing_sent {
                ui.req.fire("typing", json!({"roomId": ui.open_room, "typing": false}));
                ui.typing_sent = false;
            }
            ui.req.fire("ui.focus", json!({"roomId": ui.open_room, "visible": false}));
            ui.req.fire("room.close", json!({"roomId": ui.open_room}));
            ui.open_room.clear();
            ui.shadow.clear();
            win.set_in_chat(false);
        });
    });
    win.on_send_message(|text| {
        with_ui(|ui| {
            let Some(win) = ui.win.upgrade() else { return };
            ui.req.fire(
                "message.send",
                json!({"roomId": ui.open_room, "body": text.as_str(), "markdown": true}),
            );
            if ui.typing_sent {
                ui.req.fire("typing", json!({"roomId": ui.open_room, "typing": false}));
                ui.typing_sent = false;
            }
            win.invoke_clear_composer();
        });
    });
    win.on_composer_edited(|text| {
        with_ui(|ui| {
            // Notice on the empty↔non-empty transition only; the engine
            // handles refresh while it stays true.
            let now = !text.trim().is_empty();
            if now != ui.typing_sent && !ui.open_room.is_empty() {
                ui.typing_sent = now;
                ui.req.fire("typing", json!({"roomId": ui.open_room, "typing": now}));
            }
        });
    });
    win.on_search_edited(|q| {
        with_ui(|ui| {
            let Some(win) = ui.win.upgrade() else { return };
            ui.search = q.to_string();
            rebuild_rooms(ui, &win);
        });
    });
    win.on_new_chat(|| tracing::info!("new chat: not built yet"));
}

fn handle_event(ui: &mut UiState, v: &Value) {
    let Some(win) = ui.win.upgrade() else { return };
    match v["event"].as_str().unwrap_or("") {
        "status" => {
            let session = v["session"].as_str().unwrap_or("restoring");
            win.set_session(session.into());
            ui.my_user = v["userId"].as_str().unwrap_or("").to_string();
            ui.login_url = v["login"]["url"].as_str().unwrap_or("").to_string();
            win.set_my_initials(rows::initials(v["displayName"].as_str().unwrap_or(&ui.my_user)).into());
            win.set_my_tint(rows::tint_for(&ui.my_user));
            if let Some(img) = avatar(ui, v["avatarPath"].as_str().unwrap_or("")) {
                win.set_my_avatar(img);
            }
            let sync = v["sync"].as_str().unwrap_or("");
            let sync_err = v["syncError"].as_str().unwrap_or("");
            win.set_sync_line(match (sync, sync_err) {
                (_, e) if !e.is_empty() => format!("Sync error: {e}").into(),
                ("offline", _) => "Offline".into(),
                _ => SharedString::new(),
            });
        }
        "login.failed" => {
            win.set_login_error(v["error"]["message"].as_str().unwrap_or("login failed").into());
        }
        "login.finished" => win.set_login_error(SharedString::new()),
        "rooms.list" => {
            ui.rooms_json = v["rooms"].as_array().cloned().unwrap_or_default();
            rebuild_rooms(ui, &win);
        }
        "room.typing" => {
            let room = v["roomId"].as_str().unwrap_or("").to_string();
            let users = v["users"].as_array().cloned().unwrap_or_default();
            if users.is_empty() {
                ui.typing.remove(&room);
            } else {
                ui.typing.insert(room.clone(), users);
            }
            rebuild_rooms(ui, &win);
            if room == ui.open_room {
                win.set_typing_line(typing_line(ui).into());
            }
        }
        "timeline.reset" => {
            if v["roomId"].as_str().unwrap_or("") != ui.open_room {
                return;
            }
            ui.shadow = v["items"].as_array().cloned().unwrap_or_default();
            rebuild_timeline(ui, &win);
            win.invoke_scroll_timeline_to_end();
        }
        "timeline.diff" => {
            if v["roomId"].as_str().unwrap_or("") != ui.open_room {
                return;
            }
            let mut at_end = false;
            for op in v["ops"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
                at_end |= apply_diff(&mut ui.shadow, op);
            }
            rebuild_timeline(ui, &win);
            if at_end {
                win.invoke_scroll_timeline_to_end();
                // The room is on screen; reading it is what looking at it means.
                ui.req.fire("room.markRead", json!({"roomId": ui.open_room}));
            }
        }
        _ => {}
    }
}

/// Mirror one `eyeball_im::VectorDiff` op onto the shadow list.
/// Returns true when the op grew the tail (worth scrolling for).
fn apply_diff(shadow: &mut Vec<Value>, op: &Value) -> bool {
    let items = |k: &str| op[k].as_array().cloned().unwrap_or_default();
    let idx = |k: &str| op[k].as_u64().unwrap_or(0) as usize;
    match op["op"].as_str().unwrap_or("") {
        "append" => {
            shadow.extend(items("items"));
            true
        }
        "clear" => {
            shadow.clear();
            false
        }
        "pushFront" => {
            shadow.insert(0, op["item"].clone());
            false
        }
        "pushBack" => {
            shadow.push(op["item"].clone());
            true
        }
        "popFront" => {
            if !shadow.is_empty() {
                shadow.remove(0);
            }
            false
        }
        "popBack" => {
            shadow.pop();
            false
        }
        "insert" => {
            let i = idx("index").min(shadow.len());
            shadow.insert(i, op["item"].clone());
            i + 1 == shadow.len()
        }
        "set" => {
            let i = idx("index");
            if i < shadow.len() {
                shadow[i] = op["item"].clone();
            }
            false
        }
        "remove" => {
            let i = idx("index");
            if i < shadow.len() {
                shadow.remove(i);
            }
            false
        }
        "truncate" => {
            shadow.truncate(idx("len"));
            false
        }
        "reset" => {
            *shadow = items("items");
            true
        }
        other => {
            tracing::warn!("unknown timeline op {other:?}");
            false
        }
    }
}

fn typing_line(ui: &UiState) -> String {
    match ui.typing.get(&ui.open_room).map(|u| u.as_slice()).unwrap_or(&[]) {
        [] => String::new(),
        [one] => format!("{} is typing…", one["displayName"].as_str().unwrap_or("Someone")),
        many => format!("{} people are typing…", many.len()),
    }
}

fn avatar(ui: &mut UiState, path: &str) -> Option<slint::Image> {
    if path.is_empty() {
        return None;
    }
    if let Some(img) = ui.avatars.get(path) {
        return Some(img.clone());
    }
    let img = slint::Image::load_from_path(std::path::Path::new(path)).ok()?;
    ui.avatars.insert(path.to_string(), img.clone());
    Some(img)
}

pub fn rebuild_rooms(ui: &mut UiState, win: &AppWindow) {
    let q = ui.search.to_lowercase();
    let mut chats: Vec<RoomRow> = Vec::new();
    let mut spaces: Vec<RoomRow> = Vec::new();
    let rooms_json = std::mem::take(&mut ui.rooms_json);
    for room in &rooms_json {
        let name = room["name"].as_str().unwrap_or("").to_string();
        if !q.is_empty() && !name.to_lowercase().contains(&q) {
            continue;
        }
        let id = room["id"].as_str().unwrap_or("").to_string();
        let is_dm = room["isDm"].as_bool().unwrap_or(false);
        let tint_key = if is_dm {
            room["dmUserId"].as_str().unwrap_or(&id).to_string()
        } else {
            id.clone()
        };
        let typing = ui.typing.get(&id).cloned().unwrap_or_default();
        let preview = rows::preview_for(room, &typing, &ui.icons);
        let (badge, badge_urgent) = rows::badge_for(room);
        let unread = room["unread"].as_i64().unwrap_or(0).max(room["unreadMessages"].as_i64().unwrap_or(0)) > 0;
        let row = RoomRow {
            id: id.clone().into(),
            name: name.clone().into(),
            initials: rows::initials(&name).into(),
            avatar: avatar(ui, room["avatarPath"].as_str().unwrap_or("")).unwrap_or_default(),
            tint: rows::tint_for(&tint_key),
            preview: preview.text.into(),
            preview_icon: preview.icon,
            stamp: room["stamp"].as_str().unwrap_or("").into(),
            badge: badge.into(),
            badge_urgent,
            unread,
            is_favourite: room["isFavourite"].as_bool().unwrap_or(false),
            is_encrypted: room["isEncrypted"].as_bool().unwrap_or(false),
            has_call: room["hasActiveCall"].as_bool().unwrap_or(false),
            is_invite: room["isInvite"].as_bool().unwrap_or(false),
            is_typing: preview.typing,
        };
        if room["isSpace"].as_bool().unwrap_or(false) {
            spaces.push(row);
        } else {
            chats.push(row);
        }
    }
    ui.rooms_json = rooms_json;
    win.set_rooms(ModelRc::new(VecModel::from(chats)));
    win.set_spaces(ModelRc::new(VecModel::from(spaces)));

    // The open room's header can change under us (name, encryption, call).
    if !ui.open_room.is_empty() {
        set_chat_header(ui, win);
    }
}

pub fn set_chat_header(ui: &mut UiState, win: &AppWindow) {
    let Some(room) = ui.rooms_json.iter().find(|r| r["id"].as_str() == Some(ui.open_room.as_str())).cloned() else {
        return;
    };
    let name = room["name"].as_str().unwrap_or("").to_string();
    let is_dm = room["isDm"].as_bool().unwrap_or(false);
    let members = room["joinedMembers"].as_i64().unwrap_or(0);
    win.set_room_name(name.clone().into());
    win.set_room_encrypted(room["isEncrypted"].as_bool().unwrap_or(false));
    win.set_room_initials(rows::initials(&name).into());
    win.set_room_tint(rows::tint_for(if is_dm {
        room["dmUserId"].as_str().unwrap_or(&ui.open_room)
    } else {
        &ui.open_room
    }));
    win.set_room_avatar(avatar(ui, room["avatarPath"].as_str().unwrap_or("")).unwrap_or_default());
    // Counts go in the subtitle, worded unconditionally (ui-conventions.md).
    win.set_room_subtitle(if is_dm {
        SharedString::new()
    } else {
        format!("{members} members").into()
    });
}

pub fn rebuild_timeline(ui: &mut UiState, win: &AppWindow) {
    let is_dm = ui
        .rooms_json
        .iter()
        .find(|r| r["id"].as_str() == Some(ui.open_room.as_str()))
        .and_then(|r| r["isDm"].as_bool())
        .unwrap_or(false);
    let mut rows_out: Vec<TimelineRow> = Vec::with_capacity(ui.shadow.len());
    for (i, item) in ui.shadow.iter().enumerate() {
        let shape = rows::shape_for(item, &ui.icons);
        let (kind, body, media_icon): (&str, String, SharedString) = match shape {
            RowShape::Skip => continue,
            RowShape::Divider(label) => ("dayDivider", label, Default::default()),
            RowShape::State(text) => ("state", text, Default::default()),
            RowShape::Bubble { media_icon, body_override } => {
                let body = body_override.unwrap_or_else(|| item["body"].as_str().unwrap_or("").to_string());
                let k = match item["kind"].as_str().unwrap_or("text") {
                    k @ ("text" | "notice" | "emote") => k,
                    _ => "media",
                };
                (k, body, media_icon)
            }
        };
        let is_own = item["isOwn"].as_bool().unwrap_or(false);
        let sender = item["sender"].as_str().unwrap_or("").to_string();
        let sender_name = item["senderName"].as_str().unwrap_or(&sender).to_string();
        let group_start = !i.checked_sub(1).map(|p| rows::same_group(&ui.shadow[p], item)).unwrap_or(false);
        let group_end = !ui.shadow.get(i + 1).map(|nx| rows::same_group(item, nx)).unwrap_or(false);
        let bubble = kind != "dayDivider" && kind != "state";
        rows_out.push(TimelineRow {
            id: item["id"].as_str().unwrap_or("").into(),
            kind: kind.into(),
            body: body.into(),
            sender: sender_name.clone().into(),
            show_sender: bubble && group_start && !is_own && !is_dm && !sender_name.is_empty(),
            initials: rows::initials(&sender_name).into(),
            tint: rows::tint_for(&sender),
            show_avatar: bubble && group_end && !is_own && !is_dm,
            is_own,
            group_start,
            group_end,
            stamp: rows::bubble_stamp(item["ts"].as_i64().unwrap_or(0)).into(),
            edited: item["isEdited"].as_bool().unwrap_or(false),
            highlighted: item["isHighlighted"].as_bool().unwrap_or(false),
            media_icon,
        });
    }
    win.set_items(ModelRc::new(VecModel::from(rows_out)));
}
