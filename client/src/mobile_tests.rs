use super::*;
#[path = "mobile_content_acceptance.rs"]
mod content_acceptance;

fn run(store: &mut ClientStore, value: Value) -> Value {
    let result: Value = serde_json::from_str(&store.mobile_command(&value.to_string())).unwrap();
    assert_eq!(result["ok"], true, "{result}");
    result["value"].clone()
}
#[test]
fn wallpaper_is_local_bound_to_its_conversation_and_replaced_atomically() {
    let (dir, _server, mut alice, mut bob, _now) = crate::claims::tests::pair();
    let (_, peer) = crate::incoming::tests::trust(&mut alice, &mut bob);
    let other = transport::hex(&peer);
    let image = vec![0x7b; 150_000];
    alice.mobile_set_wallpaper("self", &image).unwrap();
    alice.mobile_set_wallpaper(&other, &[0x6c; 400]).unwrap();
    assert_eq!(
        alice.mobile_wallpaper("self").unwrap().unwrap().as_slice(),
        image
    );
    assert!(!std::fs::read(dir.path().join("alice.db"))
        .unwrap()
        .windows(100)
        .any(|w| w == &image[..100]));
    let original: Vec<u8> = alice
        .db
        .query_row(
            "SELECT state FROM mobile_wallpapers ORDER BY length(state) DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    alice
        .db
        .execute(
            "UPDATE mobile_wallpapers SET state=?1 WHERE length(state)<1000",
            [&original],
        )
        .unwrap();
    assert!(alice.mobile_wallpaper(&other).is_err());
    alice.mobile_set_wallpaper(&other, &[]).unwrap();
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON mobile_wallpapers BEGIN SELECT RAISE(ABORT,'synthetic'); END").unwrap();
    assert!(alice.mobile_set_wallpaper("self", b"replacement").is_err());
    alice.db.execute_batch("DROP TRIGGER fail").unwrap();
    assert_eq!(
        alice.mobile_wallpaper("self").unwrap().unwrap().as_slice(),
        image
    );
    let mut corrupted = original;
    let (first, remaining) = corrupted[40..].split_at_mut(65_536 + 36);
    first.swap_with_slice(&mut remaining[..65_536 + 36]);
    alice
        .db
        .execute("UPDATE mobile_wallpapers SET state=?1", [&corrupted])
        .unwrap();
    assert!(alice.mobile_wallpaper("self").is_err());
    alice.mobile_set_wallpaper("self", &image).unwrap();
    assert!(alice
        .mobile_set_wallpaper("self", &vec![0; 2 * 1024 * 1024 + 1])
        .is_err());
    drop(alice);
    let mut alice = ClientStore::open(
        &dir.path().join("alice.db"),
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap();
    assert_eq!(
        alice.mobile_wallpaper("self").unwrap().unwrap().as_slice(),
        image
    );
    alice.mobile_set_wallpaper("self", &[]).unwrap();
    assert!(alice.mobile_wallpaper("self").unwrap().is_none());
    assert_eq!(
        alice
            .db
            .query_row("SELECT count(*) FROM mobile_uploads", [], |r| r
                .get::<_, u32>(0))
            .unwrap(),
        0
    );
    let conversation = alice.mobile_conversation("self").unwrap();
    assert!(alice
        .conversation_page(conversation, None, None, None, conversations::now())
        .unwrap()
        .messages
        .is_empty());
}
#[test]
fn recovery_setup_waits_for_saved_key_and_preserves_it_after_failed_commit_and_restart() {
    let (dir, _server, mut alice, _bob, _now) = crate::claims::tests::pair();
    let generated = run(&mut alice, json!({"command":"recovery_generate"}));
    let secret = generated["secret"].as_str().unwrap();
    assert_eq!(secret.len(), 64);
    assert_ne!(secret, "00".repeat(32));
    assert!(
        !run(&mut alice, json!({"command":"storage"}))["recovery"]["enabled"]
            .as_bool()
            .unwrap()
    );
    let confirm = json!({"command":"recovery_enable","secret":secret});
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON archive BEGIN SELECT RAISE(ABORT,'synthetic'); END").unwrap();
    let failed: Value = serde_json::from_str(&alice.mobile_command(&confirm.to_string())).unwrap();
    assert_eq!(failed["ok"], false);
    assert!(matches!(alice.recovery_status(), Err(Error::NotFound)));
    alice.db.execute_batch("DROP TRIGGER fail").unwrap();
    assert_eq!(run(&mut alice, confirm)["recovery"]["enabled"], true);
    assert_eq!(
        run(&mut alice, json!({"command":"recovery_policy","days":30}))["recovery"]["days"],
        30
    );
    drop(alice);
    let mut alice = ClientStore::open(
        &dir.path().join("alice.db"),
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap();
    let restored = run(&mut alice, json!({"command":"storage"}));
    assert_eq!(restored["recovery"]["enabled"], true);
    assert_eq!(restored["recovery"]["days"], 30);
    let regenerate: Value =
        serde_json::from_str(&alice.mobile_command(r#"{"command":"recovery_generate"}"#)).unwrap();
    assert_eq!(regenerate["ok"], false);
    let replacement: Value = serde_json::from_str(&alice.mobile_command(
        &json!({"command":"recovery_enable","secret":"ff".repeat(32)}).to_string(),
    ))
    .unwrap();
    assert_eq!(replacement["ok"], false);
    assert_eq!(
        run(&mut alice, json!({"command":"recovery_policy","days":null}))["recovery"]["days"],
        Value::Null
    );
}
#[test]
fn presence_is_opt_in_encrypted_and_expires_without_a_background_heartbeat() {
    let (_dir, _server, mut alice, mut bob, now) = crate::claims::tests::pair();
    let (_, peer) = crate::incoming::tests::trust(&mut alice, &mut bob);
    run(&mut alice, json!({"command":"presence","status":"busy"}));
    assert_eq!(
        alice
            .db
            .query_row("SELECT count(*) FROM send_intents", [], |r| r
                .get::<_, u32>(0))
            .unwrap(),
        0
    );
    run(
        &mut alice,
        json!({"command":"organize","request":"95".repeat(32),"timestamp":now,"value":{"PresenceSharing":true}}),
    );
    run(&mut alice, json!({"command":"presence","status":"busy"}));
    let now = conversations::now();
    for item in alice.resume_send_intents_online(now).unwrap() {
        let session = item.result.unwrap();
        alice.send_pending_online(session, now).unwrap();
    }
    for item in bob.receive_mailbox_online(now).unwrap() {
        item.result.unwrap();
    }
    let conversation = alice.direct_conversation(peer).unwrap();
    let state = bob.conversation_activity(conversation, now).unwrap();
    assert_eq!(state.len(), 1);
    assert!(
        state[0].online && state[0].status == Some(sigil_protocol::conversation::Presence::Busy)
    );
    assert!(bob
        .conversation_page(conversation, None, None, None, now)
        .unwrap()
        .messages
        .is_empty());
    let expired = bob.conversation_activity(conversation, now + 120).unwrap();
    assert!(!expired[0].online && expired[0].status.is_none());
}
#[test]
fn mobile_call_creation_rolls_back_when_invitation_storage_fails() {
    let (_dir, _server, mut alice, mut bob, now) = crate::claims::tests::pair();
    let (_, peer) = crate::incoming::tests::trust(&mut alice, &mut bob);
    alice.db.execute_batch("CREATE TRIGGER fail_invite BEFORE INSERT ON call_jobs BEGIN SELECT RAISE(ABORT,'synthetic'); END").unwrap();
    let request = json!({"command":"call_start","peer":transport::hex(&peer),"request":"94".repeat(32),"timestamp":now});
    let failed: Value = serde_json::from_str(&alice.mobile_command(&request.to_string())).unwrap();
    assert_eq!(failed["ok"], false);
    for table in ["calls", "call_jobs", "call_history"] {
        assert_eq!(
            alice
                .db
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row
                    .get::<_, u32>(0))
                .unwrap(),
            0
        );
    }
    alice.db.execute_batch("DROP TRIGGER fail_invite").unwrap();
    assert_eq!(run(&mut alice, request)["call"], "94".repeat(32));
    assert!(alice.call([0x94; 32], now).unwrap().phase == calls::Phase::Active);
}
#[test]
fn call_history_survives_expiry_and_redial_preserves_group_kind_and_trust() {
    let (dir, _server, mut alice, mut bob, now) = crate::claims::tests::pair();
    let (_, peer) = crate::incoming::tests::trust(&mut alice, &mut bob);
    let call = [91; 32];
    alice.create_group_call(call, now, 60).unwrap();
    alice.invite_to_call(call, peer, now).unwrap();
    alice.leave_call(call, now).unwrap();
    let before = run(&mut alice, json!({"command":"calls"}));
    assert_eq!(before["calls"][0]["name"], "bob");
    assert_eq!(before["calls"][0]["outgoing"], true);
    alice.resume_calls_online(now + 60).unwrap();
    assert!(matches!(alice.call(call, now + 60), Err(Error::NotFound)));
    drop(alice);
    let mut alice = ClientStore::open(
        &dir.path().join("alice.db"),
        sigil_crypto::storage::StorageKey::new(sigil_crypto::Secret32::from_bytes([9; 32]))
            .unwrap(),
    )
    .unwrap();
    let archived = run(&mut alice, json!({"command":"calls"}));
    assert_eq!(archived["calls"][0]["name"], "bob");
    assert_eq!(archived["calls"][0]["phase"], "ended");
    // The restored clock floor is deliberately ahead of wall time in this fixture.
    let request = [92; 32];
    run(
        &mut alice,
        json!({"command":"call_redial","call":transport::hex(&call),"request":transport::hex(&request),"timestamp":now}),
    );
    assert!(!alice.call(request, now + 60).unwrap().direct);
    alice.block_peer(peer, true).unwrap();
    let denied: Value = serde_json::from_str(&alice.mobile_command(&json!({"command":"call_redial","call":transport::hex(&call),"request":"93".repeat(32),"timestamp":now}).to_string())).unwrap();
    assert_eq!(denied["ok"], false);
    let sealed: Vec<u8> = alice
        .db
        .query_row("SELECT content FROM call_history LIMIT 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(!sealed.windows(12).any(|part| part == b"\"name\":\"bob\""));
}
#[test]
fn settings_show_real_device_inventory_without_granting_trust_or_revoking_other_accounts() {
    let (_dir, _server, mut alice, bob, _) = crate::claims::tests::pair();
    let own = alice.connection_session().unwrap().unwrap().device_id;
    let first = run(&mut alice, json!({"command":"devices"}));
    assert_eq!(first["devices"][0]["id"], own);
    assert_eq!(first["devices"][0]["current"], true);
    assert_eq!(first["devices"][0]["verified"], false);
    let second = run(
        &mut alice,
        json!({"command":"devices","cursor":first["next"]}),
    );
    assert!(second["next"].is_null());
    for device in [own, bob.connection_session().unwrap().unwrap().device_id] {
        let rejected: Value = serde_json::from_str(
            &alice.mobile_command(&json!({"command":"revoke_device","device":device}).to_string()),
        )
        .unwrap();
        assert_eq!(rejected["ok"], false);
    }
    let storage = run(&mut alice, json!({"command":"storage"}));
    assert!(storage["database"].as_u64().unwrap() > 0);
    assert!(storage["media_used"].as_u64().unwrap() <= storage["media"].as_u64().unwrap());
    assert_eq!(storage["recovery"]["enabled"], false);
}
#[test]
fn filtered_history_advances_and_search_opens_the_original_message() {
    let (_dir, _server, mut alice, mut bob, now) = crate::claims::tests::pair();
    let (_, peer) = crate::incoming::tests::trust(&mut alice, &mut bob);
    let peer = transport::hex(&peer);
    for index in 1..=70u8 {
        run(
            &mut alice,
            json!({"command":"post","peer":peer,"request":format!("{index:02x}").repeat(32),"timestamp":now,"text":format!("Letter {index}")}),
        );
    }
    let own = transport::hex(&alice.account_reference().unwrap());
    run(
        &mut alice,
        json!({"command":"pin","peer":peer,"request":"71".repeat(32),"timestamp":now,"author":own,"message":"01".repeat(32),"active":true}),
    );
    let first = run(
        &mut alice,
        json!({"command":"timeline","peer":peer,"category":"Pins"}),
    );
    assert_eq!(first["messages"].as_array().unwrap().len(), 0);
    assert!(first["next"].is_number());
    let older = run(
        &mut alice,
        json!({"command":"timeline","peer":peer,"category":"Pins","before":first["next"]}),
    );
    assert_eq!(older["messages"].as_array().unwrap().len(), 1);
    assert_eq!(older["messages"][0]["text"], "Letter 1");
    assert!(older["next"].is_null());
    let at = run(
        &mut alice,
        json!({"command":"timeline","peer":peer,"author":own,"message":"01".repeat(32)}),
    );
    assert_eq!(at["messages"][0]["text"], "Letter 1");
    let search = run(
        &mut alice,
        json!({"command":"timeline","peer":peer,"category":"Search","query":"Letter 69"}),
    );
    assert_eq!(search["messages"].as_array().unwrap().len(), 1);
    assert_eq!(search["messages"][0]["text"], "Letter 69");
    let unread = run(
        &mut alice,
        json!({"command":"search","query":"","category":"Unread"}),
    );
    assert!(unread["hits"].as_array().unwrap().is_empty());
    run(
        &mut alice,
        json!({"command":"post","peer":peer,"request":"72".repeat(32),"timestamp":now,"text":"A thread reply","thread_author":own,"thread_message":"01".repeat(32)}),
    );
    let thread = run(
        &mut alice,
        json!({"command":"timeline","peer":peer,"thread_author":own,"thread_message":"01".repeat(32)}),
    );
    assert_eq!(thread["messages"].as_array().unwrap().len(), 1);
    assert_eq!(thread["messages"][0]["text"], "A thread reply");
    assert_eq!(thread["messages"][0]["thread_preview"], "Letter 1");
    let found = run(
        &mut alice,
        json!({"command":"search","query":"A thread reply","category":""}),
    );
    assert_eq!(found["hits"][0]["thread_author"], own);
    assert_eq!(found["hits"][0]["thread_message"], "01".repeat(32));
    let anchored = run(
        &mut alice,
        json!({"command":"timeline","peer":peer,"category":"Timeline","author":own,"message":"72".repeat(32),"thread_author":own,"thread_message":"01".repeat(32)}),
    );
    assert_eq!(anchored["messages"][0]["text"], "A thread reply");
    assert_eq!(anchored["people"][&own], "You");
    let root = run(
        &mut alice,
        json!({"command":"timeline","peer":peer,"thread_author":own,"thread_message":"01".repeat(32),"before":thread["next"]}),
    );
    assert_eq!(root["messages"][0]["text"], "Letter 1");
}
#[test]
fn mobile_task_undo_is_available_only_for_our_recent_completion() {
    let (_dir, _server, mut alice, _bob, now) = crate::claims::tests::pair();
    let request = "83".repeat(32);
    run(
        &mut alice,
        json!({"command":"post","peer":"self","request":request,"timestamp":now-60,"rich":true,"text":"checklist::task::Letters\n- Recent\n- Earlier;"}),
    );
    let timeline = run(&mut alice, json!({"command":"timeline","peer":"self"}));
    let message = &timeline["messages"][0];
    let card = &message["parts"][0];
    let mut action = json!({"command":"card_action","peer":"self","author":message["author"],"message":request,"card":card["id"],"item":card["items"][0]["id"],"checked":true,"timestamp":now});
    run(&mut alice, action.clone());
    let complete = run(&mut alice, json!({"command":"timeline","peer":"self"}));
    assert_eq!(
        complete["messages"][0]["parts"][0]["items"][0]["checked"],
        true
    );
    assert_eq!(
        complete["messages"][0]["parts"][0]["items"][0]["enabled"],
        true
    );
    action["checked"] = json!(false);
    run(&mut alice, action.clone());
    let undone = run(&mut alice, json!({"command":"timeline","peer":"self"}));
    assert_eq!(
        undone["messages"][0]["parts"][0]["items"][0]["checked"],
        false
    );
    action["item"] = card["items"][1]["id"].clone();
    action["checked"] = json!(true);
    action["timestamp"] = json!(now - 31);
    run(&mut alice, action.clone());
    let old = run(&mut alice, json!({"command":"timeline","peer":"self"}));
    assert_eq!(old["messages"][0]["parts"][0]["items"][1]["enabled"], false);
    action["checked"] = json!(false);
    let rejected: Value = serde_json::from_str(&alice.mobile_command(&action.to_string())).unwrap();
    assert_eq!(rejected["ok"], false);
}
#[test]
fn mobile_cards_use_authorized_state_and_hide_poll_results_until_voting() {
    let (dir, _server, mut alice, _bob, now) = crate::claims::tests::pair();
    let request = "91".repeat(32);
    run(
        &mut alice,
        json!({"command":"post","peer":"self","request":request,"timestamp":now,"rich":true,"text":"poll::closed::Lunch?\n- Soup\n- Salad;"}),
    );
    let timeline = run(&mut alice, json!({"command":"timeline","peer":"self"}));
    let message = &timeline["messages"][0];
    let poll = &message["parts"][0];
    assert_eq!(poll["text"], "Lunch?");
    assert!(poll["voters"].is_null());
    assert!(poll["items"][0]["count"].is_null());
    let vote = json!({"command":"card_action","peer":"self","author":message["author"],"message":request,"card":poll["id"],"choices":[poll["items"][1]["id"]],"timestamp":now});
    run(&mut alice, vote.clone());
    run(&mut alice, vote);
    drop(alice);
    let mut alice = ClientStore::open(
        &dir.path().join("alice.db"),
        StorageKey::new(sigil_crypto::Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap();
    let timeline = run(&mut alice, json!({"command":"timeline","peer":"self"}));
    assert_eq!(timeline["messages"].as_array().unwrap().len(), 1);
    let poll = &timeline["messages"][0]["parts"][0];
    assert_eq!(poll["voters"], 1);
    assert_eq!(poll["items"][1]["count"], 1);
    assert_eq!(poll["items"][1]["checked"], true);
    let bad = serde_json::from_str::<Value>(&alice.mobile_command(&json!({"command":"card_action","peer":"self","author":message["author"],"message":request,"card":"92".repeat(32),"choices":[],"timestamp":now}).to_string())).unwrap();
    assert_eq!(bad["ok"], false);
}
#[test]
fn uploaded_files_queue_from_encrypted_cache_and_deletion_revokes_chunk_access() {
    let (_dir, _server, mut alice, _bob, now) = crate::claims::tests::pair();
    let request = "84".repeat(32);
    let begin = json!({"command":"file_begin","peer":"self","request":request,"timestamp":now,"length":5,"name":"letter.txt","media_type":"text/plain"});
    assert_eq!(run(&mut alice, begin.clone()), run(&mut alice, begin));
    alice.mobile_file_stage(&request, 0, b"hello").unwrap();
    assert!(alice.mobile_file_stage(&request, 0, b"other").is_err());
    run(
        &mut alice,
        json!({"command":"file_finish","request":request}),
    );
    let upload = alice.mobile_upload(id(&request).unwrap()).unwrap();
    let mut cache = alice.mobile_cache().unwrap();
    for _ in 0..3 {
        alice
            .upload_attachment_step(&mut cache, upload.file)
            .unwrap();
    }
    assert_eq!(run(&mut alice, json!({"command":"file_work"}))["sent"], 1);
    let timeline = run(&mut alice, json!({"command":"timeline","peer":"self"}));
    let message = &timeline["messages"][0];
    assert_eq!(message["attachment"]["name"], "letter.txt");
    assert_eq!(message["attachment"]["length"], 5);
    let author = message["author"].as_str().unwrap();
    assert_eq!(
        &*alice
            .mobile_file_chunk("self", author, &request, 0)
            .unwrap(),
        b"hello"
    );
    run(
        &mut alice,
        json!({"command":"delete","peer":"self","request":"85".repeat(32),"timestamp":now,"author":author,"message":request}),
    );
    assert!(alice
        .mobile_file_chunk("self", author, &request, 0)
        .is_err());
}
#[test]
fn queued_attachment_keeps_reply_and_thread_across_restart() {
    let (dir, _server, mut alice, _bob, now) = crate::claims::tests::pair();
    let root = "81".repeat(32);
    run(
        &mut alice,
        json!({"command":"post","peer":"self","request":root,"timestamp":now,"text":"Thread root"}),
    );
    let author = transport::hex(&alice.account_reference().unwrap());
    let request = "82".repeat(32);
    let begin = json!({"command":"file_begin","peer":"self","request":request,"timestamp":now,"length":5,"name":"thread.txt","media_type":"text/plain","reply_author":author,"reply_message":root,"thread_author":author,"thread_message":root});
    run(&mut alice, begin.clone());
    alice.mobile_file_stage(&request, 0, b"hello").unwrap();
    run(
        &mut alice,
        json!({"command":"file_finish","request":request}),
    );
    drop(alice);
    let mut alice = ClientStore::open(
        &dir.path().join("alice.db"),
        sigil_crypto::storage::StorageKey::new(sigil_crypto::Secret32::from_bytes([9; 32]))
            .unwrap(),
    )
    .unwrap();
    run(&mut alice, begin.clone());
    let mut conflicting = begin;
    conflicting["thread_message"] = json!("83".repeat(32));
    let result: Value =
        serde_json::from_str(&alice.mobile_command(&conflicting.to_string())).unwrap();
    assert_eq!(result["ok"], false);
    let upload = alice.mobile_upload(id(&request).unwrap()).unwrap();
    let mut cache = alice.mobile_cache().unwrap();
    for _ in 0..3 {
        alice
            .upload_attachment_step(&mut cache, upload.file)
            .unwrap();
    }
    assert_eq!(run(&mut alice, json!({"command":"file_work"}))["sent"], 1);
    let timeline = run(
        &mut alice,
        json!({"command":"timeline","peer":"self","thread_author":author,"thread_message":root}),
    );
    assert_eq!(timeline["messages"].as_array().unwrap().len(), 2);
    assert_eq!(timeline["messages"][0]["reply"], "Thread root");
    assert_eq!(timeline["messages"][1]["id"], root);
    assert_eq!(timeline["messages"][0]["attachment"]["name"], "thread.txt");
}
#[test]
fn group_creation_retries_without_duplicate_groups_or_invitations() {
    let (dir, _server, mut alice, mut bob, now) = crate::claims::tests::pair();
    let mut server = sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap();
    server
        .configure_groups(sigil_protocol::groups::Configure {
            expected_revision: 0,
            enabled: true,
            storage_limit_bytes: 1024 * 1024,
        })
        .unwrap();
    alice.publish_device_binding_online().unwrap();
    bob.publish_device_binding_online().unwrap();
    let (_, peer) = crate::incoming::tests::trust(&mut alice, &mut bob);
    let request = json!({"command":"group_create","request":"73".repeat(32),"timestamp":now,"name":"Study circle","description":"An encrypted group","peers":[transport::hex(&peer)]});
    let first = run(&mut alice, request.clone());
    assert_eq!(first, run(&mut alice, request));
    let state = run(&mut alice, json!({"command":"state"}));
    let groups: Vec<_> = state["chats"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["group"] == true)
        .collect();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0]["name"], "Study circle");
    let invitations: i64 = alice
        .db
        .query_row("SELECT count(*) FROM group_invitations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(invitations, 1);
    let leave = json!({"command":"leave_group","peer":first["open"]});
    run(&mut alice, leave.clone());
    run(&mut alice, leave);
    let state = run(&mut alice, json!({"command":"state"}));
    assert!(state["chats"]
        .as_array()
        .unwrap()
        .iter()
        .all(|chat| chat["group"] != true));
}

#[test]
fn completed_structured_posts_release_draft_capacity_and_keep_a_durable_ack() {
    let (_dir, _server, mut alice, _bob, now) = crate::claims::tests::pair();
    for n in 1u64..=257 {
        let mut request = [0; 32];
        request[..8].copy_from_slice(&n.to_be_bytes());
        let request = transport::hex(&request);
        run(
            &mut alice,
            json!({"command":"post","peer":"self","request":request,"timestamp":now,"text":"note::Keep this note;","rich":true}),
        );
        assert_eq!(
            run(
                &mut alice,
                json!({"command":"post_status","peer":"self","request":request})
            )["queued"],
            true
        );
    }
    let count: i64 = alice
        .db
        .query_row("SELECT count(*) FROM structured_drafts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
    let notes = run(
        &mut alice,
        json!({"command":"search","query":"Keep this note"}),
    );
    assert!(!notes["hits"].as_array().unwrap().is_empty());
    assert!(notes["hits"]
        .as_array()
        .unwrap()
        .iter()
        .all(|v| v["noted"] == true && v["peer"] == "self"));
}

#[test]
fn account_preferences_are_private_and_conversation_overrides_survive_restart() {
    let (dir, _server, mut alice, mut bob, now) = crate::claims::tests::pair();
    let (_, peer) = crate::incoming::tests::trust(&mut alice, &mut bob);
    for (n, scope, value) in [
        (1, None, "global"),
        (2, Some(transport::hex(&peer)), "local"),
    ] {
        run(
            &mut alice,
            json!({"command":"organize","peer":scope,"request":format!("{n:02x}").repeat(32),"timestamp":now,"value":{"UiSetting":{"key":"appearance","value":value}}}),
        );
    }
    let conversation = alice.direct_conversation(peer).unwrap();
    let operation = alice
        .conversation_operation(
            [3; 32],
            Action::Private {
                conversation,
                value: conversations::Private::UiSetting {
                    key: "appearance".into(),
                    value: Some("private value".into()),
                },
            },
        )
        .unwrap();
    assert!(alice
        .queue_peer_operation(peer, &operation, now, now)
        .is_err());
    drop(alice);
    let mut alice = ClientStore::open(
        &dir.path().join("alice.db"),
        StorageKey::new(sigil_crypto::Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap();
    assert_eq!(
        alice.conversation_preferences([0; 32]).unwrap().ui["appearance"],
        "global"
    );
    assert_eq!(
        alice.conversation_preferences(conversation).unwrap().ui["appearance"],
        "local"
    );
    assert!(!std::fs::read(dir.path().join("alice.db"))
        .unwrap()
        .windows(b"private value".len())
        .any(|v| v == b"private value"));
}

#[test]
fn reading_with_receipts_disabled_updates_unread_without_disclosing_a_receipt() {
    let (_dir, _server, mut alice, mut bob, now) = crate::claims::tests::pair();
    let (a, b) = crate::incoming::tests::trust(&mut alice, &mut bob);
    run(
        &mut alice,
        json!({"command":"post","peer":transport::hex(&b),"request":"45".repeat(32),"timestamp":now,"text":"Read this privately"}),
    );
    let mut sent = alice
        .resume_send_intents_online(conversations::now())
        .unwrap();
    if sent.is_empty() {
        sent = alice
            .resume_send_intents_online(conversations::now())
            .unwrap();
    }
    for value in sent {
        alice
            .send_pending_online(value.result.unwrap(), conversations::now())
            .unwrap();
    }
    bob.accept_delivery(&crate::incoming::tests::next(&bob))
        .unwrap();
    let peer = transport::hex(&a);
    let before = run(&mut bob, json!({"command":"state"}));
    assert_eq!(before["chats"][0]["unread"], 1);
    let alert = run(&mut bob, json!({"command":"notifications"}));
    assert_eq!(alert["unread"], 1);
    assert_eq!(alert.as_object().unwrap().len(), 3);
    run(
        &mut bob,
        json!({"command":"snooze","peer":peer,"seconds":3600,"request":"48".repeat(32),"timestamp":now}),
    );
    assert_eq!(
        run(&mut bob, json!({"command":"notifications"}))["unread"],
        0
    );
    run(
        &mut bob,
        json!({"command":"snooze","peer":peer,"seconds":null,"request":"49".repeat(32),"timestamp":now}),
    );
    assert_eq!(
        run(&mut bob, json!({"command":"notifications"}))["revision"],
        alert["revision"]
    );
    run(
        &mut bob,
        json!({"command":"organize","request":"46".repeat(32),"timestamp":now,"value":{"ReadReceipts":false}}),
    );
    let conversation = bob.direct_conversation(a).unwrap();
    let m = bob
        .recent_conversation_page(conversation, None, now)
        .unwrap()
        .messages
        .remove(0);
    run(
        &mut bob,
        json!({"command":"read","peer":peer,"request":"47".repeat(32),"timestamp":now,"author":transport::hex(&m.reference.author),"message":transport::hex(&m.reference.message)}),
    );
    let view = bob
        .conversation_message(conversation, m.reference, now)
        .unwrap();
    assert!(view.seen);
    assert!(view.read.is_empty());
    assert_eq!(
        run(&mut bob, json!({"command":"state"}))["chats"][0]["unread"],
        0
    );
}
