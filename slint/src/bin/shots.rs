//! The screenshot harness: every page rendered headless with the demo
//! fixtures, one PNG each, so a phase can be checked against the QML side by
//! side without a display.
//!
//!   cargo run --bin shots -- [out-dir]        (default: shots/)

use sigil_slint::headless::{Harness, HEIGHT, WIDTH};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model};

/// Pointer events at window coordinates, for the parts of a page that only a
/// finger can reach (nothing on AppWindow opens the reaction drawer, and a
/// drag has to be driven to be checked at all).
fn point(app: &sigil_slint::AppWindow, ev: WindowEvent) {
    app.window().dispatch_event(ev);
}
fn press(app: &sigil_slint::AppWindow, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
    point(app, WindowEvent::PointerMoved { position });
    point(
        app,
        WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        },
    );
}
fn drag_to(app: &sigil_slint::AppWindow, x: f32, y: f32) {
    point(
        app,
        WindowEvent::PointerMoved {
            position: LogicalPosition::new(x, y),
        },
    );
}
fn release(app: &sigil_slint::AppWindow, x: f32, y: f32) {
    point(
        app,
        WindowEvent::PointerReleased {
            position: LogicalPosition::new(x, y),
            button: PointerEventButton::Left,
        },
    );
}
fn tap(app: &sigil_slint::AppWindow, x: f32, y: f32) {
    press(app, x, y);
    release(app, x, y);
}

/// The picker's glyph pictures are rendered one request at a time and land in
/// bursts, so a shot of it has to wait for them or catch an empty grid.
fn emoji_pictures(app: &sigil_slint::AppWindow, h: &Harness) -> anyhow::Result<()> {
    h.wait_until("emoji pictures", std::time::Duration::from_secs(30), || {
        let rows = app.get_emoji_rows();
        let drawn = (0..rows.row_count())
            .filter(|&i| {
                rows.row_data(i)
                    .and_then(|r| r.row_data(0))
                    .map(|e| e.has_img)
                    .unwrap_or(false)
            })
            .count();
        // The section captions never carry one, so three in four is every
        // glyph row — and a short search result has to pass the same bar.
        rows.row_count() > 0 && drawn * 4 >= rows.row_count() * 3
    })
}

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
    // the reaction drawer, reached the only way there is: the add-reaction
    // cell at the right end of the quick pill (chat-sheet.png: 293, 511)
    app.invoke_act("emoji-search".into(), "".into(), "".into());
    tap(&app, 293.0, 511.0);
    emoji_pictures(&app, &h)?;
    h.shoot("sheet-emoji")?;
    // the drag handle. It sits 8 + 12 into a drawer that rests at
    // 820 − 400, so the pill's middle is at y 440. Pulled 96 down — inside
    // the quarter-height threshold — the drawer follows the finger …
    press(&app, 200.0, 440.0);
    drag_to(&app, 200.0, 496.0);
    drag_to(&app, 200.0, 536.0);
    h.pump();
    h.frame("sheet-emoji-drag")?;
    // … and springs back when it is let go.
    release(&app, 200.0, 536.0);
    h.shoot("sheet-emoji-settled")?;
    // Past the threshold it goes instead, and the sheet's menu is back.
    press(&app, 200.0, 440.0);
    drag_to(&app, 200.0, 560.0);
    drag_to(&app, 200.0, 660.0);
    release(&app, 200.0, 660.0);
    h.shoot("sheet-emoji-dismissed")?;
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
    // the emoji page: the picker's own search row (with the sheet's back
    // button in it), the sectioned grid and the category bar
    app.invoke_act("emoji-search".into(), "".into(), "".into());
    app.set_at_page("emoji".into());
    emoji_pictures(&app, &h)?;
    h.shoot("attach-emoji")?;
    // the food category, jumped to from the bar: the bar is 10 cells over a
    // 400 window, so cell 4 is centred at 180, 24 up from the bottom edge.
    // Its heading has to land at the top of the grid, which is the whole
    // point of counting the section captions out of the row index.
    tap(&app, 180.0, 796.0);
    h.shoot("attach-emoji-category")?;
    // the last section, cell 9, which is where the drawable filter's block
    // walk would show up if it had miscounted: flags are nearly all
    // regional-indicator pairs and only the five single glyphs survive it
    tap(&app, 380.0, 796.0);
    h.shoot("attach-emoji-last")?;
    // and typing in the search well: results only, no section captions
    tap(&app, 150.0, 452.0);
    for c in ["c", "a", "t"] {
        point(
            &app,
            WindowEvent::KeyPressed {
                text: c.into(),
            },
        );
        point(
            &app,
            WindowEvent::KeyReleased {
                text: c.into(),
            },
        );
    }
    emoji_pictures(&app, &h)?;
    h.shoot("attach-emoji-search")?;
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
    // Two fingers, end to end. Recognising the gesture is the runtime's job
    // (and the headless platform has no way to put two fingers down), but
    // answering it is ours: this drives the bridge the way the handler does —
    // a spread, then a lift — which is also the only way to catch the settle
    // reaching for the page state while the lift still holds it.
    app.invoke_map_pinch_begin();
    for f in [1.15f32, 1.5, 2.1] {
        app.invoke_map_pinched(f, 200.0, 320.0);
    }
    app.invoke_map_pinch_end();
    h.settle();
    // The page's own choreography, beat by beat. It has to be walked out
    // before it can be walked in: `settle` never draws, so an animation it
    // jumps over is never sampled, and the page would arrive already at rest.
    // Leaving is the three beats in reverse — the footer drops, then the map,
    // and last the header hands itself back to the conversation.
    // One frame of a choreography, `at` ms in from where it started.
    let step = |h: &Harness, was: &mut u64, at: u64, name: &str| {
        h.advance(std::time::Duration::from_millis(at - *was));
        *was = at;
        h.pump();
        h.frame(name)
    };
    app.set_nav("chat".into());
    let mut was = 0u64;
    for (name, at) in [
        ("map-back-000", 0u64), // at rest, about to go
        ("map-back-120", 120),  // the footer on its way down
        ("map-back-260", 260),  // the map following it
        ("map-back-420", 420),  // the conversation's bar coming back
    ] {
        step(&h, &mut was, at, name)?;
    }
    h.settle();
    // …and the arrival: the conversation's header lets go and "Location"
    // comes in from the right (head), then the map rises (map), and only once
    // those have landed does the footer follow it up (foot). 220ms of header,
    // 260 of map from 200, 220 of footer from 440.
    app.set_nav("map".into());
    let mut was = 0u64;
    for (name, at) in [
        ("map-open-000", 0u64), // the conversation's bar, whole
        ("map-open-120", 120),  // its furniture gone, the bar bare
        ("map-open-210", 210),  // "Location" arriving from the right
        ("map-open-330", 330),  // the map on its way up
        ("map-open-460", 460),  // map landed, footer still below
        ("map-open-560", 560),  // the footer following it up
        ("map-open-700", 700),  // everything at rest
    ] {
        step(&h, &mut was, at, name)?;
    }
    // The other two location categories, so each one's own icon can be seen:
    // a plain fix is Current Location, someone else's point a dropped pin.
    app.set_mp_live(false);
    app.set_mp_status("Shared 2:41 PM".into());
    h.shoot("map-current")?;
    app.set_mp_self(false);
    app.set_mp_own(false);
    h.shoot("map-pin")?;
    app.set_mp_self(true);
    app.set_mp_own(true);
    app.set_mp_live(true);
    app.set_mp_status("Sharing until 3:00 PM".into());
    app.set_nav("chat".into());
    h.settle();

    // The chat-theme page arrives in the same three beats: the header's
    // title and Apply pill come in from the right (220ms), then the window —
    // the phone-shaped preview and the reset pill — rises (260 from 200),
    // and only once those have landed does the panel follow it up (220 from
    // 440). Leaving plays the three in reverse.
    // NOTE: the page plays this off its `opened` input, which the holder in
    // app.slint has yet to bind (`opened: chattheme-holder.active`, with
    // `slides: Theme.mode == "desktop"` and `hold: 660ms` on the holder).
    // Until it does, `opened` sits at its default of true and these six
    // frames are all the page at rest.
    app.set_nav("chattheme".into());
    let mut was = 0u64;
    for (name, at) in [
        ("chattheme-open-000", 0u64), // the bar bare, the page still empty
        ("chattheme-open-120", 120),  // the title on its way in
        ("chattheme-open-260", 260),  // header landed, the window rising
        ("chattheme-open-460", 460),  // window landed, the panel still below
        ("chattheme-open-560", 560),  // the panel following it up
        ("chattheme-open-700", 700),  // everything at rest
    ] {
        step(&h, &mut was, at, name)?;
    }
    h.settle();
    // The page wearing a theme, so the selected swatch's squircle and ring
    // can be read against the tones the whole page has taken.
    app.set_ct_accent("#b48ad6".into());
    app.set_ct_color(slint::Color::from_rgb_u8(0xb4, 0x8a, 0xd6));
    h.shoot("chattheme-themed")?;
    app.set_ct_accent(Default::default());
    app.set_ct_color(app.global::<sigil_slint::Theme>().get_accent());
    h.settle();
    // …and the way out: the panel drops, then the window, then the header.
    app.set_nav("chat".into());
    let mut was = 0u64;
    for (name, at) in [
        ("chattheme-back-000", 0u64), // at rest, about to go
        ("chattheme-back-120", 120),  // the panel on its way down
        ("chattheme-back-260", 260),  // the window following it
        ("chattheme-back-420", 420),  // the header last
    ] {
        step(&h, &mut was, at, name)?;
    }
    h.settle();

    // ---- threads: the swipe that starts one, and the two pages that open ----
    //
    // A row dragged away from its own edge replies; dragged towards it, the
    // reply goes in a thread. Only a finger reaches this, and the timeline is
    // a Flickable that parks a press for 100ms before the row sees it, so the
    // clock is walked rather than jumped. "glitch sparkle secret" is an
    // incoming row here, its bubble at 14,500 183×35, so it hugs the left:
    // right is away from its own edge, left is towards it.
    const SWIPE_Y: f32 = 517.0;
    // The finger drifts 14px down on its way across, as a real one does — far
    // past the 8px the timeline's Flickable takes a vertical drag at. Without
    // the SIGIL PATCH in flickable.rs the list would claim the gesture here
    // and the row would spring home half way, so this drift is the test.
    let swipe = |from: f32, to: f32| {
        press(&app, from, SWIPE_Y);
        h.advance(std::time::Duration::from_millis(140));
        h.pump();
        let steps = 6;
        for i in 1..=steps {
            let f = i as f32 / steps as f32;
            drag_to(&app, from + (to - from) * f, SWIPE_Y + 14.0 * f);
            h.advance(std::time::Duration::from_millis(16));
            h.pump();
        }
    };
    // away from the left edge, past the 64px detent: the reply disc stands in
    // the gap the row opened, 20px clear of the bubble and lit for the commit
    swipe(100.0, 180.0);
    h.frame("chat-swipe-reply")?;
    // released short of the detent it springs home and nothing happens
    drag_to(&app, 130.0, SWIPE_Y + 14.0);
    release(&app, 130.0, SWIPE_Y + 14.0);
    h.shoot("chat-swipe-home")?;
    // and towards its own edge: the same row, the thread glyph, the disc
    // riding in from the other side
    swipe(180.0, 100.0);
    h.frame("chat-swipe-thread")?;
    drag_to(&app, 165.0, SWIPE_Y + 14.0);
    release(&app, 165.0, SWIPE_Y + 14.0);
    h.settle();
    // and the reply committed: letting go past the detent arms the composer's
    // quote down the message sheet's own path (arm-reply), not a second one
    swipe(100.0, 180.0);
    release(&app, 180.0, SWIPE_Y + 14.0);
    h.shoot("chat-swipe-replied")?;
    app.invoke_clear_composer();
    h.settle();

    // The browser arrives out of the conversation in the conversation's own
    // three beats: the header's furniture lets go and "Threads" comes in from
    // the right (190ms of header from 250 on the way back, 220 out), then the
    // ground rises (260 from 200), and last the rows follow it up (220 from
    // 440) — the beat the conversation spends on its composer.
    app.set_nav("threads".into());
    let mut was = 0u64;
    for (name, at) in [
        ("threads-open-000", 0u64), // the conversation's bar, whole
        ("threads-open-120", 120),  // its furniture gone, the bar bare
        ("threads-open-210", 210),  // "Threads" arriving from the right
        ("threads-open-330", 330),  // the ground on its way up
        ("threads-open-460", 460),  // ground landed, the rows still below
        ("threads-open-560", 560),  // the rows following it up
        ("threads-open-700", 700),  // everything at rest
    ] {
        step(&h, &mut was, at, name)?;
    }
    h.settle();
    // A thread picked out of the browser is the conversation page again, and
    // it rises over the browser in the conversation's own beats: the header
    // morph and the sheet (280ms), then the composer after them (160 from 280).
    app.set_chat_is_thread(true);
    app.set_nav("thread".into());
    let mut was = 0u64;
    for (name, at) in [
        ("thread-open-000", 0u64), // the browser, about to be covered
        ("thread-open-100", 100),  // the bar morphing, the sheet rising
        ("thread-open-200", 200),  // the sheet most of the way up
        ("thread-open-300", 300),  // landed; the composer still below it
        ("thread-open-380", 380),  // the composer following it up
        ("thread-open-500", 500),  // everything at rest
    ] {
        step(&h, &mut was, at, name)?;
    }
    h.settle();
    h.shoot("thread")?;
    // …and back out: the thread's three beats in reverse hand the page to the
    // browser, which is still standing where it was left.
    app.set_nav("threads".into());
    let mut was = 0u64;
    for (name, at) in [
        ("thread-back-000", 0u64), // at rest, about to go
        ("thread-back-080", 80),   // the composer on its way down
        ("thread-back-200", 200),  // the sheet following it
        ("thread-back-320", 320),  // the browser alone again
    ] {
        step(&h, &mut was, at, name)?;
    }
    h.settle();
    app.set_chat_is_thread(false);
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
        // The location page in a themed room: the header band is the very
        // tone the conversation's header settled on, and the footer sheet the
        // chrome surface its own sheets sit on.
        app.set_nav("map".into());
        h.shoot("map-themed")?;
        app.set_nav("chat".into());
        h.settle();
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

    // Home's long press: the list's selection bar and what it opens. Only a
    // finger reaches this, so the hold is driven the way it happens — press,
    // the clock past HoldArea's 500ms, release. The third row ("Ideas") is
    // the pinned one, so the bar shows its pin already lit.
    app.set_nav("home".into());
    h.settle();
    let row_y = 263.0;
    press(&app, 200.0, row_y);
    // The list is a Flickable, which parks a press for 100ms before the row
    // sees it (UPSTREAM-flickable-longpress.md), so the hold's own 500ms only
    // starts after that: walk the clock rather than jumping it.
    for _ in 0..4 {
        h.advance(std::time::Duration::from_millis(200));
        h.pump();
    }
    release(&app, 200.0, row_y);
    h.shoot("home-selected")?;
    // the bar's second action, whose 48 box ends 96 from the right edge
    let bar_action = |k: f32| WIDTH as f32 - k * 48.0 + 24.0;
    tap(&app, bar_action(3.0), 30.0);
    h.shoot("home-snooze")?;
    tap(&app, 200.0, 40.0); // the scrim, to put it away
    h.settle();
    // and the question Leave chat asks first
    tap(&app, bar_action(1.0), 30.0);
    h.shoot("home-leave")?;

    // Polls. The timeline is set to poll rows alone — open, undisclosed and
    // finished — so the card is measured against the bubble it lives in and
    // the finished one can be told from the live one at a glance.
    // They are seeded into the room's own shadow rather than pushed at the
    // window, so the bridge builds them the way it builds an arriving poll
    // (the winner, the sums and the day stamp included) — and a tap then
    // travels the real path.
    app.set_nav("chat".into());
    h.settle();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let poll = |id: &str, question: &str, ended: bool, disclosed: bool, voters: i64, answers: serde_json::Value| {
        serde_json::json!({"id": id, "kind": "poll", "isOwn": false, "eventId": format!("${id}"),
            "sender": "@marlowe:sigil.test", "senderName": "Marlowe", "body": question, "ts": ts,
            "poll": {"question": question, "ended": ended, "disclosed": disclosed,
                     "voters": voters, "maxSelections": 1, "answers": answers}})
    };
    let seed = vec![
        poll("p1", "Which toolkit?", false, true, 3, serde_json::json!([
            {"id": "a1", "text": "Slint", "votes": 2, "mine": true},
            {"id": "a2", "text": "QML", "votes": 1, "mine": false},
            {"id": "a3", "text": "Both", "votes": 0, "mine": false}])),
        poll("p2", "Where do we meet?", false, false, 2, serde_json::json!([
            {"id": "b1", "text": "The lake", "votes": 0, "mine": true},
            {"id": "b2", "text": "The keep", "votes": 0, "mine": false}])),
        poll("p3", "Which toolkit?", true, true, 8, serde_json::json!([
            {"id": "c1", "text": "Slint", "votes": 5, "mine": true},
            {"id": "c2", "text": "QML", "votes": 2, "mine": false},
            {"id": "c3", "text": "Both", "votes": 1, "mine": false}])),
    ];
    sigil_slint::bridge::with_ui(|ui| {
        ui.open_room = "!marlowe".into();
        ui.shadow = seed;
        if let Some(win) = ui.win.upgrade() {
            sigil_slint::bridge::rebuild_timeline(ui, &win);
        }
    });
    h.shoot("poll-bubbles")?;
    // and the tap's own answer: no engine stands behind this one, so what the
    // card shows is purely the local echo (actions.rs poll_echo) — the pick
    // and the tally move under the finger instead of after the round trip.
    // The frame is taken WITHOUT pumping, which is the whole point: nothing
    // has been posted back from the engine yet (and in this harness the
    // engine will refuse, which puts the old answer back a pump later).
    app.invoke_act("vote".into(), "$p1".into(), "a2".into());
    h.frame("poll-voted")?;
    {
        let items = app.get_items();
        let echoed = (0..items.row_count())
            .filter_map(|i| items.row_data(i))
            .find(|r| r.event_id == "$p1")
            .and_then(|r| r.poll_options.row_data(1))
            .map(|o| o.mine && o.votes == 2)
            .unwrap_or(false);
        anyhow::ensure!(echoed, "the vote was not echoed into the poll row");
    }
    // the composer: empty (two option rows), then with the last one typed
    // into, which grows a third — caught mid-flight and again at rest.
    // Whatever room the shots before this one left open, the sheet only has
    // room in a joined one (ChatPage gives an invite's frame no height).
    app.set_chat_is_invite(false);
    app.set_attach_open(true);
    app.set_at_page("poll".into());
    h.settle();
    h.shoot("attach-poll")?;
    // The taps below are the phone page's own geometry (the desktop sheet is
    // a different height), so the typed shots belong to a themed run.
    if std::env::var_os("SIGIL_THEME_MODE").is_some() {
        let poll_type = |app: &sigil_slint::AppWindow, s: &str| {
            for c in s.chars() {
                let text: slint::SharedString = c.to_string().into();
                point(
                    app,
                    WindowEvent::KeyPressed {
                        text: text.clone(),
                    },
                );
                point(app, WindowEvent::KeyReleased { text });
            }
        };
        // The wells, at the page's own measurements: the sheet is 520 tall at
        // the window's foot, its body starts 52 in, and from there the
        // question well is 21 lower (its label over a 6 gap) and 48 tall, the
        // option rows 54 apart from 167.
        let sheet_y = HEIGHT as f32 - 520.0;
        tap(&app, 200.0, sheet_y + 97.0);
        poll_type(&app, "Lunch?");
        tap(&app, 150.0, sheet_y + 190.0);
        poll_type(&app, "Soup");
        h.settle();
        tap(&app, 150.0, sheet_y + 244.0);
        poll_type(&app, "Bread");
        // the third row growing in: 30ms into its 220, then settled
        h.advance(std::time::Duration::from_millis(30));
        h.pump();
        h.frame("attach-poll-growing")?;
        h.shoot("attach-poll-filled")?;
    }
    Ok(())
}
