use super::*;
fn open(path: &std::path::Path) -> ClientStore {
    ClientStore::open(
        path,
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap()
}
fn count(db: &rusqlite::Connection, table: &str) -> i64 {
    db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}
#[test]
fn shared_contact_action_binds_the_account_without_sending_a_request_or_accepting_replacement() {
    use sigil_protocol::text::{
        contact::Contact as SharedContact,
        structured::{Card, Construct},
        Text,
    };
    let (dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    let server = rusqlite::Connection::open(dir.path().join("server.db")).unwrap();
    server.execute("DELETE FROM allowed_senders", []).unwrap();
    alice.publish_device_binding_online().unwrap();
    bob.publish_device_binding_online().unwrap();
    let author = alice.account_reference().unwrap();
    for (number, identity, view_once) in [
        (71u8, [7; 32], false),
        (72, bob.account_reference().unwrap(), false),
        (73, bob.account_reference().unwrap(), true),
    ] {
        let card = Card {
            id: [number; 32],
            creator: author,
            created_at: now,
            content: Construct::Contact(SharedContact {
                user_id: identity,
                address: "@bob:chat.example".into(),
                display_name: Text::plain("Synthetic Bob", Default::default()).unwrap(),
                avatar_url: Some("https://external.example/avatar.png".into()),
            }),
        };
        alice
            .mobile_action(
                "self",
                &transport::hex(&card.id),
                now,
                Action::Post {
                    body: Body::Rich(card.to_bytes().unwrap()),
                    reply: None,
                    thread: None,
                    expires_at: None,
                    view_once,
                },
            )
            .unwrap();
        if !view_once {
            let timeline: Value = serde_json::from_str(
                &alice.mobile_command(r#"{"command":"timeline","peer":"self"}"#),
            )
            .unwrap();
            assert_eq!(timeline["ok"], true);
            let part = &timeline["value"]["messages"][0]["parts"][0];
            assert_eq!(part["kind"], "contact");
            assert_eq!(part["contact"]["identity"], transport::hex(&identity));
            assert!(!part.to_string().contains("external.example"));
        }
        let target = Reference {
            author,
            message: card.id,
        };
        let result = alice.mobile_open_contact_card("self", target, card.id);
        match number {
            71 => {
                assert!(matches!(result, Err(Error::SharedContactChanged)));
                assert_eq!(count(&alice.db, "mobile_contacts"), 0);
            }
            72 => {
                let value = result.unwrap();
                assert!(value["open"].as_str().unwrap().starts_with("dm:"));
                assert_eq!(count(&alice.db, "mobile_contacts"), 1);
            }
            _ => assert!(matches!(result, Err(Error::Obsolete))),
        }
        assert_eq!(count(&server, "contact_requests"), 0);
        assert_eq!(count(&server, "allowed_senders"), 0);
    }
}
#[test]
fn directory_contacts_require_acceptance_and_preserve_trust_after_restart() {
    let (dir, _fixture, mut alice, mut bob, _) = crate::claims::tests::pair();
    let server = rusqlite::Connection::open(dir.path().join("server.db")).unwrap();
    server.execute("DELETE FROM allowed_senders", []).unwrap();
    alice.publish_device_binding_online().unwrap();
    bob.publish_device_binding_online().unwrap();
    let found = alice.mobile_find("@bob:chat.example").unwrap();
    let target = found["chats"][0]["id"].as_str().unwrap().to_owned();
    assert!(target.starts_with("dm:"));
    assert_eq!(found["chats"][0]["contact_only"], true);
    assert_eq!(found["chats"][0]["devices"].as_array().unwrap().len(), 1);
    let conversation = alice.mobile_conversation(&target).unwrap();
    assert!(alice
        .mobile_action(
            &target,
            &"01".repeat(32),
            conversations::now(),
            Action::Post {
                body: Body::Text("Synthetic unsent draft".into()),
                reply: None,
                thread: None,
                expires_at: None,
                view_once: false
            }
        )
        .is_err());
    assert_eq!(count(&server, "contact_requests"), 0);
    let requested = alice.mobile_request(&target, "send").unwrap();
    assert_eq!(requested["chats"][0]["request"], "pending");
    let saved = alice.contact_for(&target).unwrap().outgoing.unwrap();
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    alice.mobile_request(&target, "send").unwrap();
    assert_eq!(alice.contact_for(&target).unwrap().outgoing.unwrap(), saved);
    assert_eq!(count(&server, "contact_requests"), 1);
    bob.mobile_contact_sync(true).unwrap();
    let state = bob.mobile_state().unwrap();
    let source = state["chats"][0]["id"].as_str().unwrap().to_owned();
    assert_eq!(state["chats"][0]["request"], "incoming");
    assert_eq!(state["chats"][0]["contact_only"], false);
    assert_eq!(state["chats"][0]["verified"], false);
    assert_eq!(count(&server, "allowed_senders"), 0);
    let declined = bob.mobile_request(&source, "decline").unwrap();
    assert_eq!(declined["chats"][0]["request"], "declined_incoming");
    assert_eq!(
        alice.mobile_request(&target, "refresh").unwrap()["chats"][0]["request"],
        "declined"
    );
    assert_eq!(
        alice.mobile_request(&target, "send").unwrap()["chats"][0]["request"],
        "declined"
    );
    assert_eq!(count(&server, "allowed_senders"), 0);
    // A failed intent write must have no server side effect.
    let mut legacy = alice.contact_for(&target).unwrap();
    legacy.work_at = waiting();
    alice.save_contact(&legacy).unwrap();
    alice
        .db
        .execute_batch("DROP TABLE mobile_contact_invite; PRAGMA user_version=75;")
        .unwrap();
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    assert_eq!(alice.contact_for(&target).unwrap().work_at, 0);
    bob.db.execute_batch("CREATE TRIGGER fail_decision BEFORE UPDATE ON mobile_contacts WHEN (SELECT state FROM mobile_contacts WHERE id=OLD.id) != NEW.state AND (SELECT count(*) FROM mobile_contacts)>0 BEGIN SELECT RAISE(ABORT,'synthetic'); END").unwrap();
    assert!(bob.mobile_request(&source, "accept").is_err());
    assert_eq!(
        server
            .query_row("SELECT state FROM contact_requests", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    bob.db.execute_batch("DROP TRIGGER fail_decision").unwrap();
    let mut contact = bob.contact_for(&source).unwrap();
    contact.decision = Some(RequestState::Accepted);
    contact.work_at = conversations::now();
    bob.save_contact(&contact).unwrap();
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    bob.mobile_contact_sync(true).unwrap();
    assert_eq!(
        bob.mobile_state().unwrap()["chats"][0]["request"],
        "accepted"
    );
    assert_eq!(count(&server, "allowed_senders"), 1);
    let peer = bob.peer(bob.mobile_peer(&source).unwrap()).unwrap();
    assert!(peer.trusted);
    assert!(!peer.verified);
    alice.mobile_contact_sync(true).unwrap();
    assert_eq!(alice.mobile_conversation(&target).unwrap(), conversation);
    let state = alice.mobile_state().unwrap();
    assert_eq!(state["chats"].as_array().unwrap().len(), 1);
    assert_eq!(state["chats"][0]["id"], target);
    assert_eq!(state["chats"][0]["verified"], true);
    let peer = alice.peer(alice.mobile_peer(&target).unwrap()).unwrap();
    assert!(peer.trusted);
    assert!(!peer.verified);
    assert_eq!(count(&server, "allowed_senders"), 2);
    let now = conversations::now();
    alice
        .mobile_action(
            &target,
            &"02".repeat(32),
            now,
            Action::Post {
                body: Body::Text("A synthetic first letter".into()),
                reply: None,
                thread: None,
                expires_at: None,
                view_once: false,
            },
        )
        .unwrap();
    for _ in 0..2 {
        for attempt in alice.resume_send_intents_online(now).unwrap() {
            alice
                .send_pending_online(attempt.result.unwrap(), now)
                .unwrap();
        }
    }
    let received = bob.receive_mailbox_online(now).unwrap();
    assert!(!received.is_empty());
    for packet in received {
        packet.result.unwrap();
    }
    assert!(bob
        .recent_conversation_page(conversation, None, now)
        .unwrap()
        .messages
        .iter()
        .any(|m| m.body == Some(Body::Text("A synthetic first letter".into()))));
    let encoded: String = server
        .query_row("SELECT payload FROM mailbox LIMIT 1", [], |r| r.get(0))
        .unwrap();
    let payload: Vec<u8> = (0..encoded.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&encoded[i..i + 2], 16).unwrap())
        .collect();
    assert!(!payload
        .windows(24)
        .any(|w| w == b"A synthetic first letter"));
    bob.mobile_block(&source, true).unwrap();
    assert!(bob
        .mobile_recipients(bob.mobile_peer(&source).unwrap())
        .is_err());
    assert_eq!(
        bob.mobile_state().unwrap()["chats"][0]["request"],
        "blocked"
    );
    bob.mobile_block(&source, false).unwrap();
    assert!(bob
        .mobile_recipients(bob.mobile_peer(&source).unwrap())
        .is_ok());
}
#[test]
fn denied_first_message_survives_until_recipient_repairs_acceptance() {
    let (dir, _fixture, mut alice, mut bob, _) = crate::claims::tests::pair();
    let server = rusqlite::Connection::open(dir.path().join("server.db")).unwrap();
    server.execute("DELETE FROM allowed_senders", []).unwrap();
    alice.publish_device_binding_online().unwrap();
    bob.publish_device_binding_online().unwrap();
    let found = alice.mobile_find("@bob:chat.example").unwrap();
    let target = found["chats"][0]["id"].as_str().unwrap().to_owned();
    alice.mobile_request(&target, "send").unwrap();
    bob.mobile_contact_sync(true).unwrap();
    let source = bob.mobile_state().unwrap()["chats"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    server.execute_batch("CREATE TRIGGER deny_permission BEFORE INSERT ON allowed_senders BEGIN SELECT RAISE(ABORT,'synthetic permission write failure'); END;").unwrap();
    assert!(bob.mobile_request(&source, "accept").is_err());
    assert!(bob.contact_for(&source).unwrap().accepted());
    server
        .execute_batch("DROP TRIGGER deny_permission")
        .unwrap();
    alice.mobile_request(&target, "refresh").unwrap();
    let message = "Synthetic queued letter";
    let now = conversations::now();
    alice
        .mobile_action(
            &target,
            &"31".repeat(32),
            now,
            Action::Post {
                body: Body::Text(message.into()),
                reply: None,
                thread: None,
                expires_at: None,
                view_once: false,
            },
        )
        .unwrap();
    let attempt = alice.resume_send_intents_online(now).unwrap().remove(0);
    assert!(
        matches!(
            attempt.result,
            Err(Error::Network(network::Error::Status { code: 403, .. }))
        ),
        "{:?}",
        attempt.result
    );
    assert_eq!(count(&alice.db, "send_intents"), 1);
    assert_eq!(count(&server, "mailbox"), 0);
    let mut contact = bob.contact_for(&source).unwrap();
    contact.work_at = 0;
    bob.save_contact(&contact).unwrap();
    bob.db
        .execute("UPDATE mobile_contact_poll SET next_at=0", [])
        .unwrap();
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    let fingerprint = device_fingerprint(&bob.own_device_binding().unwrap()).unwrap();
    let (mut schedule, _) = crate::schedule::read(&bob.db, &bob.key, &fingerprint).unwrap();
    schedule.last = conversations::now();
    schedule.next = schedule.last + 300;
    schedule.failures = 7;
    let tx = bob.db.transaction().unwrap();
    crate::schedule::write(&tx, &bob.key, &fingerprint, &schedule).unwrap();
    tx.commit().unwrap();
    let result: Value =
        serde_json::from_str(&bob.mobile_command(r#"{"command":"sync","interactive":true}"#))
            .unwrap();
    assert_eq!(result["ok"], true);
    assert_eq!(result["value"]["ran"], false);
    assert!(result["value"]["next_at"].as_u64().unwrap() <= conversations::now() + 60);
    assert_eq!(
        crate::schedule::read(&bob.db, &bob.key, &fingerprint)
            .unwrap()
            .0
            .next,
        schedule.next
    );
    for _ in 0..2 {
        for attempt in alice.resume_send_intents_online(now).unwrap() {
            alice
                .send_pending_online(attempt.result.unwrap(), now)
                .unwrap();
        }
    }
    assert_eq!(count(&alice.db, "send_intents"), 0);
    for incoming in bob.receive_mailbox_online(now).unwrap() {
        incoming.result.unwrap();
    }
    let conversation = bob.mobile_conversation(&source).unwrap();
    assert!(bob
        .recent_conversation_page(conversation, None, now)
        .unwrap()
        .messages
        .iter()
        .any(|value| value.body == Some(Body::Text(message.into()))));
}
#[test]
fn altered_requests_never_create_peers_or_transfer_trust() {
    let (dir, _fixture, mut alice, mut bob, _) = crate::claims::tests::pair();
    alice.publish_device_binding_online().unwrap();
    bob.publish_device_binding_online().unwrap();
    let state = alice.mobile_find("@bob:chat.example").unwrap();
    let target = state["chats"][0]["id"].as_str().unwrap();
    alice.mobile_request(target, "send").unwrap();
    let incoming = bob
        .connected_client()
        .unwrap()
        .contact_requests(None)
        .unwrap()
        .requests
        .remove(0);
    for field in 0..6 {
        let mut bad = incoming.clone();
        match field {
            0 => bad.account = "ab".repeat(32),
            1 => bad.origin = "other.example".into(),
            2 => bad.device = "ab".repeat(32),
            3 => bad.receipt.expires_at -= 1,
            4 => bad.signature = "00".repeat(64),
            _ => bad.binding.replace_range(0..2, "ff"),
        }
        assert!(bob.receive_contact(bad, conversations::now()).is_err());
        assert_eq!(count(&bob.db, "peers"), 0);
        assert_eq!(count(&bob.db, "mobile_contacts"), 0);
    }
    bob.receive_contact(incoming, conversations::now()).unwrap();
    let state = bob.mobile_state().unwrap();
    assert_eq!(state["chats"][0]["verified"], false);
    let key = bob
        .contact_for(state["chats"][0]["id"].as_str().unwrap())
        .unwrap()
        .id();
    assert!(!std::fs::read(dir.path().join("bob.db"))
        .unwrap()
        .windows(13)
        .any(|b| b == b"@alice:chat.ex"));
    bob.db
        .execute(
            "UPDATE mobile_contacts SET work_at=1 WHERE id=?1",
            [key.as_slice()],
        )
        .unwrap();
    assert!(matches!(bob.contact(key), Err(Error::InvalidStore)));
}
#[test]
fn acknowledged_decision_retries_after_its_local_commit_fails() {
    let (dir, _fixture, mut alice, mut bob, _) = crate::claims::tests::pair();
    alice.publish_device_binding_online().unwrap();
    bob.publish_device_binding_online().unwrap();
    let found = alice.mobile_find("@bob:chat.example").unwrap();
    alice
        .mobile_request(found["chats"][0]["id"].as_str().unwrap(), "send")
        .unwrap();
    bob.mobile_contact_sync(true).unwrap();
    let state = bob.mobile_state().unwrap();
    let source = state["chats"][0]["id"].as_str().unwrap().to_owned();
    bob.db.execute_batch("CREATE TABLE synthetic_fault(remaining INTEGER); INSERT INTO synthetic_fault VALUES(3); CREATE TRIGGER fail_ack BEFORE UPDATE ON mobile_contacts BEGIN UPDATE synthetic_fault SET remaining=remaining-1; SELECT CASE WHEN (SELECT remaining FROM synthetic_fault)=0 THEN RAISE(ABORT,'synthetic') END; END;").unwrap();
    assert!(bob.mobile_request(&source, "accept").is_err());
    let server = rusqlite::Connection::open(dir.path().join("server.db")).unwrap();
    assert_eq!(
        server
            .query_row("SELECT state FROM contact_requests", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        bob.contact_for(&source).unwrap().decision,
        Some(RequestState::Accepted)
    );
    bob.db
        .execute_batch("DROP TRIGGER fail_ack; DROP TABLE synthetic_fault")
        .unwrap();
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    bob.mobile_request(&source, "refresh").unwrap();
    assert!(bob.contact_for(&source).unwrap().decision.is_none());
    assert_eq!(count(&bob.db, "peers"), 1);
    assert!(bob.mobile_state().unwrap()["chats"][0]["verified"]
        .as_bool()
        .unwrap());
}
#[test]
fn requests_preserve_existing_conversation_references_and_cached_decisions_refresh() {
    let (_dir, _fixture, mut alice, mut bob, _) = crate::claims::tests::pair();
    let (_, peer) = crate::incoming::tests::trust(&mut alice, &mut bob);
    alice.publish_device_binding_online().unwrap();
    bob.publish_device_binding_online().unwrap();
    let before = alice.mobile_state().unwrap();
    let display = transport::hex(&peer);
    assert_eq!(before["chats"][0]["id"], display);
    assert_eq!(
        alice.mobile_find("@bob:chat.example").unwrap()["chats"][0]["id"],
        display
    );
    let found = bob.mobile_find("@alice:chat.example").unwrap();
    bob.mobile_request(found["chats"][0]["id"].as_str().unwrap(), "send")
        .unwrap();
    assert_eq!(
        bob.mobile_state().unwrap()["chats"][0]["request"],
        "pending"
    );
    assert_eq!(bob.mobile_state().unwrap()["chats"][0]["verified"], false);
    alice.mobile_contact_sync(true).unwrap();
    assert_eq!(alice.mobile_state().unwrap()["chats"][0]["id"], display);
    assert_eq!(
        alice.mobile_state().unwrap()["chats"][0]["request"],
        "incoming"
    );
    assert_eq!(alice.mobile_state().unwrap()["chats"][0]["verified"], false);
    assert!(matches!(
        alice.mobile_recipients(peer),
        Err(Error::Unprepared)
    ));
    assert!(matches!(
        alice.mobile_action(
            &display,
            &"72".repeat(32),
            conversations::now(),
            Action::Post {
                body: Body::Text("Must remain unsent".into()),
                reply: None,
                thread: None,
                expires_at: None,
                view_once: false,
            }
        ),
        Err(Error::Unprepared)
    ));
    assert_eq!(count(&alice.db, "send_intents"), 0);
    let incoming = alice.contact_for(&display).unwrap().incoming.unwrap();
    alice
        .connected_client()
        .unwrap()
        .resolve_contact_request(
            &incoming.receipt.id,
            RequestState::Declined,
            &incoming.signature,
        )
        .unwrap();
    alice.mobile_request(&display, "refresh").unwrap();
    assert_eq!(
        alice
            .contact_for(&display)
            .unwrap()
            .incoming
            .unwrap()
            .receipt
            .state,
        RequestState::Declined
    );
}

#[test]
fn mobile_contact_builder_checks_identity_without_sending_a_request() {
    let (dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    alice.publish_device_binding_online().unwrap();
    bob.publish_device_binding_online().unwrap();
    let invoke = |store: &mut ClientStore, value: Value| -> Value {
        serde_json::from_str(&store.mobile_command(&value.to_string())).unwrap()
    };
    let preview = invoke(
        &mut alice,
        json!({"command":"contact_preview","address":"@bob:chat.example"}),
    );
    assert_eq!(preview["ok"], true, "{preview}");
    let contact = preview["value"]["contact"].clone();
    let posted = invoke(
        &mut alice,
        json!({"command":"post","peer":"self","request":"c8".repeat(32),"timestamp":now,"text":"Meet Bob","rich":true,"shared_contact":contact}),
    );
    assert_eq!(posted["ok"], true, "{posted}");
    let timeline = invoke(&mut alice, json!({"command":"timeline","peer":"self"}));
    assert_eq!(
        timeline["value"]["messages"][0]["parts"][0]["kind"],
        "contact"
    );
    assert_eq!(
        timeline["value"]["messages"][0]["parts"][1]["text"],
        "Meet Bob"
    );
    let mut wrong = preview["value"]["contact"].clone();
    wrong["user_id"] = json!("01".repeat(32));
    let rejected = invoke(
        &mut alice,
        json!({"command":"post","peer":"self","request":"c9".repeat(32),"timestamp":now,"text":"Must not send","rich":true,"shared_contact":wrong}),
    );
    assert_eq!(rejected["ok"], false);
    let server = rusqlite::Connection::open(dir.path().join("server.db")).unwrap();
    assert_eq!(count(&server, "contact_requests"), 0);
}

#[test]
fn linking_recovers_an_existing_outgoing_conversation() {
    let (dir, fixture, mut phone, mut bob, now) = crate::claims::tests::pair();
    phone.publish_device_binding_online().unwrap();
    bob.publish_device_binding_online().unwrap();
    let found = phone.mobile_find("@bob:chat.example").unwrap();
    let target = found["chats"][0]["id"].as_str().unwrap().to_owned();
    phone.mobile_request(&target, "send").unwrap();
    bob.mobile_contact_sync(true).unwrap();
    let source = bob.mobile_state().unwrap()["chats"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    bob.mobile_request(&source, "accept").unwrap();
    phone.mobile_request(&target, "refresh").unwrap();
    phone
        .mobile_action(
            &target,
            &"81".repeat(32),
            now,
            Action::Post {
                body: Body::Text("Before linking".into()),
                reply: None,
                thread: None,
                expires_at: None,
                view_once: false,
            },
        )
        .unwrap();
    let mut browser = open(&dir.path().join("browser.db"));
    let offer = browser.prepare_device_link_offer([82; 32], now).unwrap();
    let (proposal, digest) = phone
        .prepare_sponsored_link([83; 32], &crate::link::offer_qr(&offer).unwrap(), now)
        .unwrap();
    browser
        .accept_link_proposal([82; 32], &proposal, now)
        .unwrap();
    let response = browser
        .confirm_link_proposal([82; 32], digest, now)
        .unwrap();
    phone
        .confirm_sponsored_link([83; 32], &response, digest, now)
        .unwrap();
    phone.authorize_sponsored_link_online([83; 32]).unwrap();
    browser
        .finish_device_link_online(
            [82; 32],
            fixture.port(),
            &[crate::network::tests::CA.to_vec()],
        )
        .unwrap();
    browser
        .prepare_prekey_publication([84; 32], true, 3600)
        .unwrap();
    browser.publish_prekey_online([84; 32]).unwrap();
    for _ in 0..8 {
        let now = conversations::now();
        phone.sync_conversation_devices(now).unwrap();
        for s in phone.resume_send_intents_online(now).unwrap() {
            phone.send_pending_online(s.result.unwrap(), now).unwrap();
        }
        for s in phone.resume_outbound_online(now).unwrap() {
            s.result.unwrap();
        }
        for received in browser.receive_mailbox_online(now).unwrap() {
            received.result.unwrap();
        }
        browser.acknowledge_incoming_online().unwrap();
        browser.mobile_contact_sync(true).unwrap();
    }
    let state = browser.mobile_state().unwrap();
    assert!(
        state["chats"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["address"] == "@bob:chat.example"),
        "Linked browser lacks the existing conversation"
    );
    let chat = state["chats"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["address"] == "@bob:chat.example")
        .unwrap();
    let conversation = browser
        .mobile_conversation(chat["id"].as_str().unwrap())
        .unwrap();
    let browser_target = chat["id"].as_str().unwrap().to_owned();
    let peer = browser
        .peer(browser.mobile_peer(&browser_target).unwrap())
        .unwrap();
    assert!(peer.trusted);
    assert!(
        !peer.verified,
        "Directory trust must not become independent verification"
    );
    assert_eq!(
        peer.fingerprint,
        device_fingerprint(&bob.own_device_binding().unwrap()).unwrap()
    );
    assert!(browser
        .recent_conversation_page(conversation, None, now)
        .unwrap()
        .messages
        .iter()
        .any(|m| m.body == Some(Body::Text("Before linking".into()))));
    bob.mobile_request(&source, "refresh").unwrap();
    bob.prepare_prekey_publication([85; 32], true, 3600)
        .unwrap();
    bob.publish_prekey_online([85; 32]).unwrap();
    browser
        .mobile_action(
            &browser_target,
            &"86".repeat(32),
            conversations::now(),
            Action::Post {
                body: Body::Text("From the linked browser".into()),
                reply: None,
                thread: None,
                expires_at: None,
                view_once: false,
            },
        )
        .unwrap();
    for _ in 0..4 {
        let now = conversations::now();
        for sent in browser.resume_send_intents_online(now).unwrap() {
            browser
                .send_pending_online(sent.result.unwrap(), now)
                .unwrap();
        }
        for sent in browser.resume_outbound_online(now).unwrap() {
            sent.result.unwrap();
        }
        for received in bob.receive_mailbox_online(now).unwrap() {
            received.result.unwrap();
        }
        bob.acknowledge_incoming_online().unwrap();
    }
    assert!(bob
        .recent_conversation_page(conversation, None, conversations::now())
        .unwrap()
        .messages
        .iter()
        .any(|m| m.body == Some(Body::Text("From the linked browser".into()))));
    let now = conversations::now();
    let mut tablet = open(&dir.path().join("tablet.db"));
    let offer = tablet.prepare_device_link_offer([87; 32], now).unwrap();
    let (proposal, digest) = browser
        .prepare_sponsored_link([88; 32], &crate::link::offer_qr(&offer).unwrap(), now)
        .unwrap();
    tablet
        .accept_link_proposal([87; 32], &proposal, now)
        .unwrap();
    let response = tablet.confirm_link_proposal([87; 32], digest, now).unwrap();
    browser
        .confirm_sponsored_link([88; 32], &response, digest, now)
        .unwrap();
    browser.authorize_sponsored_link_online([88; 32]).unwrap();
    tablet
        .finish_device_link_online(
            [87; 32],
            fixture.port(),
            &[crate::network::tests::CA.to_vec()],
        )
        .unwrap();
    tablet
        .prepare_prekey_publication([89; 32], true, 3600)
        .unwrap();
    tablet.publish_prekey_online([89; 32]).unwrap();
    for _ in 0..8 {
        let now = conversations::now();
        browser.sync_conversation_devices(now).unwrap();
        for sent in browser.resume_send_intents_online(now).unwrap() {
            browser
                .send_pending_online(sent.result.unwrap(), now)
                .unwrap();
        }
        for sent in browser.resume_outbound_online(now).unwrap() {
            sent.result.unwrap();
        }
        for received in tablet.receive_mailbox_online(now).unwrap() {
            received.result.unwrap();
        }
        tablet.acknowledge_incoming_online().unwrap();
        tablet.mobile_contact_sync(true).unwrap();
    }
    assert!(
        tablet.contact_for(&browser_target).unwrap().accepted(),
        "An imported contact must carry over when that device sponsors another link"
    );
    assert_eq!(
        tablet
            .peer(tablet.mobile_peer(&browser_target).unwrap())
            .unwrap()
            .fingerprint,
        peer.fingerprint
    );
    browser.mobile_request(&browser_target, "block").unwrap();
    let known = phone.mobile_peer(&target).unwrap();
    phone.confirm_peer(known, peer.fingerprint).unwrap();
    phone
        .save_contact(&phone.contact_for(&target).unwrap())
        .unwrap();
    for _ in 0..4 {
        let now = conversations::now();
        phone.sync_conversation_devices(now).unwrap();
        for sent in phone.resume_send_intents_online(now).unwrap() {
            phone
                .send_pending_online(sent.result.unwrap(), now)
                .unwrap();
        }
        for sent in phone.resume_outbound_online(now).unwrap() {
            sent.result.unwrap();
        }
        for received in browser.receive_mailbox_online(now).unwrap() {
            received.result.unwrap();
        }
        browser.acknowledge_incoming_online().unwrap();
    }
    assert!(
        browser.contact_for(&browser_target).unwrap().blocked,
        "A later catalog snapshot must not undo a local block"
    );
    assert!(!browser.peer(peer.id).unwrap().trusted);
}
#[test]
fn a_recovered_device_lists_every_accepted_contact_without_searching() {
    let (dir, fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    alice.after_sign_in().unwrap();
    bob.after_sign_in().unwrap();
    let found = alice.mobile_find("@bob:chat.example").unwrap();
    let target = found["chats"][0]["id"].as_str().unwrap().to_owned();
    alice.mobile_request(&target, "send").unwrap();
    bob.mobile_contact_sync(true).unwrap();
    let source = bob.mobile_state().unwrap()["chats"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    bob.mobile_request(&source, "accept").unwrap();
    bob.mobile_contact_sync(true).unwrap();
    alice.mobile_request(&target, "refresh").unwrap();
    alice.mobile_contact_sync(true).unwrap();
    assert!(alice.contact_for(&target).unwrap().accepted());
    let head = alice.prepare_recovery_upload(now).unwrap();
    let mut uploaded = false;
    for _ in 0..64 {
        if alice.upload_recovery_step().unwrap() == Some(head) {
            uploaded = true;
            break;
        }
    }
    assert!(uploaded);
    let code = alice.recovery_code().unwrap();
    let account = alice.connection_session().unwrap().unwrap().account_id;
    let mut server = sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap();
    let invite = server.invite_reauthorization(&account, 60, now).unwrap();
    let mut laptop = open(&dir.path().join("laptop.db"));
    laptop
        .prepare_enrollment(
            "chat.example",
            fixture.port(),
            &[crate::network::tests::CA.to_vec()],
            &invite.secret,
            "Synthetic laptop",
            true,
        )
        .unwrap();
    laptop.enroll_online().unwrap();
    laptop.recover_with_code_online(&code).unwrap();
    let mut restored = false;
    for _ in 0..64 {
        if laptop.download_recovery_step(true).unwrap() == Some(head) {
            restored = true;
            break;
        }
    }
    assert!(restored);
    let chats = laptop.mobile_state().unwrap()["chats"].clone();
    assert!(
        chats
            .as_array()
            .unwrap()
            .iter()
            .any(|chat| chat["address"] == "@bob:chat.example"),
        "{chats}"
    );
}
