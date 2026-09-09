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
fn unknown_contacts_require_acceptance_and_verification_after_restart() {
    let (dir, _fixture, mut alice, mut bob, _) = crate::claims::tests::pair();
    let server = rusqlite::Connection::open(dir.path().join("server.db")).unwrap();
    server.execute("DELETE FROM allowed_senders", []).unwrap();
    alice.publish_device_binding_online().unwrap();
    bob.publish_device_binding_online().unwrap();
    let found = alice.mobile_find("@bob:chat.example").unwrap();
    let target = found["chats"][0]["id"].as_str().unwrap().to_owned();
    assert!(target.starts_with("dm:"));
    assert_eq!(found["chats"][0]["contact_only"], true);
    assert_eq!(found["chats"][0]["devices"], json!([]));
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
    // A failed intent write must have no server side effect.
    bob.db.execute_batch("CREATE TRIGGER fail_decision BEFORE UPDATE ON mobile_contacts WHEN (SELECT state FROM mobile_contacts WHERE id=OLD.id) != NEW.state AND (SELECT count(*) FROM mobile_contacts)>0 BEGIN SELECT RAISE(ABORT,'synthetic'); END").unwrap();
    assert!(bob.mobile_request(&source, "accept").is_err());
    assert_eq!(
        server
            .query_row("SELECT state FROM contact_requests", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
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
    assert_eq!(count(&server, "allowed_senders"), 0);
    let peer = bob.peer(bob.mobile_peer(&source).unwrap()).unwrap();
    bob.mobile_execute(Command::Confirm {
        peer: transport::hex(&peer.id),
        fingerprint: peer.fingerprint.map(|b| format!("{b:02x}")).concat(),
    })
    .unwrap();
    alice.mobile_request(&target, "refresh").unwrap();
    assert_eq!(alice.mobile_conversation(&target).unwrap(), conversation);
    let state = alice.mobile_state().unwrap();
    assert_eq!(state["chats"].as_array().unwrap().len(), 1);
    assert_eq!(state["chats"][0]["id"], target);
    assert_eq!(state["chats"][0]["verified"], false);
    let peer = alice.peer(alice.mobile_peer(&target).unwrap()).unwrap();
    alice
        .mobile_execute(Command::Confirm {
            peer: transport::hex(&peer.id),
            fingerprint: transport::hex(&peer.fingerprint),
        })
        .unwrap();
    assert_eq!(alice.mobile_state().unwrap()["chats"][0]["verified"], true);
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
    assert!(!bob.mobile_state().unwrap()["chats"][0]["verified"]
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
    alice.mobile_contact_sync(true).unwrap();
    assert_eq!(alice.mobile_state().unwrap()["chats"][0]["id"], display);
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
