//! The end-to-end driver: the real app, the real engine linked in, a real
//! server, no display. It walks the app the way a person would and captures
//! each page on the way. The shell tests around it start the server and
//! play the other people with the command-line client.
//!
//!   drive <out-dir> <server host:port, plain http> <invite> [localpart] [scenario]
//!
//! Scenarios:
//!   doors  (default) create an account with a password, see the recovery
//!          code, land on Home, open Settings, round-trip a setting.
//!   home   create an account, wait for a request from a stranger, accept
//!          it from the Requests tab, read their message, reply.
//!
//! The account's state goes wherever XDG_STATE_HOME points; the tests use
//! a temporary directory.

use sigil_slint::headless::Harness;
use slint::{ComponentHandle, Model};
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    anyhow::ensure!(
        args.len() >= 4,
        "usage: drive <out-dir> <server> <invite> [localpart] [scenario]"
    );
    let (out, server, invite) = (&args[1], &args[2], &args[3]);
    let localpart = args.get(4).cloned().unwrap_or_else(|| "alice".into());
    let scenario = args.get(5).cloned().unwrap_or_else(|| "doors".into());
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

    match scenario.as_str() {
        "doors" => doors(&h, &app, server, invite, &localpart),
        "home" => home(&h, &app, server, invite, &localpart),
        other => anyhow::bail!("unknown scenario {other}"),
    }
}

/// Through the doors to Home, with a password.
fn enter(
    h: &Harness,
    app: &sigil_slint::AppWindow,
    server: &str,
    invite: &str,
    localpart: &str,
    password: &str,
) -> anyhow::Result<()> {
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

    // 2. create the account
    app.set_door("create".into());
    app.invoke_door_create(localpart.into(), invite.into(), password.into());
    h.wait_until("the session to come up", Duration::from_secs(60), || {
        app.get_session() == "loggedIn" || !app.get_door_error().is_empty()
    })?;
    anyhow::ensure!(
        app.get_door_error().is_empty(),
        "create failed: {}",
        app.get_door_error()
    );
    println!("signed in as {}", app.get_my_user_id());
    Ok(())
}

fn doors(
    h: &Harness,
    app: &sigil_slint::AppWindow,
    server: &str,
    invite: &str,
    localpart: &str,
) -> anyhow::Result<()> {
    h.shoot("live-door-server")?;
    enter(h, app, server, invite, localpart, "correct horse")?;

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

/// Bob: a stranger writes first; the request is accepted from Home and
/// answered from the conversation.
fn home(
    h: &Harness,
    app: &sigil_slint::AppWindow,
    server: &str,
    invite: &str,
    localpart: &str,
) -> anyhow::Result<()> {
    enter(h, app, server, invite, localpart, "")?;
    h.wait_until("rooms.list", Duration::from_secs(20), || {
        app.get_rooms_loaded()
    })?;
    h.shoot("live-home-empty")?;

    // 1. the request lands on the Requests tab
    h.wait_until("a request to arrive", Duration::from_secs(90), || {
        app.get_requests().row_count() > 0
    })?;
    let req = app.get_requests().row_data(0).expect("a request row");
    println!("request from {} : {}", req.name, req.preview);
    app.invoke_set_home_tab(1);
    h.shoot("live-requests")?;

    // 2. open it: the conversation page with the request banner
    app.invoke_room_clicked(req.id.clone());
    h.wait_until("the request page", Duration::from_secs(20), || {
        app.get_nav() == "chat" && app.get_chat_is_invite()
    })?;
    h.shoot("live-request-open")?;

    // 3. accept; the conversation replaces the request and carries the first message
    app.invoke_act("accept-invite".into(), "".into(), "".into());
    h.wait_until(
        "the request to become a conversation",
        Duration::from_secs(60),
        || app.get_requests().row_count() == 0 && app.get_rooms().row_count() == 1,
    )?;
    let room = app.get_rooms().row_data(0).expect("the conversation row");
    println!("conversation with {}", room.name);
    app.invoke_room_clicked(room.id.clone());
    h.wait_until(
        "the first message in the timeline",
        Duration::from_secs(60),
        || {
            app.get_nav() == "chat"
                && !app.get_chat_is_invite()
                && app
                    .get_items()
                    .iter()
                    .any(|i| i.body.contains("hello from alice"))
        },
    )?;
    h.shoot("live-chat-accepted")?;

    // 4. reply
    app.invoke_send_message("hi back from bob".into());
    h.wait_until(
        "the reply to show as our own",
        Duration::from_secs(60),
        || {
            app.get_items()
                .iter()
                .any(|i| i.is_own && i.body.contains("hi back from bob"))
        },
    )?;
    h.shoot("live-chat-replied")?;
    println!("drive ok");
    Ok(())
}
