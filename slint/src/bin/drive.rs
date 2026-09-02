//! The end-to-end driver: the real app, the real engine linked in, a real
//! server, no display. It walks the doors the way a person would — types a
//! server, reads what it offers, creates an account with a password, lands
//! on Home, opens the recovery code, opens Settings — and captures each
//! page on the way. The shell test around it starts the server.
//!
//!   drive <out-dir> <server host:port, plain http> <invite> [localpart]
//!
//! The account's state goes wherever XDG_STATE_HOME points; the test uses
//! a temporary directory.

use sigil_slint::headless::Harness;
use slint::ComponentHandle;
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    anyhow::ensure!(
        args.len() >= 4,
        "usage: drive <out-dir> <server> <invite> [localpart]"
    );
    let (out, server, invite) = (&args[1], &args[2], &args[3]);
    let localpart = args.get(4).cloned().unwrap_or_else(|| "alice".into());
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new("info,sigil_engine=info"))
        .init();

    let h = Harness::install(out)?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(16 * 1024 * 1024)
        .build()?;
    let app = sigil_slint::AppWindow::new()?;
    app.window().set_size(slint::PhysicalSize::new(
        sigil_slint::headless::WIDTH,
        sigil_slint::headless::HEIGHT,
    ));
    let icons = sigil_slint::rows::IconSet::from_window(&app);
    sigil_slint::bridge::start(&app, &rt, icons);
    app.show()?;

    // the engine reports its state on start: no account → loggedOut
    h.wait_until(
        "the engine to report loggedOut",
        Duration::from_secs(20),
        || app.get_session() == "loggedOut",
    )?;
    h.shoot("live-door-server")?;

    // 1. the server: a plain-http test server is reached as http://host:port
    app.invoke_door_probe(format!("http://{server}").into());
    h.wait_until("the server card", Duration::from_secs(20), || {
        app.get_door() == "choose" || !app.get_door_error().is_empty()
    })?;
    anyhow::ensure!(
        app.get_door_error().is_empty(),
        "probe failed: {}",
        app.get_door_error()
    );
    println!(
        "server offers registration={} tpm={}",
        app.get_door_registration(),
        app.get_door_tpm()
    );
    h.shoot("live-door-choose")?;

    // 2. create the account, with a password so recovery is set up
    app.set_door("create".into());
    app.invoke_door_create(
        localpart.as_str().into(),
        invite.as_str().into(),
        "correct horse".into(),
    );
    h.wait_until("the session to come up", Duration::from_secs(60), || {
        app.get_session() == "loggedIn" || !app.get_door_error().is_empty()
    })?;
    anyhow::ensure!(
        app.get_door_error().is_empty(),
        "create failed: {}",
        app.get_door_error()
    );
    println!("signed in as {}", app.get_my_user_id());

    // 3. the recovery code shows itself once
    h.wait_until("the recovery code page", Duration::from_secs(20), || {
        app.get_recovery_open()
    })?;
    anyhow::ensure!(app.get_recovery_code().len() > 40, "no recovery code shown");
    println!(
        "recovery code shown ({} chars)",
        app.get_recovery_code().len()
    );
    h.shoot("live-recovery-code")?;
    app.set_recovery_open(false);
    app.invoke_recovery_done();

    // 4. home, then settings with its state loaded
    h.wait_until("rooms.list", Duration::from_secs(20), || {
        app.get_rooms_loaded()
    })?;
    h.shoot("live-home")?;
    app.invoke_go("settings".into());
    h.wait_until(
        "recovery.status to say enabled",
        Duration::from_secs(20),
        || app.get_recovery_state() == "enabled",
    )?;
    h.wait_until("the backup to upload", Duration::from_secs(30), || {
        app.get_backup_state() == "enabled"
    })?;
    h.shoot("live-settings")?;

    // 5. the privacy shape round-trips
    app.invoke_set_clocked(120);
    h.wait_until("shape.settings", Duration::from_secs(10), || {
        app.get_shape_clocked() == 120
    })?;
    app.invoke_set_clocked(0);
    h.wait_until("shape.settings", Duration::from_secs(10), || {
        app.get_shape_clocked() == 0
    })?;
    println!("drive ok");
    Ok(())
}
