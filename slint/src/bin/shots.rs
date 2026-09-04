//! The screenshot harness: every page rendered headless with the demo
//! fixtures, one PNG each, so a phase can be checked against the QML side by
//! side without a display.
//!
//!   cargo run --bin shots -- [out-dir]        (default: shots/)

use sigil_slint::headless::{Harness, HEIGHT, WIDTH};
use slint::{ComponentHandle, Model};

fn main() -> anyhow::Result<()> {
    let out = std::env::args().nth(1).unwrap_or_else(|| "shots".into());
    let h = Harness::install(out)?;

    // the demo fixtures instead of a live engine, with the first room open
    std::env::set_var("SIGIL_SLINT_DEMO", "1");
    std::env::set_var("SIGIL_SLINT_DEMO_CHAT", "1");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let app = sigil_slint::AppWindow::new()?;
    // preview the phone palettes: SIGIL_THEME_MODE=dark|light
    if let Ok(m) = std::env::var("SIGIL_THEME_MODE") {
        app.global::<sigil_slint::Theme>().set_mode(m.as_str().into());
        sigil_slint::rows::DARK_SCHEME.store(m != "light", std::sync::atomic::Ordering::Relaxed);
        app.global::<sigil_slint::Theme>().set_system_accent(slint::Color::from_rgb_u8(0xE8, 0x91, 0x4E));
    }
    app.window()
        .set_size(slint::PhysicalSize::new(WIDTH, HEIGHT));
    let icons = sigil_slint::rows::IconSet::from_window(&app);
    sigil_slint::bridge::start(&app, &rt, icons);
    app.show()?;
    h.settle();

    // the doors, as a fresh install walks through them
    app.set_session("loggedOut".into());
    app.set_door("server".into());
    h.shoot("door-server")?;
    app.set_door_server("sigil.example.com".into());
    app.set_door_registration("invite".into());
    app.set_door("choose".into());
    h.shoot("door-choose")?;
    app.set_door("create".into());
    h.shoot("door-create")?;
    app.set_door("recover".into());
    h.shoot("door-recover")?;
    app.set_door("link".into());
    h.shoot("door-link")?;
    let offer = "sigil-link:1:8b1f0c3a9e4d7f2b6a5c1e0d9f8b7a6c5d4e3f2a1b0c9d8e7f6a5b4c3d2e1f0";
    if let Some(img) = sigil_slint::qr::image(offer) {
        app.set_link_image(img);
    }
    app.set_link_offer(offer.into());
    app.set_link_state("offer".into());
    h.shoot("door-link-offer")?;
    app.set_link_sas("🦁 🐢 🍕 🎸 🔑 🌙 ⚓".into());
    app.set_link_state("sas".into());
    h.shoot("door-link-sas")?;
    app.set_link_state(Default::default());
    app.set_session("loggedIn".into());

    // recovery: the code once, and the password page
    app.set_recovery_code("kq7m-3xvp-9tc2-hf8r-5wbn-2jd6-8lzs-4qy9-7gxe-1vbt-6nma".into());
    app.set_recovery_first_time(true);
    app.set_recovery_open(true);
    h.shoot("recovery-code")?;
    app.set_recovery_open(false);
    app.set_password_open(true);
    h.shoot("backup-password")?;
    app.set_password_open(false);

    // the open conversation and the pages that hang off it
    app.set_nav("chat".into());
    // the location card as an OWN message wearing the sent tick, so the
    // receipt mark's right-edge alignment shows under a full-bleed card
    // (BubbleDelegate.qml:1255-1256: the mark line hangs off the bubble's
    // right edge, rightMargin space(2))
    {
        let items = app.get_items();
        for i in 0..items.row_count() {
            if let Some(mut row) = items.row_data(i) {
                if row.kind == "location" {
                    row.is_own = true;
                    row.owns_receipt = true;
                    row.send_state = "sent".into();
                    items.set_row_data(i, row);
                    break;
                }
            }
        }
    }
    h.settle();
    h.shoot("chat")?;
    // the voice recorder: idle, mid-take, then with a clip ready to send
    app.set_recorder_open(true);
    h.settle();
    h.shoot("recorder-idle")?;
    app.set_rec_state("recording".into());
    app.set_rec_elapsed(4.0);
    let live: Vec<f32> = (0..60)
        .map(|i| 0.15 + 0.6 * ((i as f32 * 0.7).cos().abs()))
        .collect();
    app.set_rec_levels(slint::ModelRc::new(slint::VecModel::from(live)));
    h.settle();
    h.shoot("recorder-recording")?;
    app.set_rec_state("ready".into());
    app.set_rec_clip_duration(7.0);
    let bars: Vec<f32> = (0..60)
        .map(|i| 0.25 + 0.7 * ((i as f32 * 0.9).sin().abs()))
        .collect();
    app.set_rec_clip_waveform(slint::ModelRc::new(slint::VecModel::from(bars)));
    h.settle();
    h.shoot("recorder")?;
    app.set_recorder_open(false);
    app.set_rec_state("idle".into());
    h.settle();
    // the long-press sheet over a real bubble ("solid red …", whose box sits
    // at 14,546 207×36 in this fixture), the page frosted behind it with the
    // pressed row holding no second bubble under the lifted copy
    let pressed = {
        let items = app.get_items();
        (0..items.row_count())
            .find(|&i| {
                items
                    .row_data(i)
                    .map(|r| r.body.starts_with("solid red"))
                    .unwrap_or(false)
            })
            .unwrap_or(items.row_count() - 3)
    };
    app.invoke_debug_sheet(pressed as i32, 14.0, 546.0, 207.0, 36.0);
    h.shoot("chat-sheet")?;
    // the no-picture path (a renderer that cannot snapshot) dims the live
    // page instead of frosting it: the timeline shows through, so this frame
    // proves the pressed row holds no second bubble behind the lifted copy
    app.set_sheet_backdrop(Default::default());
    h.shoot("chat-sheet-dim")?;
    app.invoke_debug_sheet_close();
    h.settle();
    // the same message pressed where the timeline was cutting it off: a rect
    // that starts above the convo's top edge, under the header. The copy is
    // drawn from the row rather than cropped out of the window, so it must
    // still lift whole, with no header in it and nothing missing.
    app.invoke_debug_sheet(pressed as i32, 14.0, 30.0, 207.0, 36.0);
    h.shoot("chat-sheet-clipped")?;
    app.invoke_debug_sheet_close();
    h.settle();
    app.invoke_debug_sheet(pressed as i32, 14.0, 546.0, 207.0, 36.0);
    h.settle();
    // after the close settles the original bubble must be back in the
    // timeline (the sheet hid it for its lifetime — MessageSheet.qml:205)
    app.invoke_debug_sheet_close();
    h.shoot("chat-sheet-closed")?;
    h.settle();
    for page in ["search", "forward", "chattheme", "roomsettings"] {
        app.set_nav(page.into());
        h.shoot(page)?;
    }
    // the attach sheet's location pages: a dropped pin, then a live share
    // with its duration field (no engine, so the no-imagery ground stands in)
    app.set_nav("chat".into());
    h.settle();
    app.set_attach_open(true);
    h.settle();
    h.shoot("attach-grid")?;
    app.set_lp_have_fix(true);
    app.set_lp_lat(51.5007);
    app.set_lp_lon(-0.1246);
    app.set_lp_marked(true);
    app.set_lp_mark_lat(51.5007);
    app.set_lp_mark_lon(-0.1246);
    app.set_at_page("pin".into());
    h.settle();
    h.shoot("attach-pin")?;
    app.set_at_page("live".into());
    h.settle();
    h.shoot("attach-live")?;
    app.set_at_page("grid".into());
    app.set_attach_open(false);
    h.settle();
    // the map page for a live share
    app.set_mp_who("Marlowe".into());
    app.set_mp_live(true);
    app.set_mp_status("Sharing until 3:00 PM".into());
    app.set_mp_remaining("42:10".into());
    app.set_mp_lat(51.5007);
    app.set_mp_lon(-0.1246);
    app.set_mp_self(true);
    app.set_mp_initials("M".into());
    app.set_nav("map".into());
    h.shoot("map")?;
    app.set_nav("chat".into());
    h.settle();

    // the phone's attach sheet and recorder in a themed room: the room accent
    // staged on the window, as the bridge does when a chat theme is set
    if std::env::var_os("SIGIL_THEME_MODE").is_some() {
        app.set_ct_accent("#E8914E".into());
        app.set_ct_color(slint::Color::from_rgb_u8(0xE8, 0x91, 0x4E));
        h.settle();
        app.set_attach_open(true);
        h.settle();
        h.shoot("attach-grid-themed")?;
        app.set_at_page("pin".into());
        h.settle();
        h.shoot("attach-pin-themed")?;
        app.set_at_page("grid".into());
        app.set_attach_open(false);
        h.settle();
        app.set_recorder_open(true);
        h.settle();
        h.shoot("recorder-idle-themed")?;
        app.set_rec_state("recording".into());
        app.set_rec_elapsed(4.0);
        h.settle();
        h.shoot("recorder-recording-themed")?;
        app.set_rec_state("idle".into());
        app.set_recorder_open(false);
        app.set_ct_accent(Default::default());
        app.set_ct_color(app.global::<sigil_slint::Theme>().get_accent());
        h.settle();
    }

    // home and what starts from it
    app.invoke_back_to_home();
    app.set_nav("home".into());
    h.shoot("home")?;
    app.set_nav("start".into());
    h.shoot("start")?;
    app.set_recovery_state("enabled".into());
    app.set_backup_state("enabled".into());
    app.set_shape_clocked(0);
    app.set_app_version("0.1.0".into());
    app.set_nav("settings".into());
    h.shoot("settings")?;
    app.set_nav("home".into());
    h.settle();
    app.invoke_set_home_tab(1);
    h.shoot("requests")?;

    // the phone's search from home: the chip grid, a chip's rooms, a query's
    // message hits (no engine here, so the rows are set by hand)
    if std::env::var_os("SIGIL_THEME_MODE").is_some() {
        app.set_se_global(true);
        app.set_nav("search".into());
        h.settle();
        h.shoot("search-home")?;
        let all = app.get_all_rooms();
        let rows: Vec<_> = (0..all.row_count()).filter_map(|i| all.row_data(i)).collect();
        let unread: Vec<_> = rows.iter().filter(|r| r.unread && !r.is_invite).cloned().collect();
        app.set_se_kind("unread".into());
        app.set_se_rooms(slint::ModelRc::new(slint::VecModel::from(unread)));
        h.settle();
        h.shoot("search-unread")?;
        let marlowe = rows.iter().find(|r| r.name == "Marlowe").cloned().unwrap_or_default();
        let hit = |ev: &str, body: &str, stamp: &str| sigil_slint::SearchHit {
            room_id: "!marlowe".into(),
            event_id: ev.into(),
            room_name: marlowe.name.clone(),
            initials: marlowe.initials.clone(),
            avatar: marlowe.avatar.clone(),
            tint: marlowe.tint,
            body: body.into(),
            icon: Default::default(),
            stamp: stamp.into(),
        };
        app.set_se_kind(Default::default());
        app.set_se_rooms(slint::ModelRc::new(slint::VecModel::from(Vec::new())));
        app.set_se_hits(slint::ModelRc::new(slint::VecModel::from(vec![
            hit("$m5", "there is always a condition", "14:03"),
            hit("$m4", "the sword is yours, but there is a condition", "14:02"),
            hit("$f7", "the sword is yours, but there is a condition", "13:58"),
        ])));
        app.invoke_debug_search_query("condition".into());
        h.settle();
        h.shoot("search-query")?;
    }
    Ok(())
}
