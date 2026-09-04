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

/// The tile grid, at a magnification that falls between pixels.
///
/// There is no tile server in the fixtures, so the seams — the hairlines that
/// showed between tiles on the phone — are drawn against a checkerboard of
/// solid tiles instead: every pixel of the map should be one of the two
/// colours, and any pixel that is neither is a join with the ground showing
/// through it. Worth knowing when reading these: the software renderer rounds
/// a destination rect itself and has never shown a seam either way, so what
/// these shots can catch is a grid placed wrongly — a tile in the wrong place,
/// a size that has gone negative — and not the phone's hairline, which comes
/// of its own renderer rounding a position and a size apart. That one is
/// settled in `mapview`'s tests, on the arithmetic.
fn map_tiles(app: &sigil_slint::AppWindow, mag: f64, dpr: f64) {
    use sigil_slint::mapview::MapView;
    let solid = |r: u8, g: u8, b: u8| {
        let mut buf = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(512, 512);
        for p in buf.make_mut_slice() {
            *p = slint::Rgb8Pixel { r, g, b };
        }
        slint::Image::from_rgb8(buf)
    };
    let (red, blue) = (solid(255, 0, 0), solid(0, 0, 255));
    let mut v = MapView::default();
    v.resize(WIDTH as f64 / dpr, (HEIGHT - 60) as f64 / dpr);
    v.open(51.5, -0.12);
    v.scale = mag;
    let rows: Vec<sigil_slint::MapTileView> = v
        .wanted()
        .into_iter()
        .map(|(tx, ty)| {
            let (x, y, w, h) = v.place(tx, ty, dpr);
            sigil_slint::MapTileView {
                x: x.into(),
                y: y.into(),
                w: w.into(),
                h: h.into(),
                img: if (tx + ty).rem_euclid(2) == 0 {
                    red.clone()
                } else {
                    blue.clone()
                },
            }
        })
        .collect();
    app.set_mp_tiles(std::rc::Rc::new(slint::VecModel::from(rows)).into());
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

/// A real animated GIF for the engine to decode: `n` flat-coloured frames at
/// 120 ms each, so one frame is told from the next at a glance in a capture.
fn write_gif(path: &std::path::Path, n: u32, w: u32, h: u32) -> anyhow::Result<()> {
    const HUES: [[u8; 3]; 6] = [
        [230, 80, 60],
        [240, 170, 50],
        [210, 215, 70],
        [90, 200, 110],
        [70, 150, 230],
        [170, 100, 220],
    ];
    let file = std::fs::File::create(path)?;
    let mut enc = image::codecs::gif::GifEncoder::new(std::io::BufWriter::new(file));
    enc.set_repeat(image::codecs::gif::Repeat::Infinite)?;
    for i in 0..n {
        let c = HUES[(i % 6) as usize];
        let buf = image::RgbaImage::from_pixel(w, h, image::Rgba([c[0], c[1], c[2], 255]));
        enc.encode_frame(image::Frame::from_parts(
            buf,
            0,
            0,
            image::Delay::from_numer_denom_ms(120, 1),
        ))?;
    }
    Ok(())
}

/// A four-second test clip for the viewer's video path. False when this
/// machine has no encoder, which is the harness's cue to skip that section.
fn make_clip(path: &std::path::Path) -> bool {
    std::process::Command::new("ffmpeg")
        .args([
            "-hide_banner", "-loglevel", "error", "-y",
            "-f", "lavfi", "-i", "testsrc=size=320x240:rate=25:duration=4",
            "-c:v", "libx264", "-pix_fmt", "yuv420p",
        ])
        .arg(path)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
        && path.exists()
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
    // The grid, at magnifications that land between pixels: a checkerboard of
    // solid tiles, so any pixel that is neither colour is a seam.
    // Three of a phone's pixels to a logical one, as the device has, so the
    // grid is placed the way it is placed there.
    point(&app, WindowEvent::ScaleFactorChanged { scale_factor: 3.0 });
    app.window().set_size(slint::PhysicalSize::new(WIDTH, HEIGHT));
    for (mag, name) in [(0.86, "086"), (1.13, "113"), (1.68, "168")] {
        h.settle();
        map_tiles(&app, mag, 3.0);
        h.frame(&format!("map-seams-{name}"))?;
    }
    point(&app, WindowEvent::ScaleFactorChanged { scale_factor: 1.0 });
    app.window().set_size(slint::PhysicalSize::new(WIDTH, HEIGHT));
    app.set_mp_tiles(std::rc::Rc::new(slint::VecModel::from(Vec::<sigil_slint::MapTileView>::new())).into());
    app.set_nav("chat".into());
    h.settle();

    // The chat-theme page arrives in the same three beats: the header's
    // title and Apply pill come in from the right (220ms), then the window —
    // the phone-shaped preview and the reset pill — rises (260 from 200),
    // and only once those have landed does the panel follow it up (220 from
    // 440). Leaving plays the three in reverse.
    // NOTE: these frames walk the page IN without walking it out first, on
    // purpose — that is the arrival a device actually performs, and it is
    // the one a `states` transition cannot play (the page is never drawn
    // while it is put away, so the machine never sees it close). The page
    // drives its beats off elapsed time instead; see chattheme.slint.
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
    // The page is ONE scroller under its header, the way the platform
    // messenger's is: a drag anywhere carries the preview and the reset pill
    // up out of sight and brings the panel — its rounded top with it — up to
    // fill the room, the gradient grid running on past the bottom edge. It is
    // NOT the panel scrolling behind a lip that stays put, which is what it
    // did before. Only a finger reaches this, and the body is a Flickable, so
    // the press is parked 140ms and the drag walked, as the swipes below are.
    // The frame is taken with the finger still down: released, the flick
    // would carry on and the shot would catch a different offset each run.
    {
        let (x, from, to) = (14.0f32, 700.0f32, 440.0f32); // 260 up, in the panel's own margin
        press(&app, x, from);
        h.advance(std::time::Duration::from_millis(140));
        h.pump();
        for i in 1..=8 {
            drag_to(&app, x, from + (to - from) * i as f32 / 8.0);
            h.advance(std::time::Duration::from_millis(16));
            h.pump();
        }
        h.frame("chattheme-scrolled")?;
        release(&app, x, to);
        h.settle();
        // …and back to the top, so every frame after this one starts there.
        press(&app, x, to);
        h.advance(std::time::Duration::from_millis(140));
        h.pump();
        for i in 1..=8 {
            drag_to(&app, x, to + (from - to) * i as f32 / 8.0);
            h.advance(std::time::Duration::from_millis(16));
            h.pump();
        }
        release(&app, x, from);
        h.settle();
    }
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
    // An animated GIF, the whole way through: a real GIF on disk, the engine
    // decoding it into a frame strip (media.gifFrames), and the bubble
    // cycling the frames on its own clock. Slint has no animated image, so
    // only stepping the harness clock can show that the picture moves.
    let gif = h.out.join("wave.gif");
    write_gif(&gif, 6, 96, 96)?;
    let seed = vec![serde_json::json!({
        "id": "g1", "kind": "image", "isOwn": false, "eventId": "$g1",
        "sender": "@marlowe:sigil.test", "senderName": "Marlowe",
        "body": "wave.gif", "ts": ts,
        "media": {"filename": "wave.gif", "mime": "image/gif", "width": 96, "height": 96,
                  "path": gif.to_string_lossy(), "thumbnailPath": gif.to_string_lossy()}})];
    sigil_slint::bridge::with_ui(|ui| {
        ui.open_room = "!marlowe".into();
        ui.shadow = seed;
        if let Some(win) = ui.win.upgrade() {
            sigil_slint::bridge::rebuild_timeline(ui, &win);
        }
    });
    // Before a single frame is back: the badge is on the still, which is what
    // it is gated on the media type for (BubbleDelegate.qml:509).
    h.frame("gif-badge-undecoded")?;
    h.wait_until("the gif strip", std::time::Duration::from_secs(30), || {
        app.get_items()
            .iter()
            .any(|i| i.event_id == "$g1" && i.gif_frames.row_count() > 1)
    })?;
    {
        let row = app
            .get_items()
            .iter()
            .find(|i| i.event_id == "$g1")
            .expect("the gif row");
        anyhow::ensure!(row.is_gif, "the row must know it is a GIF");
        anyhow::ensure!(
            row.gif_frames.row_count() == 6 && row.gif_delays.row_count() == 6,
            "six frames and six delays, got {} and {}",
            row.gif_frames.row_count(),
            row.gif_delays.row_count()
        );
        anyhow::ensure!(
            row.gif_delays.iter().all(|d| d == 120),
            "the engine kept the fixture's 120ms frame time"
        );
    }
    h.settle();
    // One capture per frame of the strip, the clock stepped by exactly the
    // frame time between them. Different bytes mean the picture moved.
    let mut moving: Vec<std::path::PathBuf> = Vec::new();
    for i in 0..4 {
        moving.push(h.frame(&format!("gif-frame-{i}"))?);
        h.advance(std::time::Duration::from_millis(120));
        h.pump();
    }
    for (i, a) in moving.iter().enumerate() {
        for b in moving.iter().skip(i + 1) {
            anyhow::ensure!(
                std::fs::read(a)? != std::fs::read(b)?,
                "{} and {} are the same picture: the GIF is not animating",
                a.display(),
                b.display()
            );
        }
    }

    // Video playback in the expanded viewer, all the way through on a machine
    // that has a decoder: the engine's ffmpeg writes RGBA into its OMV1 shared
    // surface, the frame reader (src/video.rs) maps it, and the viewer draws
    // whatever frame is newest while the scrubber follows the media clock.
    // Skipped where there is no ffmpeg — which is every phone, and why
    // Android gets the poster and an error instead (see the report).
    if sigil_engine::media::player::available() {
        let clip = h.out.join("clip.mp4");
        if make_clip(&clip) {
            let poster = h.out.join("clip.poster.png");
            sigil_engine::media::av::poster_to(&clip, (400, 400), &poster);
            let seed = vec![serde_json::json!({
                "id": "v1", "kind": "video", "isOwn": false, "eventId": "$v1",
                "sender": "@marlowe:sigil.test", "senderName": "Marlowe",
                "body": "clip.mp4", "ts": ts,
                "media": {"filename": "clip.mp4", "mime": "video/mp4",
                          "width": 320, "height": 240, "duration": 4000.0,
                          "path": clip.to_string_lossy(),
                          "thumbnailPath": poster.to_string_lossy()}})];
            sigil_slint::bridge::with_ui(|ui| {
                ui.open_room = "!marlowe".into();
                ui.shadow = seed;
                if let Some(win) = ui.win.upgrade() {
                    sigil_slint::bridge::rebuild_timeline(ui, &win);
                }
            });
            h.settle();
            h.shoot("video-bubble")?;
            app.invoke_act("viewer-open".into(), "$v1".into(), "".into());
            app.set_viewer_open(true);
            h.settle();
            h.shoot("video-poster")?;

            app.invoke_act("viewer-playback".into(), "".into(), "".into());
            h.wait_until("a decoded frame", std::time::Duration::from_secs(30), || {
                app.get_vw_frame().size().width > 0
            })?;
            h.frame("video-playing")?;
            anyhow::ensure!(
                app.get_vw_frame().size().width == 320 && app.get_vw_frame().size().height == 240,
                "the frame off the surface is the clip's own size, got {:?}",
                app.get_vw_frame().size()
            );
            h.wait_until("the media clock", std::time::Duration::from_secs(30), || {
                app.get_vw_play_duration() > 0.0 && app.get_vw_play_pos() > 0.0
            })?;
            let (pos, dur) = (app.get_vw_play_pos(), app.get_vw_play_duration());
            anyhow::ensure!(
                (3.0..5.0).contains(&dur),
                "the scrubber knows the clip is four seconds, got {dur}"
            );
            println!("video: {pos:.2}s of {dur:.2}s, frame {:?}", app.get_vw_frame().size());
            h.frame("video-scrubber")?;

            // The scrub bar: a finger down moves the thumb without the poll
            // dragging it back, and letting go seeks there.
            app.invoke_act("viewer-scrub".into(), "true".into(), "2.5".into());
            h.pump();
            anyhow::ensure!(
                (app.get_vw_play_pos() - 2.5).abs() < 0.01,
                "the thumb follows the finger, at {}",
                app.get_vw_play_pos()
            );
            h.frame("video-scrubbing")?;
            app.invoke_act("viewer-seek".into(), "2.5".into(), "".into());
            app.invoke_act("viewer-scrub".into(), "false".into(), "0".into());
            h.wait_until("the seek to take", std::time::Duration::from_secs(30), || {
                app.get_vw_play_pos() >= 2.5
            })?;
            h.frame("video-seeked")?;

            // Pause holds the clock and the last frame; closing stops it.
            app.invoke_act("viewer-playback".into(), "".into(), "".into());
            h.settle();
            let held = app.get_vw_play_pos();
            h.settle();
            anyhow::ensure!(
                (app.get_vw_play_pos() - held).abs() < 0.01,
                "a paused clip does not advance: {held} then {}",
                app.get_vw_play_pos()
            );
            anyhow::ensure!(
                app.get_vw_frame().size().width > 0,
                "the last frame stays on screen while paused"
            );
            h.frame("video-paused")?;
            app.set_viewer_open(false);
            app.invoke_act("viewer-closed".into(), "".into(), "".into());
            h.settle();
            anyhow::ensure!(
                app.get_vw_playing_event().is_empty(),
                "closing the viewer stops playback"
            );
        }
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

    // ---- the emoji pickers against the window's own bottom zones ----
    // A headless window has neither a gesture bar nor a keyboard, so stand one
    // up: the page is handed max(safe-area.bottom, kb-overlap) either way, and
    // 24 is a gesture bar's share of that while 420 is a phone keyboard's.
    // What must hold in both shots is the same three things — the back arrow
    // LEADS the search field, the category bar is clear of the bottom zone
    // with the sheet's own ground still painting on down through it, and the
    // search row, some grid and the category bar are all on screen at once.
    app.set_at_page("emoji".into());
    app.set_attach_open(true);
    h.settle();
    // The shots above left "cat" in the well: only the picker's own reset()
    // clears that and nothing out here can call it, so type it back out. The
    // well is 12 into a 400 sheet at the window's foot, and 40 tall.
    tap(&app, 150.0, HEIGHT as f32 - 400.0 + 32.0);
    for _ in 0..8 {
        let bs: slint::SharedString = slint::platform::Key::Backspace.into();
        point(&app, WindowEvent::KeyPressed { text: bs.clone() });
        point(&app, WindowEvent::KeyReleased { text: bs });
    }
    app.invoke_act("emoji-search".into(), "".into(), "".into());
    emoji_pictures(&app, &h)?;
    for (name, inset) in [
        ("attach-emoji-gesture", 24.0),
        ("attach-emoji-keyboard", 420.0),
    ] {
        app.set_kb_overlap(inset);
        h.settle();
        h.shoot(name)?;
    }
    // and typing with the keyboard up, which is the state the picker used to
    // vanish in: the results have to land in the band left over it.
    app.set_kb_overlap(420.0);
    h.settle();
    for c in ["c", "a", "t"] {
        point(&app, WindowEvent::KeyPressed { text: c.into() });
        point(&app, WindowEvent::KeyReleased { text: c.into() });
    }
    emoji_pictures(&app, &h)?;
    h.shoot("attach-emoji-keyboard-search")?;
    app.invoke_act("emoji-search".into(), "".into(), "".into());
    app.set_kb_overlap(0.0);
    app.set_at_page("grid".into());
    app.set_attach_open(false);
    h.settle();

    // The reaction drawer has a search field too, so it answers to the same
    // two zones. Opened the only way there is (the add-reaction cell at the
    // right end of the quick pill), then the zones stood up under it.
    app.invoke_debug_sheet(pressed as i32, 14.0, 546.0, 207.0, 36.0);
    h.settle();
    tap(&app, 293.0, 511.0);
    emoji_pictures(&app, &h)?;
    for (name, inset) in [
        ("sheet-emoji-gesture", 24.0),
        ("sheet-emoji-keyboard", 420.0),
    ] {
        app.set_kb_overlap(inset);
        h.settle();
        h.shoot(name)?;
    }
    app.set_kb_overlap(0.0);
    app.invoke_debug_sheet_close();
    h.settle();

    // ---- the attachment sheet's grid, and the capture flow off it ----
    // Nine tiles in two rows of five minus one — ONE camera, whose Photo or
    // Video is chosen inside the chooser below, not as a second tile — and
    // the block flush with the sheet's 16 lead-in. It sat 37 lower than that
    // for a while: the sheet's phone height had been raised to 396 while the
    // grid still wanted 264, and the block, being a layout placed by hand,
    // spread the surplus between its two rows.
    app.set_nav("chat".into());
    app.set_at_page("grid".into());
    app.set_attach_open(true);
    h.settle();
    h.shoot("attach-grid-tiles")?;
    app.set_at_page("camera".into());
    h.settle();
    h.shoot("attach-camera")?;
    app.set_at_page("grid".into());
    app.set_attach_open(false);
    h.settle();

    // ---- media staging ----
    // What every media route now lands on: the picture over a caption field,
    // and the send that carries the caption with it. One item, then three, so
    // the aspect-fit, the close disc on the corner and the strip's page dots
    // are all readable.
    app.set_room_name("Marlowe".into());
    let sizes = [(640u32, 480u32), (480u32, 640u32), (600u32, 600u32)];
    let mut staged: Vec<sigil_slint::StagedItem> = Vec::new();
    for (i, (w, ht)) in sizes.iter().enumerate() {
        let p = h.out.join(format!("staged-{i}.png"));
        write_png(&p, *w, *ht)?;
        let img = slint::Image::load_from_path(&p)
            .map_err(|_| anyhow::anyhow!("the staging fixture must load"))?;
        staged.push(sigil_slint::StagedItem {
            img,
            path: p.to_string_lossy().to_string().into(),
            name: format!("staged-{i}.png").into(),
            video: false,
            w: *w as f32,
            h: *ht as f32,
        });
    }
    app.set_sg_items(slint::ModelRc::new(slint::VecModel::from(vec![staged[0]
        .clone()])));
    app.set_sg_cur(0);
    app.set_sg_caption("".into());
    app.set_nav("staging".into());
    h.settle();
    h.shoot("staging-one")?;
    app.set_sg_caption("The lake, this morning".into());
    h.settle();
    h.shoot("staging-captioned")?;
    app.set_sg_items(slint::ModelRc::new(slint::VecModel::from(staged.clone())));
    app.set_sg_cur(0);
    h.settle();
    h.shoot("staging-strip")?;
    // The second page of the strip: a portrait shot, which the same area
    // fits by width instead of by height.
    app.set_sg_cur(1);
    h.settle();
    h.shoot("staging-strip-2")?;
    // Taking one off leaves the rest, and the page stays.
    app.invoke_act("staging-remove".into(), "1".into(), "".into());
    h.settle();
    anyhow::ensure!(
        app.get_sg_items().row_count() == 2,
        "the close disc takes one item off, not the pick"
    );
    anyhow::ensure!(
        app.get_nav() == "staging",
        "two items left is still a staging page"
    );
    // And taking the last two off is the pick abandoned.
    app.invoke_act("staging-remove".into(), "0".into(), "".into());
    app.invoke_act("staging-remove".into(), "0".into(), "".into());
    h.settle();
    anyhow::ensure!(
        app.get_sg_items().row_count() == 0 && app.get_nav() == "chat",
        "the last item off closes the page"
    );

    // The composer's live preview: a run settles the moment its `;` lands.
    // The bridge lays the typed text out twice (as typed, and as it will
    // settle) and the page slides each character between the two; stepped
    // here at three points of the collapse, then with the caret stepped back
    // inside the run, which unfolds it to its source again.
    app.set_nav("chat".into());
    h.settle();
    let typed = "hello red::world; there";
    app.invoke_chat_composer_set(typed.into(), 17, true);
    app.invoke_composer_edited(typed.into());
    h.pump();
    h.frame("composer-live-000")?;
    h.advance(std::time::Duration::from_millis(120));
    h.pump();
    h.frame("composer-live-120")?;
    h.advance(std::time::Duration::from_millis(200));
    h.pump();
    h.frame("composer-live-320")?;
    app.invoke_chat_composer_set(typed.into(), 12, true);
    app.invoke_composer_cursor_moved(12);
    h.settle();
    h.shoot("composer-live-open")?;
    app.invoke_clear_composer();
    h.settle();

    // The expanded image viewer, from a picture in the timeline: the long
    // press that must still reach the sheet, the tap that opens the viewer
    // over the frosted conversation, the filmstrip's sliver of the next
    // picture, and the ⋮ menu on top of it. Last, because it seeds a
    // timeline of its own over whatever the room was holding.
    viewer(&app, &h, ts)?;
    Ok(())
}

/// A test picture: 4:3, bright, with a diagonal and a border, so the aspect,
/// the corner rounding and any cropping are all readable at a glance.
fn write_png(path: &std::path::Path, w: u32, h: u32) -> anyhow::Result<()> {
    let mut px = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let edge = x < 8 || y < 8 || x + 8 >= w || y + 8 >= h;
            let diag = ((x as i32 * h as i32 - y as i32 * w as i32).abs() as f32
                / (w * h) as f32)
                < 0.02;
            let c = if edge {
                [250, 250, 250]
            } else if diag {
                [230, 80, 60]
            } else {
                [
                    40 + (200 * x / w) as u8,
                    60 + (150 * y / h) as u8,
                    160u8,
                ]
            };
            px[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&px)?;
    Ok(())
}

fn viewer(app: &sigil_slint::AppWindow, h: &Harness, ts: i64) -> anyhow::Result<()> {
    app.set_attach_open(false);
    app.set_nav("chat".into());
    h.settle();
    let pic = h.out.join("lake.png");
    write_png(&pic, 640, 480)?;
    // A second, portrait picture: the viewer pages over every picture in the
    // room, so this is the one the filmstrip's sliver belongs to.
    let pic2 = h.out.join("lake-tall.png");
    write_png(&pic2, 480, 720)?;
    // Text above the pictures so the frost behind the viewer has a
    // conversation to blur, and they go last so the list rests on them.
    let msg = |id: &str, body: &str, own: bool, at: i64| {
        serde_json::json!({"id": id, "kind": "text", "isOwn": own,
            "eventId": format!("${id}"),
            "sender": if own { "@wren:sigil.test" } else { "@marlowe:sigil.test" },
            "senderName": if own { "wren" } else { "Marlowe" },
            "body": body, "ts": ts - at})
    };
    let seed = vec![
        msg("v1", "so I have been thinking about the lake", false, 300_000),
        msg("v2", "it is less a body of water and more a jurisdiction", false, 240_000),
        msg("v3", "strange women lying in ponds is no basis for government", true, 180_000),
        msg("v4", "here is the lake, for the record", false, 120_000),
        serde_json::json!({"id": "v5", "kind": "image", "isOwn": false, "eventId": "$v5",
            "sender": "@marlowe:sigil.test", "senderName": "Marlowe",
            "body": "lake-tall.png", "ts": ts - 60_000,
            "media": {"filename": "lake-tall.png", "mime": "image/png", "width": 480, "height": 720,
                      "path": pic2.to_string_lossy(), "thumbnailPath": pic2.to_string_lossy()}}),
        serde_json::json!({"id": "v6", "kind": "image", "isOwn": false, "eventId": "$v6",
            "sender": "@marlowe:sigil.test", "senderName": "Marlowe",
            "body": "lake.png", "ts": ts - 30_000,
            "media": {"filename": "lake.png", "mime": "image/png", "width": 640, "height": 480,
                      "path": pic.to_string_lossy(), "thumbnailPath": pic.to_string_lossy()}}),
    ];
    sigil_slint::bridge::with_ui(|ui| {
        ui.open_room = "!marlowe".into();
        ui.shadow = seed;
        if let Some(win) = ui.win.upgrade() {
            sigil_slint::bridge::rebuild_timeline(ui, &win);
        }
    });
    h.settle();
    h.shoot("viewer-bubble")?;

    // The long press on the picture. The image body used to carry a plain
    // TouchArea, which took the pointer on the way down and left the bubble's
    // HoldArea behind it with nothing to time, so a hold on a picture opened
    // nothing. Driven the way home-selected drives its own: press, the clock
    // walked past the list's 100ms park and HoldArea's 500, then release.
    // The wide picture is the last row, its 640×480 scaled to the bubble's
    // 300 — in this fixture its box lands at 14,517 300×225.
    let (pic_x, pic_y) = (150.0, 600.0);
    app.set_sheet_actions(slint::ModelRc::new(slint::VecModel::from(
        Vec::<sigil_slint::MenuEntry>::new(),
    )));
    press(app, pic_x, pic_y);
    for _ in 0..4 {
        h.advance(std::time::Duration::from_millis(200));
        h.pump();
    }
    release(app, pic_x, pic_y);
    h.shoot("viewer-image-sheet")?;
    anyhow::ensure!(
        app.get_sheet_actions().row_count() > 0,
        "a long press on a picture must open the message sheet"
    );
    anyhow::ensure!(
        !app.get_viewer_open(),
        "a long press is not a tap: the viewer must stay shut"
    );
    app.invoke_debug_sheet_close();
    h.settle();

    // And the tap, which is the same surface: no hold, so the viewer opens.
    tap(app, pic_x, pic_y);
    h.settle();
    anyhow::ensure!(
        app.get_viewer_open(),
        "a tap on a picture opens the viewer"
    );
    anyhow::ensure!(
        app.global::<sigil_slint::Theme>().get_viewer_frost().size().width > 0,
        "the viewer lies on a frosted picture of the page it came from"
    );
    anyhow::ensure!(
        app.get_vw_items().row_count() == 2 && app.get_vw_cur() == 1,
        "the viewer holds every picture in the room and opened on the tapped one"
    );
    // The filmstrip: a page is 56 narrower than the viewer and the pitch adds
    // 12, so 16px of the picture before this one stands inside the left edge
    // at rest. Sampled a third of the way down, clear of the top bar.
    h.shoot("viewer-open")?;
    // the ⋮ menu: the third 22px button in from the right edge, at 8 out and
    // 2 apart, so its centre is 8 + 11 = 19 from the right, 28 down.
    tap(app, WIDTH as f32 - 19.0, 28.0);
    h.shoot("viewer-menu")?;
    // Forward: the card is 160 wide 10 in from the right at y 42, its first
    // row 6 inside that and 34 tall, so the row's middle is at y 65.
    tap(app, WIDTH as f32 - 100.0, 65.0);
    h.shoot("viewer-forward")?;
    app.set_viewer_open(false);
    app.invoke_act("viewer-closed".into(), "".into(), "".into());
    h.settle();
    // The keyboard and a panel never share the bottom of the page: the
    // composer taking focus (the keyboard rising for it) closes whichever
    // panel is up, and opening a panel lets the composer's focus go.
    app.set_nav("chat".into());
    h.settle();
    app.set_attach_open(true);
    h.settle();
    app.invoke_chat_composer_set("".into(), 0, true);
    h.settle();
    anyhow::ensure!(!app.get_attach_open(), "the composer took focus but the attach sheet stayed open");
    app.set_recorder_open(true);
    h.settle();
    app.invoke_chat_composer_set("".into(), 0, true);
    h.settle();
    anyhow::ensure!(!app.get_recorder_open(), "the composer took focus but the recorder stayed open");
    h.settle();
    Ok(())
}
