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
    pub fn handle(&self) -> tokio::runtime::Handle {
        self.handle.clone()
    }

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
    /// THE timeline model. Mutated in place: handing the ListView a fresh
    /// model on every diff resets the viewport, which reads as "cannot
    /// scroll" the moment receipts start flowing.
    pub items_model: std::rc::Rc<VecModel<TimelineRow>>,
    /// Whether the last typing notice we sent said "typing".
    pub typing_sent: bool,
    // ---- Service.qml parity ----
    pub spaces_tree: Vec<Value>,
    pub receipts_by_room: HashMap<String, Vec<Value>>,
    /// roomId -> unsent composer text (Panel.qml drafts).
    pub drafts: HashMap<String, String>,
    pub presence_by_user: Value,
    pub pinned_by_room: HashMap<String, Vec<String>>,
    pub pagination_by_room: HashMap<String, String>,
    pub call: Value,
    pub devices: Value,
    pub voice_level: f32,
    // ---- per-page working state ----
    pub space_hierarchy: Vec<Value>,       // last space.hierarchy rooms
    pub spacerooms_mode: String,
    pub spacerooms_selected: std::collections::HashSet<String>,
    pub members_filter: i64,
    pub members: Vec<Value>,               // room.members of settings_room
    pub settings_room: String,             // roomId the settings pages serve
    pub settings: Value,                   // last room.settings reply
    pub search_query: String,
    pub forward_query: String,
    pub forward_item: Value,               // staged message for forward
    pub start_query_epoch: u64,
    pub dir_query_epoch: u64,
    pub doc_ctx: (String, String),         // roomId, eventId the doc page shows
    pub audio_ctx: (String, String),
    pub audio_playing: bool,
    pub new_space_avatar: String,
    pub sheet_item: Value,                 // message the action sheet targets
    pub emojis: Vec<(String, String)>,     // glyph, keywords
    pub voice_positions: HashMap<String, f64>, // eventId -> seconds (playback)
    pub chat_themes: Value,
    pub viewer_items: Vec<Value>,          // timeline items behind the viewer pager
    pub doc_pages: Vec<Value>,             // doc.page results by index
    pub stickers: Vec<Value>,
    pub voice_clip: Value,                 // voice.stop reply (path/duration/waveform)
    pub recording: bool,
    pub rec_levels: Vec<f32>,
    pub theme_pending: Value,
    pub doc_preview: Value,
    /// roomId|eventId -> doc.thumb reply (Null = asked, pending).
    pub doc_thumbs: HashMap<String, Value>,
    /// roomId|eventId -> audio.info reply (Null = asked, pending).
    pub audio_infos: HashMap<String, Value>,
    /// url -> link.preview reply (Null = asked, pending; false = failed).
    pub link_previews: HashMap<String, Value>,
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
    if std::env::var_os("SIGIL_SLINT_DEMO").is_some() {
        return start_demo(win, rt, icons);
    }
    sigil_engine::init_crypto();

    let engine = Engine::new(Hub::new());
    let req = Requester { handle: rt.handle().clone(), engine: engine.clone() };

    // Anchors animation-tick() to the wall clock for the live-share countdown.
    win.set_boot_epoch_s(chrono::Utc::now().timestamp() as i32);
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
        items_model: std::rc::Rc::new(VecModel::default()),
        typing_sent: false,
        spaces_tree: Vec::new(),
        receipts_by_room: HashMap::new(),
        drafts: HashMap::new(),
        presence_by_user: Value::Null,
        pinned_by_room: HashMap::new(),
        pagination_by_room: HashMap::new(),
        call: Value::Null,
        devices: Value::Null,
        voice_level: 0.0,
        space_hierarchy: Vec::new(),
        spacerooms_mode: "manage".into(),
        spacerooms_selected: Default::default(),
        members_filter: -1,
        members: Vec::new(),
        settings_room: String::new(),
        settings: Value::Null,
        search_query: String::new(),
        forward_query: String::new(),
        forward_item: Value::Null,
        start_query_epoch: 0,
        dir_query_epoch: 0,
        doc_ctx: Default::default(),
        audio_ctx: Default::default(),
        audio_playing: false,
        new_space_avatar: String::new(),
        sheet_item: Value::Null,
        emojis: Vec::new(),
        voice_positions: HashMap::new(),
        chat_themes: serde_json::json!({}),
        viewer_items: Vec::new(),
        doc_pages: Vec::new(),
        stickers: Vec::new(),
        voice_clip: Value::Null,
        recording: false,
        rec_levels: Vec::new(),
        theme_pending: serde_json::json!({}),
        doc_preview: Value::Null,
        doc_thumbs: HashMap::new(),
        audio_infos: HashMap::new(),
        link_previews: HashMap::new(),
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
    crate::actions::wire_extra(win);
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
            open_room(ui, &win, id.as_str());
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
            win.set_nav("home".into());
        });
    });
    win.on_send_message(|text| {
        with_ui(|ui| {
            let Some(win) = ui.win.upgrade() else { return };
            // ChatPage.send() returns early on whitespace-only input.
            if text.trim().is_empty() {
                return;
            }
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
    win.on_recover_submit(|key| {
        with_ui(|ui| {
            let Some(win) = ui.win.upgrade() else { return };
            win.set_recovery_busy(true);
            win.set_recovery_error(SharedString::new());
            ui.req.call("recovery.recover", json!({"key": key.as_str()}), |reply| {
                if let Reply::Err(e) = reply {
                    let msg = e.message.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        with_ui(|ui| {
                            if let Some(win) = ui.win.upgrade() {
                                win.set_recovery_busy(false);
                                win.set_recovery_error(msg.as_str().into());
                            }
                        });
                    });
                }
                // Success arrives as a recovery.status broadcast.
            });
        });
    });
    win.on_sign_out({
        let req = req.clone();
        move || req.fire("logout", json!({"wipe": true}))
    });
}

fn handle_event(ui: &mut UiState, v: &Value) {
    let Some(win) = ui.win.upgrade() else { return };
    match v["event"].as_str().unwrap_or("") {
        "status" => {
            let session = v["session"].as_str().unwrap_or("restoring");
            let was_in = win.get_session() == "loggedIn";
            win.set_session(session.into());
            if was_in && session != "loggedIn" {
                // Signed out from under the UI: drop everything the session owned.
                ui.rooms_json.clear();
                ui.shadow.clear();
                ui.typing.clear();
                ui.avatars.clear();
                ui.open_room.clear();
                win.set_rooms(ModelRc::new(VecModel::from(Vec::<RoomRow>::new())));
                win.set_spaces(ModelRc::new(VecModel::from(Vec::<RoomRow>::new())));
                win.set_items(ModelRc::new(VecModel::from(Vec::<TimelineRow>::new())));
                win.set_nav("home".into());
                win.set_rooms_loaded(false);
                win.set_recovery_skipped(false);
                win.set_recovery_open(false);
                win.set_my_avatar(Default::default());
            }
            ui.my_user = v["userId"].as_str().unwrap_or("").to_string();
            win.set_my_user_id(ui.my_user.as_str().into());
            win.set_my_name(v["displayName"].as_str().unwrap_or("").into());
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
        "recovery.status" => {
            tracing::info!("recovery.status: {v}");
            // Service.qml:59 — recovery state gates the page; `verified` only
            // covers the disabled case. OIDC logins come up verified with
            // secret storage still locked, so verified alone hides the page.
            let verified = v["verified"].as_bool().unwrap_or(false);
            let recovery = v["recovery"].as_str().unwrap_or("unknown");
            let needs = recovery == "incomplete" || (recovery == "disabled" && !verified);
            win.set_needs_recovery(needs);
            win.set_recovery_busy(false);
            if !needs {
                win.set_recovery_open(false);
            }
        }
        "rooms.list" => {
            win.set_rooms_loaded(v["loaded"].as_bool().unwrap_or(true));
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
        "spaces.tree" => {
            ui.spaces_tree = v["spaces"].as_array().cloned().unwrap_or_default();
            // Levels and children feed the Spaces tab rows.
            rebuild_rooms(ui, &win);
        }
        "room.receipts" => {
            let room = v["roomId"].as_str().unwrap_or("").to_string();
            ui.receipts_by_room.insert(room.clone(), v["users"].as_array().cloned().unwrap_or_default());
            if room == ui.open_room {
                rebuild_timeline(ui, &win);
            }
        }
        "presence.list" => {
            ui.presence_by_user = v["users"].clone();
            // Your own dot on the Home header avatar.
            let me = &ui.presence_by_user[ui.my_user.as_str()];
            win.set_my_presence(if me["busy"].as_bool().unwrap_or(false) {
                "busy".into()
            } else {
                me["state"].as_str().unwrap_or("").into()
            });
            rebuild_rooms(ui, &win);
        }
        "room.pinned" => {
            let room = v["roomId"].as_str().unwrap_or("").to_string();
            let ids: Vec<String> = v["events"].as_array().map(|a| {
                a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()
            }).unwrap_or_default();
            ui.pinned_by_room.insert(room.clone(), ids);
            if crate::actions::room_of_key(&ui.open_room) == room {
                rebuild_timeline(ui, &win);
                crate::actions::reload_pins_if_open(ui, &win);
            }
        }
        "timeline.paginationState" => {
            let room = v["roomId"].as_str().unwrap_or("").to_string();
            let state = v["state"].as_str().unwrap_or("idle").to_string();
            if room == ui.open_room {
                win.set_pagination_state(state.clone().into());
            }
            ui.pagination_by_room.insert(room, state);
        }
        "media.ready" => {
            crate::actions::apply_media_ready(ui, &win, v);
        }
        "call.state" => {
            ui.call = v.clone();
            win.set_call_state(v["state"].as_str().unwrap_or("idle").into());
            let incoming = &v["incoming"];
            let has_incoming = !incoming.is_null();
            win.set_call_incoming(has_incoming);
            if has_incoming {
                let caller = incoming["senderName"].as_str()
                    .or(incoming["sender"].as_str())
                    .unwrap_or("Incoming call");
                win.set_call_incoming_name(caller.into());
                win.set_call_incoming_tint(rows::tint_for(incoming["sender"].as_str().unwrap_or(caller)));
                let room = incoming["roomId"].as_str().unwrap_or("");
                let name = ui.rooms_json.iter()
                    .find(|r| r["id"].as_str() == Some(room))
                    .and_then(|r| r["name"].as_str())
                    .unwrap_or("");
                win.set_call_room_name(name.into());
            }
        }
        "call.devices" => {
            ui.devices = v.clone();
        }
        "voice.level" => {
            ui.voice_level = v["level"].as_f64().unwrap_or(0.0) as f32;
            if ui.recording {
                ui.rec_levels.push(ui.voice_level);
                if ui.rec_levels.len() > 60 {
                    let drop = ui.rec_levels.len() - 60;
                    ui.rec_levels.drain(..drop);
                }
                win.set_rec_levels(ModelRc::new(VecModel::from(ui.rec_levels.clone())));
            }
        }
        "timeline.reset" => {
            if v["roomId"].as_str().unwrap_or("") != ui.open_room {
                return;
            }
            ui.shadow = v["items"].as_array().cloned().unwrap_or_default();
            rebuild_timeline(ui, &win);
            // Once now, and again after layout: at reset time the viewport
            // height is not computed yet, so the first call clamps to the top.
            win.invoke_scroll_timeline_to_end();
            let req = ui.req.clone();
            crate::actions::after_pub(&req, 150, |_ui, win| win.invoke_scroll_timeline_to_end());
            crate::actions::after_pub(&req, 450, |_ui, win| win.invoke_scroll_timeline_to_end());
        }
        "timeline.diff" => {
            if v["roomId"].as_str().unwrap_or("") != ui.open_room {
                return;
            }
            let mut grew_tail = false;
            for op in v["ops"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
                grew_tail |= apply_diff(&mut ui.shadow, op);
            }
            // QML's atYEnd rule: stick to the bottom only when the reader is
            // there; otherwise hold their distance from it across the rebuild
            // (prepends from pagination included).
            let from_end = win.get_chat_from_end();
            rebuild_timeline(ui, &win);
            if grew_tail && from_end < 8.0 {
                win.invoke_scroll_timeline_to_end();
                ui.req.fire("room.markRead", json!({"roomId": crate::actions::room_of_key(&ui.open_room)}));
            } else {
                win.invoke_restore_timeline_from_end(from_end);
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

pub fn avatar_pub(ui: &mut UiState, path: &str) -> Option<slint::Image> {
    avatar(ui, path)
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

pub fn open_room(ui: &mut UiState, win: &AppWindow, id: &str) {
    // Panel.qml keeps unsent composer text per room: bank the outgoing room's
    // draft, and restore the incoming one below.
    if !ui.open_room.is_empty() && ui.open_room != id {
        let leaving = crate::actions::room_of_key(&ui.open_room);
        let text = win.get_ct_composer_text().to_string();
        if text.trim().is_empty() { ui.drafts.remove(&leaving); }
        else { ui.drafts.insert(leaving, text); }
    }
    ui.open_room = id.to_string();
    ui.shadow.clear();
    ui.typing_sent = false;
    ui.items_model.set_vec(Vec::new());
    win.set_typing_line(typing_line(ui).into());
    win.set_chat_is_thread(false);
    win.set_pagination_state("idle".into());
    set_chat_header(ui, win);
    win.set_nav("chat".into());
    ui.req.fire("room.open", json!({"roomId": ui.open_room, "initialItems": 80}));
    ui.req.fire("ui.focus", json!({"roomId": crate::actions::room_of_key(&ui.open_room), "visible": true}));
    ui.req.fire("room.markRead", json!({"roomId": crate::actions::room_of_key(&ui.open_room)}));
    // Pins come as a reply here and as room.pinned pushes afterwards.
    let rid = crate::actions::room_of_key(&ui.open_room);
    crate::actions::load_pinned_ids(ui, &rid);
    // Restore this room's unsent draft into the composer.
    let draft = ui.drafts.get(&rid).cloned().unwrap_or_default();
    let cursor = draft.len() as i32;
    win.invoke_chat_composer_set(draft.into(), cursor);
}

/// Point the chat surface at a different view key (a thread) without the
/// room.open side effects — thread.open already opened it engine-side.
pub fn switch_timeline(ui: &mut UiState, _win: &AppWindow, key: &str) {
    ui.open_room = key.to_string();
    ui.shadow.clear();
    ui.items_model.set_vec(Vec::new());
}

pub fn room_row_of(ui: &mut UiState, room: &Value) -> RoomRow {
    let name = room["name"].as_str().unwrap_or("").to_string();
    let id = room["id"].as_str().unwrap_or("").to_string();
    let is_dm = room["isDm"].as_bool().unwrap_or(false);
    let tint_key = if is_dm {
        room["dmUserId"].as_str().unwrap_or(&id).to_string()
    } else {
        id.clone()
    };
    let typing = ui.typing.get(&id).cloned().unwrap_or_default();
    // A room has no presence; only the person on the other end of a DM does.
    let presence = if is_dm {
        let p = &ui.presence_by_user[&tint_key];
        if p["busy"].as_bool().unwrap_or(false) { "busy".to_string() }
        else { p["state"].as_str().unwrap_or("").to_string() }
    } else {
        String::new()
    };
    let preview = rows::preview_for(room, &typing, &ui.icons);
    let (badge, badge_urgent) = rows::badge_for(room);
    let unread = room["unread"].as_i64().unwrap_or(0).max(room["unreadMessages"].as_i64().unwrap_or(0)) > 0;
    RoomRow {
        id: id.clone().into(),
        is_dm,
        is_space: room["isSpace"].as_bool().unwrap_or(false),
        topic: room["topic"].as_str().unwrap_or("").into(),
        member_count: room["joinedMembers"].as_i64().unwrap_or(0) as i32,
        is_low_priority: room["isLowPriority"].as_bool().unwrap_or(false),
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
        presence: presence.into(),
        // A draft outranks the last message unless someone is typing (QML
        // showDraft); invites keep their invitation line.
        draft: if typing.is_empty() && !room["isInvite"].as_bool().unwrap_or(false) {
            ui.drafts.get(&id).map(|d| d.trim().to_string()).unwrap_or_default().into()
        } else {
            SharedString::new()
        },
        child_count: ui.spaces_tree.iter()
            .find(|sp| sp["id"].as_str() == Some(id.as_str()))
            .and_then(|sp| sp["children"].as_array().map(|c| c.len() as i32))
            .unwrap_or(0),
    }
}

pub fn rebuild_rooms(ui: &mut UiState, win: &AppWindow) {
    let q = ui.search.to_lowercase();
    let mut chats: Vec<RoomRow> = Vec::new();
    let mut spaces: Vec<RoomRow> = Vec::new();
    let rooms_json = std::mem::take(&mut ui.rooms_json);
    for room in &rooms_json {
        let name = room["name"].as_str().unwrap_or("");
        if !q.is_empty() && !name.to_lowercase().contains(&q) {
            continue;
        }
        let row = room_row_of(ui, room);
        if room["isSpace"].as_bool().unwrap_or(false) {
            // HomePage.qml lists only top-level spaces on the Spaces tab.
            let top_level = ui.spaces_tree.iter()
                .find(|sp| sp["id"].as_str() == room["id"].as_str())
                .map(|sp| sp["level"].as_i64().unwrap_or(0) == 0)
                .unwrap_or(true);
            if top_level {
                spaces.push(row);
            }
        } else {
            chats.push(row);
        }
    }
    ui.rooms_json = rooms_json;
    win.set_rooms(ModelRc::new(VecModel::from(chats)));
    win.set_spaces(ModelRc::new(VecModel::from(spaces)));
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
    // A room has no presence; the person on the other end of a DM does.
    win.set_room_presence(if is_dm {
        let p = &ui.presence_by_user[room["dmUserId"].as_str().unwrap_or("")];
        if p["busy"].as_bool().unwrap_or(false) { "busy".into() }
        else { p["state"].as_str().unwrap_or("").into() }
    } else {
        SharedString::new()
    });
    win.set_chat_is_dm(is_dm);
    win.set_chat_is_invite(room["isInvite"].as_bool().unwrap_or(false));
    // Counts go in the subtitle, worded unconditionally (ui-conventions.md).
    win.set_room_subtitle(if is_dm {
        SharedString::new()
    } else {
        format!("{members} members").into()
    });
}

pub fn rebuild_timeline(ui: &mut UiState, win: &AppWindow) {
    let key = ui.open_room.clone();
    let room_id = crate::actions::room_of_key(&key);
    let room = ui.rooms_json.iter().find(|r| r["id"].as_str() == Some(room_id.as_str())).cloned();
    let is_dm = room.as_ref().and_then(|r| r["isDm"].as_bool()).unwrap_or(false);
    let pinned_ids = ui.pinned_by_room.get(&room_id).cloned().unwrap_or_default();
    let my_user = ui.my_user.clone();
    // The newest own message with an event id wears the sent/read mark.
    let receipt_owner = ui.shadow.iter().rev()
        .find(|i| i["isOwn"].as_bool().unwrap_or(false) && i["eventId"].as_str().map(|e| !e.is_empty()).unwrap_or(false))
        .and_then(|i| i["eventId"].as_str())
        .unwrap_or("")
        .to_string();
    let shadow = ui.shadow.clone();
    let mut rows_out: Vec<TimelineRow> = Vec::with_capacity(shadow.len());
    for (i, item) in shadow.iter().enumerate() {
        let shape = rows::shape_for(item, &ui.icons);
        let (kind, body, media_icon): (&str, String, slint::SharedString) = match shape {
            rows::RowShape::Skip => continue,
            rows::RowShape::Marker => {
                rows_out.push(TimelineRow { id: item["id"].as_str().unwrap_or("").into(), kind: "readMarker".into(), is_read_marker: true, ..Default::default() });
                continue;
            }
            rows::RowShape::Divider(label) => ("dayDivider", label, Default::default()),
            rows::RowShape::State(text) => ("state", text, Default::default()),
            rows::RowShape::Bubble { media_icon, body_override } => {
                let body = body_override.unwrap_or_else(|| item["body"].as_str().unwrap_or("").to_string());
                let k = match item["kind"].as_str().unwrap_or("text") {
                    k @ ("text" | "notice" | "emote" | "image" | "video" | "voice" | "audio" | "file" | "sticker" | "poll" | "contact" | "location") => k,
                    "liveLocation" => "location",
                    "redacted" | "utd" | "unsupported" => "notice",
                    _ => "text",
                };
                (k, body, media_icon)
            }
        };
        let is_own = item["isOwn"].as_bool().unwrap_or(false);
        let sender = item["sender"].as_str().unwrap_or("").to_string();
        let sender_name = match item["senderName"].as_str().unwrap_or("") { "" => sender.clone(), d => d.to_string() };
        let group_start = !i.checked_sub(1).map(|p| rows::same_group(&shadow[p], item)).unwrap_or(false);
        let group_end = !shadow.get(i + 1).map(|nx| rows::same_group(item, nx)).unwrap_or(false);
        let bubble = kind != "dayDivider" && kind != "state";
        let ts = item["ts"].as_i64().unwrap_or(0);
        // Session stamp above the row: day changed against the older
        // neighbour, or more than an hour passed (Service.recomputeGrouping).
        let day_label = if bubble && ts > 0 {
            let older = i.checked_sub(1).and_then(|p| shadow.get(p)).and_then(|o| o["ts"].as_i64()).filter(|t| *t > 0);
            match older {
                Some(ot) if rows::same_day(ts, ot) && ts - ot <= 3_600_000 => String::new(),
                _ => crate::project::session_label(ts),
            }
        } else {
            String::new()
        };
        let media = item.get("media").cloned().unwrap_or(Value::Null);
        let thumb_path = media["thumbnailPath"].as_str().or(media["path"].as_str()).unwrap_or("").to_string();
        let event_id = item["eventId"].as_str().unwrap_or("").to_string();
        // An image without a local file yet: ask for it; media.ready patches us.
        if kind == "image" && thumb_path.is_empty() && !event_id.is_empty() {
            ui.req.fire("media.get", json!({"roomId": room_id, "eventId": event_id, "thumbnail": {"width": 600, "height": 600}}));
        }
        // Files with readable previews render as a page card (QML docThumb).
        let doc_key = format!("{room_id}|{event_id}");
        let mut doc_lines: Vec<crate::DocLine> = Vec::new();
        let mut doc_chip = String::new();
        let mut doc_img: Option<slint::Image> = None;
        if kind == "file" && !event_id.is_empty() {
            match ui.doc_thumbs.get(&doc_key) {
                None => {
                    ui.doc_thumbs.insert(doc_key.clone(), Value::Null);
                    let req = ui.req.clone();
                    let key2 = doc_key.clone();
                    crate::actions::fetch_doc_thumb(&req, &room_id, &event_id, key2);
                }
                Some(v) if !v.is_null() => {
                    // Lines arrive structured: {"t":"p","text"} or {"t":"row","cells"}.
                    doc_lines = v["lines"].as_array().map(|a| {
                        a.iter().take(6).filter_map(|l| {
                            let text = match l["t"].as_str() {
                                Some("row") => l["cells"].as_array().map(|c| {
                                    c.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" · ")
                                }).unwrap_or_default(),
                                _ => l["text"].as_str().unwrap_or("").to_string(),
                            };
                            if text.is_empty() { return None; }
                            Some(crate::DocLine {
                                text: text.into(),
                                level: l["level"].as_i64().unwrap_or(0) as i32,
                            })
                        }).collect()
                    }).unwrap_or_default();
                    doc_chip = v["chip"].as_str()
                        .or(v["kind"].as_str())
                        .unwrap_or("").to_uppercase();
                    let ipath = v["imagePath"].as_str().unwrap_or("").to_string();
                    if !ipath.is_empty() {
                        doc_img = avatar(ui, &ipath);
                    }
                    if doc_chip.is_empty() && (!doc_lines.is_empty() || doc_img.is_some()) {
                        doc_chip = "DOC".into();
                    }
                }
                _ => {}
            }
        }
        // Fenced code splits the body into parts; the highlighter's markup is
        // flattened to plain text (Slint has no rich text — WIRING-chat.md).
        let parts: Vec<crate::MsgPart> = item["parts"].as_array().map(|a| a.iter().map(|p| {
            let t = p["t"].as_str().unwrap_or("text");
            let raw = p["html"].as_str().or(p["text"].as_str()).unwrap_or("");
            crate::MsgPart {
                t: t.into(),
                body: rows::strip_markup(raw).into(),
                lang: p["lang"].as_str().unwrap_or("").into(),
            }
        }).filter(|p: &crate::MsgPart| !p.body.is_empty()).collect()).unwrap_or_default();
        // og: card for the first link in a text body.
        let mut link_data = Value::Null;
        let mut link_url = String::new();
        if (kind == "text" || kind == "notice") && item["parts"].as_array().map(|a| a.is_empty()).unwrap_or(true) {
            if let Some(url) = rows::first_url(&body) {
                link_url = url.clone();
                match ui.link_previews.get(&url) {
                    None => {
                        ui.link_previews.insert(url.clone(), Value::Null);
                        crate::actions::fetch_link_preview(&ui.req.clone(), url);
                    }
                    Some(v) if v.is_object() => link_data = v.clone(),
                    _ => {}
                }
            }
        }
        let link_has = link_data.is_object()
            && (!link_data["title"].as_str().unwrap_or("").is_empty()
                || !link_data["imagePath"].as_str().unwrap_or("").is_empty());
        let link_only = link_has && body.trim() == link_url;
        let link_img = if link_has { avatar(ui, link_data["imagePath"].as_str().unwrap_or("")) } else { None };
        let link_accent = link_data["accent"].as_str()
            .and_then(|h| u32::from_str_radix(h.trim_start_matches('#'), 16).ok())
            .map(|c| slint::Color::from_rgb_u8((c >> 16) as u8, (c >> 8) as u8, c as u8))
            .unwrap_or(slint::Color::from_argb_u8(115, 0, 0, 0));
        // Music files render AudioBody's card: cover art + a strip tinted from
        // the art's palette (audio.info hands both over).
        let mut audio_art: Option<slint::Image> = None;
        let mut audio_tone: Option<slint::Color> = None;
        if kind == "audio" && !event_id.is_empty() {
            let akey = format!("{room_id}|{event_id}");
            match ui.audio_infos.get(&akey) {
                None => {
                    ui.audio_infos.insert(akey.clone(), Value::Null);
                    let req = ui.req.clone();
                    crate::actions::fetch_audio_info(&req, &room_id, &event_id, akey);
                }
                Some(v) if !v.is_null() => {
                    let art = v["artPath"].as_str().unwrap_or("").to_string();
                    if !art.is_empty() {
                        audio_art = avatar(ui, &art);
                    }
                    if let Some(hex) = ui.audio_infos.get(&format!("{room_id}|{event_id}")).and_then(|v| v["accent"].as_str()) {
                        if let Ok(c) = u32::from_str_radix(hex.trim_start_matches('#'), 16) {
                            audio_tone = Some(slint::Color::from_rgb_u8((c >> 16) as u8, (c >> 8) as u8, c as u8));
                        }
                    }
                }
                _ => {}
            }
        }
        // White or near-black over the strip, whichever reads (AudioBody labelInk).
        let audio_ink = audio_tone.map(|t| {
            let lum = 0.299 * t.red() as f32 / 255.0 + 0.587 * t.green() as f32 / 255.0 + 0.114 * t.blue() as f32 / 255.0;
            if lum > 0.62 { slint::Color::from_rgb_u8(23, 23, 23) } else { slint::Color::from_argb_u8(245, 255, 255, 255) }
        });
        let waveform: Vec<f64> = media["waveform"].as_array().map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
        let duration_ms = media["duration"].as_f64().unwrap_or(0.0);
        let reply = item.get("replyTo").cloned().filter(|r| !r.is_null());
        let reactions: Vec<crate::ReactionChip> = item["reactions"].as_array().map(|a| a.iter().map(|r| {
            let senders: Vec<&str> = r["senders"].as_array().map(|s2| s2.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            crate::ReactionChip {
                key: r["key"].as_str().unwrap_or("").into(),
                count: r["count"].as_i64().unwrap_or(senders.len() as i64) as i32,
                mine: senders.contains(&my_user.as_str()),
                senders_text: senders.join(", ").into(),
            }
        }).collect()).unwrap_or_default();
        let read_count = item["readBy"].as_array().map(|a| a.iter().filter(|r| r["userId"].as_str() != Some(my_user.as_str())).count()).unwrap_or(0);
        // The mark line draws reader faces, not a count (MarkStack). Only the
        // own message that wears the receipt needs them (readers = ownsReceipt ? … : []).
        let mut readers: Vec<crate::ReaderRow> = Vec::new();
        if let Some(rb) = item["readBy"].as_array().filter(|_| is_own && !event_id.is_empty() && event_id == receipt_owner) {
            for r in rb.iter().filter(|r| r["userId"].as_str() != Some(my_user.as_str())) {
                let rname = r["displayName"].as_str().unwrap_or("");
                readers.push(crate::ReaderRow {
                    avatar: avatar(ui, r["avatarPath"].as_str().unwrap_or("")).unwrap_or_default(),
                    initials: rows::initials(rname).into(),
                    tint: rows::tint_for(r["userId"].as_str().unwrap_or("")),
                });
            }
        }
        let poll = item.get("poll").cloned().unwrap_or(Value::Null);
        // The engine speaks MSC3381: `answers`, `voters`, `maxSelections`.
        let ended = poll["ended"].as_bool().unwrap_or(false);
        let top_votes = poll["answers"].as_array().map(|a| {
            a.iter().map(|o| o["votes"].as_i64().unwrap_or(0)).max().unwrap_or(0)
        }).unwrap_or(0);
        let poll_options: Vec<crate::PollOption> = poll["answers"].as_array().map(|a| a.iter().map(|o| {
            let votes = o["votes"].as_i64().unwrap_or(0);
            crate::PollOption {
                id: o["id"].as_str().unwrap_or("").into(),
                text: o["text"].as_str().unwrap_or("").into(),
                votes: votes as i32,
                mine: o["mine"].as_bool().unwrap_or(false),
                winner: ended && votes > 0 && votes == top_votes,
            }
        }).collect()).unwrap_or_default();
        let contact = item.get("contact").cloned().unwrap_or(Value::Null);
        let location = item.get("location").cloned().unwrap_or(Value::Null);
        let live = item.get("liveShare").cloned().unwrap_or(Value::Null);
        rows_out.push(TimelineRow {
            id: item["id"].as_str().unwrap_or("").into(),
            event_id: event_id.clone().into(),
            kind: kind.into(),
            body: body.into(),
            sender: sender.clone().into(),
            sender_name: sender_name.clone().into(),
            show_header: bubble && group_start && !is_own && !is_dm && !sender_name.is_empty(),
            initials: rows::initials(&sender_name).into(),
            tint: rows::tint_for(&sender),
            avatar: avatar(ui, item["senderAvatarPath"].as_str().unwrap_or("")).unwrap_or_default(),
            show_avatar: bubble && group_end && !is_own && !is_dm,
            is_own,
            group_start,
            group_end,
            stamp: rows::bubble_stamp(ts).into(),
            day_label: day_label.into(),
            edited: item["isEdited"].as_bool().unwrap_or(false),
            highlighted: item["isHighlighted"].as_bool().unwrap_or(false),
            send_state: item["sendState"].as_str().unwrap_or("sent").into(),
            send_error: item["sendError"].as_str().unwrap_or("").into(),
            read_count: read_count as i32,
            media_icon,
            thumb: avatar(ui, &thumb_path).unwrap_or_default(),
            thumb_w: media["width"].as_f64().unwrap_or(0.0) as f32,
            thumb_h: media["height"].as_f64().unwrap_or(0.0) as f32,
            media_filename: media["filename"].as_str().unwrap_or("").into(),
            media_size: media["sizeLabel"].as_str().unwrap_or("").into(),
            duration: if duration_ms > 0.0 { format!("{}:{:02}", (duration_ms as u64 / 1000) / 60, (duration_ms as u64 / 1000) % 60) } else { String::new() }.into(),
            reply: reply.as_ref().map(|r| crate::ReplyRef {
                event_id: r["eventId"].as_str().unwrap_or("").into(),
                sender_name: r["senderName"].as_str().unwrap_or("").into(),
                kind: r["kind"].as_str().unwrap_or("").into(),
                body: r["body"].as_str().unwrap_or("").into(),
                tint: rows::tint_for(r["sender"].as_str().unwrap_or("")),
            }).unwrap_or_default(),
            has_reply: reply.is_some(),
            reactions: slint::ModelRc::new(VecModel::from(reactions)),
            thread_root: item["threadRoot"].as_str().unwrap_or("").into(),
            thread_count: item["threadSummary"]["count"].as_i64().unwrap_or(0) as i32,
            thread_latest: item["threadSummary"]["body"].as_str().unwrap_or("").into(),
            parts: slint::ModelRc::new(VecModel::from(parts)),
            link_has,
            link_only,
            link_title: link_data["title"].as_str().unwrap_or("").into(),
            link_desc: link_data["description"].as_str().unwrap_or("").into(),
            link_domain: rows::domain_of(&link_url).into(),
            link_initial: rows::domain_of(&link_url).chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default().into(),
            link_url: link_url.clone().into(),
            link_iw: link_data["imageWidth"].as_f64().unwrap_or(0.0) as f32,
            link_ih: link_data["imageHeight"].as_f64().unwrap_or(0.0) as f32,
            link_is_video: link_data["isVideo"].as_bool().unwrap_or(false),
            link_accent,
            link_img: link_img.unwrap_or_default(),
            can_edit: item["can"]["edit"].as_bool().unwrap_or(false),
            can_reply: item["can"]["reply"].as_bool().unwrap_or(false),
            can_redact: item["can"]["redact"].as_bool().unwrap_or(false),
            can_react: item["can"]["react"].as_bool().unwrap_or(false),
            utd: item["kind"].as_str() == Some("utd"),
            waveform: slint::ModelRc::new(VecModel::from(rows::resample_wave(&waveform, 28))),
            voice_playing: ui.audio_playing && ui.audio_ctx.1 == event_id,
            voice_frac: if duration_ms > 0.0 { (ui.voice_positions.get(&event_id).copied().unwrap_or(0.0) * 1000.0 / duration_ms) as f32 } else { 0.0 },
            poll_question: poll["question"].as_str().unwrap_or("").into(),
            poll_options: slint::ModelRc::new(VecModel::from(poll_options)),
            poll_total: poll["voters"].as_i64().unwrap_or(0) as i32,
            poll_ended: poll["ended"].as_bool().unwrap_or(false),
            poll_multi: poll["maxSelections"].as_i64().unwrap_or(1) > 1,
            poll_max: poll["maxSelections"].as_i64().unwrap_or(1).max(1) as i32,
            poll_disclosed: poll["disclosed"].as_bool().unwrap_or(false),
            poll_vote_sum: poll["answers"].as_array().map(|a| a.iter().map(|o| o["votes"].as_i64().unwrap_or(0)).sum::<i64>()).unwrap_or(0) as i32,
            contact_name: contact["displayName"].as_str().unwrap_or("").into(),
            contact_id: contact["userId"].as_str().unwrap_or("").into(),
            contact_initials: rows::initials(contact["displayName"].as_str().unwrap_or("")).into(),
            contact_tint: rows::tint_for(contact["userId"].as_str().unwrap_or("")),
            location_label: location["description"].as_str().unwrap_or("").into(),
            location_live: live["live"].as_bool().unwrap_or(false),
            location_expires_s: (live["expiresAt"].as_f64().unwrap_or(0.0) / 1000.0) as i32,
            location_ended: item["kind"].as_str() == Some("liveLocation") && !live["live"].as_bool().unwrap_or(false),
            audio_have_art: audio_art.is_some(),
            audio_art: audio_art.unwrap_or_default(),
            audio_tone: audio_tone.unwrap_or(slint::Color::from_rgb_u8(0xa8, 0xa8, 0xa8)),
            audio_ink: audio_ink.unwrap_or(slint::Color::from_argb_u8(245, 255, 255, 255)),
            doc_lines: slint::ModelRc::new(VecModel::from(doc_lines)),
            doc_chip: doc_chip.into(),
            has_doc_image: doc_img.is_some(),
            doc_thumb: doc_img.unwrap_or_default(),
            is_read_marker: false,
            owns_receipt: is_own && !event_id.is_empty() && event_id == receipt_owner,
            receipt_count: read_count as i32,
            readers: slint::ModelRc::new(VecModel::from(readers)),
            pinned: pinned_ids.iter().any(|p| p == &event_id),
            html_ish: false,
        });
    }
    ui.items_model.set_vec(rows_out);
    win.set_items(ModelRc::from(ui.items_model.clone()));
}

fn start_demo(win: &AppWindow, rt: &tokio::runtime::Runtime, icons: IconSet) -> Requester {
    let engine = Engine::new(Hub::new());
    let req = Requester { handle: rt.handle().clone(), engine };
    win.set_boot_epoch_s(chrono::Utc::now().timestamp() as i32);
    let state = Rc::new(RefCell::new(UiState {
        win: win.as_weak(),
        req: req.clone(),
        icons,
        rooms_json: Vec::new(),
        search: String::new(),
        open_room: String::new(),
        shadow: Vec::new(),
        typing: HashMap::new(),
        my_user: "@pell:demo.host".into(),
        login_url: String::new(),
        avatars: HashMap::new(),
        items_model: std::rc::Rc::new(VecModel::default()),
        typing_sent: false,
        spaces_tree: Vec::new(),
        receipts_by_room: HashMap::new(),
        drafts: HashMap::new(),
        presence_by_user: Value::Null,
        pinned_by_room: HashMap::new(),
        pagination_by_room: HashMap::new(),
        call: Value::Null,
        devices: Value::Null,
        voice_level: 0.0,
        space_hierarchy: Vec::new(),
        spacerooms_mode: "manage".into(),
        spacerooms_selected: Default::default(),
        members_filter: -1,
        members: Vec::new(),
        settings_room: String::new(),
        settings: Value::Null,
        search_query: String::new(),
        forward_query: String::new(),
        forward_item: Value::Null,
        start_query_epoch: 0,
        dir_query_epoch: 0,
        doc_ctx: Default::default(),
        audio_ctx: Default::default(),
        audio_playing: false,
        new_space_avatar: String::new(),
        sheet_item: Value::Null,
        emojis: Vec::new(),
        voice_positions: HashMap::new(),
        chat_themes: serde_json::json!({}),
        viewer_items: Vec::new(),
        doc_pages: Vec::new(),
        stickers: Vec::new(),
        voice_clip: Value::Null,
        recording: false,
        rec_levels: Vec::new(),
        theme_pending: serde_json::json!({}),
        doc_preview: Value::Null,
        doc_thumbs: HashMap::new(),
        audio_infos: HashMap::new(),
        link_previews: HashMap::new(),
    }));
    UI.with(|ui| *ui.borrow_mut() = Some(state));

    let now = chrono::Utc::now().timestamp_millis();
    let hour = 3_600_000_i64;
    let day = 24 * hour;
    let room = |id: &str, name: &str, dm: bool, enc: bool, unread: i64, hl: i64, fav: bool, ts: i64, stamp: &str, kind: &str, sender: &str, body: &str| {
        json!({"id": id, "name": name, "isDm": dm, "dmUserId": if dm { format!("@{}:demo.host", id.trim_start_matches('!')) } else { String::new() },
               "isEncrypted": enc, "isFavourite": fav, "unread": unread, "highlights": hl, "joinedMembers": 7,
               "lastActivityTs": ts, "stamp": stamp,
               "lastMessage": {"kind": kind, "senderName": sender, "body": body}})
    };
    let rooms = json!({"event": "rooms.list", "loaded": true, "rooms": [
        room("!ladyofthelake", "LadyoftheLake", true, true, 2, 0, true, now - 600_000, "14:02", "text", "LadyoftheLake", "the sword is yours, but there is a condition"),
        room("!johnwick", "John Wick", true, true, 0, 1, true, now - 2 * hour, "12:41", "image", "John Wick", "Photo"),
        room("!ideas", "Ideas", false, true, 0, 0, true, now - 5 * hour, "09:15", "text", "morgana", "what if the bar icon pulsed on mention"),
        room("!uptime", "Uptime Alerts", false, false, 12, 0, false, now - day, "Yesterday", "text", "pellinore", "hi"),
        room("!sigiltest", "Sigil Test", false, true, 0, 0, false, now - day - 3 * hour, "Yesterday", "voice", "pellinore", "Voice message"),
        room("!godfrey", "Godfrey of Bouillon", true, true, 0, 0, false, now - 3 * day, "Mon", "text", "Godfrey", "deus vult, obviously"),
        room("!brains", "Element X Brain Trust", false, true, 0, 0, false, now - 6 * day, "Fri", "text", "kit", "fn main() but in a chat message"),
    ]});
    let item = |id: &str, kind: &str, own: bool, sender: &str, name: &str, body: &str, ts: i64, edited: bool| {
        json!({"id": id, "kind": kind, "isOwn": own, "sender": sender, "senderName": name, "body": body, "ts": ts, "isEdited": edited})
    };
    let timeline = json!({"event": "timeline.reset", "roomId": "!ladyofthelake", "items": [
        json!({"id": "d0", "kind": "dayDivider", "ts": now - day}),
        item("m1", "text", false, "@lady:demo.host", "LadyoftheLake", "so I have been thinking about the lake", now - day + 2 * hour, false),
        item("m2", "text", false, "@lady:demo.host", "LadyoftheLake", "it is less a body of water and more a jurisdiction", now - day + 2 * hour + 60_000, false),
        item("m3", "text", true, "@pell:demo.host", "pellinore", "strange women lying in ponds distributing swords is no basis for a system of government", now - day + 2 * hour + 150_000, true),
        json!({"id": "s1", "kind": "membership", "stateText": "Godfrey of Bouillon joined the room", "ts": now - day + 3 * hour}),
        json!({"id": "d1", "kind": "dayDivider", "ts": now}),
        item("m4", "text", false, "@lady:demo.host", "LadyoftheLake", "the sword is yours, but there is a condition", now - 600_000, false),
        json!({"id": "m4c", "kind": "text", "isOwn": false, "sender": "@lady:demo.host", "senderName": "LadyoftheLake",
               "body": "terms:\nfn wield(sword: Sword) -> Kingdom", "ts": now - 580_000,
               "parts": [
                   {"t": "text", "html": "terms:"},
                   {"t": "code", "lang": "rust", "html": "fn wield(sword: Sword) -&gt; Kingdom {\n    sword.raise()\n}"}
               ]}),
        item("m5", "text", true, "@pell:demo.host", "pellinore", "there is always a condition", now - 540_000, false),
        // ---- fixture block: every body kind the port renders ----
        json!({"id": "f3", "kind": "location", "isOwn": false, "sender": "@lady:demo.host", "senderName": "LadyoftheLake",
               "body": "Live location", "ts": now - 90_000, "eventId": "$f3",
               "liveShare": {"live": true, "expiresAt": now + 83_000}}),
        json!({"id": "f4", "kind": "contact", "isOwn": false, "sender": "@lady:demo.host", "senderName": "LadyoftheLake",
               "body": "Contact", "ts": now - 60_000, "eventId": "$f4",
               "contact": {"displayName": "Godfrey of Bouillon", "userId": "@godfrey:demo.host"}}),
        json!({"id": "f5", "kind": "audio", "isOwn": false, "sender": "@lady:demo.host", "senderName": "LadyoftheLake",
               "body": "lake_sounds.mp3", "ts": now - 380_000, "eventId": "$f5",
               "media": {"filename": "lake_sounds.mp3", "duration": 154_000.0}}),
        json!({"id": "f1", "kind": "poll", "isOwn": false, "sender": "@lady:demo.host", "senderName": "LadyoftheLake",
               "body": "Which toolkit?", "ts": now - 160_000, "eventId": "$f1",
               "poll": {"question": "Which toolkit?", "ended": false, "disclosed": true, "voters": 3, "maxSelections": 1,
                        "answers": [
                            {"id": "a1", "text": "Slint", "votes": 2, "mine": true},
                            {"id": "a2", "text": "QML", "votes": 1, "mine": false},
                            {"id": "a3", "text": "Both", "votes": 0, "mine": false}]}}),
        json!({"id": "f2", "kind": "voice", "isOwn": false, "sender": "@lady:demo.host", "senderName": "LadyoftheLake",
               "body": "Voice message", "ts": now - 120_000, "eventId": "$f2",
               "media": {"duration": 7000.0, "waveform": [0.2,0.5,0.9,0.4,0.7,1.0,0.6,0.3,0.8,0.5,0.2,0.6,0.9,0.4,0.3,0.7,0.5,0.8,0.2,0.4]}}),
        json!({"id": "f7", "kind": "text", "isOwn": true, "sender": "@pell:demo.host", "senderName": "pellinore",
               "body": "did this go through", "ts": now - 200_000, "sendState": "failed",
               "replyTo": {"eventId": "$f1", "senderName": "LadyoftheLake", "kind": "text", "body": "the sword is yours, but there is a condition"}}),
        json!({"id": "f6", "kind": "text", "isOwn": true, "sender": "@pell:demo.host", "senderName": "pellinore",
               "body": "read this yet?", "ts": now - 300_000, "eventId": "$f6", "sendState": "sent",
               "reactions": [{"key": "👍", "count": 1, "senders": ["@lady:demo.host"]},
                             {"key": "❤️", "count": 2, "senders": ["@pell:demo.host", "@lady:demo.host"]}],
               "threadSummary": {"count": 2, "senderName": "LadyoftheLake", "body": "in the thread we discuss tides"},
               "readBy": [{"userId": "@lady:demo.host", "displayName": "LadyoftheLake", "avatarPath": ""},
                          {"userId": "@godfrey:demo.host", "displayName": "Godfrey of Bouillon", "avatarPath": ""}]}),
    ], "len": 15});
    let status = json!({"event": "status", "session": "loggedIn", "userId": "@pell:demo.host",
                        "displayName": "Pellinore", "avatarPath": "", "sync": "", "syncError": "", "login": {"url": ""}});
    let recovery = json!({"event": "recovery.status",
        "verified": std::env::var_os("SIGIL_SLINT_DEMO_RECOVERY").is_none()});

    let events = vec![status, recovery, rooms, timeline];
    let _ = slint::invoke_from_event_loop(move || {
        with_ui(|ui| {
            // Open the room before its reset arrives, as a real open would.
            let chat = std::env::var_os("SIGIL_SLINT_DEMO_CHAT").is_some();
            if chat {
                ui.open_room = "!ladyofthelake".into();
            }
            for ev in &events {
                handle_event(ui, ev);
            }
            if chat {
                if let Some(win) = ui.win.upgrade() {
                    set_chat_header(ui, &win);
                    win.set_nav("chat".into());
                }
            }
        });
    });
    req
}
