use super::*;
use crate::{
    claims::tests::pair,
    incoming::tests::{next, trust},
};
use sigil_crypto::Secret32;
use sigil_protocol::text::{parse, Limits};
fn configure(client: &mut ClientStore, secret: u8) {
    let account =
        crate::connection::decode_id(&client.connection_session().unwrap().unwrap().account_id)
            .unwrap();
    client
        .configure_recovery("chat.example", account, Secret32::from_bytes([secret; 32]))
        .unwrap();
}
fn count(client: &ClientStore, table: &str) -> i64 {
    client
        .db
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}
#[test]
fn prepared_random_content_is_committed_once_before_send_and_restarts_without_reroll() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_, b) = trust(&mut alice, &mut bob);
    let conversation = alice.direct_conversation(b).unwrap();
    let request = || SigilTextDraft {
        conversation,
        message: [111; 32],
        source: "roll::20d20; then pick::yesno;",
        created_at: now,
        timezone: Some("UTC"),
        date_order: None,
    };
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON structured_drafts BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(matches!(
        alice.prepare_sigiltext(request()),
        Err(Error::Storage(_))
    ));
    assert_eq!(count(&alice, "structured_drafts"), 0);
    alice.db.execute_batch("DROP TRIGGER fail").unwrap();
    let bytes = alice
        .prepare_sigiltext(request())
        .unwrap()
        .to_bytes()
        .unwrap();
    assert_eq!(
        bytes,
        alice
            .prepare_sigiltext(request())
            .unwrap()
            .to_bytes()
            .unwrap()
    );
    let stored: Vec<u8> = alice
        .db
        .query_row("SELECT content FROM structured_drafts", [], |r| r.get(0))
        .unwrap();
    assert!(!stored.windows(4).any(|v| v == b"roll"));
    drop(alice);
    let mut alice = ClientStore::open(
        &dir.path().join("alice.db"),
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap();
    let document = alice.prepare_sigiltext(request()).unwrap();
    assert_eq!(bytes, document.to_bytes().unwrap());
    assert!(matches!(
        alice.prepare_sigiltext(SigilTextDraft {
            source: "roll::d6;",
            ..request()
        }),
        Err(Error::Conflict)
    ));
    let sigil_protocol::text::Document::Composition(value) = document else {
        panic!();
    };
    alice.queue_peer_composition(b, &value, now).unwrap();
    alice
        .discard_sigiltext_draft(conversation, value.id)
        .unwrap();
    assert_eq!(count(&alice, "structured_drafts"), 0);
    assert_eq!(count(&alice, "send_intents"), 1);
}
#[test]
fn prepared_contact_bindings_do_not_follow_a_reassigned_handle() {
    use sigil_protocol::text::{contact::Contact, Document};
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (_, peer) = trust(&mut alice, &mut bob);
    let conversation = alice.direct_conversation(peer).unwrap();
    let request = || SigilTextDraft {
        conversation,
        message: [141; 32],
        source: "@::@user;",
        created_at: now,
        timezone: None,
        date_order: None,
    };
    let mut known = Contact {
        user_id: [1; 32],
        address: "@user:example.org".into(),
        display_name: Text::plain("Example", Default::default()).unwrap(),
        avatar_url: None,
    };
    let first = alice
        .prepare_sigiltext_with_contacts(request(), &[known.clone()])
        .unwrap()
        .to_bytes()
        .unwrap();
    known.user_id = [2; 32];
    let Document::Composition(value) = alice
        .prepare_sigiltext_with_contacts(request(), &[known])
        .unwrap()
    else {
        panic!();
    };
    assert_eq!(value.to_bytes().unwrap(), first);
    let sigil_protocol::text::structured::Construct::Contact(contact) =
        &value.cards().next().unwrap().content
    else {
        panic!();
    };
    assert_eq!(contact.user_id, [1; 32]);
}
fn text() -> Text {
    parse(
        "bold::é 👩🏽‍💻; redact::SYNTHETIC_SECRET; blue::visible;",
        Limits::default(),
    )
    .unwrap()
}
fn retained(client: &ClientStore, id: Id) -> Text {
    let sigil_crypto::recovery::Content::Rich(bytes) = client.recovery_record(id).unwrap().content
    else {
        panic!("lost SigilText recovery type")
    };
    Text::from_bytes(&bytes).unwrap()
}
#[test]
fn canonical_direct_delivery_is_atomic_restartable_and_recoverable_on_a_fresh_device() {
    let (dir, fixture, mut alice, mut bob, now) = pair();
    let (_, b) = trust(&mut alice, &mut bob);
    configure(&mut alice, 7);
    configure(&mut bob, 8);
    let text = text();
    alice
        .queue_peer_sigiltext(b, [61; 32], &text, now, now)
        .unwrap();
    alice
        .queue_peer_sigiltext(b, [61; 32], &text, now, now)
        .unwrap();
    assert!(matches!(
        alice.queue_peer_sigiltext(b, [61; 32], &text, now + 1, now),
        Err(Error::Conflict)
    ));
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON archive_records BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(matches!(
        alice
            .resume_send_intents_online(now)
            .unwrap()
            .remove(0)
            .result,
        Err(Error::Storage(_))
    ));
    assert_eq!(count(&alice, "sessions"), 0);
    alice.db.execute_batch("DROP TRIGGER fail").unwrap();
    let mut session = None;
    for _ in 0..2 {
        if let Some(attempt) = alice.resume_send_intents_online(now).unwrap().pop() {
            session = Some(attempt.result.unwrap());
            break;
        }
    }
    let session = session.unwrap();
    alice.send_pending_online(session, now).unwrap();
    let delivery = next(&bob);
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON archive_records BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(matches!(
        bob.accept_delivery(&delivery),
        Err(Error::Storage(_))
    ));
    assert_eq!(count(&bob, "sessions"), 0);
    bob.db.execute_batch("DROP TRIGGER fail").unwrap();
    let crate::MailboxEvent::SigilText(incoming) = bob
        .receive_mailbox_online(now)
        .unwrap()
        .remove(0)
        .result
        .unwrap()
    else {
        panic!("lost SigilText mailbox type")
    };
    assert!(incoming.text().is_err());
    let event = incoming.event().unwrap();
    assert!(event.content.rich().unwrap() == text);
    assert!(!std::str::from_utf8(event.content.bytes())
        .unwrap()
        .contains("SYNTHETIC_SECRET"));
    let id = crate::event_history_id(&event, &alice.identity().unwrap());
    assert!(retained(&alice, id) == text);
    assert!(retained(&bob, id) == text);
    assert!(bob.accept_delivery(&delivery).unwrap().duplicate);
    assert_eq!(count(&bob, "archive_records"), 1);
    let head = bob.prepare_recovery_upload(now).unwrap();
    assert_eq!(bob.upload_recovery_step().unwrap(), Some(head));
    let original = bob.connection_session().unwrap().unwrap();
    let mut server = sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap();
    let invite = server
        .invite_reauthorization(&original.account_id, 60, now)
        .unwrap();
    let path = dir.path().join("rich-recovered.db");
    let open = || {
        ClientStore::open(
            &path,
            StorageKey::new(Secret32::from_bytes([29; 32])).unwrap(),
        )
        .unwrap()
    };
    let mut restored = open();
    restored
        .prepare_enrollment(
            "chat.example",
            fixture.port(),
            &[crate::network::tests::CA.to_vec()],
            &invite.secret,
            "Synthetic recovered device",
            true,
        )
        .unwrap();
    let enrolled = restored.enroll_online().unwrap();
    assert_eq!(enrolled.account_id, original.account_id);
    assert_ne!(enrolled.device_id, original.device_id);
    configure(&mut restored, 8);
    let mut complete = false;
    for _ in 0..8 {
        if restored.download_recovery_step(true).unwrap() == Some(head) {
            complete = true;
            break;
        }
        drop(restored);
        restored = open();
    }
    assert!(complete);
    assert!(retained(&restored, id) == text);
    for table in ["sessions", "identity", "peers", "prekeys", "push_state"] {
        assert_eq!(count(&restored, table), 0, "{table}")
    }
    let mut tombstone = alice.recovery_record(id).unwrap();
    tombstone.revision = 2;
    tombstone.content = sigil_crypto::recovery::Content::Deleted;
    alice.retain_recovery_record(&tombstone).unwrap();
    assert!(matches!(
        alice.queue_peer_sigiltext(b, [61; 32], &text, now, now),
        Err(Error::Obsolete)
    ));
    drop(restored);
    let db = Connection::open(&path).unwrap();
    crate::test_schema::rewind(&db, 49);
    drop(db);
    let reopened = open();
    assert_eq!(
        reopened
            .db
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        i64::from(crate::DATABASE_VERSION)
    );
    assert!(retained(&reopened, id) == text);
}

#[test]
fn authenticated_but_invalid_rich_payload_does_not_commit_the_receiver_ratchet() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (a, b) = trust(&mut alice, &mut bob);
    crate::incoming::tests::start(&mut alice, b, now);
    let received = bob.accept_delivery(&next(&bob)).unwrap();
    let prior = received.event().unwrap();
    let text = text();
    let body = text.to_bytes().unwrap();
    let frame = sigil_protocol::event::Direct {
        message: [71; 32],
        conversation: prior.conversation,
        sender: prior.recipient,
        recipient: prior.sender,
        timestamp: now,
        content: Content::Rich(&body),
    };
    let mut bytes = frame.to_bytes().unwrap();
    let at = bytes.windows(8).position(|w| w == b"<strong>").unwrap();
    bytes[at..at + 8].copy_from_slice(b"<script>");
    let packet = bob.send(received.session, [71; 32], &bytes).unwrap();
    let state = |client: &ClientStore| {
        client
            .db
            .query_row("SELECT state FROM sessions", [], |r| r.get::<_, Vec<u8>>(0))
            .unwrap()
    };
    let before = state(&alice);
    let delivery = sigil_protocol::mailbox::Delivery {
        origin: None,
        sequence: 200,
        sender_device: bob.connection_session().unwrap().unwrap().device_id,
        message_id: crate::transport::hex(&[71; 32]),
        payload: crate::transport::hex(&packet),
        expires_at: now + 60,
    };
    assert!(matches!(
        alice.accept_delivery(&delivery),
        Err(Error::InvalidEvent)
    ));
    assert_eq!(state(&alice), before);
    assert_eq!(count(&alice, "incoming"), 0);
    configure(&mut bob, 8);
    let (_, packet) = bob
        .send_peer_sigiltext(a, [72; 32], &text, now, now)
        .unwrap();
    let good = sigil_protocol::mailbox::Delivery {
        origin: None,
        sequence: 201,
        message_id: crate::transport::hex(&[72; 32]),
        payload: crate::transport::hex(&packet),
        ..delivery
    };
    let accepted = alice.accept_delivery(&good).unwrap();
    let event = accepted.event().unwrap();
    assert!(event.content.rich().unwrap() == text);
    let id = crate::event_history_id(&event, &bob.identity().unwrap());
    let mut deleted = bob.recovery_record(id).unwrap();
    deleted.revision = 2;
    deleted.content = sigil_crypto::recovery::Content::Deleted;
    bob.retain_recovery_record(&deleted).unwrap();
    assert!(matches!(
        bob.send_peer_sigiltext(a, [72; 32], &text, now, now),
        Err(Error::Obsolete)
    ));
}

fn card(client: &mut ClientStore, message: Id, now: u64) -> sigil_protocol::text::structured::Card {
    let origin = sigil_protocol::text::Origin {
        message,
        creator: client.account_reference().unwrap(),
        created_at: now,
        timezone: None,
    };
    let sigil_protocol::text::Parsed::Card(card) = sigil_protocol::text::parse_card(
        "poll::closed::Choose\n- redact::SYNTHETIC_SECRET\n- Two;",
        origin,
        Default::default(),
    )
    .unwrap()
    .content
    else {
        panic!()
    };
    *card
}
#[test]
fn direct_cards_bind_the_creator_and_survive_queued_restart_and_recovery_storage() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_, b) = trust(&mut alice, &mut bob);
    configure(&mut alice, 7);
    configure(&mut bob, 8);
    let card = card(&mut alice, [73; 32], now);
    let mut forged = card.clone();
    forged.creator = bob.account_reference().unwrap();
    assert!(matches!(
        alice.queue_peer_card(b, &forged, now),
        Err(Error::InvalidEvent)
    ));
    assert_eq!(count(&alice, "sessions"), 0);
    alice.queue_peer_card(b, &card, now).unwrap();
    drop(alice);
    let mut alice = ClientStore::open(
        &dir.path().join("alice.db"),
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap();
    let session = alice
        .resume_send_intents_online(now)
        .unwrap()
        .remove(0)
        .result
        .unwrap();
    alice.send_pending_online(session, now).unwrap();
    let delivery = next(&bob);
    let incoming = bob.accept_delivery(&delivery).unwrap();
    let event = incoming.event().unwrap();
    assert!(event.content.card().unwrap() == card);
    assert!(event.content.rich().is_err());
    assert!(bob.accept_delivery(&delivery).unwrap().duplicate);
    let id = crate::event_history_id(&event, &alice.identity().unwrap());
    for client in [&alice, &bob] {
        let sigil_crypto::recovery::Content::Rich(bytes) =
            client.recovery_record(id).unwrap().content
        else {
            panic!()
        };
        assert!(sigil_protocol::text::structured::Card::from_bytes(&bytes).unwrap() == card);
        assert!(!String::from_utf8_lossy(&bytes).contains("SYNTHETIC_SECRET"));
    }
    drop(alice);
    let db = Connection::open(dir.path().join("alice.db")).unwrap();
    crate::test_schema::rewind(&db, 50);
    drop(db);
    let mut alice = ClientStore::open(
        &dir.path().join("alice.db"),
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap();
    assert_eq!(
        alice
            .db
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        i64::from(crate::DATABASE_VERSION)
    );
    alice.queue_peer_card(b, &card, now).unwrap();
}
#[test]
fn encrypted_cards_with_forged_origin_do_not_advance_receiver_state() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (_, b) = trust(&mut alice, &mut bob);
    crate::incoming::tests::start(&mut alice, b, now);
    let received = bob.accept_delivery(&next(&bob)).unwrap();
    let prior = received.event().unwrap();
    let checkpoint = |client: &ClientStore| {
        client
            .db
            .query_row("SELECT state FROM sessions", [], |r| r.get::<_, Vec<u8>>(0))
            .unwrap()
    };
    let before = checkpoint(&alice);
    for case in 0..3u8 {
        let id = [74 + case; 32];
        let mut card = card(&mut bob, id, now);
        match case {
            0 => card.creator = alice.account_reference().unwrap(),
            1 => card.id = [99; 32],
            _ => card.created_at += 1,
        }
        let body = card.to_bytes().unwrap();
        let bytes = sigil_protocol::event::Direct {
            message: id,
            conversation: prior.conversation,
            sender: prior.recipient,
            recipient: prior.sender,
            timestamp: now,
            content: Content::Rich(&body),
        }
        .to_bytes()
        .unwrap();
        let packet = bob.send(received.session, id, &bytes).unwrap();
        let delivery = sigil_protocol::mailbox::Delivery {
            origin: None,
            sequence: 300 + i64::from(case),
            sender_device: bob.connection_session().unwrap().unwrap().device_id,
            message_id: crate::transport::hex(&id),
            payload: crate::transport::hex(&packet),
            expires_at: now + 60,
        };
        assert!(matches!(
            alice.accept_delivery(&delivery),
            Err(Error::InvalidEvent)
        ));
        assert_eq!(checkpoint(&alice), before);
        assert_eq!(count(&alice, "incoming"), 0);
    }
    // Account references deliberately exclude device keys/IDs, but include server/account.
    let mut binding = peers::parse(&bob.own_device_binding().unwrap())
        .unwrap()
        .binding;
    let reference = crate::event::account(&binding);
    binding.device = [98; 32];
    binding.identity = [97; 32];
    assert_eq!(crate::event::account(&binding), reference);
    binding.server = "other.example".into();
    assert_ne!(crate::event::account(&binding), reference);
}
