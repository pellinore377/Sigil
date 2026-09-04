//! Service.qml's function table: what each page loads when it opens
//! (`nav-opened`) and what every page action does (`act`). The protocol names
//! come from core/docs/protocol.md; the behaviors from the WIRING-*.md
//! contracts and Service.qml itself.

use serde_json::{json, Value};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::bridge::{rebuild_rooms, rebuild_timeline, with_ui, Requester, UiState};
use crate::rows::{initials, tint_for};
use crate::{project, AppWindow, SheetRect, TimelineRow, UserRow};

pub fn room_of_key(key: &str) -> String {
    match key.find('|') {
        Some(i) => key[..i].to_string(),
        None => key.to_string(),
    }
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}
fn b(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}

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
    call_ui(
        req,
        "doc.thumb",
        json!({"roomId": room_id, "eventId": event_id, "size": 0}),
        move |ui, win, out| {
            let val = match out {
                Ok(v)
                    if v["lines"]
                        .as_array()
                        .map(|a| !a.is_empty())
                        .unwrap_or(false)
                        || !v["imagePath"].as_str().unwrap_or("").is_empty() =>
                {
                    v
                }
                _ => json!(false),
            };
            ui.doc_thumbs.insert(key, val);
            rebuild_timeline(ui, win);
        },
    );
}

/// Static map composite for a location card (location.map).
pub fn fetch_location_map(req: &Requester, geo_uri: String) {
    let key = geo_uri.clone();
    call_ui(
        req,
        "location.map",
        json!({"geoUri": geo_uri, "width": 640, "height": 400}),
        move |ui, win, out| {
            let val = match out {
                Ok(v) if !v["path"].as_str().unwrap_or("").is_empty() => v,
                Ok(v) => {
                    tracing::warn!("location.map: empty reply {v}");
                    json!(false)
                }
                Err((code, msg)) => {
                    tracing::warn!("location.map: {code} {msg}");
                    json!(false)
                }
            };
            ui.location_maps.insert(key, val);
            rebuild_timeline(ui, win);
        },
    );
}

/// The map page's composite: the page's card at 2× (~400×520 logical), at
/// the page's current zoom. A stale reply (the page moved on to another
/// point or zoom) is dropped.
fn fetch_map_page(ui: &mut UiState) {
    if ui.map_geo.is_empty() {
        return;
    }
    let (geo, zoom) = (ui.map_geo.clone(), ui.map_zoom);
    call_ui(
        &ui.req.clone(),
        "location.map",
        json!({"geoUri": geo.clone(), "width": 800, "height": 1040, "zoom": zoom}),
        move |ui, win, out| {
            if ui.map_geo != geo || ui.map_zoom != zoom {
                return;
            }
            match out {
                Ok(v) => {
                    if let Some(img) = crate::bridge::avatar_pub(ui, v["path"].as_str().unwrap_or("")) {
                        win.set_mp_map(img);
                    }
                }
                // No style configured: the page keeps its pin card.
                Err((code, msg)) => tracing::debug!("location.map (page): {code} {msg}"),
            }
        },
    );
}

/// Frame strip for an animated GIF (media.gifFrames) — Slint has no
/// animated Image, so the bubble cycles PNG frames. `path` is the local file
/// when the item already carries one, which saves the engine the lookup;
/// empty is fine, and the engine finds it from the event.
pub fn fetch_gif_frames(req: &Requester, room_id: &str, event_id: &str, path: &str, key: String) {
    call_ui(
        req,
        "media.gifFrames",
        json!({"roomId": room_id, "eventId": event_id, "path": path}),
        move |ui, win, out| {
            let val = match out {
                Ok(v) if v["frames"].as_array().map(|a| a.len() > 1).unwrap_or(false) => v,
                _ => json!(false),
            };
            ui.gif_frames.insert(key, val);
            rebuild_timeline(ui, win);
        },
    );
}

/// og: card for the first link in a message (svc.linkPreview).
pub fn fetch_link_preview(req: &Requester, url: String) {
    let key = url.clone();
    call_ui(
        req,
        "link.preview",
        json!({"url": url}),
        move |ui, win, out| {
            let val = match out {
                Ok(v) => v,
                _ => json!(false),
            };
            ui.link_previews.insert(key, val);
            rebuild_timeline(ui, win);
        },
    );
}

/// Cover art + palette for a music message's card (AudioBody's audio.info).
pub fn fetch_audio_info(req: &Requester, room_id: &str, event_id: &str, key: String) {
    call_ui(
        req,
        "audio.info",
        json!({"roomId": room_id, "eventId": event_id, "size": 512}),
        move |ui, win, out| {
            let val = match out {
                Ok(v) => v,
                _ => json!(false),
            };
            ui.audio_infos.insert(key, val);
            rebuild_timeline(ui, win);
        },
    );
}

/// Public face of `after` for the bridge's layout-settle scrolls.
pub fn after_pub(
    req: &Requester,
    ms: u64,
    f: impl FnOnce(&mut UiState, &AppWindow) + Send + 'static,
) {
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

/// A bubble's rect in the snapshot's pixels. `r` is logical, from the
/// window's origin; the picture is in whatever pixels the renderer hands
/// back, so it is scaled by the picture's own size over the window's logical
/// size rather than by a scale factor the two need not agree on. The cut is
/// rounded to whole pixels on both edges so it is never resampled against
/// the rect the sheet draws it in.
fn sheet_px(win: &AppWindow, snap: &crate::frost::Snapshot, r: &SheetRect) -> crate::frost::PixelRect {
    let logical = win.window().size().to_logical(win.window().scale_factor());
    let sx = snap.buf.width() as f32 / logical.width.max(1.0);
    let sy = snap.buf.height() as f32 / logical.height.max(1.0);
    let (x0, y0) = ((r.x * sx).round(), (r.y * sy).round());
    let (x1, y1) = (((r.x + r.w) * sx).round(), ((r.y + r.h) * sy).round());
    let s = sx.min(sy);
    crate::frost::PixelRect {
        x: x0,
        y: y0,
        w: x1 - x0,
        h: y1 - y0,
        radii: [r.r_tl * s, r.r_tr * s, r.r_bl * s, r.r_br * s],
    }
}

/// A press on a bubble is lasting: take the frost now, while the finger is
/// still and nothing moves, so the lift has every frame to itself when the
/// hold fires. Dropped unused if the press lets go early.
pub fn sheet_prewarm(ui: &mut UiState, win: &AppWindow, id: &str, r: &SheetRect) {
    let frost = crate::frost::Snapshot::take(win.window()).map(|mut snap| {
        snap.mask(sheet_px(win, &snap, r));
        snap.frosted()
    });
    ui.sheet_prewarm = frost.map(|f| (id.to_string(), f));
}

/// The hold fired: the frost from the prewarm, or from a picture taken now
/// if the press arrived without one (the test hook, a hold that fired at
/// once). The lifted message is not a picture at all — the sheet draws the
/// row a second time, so nothing that overlapped the bubble can reach it.
pub fn sheet_snapshot(ui: &mut UiState, win: &AppWindow, id: &str, r: &SheetRect) {
    // the platform's long-press buzz, the moment the hold fires
    #[cfg(target_os = "android")]
    i_slint_backend_android_activity::haptic_long_press();
    let snap = crate::frost::Snapshot::take(win.window());
    let warm = match ui.sheet_prewarm.take() {
        Some((warm_id, img)) if warm_id == id => Some(img),
        _ => None,
    };
    let frost = warm.or_else(|| {
        snap.map(|mut snap| {
            snap.mask(sheet_px(win, &snap, r));
            snap.frosted()
        })
    });
    win.set_sheet_backdrop(frost.unwrap_or_default());
}

// ------------------------------------------------------- the location map

/// Place the grid for where the view is now, and ask for what is missing.
/// Called on every drag step, so it does no work beyond the arithmetic and a
/// cache lookup per visible tile; the fetches answer later and place
/// themselves.
pub fn map_place(ui: &mut UiState, win: &AppWindow) {
    let wanted = ui.mapview.wanted();
    // Mid-pinch the grid is drawn magnified, and at a magnification that is
    // rarely a whole number of pixels. The view rounds each tile's edges onto
    // whole *device* pixels, so it needs to know how many of those there are
    // to a logical one; without it adjacent tiles are feathered against the
    // ground and every join shows as a hairline.
    let dpr = f64::from(win.window().scale_factor());
    let mut rows: Vec<crate::MapTileView> = Vec::with_capacity(wanted.len());
    let mut missing: Vec<(u32, i64, i64)> = Vec::new();
    for (tx, ty) in wanted {
        let key = ui.mapview.key(tx, ty);
        let (x, y, w, h) = ui.mapview.place(tx, ty, dpr);
        match ui.mapview.have.get(&key) {
            Some(img) => rows.push(crate::MapTileView {
                x: x.into(),
                y: y.into(),
                w: w.into(),
                h: h.into(),
                img: img.clone(),
            }),
            None => {
                if ui.mapview.asked.insert(key) {
                    missing.push(key);
                }
            }
        }
    }
    let (px, py) = ui.mapview.pin();
    win.set_mp_pin_x(px.into());
    win.set_mp_pin_y(py.into());
    win.set_mp_tiles(ModelRc::new(VecModel::from(rows)));
    for (z, x, y) in missing {
        fetch_map_tile(ui, z, x, y);
    }
}

/// One rendered tile from the engine. A tile is the same picture whatever
/// page asked for it, so what comes back is kept for the session.
fn fetch_map_tile(ui: &mut UiState, z: u32, x: i64, y: i64) {
    let epoch = ui.mapview.epoch;
    call_ui(
        &ui.req.clone(),
        "map.tile",
        json!({"z": z, "x": x, "y": y}),
        move |ui, win, out| {
            ui.mapview.asked.remove(&(z, x, y));
            // The page moved to another point while this was in flight.
            if ui.mapview.epoch != epoch {
                return;
            }
            match out {
                Ok(v) => {
                    let path = v["path"].as_str().unwrap_or("");
                    if let Some(img) = crate::bridge::avatar_pub(ui, path) {
                        ui.mapview.have.insert((z, x, y), img);
                        // Only the current zoom is on screen; a tile that
                        // arrives for a zoom left behind just goes to the cache.
                        if ui.mapview.z == z {
                            map_place(ui, win);
                        }
                    }
                }
                // No tile server configured, or the tile would not render:
                // the ground shows through, and the composite still stands.
                Err((code, msg)) => tracing::debug!("map.tile {z}/{x}/{y}: {code} {msg}"),
            }
        },
    );
}

pub fn wire_extra(win: &AppWindow) {
    win.on_map_viewport(|w, h| {
        with_ui(|ui| {
            let Some(win) = ui.win.upgrade() else { return };
            ui.mapview.resize(w as f64, h as f64);
            map_place(ui, &win);
        });
    });
    win.on_map_panned(|dx, dy| {
        with_ui(|ui| {
            let Some(win) = ui.win.upgrade() else { return };
            ui.mapview.pan(dx as f64, dy as f64);
            map_place(ui, &win);
        });
    });
    // A step in or out about a tapped spot — and, with no step, the recentre
    // disc top right (MapPage.qml:351-366): the page has no callback of its
    // own for that, and this is the one that already carries a place on the
    // map to look at.
    win.on_map_zoom_at(|step, x, y| {
        with_ui(|ui| {
            let Some(win) = ui.win.upgrade() else { return };
            if step == 0 {
                ui.mapview.recentre();
            } else {
                ui.mapview.zoom(step, x as f64, y as f64);
            }
            map_place(ui, &win);
        });
    });
    win.on_map_pinch_begin(|| {
        with_ui(|ui| ui.mapview.pinch_begin());
    });
    win.on_map_pinched(|f, x, y| {
        with_ui(|ui| {
            let Some(win) = ui.win.upgrade() else { return };
            ui.mapview.pinch_to(f as f64, x as f64, y as f64);
            map_place(ui, &win);
        });
    });
    // The fingers lifted. Nothing eases anywhere: the view stays exactly
    // where they left it and only the level the tiles are fetched from
    // settles, so what used to be a lurch of up to half a level on every lift
    // is now the map simply staying put while sharper imagery arrives.
    win.on_map_pinch_end(|| {
        with_ui(|ui| {
            let Some(win) = ui.win.upgrade() else { return };
            ui.mapview.pinch_end();
            map_place(ui, &win);
        });
    });
    win.on_sheet_prewarm(|id, r| {
        with_ui(|ui| {
            let Some(win) = ui.win.upgrade() else { return };
            sheet_prewarm(ui, &win, id.as_str(), &r);
        });
    });
    win.on_sheet_snapshot(|id, r| {
        with_ui(|ui| {
            let Some(win) = ui.win.upgrade() else { return };
            sheet_snapshot(ui, &win, id.as_str(), &r);
        });
    });
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
        // Back from a thread: the conversation itself is the view again.
        "chat" => {
            if ui.open_room.contains("|thread:") {
                let key = ui.open_room.clone();
                let room = room_of_key(&key);
                ui.req.fire("room.close", json!({"roomId": key}));
                win.set_chat_is_thread(false);
                win.set_thread_key("".into());
                crate::bridge::open_room(ui, win, &room);
            }
        }
        "roomsettings" => {
            ui.settings_room = room_of_key(&ui.open_room);
            load_settings(ui, win);
            load_members(ui, win);
        }
        "notifications" | "privacy" => {
            load_settings(ui, win);
        }
        "admins" => {
            win.set_ad_note(SharedString::new());
            load_settings(ui, win);
            load_members(ui, win);
        }
        "members" => load_members(ui, win),
        "settings" => crate::bridge::load_settings_page(ui),
        // The page edits a copy seeded from the room's saved theme, exactly
        // what ChatThemePage.qml starts `pending` as; without the seed an
        // immediate Apply would wipe the theme, and the gradient grid (drawn
        // from the pending accent) would start empty.
        "chattheme" => {
            let rid = room_of_key(&ui.open_room);
            ui.theme_pending = ui.chat_themes.get(rid.as_str()).cloned().unwrap_or_else(|| json!({}));
            push_theme(ui, win);
        }
        "threads" => load_threads(ui, win),
        "pins" => load_pins(ui, win),
        "search" => {
            ui.search_query.clear();
            if phone(win) {
                win.set_se_kind(SharedString::new());
                global_search(ui, win);
            } else {
                rebuild_search(ui, win);
            }
        }
        "forward" => {
            ui.forward_query.clear();
            rebuild_forward(ui, win);
        }
        "start" => {
            win.set_st_error(SharedString::new());
            // The page opens on an empty field, and nothing fires
            // `search-edited` until something is typed: ask for the
            // suggestions here, the same way an emptied field does.
            dir_search(ui, win, "dir-search-start", "");
        }
        "addpeople" => win.set_ap_results(ModelRc::new(VecModel::from(Vec::new()))),
        // Back on Home: bank the open room's unsent composer text so its row
        // can wear the "Draft:" preview (Panel.qml drafts).
        "home" => {
            if !ui.open_room.is_empty() {
                let rid = room_of_key(&ui.open_room);
                let text = win.get_ct_composer_text().to_string();
                if text.trim().is_empty() {
                    ui.drafts.remove(&rid);
                } else {
                    ui.drafts.insert(rid, text);
                }
                crate::bridge::rebuild_rooms(ui, win);
            }
        }
        _ => {}
    }
}

fn load_settings(ui: &mut UiState, _win: &AppWindow) {
    if ui.settings_room.is_empty() {
        ui.settings_room = room_of_key(&ui.open_room);
    }
    let rid = ui.settings_room.clone();
    call_ui(
        &ui.req.clone(),
        "room.settings",
        json!({"roomId": rid}),
        move |ui, win, out| {
            let Ok(v) = out else { return };
            ui.settings = v;
            push_settings(ui, win);
        },
    );
}

/// Everything derived from ui.settings, pushed to the pages at once.
pub fn push_settings(ui: &mut UiState, win: &AppWindow) {
    let rid = ui.settings_room.clone();
    let room = ui
        .rooms_json
        .iter()
        .find(|r| s(r, "id") == rid)
        .cloned()
        .unwrap_or(Value::Null);
    let av = crate::bridge::avatar_pub(ui, room["avatarPath"].as_str().unwrap_or(""))
        .unwrap_or_default();
    win.set_rs_model(project::settings_model(&rid, &ui.settings, &room, av));
    win.set_rs_pinned_count(ui.rooms_json.iter().filter(|r| b(r, "isFavourite")).count() as i32);
    win.set_pv_server(s(&ui.settings, "slotServer").into());
    win.set_pv_epochs(ui.settings["epochs"].as_i64().unwrap_or(1) as i32);
    win.set_ad_can(b(&ui.settings, "isAdmin"));
    win.set_rs_dm_user(s(&room, "dmUserId").into());
    win.set_no_mode(
        match s(&ui.settings, "notificationMode") {
            "default" => "",
            m => m,
        }
        .into(),
    );
}

fn load_members(ui: &mut UiState, _win: &AppWindow) {
    if ui.settings_room.is_empty() {
        ui.settings_room = room_of_key(&ui.open_room);
    }
    let rid = ui.settings_room.clone();
    call_ui(
        &ui.req.clone(),
        "room.members",
        json!({"roomId": rid}),
        move |ui, win, out| {
            let Ok(v) = out else { return };
            ui.members = v["members"].as_array().cloned().unwrap_or_default();
            push_members(ui, win);
        },
    );
}

pub fn push_members(ui: &mut UiState, win: &AppWindow) {
    let filter = ui.members_filter;
    win.set_me_all(ui.members.len() as i32);
    win.set_me_filter(filter as i32);
    let rows: Vec<_> = ui
        .members
        .iter()
        .filter(|m| {
            m.get("membership")
                .and_then(Value::as_str)
                .unwrap_or("join")
                == "join"
        })
        .filter(|m| {
            let l = m["powerLevel"].as_i64().unwrap_or(0);
            match filter {
                100 => l >= 100,
                50 => (50..100).contains(&l),
                _ => true,
            }
        })
        .map(|m| project::member_row(m))
        .collect();
    win.set_me_members(ModelRc::new(VecModel::from(rows)));
    let preview: Vec<_> = ui
        .members
        .iter()
        .take(8)
        .map(|m| project::member_row(m))
        .collect();
    win.set_rs_members(ModelRc::new(VecModel::from(preview)));
    let everyone: Vec<_> = ui.members.iter().map(project::member_row).collect();
    win.set_ad_admins(everyone.iter().filter(|m| m.power_level >= 100).count() as i32);
    win.set_ad_members(ModelRc::new(VecModel::from(everyone)));
}

fn load_threads(ui: &mut UiState, win: &AppWindow) {
    let rid = room_of_key(&ui.open_room);
    win.set_th_loading(true);
    call_ui(
        &ui.req.clone(),
        "threads.list",
        json!({"roomId": rid}),
        move |_ui, win, out| {
            win.set_th_loading(false);
            win.set_th_loaded(true);
            if let Ok(v) = out {
                let rows: Vec<_> = v["threads"]
                    .as_array()
                    .map(|a| a.iter().map(project::thread_row).collect())
                    .unwrap_or_default();
                win.set_th_threads(ModelRc::new(VecModel::from(rows)));
            }
        },
    );
}

fn load_pins(ui: &mut UiState, win: &AppWindow) {
    let rid = room_of_key(&ui.open_room);
    let is_dm = ui
        .rooms_json
        .iter()
        .find(|r| s(r, "id") == rid)
        .map(|r| b(r, "isDm"))
        .unwrap_or(false);
    win.set_pi_dm(is_dm);
    win.set_pi_loading(true);
    call_ui(
        &ui.req.clone(),
        "pins.items",
        json!({"roomId": rid}),
        move |ui, win, out| {
            win.set_pi_loading(false);
            win.set_pi_loaded(true);
            if let Ok(v) = out {
                let items = v["items"].as_array().cloned().unwrap_or_default();
                let rows: Vec<TimelineRow> = items
                    .iter()
                    .map(|it| {
                        let mut row = simple_row(ui, it);
                        row.stamp = project::pin_stamp(it["ts"].as_i64().unwrap_or(0)).into();
                        row.media_filename = project::kind_words(s(it, "kind")).into();
                        row
                    })
                    .collect();
                win.set_pi_items(ModelRc::new(VecModel::from(rows)));
            }
        },
    );
}

/// pins.list reply → pinned ids cache → pin markers on bubbles.
pub fn load_pinned_ids(ui: &mut UiState, room_id: &str) {
    let rid = room_id.to_string();
    call_ui(
        &ui.req.clone(),
        "pins.list",
        json!({"roomId": rid.clone()}),
        move |ui, win, out| {
            if let Ok(v) = out {
                let ids: Vec<String> = v["events"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|e| e.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                ui.pinned_by_room.insert(rid, ids);
                rebuild_timeline(ui, win);
            }
        },
    );
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

fn phone(win: &AppWindow) -> bool {
    win.global::<crate::Theme>().get_mode() != "desktop"
}

/// The phone's search: the engine matches the query and chip over every
/// room (or the open one, from a chat); rooms and messages come back as rows.
fn global_search(ui: &mut UiState, win: &AppWindow) {
    let q = ui.search_query.trim().to_string();
    let kind = win.get_se_kind().to_string();
    ui.search_epoch += 1;
    let epoch = ui.search_epoch;
    if q.is_empty() && kind.is_empty() {
        win.set_se_rooms(ModelRc::new(VecModel::from(Vec::new())));
        win.set_se_hits(ModelRc::new(VecModel::from(Vec::new())));
        return;
    }
    let scope = if win.get_se_global() { String::new() } else { room_of_key(&ui.open_room) };
    call_ui(
        &ui.req.clone(),
        "search.global",
        json!({"query": q, "kind": kind, "roomId": scope}),
        move |ui, win, out| {
            if ui.search_epoch != epoch {
                return; // superseded
            }
            let v = out.unwrap_or_else(|_| json!({}));
            let rooms: Vec<_> = v["rooms"]
                .as_array()
                .map(|a| a.iter().map(|r| crate::bridge::room_row_of(ui, r)).collect())
                .unwrap_or_default();
            let hits: Vec<_> = v["messages"]
                .as_array()
                .map(|a| a.iter().map(|m| hit_row(ui, m)).collect())
                .unwrap_or_default();
            win.set_se_rooms(ModelRc::new(VecModel::from(rooms)));
            win.set_se_hits(ModelRc::new(VecModel::from(hits)));
        },
    );
}

fn hit_row(ui: &mut UiState, m: &Value) -> crate::SearchHit {
    let rid = s(m, "roomId").to_string();
    let name = s(m, "roomName").to_string();
    let room = ui.rooms_json.iter().find(|r| s(r, "id") == rid).cloned().unwrap_or(Value::Null);
    let tint_key = match s(&room, "dmUserId") {
        "" => rid.clone(),
        u => u.to_string(),
    };
    let icon = match s(m, "kind") {
        "image" => ui.icons.camera.clone(),
        "video" => ui.icons.video_on.clone(),
        "audio" | "voice" => ui.icons.mic_on.clone(),
        "file" => ui.icons.attach.clone(),
        "location" | "liveLocation" => ui.icons.location.clone(),
        _ => Default::default(),
    };
    crate::SearchHit {
        room_id: rid.into(),
        event_id: s(m, "eventId").into(),
        room_name: name.clone().into(),
        initials: initials(&name).into(),
        avatar: crate::bridge::avatar_pub(ui, s(&room, "avatarPath")).unwrap_or_default(),
        tint: tint_for(&tint_key),
        body: s(m, "body").into(),
        icon,
        stamp: crate::rows::home_stamp(m["ts"].as_i64().unwrap_or(0)).into(),
    }
}

fn rebuild_forward(ui: &mut UiState, win: &AppWindow) {
    let q = ui.forward_query.to_lowercase();
    let rooms = ui.rooms_json.clone();
    let rows: Vec<_> = rooms
        .iter()
        .filter(|r| !b(r, "isSpace") && !b(r, "isInvite"))
        .filter(|r| q.is_empty() || s(r, "name").to_lowercase().contains(&q))
        .map(|r| crate::bridge::room_row_of(ui, r))
        .collect();
    win.set_fw_chats(ModelRc::new(VecModel::from(rows)));
}

/// What the Start page shows before anything is typed, from the room list
/// this device already holds: whoever we have a direct conversation with.
///
/// It is deliberately not the server's list of users. The front desk knows
/// every name it hosts, but the wire has no way to ask it for them
/// (`docs/blind-backend.md` A2/B3, `protocol/src/wire.rs`) — a directory
/// anyone could page through would hand out every account on the server.
/// So suggestions are people we already know; `dir_search` then widens this
/// with the engine's fuller list (saved contacts and everyone in our
/// conversations, groups included) when it answers.
fn rebuild_start_suggestions(ui: &mut UiState, win: &AppWindow) {
    let rows: Vec<_> = ui
        .rooms_json
        .iter()
        .filter(|r| b(r, "isDm"))
        .take(12)
        .map(|r| {
            project::user_row(
                &json!({"userId": s(r, "dmUserId"), "displayName": s(r, "name")}),
                false,
            )
        })
        .collect();
    win.set_st_people(ModelRc::new(VecModel::from(rows)));
}

// ---------------------------------------------------------------- rows

fn simple_row_pure(item: &Value) -> TimelineRow {
    let sender = s(item, "sender").to_string();
    let name = match s(item, "senderName") {
        "" => sender.clone(),
        d => d.to_string(),
    };
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
    let thumb = item["media"]["thumbnailPath"]
        .as_str()
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
        after(&req, 400, |ui, _win| {
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
    // The event carries mxc + path only — no roomId/eventId — and `thumbnail`
    // is a "WxH" string, not a bool. Patch every open-room item on that mxc.
    let mxc = s(v, "mxc").to_string();
    let path = s(v, "path").to_string();
    let thumb = v["thumbnail"]
        .as_str()
        .map(|t| !t.is_empty())
        .unwrap_or(false);
    if mxc.is_empty() || path.is_empty() {
        return;
    }
    let mut touched = false;
    for item in ui.shadow.iter_mut() {
        if item["media"]["mxc"].as_str() == Some(mxc.as_str()) {
            item["media"][if thumb { "thumbnailPath" } else { "path" }] = json!(path);
            touched = true;
        }
    }
    if touched {
        rebuild_timeline(ui, win);
    }
}

// ---------------------------------------------------------------- act()

pub fn on_act(ui: &mut UiState, win: &AppWindow, action: &str, a: &str, b2: &str) {
    let req = ui.req.clone();
    let open_room = room_of_key(&ui.open_room);
    let action_is_doc = action == "doc-download";
    match action {
        // ChatPage.qml:1754 trims; a body of nothing but spaces never leaves
        "send-reply" if b2.trim().is_empty() => {}
        "send-reply" => req.fire(
            "message.reply",
            json!({"roomId": ui.open_room, "eventId": a, "body": b2.trim(), "markdown": true}),
        ),
        "send-edit" if b2.trim().is_empty() => {}
        "send-edit" => req.fire(
            "message.edit",
            json!({"roomId": ui.open_room, "eventId": a, "body": b2.trim(), "markdown": true}),
        ),
        "autocomplete-pick" => autocomplete_pick(ui, win, a.parse().unwrap_or(0)),
        "react" => {
            // The picker's RECENTS block follows what was actually used.
            note_emoji_used(b2);
            req.fire(
                "message.react",
                json!({"roomId": ui.open_room, "eventId": a, "key": b2}),
            )
        }
        "paginate" => {
            let state = ui
                .pagination_by_room
                .get(&ui.open_room)
                .map(String::as_str)
                .unwrap_or("idle");
            if state == "idle" {
                req.fire(
                    "timeline.paginate",
                    json!({"roomId": ui.open_room, "count": 50}),
                );
            }
        }
        "mark-read" => req.fire("room.markRead", json!({"roomId": open_room})),
        // A message row's swipe has passed its detent, so letting go now will
        // reply (or reply in a thread): the platform's buzz at the moment the
        // gesture commits, exactly as the long-press sheet gets one.
        "swipe-detent" => {
            #[cfg(target_os = "android")]
            i_slint_backend_android_activity::haptic_long_press();
        }
        // ---- the home list's long-press bar. `a` is always the room id.
        // The platform's buzz the moment the hold fires, as the message
        // sheet gets one (sheet_snapshot).
        "home-hold" => {
            #[cfg(target_os = "android")]
            i_slint_backend_android_activity::haptic_long_press();
        }
        // The engine caps pins at five and says so. Home's pill is set
        // optimistically in the page, so the refusal is only logged until the
        // window carries a `home-note` property the reply can land in.
        "home-pin" => {
            let on = b2 == "1";
            call_ui(
                &req,
                "room.setFavourite",
                json!({"roomId": a, "favourite": on}),
                move |_ui, _win, out| {
                    if let Err((_, m)) = out {
                        tracing::warn!("room.setFavourite refused: {m}");
                    }
                },
            );
        }
        // b2 is "<span>,<mentions>". The dialog names a span; the deadline is
        // worked out here, where the clock is. "off" (or anything else) lifts
        // the snooze; "always" runs until it is lifted by hand.
        "home-snooze" => {
            let (span, mentions) = b2.split_once(',').unwrap_or((b2, "0"));
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            let until = match span {
                "1h" => now + 3_600_000,
                "8h" => now + 28_800_000,
                "24h" => now + 86_400_000,
                "always" => i64::MAX,
                _ => 0,
            };
            req.fire(
                "room.setSnooze",
                json!({"roomId": a, "until": until, "mentions": mentions == "1"}),
            );
        }
        // b2 "1" marks unread by hand; "0" reads it, which also lifts the mark.
        "home-unread" if b2 == "1" => {
            req.fire("room.setUnread", json!({"roomId": a, "unread": true}))
        }
        "home-unread" => req.fire("room.markRead", json!({"roomId": a})),
        "home-leave" => {
            let rid = a.to_string();
            call_ui(
                &req,
                "room.leave",
                json!({"roomId": rid}),
                move |ui, win, out| {
                    if let Err((_, m)) = out {
                        tracing::warn!("room.leave refused: {m}");
                        return;
                    }
                    // The room that was open has gone with it.
                    if ui.open_room == rid {
                        ui.open_room.clear();
                        win.set_nav("home".into());
                    }
                },
            );
        }
        // PollBody.pick(): single-select taps toggle (own answer again retracts);
        // multi-select builds the selection set up to maxSelections.
        // Caption edit on a media event: empty body clears the caption.
        "send-caption" => {
            req.fire(
                "message.editCaption",
                json!({"roomId": open_room, "eventId": a, "body": b2.trim()}),
            );
            win.invoke_clear_composer();
            crate::composer::reset(&win);
        }
        "vote" => {
            let poll = ui
                .shadow
                .iter()
                .find(|i| i["eventId"].as_str() == Some(a))
                .map(|i| i["poll"].clone())
                .unwrap_or(serde_json::Value::Null);
            let max = poll["maxSelections"].as_i64().unwrap_or(1).max(1);
            let mine: Vec<String> = poll["answers"]
                .as_array()
                .map(|ans| {
                    ans.iter()
                        .filter(|o| o["mine"].as_bool().unwrap_or(false))
                        .filter_map(|o| o["id"].as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let tapped_mine = mine.iter().any(|m| m == b2);
            let before = mine.clone();
            let answers: Option<Vec<String>> = if max <= 1 {
                Some(if tapped_mine {
                    vec![]
                } else {
                    vec![b2.to_string()]
                })
            } else if tapped_mine {
                Some(mine.into_iter().filter(|m| m != b2).collect())
            } else if (mine.len() as i64) < max {
                let mut sel = mine;
                sel.push(b2.to_string());
                Some(sel)
            } else {
                None // selection full: QML pick() bails
            };
            if let Some(answers) = answers {
                // The engine applies a vote only once the event has reached
                // the server (core's poll_vote awaits send_raw before
                // apply_vote), so nothing at all moves under the finger until
                // then — two and a half seconds even over loopback in
                // tests/e2e-kinds.sh. Paint the answer here and now; the
                // engine's own timeline diff replaces it moments later, and a
                // vote that never left puts the old one back.
                let event_id = a.to_string();
                poll_echo(ui, a, &answers);
                rebuild_timeline(ui, win);
                call_ui(
                    &req,
                    "poll.vote",
                    json!({"roomId": ui.open_room, "eventId": a, "answers": answers}),
                    move |ui, win, out| {
                        let Err((_, msg)) = out else { return };
                        tracing::warn!("poll.vote refused: {msg}");
                        poll_echo(ui, &event_id, &before);
                        rebuild_timeline(ui, win);
                        win.set_chat_toast("Vote not sent".into());
                    },
                );
            }
        }
        // ---- calls: the media stack lives in crate::call ----
        // A call needs the same runtime mic grant as the recorder; ask and
        // bail — the person taps again once the dialog is answered.
        #[cfg(target_os = "android")]
        "start-call" | "join-call" | "call-accept" if !crate::platform::has_mic_permission() => {
            crate::platform::request_mic_permission();
        }
        "start-call" => {
            let room = room_of_key(&ui.open_room);
            crate::call::start(ui, win, &room);
        }
        "join-call" => {
            let room = room_of_key(&ui.open_room);
            if let Some(call_id) = ui.calls.active.get(&room).cloned() {
                crate::call::join(ui, win, &room, &call_id, false);
            }
        }
        "call-accept" => {
            if let Some(inc) = ui.calls.incoming.take() {
                crate::call::join(ui, win, &inc.room_id, &inc.call_id, false);
                crate::bridge::open_room(ui, win, &inc.room_id);
                win.set_nav("chat".into());
            }
        }
        "call-decline" => crate::call::decline(ui, win),
        "hang-up" => crate::call::hangup(ui, win),
        "set-mic" => crate::call::set_mic(ui, win, a != "true"),
        "call-react" => crate::call::react(ui, win, a),
        "select-device" => crate::call::select_device(ui, win, a, b2),
        // the page clears its floaters when it hides (CallPage.qml:25)
        "call-minimize" => {
            win.set_call_page_open(false);
            if let Some(s) = ui.calls.session.as_mut() {
                s.floaters.clear();
            }
        }
        // video is not built yet: the toggles have nothing to switch
        "set-camera" | "set-screenshare" => {}
        "call-expand" => win.set_call_page_open(true),
        "accept-invite" => req.fire("room.join", json!({"roomIdOrAlias": open_room})),
        "decline-invite" => {
            req.fire("room.leave", json!({"roomId": open_room}));
            win.set_nav("home".into());
        }
        "open-thread" => {
            call_ui(
                &req,
                "thread.open",
                json!({"roomId": open_room, "rootId": a, "initialItems": 60}),
                move |ui, win, out| {
                    if let Ok(v) = out {
                        let key = s(&v, "key").to_string();
                        if !key.is_empty() {
                            crate::bridge::switch_timeline(ui, win, &key);
                            win.set_thread_key(key.as_str().into());
                            win.set_chat_is_thread(true);
                            win.set_nav("thread".into());
                        }
                    }
                },
            );
        }
        "menu-action" => menu_action(ui, win, a, b2),
        "voice-toggle" => voice_toggle(ui, win, a),
        "voice-seek" => voice_seek(ui, win, a, b2.parse().unwrap_or(0.0)),
        "emoji-search" => push_emoji(ui, win, a),
        "jump-to" => win.invoke_scroll_timeline_to_end(),
        "open-link" => crate::platform::open_url(a),

        "theme-photo" => pick_file(ui, action),
        // A file already on disk (a drop, a share, the test driver): sent as is.
        "attach-path" => {
            let room = room_of_key(&ui.open_room);
            req.fire("attachment.send", json!({"roomId": room, "path": a}));
            win.set_attach_open(false);
        }
        "set-favourite" => req.fire(
            "room.setFavourite",
            json!({"roomId": ui.settings_room, "favourite": a == "true"}),
        ),
        "set-low-priority" => req.fire(
            "room.setLowPriority",
            json!({"roomId": ui.settings_room, "lowPriority": a == "true"}),
        ),
        "rename" => {
            let rid = ui.settings_room.clone();
            call_ui(
                &req,
                "room.setSettings",
                json!({"roomId": rid, "name": a.trim()}),
                move |ui, win, out| {
                    if let Err((_, m)) = out {
                        win.set_ad_note(m.as_str().into());
                    }
                    load_settings(ui, win);
                },
            );
        }
        "set-admin" => {
            let rid = ui.settings_room.clone();
            let key = if b2 == "true" { "add" } else { "remove" };
            win.set_ad_busy(true);
            win.set_ad_note(SharedString::new());
            call_ui(
                &req,
                "room.setAdmins",
                json!({"roomId": rid, key: [a]}),
                move |ui, win, out| {
                    win.set_ad_busy(false);
                    if let Err((_, m)) = out {
                        win.set_ad_note(m.as_str().into());
                    }
                    load_settings(ui, win);
                    load_members(ui, win);
                },
            );
        }
        "leave-room" => {
            let rid = ui.settings_room.clone();
            call_ui(
                &req,
                "room.leave",
                json!({"roomId": rid}),
                move |_ui, win, _out| {
                    win.set_nav("home".into());
                },
            );
        }
        "notif-mode" => {
            win.set_no_mode(if a == "default" { "" } else { a }.into());
            win.set_no_busy(true);
            let rid = ui.settings_room.clone();
            call_ui(
                &req,
                "room.setSettings",
                json!({"roomId": rid, "notificationMode": a}),
                move |ui, win, out| {
                    win.set_no_busy(false);
                    if let Err((_, m)) = out {
                        win.set_no_error(m.as_str().into());
                    }
                    reread_settings_later(ui);
                },
            );
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
            call_ui(
                &req,
                "room.invite",
                json!({"roomId": rid, "userId": uid.clone()}),
                move |_ui, win, out| {
                    win.set_ap_note(
                        match out {
                            Ok(_) => format!("Invited {uid}"),
                            Err((_, m)) => format!("Invite failed: {m}"),
                        }
                        .into(),
                    );
                },
            );
        }

        "start-dm" => {
            call_ui(
                &req,
                "dm.create",
                json!({"userId": a}),
                move |ui, win, out| match out {
                    Ok(v) => {
                        let rid = s(&v, "roomId").to_string();
                        if !rid.is_empty() {
                            crate::bridge::open_room(ui, win, &rid);
                            win.set_nav("chat".into());
                        }
                    }
                    Err((_, m)) => win.set_st_error(m.as_str().into()),
                },
            );
        }
        "start-submit" => start_submit(ui, win, a, b2),
        "forward-search" => {
            ui.forward_query = a.to_string();
            rebuild_forward(ui, win);
        }
        "forward-picked" => forward_picked(ui, win, a),
        "room-search" => {
            ui.search_query = a.to_string();
            if phone(win) {
                global_search(ui, win);
            } else {
                rebuild_search(ui, win);
            }
        }
        "search-kind" => {
            win.set_se_kind(a.into());
            global_search(ui, win);
        }
        // A message hit: its room, then the timeline's end (as jump-to does).
        "open-hit" => {
            if open_room != a {
                crate::bridge::open_room(ui, win, a);
            }
            win.set_nav("chat".into());
            after(&req, 500, |_ui, win| win.invoke_scroll_timeline_to_end());
        }
        "unpin" => req.fire("message.unpin", json!({"roomId": open_room, "eventId": a})),

        // ---- viewer ----
        "viewer-open" => viewer_open(ui, win, a),
        "viewer-closed" => {
            // The viewer's own video, not the voice note the composer may be
            // playing: closing on a paused clip used to stop nothing, and
            // closing while a voice note played used to stop a video that was
            // not running (ImageViewer.qml:204 stops whatever the viewer owns).
            video_end(&req, win);
            // The fade is over by the time this arrives, so the frosted
            // picture of the page can go with it.
            win.global::<crate::Theme>().set_viewer_frost(Default::default());
        }
        "viewer-page" => {
            // Turning the page leaves the old clip behind: QML stops playback
            // in onCurChanged, or the decoder runs on under the next picture.
            video_end(&req, win);
            let i: usize = a.parse().unwrap_or(0);
            if let Some(item) = ui.viewer_items.get(i).cloned() {
                let ev = s(&item, "eventId");
                if item["media"]["path"].as_str().unwrap_or("").is_empty() && !ev.is_empty() {
                    req.fire("media.get", json!({"roomId": open_room, "eventId": ev}));
                }
                // A GIF that the timeline never cached (it only looks at
                // kind == "image", while the viewer admits stickers too) has
                // no frames yet; ask for them the way the bridge does.
                gif_frames_for_viewer(ui, &item);
            }
        }
        "viewer-download" | "viewer-delete" | "viewer-react" | "viewer-forward"
        | "viewer-share" => {
            viewer_misc(ui, win, action, a);
        }
        "viewer-playback" => {
            let i = win.get_vw_cur().max(0) as usize;
            if let Some(item) = ui.viewer_items.get(i) {
                let ev = s(item, "eventId").to_string();
                if win.get_vw_playing_event() == ev.as_str() {
                    video_toggle(&req, win, &open_room);
                } else {
                    let file = item["media"]["path"].as_str().unwrap_or("").to_string();
                    video_play(&req, win, &open_room, &ev, &file, 0.0);
                }
            }
        }
        "viewer-seek" => video_seek(&req, win, &open_room, a.parse::<f64>().unwrap_or(0.0)),
        // ImageViewer.qml:659-668 — the thumb follows the finger and the poll
        // stops writing over it until the finger lifts.
        "viewer-scrub" => {
            let on = a == "true";
            VIDEO.with(|v| v.borrow_mut().scrubbing = on);
            if on {
                win.set_vw_play_pos(b2.parse::<f32>().unwrap_or(0.0));
            }
        }

        // ---- doc / audio pages ----
        "open-doc" => doc_open(ui, win, a),
        "doc-download" | "audio-download" => {
            let (rid, ev) = if action_is_doc {
                ui.doc_ctx.clone()
            } else {
                ui.audio_ctx.clone()
            };
            let dest = format!("{}/Downloads", std::env::var("HOME").unwrap_or_default());
            call_ui(
                &req,
                "media.saveAs",
                json!({"roomId": rid, "eventId": ev, "dest": dest}),
                move |_ui, win, out| {
                    let msg = match out {
                        Ok(v) => format!("Saved to {}", s(&v, "path")),
                        Err((_, m)) => m,
                    };
                    if action_is_doc {
                        toast(win, Toast::Doc, msg.as_str().into());
                    } else {
                        toast(win, Toast::Audio, msg.as_str().into());
                    }
                },
            );
        }
        "doc-page" => doc_page(ui, win, a, b2),
        "doc-sheet" => doc_sheet(ui, win, a),
        "open-audio-page" => audio_open(ui, win, a),
        "audio-toggle" => audio_page_toggle(ui, win),
        "audio-seek" => {
            let (rid, ev) = ui.audio_ctx.clone();
            req.fire(
                "audio.play",
                json!({"roomId": rid, "eventId": ev, "seek": a.parse::<f64>().unwrap_or(0.0)}),
            );
            win.set_au_playing(true);
        }

        // ---- chat theme ----
        "theme-accent" => {
            ui.theme_pending["accent"] = json!(a);
            push_theme(ui, win);
        }
        "theme-wallpaper" => {
            ui.theme_pending["wallpaper"] = json!(a);
            push_theme(ui, win);
        }
        "theme-reset" => {
            ui.theme_pending = json!({});
            push_theme(ui, win);
        }
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
            let hex = ui.theme_pending["_pick"]
                .as_str()
                .unwrap_or("#7c9fd4")
                .to_string();
            ui.theme_pending["accent"] = json!(hex);
            push_theme(ui, win);
        }

        // ---- attach ----
        "open-attach" => {
            if ui.stickers.is_empty() {
                load_stickers(ui, win);
            }
        }
        "pick-attach-files" => attach_pick(ui, AttachPick::Files, false),
        "pick-attach-gallery" => attach_pick(ui, AttachPick::Gallery, false),
        // The camera page's shutter — and, on the phone, the page's own
        // arrival. The attach sheet reaches the window through `at-page` and
        // these callbacks and nothing else, so the camera page has no channel
        // of its own to say "I am open"; it fires the shutter's callback on
        // entry as well, and the two are told apart by whether a viewfinder
        // is already up. With none, this is the page arriving (or a refused
        // permission being retried) and the answer is to open one.
        "pick-attach-camera" => {
            if !camera_shutter_handled(win) {
                attach_pick(ui, AttachPick::Photo, false)
            }
        }
        "pick-attach-video" => {
            if !camera_shutter_handled(win) {
                attach_pick(ui, AttachPick::Video, false)
            }
        }

        // ---- media staging ----
        // The step between picking and sending, so a caption can be written.
        "staging-add" => attach_pick(ui, AttachPick::Gallery, true),
        "staging-remove" => staging_remove(win, a.parse().unwrap_or(0)),
        "staging-send" => staging_send(ui, win, a),
        "staging-cancel" => staging_clear(win),
        // A location tile: reset the picker and turn the sheet's page to the
        // mode, in place (AttachMenu.qml:53-55 activate + locPicker.reset).
        "attach-location" => {
            ui.lp_mark = None;
            ui.lp_zoom = 16;
            ui.lp_epoch += 1;
            win.set_lp_marked(false);
            win.set_lp_unavailable(false);
            win.set_lp_map(Default::default());
            win.set_attach_open(true);
            win.set_at_page(a.into());
            refresh_position(ui);
            request_lp_map(ui, win);
        }
        // A tap on the picker's map (pin mode): box pixels → lat/lon around
        // the crop centre, then a re-crop centred on the new pin.
        "lp-tap" => {
            if win.get_at_page() == "pin" {
                let mut it = a.split(',');
                let x: f64 = it.next().and_then(|v| v.trim().parse().ok()).unwrap_or(-1.0);
                let y: f64 = it.next().and_then(|v| v.trim().parse().ok()).unwrap_or(-1.0);
                if let Some(view) = ui.lp_view {
                    if x >= 0.0 && y >= 0.0 {
                        let (lat, lon) = lp_tap_latlon(&view, x, y);
                        ui.lp_mark = Some((lat, lon));
                        win.set_lp_marked(true);
                        win.set_lp_mark_lat(lat as f32);
                        win.set_lp_mark_lon(lon as f32);
                        request_lp_map_debounced(ui);
                    }
                }
            }
        }
        // The picker's +/- chips (LocationPicker.qml:180-203).
        "lp-zoom" => {
            let z = (ui.lp_zoom + if a == "in" { 1 } else { -1 }).clamp(3, 19);
            if z != ui.lp_zoom {
                ui.lp_zoom = z;
                request_lp_map_debounced(ui);
            }
        }
        // The recentre chip: back to the device fix (a marked pin follows it
        // — the crop is always centred on the marker).
        "lp-recentre" => {
            refresh_position(ui);
            if let Some((lat, lon)) = ui.lp_fix {
                if ui.lp_mark.is_some() {
                    ui.lp_mark = Some((lat, lon));
                    win.set_lp_mark_lat(lat as f32);
                    win.set_lp_mark_lon(lon as f32);
                }
                ui.lp_epoch += 1;
                request_lp_map(ui, win);
            }
        }
        "position-refresh" => refresh_position(ui),
        "resized" => {
            after(&req, 120, |ui, win| {
                crate::bridge::rebuild_timeline(ui, win)
            });
        }
        // a = "lat,lon[,mode]" (mode pin|current|live — a dropped pin is not
        // m.self, AttachMenu.qml:374), b = durationMs.
        "location-share" => {
            let mut it = a.split(',');
            let lat: f64 = it
                .next()
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(f64::NAN);
            let lon: f64 = it
                .next()
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(f64::NAN);
            let mode = it.next().unwrap_or("").trim().to_string();
            let ms: f64 = b2.trim().parse().unwrap_or(0.0);
            if lat.is_finite() && lon.is_finite() {
                let room = room_of_key(&ui.open_room);
                if ms > 0.0 {
                    req.fire(
                        "location.startLive",
                        json!({"roomId": room, "lat": lat, "lon": lon, "durationMs": ms as u64}),
                    );
                } else {
                    req.fire(
                        "location.send",
                        json!({"roomId": room, "lat": lat, "lon": lon, "self": mode != "pin"}),
                    );
                }
            }
            // The sheet closes over the conversation (root.closeRequested()).
            win.set_attach_open(false);
            win.set_at_page("grid".into());
        }
        "stop-live" => req.fire(
            "location.stopLive",
            json!({"roomId": room_of_key(&ui.open_room)}),
        ),
        "open-contact" => {
            if let Some(item) = ui.shadow.iter().find(|i| s(i, "eventId") == a) {
                let c = &item["contact"];
                ui.contact_ctx = (s(c, "userId").to_string(), s(c, "displayName").to_string());
                win.set_contact_open(true);
            }
        }
        // the contact card's pills (BubbleDelegate.qml:738-753)
        "contact-save" | "contact-unsave" if !a.is_empty() => {
            if action == "contact-save" {
                req.fire("contacts.save", json!({"userId": a, "displayName": b2}));
                ui.saved_contacts.insert(a.to_string());
                win.set_chat_toast("Saved to contacts".into());
            } else {
                req.fire("contacts.remove", json!({"userId": a}));
                ui.saved_contacts.remove(a);
                win.set_chat_toast("Removed from contacts".into());
            }
            rebuild_timeline(ui, win);
        }
        // no share sheet on the desktop: the card goes to the clipboard as text
        "contact-share" => {
            crate::platform::copy_text(&format!("{b2} <{a}>"));
            win.set_chat_toast("Copied".into());
        }
        "copy-text" => {
            crate::platform::copy_text(a);
            win.set_chat_toast("Copied".into());
        }
        "spoiler-revealed" => {
            ui.spoilers_revealed.insert(a.to_string());
        }
        "contact-choice" => {
            let (uid, name) = ui.contact_ctx.clone();
            match a {
                "dm" if !uid.is_empty() => start_dm(ui, &uid),
                "save" if !uid.is_empty() => {
                    req.fire("contacts.save", json!({"userId": uid, "displayName": name}));
                    toast(win, Toast::Viewer, "Saved".into());
                }
                _ => {}
            }
        }
        "member-choice-open" => {
            ui.contact_ctx = (a.to_string(), b2.to_string());
            win.set_member_name(if b2.is_empty() { a } else { b2 }.into());
            win.set_member_open(true);
        }
        "member-choice" => {
            let (uid, name) = ui.contact_ctx.clone();
            match a {
                "dm" => start_dm(ui, &uid),
                "share" => {
                    let room = room_of_key(&ui.open_room);
                    req.fire(
                        "contact.send",
                        json!({"roomId": room, "userId": uid, "displayName": name}),
                    );
                    win.set_nav("chat".into());
                }
                _ => {}
            }
        }
        // ChatPage.qml onInsertEmoji: input.insert(input.cursorPosition, ch).
        "composer-insert" => {
            let text = win.get_ct_composer_text().to_string();
            let mut cur = (win.get_ct_composer_cursor().max(0) as usize).min(text.len());
            while cur > 0 && !text.is_char_boundary(cur) {
                cur -= 1;
            }
            let spliced = format!("{}{}{}", &text[..cur], a, &text[cur..]);
            win.invoke_chat_composer_set(spliced.into(), (cur + a.len()) as i32, true);
            // The picker's RECENTS block follows what was actually used.
            note_emoji_used(a);
        }
        "create-poll" => create_poll(ui, a, b2),
        "load-stickers" => load_stickers(ui, win),
        "send-sticker" => send_sticker(ui, a),

        // ---- voice recorder ----
        "open-recorder" => {
            ui.rec_levels.clear();
            win.set_rec_state("idle".into());
        }
        "voice-record" => {
            // Android grants the mic at runtime; ask and stay idle — the
            // person taps record again once the dialog is answered.
            #[cfg(target_os = "android")]
            if !crate::platform::has_mic_permission() {
                crate::platform::request_mic_permission();
                win.set_rec_state("idle".into());
                return;
            }
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
                    // voice.stop reports seconds (the recorder's own clock)
                    win.set_rec_clip_duration(v["duration"].as_f64().unwrap_or(0.0) as f32);
                    let wave: Vec<f64> = v["waveform"]
                        .as_array()
                        .map(|a| a.iter().filter_map(Value::as_f64).collect())
                        .unwrap_or_default();
                    win.set_rec_clip_waveform(ModelRc::new(VecModel::from(
                        crate::rows::resample_wave(&wave, 60),
                    )));
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
        // Attach STAGES the clip in the composer (ChatPage.voicePath) — the
        // send button posts it, with the typed text as its caption.
        "voice-attach" => {
            let secs = ui.voice_clip["duration"].as_f64().unwrap_or(0.0);
            let wave: Vec<f64> = ui.voice_clip["waveform"]
                .as_array()
                .map(|a| a.iter().filter_map(Value::as_f64).collect())
                .unwrap_or_default();
            win.set_voice_staged_duration(
                format!("{:02}:{:02}", (secs as u64) / 60, (secs as u64) % 60).into(),
            );
            // as many bars as the chip has room for (ChatPage.qml:1577)
            let bars = win.get_chat_clip_wave_bars().max(8) as usize;
            win.set_voice_staged_wave(ModelRc::new(VecModel::from(crate::rows::resample_wave(
                &wave, bars,
            ))));
            clip_preview_stop(ui, win);
            win.set_voice_staged(true);
            win.set_recorder_open(false);
        }
        "voice-send" => {
            let clip = std::mem::take(&mut ui.voice_clip);
            req.fire(
                "voice.send",
                json!({
                    "roomId": open_room,
                    "path": s(&clip, "path"),
                    "duration": clip["duration"].as_f64().unwrap_or(0.0),
                    "waveform": clip["waveform"].clone(),
                    "caption": a,
                }),
            );
            win.invoke_clear_composer();
            crate::composer::reset(&win);
        }
        "voice-discard" => {
            ui.voice_clip = Value::Null;
            req.fire("audio.stop", json!({}));
            clip_preview_stop(ui, win);
        }
        "voice-preview" => {
            if a == "1" {
                clip_preview_play(ui, win, ui.clip_pos);
            } else {
                req.fire("audio.stop", json!({}));
                ui.clip_timer.stop();
                win.set_chat_clip_playing(false);
            }
        }
        // a tap on the wave: play from that fraction (ChatPage.qml:1600-1610)
        "voice-preview-seek" => {
            let secs = ui.voice_clip["duration"].as_f64().unwrap_or(0.0);
            let frac: f64 = a.parse().unwrap_or(0.0);
            clip_preview_play(ui, win, (frac.clamp(0.0, 1.0) * secs).min(secs));
        }
        "voice-cancel" => {
            ui.recording = false;
            req.fire("voice.cancel", json!({}));
        }
        "open-map" => open_map(ui, win, a),
        // The page's +/- chips: the grid steps about the middle of the view,
        // and the still composite follows for a server that has no tiles.
        "map-zoom" => {
            let step = if a == "in" { 1 } else { -1 };
            let (w, h) = (ui.mapview.w / 2.0, ui.mapview.h / 2.0);
            ui.mapview.zoom(step, w, h);
            map_place(ui, win);
            let z = (ui.map_zoom + step as i64).clamp(3, 19);
            if z != ui.map_zoom {
                ui.map_zoom = z;
                fetch_map_page(ui);
            }
        }
        other => tracing::warn!("act: unknown action {other}"),
    }
}

/// The address book's usernames, for the contact card's Save pill.
pub fn load_saved_contacts(ui: &mut UiState) {
    call_ui(&ui.req.clone(), "contacts.list", json!({}), |ui, _win, out| {
        if let Ok(v) = out {
            ui.saved_contacts = v["contacts"]
                .as_array()
                .map(|a| a.iter().filter_map(|c| c["userId"].as_str().map(str::to_string)).collect())
                .unwrap_or_default();
        }
    });
}

#[derive(Clone, Copy)]
pub enum Toast {
    Viewer,
    Doc,
    Audio,
}

/// The media pages hide their toast on a timer without clearing the bound
/// text, so the same message twice needs a blank in between.
fn toast(win: &AppWindow, which: Toast, text: SharedString) {
    match which {
        Toast::Viewer => {
            win.set_vw_toast(SharedString::new());
            win.set_vw_toast(text);
        }
        Toast::Doc => {
            win.set_dc_toast(SharedString::new());
            win.set_dc_toast(text);
        }
        Toast::Audio => {
            win.set_au_toast(SharedString::new());
            win.set_au_toast(text);
        }
    }
}

fn fmt_dur(secs: f64) -> String {
    let t = secs.max(0.0).round() as u64;
    format!("{:02}:{:02}", t / 60, t % 60)
}

/// Play the staged clip from `from` seconds and tick the chip's progress
/// every 100 ms (ChatPage.qml:723-729 clipTimer): the played fraction
/// fills the bars and the duration counts down (ChatPage.qml:1615).
fn clip_preview_play(ui: &mut UiState, win: &AppWindow, from: f64) {
    let secs = ui.voice_clip["duration"].as_f64().unwrap_or(0.0);
    ui.req.fire(
        "audio.playFile",
        json!({"path": s(&ui.voice_clip, "path"), "seek": from}),
    );
    ui.clip_pos = from;
    win.set_chat_clip_frac(if secs > 0.0 { (from / secs) as f32 } else { 0.0 });
    win.set_chat_clip_playing(true);
    ui.clip_timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(100),
        || {
            crate::bridge::with_ui(|ui| {
                let Some(win) = ui.win.upgrade() else { return };
                let secs = ui.voice_clip["duration"].as_f64().unwrap_or(0.0);
                ui.clip_pos += 0.1;
                if ui.clip_pos >= secs {
                    // over: back to the start, bars dim, duration in full
                    ui.clip_timer.stop();
                    ui.clip_pos = 0.0;
                    win.set_chat_clip_frac(0.0);
                    win.set_chat_clip_playing(false);
                    win.set_voice_staged_duration(fmt_dur(secs).into());
                    return;
                }
                win.set_chat_clip_frac((ui.clip_pos / secs) as f32);
                win.set_voice_staged_duration(fmt_dur(secs - ui.clip_pos).into());
            });
        },
    );
}

fn clip_preview_stop(ui: &mut UiState, win: &AppWindow) {
    ui.clip_timer.stop();
    ui.clip_pos = 0.0;
    win.set_chat_clip_frac(0.0);
    win.set_chat_clip_playing(false);
    let secs = ui.voice_clip["duration"].as_f64().unwrap_or(0.0);
    win.set_voice_staged_duration(fmt_dur(secs).into());
}

/// The @ and # suggestions under the composer (ChatPage.qml:585-683
/// updateAutocomplete / refreshAutocomplete): walk back from the cursor
/// to the trigger at a word boundary, then match room members or joined
/// rooms against what follows it.
pub fn update_autocomplete(ui: &mut UiState, win: &AppWindow) {
    let text = win.get_ct_composer_text().to_string();
    let cursor = (win.get_ct_composer_cursor().max(0) as usize).min(text.len());
    let head = &text[..cursor];
    let mut token: Option<(usize, char)> = None;
    for (i, c) in head.char_indices().rev() {
        if c.is_whitespace() {
            break;
        }
        if c == '@' || c == '#' {
            let boundary = head[..i].chars().last().map(char::is_whitespace).unwrap_or(true);
            if boundary {
                token = Some((i, c));
            }
            break;
        }
    }
    let Some((from, kind)) = token else {
        autocomplete_clear(ui, win);
        return;
    };
    let query = head[from + 1..].to_lowercase();
    ui.ac_from = from;
    ui.ac_kind = kind;
    let me = ui.my_user.clone();
    let mut rows: Vec<UserRow> = Vec::new();
    let mut inserts = Vec::new();
    if kind == '@' {
        let room = room_of_key(&ui.open_room);
        // members come lazily: the first @ in a room fetches them
        if ui.ac_room != room {
            ui.ac_room = room.clone();
            ui.ac_members.clear();
        }
        if ui.ac_members.is_empty() && !ui.ac_fetching {
            ui.ac_fetching = true;
            call_ui(
                &ui.req.clone(),
                "room.members",
                json!({"roomId": room}),
                move |ui, win, out| {
                    ui.ac_fetching = false;
                    if let Ok(v) = out {
                        ui.ac_members = v["members"].as_array().cloned().unwrap_or_default();
                    }
                    if ui.ac_kind == '@' {
                        update_autocomplete(ui, win);
                    }
                },
            );
        }
        for m in &ui.ac_members {
            if rows.len() >= 6 {
                break;
            }
            let uid = s(m, "userId");
            if uid == me {
                continue;
            }
            let name = match s(m, "displayName") {
                "" => uid.to_string(),
                d => d.to_string(),
            };
            if query.is_empty()
                || name.to_lowercase().contains(&query)
                || uid.to_lowercase().contains(&query)
            {
                inserts.push(name.clone());
                rows.push(project::user_row(m, false));
            }
        }
    } else {
        let open = room_of_key(&ui.open_room);
        for r in &ui.rooms_json {
            if rows.len() >= 6 {
                break;
            }
            let rid = s(r, "roomId");
            // linking a room to itself is not a thing anyone means to do
            if rid == open {
                continue;
            }
            let name = s(r, "name").to_string();
            if query.is_empty() || name.to_lowercase().contains(&query) {
                inserts.push(name.clone());
                rows.push(UserRow {
                    user_id: rid.into(),
                    display_name: name.clone().into(),
                    initials: crate::rows::initials(&name).into(),
                    tint: crate::rows::tint_for(rid),
                    ..Default::default()
                });
            }
        }
    }
    ui.ac_inserts = inserts;
    win.set_chat_ac_items(ModelRc::new(VecModel::from(rows)));
    win.set_chat_ac_index(0);
}

fn autocomplete_clear(ui: &mut UiState, win: &AppWindow) {
    ui.ac_kind = ' ';
    if !ui.ac_inserts.is_empty() {
        ui.ac_inserts.clear();
        win.set_chat_ac_items(ModelRc::new(VecModel::from(Vec::<UserRow>::new())));
    }
}

/// A suggestion taken (ChatPage.qml:685-704 acceptAutocomplete): the token
/// from the trigger to the cursor becomes "@name " or "#room ".
fn autocomplete_pick(ui: &mut UiState, win: &AppWindow, idx: usize) {
    if ui.ac_inserts.is_empty() || ui.ac_kind == ' ' {
        return;
    }
    let insert = &ui.ac_inserts[idx.min(ui.ac_inserts.len() - 1)];
    let text = win.get_ct_composer_text().to_string();
    let cursor = (win.get_ct_composer_cursor().max(0) as usize).min(text.len());
    let from = ui.ac_from.min(cursor);
    let piece = format!("{}{insert} ", ui.ac_kind);
    let new_text = format!("{}{piece}{}", &text[..from], &text[cursor..]);
    win.invoke_chat_composer_set(new_text.into(), (from + piece.len()) as i32, true);
    autocomplete_clear(ui, win);
}

fn menu_action(ui: &mut UiState, win: &AppWindow, action: &str, event_id: &str) {
    let req = ui.req.clone();
    let key = ui.open_room.clone();
    let room = room_of_key(&key);
    match action {
        "prepare" => build_sheet(ui, win, event_id),
        "copy" => {
            if let Some(item) = ui
                .shadow
                .iter()
                .find(|i| i["eventId"].as_str() == Some(event_id))
            {
                crate::platform::copy_text(s(item, "body"));
            }
        }
        "forward" => {
            ui.forward_item = ui
                .shadow
                .iter()
                .find(|i| i["eventId"].as_str() == Some(event_id))
                .cloned()
                .unwrap_or(Value::Null);
            win.set_fw_mode("forward".into());
            crate::bridge::invoke_later(win, |w| w.invoke_go("forward".into()));
        }
        "pin" => {
            let pinned = ui
                .pinned_by_room
                .get(&room)
                .map(|p| p.iter().any(|e| e == event_id))
                .unwrap_or(false);
            req.fire(
                if pinned {
                    "message.unpin"
                } else {
                    "message.pin"
                },
                json!({"roomId": room, "eventId": event_id}),
            );
        }
        "redact" => req.fire(
            "message.redact",
            json!({"roomId": key, "eventId": event_id}),
        ),
        "endpoll" => req.fire("poll.end", json!({"roomId": key, "eventId": event_id})),
        "retry" | "cancelsend" => {
            if let Some(item) = ui.shadow.iter().find(|i| {
                i["id"].as_str() == Some(event_id) || i["eventId"].as_str() == Some(event_id)
            }) {
                let request = if action == "retry" {
                    "message.retry"
                } else {
                    "message.cancel"
                };
                req.fire(
                    request,
                    json!({"roomId": key, "eventId": s(item, "eventId"), "id": s(item, "id"), "txnId": s(item, "txnId")}),
                );
            }
        }
        other => tracing::info!("menu action {other} not wired"),
    }
}

/// MessageSheet.actionsFor, built bridge-side as [MenuEntry].
fn build_sheet(ui: &mut UiState, win: &AppWindow, event_id: &str) {
    let Some(item) = ui
        .shadow
        .iter()
        .find(|i| i["eventId"].as_str() == Some(event_id) || i["id"].as_str() == Some(event_id))
        .cloned()
    else {
        return;
    };
    let g = win.global::<crate::Icons>();
    let mut out: Vec<crate::MenuEntry> = Vec::new();
    let mut add = |t: &str, a: &str, icon: SharedString, danger: bool| {
        out.push(crate::MenuEntry {
            t: t.into(),
            a: a.into(),
            icon,
            danger,
        });
    };
    let send_state = s(&item, "sendState");
    let can = item["can"].clone();
    let kind = s(&item, "kind");
    if send_state == "sending" || send_state == "failed" {
        if send_state == "failed" {
            add("Retry", "retry", g.get_retry(), false);
        }
        add("Copy", "copy", g.get_copy(), false);
        add("Cancel send", "cancelsend", g.get_cancel(), true);
    } else {
        add("Reply", "reply", g.get_reply(), false);
        add("Forward", "forward", g.get_forward(), false);
        add("Copy", "copy", g.get_copy(), false);
        if !s(&item, "eventId").is_empty() {
            // Matrix has no nested threads: from inside one, or once a summary
            // exists, the entry opens the thread instead of starting one.
            if !s(&item, "threadRoot").is_empty() || !item["threadSummary"].is_null() {
                add("Open thread", "openthread", g.get_thread(), false);
            } else {
                add("Reply in thread", "thread", g.get_thread(), false);
            }
            let room = room_of_key(&ui.open_room);
            let pinned = ui
                .pinned_by_room
                .get(&room)
                .map(|p| p.iter().any(|e| e == s(&item, "eventId")))
                .unwrap_or(false);
            add(
                if pinned { "Unpin" } else { "Pin" },
                "pin",
                g.get_pin(),
                false,
            );
        }
        if b(&can, "edit") && kind != "poll" {
            // Media edits its caption, not its body (actionsFor's mediaKind arm).
            let media_kind = matches!(kind, "image" | "video" | "file" | "audio");
            if media_kind {
                let body = s(&item, "body");
                let has_caption =
                    !body.is_empty() && body != item["media"]["filename"].as_str().unwrap_or("");
                add(
                    if has_caption {
                        "Edit caption"
                    } else {
                        "Add caption"
                    },
                    "caption",
                    g.get_edit(),
                    false,
                );
            } else {
                add("Edit", "edit", g.get_edit(), false);
            }
        }
        if kind == "poll" && !item["poll"]["ended"].as_bool().unwrap_or(false) && b(&can, "redact")
        {
            add("End poll", "endpoll", g.get_poll(), false);
        }
        if b(&can, "redact") {
            add("Delete", "redact", g.get_trash(), true);
        }
    }
    ui.sheet_item = item;
    win.set_sheet_actions(ModelRc::new(VecModel::from(out)));
}

fn start_submit(ui: &mut UiState, _win: &AppWindow, mode: &str, value: &str) {
    let req = ui.req.clone();
    match mode {
        "join" => {
            call_ui(
                &req,
                "room.join",
                json!({"roomIdOrAlias": value}),
                move |ui, win, out| match out {
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
                },
            );
        }
        _ => {
            // A group with a name and nobody in it yet; people come through
            // Add people. Every Sigil conversation is private and encrypted.
            call_ui(
                &req,
                "room.create",
                json!({"name": value, "invite": []}),
                move |ui, win, out| match out {
                    Ok(v) => {
                        let rid = s(&v, "roomId").to_string();
                        if !rid.is_empty() {
                            crate::bridge::open_room(ui, win, &rid);
                            win.set_nav("chat".into());
                        }
                    }
                    Err((_, m)) => win.set_st_error(m.as_str().into()),
                },
            );
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
        req.fire(
            "message.send",
            json!({"roomId": room_id, "body": s(&item, "body"), "markdown": false}),
        );
    }
    crate::bridge::open_room(ui, win, room_id);
    win.set_nav("chat".into());
}

/// `users.search`, debounced. The query goes to the engine **as typed**:
/// `bob`, `@bob` and `@bob:sigil.test` all name the same person and the
/// engine resolves all three (`sigil::search`), which also lets a fragment
/// match the people we already know. Rewriting the query here — the old
/// `@{q}:{door_server}` — turned `wr` into `@wr:sigil.test` and stopped
/// that fragment ever matching anything.
///
/// Under two characters the Start page shows suggestions instead of
/// results: the people we know, which the engine answers for an empty
/// query. The open DMs are painted at once so the list is never blank while
/// that round trip runs; the invite page just empties.
fn dir_search(ui: &mut UiState, win: &AppWindow, which: &str, q: &str) {
    let start = which == "dir-search-start";
    let q = q.trim().to_string();
    let suggesting = q.chars().count() < 2;
    if suggesting {
        if !start {
            win.set_ap_results(ModelRc::new(VecModel::from(Vec::new())));
            return;
        }
        rebuild_start_suggestions(ui, win);
    }
    // Suggestions ask for everyone we know, not for the two letters typed
    // so far: a one-letter query is not a name the front desk can answer.
    let q = if suggesting { String::new() } else { q };
    let epoch = if start {
        ui.start_query_epoch += 1;
        ui.start_query_epoch
    } else {
        ui.dir_query_epoch += 1;
        ui.dir_query_epoch
    };
    let req = ui.req.clone();
    after(&req, 300, move |ui, win| {
        let current = if start {
            ui.start_query_epoch
        } else {
            ui.dir_query_epoch
        };
        if current != epoch {
            return; // superseded
        }
        if start && !suggesting {
            win.set_st_busy(true);
        }
        call_ui(
            &ui.req.clone(),
            "users.search",
            json!({"query": q, "limit": 12}),
            move |_ui, win, out| {
                if start {
                    win.set_st_busy(false);
                }
                if let Ok(v) = out {
                    // `results` is what the engine answers (core/docs/
                    // protocol.md). This read used to say `users`, which no
                    // engine has ever sent: every hit was thrown away and
                    // the page stayed empty however the name was typed.
                    // `users` stays as a fallback for an older engine.
                    let rows: Vec<_> = v["results"]
                        .as_array()
                        .or_else(|| v["users"].as_array())
                        .map(|a| a.iter().map(|u| project::user_row(u, false)).collect())
                        .unwrap_or_default();
                    // Suggestions keep the DMs already on screen rather than
                    // blanking them if the engine has nothing to add.
                    if suggesting && rows.is_empty() {
                        return;
                    }
                    let model = ModelRc::new(VecModel::from(rows));
                    if start {
                        win.set_st_people(model);
                    } else {
                        win.set_ap_results(model);
                    }
                }
            },
        );
    });
}

fn voice_toggle(ui: &mut UiState, win: &AppWindow, event_id: &str) {
    if ui.audio_playing && ui.audio_ctx.1 == event_id {
        voice_playback_end(ui, true);
        rebuild_timeline(ui, win);
    } else {
        voice_playback_start(ui, win, event_id, 0.0);
    }
}

fn voice_seek(ui: &mut UiState, win: &AppWindow, event_id: &str, frac: f64) {
    let dur = voice_duration(ui, event_id);
    voice_playback_start(ui, win, event_id, frac.clamp(0.0, 1.0) * dur);
}

/// A voice note's length in seconds, from the event the timeline holds.
fn voice_duration(ui: &UiState, event_id: &str) -> f64 {
    ui.shadow
        .iter()
        .find(|i| i["eventId"].as_str() == Some(event_id))
        .and_then(|i| i["media"]["duration"].as_f64())
        .unwrap_or(0.0)
        / 1000.0
}

/// Play `event_id` from `from` seconds and start the shared poll. The poll
/// only starts once the engine has answered: `audio.play` may have to fetch
/// the file first, and until it replies there is no player to ask.
fn voice_playback_start(ui: &mut UiState, win: &AppWindow, event_id: &str, from: f64) {
    let room = room_of_key(&ui.open_room);
    ui.voice_timer.stop();
    ui.voice_positions.clear();
    ui.voice_positions.insert(event_id.to_string(), from);
    ui.audio_ctx = (room.clone(), event_id.to_string());
    ui.audio_playing = true;
    let want = event_id.to_string();
    call_ui(
        &ui.req.clone(),
        "audio.play",
        json!({"roomId": room, "eventId": event_id, "seek": from}),
        move |ui, win, out| {
            // The user may have pressed pause, or started another note,
            // while the file was on its way.
            if !ui.audio_playing || ui.audio_ctx.1 != want {
                return;
            }
            match out {
                Ok(_) => voice_track(ui),
                Err(_) => {
                    voice_playback_end(ui, false);
                    rebuild_timeline(ui, win);
                }
            }
        },
    );
    rebuild_timeline(ui, win);
}

/// Playback is over — paused by hand (`tell` the engine) or run out (the
/// poll saw the player go). Either way the row goes back to zero.
fn voice_playback_end(ui: &mut UiState, tell_engine: bool) {
    ui.voice_timer.stop();
    if tell_engine {
        ui.req.fire("audio.stop", json!({}));
    }
    ui.audio_playing = false;
    ui.voice_positions.clear();
    voice_paint(ui, 0.0);
}

/// The one timer the played waveform runs on: 20 Hz is well past the ~36
/// bars a note is drawn with, and costs one engine read per tick.
fn voice_track(ui: &mut UiState) {
    let req = ui.req.clone();
    ui.voice_timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(50),
        move || {
            call_ui(&req, "audio.position", json!({}), |ui, _win, out| {
                if !ui.audio_playing {
                    ui.voice_timer.stop();
                    return;
                }
                let v = out.unwrap_or_else(|_| json!({}));
                if v["playing"].as_bool().unwrap_or(false) {
                    voice_paint(ui, v["position"].as_f64().unwrap_or(0.0));
                } else {
                    // The clip ran out: the engine has already dropped it.
                    voice_playback_end(ui, false);
                }
            });
        },
    );
}

/// Set voice-playing/voice-frac on the row that is playing and clear the
/// rest, patching the rows in place — a full rebuild 20 times a second
/// would redo every bubble in the room.
fn voice_paint(ui: &mut UiState, pos: f64) {
    use slint::Model as _;
    let (playing, ev) = (ui.audio_playing, ui.audio_ctx.1.clone());
    if playing {
        ui.voice_positions.clear();
        ui.voice_positions.insert(ev.clone(), pos);
    }
    let dur = voice_duration(ui, &ev);
    let frac = if playing && dur > 0.0 {
        (pos / dur).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    let model = ui.items_model.clone();
    for i in 0..model.row_count() {
        let Some(row) = model.row_data(i) else { continue };
        if row.kind.as_str() != "voice" && row.kind.as_str() != "audio" {
            continue;
        }
        let hit = playing && row.event_id.as_str() == ev;
        let want = if hit { frac } else { 0.0 };
        if row.voice_playing != hit || (row.voice_frac - want).abs() > 0.001 {
            let mut row = row;
            row.voice_playing = hit;
            row.voice_frac = want;
            model.set_row_data(i, row);
        }
    }
}

fn push_emoji(ui: &mut UiState, win: &AppWindow, query: &str) {
    ui.emoji_query = Some(query.to_string());
    if ui.emojis.is_empty() {
        // The desktop shell's list when it is there; the bundled copy of the
        // same shape (assets/emojis.json, generated from the Unicode data)
        // everywhere else — a phone has no shell to read from.
        let text = std::fs::read_to_string("/usr/share/omarchy/shell/plugins/emojis/emojis.json")
            .unwrap_or_else(|_| include_str!("../assets/emojis.json").to_string());
        if let Ok(v) = serde_json::from_str::<Value>(&text) {
            if let Some(arr) = v.as_array() {
                let listed: Vec<(String, String)> = arr
                    .iter()
                    .map(|e| (s(e, "e").to_string(), s(e, "k").to_string()))
                    .filter(|(g, _)| !g.is_empty())
                    .collect();
                let (drawable, bounds) = keep_drawable(listed);
                ui.emojis = drawable;
                EMOJI_BOUNDS.with(|b| *b.borrow_mut() = bounds);
            }
        }
    }
    let q = query.to_lowercase();
    let filtered: Vec<(String, String)> = ui
        .emojis
        .iter()
        .filter(|(_, k)| q.is_empty() || k.contains(&q))
        .cloned()
        .collect();
    let mut rows: Vec<ModelRc<crate::EmojiItem>> = Vec::new();
    let mut cats: Vec<i32> = Vec::new();
    let push_row = |ui: &mut UiState, rows: &mut Vec<ModelRc<crate::EmojiItem>>, chunk: &[(String, String)]| {
        let row: Vec<crate::EmojiItem> = chunk
            .iter()
            .map(|(g, k)| {
                let img = crate::bridge::emoji_image(ui, g);
                crate::EmojiItem {
                    glyph: g.as_str().into(),
                    name: k.as_str().into(),
                    has_img: img.is_some(),
                    img: img.unwrap_or_default(),
                }
            })
            .collect();
        rows.push(ModelRc::new(VecModel::from(row)));
    };
    if q.is_empty() {
        // The Google Messages picker's sectioned grid: a heading, then that
        // category's own rows, so a row never straddles a boundary. A heading
        // rides in the SAME model as the glyphs — one item, empty glyph, the
        // caption in `name` — because `emoji-rows` is the only channel the
        // picker has (app.slint's property set is fixed).
        //
        // Block boundaries were taken while the list was filtered, walking it
        // in order, so a marker glyph the device cannot draw takes only itself
        // out and never moves the boundary it named.
        let mut bounds = EMOJI_BOUNDS.with(|b| b.borrow().clone());
        bounds.resize(EMOJI_CATS.len(), 0);
        bounds.push(filtered.len());
        // RECENTS goes first, before the smileys. With none yet, the clock
        // still gets an entry so the bar and the strip stay index-for-index.
        let recents = recent_emojis(&filtered);
        if !recents.is_empty() {
            cats.push(cat_mark(&rows));
            rows.push(heading_row("RECENTS"));
            for chunk in recents.chunks(EMOJI_COLS) {
                push_row(ui, &mut rows, chunk);
            }
        } else {
            cats.push(cat_mark(&rows));
        }
        for (i, (_, label)) in EMOJI_CATS.iter().enumerate() {
            let (start, end) = (bounds[i], bounds[i + 1].max(bounds[i]));
            // An empty block gets no caption; its bar icon still points at
            // where it would have begun, which is where the next one does.
            cats.push(cat_mark(&rows));
            if start >= end {
                continue;
            }
            rows.push(heading_row(label));
            for chunk in filtered[start..end].chunks(EMOJI_COLS) {
                push_row(ui, &mut rows, chunk);
            }
        }
    } else {
        // Searching: results only, no sections (EmojiPicker.qml:89 curCat -1).
        for chunk in filtered.chunks(EMOJI_COLS) {
            push_row(ui, &mut rows, chunk);
        }
    }
    cats.resize(EMOJI_CATS.len() + 1, 0);
    win.set_emoji_rows(ModelRc::new(VecModel::from(rows)));
    win.set_emoji_cat_rows(ModelRc::new(VecModel::from(cats)));
}

/// Nine columns, the width the reference picker lays a row out in
/// (Screenshot_20260903-234221.png: cell pitch 146 physical px across 1344).
const EMOJI_COLS: usize = 9;

/// A category's jump target, for `emoji-cat-rows`. A caption block and a glyph
/// row are different heights and the height depends on the window's width, so
/// the picker works the offset out itself — and to do that it needs BOTH counts,
/// down one `[int]` channel (app.slint's property set is fixed and out of
/// reach). Captions can never reach 16, so they ride in the low nibble.
fn cat_mark(rows: &[ModelRc<crate::EmojiItem>]) -> i32 {
    let heads = rows.iter().filter(|r| is_heading_row(r)).count();
    ((rows.len() - heads) * 16 + heads) as i32
}

fn is_heading_row(row: &ModelRc<crate::EmojiItem>) -> bool {
    use slint::Model as _;
    row.row_count() == 1 && row.row_data(0).map(|e| e.glyph.is_empty()).unwrap_or(false)
}

/// The category bar, in the bundled list's Unicode order: the glyph that
/// opens each block, and the caption over it. The first has no mark — the
/// smileys open the list — and carries RECENTS in front of it.
const EMOJI_CATS: [(&str, &str); 9] = [
    ("", "SMILEYS AND EMOTIONS"),
    ("👋", "PEOPLE"),
    ("🐵", "ANIMALS AND NATURE"),
    ("🍇", "FOOD AND DRINK"),
    ("🌍", "TRAVEL AND PLACES"),
    ("🎃", "ACTIVITIES"),
    ("👓", "OBJECTS"),
    ("🏧", "SYMBOLS"),
    ("🏁", "FLAGS"),
];


/// The list cut down to what this device can actually draw, and where each
/// category starts in it.
///
/// An offer goes bad when the font cannot draw the whole entry: a code point
/// it has never heard of, or a sequence its own rules do not fold into one
/// glyph. `emoji.render` answers `not_found` for both, the cell falls through
/// to text, and the text face draws a notdef box — a tofu on a phone, the
/// loose pieces of a joined sequence beside it, or nothing at all where the
/// renderer has no emoji outlines. A picker that offers those is lying about
/// what it can send, so the test runs ONCE, here, as the list is built, and
/// the rows below never see a glyph that fails it.
///
/// The question is the engine's own — `emoji::drawable` is the same code that
/// cuts the picture, composing the sequence through the font's GSUB rules
/// (core/src/media/emoji.rs) — so the two can never disagree.
///
/// With no colour emoji font at all there are no pictures to be had and text
/// IS the intended fallback (core/src/media/emoji.rs:1-6), so the list is kept
/// whole rather than emptied.
fn keep_drawable(listed: Vec<(String, String)>) -> (Vec<(String, String)>, Vec<usize>) {
    let bounds_of = |kept: &[(String, String)], listed: &[(String, String)]| {
        // Walked in order, so a marker glyph that did not survive still leaves
        // its block starting exactly where the next kept entry does.
        let mut bounds = vec![0usize; EMOJI_CATS.len()];
        let mut cat = 1;
        let mut at = 0;
        for (g, _) in listed {
            while cat < EMOJI_CATS.len() && g == EMOJI_CATS[cat].0 {
                bounds[cat] = at;
                cat += 1;
            }
            if kept.get(at).map(|(k, _)| k == g).unwrap_or(false) {
                at += 1;
            }
        }
        for b in bounds.iter_mut().skip(cat) {
            *b = kept.len();
        }
        bounds
    };
    // The engine answers the way it will draw: the phone's own shaper on
    // Android (where the font is the vector edition with no bitmaps to cut,
    // and flags live in a second font), the colour font elsewhere.
    let glyphs: Vec<&str> = listed.iter().map(|(g, _)| g.as_str()).collect();
    let ok = sigil_engine::media::emoji::can_draw_all(&glyphs);
    let kept: Vec<(String, String)> = listed
        .iter()
        .zip(ok.iter())
        .filter(|(_, ok)| **ok)
        .map(|(e, _)| e.clone())
        .collect();
    tracing::info!(
        "emoji picker: {} of {} glyphs are drawable on this device",
        kept.len(),
        listed.len()
    );
    let bounds = bounds_of(&kept, &listed);
    (kept, bounds)
}

thread_local! {
    /// Where each category begins in the drawable list, taken once with it.
    static EMOJI_BOUNDS: std::cell::RefCell<Vec<usize>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// A section caption, as the one-item row the picker recognises by its
/// empty glyph.
fn heading_row(label: &str) -> ModelRc<crate::EmojiItem> {
    ModelRc::new(VecModel::from(vec![crate::EmojiItem {
        glyph: Default::default(),
        name: label.into(),
        has_img: false,
        img: Default::default(),
    }]))
}

/// Where the most-recently-used glyphs live: this device only, beside the
/// chat themes, never sent anywhere.
fn emoji_recents_path() -> String {
    format!(
        "{}/.local/state/sigil/emoji-recents.json",
        std::env::var("HOME").unwrap_or_default()
    )
}

thread_local! {
    /// Read once per run, written through on every pick.
    static EMOJI_RECENTS: std::cell::RefCell<Option<Vec<String>>> =
        const { std::cell::RefCell::new(None) };
}

/// Two rows' worth is what the reference picker's RECENTS block holds before
/// it starts scrolling under the next heading.
const EMOJI_RECENTS_MAX: usize = EMOJI_COLS * 2;

/// The RECENTS block, paired back up with the keywords the grid draws from.
/// A glyph the bundled list no longer has is dropped rather than shown bare —
/// and since the list `all` comes from has already been cut down to what this
/// device can draw (`keep_drawable`), so is one carried over from a phone with
/// a newer font than this one's.
fn recent_emojis(all: &[(String, String)]) -> Vec<(String, String)> {
    EMOJI_RECENTS.with(|cell| {
        let mut slot = cell.borrow_mut();
        let list = slot.get_or_insert_with(|| {
            std::fs::read_to_string(emoji_recents_path())
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .and_then(|v| {
                    Some(
                        v.as_array()?
                            .iter()
                            .filter_map(|g| g.as_str().map(str::to_string))
                            .collect(),
                    )
                })
                .unwrap_or_default()
        });
        list.iter()
            .filter_map(|g| all.iter().find(|(e, _)| e == g).cloned())
            .collect()
    })
}

/// Remember a glyph the user just sent or reacted with. The list is written
/// straight back out so it survives a restart; the picker picks it up the
/// next time it is opened (every open re-runs `emoji-search`).
pub fn note_emoji_used(glyph: &str) {
    // Reaction keys come off the wire, so take only what looks like an emoji:
    // short, and carrying at least one code point no ASCII keyboard could
    // have typed. The ASCII that is allowed through is the keycap bases —
    // 1️⃣ and #️⃣ open on a digit — so "+1" and "lol" are still refused. The
    // length is the engine's own limit (core/src/media/emoji.rs), because a
    // toned family of four runs to eleven code points.
    const KEYCAP_BASE: &str = "0123456789#*";
    if glyph.is_empty()
        || glyph.chars().count() > 32
        || !glyph.chars().any(|c| !c.is_ascii())
        || glyph.chars().any(|c| c.is_ascii() && !KEYCAP_BASE.contains(c))
    {
        return;
    }
    EMOJI_RECENTS.with(|cell| {
        let mut slot = cell.borrow_mut();
        let list = slot.get_or_insert_with(Vec::new);
        list.retain(|g| g != glyph);
        list.insert(0, glyph.to_string());
        list.truncate(EMOJI_RECENTS_MAX);
        let path = emoji_recents_path();
        if let Some(dir) = std::path::Path::new(&path).parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, json!(list).to_string());
    });
}

fn pick_file(ui: &mut UiState, purpose: &str) {
    let purpose = purpose.to_string();
    ui.req.handle().spawn(async move {
        let picked = crate::platform::pick_file().await;
        let _ = slint::invoke_from_event_loop(move || {
            with_ui(|ui| {
                let Some(win) = ui.win.upgrade() else { return };
                let Some(path) = picked else { return };
                if purpose == "theme-photo" {
                    ui.theme_pending["wallpaper"] = json!(path);
                    push_theme(ui, &win);
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

/// Ask for a GIF's frame strip if nothing has yet.
///
/// The timeline asks for its own (bridge.rs), but only for `kind == "image"` —
/// the viewer admits stickers too, and a viewer opened from the search page can
/// run ahead of the timeline's pass. `Null` in the map means a request is
/// already out; `false` means the engine said this is not animated.
fn gif_frames_for_viewer(ui: &mut UiState, item: &Value) {
    let ev = s(item, "eventId").to_string();
    if ev.is_empty() {
        return;
    }
    let mime = item["media"]["mime"].as_str().unwrap_or("");
    let filename = item["media"]["filename"].as_str().unwrap_or("");
    if !mime.contains("gif") && !filename.to_ascii_lowercase().ends_with(".gif") {
        return;
    }
    let room_id = room_of_key(&ui.open_room);
    let key = format!("{room_id}|{ev}");
    if ui.gif_frames.contains_key(&key) {
        return;
    }
    ui.gif_frames.insert(key.clone(), Value::Null);
    let path = item["media"]["path"].as_str().unwrap_or("").to_string();
    fetch_gif_frames(&ui.req.clone(), &room_id, &ev, &path, key);
}

// ------------------------------------------------------------ video playback
//
// The engine decodes into a shared-memory surface and keeps the media clock;
// this side maps the surface (crate::video), draws whatever frame is newest,
// and asks the engine four times a second where the clip has got to. Nothing
// here decides anything the chrome in viewer.slint does not already show:
// play/pause, a position, a duration, a scrub bar.

/// What the viewer's clip needs between ticks.
#[derive(Default)]
struct VideoView {
    /// The shared-memory surface `video.play` answered with.
    surface: String,
    /// Frozen: the tick is off and the last frame copied stands.
    paused: bool,
    /// A finger is on the scrub bar — the poll must not move the thumb.
    scrubbing: bool,
    /// Where a paused (or scrubbed) clip sits, so resuming starts there.
    at: f64,
    /// The clip's own file when the item already carries one, so resuming
    /// costs no lookup.
    file: String,
}

thread_local! {
    static VIDEO: std::cell::RefCell<VideoView> =
        std::cell::RefCell::new(VideoView::default());
    /// Frames off the surface; the media clock every eighth tick.
    static VIDEO_CLOCK: slint::Timer = slint::Timer::default();
}

/// Start (or restart at `seek`) the clip for `event` in the open room. `file`
/// is the local copy when the item has one; empty is fine and the engine
/// finds it from the event, downloading if it must.
// The desktop path follows an Android early return; both are real code.
#[allow(unreachable_code)]
fn video_play(req: &Requester, win: &AppWindow, room: &str, event: &str, file: &str, seek: f64) {
    // The phone has no decoder of ours, and its own player and view: that
    // view is laid over the viewer's picture (java/SigilVideo.java).
    #[cfg(target_os = "android")]
    {
        video_play_android(req, win, room, event, file, seek);
        return;
    }
    win.set_vw_playing_event(event.into());
    win.set_vw_play_pos(seek as f32);
    VIDEO.with(|v| {
        let mut v = v.borrow_mut();
        v.paused = false;
        v.at = seek;
        v.file = file.to_string();
    });
    let asked = event.to_string();
    call_ui(
        req,
        "video.play",
        json!({"roomId": room, "eventId": event, "path": file, "audio": true, "seek": seek}),
        move |ui, win, out| {
            // The viewer may have turned the page (or closed) while the
            // decoder was starting: that reply belongs to nothing now.
            if win.get_vw_playing_event() != asked.as_str() {
                if out.is_ok() {
                    ui.req.fire("video.stop", json!({}));
                }
                return;
            }
            match out {
                Ok(v) => {
                    VIDEO.with(|s| {
                        s.borrow_mut().surface = v["path"].as_str().unwrap_or("").to_string()
                    });
                    win.set_vw_play_duration(v["duration"].as_f64().unwrap_or(0.0) as f32);
                    video_tick(ui, win);
                }
                // No decoder on this device: say so, rather than leaving the
                // viewer in front of a surface nothing will ever fill.
                Err((code, msg)) => {
                    tracing::warn!("video.play: {code} {msg}");
                    video_clear(win);
                    win.set_vw_toast(if code == "unsupported" {
                        SharedString::from("This device cannot play video yet")
                    } else {
                        SharedString::from(msg)
                    });
                }
            }
        },
    );
}

/// The one clock behind playback: 60 Hz for frames (a repaint only happens
/// when the surface has a new one), 4 Hz for the position.
fn video_tick(ui: &mut UiState, win: &AppWindow) {
    let req = ui.req.clone();
    let weak = win.as_weak();
    let mut n: u32 = 0;
    VIDEO_CLOCK.with(|t| {
        t.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(16),
            move || {
                let Some(win) = weak.upgrade() else { return };
                if win.get_vw_playing_event().is_empty() {
                    return;
                }
                let surface = VIDEO.with(|v| v.borrow().surface.clone());
                if !surface.is_empty() {
                    if let Some(img) = crate::video::next_frame(&surface) {
                        win.set_vw_frame(img);
                    }
                }
                n = n.wrapping_add(1);
                if n % 16 != 0 {
                    return;
                }
                call_ui(&req, "video.position", json!({}), |_ui, win, out| {
                    let Ok(v) = out else { return };
                    if win.get_vw_playing_event().is_empty()
                        || VIDEO.with(|s| s.borrow().paused)
                    {
                        return;
                    }
                    // The clip ran out: the engine has already dropped it, so
                    // the viewer goes back to its poster and play disc. A
                    // reply that crossed a pause says `paused`, not `ended`,
                    // and must not be read as the end.
                    if v["ended"].as_bool().unwrap_or(false)
                        || !(v["playing"].as_bool().unwrap_or(false)
                            || v["paused"].as_bool().unwrap_or(false))
                    {
                        video_clear(&win);
                        return;
                    }
                    let d = v["duration"].as_f64().unwrap_or(0.0);
                    if d > 0.0 {
                        win.set_vw_play_duration(d as f32);
                    }
                    let pos = v["position"].as_f64().unwrap_or(0.0);
                    VIDEO.with(|s| s.borrow_mut().at = pos);
                    if !VIDEO.with(|s| s.borrow().scrubbing) {
                        win.set_vw_play_pos(pos as f32);
                    }
                });
            },
        )
    });
}

/// Tap on a clip that is already the viewer's: pause it, or take it up again
/// where it stopped (the QML's play/pause, ImageViewer.qml:429).
// The desktop path follows an Android early return; both are real code.
#[allow(unreachable_code)]
fn video_toggle(req: &Requester, win: &AppWindow, room: &str) {
    #[cfg(target_os = "android")]
    {
        let _ = (req, room);
        let paused = VIDEO.with(|v| v.borrow().paused);
        if paused {
            crate::platform::video_resume();
            VIDEO.with(|v| v.borrow_mut().paused = false);
            video_tick_android(win);
        } else {
            crate::platform::video_pause();
            VIDEO.with(|v| v.borrow_mut().paused = true);
            VIDEO_CLOCK.with(|t| t.stop());
        }
        return;
    }
    let (paused, at, file) = VIDEO.with(|v| {
        let v = v.borrow();
        (v.paused, v.at, v.file.clone())
    });
    if paused {
        let ev = win.get_vw_playing_event().to_string();
        video_play(req, win, room, &ev, &file, at);
        return;
    }
    call_ui(req, "video.pause", json!({}), |_ui, win, out| {
        let at = match out {
            Ok(v) => v["position"].as_f64().unwrap_or(0.0),
            Err(_) => return video_clear(&win),
        };
        VIDEO.with(|s| {
            let mut s = s.borrow_mut();
            s.paused = true;
            s.at = at;
        });
        // The frame the surface last handed over stays; the decoder and its
        // surface are gone until the clip is taken up again.
        VIDEO_CLOCK.with(|t| t.stop());
        crate::video::release();
        win.set_vw_play_pos(at as f32);
    });
}

/// The scrub bar was let go. Seeking restarts the decoder at the new place —
/// a paused clip starts playing again there, as the QML scrubber does.
// The desktop path follows an Android early return; both are real code.
#[allow(unreachable_code)]
fn video_seek(req: &Requester, win: &AppWindow, room: &str, secs: f64) {
    if win.get_vw_playing_event().is_empty() {
        return;
    }
    let secs = secs.max(0.0);
    win.set_vw_play_pos(secs as f32);
    VIDEO.with(|v| v.borrow_mut().at = secs);
    #[cfg(target_os = "android")]
    {
        let _ = (req, room);
        crate::platform::video_seek((secs * 1000.0) as i32);
        if VIDEO.with(|v| v.borrow().paused) {
            crate::platform::video_resume();
            VIDEO.with(|v| v.borrow_mut().paused = false);
            video_tick_android(win);
        }
        return;
    }
    let (paused, file) = VIDEO.with(|v| {
        let v = v.borrow();
        (v.paused, v.file.clone())
    });
    if paused {
        let ev = win.get_vw_playing_event().to_string();
        video_play(req, win, room, &ev, &file, secs);
        return;
    }
    call_ui(
        req,
        "video.seek",
        json!({"seconds": secs}),
        |_ui, win, out| match out {
            // A seek is a fresh decoder and a fresh surface.
            Ok(v) => {
                VIDEO.with(|s| {
                    s.borrow_mut().surface = v["path"].as_str().unwrap_or("").to_string()
                });
                let d = v["duration"].as_f64().unwrap_or(0.0);
                if d > 0.0 {
                    win.set_vw_play_duration(d as f32);
                }
            }
            Err((code, msg)) => {
                tracing::warn!("video.seek: {code} {msg}");
                video_clear(&win);
            }
        },
    );
}

/// Stop the decoder and put the viewer back to its poster.
fn video_end(req: &Requester, win: &AppWindow) {
    if win.get_vw_playing_event().is_empty() {
        return;
    }
    req.fire("video.stop", json!({}));
    video_clear(win);
}

/// The view half of stopping: no request, so it can run from a poll that has
/// just learned the engine dropped the clip on its own.
fn video_clear(win: &AppWindow) {
    #[cfg(target_os = "android")]
    crate::platform::video_hide();
    VIDEO_CLOCK.with(|t| t.stop());
    crate::video::release();
    VIDEO.with(|v| *v.borrow_mut() = VideoView::default());
    win.set_vw_playing_event("".into());
    win.set_vw_play_pos(0.0);
    win.set_vw_play_duration(0.0);
    win.set_vw_frame(Default::default());
}

// ---- Android: the phone's own player, over the viewer's picture ----------

/// The viewer's picture rectangle in physical pixels, for the platform view.
#[cfg(target_os = "android")]
fn video_rect(win: &AppWindow) -> (i32, i32, i32, i32) {
    let s = win.window().scale_factor();
    let px = |v: f32| (v * s).round() as i32;
    (
        px(win.get_vw_pic_x()),
        px(win.get_vw_pic_y()),
        px(win.get_vw_pic_w()).max(1),
        px(win.get_vw_pic_h()).max(1),
    )
}

/// Play `file` in the phone's player, laid over the picture. An item with
/// no local copy yet is fetched first and then played.
#[cfg(target_os = "android")]
fn video_play_android(req: &Requester, win: &AppWindow, room: &str, event: &str, file: &str, seek: f64) {
    if file.is_empty() {
        let (room, event) = (room.to_string(), event.to_string());
        win.set_vw_playing_event(event.as_str().into());
        call_ui(
            req,
            "media.get",
            json!({"roomId": room, "eventId": event}),
            move |ui, win, out| {
                if win.get_vw_playing_event() != event.as_str() {
                    return;
                }
                match out {
                    Ok(v) => {
                        let path = v["path"].as_str().unwrap_or("").to_string();
                        if path.is_empty() {
                            video_clear(win);
                        } else {
                            video_play_android(&ui.req.clone(), win, &room, &event, &path, seek);
                        }
                    }
                    Err((_, msg)) => {
                        video_clear(win);
                        win.set_vw_toast(SharedString::from(msg));
                    }
                }
            },
        );
        return;
    }
    let (x, y, w, h) = video_rect(win);
    if !crate::platform::video_show(file, x, y, w, h) {
        video_clear(win);
        win.set_vw_toast(SharedString::from("This device cannot play video"));
        return;
    }
    if seek > 0.0 {
        crate::platform::video_seek((seek * 1000.0) as i32);
    }
    win.set_vw_playing_event(event.into());
    win.set_vw_play_pos(seek as f32);
    VIDEO.with(|v| {
        let mut v = v.borrow_mut();
        v.paused = false;
        v.at = seek;
        v.file = file.to_string();
        v.surface.clear();
    });
    video_tick_android(win);
}

/// Follow the phone's player: position and length for the scrub bar, its
/// end, and the picture rectangle, which moves under a pinch or a page turn.
#[cfg(target_os = "android")]
fn video_tick_android(win: &AppWindow) {
    let weak = win.as_weak();
    let mut last = (0i32, 0i32, 0i32, 0i32);
    VIDEO_CLOCK.with(|t| {
        t.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(50),
            move || {
                let Some(win) = weak.upgrade() else { return };
                if win.get_vw_playing_event().is_empty() {
                    return;
                }
                let rect = video_rect(&win);
                if rect != last {
                    last = rect;
                    crate::platform::video_move(rect.0, rect.1, rect.2, rect.3);
                }
                let Some(st) = crate::platform::video_state() else { return };
                if let Some(f) = st.failure {
                    tracing::warn!("video: {f}");
                    video_clear(&win);
                    win.set_vw_toast(SharedString::from("This video could not be played"));
                    return;
                }
                if st.ended {
                    video_clear(&win);
                    return;
                }
                if st.duration_ms > 0 {
                    win.set_vw_play_duration(st.duration_ms as f32 / 1000.0);
                }
                let held = VIDEO.with(|s| {
                    let s = s.borrow();
                    s.paused || s.scrubbing
                });
                if !held {
                    let pos = st.position_ms as f64 / 1000.0;
                    VIDEO.with(|s| s.borrow_mut().at = pos);
                    win.set_vw_play_pos(pos as f32);
                }
            },
        )
    });
}

// ---------------------------------------------------------------------------
// The attach sheet's camera page: the phone's own viewfinder, over the sheet.
//
// The same arrangement as video playback above — a platform view laid over the
// app's one surface at a rectangle the page hands down, followed on a timer
// (java/SigilCamera.java, platform.rs's camera_*). What differs is where the
// rectangle comes from: the viewer is mounted on AppWindow and publishes
// `vw-pic-*` there, while the attach sheet is two components deep with one
// two-way property to its name, so its box and its controls ride the Theme
// global instead (style.slint says why).
//
// The page's controls are STATE, carried down by the poll below; the shutter
// is the one command, and it goes out through `attach_pick` so a shot lands on
// the staging page by the route every other attachment takes.
// ---------------------------------------------------------------------------

/// Whether the shutter callback was the page arriving rather than a capture.
/// `false` means "carry on and take the shot" — which on the desktop, where
/// there is no viewfinder to open, is always the answer.
// The desktop path follows an Android early return; both are real code.
#[allow(unreachable_code)]
fn camera_shutter_handled(win: &AppWindow) -> bool {
    #[cfg(target_os = "android")]
    {
        if win.get_at_page() != "camera" {
            return false;
        }
        if !crate::platform::camera_live() {
            camera_watch(win);
            return true;
        }
        // A camera that is still opening has nothing to give: swallow the tap
        // rather than let it fail its way to an error state.
        let ready = crate::platform::camera_state()
            .map(|s| s.state == "ready" || s.recording)
            .unwrap_or(false);
        return !ready;
    }
    let _ = win;
    false
}

/// What the page last had pushed down to it, so a pass that changes nothing
/// costs no JNI calls.
#[cfg(target_os = "android")]
#[derive(Clone)]
struct CameraView {
    rect: (i32, i32, i32, i32),
    front: bool,
    zoom: f32,
    torch: bool,
    /// The permission dialog is shown once per visit to the page.
    asked: bool,
    /// The last failure toasted: one failure, one toast.
    said: String,
}

#[cfg(target_os = "android")]
impl Default for CameraView {
    fn default() -> Self {
        Self {
            rect: (0, 0, 0, 0),
            front: false,
            zoom: 1.0,
            torch: false,
            asked: false,
            said: String::new(),
        }
    }
}

#[cfg(target_os = "android")]
thread_local! {
    static CAMERA: std::cell::RefCell<CameraView> =
        std::cell::RefCell::new(CameraView::default());
    /// Its own clock: the viewer's video and the attach sheet's camera are
    /// never both on screen, but they are not each other's business either.
    static CAMERA_CLOCK: slint::Timer = slint::Timer::default();
}

/// The preview box in physical pixels, off the Theme global the page publishes
/// it on (style.slint's cam-x/y/w/h, in window coordinates).
#[cfg(target_os = "android")]
fn camera_rect(win: &AppWindow) -> (i32, i32, i32, i32) {
    let t = win.global::<crate::Theme>();
    let s = win.window().scale_factor();
    let px = |v: f32| (v * s).round() as i32;
    (
        px(t.get_cam_x()),
        px(t.get_cam_y()),
        px(t.get_cam_w()).max(1),
        px(t.get_cam_h()).max(1),
    )
}

/// Start following the camera page. Idempotent: entering the page again (or
/// tapping the shutter after a refused permission) simply restarts it.
#[cfg(target_os = "android")]
fn camera_watch(win: &AppWindow) {
    CAMERA.with(|c| *c.borrow_mut() = CameraView::default());
    let weak = win.as_weak();
    CAMERA_CLOCK.with(|t| {
        t.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(50),
            move || {
                if let Some(win) = weak.upgrade() {
                    camera_pass(&win);
                }
            },
        )
    });
}

/// One pass: close the camera if the page has gone, otherwise open it, keep it
/// under the box, and carry the page's controls down.
#[cfg(target_os = "android")]
fn camera_pass(win: &AppWindow) {
    let theme = win.global::<crate::Theme>();

    // The page is gone — the back disc, the sheet closing behind a tile, the
    // composer taking focus, a room change, leaving the chat page, or a
    // capture landing on the staging page. Every one of those routes ends
    // here, and this is the ONLY place the view is closed, so nothing can
    // leave a camera running.
    if !win.get_attach_open() || win.get_at_page() != "camera" {
        CAMERA_CLOCK.with(|t| t.stop());
        if crate::platform::camera_live() {
            crate::platform::camera_stop_video();
            crate::platform::camera_close();
        }
        theme.set_cam_state("".into());
        theme.set_cam_recording(false);
        return;
    }

    let rect = camera_rect(win);
    // The sheet is still animating open and the box has no size yet.
    if rect.2 <= 1 || rect.3 <= 1 {
        return;
    }

    if !crate::platform::camera_live() {
        if !crate::platform::has_camera_permission() {
            let asked = CAMERA.with(|c| {
                let mut c = c.borrow_mut();
                let was = c.asked;
                c.asked = true;
                was
            });
            if !asked {
                crate::platform::request_camera_permission();
            }
            theme.set_cam_state("denied".into());
            return;
        }
        theme.set_cam_state("opening".into());
        let front = theme.get_cam_facing() == "front";
        if !crate::platform::camera_open(rect.0, rect.1, rect.2, rect.3, front) {
            theme.set_cam_state("error".into());
            CAMERA_CLOCK.with(|t| t.stop());
            return;
        }
        CAMERA.with(|c| {
            let mut c = c.borrow_mut();
            c.rect = rect;
            c.front = front;
            c.zoom = 1.0;
            c.torch = false;
        });
        return;
    }

    let mut view = CAMERA.with(|c| c.borrow().clone());
    if rect != view.rect {
        crate::platform::camera_move(rect.0, rect.1, rect.2, rect.3);
        view.rect = rect;
    }
    let front = theme.get_cam_facing() == "front";
    if front != view.front {
        crate::platform::camera_flip();
        view.front = front;
        // The other camera has its own zoom range; the chips go back to 1.0
        // rather than sit lit on a stop this lens cannot reach.
        theme.set_cam_zoom(1.0);
        view.zoom = 1.0;
    }
    let zoom = theme.get_cam_zoom();
    if (zoom - view.zoom).abs() > 0.001 {
        crate::platform::camera_zoom(zoom);
        view.zoom = zoom;
    }
    let torch = theme.get_cam_torch();
    if torch != view.torch {
        crate::platform::camera_torch(torch);
        view.torch = torch;
    }

    if let Some(st) = crate::platform::camera_state() {
        theme.set_cam_zoom_min(st.zoom_min);
        theme.set_cam_zoom_max(st.zoom_max);
        theme.set_cam_has_flash(st.has_flash);
        theme.set_cam_recording(st.recording);
        theme.set_cam_state(st.state.as_str().into());
        if let Some(f) = st.failure {
            if f != view.said {
                tracing::warn!("camera: {f}");
                win.set_chat_toast(f.as_str().into());
                view.said = f;
            }
        }
    }
    CAMERA.with(|c| *c.borrow_mut() = view);
}

fn viewer_open(ui: &mut UiState, win: &AppWindow, event_id: &str) {
    // ImageViewer.qml:70-86 — the viewer lies on a blurred, dimmed picture of
    // the page it came from. That is a live ShaderEffectSource there; Slint
    // cannot sample what is behind an element, so the picture is taken here
    // the way the long-press sheet takes its own (sheet_snapshot above, both
    // through frost.rs) — now, while nothing of the viewer is on screen yet.
    // Taken again for a second picture opened from the first would only
    // photograph the viewer, so it is skipped while one is already up.
    if !win.get_viewer_open() {
        let frost = crate::frost::Snapshot::take(win.window())
            .map(|s| s.frosted())
            .unwrap_or_default();
        win.global::<crate::Theme>().set_viewer_frost(frost);
    }
    ui.viewer_items = ui
        .shadow
        .iter()
        .filter(|i| matches!(s(i, "kind"), "image" | "sticker" | "video"))
        .cloned()
        .collect();
    // The strip the frames come from is shared with the timeline, which may
    // never have looked at this item; the viewer asks for its own.
    if let Some(item) = ui
        .viewer_items
        .iter()
        .find(|i| s(i, "eventId") == event_id)
        .cloned()
    {
        gif_frames_for_viewer(ui, &item);
    }
    let cur = ui
        .viewer_items
        .iter()
        .position(|i| s(i, "eventId") == event_id)
        .unwrap_or(0);
    let items = ui.viewer_items.clone();
    let room_id = room_of_key(&ui.open_room);
    let rows: Vec<crate::ViewerItem> = items
        .iter()
        .map(|i| {
            let media = i["media"].clone();
            // A video's `path` is the clip itself; the picture to show is its
            // poster. BubbleDelegate.qml:466 never hands a video file to the
            // image decoder either — it only logged an error and drew black.
            let path = if s(i, "kind") == "video" {
                media["thumbnailPath"].as_str().unwrap_or("")
            } else {
                media["path"]
                    .as_str()
                    .or(media["thumbnailPath"].as_str())
                    .unwrap_or("")
            }
            .to_string();
            let img = crate::bridge::avatar_pub(ui, &path);
            // GIFs animate here too, from the frame strip the bubble cached.
            let mut gif_imgs: Vec<slint::Image> = Vec::new();
            let mut gif_delays: Vec<i32> = Vec::new();
            if let Some(v) = ui
                .gif_frames
                .get(&format!("{room_id}|{}", s(i, "eventId")))
                .cloned()
            {
                if v.is_object() {
                    let paths: Vec<String> = v["frames"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(Value::as_str)
                                .map(str::to_string)
                                .collect()
                        })
                        .unwrap_or_default();
                    for p in &paths {
                        if let Some(fimg) = crate::bridge::avatar_pub(ui, p) {
                            gif_imgs.push(fimg);
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
            }
            crate::ViewerItem {
                gif_frames: ModelRc::new(VecModel::from(gif_imgs)),
                gif_delays: ModelRc::new(VecModel::from(gif_delays)),
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
        })
        .collect();
    let names: Vec<SharedString> = ui
        .rooms_json
        .iter()
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
            ui.req
                .fire("media.get", json!({"roomId": room, "eventId": ev}));
        }
    }
}

fn viewer_misc(ui: &mut UiState, win: &AppWindow, action: &str, a: &str) {
    let i = win.get_vw_cur().max(0) as usize;
    let Some(item) = ui.viewer_items.get(i).cloned() else {
        return;
    };
    let ev = s(&item, "eventId").to_string();
    let room = room_of_key(&ui.open_room);
    let req = ui.req.clone();
    match action {
        "viewer-download" => {
            let dest = format!("{}/Downloads", std::env::var("HOME").unwrap_or_default());
            call_ui(
                &req,
                "media.saveAs",
                json!({"roomId": room, "eventId": ev, "dest": dest}),
                |_ui, win, out| {
                    toast(win, Toast::Viewer, 
                        match out {
                            Ok(v) => format!("Saved to {}", s(&v, "path")),
                            Err((_, m)) => m,
                        }
                        .into(),
                    );
                },
            );
        }
        "viewer-delete" => {
            req.fire("message.redact", json!({"roomId": room, "eventId": ev}));
            win.set_viewer_open(false);
            // Pulled down rather than closed, so nothing reports `closed`:
            // drop the frost here or it outlives the viewer.
            win.global::<crate::Theme>().set_viewer_frost(Default::default());
        }
        // ImageViewer.qml:690 — the reaction is silent otherwise: the row is
        // at the foot of the viewer and the chip it adds is behind it.
        "viewer-react" => {
            req.fire(
                "message.react",
                json!({"roomId": room, "eventId": ev, "key": a}),
            );
            toast(win, Toast::Viewer, format!("Reacted {a}").into());
        }
        "viewer-share" => {
            let path = item["media"]["path"].as_str().unwrap_or("").to_string();
            if !path.is_empty() {
                crate::platform::copy_text(&path);
                toast(win, Toast::Viewer, "Path copied".into());
            }
        }
        "viewer-forward" => {
            let idx: usize = a.parse().unwrap_or(0);
            let target = ui
                .rooms_json
                .iter()
                .filter(|r| !b(r, "isSpace") && !b(r, "isInvite"))
                .nth(idx)
                .map(|r| (s(r, "id").to_string(), s(r, "name").to_string()));
            let path = item["media"]["path"]
                .as_str()
                .or(item["media"]["thumbnailPath"].as_str())
                .unwrap_or("")
                .to_string();
            if let Some((rid, name)) = target {
                if !path.is_empty() {
                    req.fire("attachment.send", json!({"roomId": rid, "path": path}));
                    // ImageViewer.qml:609 names the room it went to; the card
                    // is gone by the time the toast shows, so "Forwarded"
                    // alone left no way to tell where.
                    let name = if name.is_empty() { "room".to_string() } else { name };
                    toast(win, Toast::Viewer, format!("Forwarded to {name}").into());
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
        // DocumentPage.qml:448 elides in the middle; the name comes pre-shortened
        win.set_dc_name(
            crate::rows::elide_middle(item["media"]["filename"].as_str().unwrap_or("Document"), 34).into(),
        );
        win.set_dc_size(item["media"]["sizeLabel"].as_str().unwrap_or("").into());
    }
    call_ui(
        &ui.req.clone(),
        "doc.preview",
        json!({"roomId": room, "eventId": event_id}),
        move |ui, win, out| match out {
            Ok(v) => {
                win.set_dc_status("".into());
                let kind = s(&v, "kind").to_string();
                win.set_dc_kind(kind.as_str().into());
                // Only a PDF is drawn page by page; everything else comes
                // as blocks, whatever page count the reader guessed.
                let pages = if b(&v, "rasterisable") {
                    v["pageCount"].as_i64().unwrap_or(0)
                } else {
                    0
                };
                // The page prefixes the kind word itself ("PDF · 2 pages").
                win.set_dc_subtitle(
                    match kind.as_str() {
                        "pdf" if pages == 1 => "1 page".to_string(),
                        "pdf" => format!("{pages} pages"),
                        "sheet" => {
                            let n = v["sheets"].as_array().map(Vec::len).unwrap_or(0);
                            if n == 1 {
                                "1 sheet".to_string()
                            } else {
                                format!("{n} sheets")
                            }
                        }
                        _ => String::new(),
                    }
                    .into(),
                );
                win.set_dc_pdf(pages > 0);
                if pages > 0 {
                    ui.doc_pages = vec![Value::Null; pages as usize];
                    let blanks: Vec<crate::DocPageImage> =
                        (0..pages).map(|_| crate::DocPageImage::default()).collect();
                    win.set_dc_pages(ModelRc::new(VecModel::from(blanks)));
                }
                let blocks: Vec<crate::DocBlock> = v["blocks"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|b3| crate::DocBlock {
                                t: s(b3, "t").into(),
                                text: s(b3, "text").into(),
                                title: s(b3, "title").into(),
                                level: b3["level"].as_i64().unwrap_or(0) as i32,
                                bullet: b(b3, "bullet"),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                win.set_dc_blocks(ModelRc::new(VecModel::from(blocks)));
                let sheets: Vec<crate::SheetTab> = v["sheets"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|t| crate::SheetTab {
                                name: s(t, "name").into(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                win.set_dc_sheets(ModelRc::new(VecModel::from(sheets)));
                doc_push_sheet(win, &v, 0);
                ui.doc_preview = v;
            }
            Err((_, m)) => {
                win.set_dc_status("error".into());
                win.set_dc_error(m.as_str().into());
            }
        },
    );
}

fn doc_push_sheet(win: &AppWindow, preview: &Value, index: usize) {
    let rows_v = preview["sheetRows"].as_array().cloned().unwrap_or_default();
    let mut cols = 1;
    let rows: Vec<crate::SheetRow> = rows_v
        .iter()
        .filter(|r| r["sheet"].as_i64().unwrap_or(0) as usize == index)
        .map(|r| {
            let cells: Vec<SharedString> = r["cells"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|c| SharedString::from(c.as_str().unwrap_or("")))
                        .collect()
                })
                .unwrap_or_default();
            cols = cols.max(cells.len() as i32);
            crate::SheetRow {
                cells: ModelRc::new(VecModel::from(cells)),
            }
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
    call_ui(
        &ui.req.clone(),
        "doc.page",
        json!({"roomId": rid, "eventId": ev, "index": idx, "width": w as i64}),
        move |ui, win, out| {
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
        },
    );
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
        // AudioPage.qml:235 elides in the middle; Slint cannot, so the name comes pre-shortened
        win.set_au_title(crate::rows::elide_middle(item["media"]["filename"].as_str().unwrap_or("Audio"), 34).into());
        win.set_au_size(item["media"]["sizeLabel"].as_str().unwrap_or("").into());
        win.set_au_duration((item["media"]["duration"].as_f64().unwrap_or(0.0) / 1000.0) as f32);
    }
    call_ui(
        &ui.req.clone(),
        "audio.info",
        json!({"roomId": room, "eventId": event_id, "size": 512}),
        move |ui, win, out| {
            win.set_au_status("".into());
            let Ok(v) = out else { return };
            if let Some(d) = v["duration"].as_f64() {
                if d > 0.0 {
                    win.set_au_duration((d / 1000.0) as f32);
                }
            }
            let art = s(&v, "artPath").to_string();
            if let Some(img) = crate::bridge::avatar_pub(ui, &art) {
                win.set_au_art(img);
                win.set_au_have_art(true);
            }
            if let Some(hex) = v["accent"].as_str() {
                if let Ok(c) = u32::from_str_radix(hex.trim_start_matches('#'), 16) {
                    win.set_au_tone(slint::Color::from_rgb_u8(
                        (c >> 16) as u8,
                        (c >> 8) as u8,
                        c as u8,
                    ));
                }
            }
        },
    );
}

fn audio_page_toggle(ui: &mut UiState, win: &AppWindow) {
    let (rid, ev) = ui.audio_ctx.clone();
    if win.get_au_playing() {
        ui.req.fire("audio.stop", json!({}));
        win.set_au_playing(false);
    } else {
        ui.req.fire(
            "audio.play",
            json!({"roomId": rid, "eventId": ev, "seek": win.get_au_position() as f64}),
        );
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

/// What an attach tile asked for. Media ends on the staging page, where a
/// caption is written before anything goes; anything else is still a file on
/// disk sent straight to the open room.
#[derive(Clone, Copy)]
enum AttachPick {
    /// Any file at all (AttachMenu.qml's only media route).
    Files,
    /// Pictures and video from the gallery, several at a time.
    Gallery,
    /// The camera, for a still.
    Photo,
    /// The camera, recording.
    Video,
}

/// The extensions that stage instead of sending. A picture or a video gets a
/// caption written on it first — the platform messenger's own step, and the
/// one this app was missing; every other file goes as it always did, because
/// there is nothing to look at while you caption it.
fn media_kind(path: &str) -> Option<bool> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "heic" | "heif" | "avif" | "tif"
        | "tiff" => Some(false),
        "mp4" | "mov" | "m4v" | "mkv" | "webm" | "3gp" | "3gpp" | "avi" => Some(true),
        _ => None,
    }
}

fn base_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string()
}

/// Put files on the staging page. `append` is the add-more button, which
/// keeps what is already staged and the caption written so far.
fn stage_media(win: &AppWindow, paths: &[String], append: bool) {
    let mut rows: Vec<crate::StagedItem> = if append {
        win.get_sg_items().iter().collect()
    } else {
        Vec::new()
    };
    for path in paths {
        let video = media_kind(path).unwrap_or(false);
        // A video has no poster here — the engine's video surface decodes a
        // stream it is already playing, not an arbitrary container — so it
        // stages as its own dark card and the picture stays empty.
        let img = if video {
            slint::Image::default()
        } else {
            slint::Image::load_from_path(std::path::Path::new(path)).unwrap_or_default()
        };
        let size = img.size();
        rows.push(crate::StagedItem {
            img,
            path: path.as_str().into(),
            name: base_name(path).into(),
            video,
            w: size.width as f32,
            h: size.height as f32,
        });
    }
    if rows.is_empty() {
        return;
    }
    let count = rows.len();
    win.set_sg_items(ModelRc::new(VecModel::from(rows)));
    if append {
        // Land on what was just added.
        win.set_sg_cur(count as i32 - 1);
    } else {
        win.set_sg_cur(0);
        win.set_sg_caption(SharedString::new());
    }
    win.set_attach_open(false);
    win.set_nav("staging".into());
}

fn attach_pick(ui: &mut UiState, what: AttachPick, append: bool) {
    let room = room_of_key(&ui.open_room);
    ui.req.handle().spawn(async move {
        // The picker is a whole system UI away on Android and a subprocess on
        // the desktop; this can take a minute — a whole photo shoot, for the
        // camera — and the sheet has closed itself in the meantime
        // (attach.slint fires close-requested with the tile).
        let paths: Vec<String> = match what {
            AttachPick::Files => crate::platform::pick_file().await.into_iter().collect(),
            AttachPick::Gallery => crate::platform::pick_media().await,
            AttachPick::Photo => crate::platform::capture_media(false)
                .await
                .into_iter()
                .collect(),
            AttachPick::Video => crate::platform::capture_media(true)
                .await
                .into_iter()
                .collect(),
        };
        if paths.is_empty() {
            // Backing out of the picker is the ordinary case, so this is not a
            // warning — but it is the one line that tells the two apart in a
            // log when nothing arrives in the room.
            tracing::info!("attach: nothing was chosen");
            return;
        }
        let _ = slint::invoke_from_event_loop(move || {
            with_ui(|ui| {
                let Some(win) = ui.win.upgrade() else { return };
                // Pictures and video stage; the file picker's other answers
                // are sent as they always were, one each, in the order they
                // were chosen.
                let (media, plain): (Vec<String>, Vec<String>) =
                    paths.into_iter().partition(|p| media_kind(p).is_some());
                for path in &plain {
                    ui.req.fire(
                        "attachment.send",
                        json!({"roomId": room.clone(), "path": path}),
                    );
                }
                if media.is_empty() {
                    win.set_attach_open(false);
                } else {
                    stage_media(&win, &media, append);
                }
            });
        });
    });
}

/// Everything staged goes now, in order, with the caption on the FIRST of
/// them: `attachment.send` takes a `caption` beside the path (core's
/// `Manifest.caption`), so it rides on the same event the picture does —
/// the timeline shows it under the media (bubble.slint's `caption-visible`,
/// which is "the body is not just the filename"). Repeating it under every
/// picture of a set would read as a stutter, so the set is captioned once.
fn staging_send(ui: &mut UiState, win: &AppWindow, caption: &str) {
    let room = room_of_key(&ui.open_room);
    let caption = caption.trim().to_string();
    let items = win.get_sg_items();
    for (i, it) in items.iter().enumerate() {
        let mut p = json!({"roomId": room.clone(), "path": it.path.as_str()});
        if i == 0 && !caption.is_empty() {
            p["caption"] = json!(caption);
        }
        ui.req.fire("attachment.send", p);
    }
    staging_clear(win);
    win.set_nav("chat".into());
}

fn staging_clear(win: &AppWindow) {
    win.set_sg_items(ModelRc::new(VecModel::from(Vec::<crate::StagedItem>::new())));
    win.set_sg_cur(0);
    win.set_sg_caption(SharedString::new());
}

/// One item's close disc. The last one taken off is the whole pick abandoned,
/// so the page goes with it.
fn staging_remove(win: &AppWindow, idx: usize) {
    let rows: Vec<crate::StagedItem> = win
        .get_sg_items()
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != idx)
        .map(|(_, it)| it)
        .collect();
    if rows.is_empty() {
        staging_clear(win);
        win.set_nav("chat".into());
        return;
    }
    let last = rows.len() as i32 - 1;
    win.set_sg_items(ModelRc::new(VecModel::from(rows)));
    win.set_sg_cur(win.get_sg_cur().min(last).max(0));
}

fn create_poll(ui: &mut UiState, question: &str, packed: &str) {
    let mut parts = packed.split('\u{1f}');
    let closed = parts.next().unwrap_or("0") == "1";
    // The sheet's only seam here is create-poll(question, options, closed),
    // so "voters may pick more than one" arrives as a \u{1}1 / \u{1}0 header
    // on the question — a control character no text field can produce, and
    // one an older caller (drive.rs) simply does not send.
    let (multi, question) = match question.strip_prefix('\u{1}') {
        Some(rest) => (rest.starts_with('1'), rest.get(1..).unwrap_or("")),
        None => (false, question),
    };
    let question = question.trim();
    // The sheet cannot trim (Slint has no trim, and its option slots are
    // plain text), so this is where a blank or whitespace-only option is
    // dropped — before the poll is built, not merely hidden in the view.
    let options: Vec<&str> = parts.map(str::trim).filter(|o| !o.is_empty()).collect();
    if options.len() < 2 || question.is_empty() {
        if let Some(win) = ui.win.upgrade() {
            win.set_chat_toast(
                if question.is_empty() {
                    "A poll needs a question"
                } else {
                    "A poll needs two options"
                }
                .into(),
            );
        }
        return;
    }
    // Multiple answers means "as many as there are" — the engine clamps
    // maxSelections to the option count anyway (core kinds.rs poll_create).
    let max = if multi { options.len() } else { 1 };
    let room = room_of_key(&ui.open_room);
    ui.req.fire(
        "poll.create",
        json!({"roomId": room, "question": question, "options": options,
               "closed": closed, "maxSelections": max}),
    );
    if let Some(win) = ui.win.upgrade() {
        win.set_attach_open(false);
    }
}

/// Show a vote the instant it is cast: `mine` moves to the answers just
/// chosen, each one's count and the voter total move with it, and the engine's
/// next timeline diff overwrites the lot with the truth.
fn poll_echo(ui: &mut UiState, event_id: &str, answers: &[String]) {
    let Some(item) = ui
        .shadow
        .iter_mut()
        .find(|i| i["eventId"].as_str() == Some(event_id))
    else {
        return;
    };
    let Some(poll) = item.get_mut("poll").and_then(Value::as_object_mut) else {
        return;
    };
    if poll.get("ended").and_then(Value::as_bool).unwrap_or(false) {
        return;
    }
    let mut voted_before = false;
    if let Some(list) = poll.get_mut("answers").and_then(Value::as_array_mut) {
        for o in list.iter_mut() {
            let id = o["id"].as_str().unwrap_or("").to_string();
            let was = o["mine"].as_bool().unwrap_or(false);
            let now = answers.iter().any(|x| *x == id);
            voted_before |= was;
            if was != now {
                let n = o["votes"].as_i64().unwrap_or(0) + if now { 1 } else { -1 };
                o["votes"] = json!(n.max(0));
            }
            o["mine"] = json!(now);
        }
    }
    let voters = poll.get("voters").and_then(Value::as_i64).unwrap_or(0);
    let voters = match (voted_before, answers.is_empty()) {
        (false, false) => voters + 1, // a new voter
        (true, true) => voters - 1,   // took the vote back
        _ => voters,
    };
    poll.insert("voters".into(), json!(voters.max(0)));
}

fn load_stickers(ui: &mut UiState, _win: &AppWindow) {
    call_ui(
        &ui.req.clone(),
        "stickers.list",
        json!({}),
        |ui, win, out| {
            let Ok(v) = out else { return };
            win.set_at_sticker_dir(s(&v, "dir").into());
            ui.stickers = v["stickers"].as_array().cloned().unwrap_or_default();
            let stickers = ui.stickers.clone();
            let rows: Vec<crate::StickerItem> = stickers
                .iter()
                .map(|st| crate::StickerItem {
                    path: s(st, "path").into(),
                    art: crate::bridge::avatar_pub(ui, s(st, "path")).unwrap_or_default(),
                    body: s(st, "body").into(),
                    url: s(st, "url").into(),
                    w: st["width"].as_i64().unwrap_or(0) as i32,
                    h: st["height"].as_i64().unwrap_or(0) as i32,
                })
                .collect();
            win.set_at_stickers(ModelRc::new(VecModel::from(rows)));
            win.set_at_stickers_loaded(true);
        },
    );
}

fn send_sticker(ui: &mut UiState, index: &str) {
    let idx: usize = index.parse().unwrap_or(0);
    let Some(st) = ui.stickers.get(idx).cloned() else {
        return;
    };
    let room = room_of_key(&ui.open_room);
    ui.req.fire(
        "sticker.send",
        json!({
            "roomId": room,
            "url": s(&st, "url"),
            "body": match s(&st, "body") { "" => "Sticker", b3 => b3 },
            "width": st["width"].as_i64().unwrap_or(0),
            "height": st["height"].as_i64().unwrap_or(0),
        }),
    );
    if let Some(win) = ui.win.upgrade() {
        win.set_attach_open(false);
    }
}

// ---------------------------------------------------------------- theme

/// The saved themes, read once at start; theme_apply keeps it current.
pub fn load_themes() -> Value {
    std::fs::read_to_string(themes_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}))
}

fn themes_path() -> String {
    format!(
        "{}/.local/state/sigil/chat-themes.json",
        std::env::var("HOME").unwrap_or_default()
    )
}

/// The nine wallpaper gradients: 3 hue shifts x 3 depths from the pending
/// accent (ChatThemePage.qml:34-45 gradPair). A grey accent has no hue; the
/// QML's hslHue < 0 falls back to 0.6 and so does this.
fn theme_gradients(accent: &str) -> Vec<crate::GradPair> {
    let c = u32::from_str_radix(accent.trim_start_matches('#'), 16).unwrap_or(0x00a8a8a8);
    let (r, g, b) = (
        ((c >> 16) & 0xff) as f32 / 255.0,
        ((c >> 8) & 0xff) as f32 / 255.0,
        (c & 0xff) as f32 / 255.0,
    );
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    let l = (mx + mn) / 2.0;
    let d = mx - mn;
    let s2 = if d == 0.0 { 0.0 } else { d / (1.0 - (2.0 * l - 1.0).abs()) };
    let h = if d == 0.0 {
        0.6
    } else if mx == r {
        (((g - b) / d).rem_euclid(6.0)) / 6.0
    } else if mx == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    let hsl = |h: f32, s2: f32, l: f32| {
        let c = (1.0 - (2.0 * l - 1.0).abs()) * s2;
        let x = c * (1.0 - ((h * 6.0) % 2.0 - 1.0).abs());
        let m = l - c / 2.0;
        let (r, g, b) = match (h * 6.0) as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        slint::Color::from_rgb_u8(
            ((r + m) * 255.0).round() as u8,
            ((g + m) * 255.0).round() as u8,
            ((b + m) * 255.0).round() as u8,
        )
    };
    (0..9)
        .map(|i| {
            let hh = (h + [-0.04, 0.0, 0.04][i % 3] + 1.0) % 1.0;
            let row = i / 3;
            let sat = s2.max(0.5);
            let l2 = l.clamp(0.25, 0.55);
            let top = [l2 * 1.15, l2 * 0.85, l2 * 0.6][row].min(0.62);
            let bot = [l2 * 0.45, l2 * 0.3, l2 * 0.18][row];
            crate::GradPair { top: hsl(hh, sat, top), bot: hsl(hh, sat, bot) }
        })
        .collect()
}

fn push_theme(ui: &mut UiState, win: &AppWindow) {
    let t = ui.theme_pending.clone();
    set_theme_props(ui, win, &t);
}

/// The swatches the chat-theme page offers, in its own order. The page
/// resolves the same six to colours (pages/chattheme.slint's `pal-color`,
/// since Slint parses no colour from a string); this side decides whether a
/// pending accent is one of them or a colour of the user's own.
pub const THEME_PALETTE: [&str; 6] = [
    "#7c9fd4", "#5cb8d6", "#b48ad6", "#9aab7e", "#e0a370", "#d98aa8",
];

/// Everything the chrome derives from a theme record {accent, wallpaper}:
/// used for the editor's pending copy and for a room's saved theme on open.
pub fn set_theme_props(ui: &mut UiState, win: &AppWindow, t: &Value) {
    let accent = t["accent"].as_str().unwrap_or("").to_string();
    let wallpaper = t["wallpaper"].as_str().unwrap_or("").to_string();
    win.set_ct_accent(accent.as_str().into());
    win.set_ct_wallpaper(wallpaper.as_str().into());
    if let Ok(c) = u32::from_str_radix(accent.trim_start_matches('#'), 16) {
        win.set_ct_color(slint::Color::from_rgb_u8(
            (c >> 16) as u8,
            (c >> 8) as u8,
            c as u8,
        ));
    }
    // "Custom" means a colour the palette does not offer, not merely that a
    // colour is set: picking a palette swatch was lighting the custom entry
    // as well as the swatch.
    win.set_ct_custom(!accent.is_empty() && !THEME_PALETTE.contains(&accent.as_str()));
    win.set_ct_gradients(ModelRc::new(VecModel::from(theme_gradients(if accent.is_empty() { "#a8a8a8" } else { &accent }))));
    if let Some(n) = wallpaper
        .strip_prefix("grad:")
        .and_then(|n| n.parse::<i32>().ok())
    {
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
    let mut all: Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}));
    let pending = ui.theme_pending.clone();
    let empty = pending["accent"].as_str().unwrap_or("").is_empty()
        && pending["wallpaper"].as_str().unwrap_or("").is_empty();
    if empty {
        if let Some(o) = all.as_object_mut() {
            o.remove(&rid);
        }
    } else {
        all[rid.as_str()] = json!({"accent": pending["accent"], "wallpaper": pending["wallpaper"]});
    }
    if let Some(dir) = std::path::Path::new(&path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, all.to_string());
    ui.chat_themes = all;
    crate::bridge::invoke_later(win, |w| w.invoke_go_back());
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
    (
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b3 + m) * 255.0) as u8,
    )
}

// ---------------------------------------------------------------- places, people

/// position.get's shape onto the picker. A fresh fix re-crops the picker's
/// map when its page is showing (LocationPicker.qml:118-120 — the view
/// follows the fix unless a pin holds the centre).
pub fn apply_position(ui: &mut UiState, win: &AppWindow, v: &Value) {
    let known = b(v, "known");
    win.set_lp_have_fix(known);
    win.set_lp_lat(v["lat"].as_f64().unwrap_or(0.0) as f32);
    win.set_lp_lon(v["lon"].as_f64().unwrap_or(0.0) as f32);
    win.set_lp_error(s(v, "error").into());
    let fix = if known {
        Some((
            v["lat"].as_f64().unwrap_or(0.0),
            v["lon"].as_f64().unwrap_or(0.0),
        ))
    } else {
        None
    };
    let moved = fix != ui.lp_fix;
    ui.lp_fix = fix;
    if moved
        && ui.lp_mark.is_none()
        && win.get_attach_open()
        && matches!(win.get_at_page().as_str(), "pin" | "current" | "live")
    {
        request_lp_map_debounced(ui);
    }
}

fn refresh_position(ui: &mut UiState) {
    call_ui(
        &ui.req.clone(),
        "position.refresh",
        json!({}),
        |ui, win, out| {
            if let Ok(v) = out {
                apply_position(ui, win, &v);
            }
        },
    );
}

// The picker's imagery: the engine's crop is in Web-Mercator world pixels —
// at zoom z the world is TILE_PX·2^z across (core/src/maps/composite.rs
// world_px, 512px tiles) — and the request is exactly 2× the map box, so one
// logical box pixel is two world pixels.
const LP_TILE_PX: f64 = 512.0;

fn lp_world(zoom: i64) -> f64 {
    LP_TILE_PX * f64::powi(2.0, zoom as i32)
}

fn lat_to_world_y(lat: f64, zoom: i64) -> f64 {
    let lr = lat.to_radians();
    (1.0 - ((lr.tan() + 1.0 / lr.cos()).ln()) / std::f64::consts::PI) / 2.0 * lp_world(zoom)
}

fn world_y_to_lat(y: f64, zoom: i64) -> f64 {
    // the inverse: lat = atan(sinh(π·(1 − 2y/world)))
    (std::f64::consts::PI * (1.0 - 2.0 * y / lp_world(zoom)))
        .sinh()
        .atan()
        .to_degrees()
}

/// A tap at (x, y) logical px in the picker's map box → the point under it.
fn lp_tap_latlon(view: &crate::bridge::LpView, x: f64, y: f64) -> (f64, f64) {
    let dx = (x - view.box_w / 2.0) * 2.0;
    let dy = (y - view.box_h / 2.0) * 2.0;
    let lon = view.lon + dx / lp_world(view.zoom) * 360.0;
    let lat = world_y_to_lat(lat_to_world_y(view.lat, view.zoom) + dy, view.zoom);
    (
        lat.clamp(-85.05, 85.05),
        (lon + 180.0).rem_euclid(360.0) - 180.0,
    )
}

/// The picker's crop: centred on the pin, else the fix, else mid-US at z4
/// (LocationPicker.qml:45-46, :118-120), at 2× the map box. The box height
/// is the attach.slint constant per mode — keep them in step.
fn request_lp_map(ui: &mut UiState, win: &AppWindow) {
    let (lat, lon, zoom) = match (ui.lp_mark, ui.lp_fix) {
        (Some((la, lo)), _) => (la, lo, ui.lp_zoom),
        (None, Some((la, lo))) => (la, lo, ui.lp_zoom),
        (None, None) => (39.5, -98.35, 4),
    };
    let box_w = (win.get_logical_width() as f64 - 32.0).max(64.0);
    let box_h = if win.get_at_page() == "live" { 294.0 } else { 340.0 };
    ui.lp_view = Some(crate::bridge::LpView {
        lat,
        lon,
        zoom,
        box_w,
        box_h,
    });
    let epoch = ui.lp_epoch;
    call_ui(
        &ui.req.clone(),
        "location.map",
        json!({"geoUri": format!("geo:{lat:.6},{lon:.6}"),
               "width": (box_w * 2.0) as u64, "height": (box_h * 2.0) as u64, "zoom": zoom}),
        move |ui, win, out| {
            if ui.lp_epoch != epoch {
                return; // the picker moved on
            }
            match out {
                Ok(v) => {
                    if let Some(img) =
                        crate::bridge::avatar_pub(ui, v["path"].as_str().unwrap_or(""))
                    {
                        win.set_lp_map(img);
                        win.set_lp_unavailable(false);
                    }
                }
                Err((code, msg)) => {
                    // No style configured: the pin card stays, and says so.
                    if code == "unavailable" {
                        win.set_lp_unavailable(true);
                    }
                    tracing::debug!("location.map (picker): {code} {msg}");
                }
            }
        },
    );
}

/// Re-crop 150ms after the last tap or zoom, dropping superseded requests.
fn request_lp_map_debounced(ui: &mut UiState) {
    ui.lp_epoch += 1;
    let epoch = ui.lp_epoch;
    after(&ui.req.clone(), 150, move |ui, win| {
        if ui.lp_epoch == epoch {
            request_lp_map(ui, win);
        }
    });
}

/// The map page for a place: the card's facts, no tiles.
fn open_map(ui: &mut UiState, win: &AppWindow, event_id: &str) {
    let Some(item) = ui
        .shadow
        .iter()
        .find(|i| s(i, "eventId") == event_id)
        .cloned()
    else {
        return;
    };
    let loc = &item["location"];
    let live = &item["liveShare"];
    let is_own = b(&item, "isOwn");
    let running = live["live"].as_bool().unwrap_or(false);
    let ended = live["ended"].as_bool().unwrap_or(false);
    let expires = live["expiresAt"].as_f64().unwrap_or(0.0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0);
    let sender_name = s(&item, "senderName").to_string();
    win.set_mp_who(if is_own {
        "You".into()
    } else {
        sender_name.as_str().into()
    });
    win.set_mp_own(is_own);
    win.set_mp_live(running);
    win.set_mp_ended(ended);
    win.set_mp_stoppable(is_own && running);
    win.set_mp_lat(loc["lat"].as_f64().unwrap_or(0.0) as f32);
    win.set_mp_lon(loc["lon"].as_f64().unwrap_or(0.0) as f32);
    win.set_mp_desc(s(loc, "description").into());
    win.set_mp_self(loc["self"].as_bool().unwrap_or(true));
    win.set_mp_initials(crate::rows::initials(&sender_name).into());
    win.set_mp_tint(crate::rows::tint_for(s(&item, "sender")));
    // The sender's face for the pin's head and the footer row (MapPage.qml:410).
    let face = s(&item, "senderAvatarPath").to_string();
    win.set_mp_avatar(crate::bridge::avatar_pub(ui, &face).unwrap_or_default());
    let stamp = crate::rows::bubble_stamp(item["ts"].as_i64().unwrap_or(0));
    win.set_mp_status(
        if running {
            format!(
                "Sharing until {}",
                crate::rows::bubble_stamp(expires as i64)
            )
        } else if ended {
            "Live share ended".to_string()
        } else {
            format!("Shared {stamp}")
        }
        .into(),
    );
    let left = ((expires - now) / 1000.0).max(0.0) as u64;
    win.set_mp_remaining(
        if running && left > 0 {
            if left >= 3600 {
                format!("{}h {:02}m", left / 3600, (left % 3600) / 60)
            } else {
                format!("{}:{:02}", left / 60, left % 60)
            }
        } else {
            String::new()
        }
        .into(),
    );
    // The page's imagery: a fresh composite at its own size; the card's
    // 640×400 stays for the bubble. Clear the last point's picture first.
    win.set_mp_map(Default::default());
    ui.map_geo = s(loc, "geoUri").to_string();
    ui.map_zoom = 15;
    fetch_map_page(ui);
    // The live grid centres on the same point. It only draws where the server
    // serves single tiles; where it does not, the composite above stands.
    win.set_mp_tiles(ModelRc::new(VecModel::from(Vec::<crate::MapTileView>::new())));
    ui.mapview.open(
        loc["lat"].as_f64().unwrap_or(0.0),
        loc["lon"].as_f64().unwrap_or(0.0),
    );
    map_place(ui, win);
}

/// dm.create is idempotent: an existing conversation comes back.
fn start_dm(ui: &mut UiState, user_id: &str) {
    call_ui(
        &ui.req.clone(),
        "dm.create",
        json!({"userId": user_id}),
        |ui, win, out| {
            if let Ok(v) = out {
                let rid = s(&v, "roomId").to_string();
                if !rid.is_empty() {
                    crate::bridge::open_room(ui, win, &rid);
                    win.set_nav("chat".into());
                }
            }
        },
    );
}

/// The sheet's quick reactions and the open picker, again, once an emoji
/// picture has arrived.
pub fn refresh_emoji_views(ui: &mut UiState, win: &AppWindow) {
    let quick: Vec<crate::EmojiItem> = ["👍", "❤️", "😂", "😮", "😢", "😡"]
        .iter()
        .map(|g| {
            let img = crate::bridge::emoji_image(ui, g);
            crate::EmojiItem {
                glyph: (*g).into(),
                name: "".into(),
                has_img: img.is_some(),
                img: img.unwrap_or_default(),
            }
        })
        .collect();
    win.set_quick_emoji(ModelRc::new(VecModel::from(quick)));
    if let Some(q) = ui.emoji_query.clone() {
        push_emoji(ui, win, &q);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::LpView;

    fn view(lat: f64, lon: f64, zoom: i64) -> LpView {
        LpView {
            lat,
            lon,
            zoom,
            box_w: 368.0,
            box_h: 340.0,
        }
    }

    #[test]
    fn mercator_y_round_trips() {
        for &lat in &[-72.3, -33.9, 0.0, 37.7749, 51.5007, 84.9] {
            for &z in &[3i64, 10, 16, 19] {
                let back = world_y_to_lat(lat_to_world_y(lat, z), z);
                assert!((back - lat).abs() < 1e-9, "lat {lat} z {z} came back {back}");
            }
        }
    }

    #[test]
    fn a_centre_tap_lands_on_the_centre() {
        let v = view(51.5007, -0.1246, 16);
        let (lat, lon) = lp_tap_latlon(&v, v.box_w / 2.0, v.box_h / 2.0);
        assert!((lat - 51.5007).abs() < 1e-9);
        assert!((lon - -0.1246).abs() < 1e-9);
    }

    #[test]
    fn taps_move_the_pin_the_right_way() {
        let v = view(51.5007, -0.1246, 16);
        // 10 logical px right = 20 world px = 20/(512·2^16)·360 degrees east.
        let (_, lon) = lp_tap_latlon(&v, v.box_w / 2.0 + 10.0, v.box_h / 2.0);
        let dlon = 20.0 / (512.0 * 65536.0) * 360.0;
        assert!((lon - (-0.1246 + dlon)).abs() < 1e-12);
        // Below the centre is south; above is north.
        let (south, _) = lp_tap_latlon(&v, v.box_w / 2.0, v.box_h / 2.0 + 40.0);
        let (north, _) = lp_tap_latlon(&v, v.box_w / 2.0, v.box_h / 2.0 - 40.0);
        assert!(south < 51.5007 && 51.5007 < north);
    }

    #[test]
    fn a_tap_round_trips_through_a_recentred_crop() {
        // Drop a pin off-centre, recentre the crop on it, and the same box
        // point relative to the new centre names the same place.
        let v = view(37.7749, -122.4194, 15);
        let (lat, lon) = lp_tap_latlon(&v, 300.0, 80.0);
        let v2 = view(lat, lon, 15);
        let (lat2, lon2) = lp_tap_latlon(&v2, v2.box_w / 2.0, v2.box_h / 2.0);
        assert!((lat2 - lat).abs() < 1e-9 && (lon2 - lon).abs() < 1e-9);
    }

    #[test]
    fn longitude_wraps_and_latitude_clamps() {
        let v = view(0.0, 179.9, 3); // a tiny world: taps reach far
        let (_, lon) = lp_tap_latlon(&v, v.box_w / 2.0 + 100.0, v.box_h / 2.0);
        assert!((-180.0..=180.0).contains(&lon) && lon < 0.0, "wrapped east: {lon}");
        let v = view(84.0, 0.0, 3);
        let (lat, _) = lp_tap_latlon(&v, v.box_w / 2.0, 0.0);
        assert!(lat <= 85.05);
    }
}
