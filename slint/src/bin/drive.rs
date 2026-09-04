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
//!          edit and a deletion, each checked in the timeline; then a
//!          picture and the viewer, a document and its page, a track and
//!          its page, and a voice message.
//!   groups create a group, add someone, wait for DRIVE_SYNC to appear (the
//!          test lets them join first), make them admin, rename, leave.
//!   caller create an account, start a conversation with @alice, and once
//!          she answers start a voice call: hear her, react, mute, minimise,
//!          hang up. Runs against a second drive playing `callee`.
//!   callee create an account, accept the stranger's conversation, answer,
//!          take the incoming call, hear the caller, see the reaction, and
//!          see the call end.
//!   kinds  the home scenario, then a pin, a poll (voted on from both
//!          sides, then ended), a thread, a sticker, a contact card, a
//!          place, and a link preview; the test plays the other side.
//!
//! The account's state goes wherever XDG_STATE_HOME points; the tests use
//! a temporary directory.
//!   oidc   a server whose registration is a sign-in at its provider:
//!          probe, the browser round-trip, name, recovery password, Home.
//!   oidc-back the same account on a second device: the sign-in holds the
//!          name, the recovery password alone brings everything back.

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
    let req = sigil_slint::bridge::start(&app, &rt, icons);
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
        "chat" => chat(&h, &app, &req, server, invite, &localpart),
        "groups" => groups(&h, &app, server, invite, &localpart),
        "kinds" => kinds(&h, &app, server, invite, &localpart),
        "caller" => caller(&h, &app, server, invite, &localpart),
        "callee" => callee(&h, &app, server, invite, &localpart),
        "oidc" => oidc(&h, &app, server, &localpart),
        "oidc-back" => oidc_back(&h, &app, server, &localpart),
        other => anyhow::bail!("unknown scenario {other}"),
    }
}

/// The doors on a server whose registration is a sign-in at its identity
/// provider: probe (the browser opens by itself; SIGIL_BROWSER fetches the
/// login page, which the fake issuer answers by redirecting straight
/// back), choose the name, set the recovery password, land on Home with
/// no printed code in the way.
fn oidc(
    h: &Harness,
    app: &sigil_slint::AppWindow,
    server: &str,
    localpart: &str,
) -> anyhow::Result<()> {
    oidc_signin(h, app, server)?;
    anyhow::ensure!(
        app.get_door() == "name",
        "expected the name door, got {}",
        app.get_door()
    );
    println!("signed in at the provider as {}", app.get_door_oidc_user());
    h.shoot("live-door-oidc-done")?;
    // the name as suggested, then the recovery password
    app.set_door("password".into());
    h.shoot("live-door-password")?;
    app.invoke_door_create(localpart.into(), "".into(), "correct horse".into());
    h.wait_until("the session to come up", Duration::from_secs(60), || {
        app.get_session() == "loggedIn" || !app.get_door_error().is_empty()
    })?;
    anyhow::ensure!(
        app.get_door_error().is_empty(),
        "create failed: {}",
        app.get_door_error()
    );
    println!("signed in as {}", app.get_my_user_id());
    h.wait_until("rooms.list", Duration::from_secs(20), || {
        app.get_rooms_loaded()
    })?;
    // the server took the escrow, so no code page stands between the
    // doors and Home
    anyhow::ensure!(!app.get_recovery_open(), "the recovery code page opened");
    h.shoot("live-home-oidc")?;
    app.invoke_go("settings".into());
    h.wait_until("recovery.status to say escrowed", Duration::from_secs(20), || {
        app.get_recovery_state() == "enabled" && app.get_recovery_escrow()
    })?;
    h.shoot("live-settings-oidc")?;
    println!("drive oidc ok");
    Ok(())
}

/// A second device for that account: the sign-in holds the name, so the
/// welcome door asks for the recovery password alone.
fn oidc_back(
    h: &Harness,
    app: &sigil_slint::AppWindow,
    server: &str,
    localpart: &str,
) -> anyhow::Result<()> {
    oidc_signin(h, app, server)?;
    anyhow::ensure!(
        app.get_door() == "welcome",
        "expected the welcome door, got {}",
        app.get_door()
    );
    anyhow::ensure!(
        app.get_door_oidc_user() == localpart,
        "the sign-in holds {}, not {localpart}",
        app.get_door_oidc_user()
    );
    println!("welcome back as {}", app.get_door_oidc_user());
    h.shoot("live-door-welcome")?;
    // a wrong password is refused and the door stays
    app.invoke_door_recover(localpart.into(), "wrong horse".into(), "".into());
    h.wait_until("the wrong password to be refused", Duration::from_secs(60), || {
        app.get_session() == "loggedIn" || !app.get_door_error().is_empty()
    })?;
    anyhow::ensure!(app.get_session() != "loggedIn", "a wrong password restored the account");
    println!("wrong password refused: {}", app.get_door_error());
    app.invoke_door_recover(localpart.into(), "correct horse".into(), "".into());
    h.wait_until("the session to come back", Duration::from_secs(60), || {
        app.get_session() == "loggedIn"
            || (!app.get_door_error().is_empty() && !app.get_door_busy())
    })?;
    anyhow::ensure!(
        app.get_session() == "loggedIn",
        "restore failed: {}",
        app.get_door_error()
    );
    println!("restored as {}", app.get_my_user_id());
    h.wait_until("rooms.list", Duration::from_secs(20), || {
        app.get_rooms_loaded()
    })?;
    h.shoot("live-home-oidc-back")?;
    println!("drive oidc-back ok");
    Ok(())
}

/// Type the server and wait for the sign-in the probe starts to come back.
fn oidc_signin(
    h: &Harness,
    app: &sigil_slint::AppWindow,
    server: &str,
) -> anyhow::Result<()> {
    // a bare name is typed as people type it; host:port is a plain-http test server
    let typed = if server.contains(':') {
        format!("http://{server}")
    } else {
        server.to_string()
    };
    app.invoke_door_probe(typed.into());
    h.wait_until("the server card", Duration::from_secs(20), || {
        app.get_door() != "server" || !app.get_door_error().is_empty()
    })?;
    anyhow::ensure!(
        app.get_door_error().is_empty(),
        "probe failed: {}",
        app.get_door_error()
    );
    println!(
        "server offers registration={} via {}",
        app.get_door_registration(),
        app.get_door_oidc_name()
    );
    anyhow::ensure!(
        app.get_door_registration() == "oidc",
        "expected the oidc gate"
    );
    anyhow::ensure!(
        app.get_door() == "signin",
        "expected the sign-in to start by itself, got the {} door",
        app.get_door()
    );
    h.shoot("live-door-oidc")?;
    h.wait_until("the browser to come back", Duration::from_secs(60), || {
        app.get_door() != "signin" || app.get_door_oidc_state() == "failed"
    })?;
    anyhow::ensure!(
        app.get_door_oidc_state() == "done",
        "sign-in failed: {}",
        app.get_door_error()
    );
    Ok(())
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
        "the reply to show as our own and sent",
        Duration::from_secs(60),
        || {
            app.get_items()
                .iter()
                .any(|i| i.is_own && i.body.contains("hi back from bob") && i.send_state == "sent")
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
    req: &sigil_slint::bridge::Requester,
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
            app.get_items().iter().any(|i| {
                i.is_own && i.kind == "image" && i.thumb.size().width > 0 && i.send_state == "sent"
            })
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

    // 6. a document: a few lines of markdown, sent as a file. The bubble
    //    shows its first lines; the document page shows all of it.
    use slint::Model as _;
    let notes = h.out.join("notes.md");
    std::fs::write(
        &notes,
        "# Plans for the week\n\nMeet at the tower on Tuesday.\n\n## Bring\n\n- lanterns\n- rope\n\nDo not tell the ferryman.\n",
    )?;
    app.invoke_act(
        "attach-path".into(),
        notes.to_string_lossy().to_string().into(),
        "".into(),
    );
    h.wait_until(
        "the document to appear with its first lines",
        Duration::from_secs(90),
        || {
            app.get_items().iter().any(|i| {
                i.is_own
                    && i.kind == "file"
                    && i.doc_lines.row_count() > 0
                    && i.send_state == "sent"
            })
        },
    )?;
    println!("document sent");
    h.shoot("live-chat-doc")?;
    let doc_item = app
        .get_items()
        .iter()
        .find(|i| i.kind == "file")
        .expect("the document row");
    app.invoke_act("open-doc".into(), doc_item.event_id.clone(), "".into());
    app.invoke_go("doc".into());
    h.wait_until("the document page", Duration::from_secs(30), || {
        app.get_dc_status().is_empty() && app.get_dc_blocks().row_count() > 0
    })?;
    anyhow::ensure!(
        app.get_dc_name() == "notes.md",
        "the document page shows the file's name, got {}",
        app.get_dc_name()
    );
    println!("document page open");
    h.shoot("live-doc")?;
    app.invoke_go_back();

    // 7. a track: a one-second tone as a WAV, sent, then its page opened.
    //    Without ffmpeg on the machine the page has no length or art, which
    //    is what it does for any track it cannot read; it still opens.
    let tone = h.out.join("tone.wav");
    std::fs::write(&tone, wav_tone())?;
    app.invoke_act(
        "attach-path".into(),
        tone.to_string_lossy().to_string().into(),
        "".into(),
    );
    h.wait_until("the track to appear", Duration::from_secs(90), || {
        app.get_items()
            .iter()
            .any(|i| i.is_own && i.kind == "audio" && i.send_state == "sent")
    })?;
    println!("track sent");
    h.shoot("live-chat-audio")?;
    let track = app
        .get_items()
        .iter()
        .find(|i| i.kind == "audio")
        .expect("the track row");
    app.invoke_act("open-audio-page".into(), track.event_id.clone(), "".into());
    app.invoke_go("audio".into());
    h.wait_until("the audio page", Duration::from_secs(30), || {
        app.get_au_status().is_empty()
    })?;
    anyhow::ensure!(
        app.get_au_title() == "tone.wav",
        "the audio page shows the file's name, got {}",
        app.get_au_title()
    );
    println!("audio page open");
    h.shoot("live-audio")?;
    app.invoke_go_back();

    // 8. a voice message. There is no microphone here, so the recorder's
    //    own send is fired with the tone as the clip and a made-up
    //    waveform; the bubble must draw the bars and the length.
    let room_id = app
        .get_rooms()
        .row_data(0)
        .map(|r| r.id.to_string())
        .unwrap_or_default();
    let wave: Vec<f64> = (0..40)
        .map(|i| 0.2 + 0.8 * ((i as f64 * 0.7).sin().abs()))
        .collect();
    req.fire(
        "voice.send",
        serde_json::json!({
            "roomId": room_id,
            "path": tone.to_string_lossy(),
            "duration": 1.0,
            "waveform": wave,
            "caption": "",
        }),
    );
    h.wait_until("the voice message", Duration::from_secs(90), || {
        app.get_items().iter().any(|i| {
            i.is_own && i.kind == "voice" && i.waveform.row_count() > 0 && i.send_state == "sent"
        })
    })?;
    println!("voice message sent");
    h.shoot("live-chat-voice")?;

    // 9. offline: the test takes the server down when asked (DRIVE_SYNC
    //    names a directory; "down" and "up" appear there). A message sent
    //    meanwhile shows as failed, and goes out on retry once it is back.
    if let Ok(dir) = std::env::var("DRIVE_SYNC") {
        let dir = std::path::PathBuf::from(dir);
        println!("server down please");
        h.wait_until(
            "the server to be taken down",
            Duration::from_secs(60),
            || dir.join("down").exists(),
        )?;
        app.invoke_send_message("sent while offline".into());
        h.wait_until("the message to fail", Duration::from_secs(120), || {
            find("sent while offline")
                .map(|i| i.send_state == "failed")
                .unwrap_or(false)
        })?;
        println!("message failed as expected");
        h.shoot("live-chat-failed")?;
        println!("server up please");
        h.wait_until("the server to come back", Duration::from_secs(60), || {
            dir.join("up").exists()
        })?;
        let failed = find("sent while offline").expect("the failed row");
        app.invoke_act(
            "menu-action".into(),
            "retry".into(),
            failed.event_id.clone(),
        );
        h.wait_until("the retry to go out", Duration::from_secs(120), || {
            find("sent while offline")
                .map(|i| i.send_state == "sent" && !i.event_id.starts_with("local:"))
                .unwrap_or(false)
        })?;
        println!("retried and sent");
        h.shoot("live-chat-retried")?;
    }
    println!("drive chat ok");
    Ok(())
}

/// Bob calls Alice, who is another instance of this app (`callee`).
fn caller(
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
    app.invoke_act("start-dm".into(), "@alice:sigil.test".into(), "".into());
    h.wait_until("the conversation", Duration::from_secs(60), || {
        app.get_nav() == "chat"
    })?;
    app.invoke_send_message("call me".into());
    h.wait_until("alice's answer", Duration::from_secs(180), || {
        app.get_items()
            .iter()
            .any(|i| !i.is_own && i.body.contains("ready"))
    })?;
    println!("alice is ready");
    app.invoke_act("start-call".into(), "false".into(), "".into());
    h.wait_until(
        "the call to connect and alice to be heard",
        Duration::from_secs(120),
        || {
            app.get_call_state() == "connected"
                && app.get_call_tiles().row_count() == 2
                && app
                    .get_call_tiles()
                    .row_data(0)
                    .map(|t| t.speaking && t.display_name == "alice")
                    .unwrap_or(false)
                && app
                    .get_call_tiles()
                    .row_data(1)
                    .map(|t| t.speaking)
                    .unwrap_or(false)
        },
    )?;
    println!("in the call, both heard");
    h.shoot("live-call")?;
    // alice says so in the conversation once she hears us; only then
    // the things she has to see: the reaction, the mute, the end
    h.wait_until("alice to have heard us", Duration::from_secs(120), || {
        app.get_items()
            .iter()
            .any(|i| !i.is_own && i.body.contains("heard you"))
    })?;
    app.invoke_act("call-react".into(), "👍".into(), "".into());
    h.wait_until("the reaction to float", Duration::from_secs(10), || {
        app.get_call_floaters().row_count() > 0
    })?;
    h.shoot("live-call-react")?;
    app.invoke_act("set-mic".into(), "false".into(), "".into());
    h.wait_until("the mute", Duration::from_secs(10), || {
        app.get_call_mic_muted()
    })?;
    h.shoot("live-call-muted")?;
    app.invoke_act("call-minimize".into(), "".into(), "".into());
    h.wait_until("the pill", Duration::from_secs(10), || {
        !app.get_call_page_open()
    })?;
    h.shoot("live-call-pip")?;
    // long enough for the other side to see the mute and the reaction
    h.wait_until("a moment", Duration::from_secs(5), || false)
        .ok();
    app.invoke_act("hang-up".into(), "".into(), "".into());
    h.wait_until("the call to end", Duration::from_secs(20), || {
        !app.get_in_call()
    })?;
    println!("hung up");
    // alice says goodbye once her side ends: the end announcement got there
    h.wait_until("alice's goodbye", Duration::from_secs(60), || {
        app.get_items()
            .iter()
            .any(|i| !i.is_own && i.body.contains("bye"))
    })?;
    println!("drive caller ok");
    Ok(())
}

/// Alice, called by Bob (`caller`).
fn callee(
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
    h.wait_until("bob's request", Duration::from_secs(180), || {
        app.get_requests().row_count() > 0
    })?;
    let req = app.get_requests().row_data(0).expect("a request row");
    app.invoke_room_clicked(req.id.clone());
    h.wait_until("the request page", Duration::from_secs(20), || {
        app.get_nav() == "chat" && app.get_chat_is_invite()
    })?;
    app.invoke_act("accept-invite".into(), "".into(), "".into());
    h.wait_until("the conversation", Duration::from_secs(60), || {
        app.get_requests().row_count() == 0 && app.get_rooms().row_count() == 1
    })?;
    let room = app.get_rooms().row_data(0).expect("the conversation row");
    app.invoke_room_clicked(room.id.clone());
    h.wait_until("bob's message", Duration::from_secs(60), || {
        app.get_nav() == "chat" && app.get_items().iter().any(|i| i.body.contains("call me"))
    })?;
    app.invoke_send_message("ready".into());
    h.wait_until("the incoming call", Duration::from_secs(180), || {
        app.get_call_incoming()
    })?;
    println!("incoming call from {}", app.get_call_incoming_name());
    h.shoot("live-call-incoming")?;
    app.invoke_act("call-accept".into(), "false".into(), "".into());
    let mut polls = 0u32;
    h.wait_until(
        "the call to connect and bob to be heard",
        Duration::from_secs(120),
        || {
            polls += 1;
            if polls % 40 == 0 {
                println!("callee: {}", dump_call(app));
            }
            app.get_call_state() == "connected"
                && app.get_call_tiles().row_count() == 2
                && app
                    .get_call_tiles()
                    .row_data(0)
                    .map(|t| t.speaking && t.display_name == "bob")
                    .unwrap_or(false)
        },
    )?;
    println!("in the call, bob heard");
    h.shoot("live-call-callee")?;
    app.invoke_send_message("heard you".into());
    h.wait_until("bob's reaction", Duration::from_secs(60), || {
        app.get_call_floaters().row_count() > 0
    })?;
    h.shoot("live-call-callee-react")?;
    h.wait_until("bob's mute", Duration::from_secs(60), || {
        app.get_call_tiles()
            .row_data(0)
            .map(|t| t.mic_muted)
            .unwrap_or(false)
    })?;
    h.shoot("live-call-callee-muted")?;
    h.wait_until("the call to end", Duration::from_secs(120), || {
        !app.get_in_call()
    })?;
    println!("call ended");
    app.invoke_send_message("bye".into());
    h.wait_until("the goodbye to go", Duration::from_secs(60), || {
        app.get_items()
            .iter()
            .any(|i| i.is_own && i.body == "bye" && i.send_state == "sent")
    })?;
    println!("drive callee ok");
    Ok(())
}

/// Everything beyond text and files: the test's Alice votes, answers in the
/// thread and shares a place when the drive prints the ids she needs.
fn kinds(
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

    // 1. pin alice's message; the pins page lists it
    app.invoke_act("menu-action".into(), "pin".into(), alice.event_id.clone());
    h.wait_until("the pin marker", Duration::from_secs(60), || {
        find("hello from alice").map(|i| i.pinned).unwrap_or(false)
    })?;
    println!("pinned");
    app.invoke_go("pins".into());
    h.wait_until("the pins page", Duration::from_secs(20), || {
        app.get_pi_loaded() && app.get_pi_items().row_count() == 1
    })?;
    h.shoot("live-pins")?;
    app.invoke_go_back();

    // 2. a poll: alice votes (the test), bob votes, bob ends it
    app.invoke_act(
        "create-poll".into(),
        "Lunch?".into(),
        "0\u{1f}Soup\u{1f}Bread".into(),
    );
    // The poll is drawn from a local echo the moment it is asked for (core's
    // send_echoed), so the row arrives carrying `local:…`; what the test
    // hands Alice has to be the id the server gave it a moment later.
    h.wait_until("the poll", Duration::from_secs(60), || {
        app.get_items().iter().any(|i| {
            i.kind == "poll"
                && i.poll_options.row_count() == 2
                && !i.event_id.is_empty()
                && !i.event_id.starts_with("local:")
        })
    })?;
    let poll = app
        .get_items()
        .iter()
        .find(|i| i.kind == "poll")
        .expect("the poll row");
    println!("poll {}", poll.event_id);
    h.wait_until("alice's vote", Duration::from_secs(120), || {
        app.get_items()
            .iter()
            .find(|i| i.kind == "poll")
            .map(|i| {
                i.poll_options
                    .row_data(1)
                    .map(|o| o.votes == 1)
                    .unwrap_or(false)
            })
            .unwrap_or(false)
    })?;
    println!("alice voted");
    app.invoke_act("vote".into(), poll.event_id.clone(), "0".into());
    h.wait_until("bob's vote", Duration::from_secs(60), || {
        app.get_items()
            .iter()
            .find(|i| i.kind == "poll")
            .map(|i| {
                i.poll_options
                    .row_data(0)
                    .map(|o| o.mine && o.votes == 1)
                    .unwrap_or(false)
            })
            .unwrap_or(false)
    })?;
    h.shoot("live-poll")?;
    app.invoke_act(
        "menu-action".into(),
        "endpoll".into(),
        poll.event_id.clone(),
    );
    h.wait_until("the poll to end", Duration::from_secs(60), || {
        app.get_items()
            .iter()
            .any(|i| i.kind == "poll" && i.poll_ended)
    })?;
    println!("poll ended");
    h.shoot("live-poll-ended")?;

    // 3. a thread under alice's message: bob answers in it, then alice does
    app.invoke_act("open-thread".into(), alice.event_id.clone(), "".into());
    h.wait_until("the thread view", Duration::from_secs(20), || {
        app.get_nav() == "thread" && app.get_items().row_count() == 1
    })?;
    app.invoke_send_message("in the thread".into());
    h.wait_until("bob's thread reply", Duration::from_secs(60), || {
        app.get_items().row_count() == 2
            && find("in the thread")
                .map(|i| i.send_state == "sent")
                .unwrap_or(false)
    })?;
    println!("thread {}", alice.event_id);
    h.wait_until("alice's thread reply", Duration::from_secs(120), || {
        find("alice in the thread").is_some()
    })?;
    h.shoot("live-thread")?;
    app.invoke_go_back();
    h.wait_until("back to the threads page", Duration::from_secs(20), || {
        app.get_nav() == "threads"
    })?;
    h.wait_until("the threads list", Duration::from_secs(20), || {
        app.get_th_loaded() && app.get_th_threads().row_count() == 1
    })?;
    anyhow::ensure!(
        app.get_th_threads()
            .row_data(0)
            .map(|t| t.reply_count)
            .unwrap_or(0)
            == 2,
        "the thread counts both replies"
    );
    h.shoot("live-threads")?;
    app.invoke_go_back();
    h.wait_until("the chat again", Duration::from_secs(20), || {
        app.get_nav() == "chat"
            && find("hello from alice")
                .map(|i| i.thread_count == 2)
                .unwrap_or(false)
    })?;
    h.shoot("live-thread-chip")?;

    // 4. a sticker from a local pack
    let packs =
        std::path::PathBuf::from(std::env::var("XDG_STATE_HOME")?).join("sigil/stickers/smiles");
    std::fs::create_dir_all(&packs)?;
    std::fs::write(packs.join("smile.png"), sticker_png())?;
    app.invoke_act("load-stickers".into(), "".into(), "".into());
    h.wait_until("the sticker pack", Duration::from_secs(20), || {
        app.get_at_stickers().row_count() == 1
    })?;
    app.invoke_act("send-sticker".into(), "0".into(), "".into());
    h.wait_until("the sticker", Duration::from_secs(60), || {
        app.get_items()
            .iter()
            .any(|i| i.is_own && i.kind == "sticker" && i.send_state == "sent")
    })?;
    println!("sticker sent");
    h.shoot("live-sticker")?;

    // 5. alice's contact card, shared from the member sheet
    app.invoke_act(
        "member-choice-open".into(),
        "@alice:sigil.test".into(),
        "alice".into(),
    );
    h.wait_until("the member sheet", Duration::from_secs(10), || {
        app.get_member_open()
    })?;
    h.shoot("live-member-sheet")?;
    app.set_member_open(false);
    app.invoke_act("member-choice".into(), "share".into(), "".into());
    h.wait_until("the contact card", Duration::from_secs(60), || {
        app.get_items()
            .iter()
            .any(|i| i.is_own && i.kind == "contact" && i.contact_id == "@alice:sigil.test")
    })?;
    println!("contact sent");
    h.shoot("live-contact")?;

    // 6. a place, through the picker, then the map page; alice sends one back
    app.invoke_act("attach-location".into(), "current".into(), "".into());
    h.wait_until("the picker", Duration::from_secs(20), || {
        app.get_attach_open() && app.get_at_page() == "current"
    })?;
    h.shoot("live-locpick")?;
    app.invoke_act(
        "location-share".into(),
        "51.5007,-0.1246".into(),
        "0".into(),
    );
    h.wait_until("the place", Duration::from_secs(60), || {
        app.get_nav() == "chat"
            && app
                .get_items()
                .iter()
                .any(|i| i.is_own && i.kind == "location")
    })?;
    println!("location sent");
    h.shoot("live-location")?;
    let place = app
        .get_items()
        .iter()
        .find(|i| i.is_own && i.kind == "location")
        .expect("the place row");
    app.invoke_act("open-map".into(), place.event_id.clone(), "".into());
    app.invoke_go("map".into());
    h.wait_until("the map page", Duration::from_secs(20), || {
        app.get_nav() == "map" && app.get_mp_who() == "You"
    })?;
    h.shoot("live-map")?;
    app.invoke_go_back();
    h.wait_until("alice's place", Duration::from_secs(120), || {
        app.get_items()
            .iter()
            .any(|i| !i.is_own && i.kind == "location" && i.location_label == "Paris")
    })?;
    println!("alice's place arrived");

    // 7. a link preview, fetched by the app once the switch is on
    app.invoke_set_previews(true);
    h.wait_until("previews on", Duration::from_secs(20), || {
        app.get_shape_previews()
    })?;
    app.invoke_send_message("see http://127.0.0.1:18450/page.html".into());
    // the picture too: the engine's cached copy has to be named for the
    // format it is, or neither it nor Slint's loader can decode the file.
    h.wait_until("the link card", Duration::from_secs(60), || {
        find("page.html")
            .map(|i| {
                i.link_has
                    && i.link_title == "A Sigil test page"
                    && i.link_img.size().width == 16
                    && i.send_state == "sent"
            })
            .unwrap_or(false)
    })?;
    println!(
        "link previewed as {}",
        find("page.html")
            .map(|i| i.event_id.to_string())
            .unwrap_or_default()
    );
    h.shoot("live-link")?;
    println!("drive kinds ok");
    Ok(())
}

/// A 48 px yellow disc with two eyes, as PNG bytes.
fn sticker_png() -> Vec<u8> {
    let n = 48u32;
    let mut px = vec![0u8; (n * n * 4) as usize];
    for y in 0..n {
        for x in 0..n {
            let (dx, dy) = (x as f32 - 23.5, y as f32 - 23.5);
            let i = ((y * n + x) * 4) as usize;
            if dx * dx + dy * dy <= 22.0 * 22.0 {
                let eye = (dx.abs() - 8.0).abs() < 3.0 && (dy + 5.0).abs() < 3.0;
                let (r, g, b) = if eye { (40, 40, 40) } else { (250, 205, 60) };
                px[i..i + 4].copy_from_slice(&[r, g, b, 255]);
            }
        }
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, n, n);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().expect("png header");
        w.write_image_data(&px).expect("png data");
    }
    out
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

/// One second of a 440 Hz tone: 8 kHz, 16-bit, mono, as a WAV file.
fn wav_tone() -> Vec<u8> {
    let rate: u32 = 8_000;
    let samples: Vec<i16> = (0..rate)
        .map(|i| {
            let t = i as f32 / rate as f32;
            ((t * 440.0 * std::f32::consts::TAU).sin() * 12_000.0) as i16
        })
        .collect();
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// The call as the page sees it, one line, for the logs.
fn dump_call(app: &sigil_slint::AppWindow) -> String {
    let tiles: Vec<String> = app
        .get_call_tiles()
        .iter()
        .map(|t| {
            format!(
                "{}:{}{}",
                t.display_name,
                if t.speaking { "speaking" } else { "quiet" },
                if t.mic_muted { ",muted" } else { "" }
            )
        })
        .collect();
    format!(
        "state={} status={} tiles=[{}] err={}",
        app.get_call_state(),
        app.get_call_status(),
        tiles.join(" "),
        app.get_call_error()
    )
}
