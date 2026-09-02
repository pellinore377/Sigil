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
    h.shoot("chat")?;
    // the long-press sheet over the newest own message, page frosted behind it
    app.invoke_debug_sheet(
        app.get_items().row_count() as i32 - 3,
        250.0,
        560.0,
        130.0,
        44.0,
    );
    h.shoot("chat-sheet")?;
    app.invoke_debug_sheet_close();
    h.settle();
    for page in ["search", "forward", "chattheme", "roomsettings"] {
        app.set_nav(page.into());
        h.shoot(page)?;
    }
    app.set_nav("chat".into());
    h.settle();

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
    Ok(())
}
