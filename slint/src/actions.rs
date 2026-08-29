//! Service.qml's function table: what each page loads when it opens
//! (`nav-opened`) and what every page action does (`act`). The protocol names
//! come from core/docs/protocol.md; the behaviors from the WIRING-*.md
//! contracts and Service.qml itself.

use serde_json::{json, Value};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::bridge::{rebuild_rooms, rebuild_timeline, with_ui, Requester, UiState};
use crate::rows::{initials, tint_for};
use crate::{project, AppWindow, HierarchyRow, TimelineRow};

pub fn room_of_key(key: &str) -> String {
    match key.find('|') {
        Some(i) => key[..i].to_string(),
        None => key.to_string(),
    }
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str { v.get(k).and_then(Value::as_str).unwrap_or("") }
fn b(v: &Value, k: &str) -> bool { v.get(k).and_then(Value::as_bool).unwrap_or(false) }

/// Run an engine request; the continuation runs on the UI thread with state.
fn call_ui(
    req: &Requester,
    name: &str,
    params: Value,
    done: impl FnOnce(&mut UiState, &AppWindow, Result<Value, (String, String)>) + Send + 'static,
) {
    req.call(name, params, move |reply| {
        let out = match reply {
            sigil_engine::ipc::wire::Reply::Ok(v) => Ok(v),
            sigil_engine::ipc::wire::Reply::Err(e) => Err((e.code, e.message)),
        };
        let _ = slint::invoke_from_event_loop(move || {
            with_ui(|ui| {
                if let Some(win) = ui.win.upgrade() {
                    done(ui, &win, out);
                }
            });
        });
    });
}

/// Ask the engine for a document's bubble preview; the reply re-renders the
/// timeline through the cache (Service.docThumb).
pub fn fetch_doc_thumb(req: &Requester, room_id: &str, event_id: &str, key: String) {
    call_ui(req, "doc.thumb", json!({"roomId": room_id, "eventId": event_id, "size": 0}), move |ui, win, out| {
        let val = match out {
            Ok(v) if v["lines"].as_array().map(|a| !a.is_empty()).unwrap_or(false)
                || !v["imagePath"].as_str().unwrap_or("").is_empty() => v,
            _ => json!(false),
        };
        ui.doc_thumbs.insert(key, val);
        rebuild_timeline(ui, win);
    });
}

/// Public face of `after` for the bridge's layout-settle scrolls.
pub fn after_pub(req: &Requester, ms: u64, f: impl FnOnce(&mut UiState, &AppWindow) + Send + 'static) {
    after(req, ms, f);
}

/// Run `f` on the UI thread after `ms`.
fn after(req: &Requester, ms: u64, f: impl FnOnce(&mut UiState, &AppWindow) + Send + 'static) {
    req.handle().spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
        let _ = slint::invoke_from_event_loop(move || {
            with_ui(|ui| {
                if let Some(win) = ui.win.upgrade() {
                    f(ui, &win);
                }
            });
        });
    });
}

pub fn wire_extra(win: &AppWindow) {
    win.on_nav_opened(|page| {
        with_ui(|ui| {
            let Some(win) = ui.win.upgrade() else { return };
            on_nav_opened(ui, &win, page.as_str());
        });
    });
    win.on_act(|action, a, b| {
        with_ui(|ui| {
            let Some(win) = ui.win.upgrade() else { return };
            on_act(ui, &win, action.as_str(), a.as_str(), b.as_str());
        });
    });
}

// ---------------------------------------------------------------- loaders

pub fn on_nav_opened(ui: &mut UiState, win: &AppWindow, page: &str) {
    match page {
        "space" => load_space(ui, win),
        "spacerooms" => build_spacerooms(ui, win),
        "roomsettings" => {
            ui.settings_room = room_of_key(&ui.open_room);
            load_settings(ui, win);
            load_members(ui, win);
        }
        "notifications" | "security" | "roles" | "permissions" => {
            load_settings(ui, win);
        }
        "members" => load_members(ui, win),
        "spacesettings" => {
            ui.settings_room = win.get_space_id().to_string();
            load_settings(ui, win);
        }
        "threads" => load_threads(ui, win),
        "pins" => load_pins(ui, win),
        "search" => {
            ui.search_query.clear();
            rebuild_search(ui, win);
        }
        "forward" => {
            ui.forward_query.clear();
            rebuild_forward(ui, win);
        }
        "start" => {
            win.set_st_error(SharedString::new());
            rebuild_start_suggestions(ui, win);
        }
        "newspace" => {
            ui.new_space_avatar.clear();
            win.set_ns_has_avatar(false);
            win.set_ns_error(SharedString::new());
            win.set_ns_busy(false);
        }
        "addpeople" => win.set_ap_results(ModelRc::new(VecModel::from(Vec::new()))),
        _ => {}
    }
}

fn load_space(ui: &mut UiState, win: &AppWindow) {
    let sid = win.get_space_id().to_string();
    ui.settings_room = sid.clone();
    let tree = ui.spaces_tree.iter().find(|sp| s(sp, "id") == sid).cloned();
    let room = ui.rooms_json.iter().find(|r| s(r, "id") == sid).cloned();
    let name = tree.as_ref().map(|t| s(t, "name").to_string())
        .filter(|n| !n.is_empty())
        .or_else(|| room.as_ref().map(|r| s(r, "name").to_string()))
        .unwrap_or_else(|| sid.clone());
    win.set_sp_name(name.as_str().into());
    win.set_sp_initials(initials(&name).into());
    win.set_sp_tint(tint_for(&sid));
    if let Some(r) = &room {
        win.set_sp_member_count(r["joinedMembers"].as_i64().unwrap_or(0) as i32);
    }
    win.set_sp_loading(true);
    win.set_sp_loaded(false);
    call_ui(&ui.req.clone(), "room.settings", json!({"roomId": sid}), move |ui, win, out| {
        if let Ok(v) = out {
            win.set_sp_public(s(&v, "joinRule") == "public");
            ui.settings = v;
        }
    });
    load_hierarchy(ui, win, &sid);
}

fn load_hierarchy(ui: &mut UiState, _win: &AppWindow, sid: &str) {
    let sid = sid.to_string();
    call_ui(&ui.req.clone(), "space.hierarchy", json!({"spaceId": sid}), move |ui, win, out| {
        win.set_sp_loading(false);
        win.set_sp_loaded(true);
        match out {
            Ok(v) => {
                ui.space_hierarchy = v["rooms"].as_array().cloned().unwrap_or_default();
                let sid2 = win.get_space_id().to_string();
                let rows: Vec<HierarchyRow> = ui.space_hierarchy.iter()
                    .filter(|r| s(r, "id") != sid2)
                    .map(|r| project::hierarchy_row(r, false))
                    .collect();
                win.set_sp_children(ModelRc::new(VecModel::from(rows)));
                if win.get_nav() == "spacerooms" {
                    build_spacerooms(ui, win);
                }
            }
            Err(_) => win.set_sp_note("Could not load this space".into()),
        }
    });
}

fn build_spacerooms(ui: &mut UiState, win: &AppWindow) {
    let sid = win.get_space_id().to_string();
    win.set_sr_mode(ui.spacerooms_mode.as_str().into());
    let rows: Vec<HierarchyRow> = if ui.spacerooms_mode == "add" {
        let children: Vec<String> = ui.space_hierarchy.iter().map(|r| s(r, "id").to_string()).collect();
        ui.rooms_json.iter()
            .filter(|r| !b(r, "isSpace") && !b(r, "isInvite") && s(r, "id") != sid && !children.iter().any(|c| c == s(r, "id")))
            .map(|r| {
                let picked = ui.spacerooms_selected.contains(s(r, "id"));
                let mut row = project::hierarchy_row(r, picked);
                row.joined = true;
                row.member_count = r["joinedMembers"].as_i64().unwrap_or(0) as i32;
                row
            })
            .collect()
    } else {
        ui.space_hierarchy.iter()
            .filter(|r| s(r, "id") != sid)
            .map(|r| project::hierarchy_row(r, ui.spacerooms_selected.contains(s(r, "id"))))
            .collect()
    };
    win.set_sr_selected(ui.spacerooms_selected.len() as i32);
    win.set_sr_rows(ModelRc::new(VecModel::from(rows)));
}

fn load_settings(ui: &mut UiState, _win: &AppWindow) {
    if ui.settings_room.is_empty() {
        ui.settings_room = room_of_key(&ui.open_room);
    }
    let rid = ui.settings_room.clone();
    call_ui(&ui.req.clone(), "room.settings", json!({"roomId": rid}), move |ui, win, out| {
        let Ok(v) = out else { return };
        ui.settings = v;
        push_settings(ui, win);
    });
}

/// Everything derived from ui.settings, pushed to the pages at once.
pub fn push_settings(ui: &mut UiState, win: &AppWindow) {
    let rid = ui.settings_room.clone();
    let room = ui.rooms_json.iter().find(|r| s(r, "id") == rid).cloned().unwrap_or(Value::Null);
    let av = crate::bridge::avatar_pub(ui, room["avatarPath"].as_str().unwrap_or("")).unwrap_or_default();
    win.set_rs_model(project::settings_model(&rid, &ui.settings, &room, av));
    win.set_rs_spaces(ModelRc::new(VecModel::from(project::space_membership_rows(&ui.spaces_tree, &rid))));
    win.set_rs_pinned_count(ui.rooms_json.iter().filter(|r| b(r, "isFavourite")).count() as i32);
    win.set_rs_dm_user(s(&room, "dmUserId").into());
    win.set_no_mode(match s(&ui.settings, "notificationMode") { "default" => "", m => m }.into());
    let users = ui.settings["powerLevels"]["users"].as_object().cloned().unwrap_or_default();
    let admins = users.values().filter(|l| l.as_i64().unwrap_or(0) >= 100).count();
    let mods = users.values().filter(|l| { let n = l.as_i64().unwrap_or(0); (50..100).contains(&n) }).count();
    win.set_ro_admins(admins as i32);
    win.set_ro_mods(mods as i32);
    win.set_pe_member(ModelRc::new(VecModel::from(project::power_rows(&ui.settings, &[
        ("invite", "Invite people"), ("kick", "Remove people"), ("ban", "Ban people"),
    ]))));
    win.set_pe_detail(ModelRc::new(VecModel::from(project::power_rows(&ui.settings, &[
        ("name", "Change name"), ("avatar", "Change avatar"), ("topic", "Change topic"),
    ]))));
    win.set_pe_content(ModelRc::new(VecModel::from(project::power_rows(&ui.settings, &[
        ("eventsDefault", "Send messages"), ("redact", "Remove messages"), ("liveLocation", "Share live location"),
    ]))));
    let parent = ui.spaces_tree.iter().find(|sp| {
        sp["children"].as_array().map(|c| c.iter().any(|x| x.as_str() == Some(rid.as_str()))).unwrap_or(false)
    });
    win.set_sec_space_id(parent.map(|p| s(p, "id")).unwrap_or("").into());
    win.set_sec_space_name(parent.map(|p| s(p, "name")).unwrap_or("").into());
}

fn load_members(ui: &mut UiState, _win: &AppWindow) {
    if ui.settings_room.is_empty() {
        ui.settings_room = room_of_key(&ui.open_room);
    }
    let rid = ui.settings_room.clone();
    call_ui(&ui.req.clone(), "room.members", json!({"roomId": rid}), move |ui, win, out| {
        let Ok(v) = out else { return };
        ui.members = v["members"].as_array().cloned().unwrap_or_default();
        push_members(ui, win);
    });
}

pub fn push_members(ui: &mut UiState, win: &AppWindow) {
    let filter = ui.members_filter;
    win.set_me_all(ui.members.len() as i32);
    win.set_me_filter(filter as i32);
    let rows: Vec<_> = ui.members.iter()
        .filter(|m| m.get("membership").and_then(Value::as_str).unwrap_or("join") == "join")
        .filter(|m| {
            let l = m["powerLevel"].as_i64().unwrap_or(0);
            match filter {
                100 => l >= 100,
                50 => (50..100).contains(&l),
                _ => true,
            }
        })
        .map(|m| project::member_row(m, &ui.presence_by_user))
        .collect();
    win.set_me_members(ModelRc::new(VecModel::from(rows)));
    let preview: Vec<_> = ui.members.iter().take(8).map(|m| project::member_row(m, &ui.presence_by_user)).collect();
    win.set_rs_members(ModelRc::new(VecModel::from(preview)));
}

fn load_threads(ui: &mut UiState, win: &AppWindow) {
    let rid = room_of_key(&ui.open_room);
    win.set_th_loading(true);
    call_ui(&ui.req.clone(), "threads.list", json!({"roomId": rid}), move |_ui, win, out| {
        win.set_th_loading(false);
        win.set_th_loaded(true);
        if let Ok(v) = out {
            let rows: Vec<_> = v["threads"].as_array().map(|a| a.iter().map(project::thread_row).collect()).unwrap_or_default();
            win.set_th_threads(ModelRc::new(VecModel::from(rows)));
        }
    });
}

fn load_pins(ui: &mut UiState, win: &AppWindow) {
    let rid = room_of_key(&ui.open_room);
    let is_dm = ui.rooms_json.iter().find(|r| s(r, "id") == rid).map(|r| b(r, "isDm")).unwrap_or(false);
    win.set_pi_dm(is_dm);
    win.set_pi_loading(true);
    call_ui(&ui.req.clone(), "pins.items", json!({"roomId": rid}), move |ui, win, out| {
        win.set_pi_loading(false);
        win.set_pi_loaded(true);
        if let Ok(v) = out {
            let items = v["items"].as_array().cloned().unwrap_or_default();
            let rows: Vec<TimelineRow> = items.iter().map(|it| {
                let mut row = simple_row(ui, it);
                row.stamp = project::pin_stamp(it["ts"].as_i64().unwrap_or(0)).into();
                row.media_filename = project::kind_words(s(it, "kind")).into();
                row
            }).collect();
            win.set_pi_items(ModelRc::new(VecModel::from(rows)));
        }
    });
}

/// pins.list reply → pinned ids cache → pin markers on bubbles.
pub fn load_pinned_ids(ui: &mut UiState, room_id: &str) {
    let rid = room_id.to_string();
    call_ui(&ui.req.clone(), "pins.list", json!({"roomId": rid.clone()}), move |ui, win, out| {
        if let Ok(v) = out {
            let ids: Vec<String> = v["events"].as_array().map(|a| {
                a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()
            }).unwrap_or_default();
            ui.pinned_by_room.insert(rid, ids);
            rebuild_timeline(ui, win);
        }
    });
}

pub fn reload_pins_if_open(ui: &mut UiState, win: &AppWindow) {
    if win.get_nav() == "pins" {
        load_pins(ui, win);
    }
}

fn rebuild_search(ui: &mut UiState, win: &AppWindow) {
    let items = ui.shadow.clone();
    let out = project::collect_search(&items, &ui.search_query, simple_row_pure);
    win.set_se_searching(ui.search_query.chars().count() >= 2);
    let mut images = Vec::new();
    for item in items.iter().rev() {
        if s(item, "kind") == "image" && images.len() < 12 {
            images.push(simple_row(ui, item));
        }
    }
    win.set_se_results(ModelRc::new(VecModel::from(out.results)));
    win.set_se_images(ModelRc::new(VecModel::from(images)));
    win.set_se_links(ModelRc::new(VecModel::from(out.links)));
}

fn rebuild_forward(ui: &mut UiState, win: &AppWindow) {
    let q = ui.forward_query.to_lowercase();
    let rooms = ui.rooms_json.clone();
    let rows: Vec<_> = rooms.iter()
        .filter(|r| !b(r, "isSpace") && !b(r, "isInvite"))
        .filter(|r| q.is_empty() || s(r, "name").to_lowercase().contains(&q))
        .map(|r| crate::bridge::room_row_of(ui, r))
        .collect();
    win.set_fw_chats(ModelRc::new(VecModel::from(rows)));
}

fn rebuild_start_suggestions(ui: &mut UiState, win: &AppWindow) {
    let rows: Vec<_> = ui.rooms_json.iter()
        .filter(|r| b(r, "isDm"))
        .take(12)
        .map(|r| project::user_row(&json!({"userId": s(r, "dmUserId"), "displayName": s(r, "name")}), false))
        .collect();
    win.set_st_people(ModelRc::new(VecModel::from(rows)));
}

// ---------------------------------------------------------------- rows

fn simple_row_pure(item: &Value) -> TimelineRow {
    let sender = s(item, "sender").to_string();
    let name = match s(item, "senderName") { "" => sender.clone(), d => d.to_string() };
    TimelineRow {
        id: s(item, "id").into(),
        event_id: s(item, "eventId").into(),
        kind: s(item, "kind").into(),
        body: s(item, "body").into(),
        sender: sender.clone().into(),
        sender_name: name.clone().into(),
        initials: initials(&name).into(),
        tint: tint_for(&sender),
        stamp: crate::rows::bubble_stamp(item["ts"].as_i64().unwrap_or(0)).into(),
        ..Default::default()
    }
}

fn simple_row(ui: &mut UiState, item: &Value) -> TimelineRow {
    let mut row = simple_row_pure(item);
    let thumb = item["media"]["thumbnailPath"].as_str()
        .or(item["media"]["path"].as_str())
        .unwrap_or("")
        .to_string();
    if let Some(img) = crate::bridge::avatar_pub(ui, &thumb) {
        row.thumb = img;
    }
    row
}

pub fn apply_media_ready(ui: &mut UiState, win: &AppWindow, v: &Value) {
    if s(v, "kind") == "avatar" {
        let req = ui.req.clone();
        after(&req, 400, |ui, win| {
            call_ui(&ui.req.clone(), "rooms.list", json!({}), |ui, win, out| {
                if let Ok(r) = out {
                    if let Some(rooms) = r["rooms"].as_array() {
                        ui.rooms_json = rooms.clone();
                        rebuild_rooms(ui, win);
                    }
                }
            });
        });
        return;
    }
    let room = s(v, "roomId").to_string();
    if room != room_of_key(&ui.open_room) {
        return;
    }
    let event_id = s(v, "eventId").to_string();
    let path = s(v, "path").to_string();
    let thumb = b(v, "thumbnail");
    for item in ui.shadow.iter_mut() {
        if item["eventId"].as_str() == Some(event_id.as_str()) && !item["media"].is_null() {
            item["media"][if thumb { "thumbnailPath" } else { "path" }] = json!(path);
            break;
        }
    }
    rebuild_timeline(ui, win);
}

// ---------------------------------------------------------------- act()

pub fn on_act(ui: &mut UiState, win: &AppWindow, action: &str, a: &str, b2: &str) {
    let req = ui.req.clone();
    let open_room = room_of_key(&ui.open_room);
    let action_is_doc = action == "doc-download";
    match action {
        "send-reply" => req.fire("message.reply", json!({"roomId": ui.open_room, "eventId": a, "body": b2, "markdown": true})),
        "send-edit" => req.fire("message.edit", json!({"roomId": ui.open_room, "eventId": a, "body": b2, "markdown": true})),
        "react" => req.fire("message.react", json!({"roomId": ui.open_room, "eventId": a, "key": b2})),
        "paginate" => {
            let state = ui.pagination_by_room.get(&ui.open_room).map(String::as_str).unwrap_or("idle");
            if state == "idle" {
                req.fire("timeline.paginate", json!({"roomId": ui.open_room, "count": 50}));
            }
        }
        "mark-read" => req.fire("room.markRead", json!({"roomId": open_room})),
        "vote" => req.fire("poll.vote", json!({"roomId": ui.open_room, "eventId": a, "answers": [b2]})),
        "start-call" => req.fire("call.start", json!({"roomId": open_room, "video": a == "true"})),
        "join-call" => req.fire("call.join", json!({"roomId": open_room, "video": false})),
        "call-accept" => {
            let room = ui.call["incoming"]["roomId"].as_str().unwrap_or("").to_string();
            if !room.is_empty() {
                req.fire("call.join", json!({"roomId": room, "video": a == "true"}));
            }
        }
        "call-decline" => {
            let room = ui.call["incoming"]["roomId"].as_str().unwrap_or("").to_string();
            if !room.is_empty() {
                req.fire("call.decline", json!({"roomId": room}));
            }
        }
        "accept-invite" => req.fire("room.join", json!({"roomIdOrAlias": open_room})),
        "decline-invite" => {
            req.fire("room.leave", json!({"roomId": open_room}));
            win.set_nav("home".into());
        }
        "open-thread" => {
            call_ui(&req, "thread.open", json!({"roomId": open_room, "rootId": a, "initialItems": 60}), move |ui, win, out| {
                if let Ok(v) = out {
                    let key = s(&v, "key").to_string();
                    if !key.is_empty() {
                        crate::bridge::switch_timeline(ui, win, &key);
                        win.set_thread_key(key.as_str().into());
                        win.set_chat_is_thread(true);
                        win.set_nav("thread".into());
                    }
                }
            });
        }
        "menu-action" => menu_action(ui, win, a, b2),
        "voice-toggle" => voice_toggle(ui, win, a),
        "voice-seek" => voice_seek(ui, win, a, b2.parse().unwrap_or(0.0)),
        "emoji-search" => push_emoji(ui, win, a),
        "jump-to" => win.invoke_scroll_timeline_to_end(),
        "open-link" => crate::platform::open_url(a),

        "open-space" => {}
        "join-room" => {
            call_ui(&req, "room.join", json!({"roomIdOrAlias": a}), move |ui, win, out| {
                match out {
                    Ok(_) => { let sid = win.get_space_id().to_string(); load_hierarchy(ui, win, &sid); }
                    Err((_, m)) => win.set_sp_note(format!("Could not join: {m}").into()),
                }
            });
        }
        "space-new-room" => {}
        "spacerooms-mode" => {
            ui.spacerooms_mode = a.to_string();
            ui.spacerooms_selected.clear();
        }
        "spacerooms-toggle" => {
            if !ui.spacerooms_selected.remove(a) {
                ui.spacerooms_selected.insert(a.to_string());
            }
            build_spacerooms(ui, win);
        }
        "spacerooms-apply" => spacerooms_apply(ui, win),
        "share-space" => {
            let alias = ui.rooms_json.iter().find(|r| s(r, "id") == win.get_space_id().as_str())
                .map(|r| s(r, "canonicalAlias").to_string())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| win.get_space_id().to_string());
            crate::platform::copy_text(&format!("https://matrix.to/#/{alias}"));
            win.set_sp_note("Link copied".into());
        }
        "leave-space" => {
            let sid = win.get_space_id().to_string();
            call_ui(&req, "room.leave", json!({"roomId": sid}), move |_ui, win, out| {
                match out {
                    Ok(_) => win.set_nav("home".into()),
                    Err((_, m)) => win.set_sp_note(format!("Could not leave: {m}").into()),
                }
            });
        }
        "pick-newspace-avatar" | "pick-space-avatar" | "theme-photo" => pick_file(ui, action),
        "create-space" => create_space(ui, win, a, b2),
        "space-save" => space_save(ui, win, a, b2),

        "set-favourite" => req.fire("room.setFavourite", json!({"roomId": ui.settings_room, "favourite": a == "true"})),
        "set-low-priority" => req.fire("room.setLowPriority", json!({"roomId": ui.settings_room, "lowPriority": a == "true"})),
        "toggle-space" => {
            let request = if b2 == "true" { "space.addRoom" } else { "space.removeRoom" };
            req.fire(request, json!({"spaceId": a, "roomId": ui.settings_room}));
        }
        "leave-room" => {
            let rid = ui.settings_room.clone();
            call_ui(&req, "room.leave", json!({"roomId": rid}), move |_ui, win, _out| {
                win.set_nav("home".into());
            });
        }
        "notif-mode" => {
            win.set_no_mode(if a == "default" { "" } else { a }.into());
            win.set_no_busy(true);
            let rid = ui.settings_room.clone();
            call_ui(&req, "room.setSettings", json!({"roomId": rid, "notificationMode": a}), move |ui, win, out| {
                win.set_no_busy(false);
                if let Err((_, m)) = out {
                    win.set_no_error(m.as_str().into());
                }
                reread_settings_later(ui);
            });
        }
        "security-save" => {
            let mut parts = a.split('\u{1f}');
            let jr = parts.next().unwrap_or("").to_string();
            let hv = parts.next().unwrap_or("").to_string();
            let enc = b2 == "true";
            let mut p = json!({"roomId": ui.settings_room});
            let cur = ui.settings.clone();
            if !jr.is_empty() && jr != s(&cur, "joinRule") {
                p["joinRule"] = json!(jr);
                if jr == "restricted" {
                    p["restrictedTo"] = json!(win.get_sec_space_id().as_str());
                }
            }
            if !hv.is_empty() && hv != s(&cur, "historyVisibility") {
                p["historyVisibility"] = json!(hv);
            }
            if enc && !b(&cur, "isEncrypted") {
                p["encrypted"] = json!(true);
            }
            win.set_sec_busy(true);
            call_ui(&req, "room.setSettings", p, move |ui, win, out| {
                win.set_sec_busy(false);
                if let Err((_, m)) = out {
                    win.set_sec_error(m.as_str().into());
                }
                reread_settings_later(ui);
            });
        }
        "set-my-role" => {
            let level: i64 = a.parse().unwrap_or(0);
            let me = ui.my_user.clone();
            let rid = ui.settings_room.clone();
            win.set_ro_busy(true);
            call_ui(&req, "room.setPowerLevel", json!({"roomId": rid, "userId": me, "level": level}), move |ui, win, out| {
                win.set_ro_busy(false);
                if let Err((_, m)) = out {
                    win.set_ro_note(m.as_str().into());
                }
                reread_settings_later(ui);
            });
        }
        "set-power" => {
            let level: i64 = b2.parse().unwrap_or(0);
            let rid = ui.settings_room.clone();
            win.set_pe_busy(true);
            call_ui(&req, "room.setPowerLevel", json!({"roomId": rid, "key": a, "level": level}), move |ui, win, out| {
                win.set_pe_busy(false);
                if let Err((_, m)) = out {
                    win.set_pe_note(m.as_str().into());
                }
                reread_settings_later(ui);
            });
        }
        "reset-permissions" => {
            let rid = ui.settings_room.clone();
            win.set_ro_busy(true);
            // The Matrix defaults, ONE KEY AT A TIME (a half-applied batch
            // leaves the room in a state nobody chose).
            for (key, level) in [("invite", 0), ("kick", 50), ("ban", 50), ("redact", 50),
                                 ("eventsDefault", 0), ("stateDefault", 50), ("name", 50),
                                 ("avatar", 50), ("topic", 50), ("liveLocation", 50)] {
                req.fire("room.setPowerLevel", json!({"roomId": rid, "key": key, "level": level}));
            }
            after(&req, 1500, |ui, win| {
                win.set_ro_busy(false);
                load_settings(ui, win);
            });
        }
        "members-filter" => {
            ui.members_filter = a.parse().unwrap_or(-1);
            load_members(ui, win);
        }
        "load-members" => {
            ui.settings_room = a.to_string();
            ui.members_filter = -1;
            load_members(ui, win);
        }
        "dir-search" | "dir-search-start" => dir_search(ui, win, action, a),
        "invite-user" => {
            let rid = ui.settings_room.clone();
            let uid = a.to_string();
            call_ui(&req, "room.invite", json!({"roomId": rid, "userId": uid.clone()}), move |_ui, win, out| {
                win.set_ap_note(match out {
                    Ok(_) => format!("Invited {uid}"),
                    Err((_, m)) => format!("Invite failed: {m}"),
                }.into());
            });
        }

        "start-dm" => {
            call_ui(&req, "dm.create", json!({"userId": a}), move |ui, win, out| {
                match out {
                    Ok(v) => {
                        let rid = s(&v, "roomId").to_string();
                        if !rid.is_empty() {
                            crate::bridge::open_room(ui, win, &rid);
                            win.set_nav("chat".into());
                        }
                    }
                    Err((_, m)) => win.set_st_error(m.as_str().into()),
                }
            });
        }
        "start-submit" => start_submit(ui, win, a, b2),
        "forward-search" => {
            ui.forward_query = a.to_string();
            rebuild_forward(ui, win);
        }
        "forward-picked" => forward_picked(ui, win, a),
        "room-search" => {
            ui.search_query = a.to_string();
            rebuild_search(ui, win);
        }
        "unpin" => req.fire("message.unpin", json!({"roomId": open_room, "eventId": a})),

        // ---- viewer ----
        "viewer-open" => viewer_open(ui, win, a),
        "viewer-closed" => {
            if ui.audio_playing { req.fire("video.stop", json!({})); }
        }
        "viewer-page" => {
            let i: usize = a.parse().unwrap_or(0);
            if let Some(item) = ui.viewer_items.get(i).cloned() {
                let ev = s(&item, "eventId");
                if item["media"]["path"].as_str().unwrap_or("").is_empty() && !ev.is_empty() {
                    req.fire("media.get", json!({"roomId": open_room, "eventId": ev}));
                }
            }
        }
        "viewer-download" | "viewer-delete" | "viewer-react" | "viewer-forward" | "viewer-share" => {
            viewer_misc(ui, win, action, a);
        }
        "viewer-playback" => {
            let i = win.get_vw_cur().max(0) as usize;
            if let Some(item) = ui.viewer_items.get(i) {
                let ev = s(item, "eventId").to_string();
                if win.get_vw_playing_event() == ev.as_str() {
                    req.fire("video.stop", json!({}));
                    win.set_vw_playing_event("".into());
                } else {
                    req.fire("video.play", json!({"roomId": open_room, "eventId": ev, "audio": true}));
                    win.set_vw_playing_event(ev.as_str().into());
                }
            }
        }
        "viewer-seek" => req.fire("video.seek", json!({"seconds": a.parse::<f64>().unwrap_or(0.0)})),
        "viewer-scrub" => {}

        // ---- doc / audio pages ----
        "open-doc" => doc_open(ui, win, a),
        "doc-download" | "audio-download" => {
            let (rid, ev) = if action_is_doc { ui.doc_ctx.clone() } else { ui.audio_ctx.clone() };
            let dest = format!("{}/Downloads", std::env::var("HOME").unwrap_or_default());
            call_ui(&req, "media.saveAs", json!({"roomId": rid, "eventId": ev, "dest": dest}), move |_ui, win, out| {
                let msg = match out { Ok(v) => format!("Saved to {}", s(&v, "path")), Err((_, m)) => m };
                if action_is_doc { win.set_dc_toast(msg.as_str().into()); } else { win.set_au_toast(msg.as_str().into()); }
            });
        }
        "doc-page" => doc_page(ui, win, a, b2),
        "doc-sheet" => doc_sheet(ui, win, a),
        "open-audio-page" => audio_open(ui, win, a),
        "audio-toggle" => audio_page_toggle(ui, win),
        "audio-seek" => {
            let (rid, ev) = ui.audio_ctx.clone();
            req.fire("audio.play", json!({"roomId": rid, "eventId": ev, "seek": a.parse::<f64>().unwrap_or(0.0)}));
            win.set_au_playing(true);
        }

        // ---- chat theme ----
        "theme-accent" => { ui.theme_pending["accent"] = json!(a); push_theme(ui, win); }
        "theme-wallpaper" => { ui.theme_pending["wallpaper"] = json!(a); push_theme(ui, win); }
        "theme-reset" => { ui.theme_pending = json!({}); push_theme(ui, win); }
        "theme-apply" => theme_apply(ui, win),
        "theme-pick" => {
            let mut hs = a.split(',');
            let h: f32 = hs.next().unwrap_or("0").parse().unwrap_or(0.0);
            let sat: f32 = hs.next().unwrap_or("0").parse().unwrap_or(0.0);
            let v: f32 = b2.parse().unwrap_or(0.0);
            let (r, g, b3) = hsv_rgb(h, sat, v);
            win.set_ct_pick(slint::Color::from_rgb_u8(r, g, b3));
            let (r2, g2, b4) = hsv_rgb(h, sat, 1.0);
            win.set_ct_pick_end(slint::Color::from_rgb_u8(r2, g2, b4));
            ui.theme_pending["_pick"] = json!(format!("#{r:02X}{g:02X}{b3:02X}"));
        }
        "theme-accept-custom" => {
            let hex = ui.theme_pending["_pick"].as_str().unwrap_or("#7c9fd4").to_string();
            ui.theme_pending["accent"] = json!(hex);
            push_theme(ui, win);
        }

        // ---- attach ----
        "open-attach" => {
            if ui.stickers.is_empty() {
                load_stickers(ui, win);
            }
        }
        "pick-attach-files" => attach_files(ui),
        "attach-location" => tracing::info!("location share ({a}): the maps gap — no picker without a map widget"),
        "composer-insert" => tracing::info!("composer emoji insert needs a chat-page function; deferred"),
        "create-poll" => create_poll(ui, a, b2),
        "load-stickers" => load_stickers(ui, win),
        "send-sticker" => send_sticker(ui, a),

        // ---- voice recorder ----
        "open-recorder" => {
            ui.rec_levels.clear();
            win.set_rec_state("idle".into());
        }
        "voice-record" => {
            ui.recording = true;
            ui.rec_levels.clear();
            win.set_rec_state("recording".into());
            win.set_rec_elapsed(0.0);
            req.fire("voice.start", json!({}));
            tick_recorder(ui);
        }
        "voice-stop" => {
            ui.recording = false;
            call_ui(&req, "voice.stop", json!({}), move |ui, win, out| {
                if let Ok(v) = out {
                    win.set_rec_clip_duration((v["duration"].as_f64().unwrap_or(0.0) / 1000.0) as f32);
                    let wave: Vec<f64> = v["waveform"].as_array().map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
                    win.set_rec_clip_waveform(ModelRc::new(VecModel::from(crate::rows::resample_wave(&wave, 60))));
                    ui.voice_clip = v;
                    win.set_rec_state("ready".into());
                } else {
                    win.set_rec_state("idle".into());
                }
            });
        }
        "voice-restart" => {
            req.fire("voice.cancel", json!({}));
            ui.recording = true;
            ui.rec_levels.clear();
            win.set_rec_state("recording".into());
            win.set_rec_elapsed(0.0);
            req.fire("voice.start", json!({}));
            tick_recorder(ui);
        }
        "voice-attach" => {
            let clip = std::mem::take(&mut ui.voice_clip);
            req.fire("voice.send", json!({
                "roomId": open_room,
                "path": s(&clip, "path"),
                "duration": clip["duration"].as_f64().unwrap_or(0.0),
                "waveform": clip["waveform"].clone(),
                "caption": "",
            }));
            win.set_recorder_open(false);
        }
        "voice-cancel" => {
            ui.recording = false;
            req.fire("voice.cancel", json!({}));
        }
        "open-map" => tracing::info!("map page carries the static card; nothing to load beyond the item"),
        other => tracing::warn!("act: unknown action {other}"),
    }
}

fn menu_action(ui: &mut UiState, win: &AppWindow, action: &str, event_id: &str) {
    let req = ui.req.clone();
    let key = ui.open_room.clone();
    let room = room_of_key(&key);
    match action {
        "prepare" => build_sheet(ui, win, event_id),
        "copy" => {
            if let Some(item) = ui.shadow.iter().find(|i| i["eventId"].as_str() == Some(event_id)) {
                crate::platform::copy_text(s(item, "body"));
            }
        }
        "forward" => {
            ui.forward_item = ui.shadow.iter().find(|i| i["eventId"].as_str() == Some(event_id)).cloned().unwrap_or(Value::Null);
            win.set_fw_mode("forward".into());
            win.invoke_go("forward".into());
        }
        "pin" => {
            let pinned = ui.pinned_by_room.get(&room).map(|p| p.iter().any(|e| e == event_id)).unwrap_or(false);
            req.fire(if pinned { "message.unpin" } else { "message.pin" }, json!({"roomId": room, "eventId": event_id}));
        }
        "redact" => req.fire("message.redact", json!({"roomId": key, "eventId": event_id})),
        "endpoll" => req.fire("poll.end", json!({"roomId": key, "eventId": event_id})),
        "retry" | "cancelsend" => {
            if let Some(item) = ui.shadow.iter().find(|i| i["id"].as_str() == Some(event_id) || i["eventId"].as_str() == Some(event_id)) {
                let request = if action == "retry" { "message.retry" } else { "message.cancelSend" };
                req.fire(request, json!({"roomId": key, "id": s(item, "id"), "txnId": s(item, "txnId")}));
            }
        }
        other => tracing::info!("menu action {other} not wired"),
    }
}

/// MessageSheet.actionsFor, built bridge-side as [MenuEntry].
fn build_sheet(ui: &mut UiState, win: &AppWindow, event_id: &str) {
    let Some(item) = ui.shadow.iter().find(|i| i["eventId"].as_str() == Some(event_id) || i["id"].as_str() == Some(event_id)).cloned() else { return };
    let g = win.global::<crate::Icons>();
    let mut out: Vec<crate::MenuEntry> = Vec::new();
    let mut add = |t: &str, a: &str, icon: SharedString, danger: bool| {
        out.push(crate::MenuEntry { t: t.into(), a: a.into(), icon, danger });
    };
    let send_state = s(&item, "sendState");
    let can = item["can"].clone();
    let kind = s(&item, "kind");
    if send_state == "sending" || send_state == "failed" {
        if send_state == "failed" { add("Retry", "retry", g.get_retry(), false); }
        add("Copy", "copy", g.get_copy(), false);
        add("Cancel send", "cancelsend", g.get_cancel(), true);
    } else {
        add("Reply", "reply", g.get_reply(), false);
        add("Forward", "forward", g.get_forward(), false);
        add("Copy", "copy", g.get_copy(), false);
        if !s(&item, "eventId").is_empty() {
            let room = room_of_key(&ui.open_room);
            let pinned = ui.pinned_by_room.get(&room).map(|p| p.iter().any(|e| e == s(&item, "eventId"))).unwrap_or(false);
            add(if pinned { "Unpin" } else { "Pin" }, "pin", g.get_pin(), false);
            add("Reply in thread", "thread", g.get_thread(), false);
        }
        if b(&can, "edit") && kind != "poll" {
            add("Edit", "edit", g.get_edit(), false);
        }
        if kind == "poll" && b(&can, "redact") {
            add("End poll", "endpoll", g.get_poll(), false);
        }
        if b(&can, "redact") {
            add("Delete", "redact", g.get_trash(), true);
        }
    }
    ui.sheet_item = item;
    win.set_sheet_actions(ModelRc::new(VecModel::from(out)));
}

fn spacerooms_apply(ui: &mut UiState, win: &AppWindow) {
    let sid = win.get_space_id().to_string();
    let add = ui.spacerooms_mode == "add";
    let req = ui.req.clone();
    let ids: Vec<String> = ui.spacerooms_selected.drain().collect();
    win.set_sr_busy(true);
    for rid in &ids {
        req.fire(if add { "space.addRoom" } else { "space.removeRoom" },
                 json!({"spaceId": sid, "roomId": rid}));
    }
    after(&req, 1200, move |ui, win| {
        win.set_sr_busy(false);
        let sid = win.get_space_id().to_string();
        load_hierarchy(ui, win, &sid);
    });
}

fn create_space(ui: &mut UiState, win: &AppWindow, name: &str, packed: &str) {
    let mut parts = packed.split('\u{1f}');
    let topic = parts.next().unwrap_or("").to_string();
    let private = parts.next().unwrap_or("true") == "true";
    let avatar = ui.new_space_avatar.clone();
    win.set_ns_busy(true);
    call_ui(&ui.req.clone(), "room.create",
        json!({"name": name, "topic": topic, "private": private, "encrypted": false, "space": true}),
        move |ui, win, out| {
            win.set_ns_busy(false);
            match out {
                Ok(v) => {
                    let sid = s(&v, "roomId").to_string();
                    // Avatar second: uploading first orphans the file if create fails.
                    if !avatar.is_empty() && !sid.is_empty() {
                        ui.req.fire("room.setAvatar", json!({"roomId": sid, "path": avatar}));
                    }
                    if !sid.is_empty() {
                        win.set_space_id(sid.as_str().into());
                        win.invoke_go("space".into());
                    }
                }
                Err((_, m)) => win.set_ns_error(m.as_str().into()),
            }
        });
}

fn space_save(ui: &mut UiState, win: &AppWindow, name: &str, topic: &str) {
    let rid = ui.settings_room.clone();
    let mut p = json!({"roomId": rid});
    if name != s(&ui.settings, "name") { p["name"] = json!(name); }
    if topic != s(&ui.settings, "topic") { p["topic"] = json!(topic); }
    let avatar = win.get_ss_avatar_path().to_string();
    win.set_ss_busy(true);
    call_ui(&ui.req.clone(), "room.setSettings", p, move |ui, win, out| {
        win.set_ss_busy(false);
        match out {
            Ok(_) => {
                if !avatar.is_empty() {
                    ui.req.fire("room.setAvatar", json!({"roomId": ui.settings_room, "path": avatar}));
                }
                reread_settings_later(ui);
            }
            Err((_, m)) => win.set_ss_error(m.as_str().into()),
        }
    });
}

fn start_submit(ui: &mut UiState, win: &AppWindow, mode: &str, value: &str) {
    let req = ui.req.clone();
    match mode {
        "join" => {
            call_ui(&req, "room.join", json!({"roomIdOrAlias": value}), move |ui, win, out| {
                match out {
                    Ok(v) => {
                        let rid = s(&v, "roomId").to_string();
                        if !rid.is_empty() {
                            crate::bridge::open_room(ui, win, &rid);
                            win.set_nav("chat".into());
                        } else {
                            win.set_nav("home".into());
                        }
                    }
                    Err((_, m)) => win.set_st_error(m.as_str().into()),
                }
            });
        }
        "space" => {
            call_ui(&req, "room.create", json!({"name": value, "topic": "", "private": true, "encrypted": false, "space": true}), move |_ui, win, out| {
                match out {
                    Ok(_) => win.set_nav("home".into()),
                    Err((_, m)) => win.set_st_error(m.as_str().into()),
                }
            });
        }
        _ => {
            let space = win.get_space_id().to_string();
            let from_space = !space.is_empty();
            call_ui(&req, "room.create", json!({"name": value, "topic": "", "private": true, "encrypted": true}), move |ui, win, out| {
                match out {
                    Ok(v) => {
                        let rid = s(&v, "roomId").to_string();
                        if from_space && !rid.is_empty() {
                            ui.req.fire("space.addRoom", json!({"spaceId": space, "roomId": rid}));
                        }
                        if !rid.is_empty() {
                            crate::bridge::open_room(ui, win, &rid);
                            win.set_nav("chat".into());
                        }
                    }
                    Err((_, m)) => win.set_st_error(m.as_str().into()),
                }
            });
        }
    }
}

fn forward_picked(ui: &mut UiState, win: &AppWindow, room_id: &str) {
    let item = std::mem::take(&mut ui.forward_item);
    let req = ui.req.clone();
    let path = item["media"]["path"].as_str().unwrap_or("");
    if !path.is_empty() {
        req.fire("attachment.send", json!({"roomId": room_id, "path": path}));
    } else {
        req.fire("message.send", json!({"roomId": room_id, "body": s(&item, "body"), "markdown": false}));
    }
    crate::bridge::open_room(ui, win, room_id);
    win.set_nav("chat".into());
}

fn dir_search(ui: &mut UiState, win: &AppWindow, which: &str, q: &str) {
    let start = which == "dir-search-start";
    let q = q.trim().to_string();
    if q.chars().count() < 2 {
        if start {
            rebuild_start_suggestions(ui, win);
        } else {
            win.set_ap_results(ModelRc::new(VecModel::from(Vec::new())));
        }
        return;
    }
    let epoch = if start {
        ui.start_query_epoch += 1;
        ui.start_query_epoch
    } else {
        ui.dir_query_epoch += 1;
        ui.dir_query_epoch
    };
    let req = ui.req.clone();
    after(&req, 300, move |ui, win| {
        let current = if start { ui.start_query_epoch } else { ui.dir_query_epoch };
        if current != epoch {
            return; // superseded
        }
        if start { win.set_st_busy(true); }
        call_ui(&ui.req.clone(), "users.search", json!({"query": q, "limit": 12}), move |_ui, win, out| {
            if start { win.set_st_busy(false); }
            if let Ok(v) = out {
                let rows: Vec<_> = v["users"].as_array().map(|a| a.iter().map(|u| project::user_row(u, false)).collect()).unwrap_or_default();
                let model = ModelRc::new(VecModel::from(rows));
                if start { win.set_st_people(model); } else { win.set_ap_results(model); }
            }
        });
    });
}

fn voice_toggle(ui: &mut UiState, win: &AppWindow, event_id: &str) {
    let req = ui.req.clone();
    if ui.audio_playing && ui.audio_ctx.1 == event_id {
        req.fire("audio.stop", json!({}));
        ui.audio_playing = false;
    } else {
        let room = room_of_key(&ui.open_room);
        ui.audio_ctx = (room.clone(), event_id.to_string());
        ui.audio_playing = true;
        req.fire("audio.play", json!({"roomId": room, "eventId": event_id, "seek": 0}));
    }
    rebuild_timeline(ui, win);
}

fn voice_seek(ui: &mut UiState, win: &AppWindow, event_id: &str, frac: f64) {
    let room = room_of_key(&ui.open_room);
    let dur = ui.shadow.iter()
        .find(|i| i["eventId"].as_str() == Some(event_id))
        .and_then(|i| i["media"]["duration"].as_f64())
        .unwrap_or(0.0) / 1000.0;
    ui.audio_ctx = (room.clone(), event_id.to_string());
    ui.audio_playing = true;
    ui.req.fire("audio.play", json!({"roomId": room, "eventId": event_id, "seek": frac * dur}));
    rebuild_timeline(ui, win);
}

fn push_emoji(ui: &mut UiState, win: &AppWindow, query: &str) {
    if ui.emojis.is_empty() {
        if let Ok(text) = std::fs::read_to_string("/usr/share/omarchy/shell/plugins/emojis/emojis.json") {
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                if let Some(arr) = v.as_array() {
                    ui.emojis = arr.iter()
                        .map(|e| (s(e, "e").to_string(), s(e, "k").to_string()))
                        .filter(|(g, _)| !g.is_empty())
                        .collect();
                }
            }
        }
    }
    let q = query.to_lowercase();
    let filtered: Vec<_> = ui.emojis.iter()
        .filter(|(_, k)| q.is_empty() || k.contains(&q))
        .collect();
    let mut rows: Vec<ModelRc<crate::EmojiItem>> = Vec::new();
    for chunk in filtered.chunks(8) {
        let row: Vec<crate::EmojiItem> = chunk.iter()
            .map(|(g, k)| crate::EmojiItem { glyph: g.as_str().into(), name: k.as_str().into() })
            .collect();
        rows.push(ModelRc::new(VecModel::from(row)));
    }
    let cat_marks = ["👋", "🐵", "🍇", "🌍", "🎃", "👓", "🏧", "🏁"];
    let cats: Vec<i32> = cat_marks.iter()
        .map(|m| filtered.iter().position(|(g, _)| g == m).map(|i| (i / 8) as i32).unwrap_or(0))
        .collect();
    win.set_emoji_rows(ModelRc::new(VecModel::from(rows)));
    win.set_emoji_cat_rows(ModelRc::new(VecModel::from(cats)));
}

fn pick_file(ui: &mut UiState, purpose: &str) {
    let purpose = purpose.to_string();
    ui.req.handle().spawn(async move {
        let picked = crate::platform::pick_file().await;
        let _ = slint::invoke_from_event_loop(move || {
            with_ui(|ui| {
                let Some(win) = ui.win.upgrade() else { return };
                let Some(path) = picked else { return };
                match purpose.as_str() {
                    "pick-newspace-avatar" => {
                        ui.new_space_avatar = path.clone();
                        if let Some(img) = crate::bridge::avatar_pub(ui, &path) {
                            win.set_ns_avatar(img);
                            win.set_ns_has_avatar(true);
                        }
                    }
                    "pick-space-avatar" => {
                        win.set_ss_avatar_path(path.as_str().into());
                        if let Some(img) = crate::bridge::avatar_pub(ui, &path) {
                            win.set_ss_avatar(img);
                        }
                    }
                    _ => {}
                }
            });
        });
    });
}

fn reread_settings_later(ui: &mut UiState) {
    // A read issued the instant the write returns still sees the old value
    // (docs/development.md); wait out the propagation.
    let req = ui.req.clone();
    after(&req, 1200, |ui, win| load_settings(ui, win));
}

// ---------------------------------------------------------------- media pass

fn viewer_open(ui: &mut UiState, win: &AppWindow, event_id: &str) {
    ui.viewer_items = ui.shadow.iter()
        .filter(|i| matches!(s(i, "kind"), "image" | "sticker" | "video"))
        .cloned()
        .collect();
    let cur = ui.viewer_items.iter().position(|i| s(i, "eventId") == event_id).unwrap_or(0);
    let items = ui.viewer_items.clone();
    let rows: Vec<crate::ViewerItem> = items.iter().map(|i| {
        let media = i["media"].clone();
        let path = media["path"].as_str().or(media["thumbnailPath"].as_str()).unwrap_or("").to_string();
        let img = crate::bridge::avatar_pub(ui, &path);
        crate::ViewerItem {
            event_id: s(i, "eventId").into(),
            kind: s(i, "kind").into(),
            sender_name: s(i, "senderName").into(),
            is_own: b(i, "isOwn"),
            ts_label: crate::project::session_label(i["ts"].as_i64().unwrap_or(0)).into(),
            have_img: img.is_some(),
            img: img.unwrap_or_default(),
            w: media["width"].as_i64().unwrap_or(0) as i32,
            h: media["height"].as_i64().unwrap_or(0) as i32,
            can_redact: b(&i["can"], "redact"),
        }
    }).collect();
    let names: Vec<SharedString> = ui.rooms_json.iter()
        .filter(|r| !b(r, "isSpace") && !b(r, "isInvite"))
        .take(8)
        .map(|r| SharedString::from(s(r, "name")))
        .collect();
    win.set_vw_items(ModelRc::new(VecModel::from(rows)));
    win.set_vw_forward_names(ModelRc::new(VecModel::from(names)));
    win.set_vw_cur(cur as i32);
    win.set_viewer_open(true);
    if let Some(item) = ui.viewer_items.get(cur) {
        let ev = s(item, "eventId");
        if item["media"]["path"].as_str().unwrap_or("").is_empty() && !ev.is_empty() {
            let room = room_of_key(&ui.open_room);
            ui.req.fire("media.get", json!({"roomId": room, "eventId": ev}));
        }
    }
}

fn viewer_misc(ui: &mut UiState, win: &AppWindow, action: &str, a: &str) {
    let i = win.get_vw_cur().max(0) as usize;
    let Some(item) = ui.viewer_items.get(i).cloned() else { return };
    let ev = s(&item, "eventId").to_string();
    let room = room_of_key(&ui.open_room);
    let req = ui.req.clone();
    match action {
        "viewer-download" => {
            let dest = format!("{}/Downloads", std::env::var("HOME").unwrap_or_default());
            call_ui(&req, "media.saveAs", json!({"roomId": room, "eventId": ev, "dest": dest}), |_ui, win, out| {
                win.set_vw_toast(match out { Ok(v) => format!("Saved to {}", s(&v, "path")), Err((_, m)) => m }.into());
            });
        }
        "viewer-delete" => {
            req.fire("message.redact", json!({"roomId": room, "eventId": ev}));
            win.set_viewer_open(false);
        }
        "viewer-react" => req.fire("message.react", json!({"roomId": room, "eventId": ev, "key": a})),
        "viewer-share" => {
            let path = item["media"]["path"].as_str().unwrap_or("").to_string();
            if !path.is_empty() {
                crate::platform::copy_text(&path);
                win.set_vw_toast("Path copied".into());
            }
        }
        "viewer-forward" => {
            let idx: usize = a.parse().unwrap_or(0);
            let rid = ui.rooms_json.iter()
                .filter(|r| !b(r, "isSpace") && !b(r, "isInvite"))
                .nth(idx)
                .map(|r| s(r, "id").to_string());
            let path = item["media"]["path"].as_str()
                .or(item["media"]["thumbnailPath"].as_str())
                .unwrap_or("").to_string();
            if let Some(rid) = rid {
                if !path.is_empty() {
                    req.fire("attachment.send", json!({"roomId": rid, "path": path}));
                    win.set_vw_toast("Forwarded".into());
                }
            }
        }
        _ => {}
    }
}

fn doc_open(ui: &mut UiState, win: &AppWindow, event_id: &str) {
    let room = room_of_key(&ui.open_room);
    ui.doc_ctx = (room.clone(), event_id.to_string());
    ui.doc_pages.clear();
    win.set_dc_status("loading".into());
    win.set_dc_blocks(ModelRc::new(VecModel::from(Vec::new())));
    win.set_dc_pages(ModelRc::new(VecModel::from(Vec::new())));
    if let Some(item) = ui.shadow.iter().find(|i| s(i, "eventId") == event_id) {
        win.set_dc_name(item["media"]["filename"].as_str().unwrap_or("Document").into());
        win.set_dc_size(item["media"]["sizeLabel"].as_str().unwrap_or("").into());
    }
    call_ui(&ui.req.clone(), "doc.preview", json!({"roomId": room, "eventId": event_id}), move |ui, win, out| {
        match out {
            Ok(v) => {
                win.set_dc_status("".into());
                let kind = s(&v, "kind").to_string();
                win.set_dc_kind(kind.as_str().into());
                win.set_dc_subtitle(match kind.as_str() {
                    "pdf" => format!("PDF · {} pages", v["pages"].as_i64().unwrap_or(0)),
                    "sheet" => format!("Spreadsheet · {} sheets", v["sheets"].as_array().map(Vec::len).unwrap_or(0)),
                    k => k.to_string(),
                }.into());
                let pages = v["pages"].as_i64().unwrap_or(0);
                win.set_dc_pdf(pages > 0);
                if pages > 0 {
                    ui.doc_pages = vec![Value::Null; pages as usize];
                    let blanks: Vec<crate::DocPageImage> = (0..pages).map(|_| crate::DocPageImage::default()).collect();
                    win.set_dc_pages(ModelRc::new(VecModel::from(blanks)));
                }
                let blocks: Vec<crate::DocBlock> = v["blocks"].as_array().map(|a| a.iter().map(|b3| crate::DocBlock {
                    t: s(b3, "t").into(),
                    text: s(b3, "text").into(),
                    title: s(b3, "title").into(),
                    level: b3["level"].as_i64().unwrap_or(0) as i32,
                    bullet: b(b3, "bullet"),
                }).collect()).unwrap_or_default();
                win.set_dc_blocks(ModelRc::new(VecModel::from(blocks)));
                let sheets: Vec<crate::SheetTab> = v["sheets"].as_array().map(|a| a.iter().map(|t| crate::SheetTab { name: s(t, "name").into() }).collect()).unwrap_or_default();
                win.set_dc_sheets(ModelRc::new(VecModel::from(sheets)));
                doc_push_sheet(win, &v, 0);
                ui.doc_preview = v;
            }
            Err((_, m)) => {
                win.set_dc_status("error".into());
                win.set_dc_error(m.as_str().into());
            }
        }
    });
}

fn doc_push_sheet(win: &AppWindow, preview: &Value, index: usize) {
    let rows_v = preview["sheetRows"].as_array().cloned().unwrap_or_default();
    let mut cols = 1;
    let rows: Vec<crate::SheetRow> = rows_v.iter()
        .filter(|r| r["sheet"].as_i64().unwrap_or(0) as usize == index)
        .map(|r| {
            let cells: Vec<SharedString> = r["cells"].as_array().map(|a| a.iter().map(|c| SharedString::from(c.as_str().unwrap_or(""))).collect()).unwrap_or_default();
            cols = cols.max(cells.len() as i32);
            crate::SheetRow { cells: ModelRc::new(VecModel::from(cells)) }
        })
        .collect();
    win.set_dc_cols(cols);
    win.set_dc_rows(ModelRc::new(VecModel::from(rows)));
}

fn doc_page(ui: &mut UiState, _win: &AppWindow, index: &str, width: &str) {
    let idx: usize = index.parse().unwrap_or(0);
    if ui.doc_pages.get(idx).map(|p| !p.is_null()).unwrap_or(false) {
        return;
    }
    let (rid, ev) = ui.doc_ctx.clone();
    let w: f64 = width.parse().unwrap_or(800.0);
    call_ui(&ui.req.clone(), "doc.page", json!({"roomId": rid, "eventId": ev, "index": idx, "width": w as i64}), move |ui, win, out| {
        let Ok(v) = out else { return };
        if let Some(slot) = ui.doc_pages.get_mut(idx) {
            *slot = v.clone();
        }
        let path = s(&v, "path").to_string();
        if let Some(img) = crate::bridge::avatar_pub(ui, &path) {
            use slint::Model as _;
            let pages = win.get_dc_pages();
            if let Some(mut row) = pages.row_data(idx) {
                row.img = img;
                row.loaded = true;
                pages.set_row_data(idx, row);
            }
        }
    });
}

fn doc_sheet(ui: &mut UiState, win: &AppWindow, index: &str) {
    let idx: usize = index.parse().unwrap_or(0);
    let preview = ui.doc_preview.clone();
    doc_push_sheet(win, &preview, idx);
}

fn audio_open(ui: &mut UiState, win: &AppWindow, event_id: &str) {
    let room = room_of_key(&ui.open_room);
    ui.audio_ctx = (room.clone(), event_id.to_string());
    win.set_au_status("loading".into());
    win.set_au_playing(false);
    win.set_au_position(0.0);
    if let Some(item) = ui.shadow.iter().find(|i| s(i, "eventId") == event_id) {
        win.set_au_title(item["media"]["filename"].as_str().unwrap_or("Audio").into());
        win.set_au_size(item["media"]["sizeLabel"].as_str().unwrap_or("").into());
        win.set_au_duration((item["media"]["duration"].as_f64().unwrap_or(0.0) / 1000.0) as f32);
    }
    call_ui(&ui.req.clone(), "audio.info", json!({"roomId": room, "eventId": event_id, "size": 512}), move |ui, win, out| {
        win.set_au_status("".into());
        let Ok(v) = out else { return };
        if let Some(d) = v["duration"].as_f64() {
            if d > 0.0 { win.set_au_duration((d / 1000.0) as f32); }
        }
        let art = s(&v, "artPath").to_string();
        if let Some(img) = crate::bridge::avatar_pub(ui, &art) {
            win.set_au_art(img);
            win.set_au_have_art(true);
        }
        if let Some(hex) = v["accent"].as_str() {
            if let Ok(c) = u32::from_str_radix(hex.trim_start_matches('#'), 16) {
                win.set_au_tone(slint::Color::from_rgb_u8((c >> 16) as u8, (c >> 8) as u8, c as u8));
            }
        }
    });
}

fn audio_page_toggle(ui: &mut UiState, win: &AppWindow) {
    let (rid, ev) = ui.audio_ctx.clone();
    if win.get_au_playing() {
        ui.req.fire("audio.stop", json!({}));
        win.set_au_playing(false);
    } else {
        ui.req.fire("audio.play", json!({"roomId": rid, "eventId": ev, "seek": win.get_au_position() as f64}));
        win.set_au_playing(true);
        tick_audio(ui);
    }
}

fn tick_audio(ui: &mut UiState) {
    let req = ui.req.clone();
    after(&req, 250, |ui, win| {
        if win.get_au_playing() {
            win.set_au_position(win.get_au_position() + 0.25);
            if win.get_au_position() >= win.get_au_duration() && win.get_au_duration() > 0.0 {
                win.set_au_playing(false);
                win.set_au_position(0.0);
                return;
            }
            tick_audio(ui);
        }
    });
}

fn tick_recorder(ui: &mut UiState) {
    let req = ui.req.clone();
    after(&req, 100, |ui, win| {
        if ui.recording {
            win.set_rec_elapsed(win.get_rec_elapsed() + 0.1);
            tick_recorder(ui);
        }
    });
}

fn attach_files(ui: &mut UiState) {
    let room = room_of_key(&ui.open_room);
    ui.req.handle().spawn(async move {
        if let Some(path) = crate::platform::pick_file().await {
            let _ = slint::invoke_from_event_loop(move || {
                with_ui(|ui| {
                    ui.req.fire("attachment.send", json!({"roomId": room, "path": path}));
                    if let Some(win) = ui.win.upgrade() {
                        win.set_attach_open(false);
                    }
                });
            });
        }
    });
}

fn create_poll(ui: &mut UiState, question: &str, packed: &str) {
    let mut parts = packed.split('\u{1f}');
    let closed = parts.next().unwrap_or("0") == "1";
    let options: Vec<&str> = parts.filter(|o| !o.trim().is_empty()).collect();
    if options.len() < 2 || question.trim().is_empty() {
        return;
    }
    let room = room_of_key(&ui.open_room);
    ui.req.fire("poll.create", json!({"roomId": room, "question": question, "options": options, "closed": closed}));
    if let Some(win) = ui.win.upgrade() {
        win.set_attach_open(false);
    }
}

fn load_stickers(ui: &mut UiState, _win: &AppWindow) {
    call_ui(&ui.req.clone(), "stickers.list", json!({}), |ui, win, out| {
        let Ok(v) = out else { return };
        ui.stickers = v["stickers"].as_array().cloned().unwrap_or_default();
        let stickers = ui.stickers.clone();
        let rows: Vec<crate::StickerItem> = stickers.iter().map(|st| crate::StickerItem {
            path: s(st, "path").into(),
            art: crate::bridge::avatar_pub(ui, s(st, "path")).unwrap_or_default(),
            body: s(st, "body").into(),
            url: s(st, "url").into(),
            w: st["width"].as_i64().unwrap_or(0) as i32,
            h: st["height"].as_i64().unwrap_or(0) as i32,
        }).collect();
        win.set_at_stickers(ModelRc::new(VecModel::from(rows)));
        win.set_at_stickers_loaded(true);
    });
}

fn send_sticker(ui: &mut UiState, index: &str) {
    let idx: usize = index.parse().unwrap_or(0);
    let Some(st) = ui.stickers.get(idx).cloned() else { return };
    let room = room_of_key(&ui.open_room);
    ui.req.fire("sticker.send", json!({
        "roomId": room,
        "url": s(&st, "url"),
        "body": match s(&st, "body") { "" => "Sticker", b3 => b3 },
        "width": st["width"].as_i64().unwrap_or(0),
        "height": st["height"].as_i64().unwrap_or(0),
    }));
    if let Some(win) = ui.win.upgrade() {
        win.set_attach_open(false);
    }
}

// ---------------------------------------------------------------- theme

fn themes_path() -> String {
    format!("{}/.local/state/sigil/chat-themes.json", std::env::var("HOME").unwrap_or_default())
}

fn push_theme(ui: &mut UiState, win: &AppWindow) {
    let accent = ui.theme_pending["accent"].as_str().unwrap_or("").to_string();
    let wallpaper = ui.theme_pending["wallpaper"].as_str().unwrap_or("").to_string();
    win.set_ct_accent(accent.as_str().into());
    win.set_ct_wallpaper(wallpaper.as_str().into());
    if let Ok(c) = u32::from_str_radix(accent.trim_start_matches('#'), 16) {
        win.set_ct_color(slint::Color::from_rgb_u8((c >> 16) as u8, (c >> 8) as u8, c as u8));
    }
    win.set_ct_custom(!accent.is_empty());
    if let Some(n) = wallpaper.strip_prefix("grad:").and_then(|n| n.parse::<i32>().ok()) {
        win.set_ct_grad(n);
    } else {
        win.set_ct_grad(-1);
        if !wallpaper.is_empty() {
            if let Some(img) = crate::bridge::avatar_pub(ui, &wallpaper) {
                win.set_ct_wallpaper_img(img);
            }
        }
    }
}

fn theme_apply(ui: &mut UiState, win: &AppWindow) {
    let rid = room_of_key(&ui.open_room);
    // The same file Panel.qml writes, so both frontends share themes.
    let path = themes_path();
    let mut all: Value = std::fs::read_to_string(&path).ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}));
    let pending = ui.theme_pending.clone();
    let empty = pending["accent"].as_str().unwrap_or("").is_empty()
        && pending["wallpaper"].as_str().unwrap_or("").is_empty();
    if empty {
        if let Some(o) = all.as_object_mut() { o.remove(&rid); }
    } else {
        all[rid.as_str()] = json!({"accent": pending["accent"], "wallpaper": pending["wallpaper"]});
    }
    if let Some(dir) = std::path::Path::new(&path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, all.to_string());
    ui.chat_themes = all;
    win.invoke_go_back();
}

fn hsv_rgb(h: f32, s2: f32, v: f32) -> (u8, u8, u8) {
    let h6 = (h.fract() * 6.0).max(0.0);
    let c = v * s2;
    let x = c * (1.0 - (h6 % 2.0 - 1.0).abs());
    let (r, g, b3) = match h6 as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    (((r + m) * 255.0) as u8, ((g + m) * 255.0) as u8, ((b3 + m) * 255.0) as u8)
}
