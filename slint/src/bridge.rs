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
pub use sigil_engine::ipc::wire::Reply as EngineReply;
use sigil_engine::ipc::wire::{Reply, Request};

use crate::rows::{self, IconSet};
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

    /// Fire and forget, except that a refusal is worth a line in the log.
    pub fn fire(&self, req: &str, params: Value) {
        let name = req.to_string();
        self.call(req, params, move |reply| {
            if let Reply::Err(e) = reply {
                tracing::warn!("{name}: {} {}", e.code, e.message);
            }
        });
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
    /// The server the doors were shown for; the username's second half.
    pub door_server: String,
    /// The Envoy the doors reach the server through: the address the user
    /// typed, as a WebSocket URL. Normally wss://<server>/envoy; a test
    /// server on loopback is where it differs.
    pub door_envoy: String,
    /// the server's identity provider, when registration is gated by one
    pub door_oidc_issuer: String,
    pub door_oidc_client: String,
    /// Set while an account is being created with a password: the recovery
    /// code page opens the moment the session comes up.
    pub show_code_on_login: bool,
    /// Avatar images by path; a cache because rooms.list re-arrives constantly.
    pub avatars: HashMap<String, slint::Image>,
    /// THE timeline model. Mutated in place: handing the ListView a fresh
    /// model on every diff resets the viewport, which reads as "cannot
    /// scroll" the moment receipts start flowing.
    pub items_model: std::rc::Rc<VecModel<TimelineRow>>,
    /// Whether the last typing notice we sent said "typing".
    pub typing_sent: bool,
    // ---- Service.qml parity ----
    pub receipts_by_room: HashMap<String, Vec<Value>>,
    /// roomId -> unsent composer text (Panel.qml drafts).
    pub drafts: HashMap<String, String>,
    pub pinned_by_room: HashMap<String, Vec<String>>,
    /// The person a contact card or member sheet is about: (user id, name).
    pub contact_ctx: (String, String),
    pub pagination_by_room: HashMap<String, String>,
    pub call: Value,
    pub devices: Value,
    pub voice_level: f32,
    // ---- per-page working state ----
    pub members_filter: i64,
    pub members: Vec<Value>,   // room.members of settings_room
    pub settings_room: String, // roomId the settings pages serve
    pub settings: Value,       // last room.settings reply
    pub search_query: String,
    pub forward_query: String,
    pub forward_item: Value, // staged message for forward
    pub start_query_epoch: u64,
    pub dir_query_epoch: u64,
    pub doc_ctx: (String, String), // roomId, eventId the doc page shows
    pub audio_ctx: (String, String),
    pub audio_playing: bool,
    pub sheet_item: Value,             // message the action sheet targets
    pub emojis: Vec<(String, String)>, // glyph, keywords
    pub voice_positions: HashMap<String, f64>, // eventId -> seconds (playback)
    pub chat_themes: Value,
    pub viewer_items: Vec<Value>, // timeline items behind the viewer pager
    pub doc_pages: Vec<Value>,    // doc.page results by index
    pub stickers: Vec<Value>,
    pub voice_clip: Value, // voice.stop reply (path/duration/waveform)
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
    /// glyph -> emoji.render reply (Null = asked, false = none).
    pub emoji_imgs: HashMap<String, Value>,
    /// The call in progress, the incoming one, and every call announced.
    pub calls: crate::call::Calls,
    /// A re-render is already scheduled for arriving emoji pictures.
    pub emoji_refresh_pending: bool,
    /// The picker's last query, to re-run it when pictures arrive.
    pub emoji_query: Option<String>,
    /// Ids appended live since the last reset, still to play their entry.
    pub entry_pending: std::collections::HashSet<String>,
    /// When the current timeline was reset; entries animate only after it settled.
    pub reset_at: Option<std::time::Instant>,
    /// roomId|eventId -> media.gifFrames reply (Null = pending; false = not animated).
    pub gif_frames: HashMap<String, Value>,
    /// geoUri -> location.map reply (Null = pending; false = failed).
    pub location_maps: HashMap<String, Value>,
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
    let req = Requester {
        handle: rt.handle().clone(),
        engine: engine.clone(),
    };

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
        door_server: String::new(),
        door_envoy: String::new(),
        door_oidc_issuer: String::new(),
        door_oidc_client: String::new(),
        show_code_on_login: false,
        avatars: HashMap::new(),
        items_model: std::rc::Rc::new(VecModel::default()),
        typing_sent: false,
        receipts_by_room: HashMap::new(),
        drafts: HashMap::new(),
        pinned_by_room: HashMap::new(),
        contact_ctx: Default::default(),
        pagination_by_room: HashMap::new(),
        call: Value::Null,
        devices: Value::Null,
        voice_level: 0.0,
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
        emoji_imgs: HashMap::new(),
        calls: Default::default(),
        emoji_refresh_pending: false,
        emoji_query: None,
        entry_pending: Default::default(),
        reset_at: None,
        gif_frames: HashMap::new(),
        location_maps: HashMap::new(),
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
            let Ok(v) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let _ = slint::invoke_from_event_loop(move || with_ui(|ui| handle_event(ui, &v)));
        }
    });
    {
        let engine = engine.clone();
        rt.spawn(async move { engine.startup().await });
    }
    wire_callbacks(win, req.clone());
    crate::actions::wire_extra(win);
    // the sheet's quick reactions as pictures, once the engine is up
    crate::actions::after_pub(&req, 400, |ui, win| {
        crate::actions::refresh_emoji_views(ui, win)
    });
    req
}

/// UI actions → engine requests. Every handler runs on the UI thread; the
/// Requester hops to the runtime.
fn wire_callbacks(win: &AppWindow, req: Requester) {
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
                ui.req
                    .fire("typing", json!({"roomId": ui.open_room, "typing": false}));
                ui.typing_sent = false;
            }
            ui.req.fire(
                "ui.focus",
                json!({"roomId": ui.open_room, "visible": false}),
            );
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
                ui.req
                    .fire("typing", json!({"roomId": ui.open_room, "typing": false}));
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
                ui.req
                    .fire("typing", json!({"roomId": ui.open_room, "typing": now}));
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
    win.on_sign_out({
        let req = req.clone();
        move || req.fire("logout", json!({"wipe": true}))
    });
    wire_doors(win, req.clone());
    wire_settings(win, req);
}

/// Reply → UI thread, for handlers that want to touch the window afterwards.
pub fn on_ui(f: impl FnOnce(&mut UiState, &AppWindow) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || {
        with_ui(|ui| {
            if let Some(win) = ui.win.upgrade() {
                f(ui, &win);
            }
        })
    });
}

fn door_fail(win: &AppWindow, msg: &str) {
    win.set_door_busy(false);
    win.set_door_error(msg.into());
}

fn door_busy(busy: bool) {
    with_ui(|ui| {
        if let Some(win) = ui.win.upgrade() {
            win.set_door_busy(busy);
            win.set_door_error(SharedString::new());
        }
    });
}

fn full_username(localpart: &str) -> String {
    let server = with_ui_get(|ui| ui.door_server.clone());
    format!(
        "@{}:{server}",
        localpart.trim().trim_start_matches('@').to_lowercase()
    )
}

/// The doors: server first, then create, restore, or link.
fn wire_doors(win: &AppWindow, req: Requester) {
    win.on_door_probe({
        let req = req.clone();
        move |server| {
            let typed = server.trim().trim_end_matches('/').to_string();
            let base = if typed.contains("://") {
                typed.clone()
            } else {
                format!("https://{typed}")
            };
            let envoy = format!("{}/envoy", base.replacen("http", "ws", 1));
            let server = typed
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .to_string();
            with_ui(|ui| {
                ui.door_server = server.clone();
                ui.door_envoy = envoy;
            });
            door_busy(true);
            // the name as typed: the engine resolves it (a pointer on the bare
            // domain may move it); a typed scheme is a test server, taken as is
            req.call("account.probe", json!({"server": typed}), |reply| {
                on_ui(move |ui, win| {
                    win.set_door_busy(false);
                    match reply {
                        Reply::Ok(v) => {
                            win.set_door_registration(
                                v["registration"].as_str().unwrap_or("invite").into(),
                            );
                            win.set_door_oidc_name(v["oidc"]["name"].as_str().unwrap_or("").into());
                            win.set_door_oidc_state(SharedString::new());
                            win.set_door_oidc_user(SharedString::new());
                            ui.door_oidc_issuer =
                                v["oidc"]["issuer"].as_str().unwrap_or("").to_string();
                            ui.door_oidc_client =
                                v["oidc"]["clientId"].as_str().unwrap_or("").to_string();
                            win.set_door_tpm(v["tpm"].as_bool().unwrap_or(false));
                            if let Some(h) = v["hostname"].as_str() {
                                win.set_door_server(h.into());
                                ui.door_server = h.to_string();
                            }
                            // the name may point elsewhere (a pointer on the bare domain)
                            if let Some(e) = v["envoy"].as_str() {
                                ui.door_envoy = e.to_string();
                            }
                            win.set_door("choose".into());
                        }
                        Reply::Err(e) => win.set_door_error(
                            format!("Could not reach that server: {}", e.message).into(),
                        ),
                    }
                })
            });
        }
    });
    win.on_door_create({
        let req = req.clone();
        move |name, invite, password| {
            let has_pw = !password.is_empty();
            let username = full_username(&name);
            with_ui(|ui| ui.show_code_on_login = has_pw);
            door_busy(true);
            let envoy = with_ui_get(|ui| ui.door_envoy.clone());
            let mut p = json!({"username": username, "invite": invite.trim(), "envoy": envoy});
            if has_pw {
                p["password"] = json!(password.as_str());
            }
            req.call("account.create", p, |reply| {
                on_ui(move |ui, win| {
                    if let Reply::Err(e) = reply {
                        ui.show_code_on_login = false;
                        door_fail(win, &e.message);
                    }
                    // success arrives as a status event with session loggedIn
                })
            });
        }
    });
    win.on_door_oidc_start({
        let req = req.clone();
        move || {
            let (server, issuer, client) = with_ui_get(|ui| {
                (
                    ui.door_server.clone(),
                    ui.door_oidc_issuer.clone(),
                    ui.door_oidc_client.clone(),
                )
            });
            on_ui(|_, win| {
                win.set_door_error(SharedString::new());
                win.set_door_oidc_state("waiting".into());
            });
            req.call(
                "account.oidcStart",
                json!({"server": server, "issuer": issuer, "clientId": client}),
                |reply| {
                    on_ui(move |_ui, win| match reply {
                        Reply::Ok(v) => {
                            if let Some(url) = v["url"].as_str() {
                                crate::platform::open_url(url);
                            }
                        }
                        Reply::Err(e) => {
                            win.set_door_oidc_state("failed".into());
                            win.set_door_error(e.message.into());
                        }
                    })
                },
            );
        }
    });
    win.on_door_recover({
        let req = req.clone();
        move |name, password, code| {
            let username = full_username(&name);
            door_busy(true);
            req.call(
                "account.recover",
                json!({"username": username, "password": password.as_str(), "code": code.trim(),
                       "envoy": with_ui_get(|ui| ui.door_envoy.clone())}),
                |reply| {
                    on_ui(move |_ui, win| {
                        if let Reply::Err(e) = reply {
                            door_fail(win, &e.message);
                        }
                    })
                },
            );
        }
    });
    win.on_door_link_start({
        let req = req.clone();
        move |name| {
            let username = full_username(&name);
            door_busy(true);
            let envoy = with_ui_get(|ui| ui.door_envoy.clone());
            req.call(
                "link.offer",
                json!({"username": username, "envoy": envoy}),
                |reply| {
                    on_ui(move |_ui, win| {
                        win.set_door_busy(false);
                        match reply {
                            Reply::Ok(v) => {
                                let offer = v["offer"].as_str().unwrap_or("").to_string();
                                if let Some(img) = crate::qr::image(&offer) {
                                    win.set_link_image(img);
                                }
                                win.set_link_offer(offer.as_str().into());
                                win.set_link_state("offer".into());
                            }
                            Reply::Err(e) => door_fail(win, &e.message),
                        }
                    })
                },
            );
        }
    });
    win.on_door_link_cancel(|| {
        // The offer slot expires on its own; the new device just stops
        // showing the code.
        with_ui(|ui| {
            if let Some(win) = ui.win.upgrade() {
                win.set_link_state(SharedString::new());
                win.set_link_offer(SharedString::new());
                win.set_link_sas(SharedString::new());
                win.set_door_error(SharedString::new());
            }
        });
    });
}

/// Settings: recovery, the privacy shape, notifications, linking from the
/// signed-in side.
fn wire_settings(win: &AppWindow, req: Requester) {
    win.on_recovery_copy(|| {
        with_ui(|ui| {
            if let Some(win) = ui.win.upgrade() {
                let code = win.get_recovery_code().to_string();
                if !code.is_empty() {
                    crate::platform::copy_text(&code);
                }
            }
        });
    });
    win.on_recovery_done(|| {});
    win.on_set_password({
        let req = req.clone();
        move |pw| {
            with_ui(|ui| {
                if let Some(win) = ui.win.upgrade() {
                    win.set_password_busy(true);
                    win.set_password_error(SharedString::new());
                }
            });
            let first = with_ui_get(|ui| {
                ui.win
                    .upgrade()
                    .map(|w| w.get_recovery_state() != "enabled")
                    .unwrap_or(true)
            });
            let req2 = req.clone();
            req.call(
                "account.setPassword",
                json!({"password": pw.as_str()}),
                move |reply| {
                    match reply {
                        Reply::Ok(_) => {
                            // a first password means a first code: show it once
                            req2.call("recovery.code", json!({}), move |r| {
                                on_ui(move |_ui, win| {
                                    win.set_password_busy(false);
                                    win.set_password_open(false);
                                    if let Reply::Ok(v) = r {
                                        win.set_recovery_code(
                                            v["code"].as_str().unwrap_or("").into(),
                                        );
                                        if first {
                                            win.set_recovery_first_time(true);
                                            win.set_recovery_open(true);
                                        }
                                    }
                                })
                            });
                        }
                        Reply::Err(e) => on_ui(move |_ui, win| {
                            win.set_password_busy(false);
                            win.set_password_error(e.message.as_str().into());
                        }),
                    }
                },
            );
        }
    });
    win.on_set_clocked({
        let req = req.clone();
        move |n| {
            req.call("shape.settings", json!({"clockedSeconds": n}), |reply| {
                on_ui(move |_ui, win| {
                    if let Reply::Ok(v) = reply {
                        apply_shape(win, &v);
                    }
                })
            });
        }
    });
    win.on_set_proxy({
        let req = req.clone();
        move |p| {
            req.call("shape.settings", json!({"socksProxy": p.trim()}), |reply| {
                on_ui(move |_ui, win| {
                    if let Reply::Ok(v) = reply {
                        apply_shape(win, &v);
                    }
                })
            });
        }
    });
    win.on_set_previews({
        let req = req.clone();
        move |on| {
            req.call("shape.settings", json!({"linkPreviews": on}), |reply| {
                on_ui(move |ui, win| {
                    if let Reply::Ok(v) = reply {
                        apply_shape(win, &v);
                        // cards asked for while previews were off are asked again
                        ui.link_previews.clear();
                        rebuild_timeline(ui, win);
                    }
                })
            });
        }
    });
    win.on_set_notify({
        let req = req.clone();
        move |key, on| {
            req.call("notify.settings", json!({key.as_str(): on}), |reply| {
                on_ui(move |_ui, win| {
                    if let Reply::Ok(v) = reply {
                        apply_notify(win, &v);
                    }
                })
            });
        }
    });
    win.on_link_scan({
        let req = req.clone();
        move |offer| {
            with_ui(|ui| {
                if let Some(win) = ui.win.upgrade() {
                    win.set_settings_busy(true);
                    win.set_scan_error(SharedString::new());
                }
            });
            req.call("link.scan", json!({"offer": offer.trim()}), |reply| {
                on_ui(move |_ui, win| {
                    win.set_settings_busy(false);
                    match reply {
                        Reply::Ok(v) => {
                            win.set_scan_sas(v["sas"].as_str().unwrap_or("").into());
                            win.set_scan_state("sas".into());
                        }
                        Reply::Err(e) => {
                            win.set_scan_state("failed".into());
                            win.set_scan_error(e.message.as_str().into());
                        }
                    }
                })
            });
        }
    });
    win.on_link_confirm({
        let req = req.clone();
        move |ok| {
            with_ui(|ui| {
                if let Some(win) = ui.win.upgrade() {
                    win.set_scan_state(if ok { "joining" } else { "" }.into());
                }
            });
            req.call("link.confirm", json!({"ok": ok}), move |reply| {
                on_ui(move |_ui, win| {
                    if let Reply::Err(e) = reply {
                        win.set_scan_state("failed".into());
                        win.set_scan_error(e.message.as_str().into());
                    }
                    // the rest arrives as link.state events
                })
            });
        }
    });
}

fn with_ui_get<T: Default>(f: impl FnOnce(&mut UiState) -> T) -> T {
    let mut out = T::default();
    with_ui(|ui| out = f(ui));
    out
}

pub fn apply_shape(win: &AppWindow, v: &Value) {
    win.set_shape_clocked(v["clockedSeconds"].as_i64().unwrap_or(0) as i32);
    win.set_shape_proxy(v["socksProxy"].as_str().unwrap_or("").into());
    win.set_shape_previews(v["linkPreviews"].as_bool().unwrap_or(false));
}

pub fn apply_notify(win: &AppWindow, v: &Value) {
    win.set_notify_enabled(v["enabled"].as_bool().unwrap_or(true));
    win.set_notify_dms(v["dms"].as_bool().unwrap_or(true));
    win.set_notify_mentions(v["mentions"].as_bool().unwrap_or(true));
    win.set_notify_calls(v["calls"].as_bool().unwrap_or(true));
}

/// Everything the settings page shows, asked for when it opens.
pub fn load_settings_page(ui: &mut UiState) {
    let req = ui.req.clone();
    req.call("shape.settings", json!({}), |reply| {
        on_ui(move |_ui, win| {
            if let Reply::Ok(v) = reply {
                apply_shape(win, &v);
            }
        })
    });
    req.call("notify.settings", json!({}), |reply| {
        on_ui(move |_ui, win| {
            if let Reply::Ok(v) = reply {
                apply_notify(win, &v);
            }
        })
    });
    req.call("recovery.code", json!({}), |reply| {
        on_ui(move |_ui, win| {
            if let Reply::Ok(v) = reply {
                win.set_recovery_code(v["code"].as_str().unwrap_or("").into());
                win.set_recovery_first_time(false);
            }
        })
    });
    if let Some(win) = ui.win.upgrade() {
        win.set_scan_state(SharedString::new());
        win.set_scan_error(SharedString::new());
        win.set_app_version(env!("CARGO_PKG_VERSION").into());
    }
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
                win.set_requests(ModelRc::new(VecModel::from(Vec::<RoomRow>::new())));
                win.set_items(ModelRc::new(VecModel::from(Vec::<TimelineRow>::new())));
                win.set_nav("home".into());
                win.set_rooms_loaded(false);
                win.set_recovery_open(false);
                win.set_password_open(false);
                win.set_my_avatar(Default::default());
                win.set_door("server".into());
            }
            if !was_in && session == "loggedIn" {
                win.set_door_busy(false);
                win.set_link_state(SharedString::new());
                if ui.show_code_on_login {
                    ui.show_code_on_login = false;
                    ui.req.call("recovery.code", json!({}), |reply| {
                        on_ui(move |_ui, win| {
                            if let Reply::Ok(v) = reply {
                                win.set_recovery_code(v["code"].as_str().unwrap_or("").into());
                                win.set_recovery_first_time(true);
                                win.set_recovery_open(true);
                            }
                        })
                    });
                }
            }
            ui.my_user = v["userId"].as_str().unwrap_or("").to_string();
            win.set_my_user_id(ui.my_user.as_str().into());
            let shown = match v["displayName"].as_str() {
                Some(d) if !d.is_empty() => d.to_string(),
                _ => rows::localpart(&ui.my_user),
            };
            win.set_my_name(shown.as_str().into());
            if let Some(server) = ui.my_user.split_once(':').map(|(_, s)| s.to_string()) {
                ui.door_server = server.clone();
                win.set_door_server(server.as_str().into());
            }
            win.set_my_initials(rows::initials(&shown).into());
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
        "recovery.status" => {
            win.set_recovery_state(v["recovery"].as_str().unwrap_or("disabled").into());
            win.set_backup_state(v["backup"].as_str().unwrap_or("disabled").into());
        }
        // The link exchange, seen from either side. The new device sits on
        // the doors page; the signed-in device sits in Settings.
        // The sign-in at the server's provider, started from the create door.
        "oidc.state" => {
            let state = v["state"].as_str().unwrap_or("");
            win.set_door_oidc_state(state.into());
            match state {
                "done" => win.set_door_oidc_user(v["name"].as_str().unwrap_or("").into()),
                "failed" => {
                    win.set_door_error(v["error"].as_str().unwrap_or("the sign-in failed").into())
                }
                _ => {}
            }
        }
        "link.state" => {
            let state = v["state"].as_str().unwrap_or("");
            if win.get_session() == "loggedIn" {
                win.set_scan_state(state.into());
                if state == "sas" {
                    win.set_scan_sas(v["sas"].as_str().unwrap_or("").into());
                }
                if state == "failed" {
                    win.set_scan_error(v["error"].as_str().unwrap_or("linking failed").into());
                }
            } else {
                win.set_link_state(state.into());
                match state {
                    "offer" => {
                        let offer = v["offer"].as_str().unwrap_or("").to_string();
                        if let Some(img) = crate::qr::image(&offer) {
                            win.set_link_image(img);
                        }
                        win.set_link_offer(offer.as_str().into());
                    }
                    "sas" => win.set_link_sas(v["sas"].as_str().unwrap_or("").into()),
                    "failed" => {
                        win.set_link_state(SharedString::new());
                        win.set_door_error(v["error"].as_str().unwrap_or("linking failed").into());
                    }
                    _ => {}
                }
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
        "room.receipts" => {
            let room = v["roomId"].as_str().unwrap_or("").to_string();
            ui.receipts_by_room.insert(
                room.clone(),
                v["users"].as_array().cloned().unwrap_or_default(),
            );
            if room == ui.open_room {
                rebuild_timeline(ui, &win);
            }
        }
        "position" => {
            crate::actions::apply_position(&win, &v);
        }
        "room.pinned" => {
            let room = v["roomId"].as_str().unwrap_or("").to_string();
            let ids: Vec<String> = v["events"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|e| e.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
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
            crate::call::on_engine_call_state(ui, &win, &v);
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
            ui.reset_at = Some(std::time::Instant::now());
            ui.entry_pending.clear();
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
            // ChatPage.claimEntry: a message appended once the view has settled
            // plays its entry animation, once.
            let settled = ui
                .reset_at
                .map(|t| t.elapsed() > std::time::Duration::from_millis(600))
                .unwrap_or(false);
            for op in v["ops"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
                if settled && op["op"].as_str() == Some("pushBack") {
                    if let Some(id) = op["item"]["id"].as_str() {
                        ui.entry_pending.insert(id.to_string());
                    }
                }
                grew_tail |= apply_diff(&mut ui.shadow, op);
            }
            // QML's atYEnd rule: stick to the bottom only when the reader is
            // there; otherwise hold their distance from it across the rebuild
            // (prepends from pagination included).
            let from_end = win.get_chat_from_end();
            rebuild_timeline(ui, &win);
            if grew_tail && from_end < 8.0 {
                win.invoke_scroll_timeline_to_end();
                ui.req.fire(
                    "room.markRead",
                    json!({"roomId": crate::actions::room_of_key(&ui.open_room)}),
                );
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
    match ui
        .typing
        .get(&ui.open_room)
        .map(|u| u.as_slice())
        .unwrap_or(&[])
    {
        [] => String::new(),
        [one] => format!(
            "{} is typing…",
            one["displayName"].as_str().unwrap_or("Someone")
        ),
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
        if text.trim().is_empty() {
            ui.drafts.remove(&leaving);
        } else {
            ui.drafts.insert(leaving, text);
        }
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
    ui.req.fire(
        "room.open",
        json!({"roomId": ui.open_room, "initialItems": 80}),
    );
    ui.req.fire(
        "ui.focus",
        json!({"roomId": crate::actions::room_of_key(&ui.open_room), "visible": true}),
    );
    ui.req.fire(
        "room.markRead",
        json!({"roomId": crate::actions::room_of_key(&ui.open_room)}),
    );
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
    let preview = rows::preview_for(room, &typing, &ui.icons);
    let (badge, badge_urgent) = rows::badge_for(room);
    let unread = room["unread"]
        .as_i64()
        .unwrap_or(0)
        .max(room["unreadMessages"].as_i64().unwrap_or(0))
        > 0;
    RoomRow {
        id: id.clone().into(),
        is_dm,
        topic: room["topic"].as_str().unwrap_or("").into(),
        member_count: room["joinedMembers"].as_i64().unwrap_or(0) as i32,
        is_low_priority: room["isLowPriority"].as_bool().unwrap_or(false),
        name: name.clone().into(),
        initials: rows::initials(&name).into(),
        avatar: avatar(ui, room["avatarPath"].as_str().unwrap_or("")).unwrap_or_default(),
        tint: rows::tint_for(&tint_key),
        preview: preview.text.into(),
        preview_icon: preview.icon,
        stamp: match room["stamp"].as_str().unwrap_or("") {
            "" => rows::home_stamp(room["lastActivityTs"].as_i64().unwrap_or(0)).into(),
            st => st.into(),
        },
        badge: badge.into(),
        badge_urgent,
        unread,
        is_favourite: room["isFavourite"].as_bool().unwrap_or(false),
        is_encrypted: room["isEncrypted"].as_bool().unwrap_or(false),
        has_call: room["hasActiveCall"].as_bool().unwrap_or(false),
        is_invite: room["isInvite"].as_bool().unwrap_or(false),
        is_typing: preview.typing,
        // A draft outranks the last message unless someone is typing (QML
        // showDraft); invites keep their invitation line.
        draft: if typing.is_empty() && !room["isInvite"].as_bool().unwrap_or(false) {
            ui.drafts
                .get(&id)
                .map(|d| d.trim().to_string())
                .unwrap_or_default()
                .into()
        } else {
            SharedString::new()
        },
    }
}

pub fn rebuild_rooms(ui: &mut UiState, win: &AppWindow) {
    let q = ui.search.to_lowercase();
    let mut chats: Vec<RoomRow> = Vec::new();
    let mut requests: Vec<RoomRow> = Vec::new();
    let mut rooms_json = std::mem::take(&mut ui.rooms_json);
    // HomePage.qml's order, re-applied here as it was there: pinned first,
    // then highlights, then unread, then most recent activity.
    rooms_json.sort_by(|a, b| {
        let key = |r: &Value| {
            let unread = r["unread"]
                .as_i64()
                .unwrap_or(0)
                .max(r["unreadMessages"].as_i64().unwrap_or(0))
                > 0;
            (
                !r["isFavourite"].as_bool().unwrap_or(false),
                !(r["highlights"].as_i64().unwrap_or(0) > 0),
                !unread,
                -r["lastActivityTs"].as_i64().unwrap_or(0),
            )
        };
        key(a).cmp(&key(b))
    });
    for room in &rooms_json {
        let name = room["name"].as_str().unwrap_or("");
        if !q.is_empty() && !name.to_lowercase().contains(&q) {
            continue;
        }
        let row = room_row_of(ui, room);
        // A request (someone new wrote to us) lives on its own tab until
        // accepted; the Home rows are the same shape either way.
        if room["isInvite"].as_bool().unwrap_or(false) {
            requests.push(row);
        } else {
            chats.push(row);
        }
    }
    ui.rooms_json = rooms_json;
    win.set_rooms(ModelRc::new(VecModel::from(chats)));
    win.set_requests(ModelRc::new(VecModel::from(requests)));
    if !ui.open_room.is_empty() {
        set_chat_header(ui, win);
    }
}

pub fn set_chat_header(ui: &mut UiState, win: &AppWindow) {
    let Some(room) = ui
        .rooms_json
        .iter()
        .find(|r| r["id"].as_str() == Some(ui.open_room.as_str()))
        .cloned()
    else {
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
    let room = ui
        .rooms_json
        .iter()
        .find(|r| r["id"].as_str() == Some(room_id.as_str()))
        .cloned();
    let is_dm = room
        .as_ref()
        .and_then(|r| r["isDm"].as_bool())
        .unwrap_or(false);
    let pinned_ids = ui.pinned_by_room.get(&room_id).cloned().unwrap_or_default();
    let my_user = ui.my_user.clone();
    // The newest own message with an event id wears the sent/read mark.
    let receipt_owner = ui
        .shadow
        .iter()
        .rev()
        .find(|i| {
            i["isOwn"].as_bool().unwrap_or(false)
                && i["eventId"]
                    .as_str()
                    .map(|e| !e.is_empty())
                    .unwrap_or(false)
        })
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
                rows_out.push(TimelineRow {
                    id: item["id"].as_str().unwrap_or("").into(),
                    kind: "readMarker".into(),
                    is_read_marker: true,
                    ..Default::default()
                });
                continue;
            }
            rows::RowShape::Divider(label) => ("dayDivider", label, Default::default()),
            rows::RowShape::State(text) => ("state", text, Default::default()),
            rows::RowShape::Bubble {
                media_icon,
                body_override,
            } => {
                let body = body_override
                    .unwrap_or_else(|| item["body"].as_str().unwrap_or("").to_string());
                let k = match item["kind"].as_str().unwrap_or("text") {
                    k @ ("text" | "notice" | "emote" | "image" | "video" | "voice" | "audio"
                    | "file" | "sticker" | "poll" | "contact" | "location" | "redacted"
                    | "utd" | "unsupported") => k,
                    "liveLocation" => "location",
                    _ => "text",
                };
                (k, body, media_icon)
            }
        };
        let is_own = item["isOwn"].as_bool().unwrap_or(false);
        let sender = item["sender"].as_str().unwrap_or("").to_string();
        let sender_name = match item["senderName"].as_str().unwrap_or("") {
            "" => sender.clone(),
            d => d.to_string(),
        };
        let group_start = !i
            .checked_sub(1)
            .map(|p| rows::same_group(&shadow[p], item))
            .unwrap_or(false);
        let group_end = !shadow
            .get(i + 1)
            .map(|nx| rows::same_group(item, nx))
            .unwrap_or(false);
        let bubble = kind != "dayDivider" && kind != "state";
        let ts = item["ts"].as_i64().unwrap_or(0);
        // Session stamp above the row: day changed against the older
        // neighbour, or more than an hour passed (Service.recomputeGrouping).
        let day_label = if bubble && ts > 0 {
            let older = i
                .checked_sub(1)
                .and_then(|p| shadow.get(p))
                .and_then(|o| o["ts"].as_i64())
                .filter(|t| *t > 0);
            match older {
                Some(ot) if rows::same_day(ts, ot) && ts - ot <= 3_600_000 => String::new(),
                _ => crate::project::session_label(ts),
            }
        } else {
            String::new()
        };
        let media = item.get("media").cloned().unwrap_or(Value::Null);
        let thumb_path = media["thumbnailPath"]
            .as_str()
            .or(media["path"].as_str())
            .unwrap_or("")
            .to_string();
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
                    doc_lines = v["lines"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .take(6)
                                .filter_map(|l| {
                                    let text = match l["t"].as_str() {
                                        Some("row") => l["cells"]
                                            .as_array()
                                            .map(|c| {
                                                c.iter()
                                                    .filter_map(Value::as_str)
                                                    .collect::<Vec<_>>()
                                                    .join(" · ")
                                            })
                                            .unwrap_or_default(),
                                        _ => l["text"].as_str().unwrap_or("").to_string(),
                                    };
                                    if text.is_empty() {
                                        return None;
                                    }
                                    Some(crate::DocLine {
                                        text: text.into(),
                                        level: l["level"].as_i64().unwrap_or(0) as i32,
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    doc_chip = v["chip"]
                        .as_str()
                        .or(v["kind"].as_str())
                        .unwrap_or("")
                        .to_uppercase();
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
        // An animated GIF cycles engine-decoded frames (no animated Image).
        let mut gif_imgs: Vec<slint::Image> = Vec::new();
        let mut gif_delays: Vec<i32> = Vec::new();
        if kind == "image" && !event_id.is_empty() {
            let mime = media["mime"].as_str().unwrap_or("");
            let fname = media["filename"]
                .as_str()
                .unwrap_or("")
                .to_ascii_lowercase();
            if mime.contains("gif") || fname.ends_with(".gif") {
                let gkey = format!("{room_id}|{event_id}");
                match ui.gif_frames.get(&gkey).cloned() {
                    None => {
                        ui.gif_frames.insert(gkey.clone(), Value::Null);
                        crate::actions::fetch_gif_frames(
                            &ui.req.clone(),
                            &room_id,
                            &event_id,
                            gkey,
                        );
                    }
                    Some(v) if v.is_object() => {
                        let paths: Vec<String> = v["frames"]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .filter_map(Value::as_str)
                                    .map(str::to_string)
                                    .collect()
                            })
                            .unwrap_or_default();
                        for path in &paths {
                            if let Some(img) = avatar(ui, path) {
                                gif_imgs.push(img);
                            }
                        }
                        gif_delays = v["delays"]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .filter_map(Value::as_i64)
                                    .map(|d| d as i32)
                                    .collect()
                            })
                            .unwrap_or_default();
                        if gif_imgs.len() != gif_delays.len() {
                            gif_imgs.clear();
                            gif_delays.clear();
                        }
                    }
                    _ => {}
                }
            }
        }
        // Fenced code splits the body into parts; the highlighter's markup is
        // flattened to plain text (Slint has no rich text — WIRING-chat.md).
        let parts: Vec<crate::MsgPart> = item["parts"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|p| {
                        let t = p["t"].as_str().unwrap_or("text");
                        let raw = p["html"].as_str().or(p["text"].as_str()).unwrap_or("");
                        crate::MsgPart {
                            t: t.into(),
                            body: rows::strip_markup(raw).into(),
                            lang: p["lang"].as_str().unwrap_or("").into(),
                        }
                    })
                    .filter(|p: &crate::MsgPart| !p.body.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        // og: card for the first link in a text body.
        let mut link_data = Value::Null;
        let mut link_url = String::new();
        if (kind == "text" || kind == "notice")
            && item["parts"]
                .as_array()
                .map(|a| a.is_empty())
                .unwrap_or(true)
        {
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
        let link_img = if link_has {
            avatar(ui, link_data["imagePath"].as_str().unwrap_or(""))
        } else {
            None
        };
        let link_accent = link_data["accent"]
            .as_str()
            .and_then(|h| u32::from_str_radix(h.trim_start_matches('#'), 16).ok())
            .map(|c| slint::Color::from_rgb_u8((c >> 16) as u8, (c >> 8) as u8, c as u8))
            .unwrap_or(slint::Color::from_argb_u8(115, 0, 0, 0));
        // Markup renders through StyledText, parsed here — the language-side
        // @markdown interpolates runtime strings as PLAIN text by design.
        // Animated short runs render per glyph; colours-only rides StyledText.
        // SigilText: a body with effects is laid out glyph by glyph in Rust
        // (fx.rs), wrapped to the bubble's width, and drawn one Text per glyph.
        let (fx_chars, fx_w, fx_h, fx_spoiler): (Vec<crate::FxChar>, f32, f32, bool) =
            if matches!(kind, "text" | "notice" | "emote") {
                let tl_w = win.get_timeline_w() as f32;
                let tl_w = if tl_w > 0.0 {
                    tl_w
                } else {
                    crate::headless::WIDTH as f32
                };
                let max_w = tl_w * 0.78 - 22.0;
                match crate::fx::layout(
                    &body,
                    &item["effects"],
                    chrono::Utc::now().timestamp_millis() - ts < 5_000,
                    12.0,
                    max_w,
                ) {
                    Some(lay) => (
                        lay.glyphs
                            .into_iter()
                            .map(|g| {
                                let parsed = g.color.as_deref().and_then(rows::hex_color);
                                crate::FxChar {
                                    ch: g.ch.into(),
                                    has_color: parsed.is_some(),
                                    color: parsed
                                        .unwrap_or(slint::Color::from_rgb_u8(0xc6, 0xc6, 0xc6)),
                                    anim: g.anim.into(),
                                    idx: g.idx,
                                    x: g.x,
                                    y: g.y,
                                    w: g.w,
                                    size: g.size,
                                    bold: g.bold,
                                    italic: g.italic,
                                    underline: g.underline,
                                    strike: g.strike,
                                    mono: g.mono,
                                    mark: g.mark,
                                    mark_color: g
                                        .mark_color
                                        .as_deref()
                                        .and_then(rows::hex_color)
                                        .unwrap_or(slint::Color::from_rgb_u8(0xe8, 0xc8, 0x40)),
                                    spoiler: g.spoiler,
                                }
                            })
                            .collect(),
                        lay.width,
                        lay.height,
                        lay.has_spoiler,
                    ),
                    None => (Vec::new(), 0.0, 0.0, false),
                }
            } else {
                (Vec::new(), 0.0, 0.0, false)
            };
        let rich_body: Option<slint::StyledText> = if matches!(kind, "text" | "notice" | "emote")
            && item["parts"]
                .as_array()
                .map(|a| a.is_empty())
                .unwrap_or(true)
        {
            // SigilText colour effects outrank plain markup for the body.
            rows::effects_markdown(&body, &item["effects"])
                .or_else(|| {
                    item["html"]
                        .as_str()
                        .filter(|h| !h.is_empty())
                        .map(rows::html_to_markdown)
                })
                .and_then(|md| slint::StyledText::from_markdown(&md).ok())
        } else {
            None
        };
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
                    if let Some(hex) = ui
                        .audio_infos
                        .get(&format!("{room_id}|{event_id}"))
                        .and_then(|v| v["accent"].as_str())
                    {
                        if let Ok(c) = u32::from_str_radix(hex.trim_start_matches('#'), 16) {
                            audio_tone = Some(slint::Color::from_rgb_u8(
                                (c >> 16) as u8,
                                (c >> 8) as u8,
                                c as u8,
                            ));
                        }
                    }
                }
                _ => {}
            }
        }
        // White or near-black over the strip, whichever reads (AudioBody labelInk).
        let audio_ink = audio_tone.map(|t| {
            let lum = 0.299 * t.red() as f32 / 255.0
                + 0.587 * t.green() as f32 / 255.0
                + 0.114 * t.blue() as f32 / 255.0;
            if lum > 0.62 {
                slint::Color::from_rgb_u8(23, 23, 23)
            } else {
                slint::Color::from_argb_u8(245, 255, 255, 255)
            }
        });
        let waveform: Vec<f64> = media["waveform"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_f64).collect())
            .unwrap_or_default();
        let duration_ms = media["duration"].as_f64().unwrap_or(0.0);
        let reply = item.get("replyTo").cloned().filter(|r| !r.is_null());
        let reactions: Vec<crate::ReactionChip> = item["reactions"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|r| {
                        let senders: Vec<&str> = r["senders"]
                            .as_array()
                            .map(|s2| s2.iter().filter_map(Value::as_str).collect())
                            .unwrap_or_default();
                        let img = emoji_image(ui, r["key"].as_str().unwrap_or(""));
                        crate::ReactionChip {
                            key: r["key"].as_str().unwrap_or("").into(),
                            count: r["count"].as_i64().unwrap_or(senders.len() as i64) as i32,
                            mine: senders.contains(&my_user.as_str()),
                            senders_text: senders.join(", ").into(),
                            has_img: img.is_some(),
                            img: img.unwrap_or_default(),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let read_count = item["readBy"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter(|r| r["userId"].as_str() != Some(my_user.as_str()))
                    .count()
            })
            .unwrap_or(0);
        // The mark line draws reader faces, not a count (MarkStack). Only the
        // own message that wears the receipt needs them (readers = ownsReceipt ? … : []).
        let mut readers: Vec<crate::ReaderRow> = Vec::new();
        if let Some(rb) = item["readBy"]
            .as_array()
            .filter(|_| is_own && !event_id.is_empty() && event_id == receipt_owner)
        {
            for r in rb
                .iter()
                .filter(|r| r["userId"].as_str() != Some(my_user.as_str()))
            {
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
        let top_votes = poll["answers"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|o| o["votes"].as_i64().unwrap_or(0))
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        let poll_options: Vec<crate::PollOption> = poll["answers"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|o| {
                        let votes = o["votes"].as_i64().unwrap_or(0);
                        crate::PollOption {
                            id: o["id"].as_str().unwrap_or("").into(),
                            text: o["text"].as_str().unwrap_or("").into(),
                            votes: votes as i32,
                            mine: o["mine"].as_bool().unwrap_or(false),
                            winner: ended && votes > 0 && votes == top_votes,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let contact = item.get("contact").cloned().unwrap_or(Value::Null);
        let location = item.get("location").cloned().unwrap_or(Value::Null);
        let live = item.get("liveShare").cloned().unwrap_or(Value::Null);
        // The card shows an actual map: an engine-composited OSM tile crop.
        let mut location_map: Option<slint::Image> = None;
        if matches!(kind, "location" | "liveLocation") {
            let geo = location["geoUri"].as_str().unwrap_or("").to_string();
            if !geo.is_empty() {
                match ui.location_maps.get(&geo).cloned() {
                    None => {
                        ui.location_maps.insert(geo.clone(), Value::Null);
                        crate::actions::fetch_location_map(&ui.req.clone(), geo);
                    }
                    Some(v) if v.is_object() => {
                        location_map = avatar(ui, v["path"].as_str().unwrap_or(""));
                    }
                    _ => {}
                }
            }
        }
        rows_out.push(TimelineRow {
            id: item["id"].as_str().unwrap_or("").into(),
            event_id: event_id.clone().into(),
            kind: kind.into(),
            // Markup renders through StyledText; code-part messages lay out
            // as parts instead, and a plain body stays on the fast Text path.
            rich_body: rich_body.clone().unwrap_or_default(),
            has_rich: rich_body.is_some() && fx_chars.is_empty(),
            fx_chars: slint::ModelRc::new(VecModel::from(fx_chars)),
            fx_w,
            fx_h,
            fx_spoiler,
            is_new: ui.entry_pending.contains(item["id"].as_str().unwrap_or("")),
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
            // Only visual kinds load a thumbnail — a file's cached path is a
            // document, and Image::load on a .txt just prints an error.
            thumb: if matches!(kind, "image" | "video" | "sticker") {
                avatar(ui, &thumb_path).unwrap_or_default()
            } else {
                Default::default()
            },
            thumb_w: media["width"].as_f64().unwrap_or(0.0) as f32,
            thumb_h: media["height"].as_f64().unwrap_or(0.0) as f32,
            media_filename: media["filename"].as_str().unwrap_or("").into(),
            gif_frames: slint::ModelRc::new(VecModel::from(gif_imgs)),
            gif_delays: slint::ModelRc::new(VecModel::from(gif_delays)),
            media_size: media["sizeLabel"].as_str().unwrap_or("").into(),
            duration: if duration_ms > 0.0 {
                format!(
                    "{}:{:02}",
                    (duration_ms as u64 / 1000) / 60,
                    (duration_ms as u64 / 1000) % 60
                )
            } else {
                String::new()
            }
            .into(),
            reply: reply
                .as_ref()
                .map(|r| crate::ReplyRef {
                    event_id: r["eventId"].as_str().unwrap_or("").into(),
                    sender_name: r["senderName"].as_str().unwrap_or("").into(),
                    kind: r["kind"].as_str().unwrap_or("").into(),
                    body: r["body"].as_str().unwrap_or("").into(),
                    tint: rows::tint_for(r["sender"].as_str().unwrap_or("")),
                })
                .unwrap_or_default(),
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
            link_initial: rows::domain_of(&link_url)
                .chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_default()
                .into(),
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
            voice_frac: if duration_ms > 0.0 {
                (ui.voice_positions.get(&event_id).copied().unwrap_or(0.0) * 1000.0 / duration_ms)
                    as f32
            } else {
                0.0
            },
            poll_question: poll["question"].as_str().unwrap_or("").into(),
            poll_options: slint::ModelRc::new(VecModel::from(poll_options)),
            poll_total: poll["voters"].as_i64().unwrap_or(0) as i32,
            poll_ended: poll["ended"].as_bool().unwrap_or(false),
            poll_multi: poll["maxSelections"].as_i64().unwrap_or(1) > 1,
            poll_max: poll["maxSelections"].as_i64().unwrap_or(1).max(1) as i32,
            poll_disclosed: poll["disclosed"].as_bool().unwrap_or(false),
            poll_vote_sum: poll["answers"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|o| o["votes"].as_i64().unwrap_or(0))
                        .sum::<i64>()
                })
                .unwrap_or(0) as i32,
            contact_name: contact["displayName"].as_str().unwrap_or("").into(),
            contact_id: contact["userId"].as_str().unwrap_or("").into(),
            contact_initials: rows::initials(contact["displayName"].as_str().unwrap_or("")).into(),
            contact_tint: rows::tint_for(contact["userId"].as_str().unwrap_or("")),
            location_label: location["description"].as_str().unwrap_or("").into(),
            location_live: live["live"].as_bool().unwrap_or(false),
            location_expires_s: (live["expiresAt"].as_f64().unwrap_or(0.0) / 1000.0) as i32,
            location_map: location_map.unwrap_or_default(),
            location_ended: item["kind"].as_str() == Some("liveLocation")
                && !live["live"].as_bool().unwrap_or(false),
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
    // an entry plays once: the next rebuild sees these as settled
    ui.entry_pending.clear();
}

fn start_demo(win: &AppWindow, rt: &tokio::runtime::Runtime, icons: IconSet) -> Requester {
    let engine = Engine::new(Hub::new());
    let req = Requester {
        handle: rt.handle().clone(),
        engine,
    };
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
        my_user: "@wren:sigil.test".into(),
        door_server: String::new(),
        door_envoy: String::new(),
        door_oidc_issuer: String::new(),
        door_oidc_client: String::new(),
        show_code_on_login: false,
        avatars: HashMap::new(),
        items_model: std::rc::Rc::new(VecModel::default()),
        typing_sent: false,
        receipts_by_room: HashMap::new(),
        drafts: HashMap::new(),
        pinned_by_room: HashMap::new(),
        contact_ctx: Default::default(),
        pagination_by_room: HashMap::new(),
        call: Value::Null,
        devices: Value::Null,
        voice_level: 0.0,
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
        emoji_imgs: HashMap::new(),
        calls: Default::default(),
        emoji_refresh_pending: false,
        emoji_query: None,
        entry_pending: Default::default(),
        reset_at: None,
        gif_frames: HashMap::new(),
        location_maps: HashMap::new(),
    }));
    UI.with(|ui| *ui.borrow_mut() = Some(state));

    let now = chrono::Utc::now().timestamp_millis();
    let hour = 3_600_000_i64;
    let day = 24 * hour;
    let room = |id: &str,
                name: &str,
                dm: bool,
                enc: bool,
                unread: i64,
                hl: i64,
                fav: bool,
                ts: i64,
                stamp: &str,
                kind: &str,
                sender: &str,
                body: &str| {
        json!({"id": id, "name": name, "isDm": dm, "dmUserId": if dm { format!("@{}:sigil.test", id.trim_start_matches('!')) } else { String::new() },
               "isEncrypted": enc, "isFavourite": fav, "unread": unread, "highlights": hl, "joinedMembers": 7,
               "lastActivityTs": ts, "stamp": stamp,
               "lastMessage": {"kind": kind, "senderName": sender, "body": body}})
    };
    let rooms = json!({"event": "rooms.list", "loaded": true, "rooms": [
        room("!marlowe", "Marlowe", true, true, 2, 0, true, now - 600_000, "14:02", "text", "Marlowe", "the sword is yours, but there is a condition"),
        // two requests: strangers who wrote first, waiting on the Requests tab
        json!({"id": "req:7f3a9c02e1b4d6f8", "name": "merlin", "isDm": true, "dmUserId": "@merlin:camelot.example",
               "isEncrypted": true, "isInvite": true, "inviter": "@merlin:camelot.example", "unread": 1, "highlights": 1, "joinedMembers": 1,
               "lastActivityTs": now - 1_200_000, "stamp": "13:52",
               "lastMessage": {"kind": "invite", "sender": "@merlin:camelot.example", "body": "it is I, the wizard. we should talk about the sword"}}),
        json!({"id": "req:0c41be77a2d95e10", "name": "mordred", "isDm": true, "dmUserId": "@mordred:orkney.example",
               "isEncrypted": true, "isInvite": true, "inviter": "@mordred:orkney.example", "unread": 1, "highlights": 1, "joinedMembers": 1,
               "lastActivityTs": now - 4 * hour, "stamp": "10:10",
               "lastMessage": {"kind": "invite", "sender": "@mordred:orkney.example", "body": "Invitation"}}),
        room("!johnwick", "John Wick", true, true, 0, 1, true, now - 2 * hour, "12:41", "image", "John Wick", "Photo"),
        room("!ideas", "Ideas", false, true, 0, 0, true, now - 5 * hour, "09:15", "text", "morgana", "what if the bar icon pulsed on mention"),
        room("!uptime", "Uptime Alerts", false, false, 12, 0, false, now - day, "Yesterday", "text", "wren", "hi"),
        room("!sigiltest", "Sigil Test", false, true, 0, 0, false, now - day - 3 * hour, "Yesterday", "voice", "wren", "Voice message"),
        room("!godfrey", "Godfrey of Bouillon", true, true, 0, 0, false, now - 3 * day, "Mon", "text", "Godfrey", "deus vult, obviously"),
        room("!brains", "Element X Brain Trust", false, true, 0, 0, false, now - 6 * day, "Fri", "text", "kit", "fn main() but in a chat message"),
    ]});
    let item = |id: &str,
                kind: &str,
                own: bool,
                sender: &str,
                name: &str,
                body: &str,
                ts: i64,
                edited: bool| {
        json!({"id": id, "kind": kind, "isOwn": own, "sender": sender, "senderName": name, "body": body, "ts": ts, "isEdited": edited})
    };
    let timeline = json!({"event": "timeline.reset", "roomId": "!marlowe", "items": [
        json!({"id": "d0", "kind": "dayDivider", "ts": now - day}),
        item("m1", "text", false, "@marlowe:sigil.test", "Marlowe", "so I have been thinking about the lake", now - day + 2 * hour, false),
        item("m2", "text", false, "@marlowe:sigil.test", "Marlowe", "it is less a body of water and more a jurisdiction", now - day + 2 * hour + 60_000, false),
        item("m3", "text", true, "@wren:sigil.test", "wren", "strange women lying in ponds distributing swords is no basis for a system of government", now - day + 2 * hour + 150_000, true),
        json!({"id": "s1", "kind": "membership", "stateText": "Godfrey of Bouillon joined the room", "ts": now - day + 3 * hour}),
        json!({"id": "d1", "kind": "dayDivider", "ts": now}),
        item("m4", "text", false, "@marlowe:sigil.test", "Marlowe", "the sword is yours, but there is a condition", now - 600_000, false),
        json!({"id": "m4c", "kind": "text", "isOwn": false, "sender": "@marlowe:sigil.test", "senderName": "Marlowe",
               "body": "terms:\nfn wield(sword: Sword) -> Kingdom", "ts": now - 580_000,
               "parts": [
                   {"t": "text", "html": "terms:"},
                   {"t": "code", "lang": "rust", "html": "fn wield(sword: Sword) -&gt; Kingdom {\n    sword.raise()\n}"}
               ]}),
        item("m5", "text", true, "@wren:sigil.test", "wren", "there is always a condition", now - 540_000, false),
        // ---- fixture block: every body kind the port renders ----
        json!({"id": "f4", "kind": "contact", "isOwn": false, "sender": "@marlowe:sigil.test", "senderName": "Marlowe",
               "body": "Contact", "ts": now - 60_000, "eventId": "$f4",
               "contact": {"displayName": "Godfrey of Bouillon", "userId": "@godfrey:sigil.test"}}),
        json!({"id": "f5", "kind": "audio", "isOwn": false, "sender": "@marlowe:sigil.test", "senderName": "Marlowe",
               "body": "lake_sounds.mp3", "ts": now - 380_000, "eventId": "$f5",
               "media": {"filename": "lake_sounds.mp3", "duration": 154_000.0}}),
        json!({"id": "f1", "kind": "poll", "isOwn": false, "sender": "@marlowe:sigil.test", "senderName": "Marlowe",
               "body": "Which toolkit?", "ts": now - 160_000, "eventId": "$f1",
               "poll": {"question": "Which toolkit?", "ended": false, "disclosed": true, "voters": 3, "maxSelections": 1,
                        "answers": [
                            {"id": "a1", "text": "Slint", "votes": 2, "mine": true},
                            {"id": "a2", "text": "QML", "votes": 1, "mine": false},
                            {"id": "a3", "text": "Both", "votes": 0, "mine": false}]}}),
        json!({"id": "f2", "kind": "voice", "isOwn": false, "sender": "@marlowe:sigil.test", "senderName": "Marlowe",
               "body": "Voice message", "ts": now - 120_000, "eventId": "$f2",
               "media": {"duration": 7000.0, "waveform": [0.2,0.5,0.9,0.4,0.7,1.0,0.6,0.3,0.8,0.5,0.2,0.6,0.9,0.4,0.3,0.7,0.5,0.8,0.2,0.4]}}),
        json!({"id": "f7", "kind": "text", "isOwn": true, "sender": "@wren:sigil.test", "senderName": "wren",
               "body": "did this go through", "ts": now - 200_000, "sendState": "failed",
               "replyTo": {"eventId": "$f1", "senderName": "Marlowe", "kind": "text", "body": "the sword is yours, but there is a condition"}}),
        json!({"id": "f6", "kind": "text", "isOwn": true, "sender": "@wren:sigil.test", "senderName": "wren",
               "body": "read this yet?", "ts": now - 300_000, "eventId": "$f6", "sendState": "sent",
               "reactions": [{"key": "👍", "count": 1, "senders": ["@marlowe:sigil.test"]},
                             {"key": "❤️", "count": 2, "senders": ["@wren:sigil.test", "@marlowe:sigil.test"]}],
               "threadSummary": {"count": 2, "senderName": "Marlowe", "body": "in the thread we discuss tides"},
               "readBy": [{"userId": "@marlowe:sigil.test", "displayName": "Marlowe", "avatarPath": ""},
                          {"userId": "@godfrey:sigil.test", "displayName": "Godfrey of Bouillon", "avatarPath": ""}]}),
        json!({"id": "f8", "kind": "text", "isOwn": false, "sender": "@marlowe:sigil.test", "senderName": "Marlowe",
               "body": "bold and italic and code and a link", "ts": now - 30_000, "eventId": "$f8",
               "html": "the sword is <b>bold</b>, the lake is <i>italic</i>, the terms are <code>inline code</code>, and the map is <a href=\"https://slint.dev\">a tappable link</a>"}),
        json!({"id": "f3", "kind": "location", "isOwn": false, "sender": "@marlowe:sigil.test", "senderName": "Marlowe",
               "body": "Live location", "ts": now - 90_000, "eventId": "$f3",
               "location": {"geoUri": "geo:48.8583736,2.2944813"},
               "liveShare": {"live": true, "expiresAt": now + 83_000}}),
        json!({"id": "f9", "kind": "text", "isOwn": false, "sender": "@marlowe:sigil.test", "senderName": "Marlowe",
               "body": "solid red, a gradient, and a rainbow", "ts": now - 10_000, "eventId": "$f9",
               "effects": [
                   {"start": 0, "end": 9, "color": {"type": "solid", "rgb": {"dark": "#e06c75", "light": "#a03030"}}},
                   {"start": 11, "end": 21, "color": {"type": "gradient", "stops": [
                       {"dark": "#61afef", "light": "#2060a0"}, {"dark": "#c678dd", "light": "#803090"}]}},
                   {"start": 27, "end": 36, "color": {"type": "rainbow"}}
               ]}),
        json!({"id": "fA", "kind": "text", "isOwn": false, "sender": "@marlowe:sigil.test", "senderName": "Marlowe",
               "body": "Terms\nfirst\nsecond", "ts": now - 5_000, "eventId": "$fA",
               "html": "<h2>Terms</h2><ul><li>keep the <b>sword</b> dry</li><li>return it <i>eventually</i></li></ul><blockquote>the lake remembers</blockquote>"}),
        json!({"id": "fB", "kind": "text", "isOwn": false, "sender": "@marlowe:sigil.test", "senderName": "Marlowe",
               "body": "wave hello shake pulse!", "ts": now - 2_000, "eventId": "$fB",
               "effects": [
                   {"start": 0, "end": 10, "animation": "wave", "color": {"type": "solid", "rgb": {"dark": "#61afef", "light": "#2060a0"}}},
                   {"start": 11, "end": 16, "animation": "shake", "color": {"type": "solid", "rgb": {"dark": "#e06c75", "light": "#a03030"}}},
                   {"start": 17, "end": 23, "animation": "pulse", "color": {"type": "rainbow"}}
               ]}),
        json!({"id": "fC", "kind": "text", "isOwn": false, "sender": "@marlowe:sigil.test", "senderName": "Marlowe",
               "body": "glitch sparkle secret", "ts": now - 1_000, "eventId": "$fC",
               "effects": [
                   {"start": 0, "end": 6, "animation": "glitch"},
                   {"start": 7, "end": 14, "animation": "sparkle", "color": {"type": "solid", "rgb": {"dark": "#e5c07b", "light": "#906010"}}},
                   {"start": 15, "end": 21, "animation": "blur"}
               ]}),
    ], "len": 19});
    let status = json!({"event": "status", "session": "loggedIn", "userId": "@wren:sigil.test",
                        "displayName": "Wren", "avatarPath": "", "sync": "", "syncError": "", "login": {"url": ""}});
    let recovery = json!({"event": "recovery.status",
        "verified": std::env::var_os("SIGIL_SLINT_DEMO_RECOVERY").is_none()});

    let events = vec![status, recovery, rooms, timeline];
    let _ = slint::invoke_from_event_loop(move || {
        with_ui(|ui| {
            // Open the room before its reset arrives, as a real open would.
            let chat = std::env::var_os("SIGIL_SLINT_DEMO_CHAT").is_some();
            if chat {
                ui.open_room = "!marlowe".into();
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
    // The same wiring as the live path, so the demo (and the screenshot
    // harness on top of it) drives the real handlers; requests go to an
    // engine with no account and are answered with errors, which is fine.
    wire_callbacks(win, req.clone());
    crate::actions::wire_extra(win);
    req
}

/// An emoji as a picture, from the engine's colour font, asked for once;
/// None while it is pending or when the device has no such font (the text
/// glyph stands in). Replies re-render whatever shows emoji.
pub fn emoji_image(ui: &mut UiState, glyph: &str) -> Option<slint::Image> {
    if glyph.is_empty() {
        return None;
    }
    match ui.emoji_imgs.get(glyph) {
        Some(v) if v.is_object() => {
            let path = v["path"].as_str().unwrap_or("").to_string();
            avatar_pub(ui, &path)
        }
        Some(_) => None,
        None => {
            ui.emoji_imgs.insert(glyph.to_string(), Value::Null);
            let key = glyph.to_string();
            let req = ui.req.clone();
            req.call("emoji.render", json!({"text": glyph}), move |reply| {
                on_ui(move |ui, win| {
                    let val = match reply {
                        Reply::Ok(v) if !v["path"].as_str().unwrap_or("").is_empty() => v,
                        _ => json!(false),
                    };
                    ui.emoji_imgs.insert(key, val);
                    let _ = win;
                    // pictures arrive in a burst: one re-render for the lot
                    if !ui.emoji_refresh_pending {
                        ui.emoji_refresh_pending = true;
                        let req = ui.req.clone();
                        crate::actions::after_pub(&req, 120, |ui, win| {
                            ui.emoji_refresh_pending = false;
                            rebuild_timeline(ui, win);
                            crate::actions::refresh_emoji_views(ui, win);
                        });
                    }
                });
            });
            None
        }
    }
}
