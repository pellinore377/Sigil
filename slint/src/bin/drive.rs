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
//!   chat   the home scenario, then a reply with a quote, a reaction, an
//!          edit and a deletion, each checked in the timeline.
//!   groups create a group, add someone, wait for DRIVE_SYNC to appear (the
//!          test lets them join first), make them admin, rename, leave.
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
        "chat" => chat(&h, &app, server, invite, &localpart),
        "groups" => groups(&h, &app, server, invite, &localpart),
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

/// Bob again, going on to everything a message can have done to it.
fn chat(
    h: &Harness,
    app: &sigil_slint::AppWindow,
    server: &str,
    invite: &str,
    localpart: &str,
) -> anyhow::Result<()> {
    home(h, app, server, invite, localpart)?;
    let find = |needle: &str| -> Option<sigil_slint::TimelineRow> {
        app.get_items().iter().find(|i| i.body.contains(needle))
    };
    let alice = find("hello from alice").expect("alice's message");

    // 1. a reply with a quote
    app.invoke_act(
        "send-reply".into(),
        alice.event_id.clone(),
        "quoting you".into(),
    );
    h.wait_until("the reply with its quote", Duration::from_secs(60), || {
        find("quoting you")
            .map(|i| i.has_reply && i.reply.body.contains("hello from alice"))
            .unwrap_or(false)
    })?;
    println!("replied with a quote");

    // 2. a reaction on alice's message
    app.invoke_act("react".into(), alice.event_id.clone(), "👍".into());
    h.wait_until("the reaction to show", Duration::from_secs(60), || {
        find("hello from alice")
            .map(|i| i.reactions.iter().any(|r| r.key == "👍" && r.mine))
            .unwrap_or(false)
    })?;
    println!("reacted");
    h.shoot("live-chat-reacted")?;

    // 3. edit our own reply
    let mine = find("hi back from bob").expect("our message");
    anyhow::ensure!(
        mine.can_edit && mine.can_redact,
        "own message should be editable and deletable"
    );
    anyhow::ensure!(
        !alice.can_edit && !alice.can_redact,
        "alice's message must not be editable here"
    );
    app.invoke_act(
        "send-edit".into(),
        mine.event_id.clone(),
        "hi back from bob, edited".into(),
    );
    h.wait_until("the edit to land", Duration::from_secs(60), || {
        find("hi back from bob, edited")
            .map(|i| i.edited)
            .unwrap_or(false)
    })?;
    println!("edited");
    h.shoot("live-chat-edited")?;

    // 4. delete the quoted reply
    let quoted = find("quoting you").expect("the quoted reply");
    app.invoke_act(
        "menu-action".into(),
        "redact".into(),
        quoted.event_id.clone(),
    );
    h.wait_until("the deletion to land", Duration::from_secs(60), || {
        app.get_items()
            .iter()
            .any(|i| i.event_id == quoted.event_id && i.kind == "redacted")
    })?;
    println!("deleted");
    h.shoot("live-chat-deleted")?;

    // 5. a picture: one of our own captures, sent as a file, then opened
    let pic = h.out.join("live-chat-reacted.png");
    app.invoke_act(
        "attach-path".into(),
        pic.to_string_lossy().to_string().into(),
        "".into(),
    );
    h.wait_until(
        "the picture to appear with its thumbnail",
        Duration::from_secs(90),
        || {
            app.get_items()
                .iter()
                .any(|i| i.is_own && i.kind == "image" && i.thumb.size().width > 0)
        },
    )?;
    println!("picture sent");
    h.shoot("live-chat-picture")?;
    let pic_item = app
        .get_items()
        .iter()
        .find(|i| i.kind == "image")
        .expect("the picture row");
    app.invoke_act("viewer-open".into(), pic_item.event_id.clone(), "".into());
    h.wait_until("the viewer", Duration::from_secs(20), || {
        app.get_viewer_open()
    })?;
    h.shoot("live-viewer")?;
    app.set_viewer_open(false);
    println!("drive chat ok");
    Ok(())
}

/// Bob makes a group and runs it: adds Alice, makes her an admin, renames
/// it, and leaves. Between adding and the rest the test has Alice accept.
fn groups(
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

    // 1. a group from Start chat
    app.invoke_go("start".into());
    app.invoke_act("start-submit".into(), "create".into(), "the plan".into());
    h.wait_until("the group to open", Duration::from_secs(60), || {
        app.get_nav() == "chat" && app.get_rooms().row_count() == 1
    })?;
    let room = app.get_rooms().row_data(0).expect("the group row");
    println!("group {} created", room.name);

    // 2. its settings, then Add people
    app.invoke_go("roomsettings".into());
    h.wait_until("room.settings", Duration::from_secs(20), || {
        app.get_rs_model().room_id != ""
    })?;
    anyhow::ensure!(
        app.get_rs_model().can_edit_info,
        "the creator should be an admin"
    );
    h.shoot("live-group-settings")?;
    app.invoke_act("invite-user".into(), "@alice:sigil.test".into(), "".into());
    h.wait_until("alice to be added", Duration::from_secs(60), || {
        app.get_ap_note().starts_with("Invited")
    })?;
    println!("invited alice");
    app.invoke_act("load-members".into(), room.id.clone(), "".into());
    h.wait_until("two members", Duration::from_secs(20), || {
        app.get_me_members().row_count() == 2
    })?;
    app.invoke_go("members".into());
    h.shoot("live-group-members")?;

    // the test lets alice join before the policy changes below
    if let Ok(marker) = std::env::var("DRIVE_SYNC") {
        h.wait_until("the test's go-ahead", Duration::from_secs(120), || {
            std::path::Path::new(&marker).exists()
        })?;
    }

    // 3. admins: alice becomes one
    app.invoke_go("admins".into());
    h.wait_until("the admins page", Duration::from_secs(20), || {
        app.get_ad_members().row_count() == 2 && app.get_ad_can()
    })?;
    anyhow::ensure!(app.get_ad_admins() == 1, "one admin to begin with");
    h.shoot("live-group-admins")?;
    app.invoke_act(
        "set-admin".into(),
        "@alice:sigil.test".into(),
        "true".into(),
    );
    h.wait_until("two admins", Duration::from_secs(60), || {
        app.get_ad_admins() == 2
    })?;
    println!("alice is an admin");

    // 4. rename, privacy page, leave
    app.invoke_act("rename".into(), "the better plan".into(), "".into());
    h.wait_until("the new name", Duration::from_secs(60), || {
        app.get_rooms()
            .row_data(0)
            .map(|r| r.name == "the better plan")
            .unwrap_or(false)
    })?;
    println!("renamed");
    app.invoke_go("privacy".into());
    h.wait_until("the privacy page", Duration::from_secs(20), || {
        app.get_pv_server() != ""
    })?;
    h.shoot("live-group-privacy")?;
    app.invoke_act("leave-room".into(), "".into(), "".into());
    h.wait_until("to have left", Duration::from_secs(60), || {
        app.get_nav() == "home" && app.get_rooms().row_count() == 0
    })?;
    println!("left");
    println!("drive groups ok");
    Ok(())
}
