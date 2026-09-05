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
fn solid_tile(r: u8, g: u8, b: u8) -> slint::Image {
    let mut buf = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(512, 512);
    for p in buf.make_mut_slice() {
        *p = slint::Rgb8Pixel { r, g, b };
    }
    slint::Image::from_rgb8(buf)
}

/// A tile in four flat quarters, so a crop of it says which quarter it took:
/// white top-left, yellow top-right, green bottom-left, magenta bottom-right.
/// This is the parent in the gap shot — a child standing on it comes out one
/// flat colour, and WHICH colour is the whole assertion.
fn quartered_tile() -> slint::Image {
    let mut buf = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(512, 512);
    let w = buf.width() as usize;
    for (i, p) in buf.make_mut_slice().iter_mut().enumerate() {
        let (x, y) = (i % w, i / w);
        *p = match (x >= 256, y >= 256) {
            (false, false) => slint::Rgb8Pixel { r: 255, g: 255, b: 255 },
            (true, false) => slint::Rgb8Pixel { r: 255, g: 220, b: 0 },
            (false, true) => slint::Rgb8Pixel { r: 0, g: 190, b: 90 },
            (true, true) => slint::Rgb8Pixel { r: 230, g: 0, b: 200 },
        };
    }
    slint::Image::from_rgb8(buf)
}

/// Place the grid the way the app does — through `MapView::layout`, so the
/// shot exercises the real placement and the real substitution.
///
/// Without `holes` this is the seam fixture from before: a checkerboard of
/// solid tiles, so any pixel that is neither colour is a join with the ground
/// showing through.
///
/// With it, one whole 2×2 quad — the four children of a single parent — is
/// taken out of the level being drawn and that parent is put in hand instead.
/// Every one of the four must then come back as ONE flat colour, and the four
/// of them together must reproduce `quartered_tile`: white, yellow, green and
/// magenta, in that arrangement, at four times the size. That is the fallback
/// and the crop arithmetic both, drawn where the eye can check them — a hole
/// left blank would be the page's ground, and a crop gone astray would put
/// the colours in the wrong corners or show four of them inside one tile.
fn map_tiles(app: &sigil_slint::AppWindow, mag: f64, dpr: f64, holes: bool) {
    use sigil_slint::mapview::MapView;
    let (red, blue) = (solid_tile(255, 0, 0), solid_tile(0, 0, 255));
    let mut v = MapView::default();
    v.resize(WIDTH as f64 / dpr, (HEIGHT - 60) as f64 / dpr);
    v.open(51.5, -0.12);
    v.scale = mag;
    let tiles = v.wanted();
    // The quad to knock out: the four children of the parent the middle of the
    // view is standing on — snapped to even, which is what makes them one
    // parent's four and not a square straddling two.
    let (bx, by) = (
        (v.cx / sigil_slint::mapview::TILE).floor() as i64 & !1,
        (v.cy / sigil_slint::mapview::TILE).floor() as i64 & !1,
    );
    let drop: std::collections::HashSet<(i64, i64)> = if holes {
        [(bx, by), (bx + 1, by), (bx, by + 1), (bx + 1, by + 1)].into_iter().collect()
    } else {
        Default::default()
    };
    for &(tx, ty) in &tiles {
        if drop.contains(&(tx, ty)) {
            continue;
        }
        let img = if (tx + ty).rem_euclid(2) == 0 { red.clone() } else { blue.clone() };
        v.have.insert(v.key(tx, ty), img);
    }
    // …and the parent of the quad, in hand, for the holes to borrow from.
    if holes {
        let (k, _) = sigil_slint::mapview::ancestor(v.z, bx, by, 1);
        v.have.insert(k, quartered_tile());
    }
    let plan = v.layout(dpr, &|k| v.have.contains_key(&k));
    let rows: Vec<sigil_slint::MapTileView> = plan
        .rows
        .iter()
        .filter_map(|p| {
            let img = v.have.get(&p.key)?;
            let sz = img.size();
            let (sw, sh) = (sz.width as f32, sz.height as f32);
            Some(sigil_slint::MapTileView {
                x: p.x.into(),
                y: p.y.into(),
                w: p.w.into(),
                h: p.h.into(),
                sx: (p.fx * sw).floor() as i32,
                sy: (p.fy * sh).floor() as i32,
                sw: (p.fw * sw).ceil() as i32,
                sh: (p.fh * sh).ceil() as i32,
                img: img.clone(),
            })
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
    sigil_slint::register_fonts();
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
    // The remembered keyboard height is a device's own, read from and written
    // to the state directory at boot. Neither belongs in a shot: a developer
    // who has run the phone build would have a different number in the file
    // and different pictures out of this, and driving a fake keyboard below
    // would write that fake back over their real one. Pinned to the default,
    // and the store disconnected.
    app.set_kb_height_px(sigil_slint::bridge::KB_HEIGHT_DEFAULT);
    app.on_kb_height_seen(|_| {});
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
    // The phone turned on its side and back. The rows are virtualised (their
    // heights remembered in `known-h`), and a width change makes every
    // remembered height wrong at once; the timeline after the turn back must
    // read exactly like the one before it, no row over another.
    app.window().set_size(slint::PhysicalSize::new(HEIGHT, WIDTH));
    h.settle();
    h.shoot("chat-landscape")?;
    app.window().set_size(slint::PhysicalSize::new(WIDTH, HEIGHT));
    h.settle();
    h.shoot("chat-turned-back")?;
    // The location card at each of its three categories. Two things to read
    // off these: the card's glyph is that CATEGORY'S own — share_location for
    // a live share, my_location for a fix, pin_drop for a dropped point, the
    // same picture the attach tile and the location page show — and the
    // marker's face is OPAQUE. The face is the one that bit: with no cached
    // picture `Avatar` grounds the initials at `tint.with-alpha(0.55)`
    // (components.slint:59), so on a map the street showed through the
    // sender's head.
    {
        let items = app.get_items();
        let loc = (0..items.row_count())
            .find(|i| items.row_data(*i).map(|r| r.kind == "location").unwrap_or(false));
        if let Some(i) = loc {
            for (name, live, ended, self_loc) in [
                ("bubble-loc-live", true, false, true),
                ("bubble-loc-ended", false, true, true),
                ("bubble-loc-current", false, false, true),
                ("bubble-loc-pin", false, false, false),
            ] {
                if let Some(mut row) = items.row_data(i) {
                    row.location_live = live;
                    row.location_ended = ended;
                    row.location_self = self_loc;
                    // No composite: the card falls back to its category glyph,
                    // which is the half of this that is about icons.
                    row.location_map = Default::default();
                    items.set_row_data(i, row);
                }
                h.settle();
                h.shoot(name)?;
            }
            // …and once more over imagery, where the marker's face is drawn
            // and the ground behind it is the thing being checked.
            if let Some(mut row) = items.row_data(i) {
                row.location_live = true;
                row.location_ended = false;
                row.location_self = true;
                row.location_map = solid_tile(0, 150, 220);
                items.set_row_data(i, row);
            }
            h.settle();
            h.shoot("bubble-loc-face")?;
        }
    }
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

    // ---- the foot holds still: keyboard ↔ attach ↔ recorder ----
    //
    // The reference (Google Messages) opens every panel under the composer at
    // EXACTLY the height of the keyboard it replaces: measured off it, the
    // keyboard is 782 px tall and the attachment panel is 782 px tall, and the
    // composer band's pixels are identical between the two shots. So this is
    // an assertion before it is a picture — the page hands `chat-panel-top`
    // back and it must be the same number in every state.
    //
    // The remembered height is set here rather than driven through a fake
    // keyboard: the app keeps the TALLEST keyboard it has ever seen (a user
    // who has shrunk theirs for a moment must not shrink the panels with it),
    // so a short keyboard would no longer move it.
    //
    // Three heights, because they answer different questions. 290 clears the
    // tile grid's 264 — 16, two 116 rows, 16 — so the whole grid is in view,
    // there is nothing to drag and no handle is drawn; and it clears the
    // recorder's 284 floor, so both panels can be exactly the keyboard's
    // height and the composer can be checked across all of it. 240 does not
    // clear the grid, so the handle and the drag exist there. 180 is under
    // even one row, and only a picture.
    //
    // The phone palette for the length of it: the handle, the scrolling grid
    // and the translucent panel ground are all phone-only, and shots run in
    // desktop mode unless SIGIL_THEME_MODE says otherwise.
    let shot_mode = app.global::<sigil_slint::Theme>().get_mode();
    if shot_mode == "desktop" {
        app.global::<sigil_slint::Theme>().set_mode("dark".into());
    }
    const ROOMY_KB: f32 = 290.0;
    const TIGHT_KB: f32 = 240.0;
    let mid = WIDTH as f32 / 2.0;
    // Where the composer stands with nothing under it at all: what a panel
    // being put away has to give back.
    let y_bare = app.get_chat_panel_top();
    app.set_kb_height_px(ROOMY_KB as i32);
    app.set_kb_overlap(ROOMY_KB);
    h.settle();
    let y_keyboard = app.get_chat_panel_top();
    h.shoot("foot-keyboard")?;
    // the attach panel opened while the keyboard is still up: it must be full
    // height AT ONCE, standing where the keyboard stands, so that the band
    // above it does not sink as the keyboard leaves
    app.set_attach_open(true);
    h.settle();
    let y_attach_over_keyboard = app.get_chat_panel_top();
    // ... and the keyboard goes
    app.set_kb_overlap(0.0);
    h.settle();
    let y_attach = app.get_chat_panel_top();
    // The whole grid at 290, and NO handle: the affordance exists only when
    // there is somewhere for the sheet to go.
    h.shoot("foot-attach-roomy")?;
    // A tap in the lead-in above the first row — where the handle's strip
    // would be if there were one — must do nothing whatever.
    tap(&app, mid, HEIGHT as f32 - ROOMY_KB + 8.0);
    h.settle();
    anyhow::ensure!(
        (app.get_chat_panel_top() - y_attach).abs() < 0.5 && app.get_attach_open(),
        "there is a handle in a panel that does not need one: a tap at its \
         place moved the composer from {y_attach} to {}",
        app.get_chat_panel_top()
    );
    app.set_attach_open(false);
    h.settle();

    // The recorder at the same 290: a card squeezed from 300 to 190 under a
    // pill row and a level band that keep their sizes, which is the give the
    // panel has when the keyboard is shorter than the panel's natural height.
    app.set_kb_overlap(ROOMY_KB);
    app.set_recorder_open(true);
    h.settle();
    let y_recorder_over_keyboard = app.get_chat_panel_top();
    app.set_kb_overlap(0.0);
    h.settle();
    let y_recorder = app.get_chat_panel_top();
    h.shoot("foot-recorder-short")?;
    app.set_recorder_open(false);
    h.settle();

    // ---- the swap, both ways, frame by frame ----
    //
    // Tapping the mic with the keyboard up, and tapping the field with the
    // recorder up, are the two swaps the user called "very glitchy, it bounces
    // in a way it shouldn't". A gesture bar is standing under all of it,
    // because the bounce was about a gesture bar: the page used to split the
    // window's one bottom inset at 96, and the IME's per-frame slide walks
    // that number down through 96 — so the last stretch of a LEAVING keyboard
    // was read as a gesture bar, the recorder's floor (which counts the bar
    // from the inside) grew by that much, and the composer jumped with it.
    // The window hands the two down separately now, and the check is exactly
    // that: through the whole slide the composer holds still and the panel
    // does not change height by so much as a pixel.
    //
    // The phone's own numbers: a 306 keyboard over a 24 bar. The recorder's
    // floor — 8 + a 178 card + 8 + the 56 pill row + 28 + the bar = 302 — has
    // to fit inside the keyboard for the panel to be able to BE the keyboard's
    // height, and 306 is what the reference phone's is. It did not fit while
    // the card's floor was 184: the panel came out 308, two taller than the
    // keyboard, so the keyboard never covered it, the handover sat out its
    // 700 of give-up with the recorder standing there, and swapping back to
    // the keyboard was the last transition that still moved the composer.
    const BAR: f32 = 24.0;
    const SWAP_KB: f32 = 306.0;
    app.set_gesture_overlap(BAR);
    app.set_kb_height_px(SWAP_KB as i32);
    app.set_kb_overlap(SWAP_KB);
    h.settle();
    let y_kb_bar = app.get_chat_panel_top();
    app.set_recorder_open(true);
    h.settle();
    let rec_h = app.get_chat_panel_h();
    anyhow::ensure!(
        rec_h > 0.0,
        "the recorder did not open, so the swap proves nothing"
    );
    // The whole arrangement rests on this: the panel IS the keyboard. A floor
    // that will not fit inside the keyboard is the fault at its source, and
    // shows up downstream as a keyboard that never covers the panel.
    anyhow::ensure!(
        (rec_h - SWAP_KB).abs() < 0.5,
        "the recorder came out {rec_h} tall under a {SWAP_KB} keyboard — its \
         floor does not fit inside the keyboard it has to stand in for"
    );
    // the keyboard leaves, one frame at a time, straight through 96
    let mut prev = app.get_chat_panel_top();
    for step in (0..=12).rev() {
        app.set_kb_overlap(SWAP_KB * step as f32 / 12.0);
        h.pump();
        let y = app.get_chat_panel_top();
        anyhow::ensure!(
            y >= prev - 0.5,
            "the composer went back up as the keyboard left under the recorder: \
             {y} after {prev}"
        );
        anyhow::ensure!(
            (app.get_chat_panel_h() - rec_h).abs() < 0.5,
            "the recorder resized mid-swap: {} vs {rec_h} at inset {}",
            app.get_chat_panel_h(),
            SWAP_KB * step as f32 / 12.0
        );
        anyhow::ensure!(
            (y - y_kb_bar).abs() < 0.5,
            "the composer moved during the swap into the recorder: {y} vs {y_kb_bar}"
        );
        prev = y;
    }
    // ...and back the other way: the field is tapped, the recorder holds its
    // height while the keyboard climbs through it, and it goes only once the
    // keyboard has covered it.
    tap(&app, mid, y_kb_bar + 30.0);
    for _ in 0..4 {
        h.pump();
    }
    anyhow::ensure!(
        app.get_recorder_open(),
        "the recorder went before the keyboard arrived — that is the bounce"
    );
    for step in 0..=12 {
        app.set_kb_overlap(SWAP_KB * step as f32 / 12.0);
        h.pump();
        let y = app.get_chat_panel_top();
        anyhow::ensure!(
            (y - y_kb_bar).abs() < 0.5,
            "the composer moved during the swap out of the recorder: {y} vs \
             {y_kb_bar} at inset {}",
            SWAP_KB * step as f32 / 12.0
        );
        if app.get_recorder_open() {
            anyhow::ensure!(
                (app.get_chat_panel_h() - rec_h).abs() < 0.5,
                "the recorder resized while the keyboard climbed through it: {} \
                 vs {rec_h}",
                app.get_chat_panel_h()
            );
        }
    }
    anyhow::ensure!(
        !app.get_recorder_open(),
        "the recorder was still open behind a keyboard that had covered it"
    );
    app.set_kb_overlap(0.0);
    app.set_gesture_overlap(0.0);
    app.set_kb_height_px(ROOMY_KB as i32);
    // the field still has the focus; a panel opening is what clears it
    app.set_attach_open(true);
    h.settle();
    app.set_attach_open(false);
    h.settle();

    // ---- the IME's own frames ----
    //
    // Android reports the keyboard's inset once per frame of its animation
    // (SlintAndroidJavaHelper's WindowInsetsAnimation.Callback), and the
    // backend now asks for a frame when one arrives — without that the value
    // sat in Slint's property graph until something unrelated happened to
    // draw, which on screen read as the composer chasing the keyboard rather
    // than being pushed by it. Nothing on this side may smooth those frames:
    // the composer's lift is the inset, with no tween anywhere between them.
    // Driven here as the IME drives it — a ramp — and checked at every step.
    let ime_ramp: Vec<f32> = (0..=12).map(|i| ROOMY_KB * i as f32 / 12.0).collect();
    let mut last = app.get_chat_panel_top();
    for &inset in &ime_ramp {
        app.set_kb_overlap(inset);
        h.pump();
        let y = app.get_chat_panel_top();
        anyhow::ensure!(
            y <= last + 0.5,
            "the composer went back down during the keyboard's rise: {y} after {last}"
        );
        // ...and the timeline comes up with it. The keyboard is counted in the
        // composer's band AND in the room the list gives back at its end, and
        // if those two ever disagree the newest message is left stranded a
        // keyboard's height above the well.
        anyhow::ensure!(
            (app.get_chat_msg_gap() - 16.0).abs() < 0.5,
            "the newest message is {} above the well at inset {inset}, not 16",
            app.get_chat_msg_gap()
        );
        // No tween: the frame the inset arrived on is the frame it is at.
        if inset > 96.0 {
            anyhow::ensure!(
                (y - (y_keyboard + (ROOMY_KB - inset))).abs() < 0.5,
                "the composer lagged the keyboard's own frame: inset {inset} \
                 put it at {y}, not {}",
                y_keyboard + (ROOMY_KB - inset)
            );
        }
        last = y;
    }
    app.set_kb_overlap(0.0);
    h.settle();

    // ---- the handover ----
    //
    // A tap on the text field asks for the keyboard while a panel is standing
    // there. The panel must NOT go yet: a panel falling to nothing against a
    // keyboard still on its way makes max() dip at the crossing, which is the
    // bob the other way round. It holds its height, the keyboard's frames come
    // in underneath it, and the moment they cover it it is dropped behind
    // them. Pumped rather than settled — `settle` jumps the clock three
    // seconds, which would run the handover's own give-up timer.
    app.set_attach_open(true);
    h.settle();
    tap(&app, mid, y_attach + 30.0);
    for _ in 0..4 {
        h.pump();
    }
    anyhow::ensure!(
        app.get_attach_open(),
        "the panel went before the keyboard arrived — that is the bob"
    );
    let y_holding = app.get_chat_panel_top();
    // ...and the keyboard rises through it, frame by frame. Not "never goes
    // backward" here but "never moves at all": the panel holds the foot at its
    // own height until the keyboard passes it, and the keyboard is rising to
    // exactly that height, so every frame lands on the same number.
    for &inset in &ime_ramp {
        app.set_kb_overlap(inset);
        h.pump();
        anyhow::ensure!(
            (app.get_chat_panel_top() - y_keyboard).abs() < 0.5,
            "the composer moved during the handover: inset {inset} put it at {}",
            app.get_chat_panel_top()
        );
    }
    let y_landed = app.get_chat_panel_top();
    anyhow::ensure!(
        !app.get_attach_open(),
        "the panel was still open behind a keyboard that had covered it"
    );
    app.set_kb_overlap(0.0);
    // Opening a panel clears the composer's focus, which is the only way back
    // out of the field from here; shutting it again leaves the page as found.
    app.set_attach_open(true);
    h.settle();
    app.set_attach_open(false);
    h.settle();

    // Half a pixel of slack for the software renderer's rounding; anything
    // more is the composer bobbing, which is the whole bug.
    for (what, y) in [
        ("attach over the keyboard", y_attach_over_keyboard),
        ("attach alone", y_attach),
        ("recorder over the keyboard", y_recorder_over_keyboard),
        ("recorder alone", y_recorder),
        ("a panel holding for the keyboard it asked for", y_holding),
        ("the keyboard landed over it", y_landed),
    ] {
        anyhow::ensure!(
            (y - y_keyboard).abs() < 0.5,
            "the composer moved: {what} puts its top at {y}, the keyboard at {y_keyboard}"
        );
    }

    // ---- and the timeline stays under the composer's chin ----
    //
    // The keyboard's height goes into the composer's band (`composer-inset`)
    // and into the room the list gives back at its end (`list-pad`), and the
    // composer is then placed from the same two numbers. Count it once too
    // often in any of the three and the newest message is stranded a whole
    // keyboard above the well — a band of nothing where the conversation
    // should be. 8 of air over the band, and the well's own 8 inside it: 16,
    // in every state there is.
    for (what, set) in [
        ("nothing under it", 0u8),
        ("the keyboard up", 1),
        ("the attach panel open", 2),
        ("the recorder open", 3),
    ] {
        app.set_kb_overlap(if set == 1 { ROOMY_KB } else { 0.0 });
        app.set_attach_open(set == 2);
        app.set_recorder_open(set == 3);
        h.settle();
        anyhow::ensure!(
            (app.get_chat_msg_gap() - 16.0).abs() < 0.5,
            "with {what} the newest message sits {} above the composer's well, \
             not 16",
            app.get_chat_msg_gap()
        );
    }
    app.set_attach_open(false);
    app.set_recorder_open(false);
    app.set_kb_overlap(0.0);
    h.settle();

    // ---- a drag on the conversation puts the KEYBOARD down too ----
    //
    // The same gesture and the same rule as the panels: reaching past what is
    // standing under the composer, to read, is asking for it to be gone. The
    // only handle on the keyboard is the focus, so that is what the drag lets
    // go of; the platform lowers the IME and its own frames bring the composer
    // down, with no animation of ours running beside them. Headless there is
    // no IME to watch, so what is checked is both halves of that: the focus
    // goes, and when the frames come the composer rides them down without ever
    // going back up.
    tap(&app, mid, y_bare + 30.0);
    app.set_kb_overlap(ROOMY_KB);
    h.settle();
    anyhow::ensure!(
        app.get_chat_composer_focused(),
        "the tap did not put the caret in the field, so the drag proves nothing"
    );
    let y_typing = app.get_chat_panel_top();
    let (kx, ky) = (mid, y_typing - 260.0);
    press(&app, kx, ky);
    h.advance(std::time::Duration::from_millis(140));
    h.pump();
    for i in 1..=8 {
        drag_to(&app, kx, ky + 12.0 * i as f32);
        h.advance(std::time::Duration::from_millis(16));
        h.pump();
    }
    release(&app, kx, ky + 96.0);
    h.settle();
    anyhow::ensure!(
        !app.get_chat_composer_focused(),
        "a drag on the conversation left the caret in the field, so the \
         keyboard would have stayed up"
    );
    // ...and the IME goes down, frame by frame, with the composer on it
    let mut prev = app.get_chat_panel_top();
    for step in (0..=12).rev() {
        app.set_kb_overlap(ROOMY_KB * step as f32 / 12.0);
        h.pump();
        let y = app.get_chat_panel_top();
        anyhow::ensure!(
            y >= prev - 0.5,
            "the composer went back up as the keyboard left: {y} after {prev}"
        );
        prev = y;
    }
    anyhow::ensure!(
        (app.get_chat_panel_top() - y_bare).abs() < 0.5,
        "the composer did not come all the way back down: {} vs {y_bare}",
        app.get_chat_panel_top()
    );
    h.settle();

    // ---- and now every order of every switch, over and over ----
    //
    // The composer was still "moving intermittently when opening and closing
    // and switching between the keyboard, attachment panel, and voice panel" —
    // intermittently, which means the bad cases are the orderings nobody
    // reaches by hand. So they are all reached here: a walk over the four
    // states in a fixed pseudo-random order (the same walk every run — a test
    // that shuffles is a test that cannot be re-run), at three keyboard
    // heights, with the IME's own frames wherever a keyboard comes or goes AND
    // the switch itself landing at a random frame in the MIDDLE of them. That
    // last part is the whole point: a panel opened while the keyboard is
    // halfway out is the case a person hits by being quick, and it is where
    // every one of these bugs lived.
    //
    // What is checked, at every frame: the composer's top is exactly
    // `y_bare - foot`, where the foot is the taller of the panel that OUGHT to
    // be there — worked out here, never read back off the page — and the
    // keyboard that is there. Reading the page's own panel height back would
    // make the check agree with whatever the page did, which is the one thing
    // it must not do.
    {
        let bar = 24.0f32;
        app.set_gesture_overlap(bar);
        // 306 over a 24 bar is the reference phone's. 340 is a keyboard both
        // panels clear easily; 240 is one shorter than the recorder's own 302
        // floor, where the panel legitimately stands taller than the keyboard.
        for &kb in [306.0f32, 340.0, 240.0].iter() {
            app.set_kb_height_px(kb as i32);
            app.set_kb_overlap(0.0);
            app.set_attach_open(false);
            app.set_recorder_open(false);
            h.settle();
            // What each state's panel MUST be. The attach grid takes the
            // keyboard exactly (its floor — a 38 handle strip, 16, one 116 row
            // and the bar — is 194, and no keyboard here is under that). The
            // recorder takes the keyboard too, unless the keyboard is under
            // its own floor of 8 + a 178 card + 8 + the 56 pill row + 28 + the
            // bar = 302, and then it stands at the floor and says so.
            let want_panel = move |state: u32| -> f32 {
                match state {
                    2 => kb,
                    3 => kb.max(302.0),
                    _ => 0.0,
                }
            };
            let check = |app: &sigil_slint::AppWindow,
                         panel: f32,
                         kb_now: f32,
                         what: &str|
             -> anyhow::Result<()> {
                let foot = panel.max(if panel > 0.0 { kb_now } else { kb_now.max(bar) });
                let want = y_bare - foot;
                anyhow::ensure!(
                    (app.get_chat_panel_top() - want).abs() < 0.5,
                    "{what} (keyboard {kb}): the composer is at {}, and a \
                     {panel} panel under a keyboard of {kb_now} puts it at \
                     {want} — the page has the panel at {}",
                    app.get_chat_panel_top(),
                    app.get_chat_panel_h()
                );
                Ok(())
            };
            // Do the thing a finger would do to get from here to there.
            let act = |app: &sigil_slint::AppWindow, from: u32, to: u32| {
                match to {
                    2 => app.set_attach_open(true),
                    3 => app.set_recorder_open(true),
                    // the field: a handover when a panel is standing on it
                    1 => {
                        if from >= 2 {
                            tap(app, mid, app.get_chat_panel_top() + 30.0);
                        }
                    }
                    _ => {
                        app.set_attach_open(false);
                        app.set_recorder_open(false);
                    }
                }
            };
            let mut state = 0u32; // 0 nothing, 1 keyboard, 2 attach, 3 recorder
            let mut seed = 0x2545_F491u32;
            let mut rng = move || {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 16) & 0x7fff
            };
            for _ in 0..28 {
                let to = rng() % 4;
                if to == state {
                    continue;
                }
                // The frame of the keyboard's own slide that the finger lands
                // on. 0 is "before it started", 12 "after it finished".
                let at = (rng() % 13) as i32;
                if state == 1 && to != 1 {
                    // the keyboard goes; the panel (if any) arrives at `at`
                    for step in (0..=12).rev() {
                        let f = 12 - step;
                        if f == at {
                            act(&app, state, to);
                            for _ in 0..2 {
                                h.pump();
                            }
                        }
                        let now = kb * step as f32 / 12.0;
                        app.set_kb_overlap(now);
                        h.pump();
                        let panel = if f >= at { want_panel(to) } else { 0.0 };
                        check(&app, panel, now, "as the keyboard left")?;
                    }
                    if at > 12 {
                        act(&app, state, to);
                    }
                } else if state != 1 && to == 1 {
                    // the field is tapped, then the keyboard climbs. The panel
                    // holds its height until the keyboard covers it and goes
                    // behind it then, so the foot is the taller of the two the
                    // whole way — and whether it has gone yet is the page's to
                    // say, but how tall it was while it stood is not.
                    act(&app, state, to);
                    for _ in 0..2 {
                        h.pump();
                    }
                    for step in 0..=12 {
                        let now = kb * step as f32 / 12.0;
                        app.set_kb_overlap(now);
                        h.pump();
                        let standing =
                            app.get_attach_open() || app.get_recorder_open();
                        let panel = if standing { want_panel(state) } else { 0.0 };
                        check(&app, panel, now, "as the keyboard came")?;
                    }
                } else {
                    // no keyboard either side: the ordinary 220 wipe, and the
                    // two panels are not always the same height, so only the
                    // ends of it are anybody's business
                    act(&app, state, to);
                }
                h.settle();
                check(
                    &app,
                    want_panel(to),
                    if to == 1 { kb } else { 0.0 },
                    "once it had settled",
                )?;
                anyhow::ensure!(
                    (app.get_chat_panel_h() - want_panel(to)).abs() < 0.5,
                    "the panel settled at {} where it should be {} (keyboard \
                     {kb}, from {state} to {to})",
                    app.get_chat_panel_h(),
                    want_panel(to)
                );
                state = to;
            }
            // the field may be holding the focus; a panel opening lets it go
            app.set_attach_open(true);
            h.settle();
            app.set_attach_open(false);
            app.set_kb_overlap(0.0);
            h.settle();
        }
        app.set_gesture_overlap(0.0);
        app.set_kb_height_px(ROOMY_KB as i32);
        h.settle();
    }

    // ---- the timeline holds its place ----
    //
    // Everything that comes and goes under the composer — a panel, the
    // keyboard, the band itself — is padding at the END of the list's content,
    // so a view that is holding its place is a view whose `viewport-y` does
    // not change. When the list is at its end that offset is pinned and moves
    // with the padding, which is right; when the user has scrolled up it is
    // theirs, and nothing may touch it. Opening a panel from up there used to
    // be the complaint.
    {
        let bar = 24.0f32;
        app.set_gesture_overlap(bar);
        app.set_kb_height_px(ROOMY_KB as i32);
        app.set_kb_overlap(0.0);
        app.set_attach_open(false);
        app.set_recorder_open(false);
        h.settle();
        // up a screen, and held there: the drag ends still, so the fling does
        // not carry it somewhere this test cannot name.
        let (sx, sy) = (mid, y_bare - 300.0);
        press(&app, sx, sy);
        h.advance(std::time::Duration::from_millis(140));
        h.pump();
        for i in 1..=10 {
            drag_to(&app, sx, sy + 24.0 * i as f32);
            h.advance(std::time::Duration::from_millis(16));
            h.pump();
        }
        drag_to(&app, sx, sy + 240.0);
        h.advance(std::time::Duration::from_millis(240));
        h.pump();
        release(&app, sx, sy + 240.0);
        h.settle();
        let vy_up = app.get_chat_vp_y();
        anyhow::ensure!(
            app.get_chat_from_end() > 12.0,
            "the list is still at its end, so 'scrolled up' proves nothing: \
             {} from the end",
            app.get_chat_from_end()
        );
        for (what, open) in [
            ("the attach panel", 2u8),
            ("the recorder", 3),
            ("the keyboard", 1),
        ] {
            match open {
                2 => app.set_attach_open(true),
                3 => app.set_recorder_open(true),
                _ => {
                    for step in 0..=12 {
                        app.set_kb_overlap(ROOMY_KB * step as f32 / 12.0);
                        h.pump();
                    }
                }
            }
            h.settle();
            anyhow::ensure!(
                (app.get_chat_vp_y() - vy_up).abs() < 0.5,
                "{what} opening over a timeline the user had scrolled up moved \
                 it: {} vs {vy_up}",
                app.get_chat_vp_y()
            );
            app.set_attach_open(false);
            app.set_recorder_open(false);
            app.set_kb_overlap(0.0);
            h.settle();
            anyhow::ensure!(
                (app.get_chat_vp_y() - vy_up).abs() < 0.5,
                "{what} closing over a timeline the user had scrolled up moved \
                 it: {} vs {vy_up}",
                app.get_chat_vp_y()
            );
        }

        // ---- ...and a flick that outlives the keyboard ----
        //
        // "The timeline seems to freak out intermittently when scrolling up
        // with the keyboard open, when the keyboard then closes." The flick is
        // still running while the IME takes 13 frames to shrink the list's end
        // padding, and every one of those frames used to be a chance for the
        // pin to write `viewport-y` straight over the flick's own animation.
        // Here the two are deliberately overlapped, and the offset is read
        // every frame: it may only travel the way the finger sent it.
        app.set_kb_overlap(ROOMY_KB);
        h.settle();
        let (fx, fy) = (mid, y_bare - 300.0);
        press(&app, fx, fy);
        h.advance(std::time::Duration::from_millis(140));
        h.pump();
        for i in 1..=8 {
            drag_to(&app, fx, fy + 20.0 * i as f32);
            h.advance(std::time::Duration::from_millis(16));
            h.pump();
        }
        // released WITH speed on it, so the fling is still running below
        release(&app, fx, fy + 160.0);
        let mut prev = app.get_chat_vp_y();
        for step in (0..=12).rev() {
            app.set_kb_overlap(ROOMY_KB * step as f32 / 12.0);
            h.advance(std::time::Duration::from_millis(16));
            h.pump();
            let vy = app.get_chat_vp_y();
            // the finger went down the screen, which walks the offset back
            // toward the top of the conversation: it may rise and it may
            // stop, and it may not turn round or leap
            anyhow::ensure!(
                vy >= prev - 0.5,
                "the timeline jumped back as the keyboard closed under the \
                 flick: {vy} after {prev}"
            );
            anyhow::ensure!(
                vy - prev < 400.0,
                "the timeline teleported as the keyboard closed under the \
                 flick: {vy} after {prev}"
            );
            prev = vy;
        }
        h.settle();

        // ...and the same the other way, flung back TOWARD the end, which is
        // where the pin lives: as the fling nears the bottom the list becomes
        // "stuck" again while it is still moving, and every IME frame is then
        // a chance for the pin to slam it the rest of the way. It may coast in
        // and it may stop; it may not be thrown.
        app.set_kb_overlap(ROOMY_KB);
        h.settle();
        press(&app, fx, fy);
        h.advance(std::time::Duration::from_millis(140));
        h.pump();
        for i in 1..=8 {
            drag_to(&app, fx, fy - 20.0 * i as f32);
            h.advance(std::time::Duration::from_millis(16));
            h.pump();
        }
        release(&app, fx, fy - 160.0);
        let mut prev = app.get_chat_vp_y();
        for step in (0..=12).rev() {
            app.set_kb_overlap(ROOMY_KB * step as f32 / 12.0);
            h.advance(std::time::Duration::from_millis(16));
            h.pump();
            let vy = app.get_chat_vp_y();
            anyhow::ensure!(
                vy <= prev + 0.5,
                "the timeline jumped forward as the keyboard closed under the \
                 flick: {vy} after {prev}"
            );
            anyhow::ensure!(
                prev - vy < 400.0,
                "the timeline was thrown to the end as the keyboard closed \
                 under the flick: {vy} after {prev}"
            );
            prev = vy;
        }
        h.settle();
        app.set_gesture_overlap(0.0);
        app.set_kb_overlap(0.0);
        h.settle();
    }

    // ---- the handle, and the drag, at a keyboard the grid does not fit in ----
    app.set_kb_height_px(TIGHT_KB as i32);
    app.set_attach_open(true);
    h.settle();
    let y_tight = app.get_chat_panel_top();
    // 240: the handle's strip, the 16 lead-in, one row of tiles and the next
    // cut off at the fold, so the grid scrolls — the state the reference's own
    // shot is in, its last row peeking under the edge.
    h.shoot("foot-attach-short")?;
    //
    // The finger takes the sheet 1:1 between its two heights and NOTHING
    // latches on the way: an earlier pass decided open/shut from the move
    // handler, and one gesture that crossed the threshold a few times flipped
    // the panel open and shut over and over — "it freaks out". So the drag is
    // checked frame by frame: at every step the panel is exactly as tall as
    // the finger has made it, and the state has not moved.
    let handle_y = HEIGHT as f32 - TIGHT_KB + 19.0;
    press(&app, mid, handle_y);
    // Up, back down, up again, past the halfway mark and back under it: a
    // gesture that would have flipped the old latch four times.
    for step in [10.0f32, 30.0, 55.0, 30.0, 80.0, 20.0, 120.0] {
        drag_to(&app, mid, handle_y - step);
        h.pump();
        anyhow::ensure!(
            (app.get_chat_panel_top() - (y_tight - step)).abs() < 0.5,
            "the sheet did not follow the finger: {step} of drag put the composer \
             at {} rather than {}",
            app.get_chat_panel_top(),
            y_tight - step
        );
    }
    // 120 up from the resting height is well past the flick threshold, so the
    // release opens it the whole way.
    release(&app, mid, handle_y - 120.0);
    h.settle();
    let y_full = app.get_chat_panel_top();
    anyhow::ensure!(
        y_full < y_tight - 120.0,
        "the release did not snap the sheet open: {y_full}"
    );
    h.shoot("foot-attach-full")?;
    // ...and dragging it back down puts it where it was. The composer's band
    // is all that lies between the panel's top edge and the page's own top in
    // either state, so its height comes straight out of the collapsed y.
    let band = HEIGHT as f32 - TIGHT_KB - y_tight;
    let handle_full_y = y_full + band + 19.0;
    press(&app, mid, handle_full_y);
    drag_to(&app, mid, handle_full_y + 60.0);
    release(&app, mid, handle_full_y + 60.0);
    h.settle();
    anyhow::ensure!(
        (app.get_chat_panel_top() - y_tight).abs() < 0.5,
        "the handle dragged back down did not collapse the panel: {} vs {y_tight}",
        app.get_chat_panel_top()
    );
    // A tap on the handle toggles ONCE — no travel, one decision.
    tap(&app, mid, handle_y);
    h.settle();
    anyhow::ensure!(
        (app.get_chat_panel_top() - y_full).abs() < 0.5,
        "a tap on the handle did not open the sheet: {} vs {y_full}",
        app.get_chat_panel_top()
    );
    tap(&app, mid, handle_full_y);
    h.settle();
    anyhow::ensure!(
        (app.get_chat_panel_top() - y_tight).abs() < 0.5,
        "a second tap did not close it again: {}",
        app.get_chat_panel_top()
    );

    // ---- a drag on the conversation puts the panel away ----
    //
    // Reaching past an open panel to read is asking for the panel to be gone,
    // and the reference answers it that way: the attach sheet, the picker or
    // the recorder shuts with its ordinary wipe and the composer goes back to
    // the bottom while the list carries on scrolling under the finger. Hung on
    // the list's `flicked`, which Slint raises for a finger or a wheel and
    // never for an offset written from code — so the pin that follows the
    // panel's own opening cannot trip it. The drag is walked, as the emoji
    // grid's is, so the Flickable takes the grab off the bubble underneath.
    // Downward, into the history: the list is parked at its end, so a drag the
    // other way has nowhere to go and the Flickable never moves at all.
    let (drag_x, drag_from) = (mid, y_tight - 260.0);
    press(&app, drag_x, drag_from);
    h.advance(std::time::Duration::from_millis(140));
    h.pump();
    for i in 1..=8 {
        drag_to(&app, drag_x, drag_from + 12.0 * i as f32);
        h.advance(std::time::Duration::from_millis(16));
        h.pump();
    }
    release(&app, drag_x, drag_from + 96.0);
    h.settle();
    anyhow::ensure!(
        !app.get_attach_open() && !app.get_recorder_open(),
        "a drag on the conversation left the panel open"
    );
    anyhow::ensure!(
        (app.get_chat_panel_top() - y_bare).abs() < 0.5,
        "the composer did not go back to the bottom after the drag shut the \
         panel: {} vs {y_bare}",
        app.get_chat_panel_top()
    );
    // ...and the pin that a panel's own opening sets off must NOT count as one:
    // open it again, settle, and it is still open.
    app.set_attach_open(true);
    h.settle();
    anyhow::ensure!(
        app.get_attach_open() && (app.get_chat_panel_top() - y_tight).abs() < 0.5,
        "the panel's own opening tripped the list's drag guard: {}",
        app.get_chat_panel_top()
    );
    // Shutting the panel drops the latch with it, so the next visit is the
    // keyboard's height again rather than the page it was left at.
    app.set_attach_open(false);
    h.settle();

    // A keyboard shorter than one row of tiles and the handle above it: the
    // panel stops at that floor and the grid scrolls inside whatever it got.
    // The recorder cannot follow a keyboard this short — its pill row and
    // level band are fixed and its floor is 284 — so this one is a picture
    // only, with no claim about the composer's y.
    app.set_kb_height_px(180);
    app.set_attach_open(true);
    h.settle();
    h.shoot("foot-attach-tiny")?;
    app.set_attach_open(false);
    app.set_kb_overlap(0.0);
    h.settle();
    if shot_mode == "desktop" {
        app.global::<sigil_slint::Theme>().set_mode("desktop".into());
    }
    // Put the remembered height back where the rest of the run expects it: a
    // fake keyboard stood up here would otherwise size every panel in every
    // later shot.
    app.set_kb_height_px(sigil_slint::bridge::KB_HEIGHT_DEFAULT);
    h.settle();
    // the long-press sheet over a real bubble ("solid red …"), pressed the
    // way a finger presses it: the row fires the menu with its own live
    // rectangle, the list glides the bubble to the sheet's resting place,
    // and the scrim's hole (the bubble's own outline) rides with it
    let (pressed, pressed_id, first_id) = {
        let items = app.get_items();
        let i = (0..items.row_count())
            .find(|&i| {
                items
                    .row_data(i)
                    .map(|r| r.body.starts_with("solid red"))
                    .unwrap_or(false)
            })
            .unwrap_or(items.row_count() - 3);
        let id = items.row_data(i).map(|r| r.event_id.to_string()).unwrap_or_default();
        // the oldest message with an id: the one the timeline may be cutting
        // off under the header
        let first = (0..items.row_count())
            .filter_map(|i| items.row_data(i))
            .find(|r| !r.event_id.is_empty() && r.kind != "state" && r.kind != "dayDivider")
            .map(|r| r.event_id.to_string())
            .unwrap_or_default();
        (i, id, first)
    };
    app.invoke_debug_sheet_id(pressed_id.clone().into());
    h.settle();
    eprintln!(
        "SHEET hole-y {} copy-y {} bubble-top {} copy-h {} copy-w {} body {:?} fx {} fx-h {}",
        app.get_debug_hole_y(),
        app.get_debug_copy_y(),
        app.get_debug_bubble_top(),
        app.get_debug_copy_h(),
        app.get_debug_copy_w(),
        app.get_debug_copy_body(),
        app.get_debug_copy_fx(),
        app.get_debug_copy_fxh()
    );
    h.shoot("chat-sheet")?;
    app.invoke_debug_sheet_close();
    h.settle();
    // a message at the far end of the timeline: the list opens padding at
    // that end and glides it down into the band, so the pill has its room
    app.invoke_debug_sheet_id(first_id.into());
    h.settle();
    h.shoot("chat-sheet-clipped")?;
    app.invoke_debug_sheet_close();
    h.settle();
    let _ = pressed;
    app.invoke_debug_sheet_id(pressed_id.clone().into());
    h.settle();
    // the reaction drawer, reached the only way there is: the add-reaction
    // cell at the right end of the quick pill (chat-sheet.png: 293, 511)
    app.invoke_act("emoji-search".into(), "".into(), "".into());
    app.invoke_debug_sheet_drawer();
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
    // Current and Live Location are YOU, so the marker wears your face — the
    // very marker the timeline's location bubble draws, initials-on-tint where
    // no picture has been cached. Drop a Pin above keeps the bare pin, because
    // there is nobody to show; that difference is the whole of this pair.
    app.set_at_page("current".into());
    h.settle();
    h.shoot("attach-current")?;
    app.set_at_page("live".into());
    h.settle();
    h.shoot("attach-live")?;
    app.set_at_page("pin".into());
    h.settle();
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
        map_tiles(&app, mag, 3.0, false);
        h.frame(&format!("map-seams-{name}"))?;
    }
    // …and the same grid with a whole quad missing at the level being drawn.
    // The four holes must be covered by their parent from the level above,
    // each cropped to the quarter it stands on — white, yellow, green and
    // magenta in those corners, never the page's ground. One device pixel to
    // a logical one here, unlike the seam shots: at three the map area is a
    // tile and a half across and the quad has nowhere to be seen.
    point(&app, WindowEvent::ScaleFactorChanged { scale_factor: 1.0 });
    app.window().set_size(slint::PhysicalSize::new(WIDTH, HEIGHT));
    for (mag, name) in [(1.0, "100"), (1.13, "113")] {
        h.settle();
        map_tiles(&app, mag, 1.0, true);
        h.frame(&format!("map-gaps-{name}"))?;
    }
    // The footer against the phone's gesture bar. This page runs edge to edge
    // (app.slint's `to-the-edge`), so the window paints nothing under the bar
    // and the page's own bottom edge IS the screen's — the sheet reserves the
    // inset inside its height, chat.slint's composer fashion, and its ground
    // fills it. What must hold: the person's row is wholly above the strip,
    // and the sheet's ground still runs to the very bottom of the window.
    // A device's bar is 48 of these; the strip is drawn over the shot so the
    // clearance can be measured with an eye rather than a calculation.
    app.set_gesture_overlap(48.0);
    h.settle();
    h.frame("map-footer-gesture")?;
    app.set_gesture_overlap(0.0);
    h.settle();
    h.frame("map-footer-flat")?;
    // The map must reach the footer. It stopped doing so once the attach
    // sheet's picker began sharing this page's view: the picker's map box is a
    // 294px panel and it reports its size when the sheet BUILDS it, which is
    // long after this page reported its own — so the page ended up drawing a
    // screenful of map through a panel-sized viewport, with a wide dark band
    // where the rest of it should have been. Checked with a gesture bar and
    // without, and after a trip through the picker, which is the order that
    // broke it.
    // The header is 60 on the phone; the map box is everything under it, down
    // to the page's own bottom edge, with the footer sheet lying over its foot.
    // (The other half of it — that the GRID is drawn for the whole box and not
    // for some other surface's smaller one — is arithmetic, and lives in
    // mapview's own tests, which can drive the two surfaces directly.)
    let header_h = 60.0f32;
    let map_check = |name: &str, app: &sigil_slint::AppWindow| -> anyhow::Result<()> {
        let (bottom, foot, page) =
            (app.get_map_bottom(), app.get_map_foot_top(), app.get_map_page_h());
        // The map's ground runs to the page's own bottom edge — it must not be
        // short by a gesture inset, a keyboard, or a panel that is not there.
        anyhow::ensure!(
            (bottom - page).abs() < 0.5,
            "{name}: the map box ends at {bottom} on a page {page} tall — \
             {} short of the bottom",
            page - bottom
        );
        // …and the footer lies over its foot, with the map behind it all the
        // way up to the header.
        anyhow::ensure!(
            foot > header_h && foot < bottom,
            "{name}: the footer at {foot} is not inside a map box ending at {bottom}"
        );
        Ok(())
    };
    for (name, inset) in [("flat", 0.0f32), ("gesture", 48.0)] {
        app.set_kb_overlap(inset);
        h.settle();
        map_check(name, &app)?;
    }
    app.set_kb_overlap(0.0);
    h.settle();
    // And the order that broke it: the attach sheet's location picker takes
    // the shared view, sizes it to its own panel, and hands it back.
    app.set_nav("chat".into());
    app.set_attach_open(true);
    app.set_at_page("live".into());
    h.settle();
    app.set_at_page("grid".into());
    app.set_attach_open(false);
    app.set_nav("map".into());
    h.settle();
    map_check("after the picker", &app)?;
    // The marker wearing a face, over imagery, where both of the things that
    // were wrong with it can be measured rather than admired: the face must be
    // OPAQUE (a flat green disc, not green mixed with whatever tile is under
    // it) and it must be CENTRED on the pin's head. The head's own centre is
    // not a matter of taste — the filled `place` glyph's head lobe runs from
    // 12 to 36 of its 48px box and its counter from 20 to 28, so both are
    // centred at exactly 24, and the pin is drawn at an advance of a whole em
    // so nothing shifts it. `pin-x` is set to the middle of the map area, so
    // the disc's centre must land on that column.
    app.set_mp_avatar(solid_tile(0, 220, 0));
    app.set_mp_pin_x(WIDTH as f32 / 2.0);
    app.set_mp_pin_y(260.0);
    h.settle();
    h.frame("map-pin-face")?;
    app.set_mp_avatar(Default::default());
    app.set_mp_pin_x(-1000.0);
    app.set_mp_tiles(std::rc::Rc::new(slint::VecModel::from(Vec::<sigil_slint::MapTileView>::new())).into());
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
    // The two zones are separate numbers now: a gesture bar is not a very
    // short keyboard, and the page is told which it is looking at.
    for (name, gesture, kb) in [
        ("attach-emoji-gesture", 24.0, 0.0),
        ("attach-emoji-keyboard", 0.0, 420.0),
    ] {
        app.set_gesture_overlap(gesture);
        app.set_kb_overlap(kb);
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
    app.invoke_debug_sheet_id(pressed_id.into());
    h.settle();
    app.invoke_debug_sheet_drawer();
    emoji_pictures(&app, &h)?;
    for (name, gesture, kb) in [
        ("sheet-emoji-gesture", 24.0, 0.0),
        ("sheet-emoji-keyboard", 0.0, 420.0),
    ] {
        app.set_gesture_overlap(gesture);
        app.set_kb_overlap(kb);
        h.settle();
        h.shoot(name)?;
    }
    app.set_kb_overlap(0.0);
    app.invoke_debug_sheet_close();
    h.settle();

    // ---- the attachment sheet's grid, and the capture flow off it ----
    // Nine tiles in two rows of five minus one — ONE camera, whose Photo or
    // Video is chosen inside the chooser below, not as a second tile — and
    // the block flush with the 16 lead-in under the phone's 38 handle strip
    // (54 from the sheet's top edge to the first disc, which is what the
    // reference measures). It sat 37 lower than its lead-in for a while: the
    // sheet's phone height had been raised to 396 while the grid still wanted
    // 264, and the block, being a layout placed by hand, spread the surplus
    // between its two rows. It cannot again — the grid scrolls in its own
    // frame now and states its own height inside it.
    app.set_nav("chat".into());
    app.set_at_page("grid".into());
    app.set_attach_open(true);
    h.settle();
    h.shoot("attach-grid-tiles")?;
    // The camera tile opens no page on the phone — it asks for a viewfinder
    // of the phone's own and closes the sheet — so the chooser only exists to
    // be shot on the desktop.
    if std::env::var_os("SIGIL_THEME_MODE").is_none() {
        app.set_at_page("camera".into());
        h.settle();
        h.shoot("attach-camera")?;
    }
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

    // ---- the emoji picker at rest, and the grow on the first scroll ----
    //
    // The reference (Google Messages, Screenshot_20260903-234221.png) snaps
    // the picker to the whole page below the composer the moment the grid is
    // scrolled: the conversation goes, the composer comes to rest just under
    // the header, and the category bar stays where it was, on the gesture bar.
    // So there are three things to see here — the resting height, the grow
    // caught in flight, and the settled full height with the category bar
    // still standing off the bottom zone — and one to see in all of them: the
    // 20 of clear band under the search row, which used to be the head of the
    // scrolled content and so vanished at exactly the moment a glyph came up
    // to fill it.
    app.set_nav("chat".into());
    app.set_gesture_overlap(24.0); // a gesture bar under the panel
    app.set_at_page("emoji".into());
    app.set_attach_open(true);
    h.settle();
    // The blocks above leave "cat" in the well and the grid parked on a
    // category. The first key of the category bar is both undos at once — it
    // clears the search (the picker's own reset(), which nothing out here can
    // reach otherwise) and jumps the grid to RECENTS, offset zero. Ten keys
    // across the window, the bar 48 tall standing on the 24 gesture strip.
    tap(&app, 20.0, HEIGHT as f32 - 24.0 - 24.0);
    app.invoke_act("emoji-search".into(), "".into(), "".into());
    emoji_pictures(&app, &h)?;
    h.shoot("emoji-rest")?;
    let y_emoji_rest = app.get_chat_panel_top();
    {
        // A finger is the only thing that scrolls a Flickable, and the drag
        // has to be walked for it to take the grab off the cell underneath.
        // 140 up, in the middle of the grid band (the sheet's 400 puts it
        // between the search row at 432 and the category bar at 748).
        let (x, from, to) = (200.0f32, 700.0f32, 560.0f32);
        press(&app, x, from);
        h.advance(std::time::Duration::from_millis(140));
        h.pump();
        // The FIRST step is the trigger, so the grow is caught 50ms in — the
        // search row on its way up, the category bar where it always was.
        drag_to(&app, x, from - 20.0);
        h.advance(std::time::Duration::from_millis(50));
        h.pump();
        h.frame("emoji-growing")?;
        for i in 2..=8 {
            drag_to(&app, x, from + (to - from) * i as f32 / 8.0);
            h.advance(std::time::Duration::from_millis(16));
            h.pump();
        }
        release(&app, x, to);
        h.settle();
        // The grid's own offset is left alone by the grow. What that shows
        // here is the reference's own answer: 348 of new room opens above a
        // grid only 140 down, so it runs out of scroll and comes to rest at
        // the top — which is exactly where the reference's expanded shot is,
        // RECENTS first, after the scroll that expanded it. A grid further
        // down keeps its place, the new rows filling in above.
        h.shoot("emoji-expanded")?;
    }
    // ...and scrolling the grid back to its top puts the panel down again.
    //
    // The reference collapses on the way back up: the expanded height is the
    // browsing position, not a mode you have to leave by hand. The shot above
    // leaves the grid AT the top already (the grow opened more room than the
    // grid had travelled), so it is walked down into the list first — which
    // must NOT collapse anything, the picker only comes down at the top — and
    // then back up to it.
    {
        let (x, y0) = (200.0f32, 500.0f32);
        let y_emoji_full = app.get_chat_panel_top();
        anyhow::ensure!(
            y_emoji_full < y_emoji_rest - 40.0,
            "the picker did not expand, so its collapse proves nothing: \
             {y_emoji_full} vs {y_emoji_rest}"
        );
        // Every drag here ends by holding still for a moment before the
        // finger lifts. A release with speed on it starts Slint's fling, and
        // `settle` jumps the clock three seconds — the whole fling in one go,
        // which carried the grid thousands of pixels down and left the walk
        // back nowhere near the top. Held still, the offset is exactly what
        // the finger asked for.
        press(&app, x, y0);
        h.advance(std::time::Duration::from_millis(140));
        h.pump();
        for i in 1..=10 {
            drag_to(&app, x, y0 - 16.0 * i as f32);
            h.advance(std::time::Duration::from_millis(16));
            h.pump();
        }
        drag_to(&app, x, y0 - 160.0);
        h.advance(std::time::Duration::from_millis(220));
        h.pump();
        release(&app, x, y0 - 160.0);
        h.settle();
        anyhow::ensure!(
            app.get_chat_emoji_grid_y() < -100.0,
            "the grid did not scroll, so nothing below proves anything: \
             offset {}",
            app.get_chat_emoji_grid_y()
        );
        anyhow::ensure!(
            (app.get_chat_panel_top() - y_emoji_full).abs() < 0.5,
            "the picker came down while the grid was still away from its top: \
             {} vs {y_emoji_full}",
            app.get_chat_panel_top()
        );
        // ...and back up to the top, which is what puts it down. Walked rather
        // than measured: how far the grid has to travel depends on the row
        // height, which depends on the palette, so the drag repeats until the
        // Flickable's own clamp holds it at zero.
        let up = y0 - 160.0;
        for _ in 0..6 {
            if app.get_chat_emoji_grid_y() >= 0.0 {
                break;
            }
            press(&app, x, up);
            h.advance(std::time::Duration::from_millis(140));
            h.pump();
            for i in 1..=16 {
                drag_to(&app, x, up + 20.0 * i as f32);
                h.advance(std::time::Duration::from_millis(16));
                h.pump();
            }
            drag_to(&app, x, up + 320.0);
            h.advance(std::time::Duration::from_millis(220));
            h.pump();
            release(&app, x, up + 320.0);
            h.settle();
        }
        anyhow::ensure!(
            app.get_chat_emoji_grid_y() >= 0.0,
            "the grid never came back to its top: offset {}",
            app.get_chat_emoji_grid_y()
        );
        h.shoot("emoji-collapsed")?;
        anyhow::ensure!(
            (app.get_chat_panel_top() - y_emoji_rest).abs() < 0.5,
            "the grid was scrolled back to its top and the picker stayed up: \
             {} vs {y_emoji_rest}",
            app.get_chat_panel_top()
        );
    }
    // Shutting the panel drops the latch (the host clears the picker's
    // `active`), so the next open is the resting height again — the sheet is
    // built once and outlives a close, and a picker that came back expanded
    // would be the wrong size for a page nobody had scrolled yet.
    app.set_attach_open(false);
    h.settle();
    app.set_attach_open(true);
    h.settle();
    h.shoot("emoji-reopened")?;
    app.set_attach_open(false);
    app.set_at_page("grid".into());
    app.set_gesture_overlap(0.0);
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
    // The press lands on the TALL picture, the first of the two: the list is
    // still settling its rows as the pictures decode, so the point that is
    // over a picture when the hold fires is higher up the column than the
    // resting boxes in viewer-bubble.png suggest.
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
    // Which of the two pictures the fixed tap lands on moves with every
    // change to the conversation's layout; the viewer only has to hold both
    // and open on one of them. The rest of this block wants the TALL
    // picture (index 0), and says so.
    anyhow::ensure!(
        app.get_vw_items().row_count() == 2 && app.get_vw_cur() >= 0,
        "the viewer holds every picture in the room and opened on a tapped one: {} items, cur {}",
        app.get_vw_items().row_count(),
        app.get_vw_cur()
    );
    app.set_vw_cur(0);
    h.settle();
    // ---- a full picture arriving for the page you are looking at ----
    //
    // The pager rests on the TALL screenshot-shaped picture; the LANDSCAPE
    // photo is its neighbour. Staged, because the fixture opens with every
    // file already in hand: the first page is given an mxc it has not
    // resolved, then media.ready hands it a small thumbnail, and only after
    // that the full file. The frame must not move across either — it is
    // computed from the event's declared 480×720, not from whatever was
    // decoded — and the thumbnail must stay painted while the full one fades
    // up over it, which is the flash the phone was showing.
    let thumb = h.out.join("lake-thumb.png");
    write_png(&thumb, 120, 180)?;
    let ready = |mxc: &str, path: &std::path::Path, size: &str| {
        let v = serde_json::json!({"mxc": mxc, "path": path.to_string_lossy(),
                                   "thumbnail": size});
        sigil_slint::bridge::with_ui(|ui| {
            if let Some(win) = ui.win.upgrade() {
                sigil_slint::actions::apply_media_ready(ui, &win, &v);
            }
        });
    };
    sigil_slint::bridge::with_ui(|ui| {
        let m = &mut ui.viewer_items[0]["media"];
        m["mxc"] = serde_json::json!("mxc://sigil.test/shot");
        m["path"] = serde_json::json!("");
        m["thumbnailPath"] = serde_json::json!("");
    });
    let neighbour = app.get_vw_items().row_data(1).map(|r| r.img.clone());
    ready("mxc://sigil.test/shot", &thumb, "120x180");
    h.settle();
    let frame = (app.get_vw_pic_x(), app.get_vw_pic_y(), app.get_vw_pic_w(), app.get_vw_pic_h());
    h.shoot("viewer-thumb")?;
    // …and now the full file, a wholly different resolution.
    ready("mxc://sigil.test/shot", &pic2, "");
    h.pump(); // the frame the swap lands on: the fade starts from here
    h.advance(std::time::Duration::from_millis(70));
    h.pump();
    h.frame("viewer-thumb-fading")?; // mid-fade: `shoot` would settle it first
    anyhow::ensure!(
        (app.get_vw_pic_x(), app.get_vw_pic_y(), app.get_vw_pic_w(), app.get_vw_pic_h()) == frame,
        "the picture's frame moved when its resolution changed"
    );
    h.settle();
    h.shoot("viewer-full")?;
    anyhow::ensure!(
        (app.get_vw_pic_x(), app.get_vw_pic_y(), app.get_vw_pic_w(), app.get_vw_pic_h()) == frame,
        "the picture's frame moved once the full file had landed"
    );
    anyhow::ensure!(
        app.get_vw_items().row_data(0).map(|r| r.has_full) == Some(true),
        "the page kept no thumbnail to fade the full picture in over"
    );
    anyhow::ensure!(
        app.get_vw_items().row_data(1).map(|r| r.img.clone()) == neighbour,
        "a picture landing for one page replaced its neighbour's too"
    );

    // A page turn must not rebuild the pager. Every page has to keep the very
    // Image handle it already had — a new handle is a new cache key, and the
    // renderer decodes the picture again: the neighbours "reloading" on every
    // swipe, which is what the phone showed. Only a page that gains its
    // full-resolution picture may change, and one row at a time.
    let before: Vec<slint::Image> = app.get_vw_items().iter().map(|r| r.img.clone()).collect();
    app.set_vw_cur(1);
    app.invoke_act("viewer-page".into(), "1".into(), "".into());
    h.settle();
    let after: Vec<slint::Image> = app.get_vw_items().iter().map(|r| r.img.clone()).collect();
    anyhow::ensure!(
        before == after,
        "a page turn replaced the pager's pictures: the neighbours re-decode"
    );
    // The filmstrip: the pitch is set so 16px of the picture before this one
    // stands inside the left edge at rest, whatever its shape.
    h.shoot("viewer-open")?;
    // The header's action set changes with the page — a picture of your own
    // carries the bin, one of theirs does not. It cross-fades in its own slot
    // instead of popping: ⋮ never moves, the download slides the one slot.
    if let Some(mut row) = app.get_vw_items().row_data(1) {
        row.can_redact = true;
        app.get_vw_items().set_row_data(1, row);
    }
    h.pump(); // the frame the change lands on: the fade starts from here
    h.advance(std::time::Duration::from_millis(90));
    h.pump();
    h.frame("viewer-icons-fading")?; // mid-fade: `shoot` would settle it first
    h.settle();
    h.shoot("viewer-icons-own")?;
    if let Some(mut row) = app.get_vw_items().row_data(1) {
        row.can_redact = false;
        app.get_vw_items().set_row_data(1, row);
    }
    h.settle();
    // The add-reaction drawer: the full picker over the picture. Raised the
    // way the glyph raises it (load, then open), and put away again.
    app.invoke_act("emoji-search".into(), "".into(), "".into());
    app.set_viewer_picker_open(true);
    h.settle();
    h.shoot("viewer-emoji")?;
    anyhow::ensure!(
        app.get_emoji_rows().row_count() > 0,
        "the viewer's drawer shows the emoji rows"
    );
    // The handle drags. The drawer's head rests at 0.62 of the viewer, so the
    // 24 grab strip is 8 below that; the finger takes it down 1:1 and either
    // it settles back or, past a quarter of its own height, it goes. The
    // handle travels WITH the drawer, so each move is a step from where the
    // pill now is, not from where the gesture began — the same arithmetic the
    // message sheet's drawer uses, and the reason the port's static pill felt
    // like it was fighting the finger. Seven waypoints, then a release short
    // of the threshold: the drawer must still be open.
    let head = HEIGHT as f32 - (HEIGHT as f32 * 0.62);
    let grab = head + 18.0;
    press(app, 200.0, grab);
    h.pump();
    for (n, step) in [8.0f32, 20.0, 34.0, 46.0, 58.0, 68.0, 76.0].iter().enumerate() {
        drag_to(app, 200.0, grab + step);
        h.pump();
        h.frame(&format!("viewer-drawer-drag-{n}"))?;
    }
    release(app, 200.0, grab + 76.0);
    h.settle();
    anyhow::ensure!(
        app.get_viewer_picker_open(),
        "the drawer was let go short of a quarter of its height and still went"
    );
    h.shoot("viewer-drawer-settled")?;
    // …and past the threshold it dismisses. A quarter of 0.62 of 820 is 127.
    press(app, 200.0, grab);
    h.pump();
    for step in [40.0f32, 90.0, 140.0, 180.0] {
        drag_to(app, 200.0, grab + step);
        h.pump();
    }
    release(app, 200.0, grab + 180.0);
    h.settle();
    anyhow::ensure!(
        !app.get_viewer_picker_open(),
        "the drawer was dragged past a quarter of its height and stayed"
    );
    app.set_viewer_picker_open(false);
    h.settle();
    // the ⋮ menu: the last of the 48 tap targets on the reference's pitch,
    // its centre 28 in from the right edge and 32 down the 64 bar.
    tap(app, WIDTH as f32 - 28.0, 32.0);
    h.shoot("viewer-menu")?;
    // Forward: the card is 160 wide 10 in from the right at y 59, its first
    // row 6 inside that and 34 tall, so the row's middle is at y 82.
    tap(app, WIDTH as f32 - 100.0, 82.0);
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

    // ---- the camera ----
    //
    // On the PHONE there is nothing to shoot: the viewfinder is a window of
    // the phone's own laid over the whole screen (java/SigilCamera.java), and
    // the tile's whole job here is to ask for it and get out of the way — the
    // sheet closes and no page opens. So the only camera chrome this harness
    // can see is the DESKTOP chooser, which has no camera behind it and picks
    // Photo or Video before handing the job to the machine.
    if std::env::var_os("SIGIL_THEME_MODE").is_none() {
        app.set_nav("chat".into());
        app.set_attach_open(true);
        app.set_at_page("camera".into());
        h.settle();
        let theme = app.global::<sigil_slint::Theme>();
        theme.set_cam_mode("photo".into());
        h.settle();
        h.shoot("attach-camera-photo")?;
        // Both faces of the shutter. The selector is tapped rather than set on
        // the phone; here the sheet's foot moves with the composer and the
        // conversation band, so the mode is driven through the property the
        // selector writes and what is checked is that the chooser follows it.
        theme.set_cam_mode("video".into());
        h.settle();
        h.shoot("attach-camera-video")?;
        theme.set_cam_mode("photo".into());
        app.set_at_page("grid".into());
        app.set_attach_open(false);
        h.settle();
    }
    Ok(())
}
