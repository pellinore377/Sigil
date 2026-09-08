use super::*;
use crate::claims::tests::pair;
use crate::incoming::tests::{next, trust};
fn post(text: &str) -> Action {
    Action::Post {
        body: Body::Text(text.into()),
        reply: None,
        thread: None,
        expires_at: None,
        view_once: false,
    }
}

#[test]
fn deleted_bodies_are_compacted_without_reviving_on_replay() {
    let (_dir, _fixture, mut a, _b, now) = pair();
    a.enable_history_recovery().unwrap();
    let author = account(&mut a);
    let original = a
        .conversation_operation([220; 32], post("secret to remove"))
        .unwrap();
    let conversation = a.note_to_self(&original, now, now).unwrap();
    let target = Reference {
        author,
        message: original.id,
    };
    let remove = a
        .conversation_operation(
            [221; 32],
            Action::Delete {
                target: target.clone(),
            },
        )
        .unwrap();
    a.note_to_self(&remove, now, now).unwrap();
    a.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON conversation_removed BEGIN SELECT RAISE(ABORT,'synthetic'); END").unwrap();
    assert!(matches!(a.maintain_history(now), Err(Error::Storage(_))));
    a.db.execute_batch("DROP TRIGGER fail").unwrap();
    assert_eq!(a.maintain_history(now).unwrap().redacted, 1);
    let entry = super::original(&a.db, &a.key, &conversation, &target)
        .unwrap()
        .unwrap();
    assert!(
        matches!(entry.operation.action, Action::Post { body:Body::Text(ref v), .. } if v==" ")
    );
    assert!(matches!(
        a.recovery_record(history_id(&entry)).unwrap().content,
        sigil_crypto::recovery::Content::Redacted { .. }
    ));
    let mut forged = a.recovery_record(history_id(&entry)).unwrap();
    if let sigil_crypto::recovery::Content::Redacted { original, .. } = &mut forged.content {
        *original = [99; 32];
    }
    let tx =
        a.db.transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
    assert!(matches!(
        restore(&tx, &a.key, &forged),
        Err(Error::Conflict)
    ));
    drop(tx);
    // Replaying authenticated history cannot restore a removed body.
    let tx =
        a.db.transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
    let mut replay = entry;
    replay.operation = original;
    ingest_only(&tx, &a.key, &replay).unwrap();
    tx.commit().unwrap();
    assert!(
        a.conversation_message(conversation, target, now)
            .unwrap()
            .deleted
    );
}
fn account(c: &mut ClientStore) -> Id {
    event::account(
        &peers::parse(&c.own_device_binding().unwrap())
            .unwrap()
            .binding,
    )
}

#[test]
fn deleted_transport_bodies_are_erased_atomically_and_replay_stays_obsolete() {
    let (dir, _fixture, mut a, mut b, now) = pair();
    let (_, peer) = trust(&mut a, &mut b);
    let author = account(&mut a);
    let op = a
        .conversation_operation([240; 32], post("erase this transport body"))
        .unwrap();
    deliver(&mut a, &mut b, peer, &op, now);
    let before: Vec<u8> =
        a.db.query_row("SELECT content FROM outbox LIMIT 1", [], |r| r.get(0))
            .unwrap();
    let remove = a
        .conversation_operation(
            [241; 32],
            Action::Delete {
                target: Reference {
                    author,
                    message: op.id,
                },
            },
        )
        .unwrap();
    deliver(&mut a, &mut b, peer, &remove, now);
    a.maintain_history(now).unwrap();
    b.maintain_history(now).unwrap();
    a.db.execute_batch("CREATE TRIGGER fail_erase BEFORE UPDATE OF content ON outbox BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(matches!(
        a.erase_obsolete_journals(now),
        Err(Error::Storage(_))
    ));
    assert_eq!(
        a.db.query_row("SELECT content FROM outbox LIMIT 1", [], |r| r
            .get::<_, Vec<u8>>(0))
            .unwrap(),
        before
    );
    a.db.execute_batch("DROP TRIGGER fail_erase").unwrap();
    assert!(a.erase_obsolete_journals(now).unwrap().erased > 0);
    assert!(b.erase_obsolete_journals(now).unwrap().erased > 0);
    let raw: Vec<u8> =
        a.db.query_row("SELECT content FROM outbox LIMIT 1", [], |r| r.get(0))
            .unwrap();
    assert_ne!(raw, before);
    let bytes = std::fs::read(dir.path().join("alice.db")).unwrap();
    assert!(!bytes.windows(before.len()).any(|v| v == before));
    assert!(!dir.path().join("alice.db-wal").exists());
    drop(a);
    let mut a = ClientStore::open(
        &dir.path().join("alice.db"),
        StorageKey::new(sigil_crypto::Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap();
    assert_eq!(a.erase_obsolete_journals(now).unwrap().erased, 0);
    assert!(matches!(
        a.queue_peer_operation(peer, &op, now, now),
        Err(Error::Obsolete)
    ));
}
fn deliver(c: &mut ClientStore, other: &mut ClientStore, peer: Id, op: &Operation, now: u64) {
    c.queue_peer_operation(peer, op, now, now).unwrap();
    let mut sent = c.resume_send_intents_online(now).unwrap();
    if sent.is_empty() {
        sent = c.resume_send_intents_online(now).unwrap();
    }
    for v in sent {
        let session = v.result.unwrap();
        c.send_pending_online(session, now).unwrap();
    }
    other.accept_delivery(&next(other)).unwrap();
    other.acknowledge_incoming_online().unwrap();
}
#[test]
fn direct_actions_survive_reordering_forgery_replay_and_restart() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (a, b) = trust(&mut alice, &mut bob);
    let conversation = alice.direct_conversation(b).unwrap();
    let author = account(&mut alice);
    let target = Reference {
        author,
        message: [70; 32],
    };
    let original = alice
        .conversation_operation([70; 32], post("original"))
        .unwrap();
    deliver(&mut alice, &mut bob, b, &original, now);
    let edit = alice
        .conversation_operation(
            [71; 32],
            Action::Edit {
                target: target.clone(),
                body: Body::Text("edited".into()),
            },
        )
        .unwrap();
    deliver(&mut alice, &mut bob, b, &edit, now);
    let (edit_session, edit_message): (Vec<u8>, Vec<u8>) = alice
        .db
        .query_row(
            "SELECT session,id FROM outbox ORDER BY rowid DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let edit_session: Id = edit_session.try_into().unwrap();
    let edit_message: Id = edit_message.try_into().unwrap();
    let raw = alice.outgoing_message(edit_session, edit_message).unwrap();
    let envelope = sigil_protocol::event::Direct::from_bytes(&raw).unwrap();
    assert!(
        matches!(envelope.content,Content::Conversation(bytes) if Operation::from_bytes(bytes).unwrap().id==edit.id)
    );
    assert!(
        matches!(bob.conversation_message(conversation,target.clone(),now).unwrap().body,Some(Body::Text(v)) if v=="edited")
    );
    let forged = bob
        .conversation_operation(
            [72; 32],
            Action::Delete {
                target: target.clone(),
            },
        )
        .unwrap();
    deliver(&mut bob, &mut alice, a, &forged, now);
    assert!(
        !alice
            .conversation_message(conversation, target.clone(), now)
            .unwrap()
            .deleted
    );
    let reaction = bob
        .conversation_operation(
            [73; 32],
            Action::Reaction {
                target: target.clone(),
                emoji: "👍".into(),
                active: true,
            },
        )
        .unwrap();
    deliver(&mut bob, &mut alice, a, &reaction, now);
    assert_eq!(
        alice
            .conversation_message(conversation, target.clone(), now)
            .unwrap()
            .reactions
            .len(),
        1
    );
    let switch = bob
        .conversation_operation(
            [125; 32],
            Action::Reaction {
                target: target.clone(),
                emoji: "❤️".into(),
                active: true,
            },
        )
        .unwrap();
    deliver(&mut bob, &mut alice, a, &switch, now);
    assert_eq!(
        alice
            .conversation_message(conversation, target.clone(), now)
            .unwrap()
            .reactions,
        vec![(account(&mut bob), "❤️".into())]
    );
    let remove = bob
        .conversation_operation(
            [126; 32],
            Action::Reaction {
                target: target.clone(),
                emoji: "❤️".into(),
                active: false,
            },
        )
        .unwrap();
    deliver(&mut bob, &mut alice, a, &remove, now);
    assert!(alice
        .conversation_message(conversation, target.clone(), now)
        .unwrap()
        .reactions
        .is_empty());
    let delete = alice
        .conversation_operation(
            [74; 32],
            Action::Delete {
                target: target.clone(),
            },
        )
        .unwrap();
    deliver(&mut alice, &mut bob, b, &delete, now);
    assert!(matches!(
        alice.outgoing_message(edit_session, edit_message),
        Err(Error::Obsolete)
    ));
    drop(bob);
    let mut bob = ClientStore::open(
        &dir.path().join("bob.db"),
        StorageKey::new(sigil_crypto::Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap();
    assert!(
        bob.conversation_message(conversation, target.clone(), now)
            .unwrap()
            .deleted
    );
    let later = alice
        .conversation_operation(
            [75; 32],
            Action::Edit {
                target: target.clone(),
                body: Body::Text("cannot resurrect".into()),
            },
        )
        .unwrap();
    assert!(matches!(
        alice.queue_peer_operation(b, &later, now, now),
        Err(Error::Obsolete)
    ));
    assert!(bob
        .conversation_message(conversation, target, now)
        .unwrap()
        .body
        .is_none());
}
#[test]
fn private_drafts_preserve_concurrency_and_explicit_resolution() {
    let (_dir, _fixture, mut c, _, now) = pair();
    let own = c.own_device_binding().unwrap();
    let actor = account(&mut c);
    let conversation = [5; 32];
    let first = c
        .conversation_operation(
            [1; 32],
            Action::Private {
                conversation,
                value: Private::Draft {
                    text: "first".into(),
                    observed: vec![],
                },
            },
        )
        .unwrap();
    c.apply_private_operation(&first, now).unwrap();
    let other = Operation {
        id: [2; 32],
        version: Version {
            device: [3; 32],
            counter: 1,
        },
        action: Action::Private {
            conversation,
            value: Private::Draft {
                text: "concurrent".into(),
                observed: vec![],
            },
        },
    };
    let tx = c.db.transaction().unwrap();
    retain(
        &tx,
        &c.key,
        &own,
        conversation,
        (actor, peers::parse(&own).unwrap().binding.identity),
        now,
        Content::Conversation(&other.to_bytes().unwrap()),
    )
    .unwrap();
    tx.commit().unwrap();
    let preferences = c.conversation_preferences(conversation).unwrap();
    assert_eq!(preferences.drafts.len(), 2);
    let mut observed: Vec<_> = preferences.drafts.into_iter().map(|v| v.version).collect();
    observed.sort();
    let resolved = c
        .conversation_operation(
            [4; 32],
            Action::Private {
                conversation,
                value: Private::Draft {
                    text: "merged".into(),
                    observed,
                },
            },
        )
        .unwrap();
    c.apply_private_operation(&resolved, now).unwrap();
    let preferences = c.conversation_preferences(conversation).unwrap();
    assert_eq!(preferences.drafts.len(), 1);
    assert_eq!(preferences.drafts[0].text, "merged");
}
#[test]
fn remote_private_state_and_device_spoofing_roll_back() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (_, b) = trust(&mut alice, &mut bob);
    let op = alice
        .conversation_operation(
            [40; 32],
            Action::Private {
                conversation: [1; 32],
                value: Private::ReadReceipts(false),
            },
        )
        .unwrap();
    assert!(matches!(
        alice.queue_peer_operation(b, &op, now, now),
        Err(Error::InvalidEvent)
    ));
    let mut op = alice
        .conversation_operation([41; 32], post("spoof"))
        .unwrap();
    op.version.device = [9; 32];
    assert!(matches!(
        alice.queue_peer_operation(b, &op, now, now),
        Err(Error::InvalidEvent)
    ));
    assert_eq!(
        bob.db
            .query_row("SELECT count(*) FROM conversation_ops", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

fn linked() -> (
    tempfile::TempDir,
    crate::network::tests::Fixture,
    ClientStore,
    ClientStore,
    Id,
    Id,
    u64,
) {
    let (dir, fixture, invitation, now) = crate::connection::tests::setup();
    let open = |name: &str| {
        ClientStore::open(
            &dir.path().join(name),
            StorageKey::new(sigil_crypto::Secret32::from_bytes([9; 32])).unwrap(),
        )
        .unwrap()
    };
    let mut a = open("sponsor.db");
    crate::connection::tests::prepare(&mut a, &fixture, &invitation.secret);
    a.enroll_online().unwrap();
    let mut b = open("joined.db");
    let offer = b.prepare_device_link_offer([1; 32], now).unwrap();
    let (proposal, digest) = a
        .prepare_sponsored_link([2; 32], &crate::link::offer_qr(&offer).unwrap(), now)
        .unwrap();
    b.accept_link_proposal([1; 32], &proposal, now).unwrap();
    let response = b.confirm_link_proposal([1; 32], digest, now).unwrap();
    let proof = a
        .confirm_sponsored_link([2; 32], &response, digest, now)
        .unwrap();
    a.authorize_sponsored_link_online([2; 32]).unwrap();
    b.finish_device_link_online(
        [1; 32],
        fixture.port(),
        &[crate::network::tests::CA.to_vec()],
    )
    .unwrap();
    b.prepare_prekey_publication([40; 32], true, 3600).unwrap();
    b.publish_prekey_online([40; 32]).unwrap();
    let ap = peers::reference(&proof.sponsor.binding.server, &proof.sponsor.binding.device);
    let bp = peers::reference(&proof.joining.binding.server, &proof.joining.binding.device);
    (dir, fixture, a, b, ap, bp, now)
}
fn pump(from: &mut ClientStore, to: &mut ClientStore, now: u64) {
    from.sync_conversation_devices(now).unwrap();
    for _ in 0..2 {
        for s in from.resume_send_intents_online(now).unwrap() {
            let session = s.result.unwrap();
            from.send_pending_online(session, now).unwrap();
        }
    }
    let incoming = to.receive_mailbox_online(now).unwrap();
    for item in incoming {
        assert!(item.result.is_ok(), "{:?}", item.result.err());
    }
    to.acknowledge_incoming_online().unwrap();
}

#[test]
fn erased_history_sync_fragments_survive_legacy_backfill_and_restarts() {
    let (dir, _fixture, mut a, mut b, _ap, _bp, now) = linked();
    let author = account(&mut a);
    let post = a
        .conversation_operation([248; 32], post(&"synthetic fragment ".repeat(1700)))
        .unwrap();
    let conversation = a.note_to_self(&post, now, now).unwrap();
    for _ in 0..5 {
        pump(&mut a, &mut b, now);
    }
    assert!(b
        .conversation_message(
            conversation,
            Reference {
                author,
                message: post.id
            },
            now
        )
        .unwrap()
        .body
        .is_some());
    // Reconstruct associations for journals written before the erasure migration.
    for c in [&mut a, &mut b] {
        c.db.execute_batch("DELETE FROM conversation_transfer_origins")
            .unwrap();
        c.erase_obsolete_journals(now).unwrap();
    }
    let delete = a
        .conversation_operation(
            [249; 32],
            Action::Delete {
                target: Reference {
                    author,
                    message: post.id,
                },
            },
        )
        .unwrap();
    a.note_to_self(&delete, now, now).unwrap();
    for _ in 0..3 {
        pump(&mut a, &mut b, now);
    }
    for c in [&mut a, &mut b] {
        c.maintain_history(now).unwrap();
        let mut erased = 0;
        for _ in 0..4 {
            erased += c.erase_obsolete_journals(now).unwrap().erased;
        }
        assert!(erased >= 2);
        assert!(
            c.conversation_message(
                conversation,
                Reference {
                    author,
                    message: post.id
                },
                now
            )
            .unwrap()
            .deleted
        );
    }
    drop(a);
    let mut a = ClientStore::open(
        &dir.path().join("sponsor.db"),
        StorageKey::new(sigil_crypto::Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap();
    assert_eq!(a.erase_obsolete_journals(now).unwrap().erased, 0);
}
#[test]
fn linked_devices_sync_fragmented_content_drafts_and_private_collections_after_restart() {
    let (dir, _fixture, mut a, mut b, _ap, _bp, now) = linked();
    let author = account(&mut a);
    let op = a
        .conversation_operation([50; 32], post(&"large 🖋️ ".repeat(3000)))
        .unwrap();
    let conversation = a.note_to_self(&op, now, now).unwrap();
    let draft = a
        .conversation_operation(
            [51; 32],
            Action::Private {
                conversation,
                value: Private::Draft {
                    text: "sponsor draft".into(),
                    observed: vec![],
                },
            },
        )
        .unwrap();
    a.apply_private_operation(&draft, now).unwrap();
    let draft = b
        .conversation_operation(
            [52; 32],
            Action::Private {
                conversation,
                value: Private::Draft {
                    text: "joined draft".into(),
                    observed: vec![],
                },
            },
        )
        .unwrap();
    b.apply_private_operation(&draft, now).unwrap();
    for n in 0..3 {
        let action = match n {
            0 => Private::CollectionsEnabled(true),
            1 => Private::Collection {
                id: [70; 32],
                name: "People".into(),
                present: true,
            },
            _ => Private::CollectionMember {
                id: [70; 32],
                present: true,
            },
        };
        let op = a
            .conversation_operation(
                [60 + n; 32],
                Action::Private {
                    conversation,
                    value: action,
                },
            )
            .unwrap();
        a.apply_private_operation(&op, now).unwrap();
    }
    pump(&mut a, &mut b, now);
    drop(a);
    let mut a = ClientStore::open(
        &dir.path().join("sponsor.db"),
        StorageKey::new(sigil_crypto::Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap();
    for _ in 0..5 {
        pump(&mut a, &mut b, now);
        pump(&mut b, &mut a, now);
    }
    for c in [&mut a, &mut b] {
        let p = c.conversation_preferences(conversation).unwrap();
        assert_eq!(p.drafts.len(), 2);
        assert!(p.collections_enabled);
        assert_eq!(p.collections[0].1, "People");
        assert_eq!(p.collection_members, vec![[70; 32]]);
        assert!(
            matches!(c.conversation_message(conversation,Reference{author,message:op.id},now).unwrap().body,Some(Body::Text(v)) if v=="large 🖋️ ".repeat(3000))
        );
    }
}
#[test]
fn expiry_view_once_receipts_threads_and_privacy_preferences() {
    let (_dir, _fixture, mut a, _, now) = pair();
    let author = account(&mut a);
    let op = a
        .conversation_operation(
            [80; 32],
            Action::Post {
                body: Body::Text("once".into()),
                reply: None,
                thread: None,
                expires_at: Some(now + 60),
                view_once: true,
            },
        )
        .unwrap();
    let conv = a.note_to_self(&op, now, now).unwrap();
    let reference = Reference {
        author,
        message: op.id,
    };
    assert!(a
        .conversation_message(conv, reference.clone(), now)
        .unwrap()
        .body
        .is_none());
    a.db.execute_batch("CREATE TRIGGER fail_consume BEFORE INSERT ON conversation_ops BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(a
        .consume_view_once(conv, reference.clone(), [81; 32], now)
        .is_err());
    a.db.execute_batch("DROP TRIGGER fail_consume;").unwrap();
    assert!(
        matches!(a.consume_view_once(conv,reference.clone(),[81;32],now).unwrap(),Body::Text(v) if v=="once")
    );
    assert!(matches!(
        a.consume_view_once(conv, reference, [82; 32], now),
        Err(Error::Obsolete)
    ));
    let root = a.conversation_operation([83; 32], post("root")).unwrap();
    a.note_to_self(&root, now, now).unwrap();
    let reference = Reference {
        author,
        message: root.id,
    };
    let reply = a
        .conversation_operation(
            [84; 32],
            Action::Post {
                body: Body::Text("reply".into()),
                reply: Some(reference.clone()),
                thread: Some(reference.clone()),
                expires_at: Some(now + 1),
                view_once: false,
            },
        )
        .unwrap();
    a.note_to_self(&reply, now, now).unwrap();
    assert_eq!(
        a.conversation_page(conv, None, Some(&reference), None, now)
            .unwrap()
            .messages
            .len(),
        1
    );
    assert!(a
        .conversation_page(conv, None, Some(&reference), None, now + 1)
        .unwrap()
        .messages
        .is_empty());
    assert!(!a.conversation_preferences(conv).unwrap().presence_sharing);
    let op = a
        .conversation_operation(
            [85; 32],
            Action::Private {
                conversation: conv,
                value: Private::TypingIndicators(false),
            },
        )
        .unwrap();
    a.apply_private_operation(&op, now).unwrap();
    let op = a
        .conversation_operation(
            [86; 32],
            Action::Typing {
                active: true,
                until: now + 30,
            },
        )
        .unwrap();
    assert!(matches!(
        require_send(
            &a.db,
            &a.key,
            conv,
            Content::Conversation(&op.to_bytes().unwrap()),
            now,
            now
        ),
        Err(Error::Obsolete)
    ));
}

#[test]
fn conversation_file_views_enforce_deletion_and_once_before_exposing_plaintext() {
    let (dir, _fixture, mut a, mut b, now) = pair();
    let (_, peer) = trust(&mut a, &mut b);
    let author = account(&mut a);
    let conv = a.direct_conversation(peer).unwrap();
    let mut sender = a
        .open_attachment_cache(&dir.path().join("files-a.db"), 8 * 1024 * 1024)
        .unwrap();
    let file = sender
        .prepare_upload(
            5,
            crate::attachments::Metadata {
                name: "synthetic.txt".into(),
                media_type: "text/plain".into(),
            },
            None,
        )
        .unwrap();
    sender.stage_chunk(file, 0, b"hello").unwrap();
    sender.finish_staging(file).unwrap();
    for _ in 0..3 {
        a.upload_attachment_step(&mut sender, file).unwrap();
    }
    let mut receiver = b
        .open_attachment_cache(&dir.path().join("files-b.db"), 8 * 1024 * 1024)
        .unwrap();
    for (id, once) in [(90, false), (91, true)] {
        a.queue_conversation_file(
            Destination::Peer(peer),
            &mut sender,
            file,
            FilePost {
                id: [id; 32],
                timestamp: now,
                reply: None,
                thread: None,
                expires_at: None,
                view_once: once,
            },
            now,
        )
        .unwrap();
        for _ in 0..2 {
            for send in a.resume_send_intents_online(now).unwrap() {
                a.send_pending_online(send.result.unwrap(), now).unwrap();
            }
        }
        b.accept_delivery(&next(&b)).unwrap();
        b.acknowledge_incoming_online().unwrap();
        let target = Reference {
            author,
            message: [id; 32],
        };
        assert_eq!(
            b.prepare_conversation_file(&mut receiver, conv, target.clone(), now)
                .unwrap(),
            file
        );
        if id == 90 {
            b.download_attachment_step(&mut receiver, file, now)
                .unwrap();
            b.download_attachment_step(&mut receiver, file, now)
                .unwrap();
            assert_eq!(
                b.conversation_file_chunk(&receiver, conv, target, 0, now)
                    .unwrap()
                    .as_slice(),
                b"hello"
            );
        } else {
            assert!(b
                .conversation_file_chunk(&receiver, conv, target.clone(), 0, now)
                .is_err());
            let mut view = b
                .open_view_once_file(&receiver, conv, target.clone(), [92; 32], now)
                .unwrap();
            assert_eq!(view.chunk(&receiver, 0, now).unwrap().as_slice(), b"hello");
            assert!(b
                .open_view_once_file(&receiver, conv, target, [93; 32], now)
                .is_err());
        }
    }
    let target = Reference {
        author,
        message: [90; 32],
    };
    let deleted = a
        .conversation_operation(
            [94; 32],
            Action::Delete {
                target: target.clone(),
            },
        )
        .unwrap();
    deliver(&mut a, &mut b, peer, &deleted, now);
    assert!(matches!(
        b.conversation_file_chunk(&receiver, conv, target, 0, now),
        Err(Error::Obsolete)
    ));
}
#[test]
fn expired_content_does_not_reappear_after_clock_rollback_and_ephemeral_edits_are_not_archived() {
    let (_dir, _fixture, mut a, _, now) = pair();
    let own = peers::parse(&a.own_device_binding().unwrap())
        .unwrap()
        .binding;
    let author = event::account(&own);
    a.configure_recovery(
        &own.server,
        own.account,
        sigil_crypto::Secret32::from_bytes([7; 32]),
    )
    .unwrap();
    let original = a
        .conversation_operation(
            [95; 32],
            Action::Post {
                body: Body::Text("temporary".into()),
                reply: None,
                thread: None,
                expires_at: Some(now + 1),
                view_once: false,
            },
        )
        .unwrap();
    let conv = a.note_to_self(&original, now, now).unwrap();
    let target = Reference {
        author,
        message: original.id,
    };
    let edit = a
        .conversation_operation(
            [96; 32],
            Action::Edit {
                target: target.clone(),
                body: Body::Text("temporary edit".into()),
            },
        )
        .unwrap();
    a.note_to_self(&edit, now, now).unwrap();
    assert!(a.recovery_records(None).unwrap().is_empty());
    assert!(a
        .conversation_message(conv, target.clone(), now + 1)
        .unwrap()
        .body
        .is_none());
    assert!(a
        .conversation_message(conv, target, now)
        .unwrap()
        .body
        .is_none());
    let op = a
        .conversation_operation(
            [97; 32],
            Action::Post {
                body: Body::Text("once expires".into()),
                reply: None,
                thread: None,
                expires_at: Some(now + 2),
                view_once: true,
            },
        )
        .unwrap();
    a.note_to_self(&op, now, now).unwrap();
    let target = Reference {
        author,
        message: op.id,
    };
    assert!(a
        .consume_view_once(conv, target.clone(), [98; 32], now + 2)
        .is_err());
    assert!(a.consume_view_once(conv, target, [99; 32], now).is_err());
}

#[test]
fn linked_sync_defers_unknown_edits_and_excludes_temporary_bodies() {
    let (dir, _fixture, mut a, mut b, _ap, _bp, now) = linked();
    let author = account(&mut a);
    let original = a
        .conversation_operation([100; 32], post("original"))
        .unwrap();
    let edit = a
        .conversation_operation(
            [101; 32],
            Action::Edit {
                target: Reference {
                    author,
                    message: original.id,
                },
                body: Body::Text("ordinary edit".into()),
            },
        )
        .unwrap();
    let conv = a.note_to_self(&edit, now, now).unwrap();
    for _ in 0..3 {
        pump(&mut a, &mut b, now);
    }
    assert_eq!(
        b.db.query_row("SELECT count(*) FROM conversation_ops", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(a);
    let mut a = ClientStore::open(
        &dir.path().join("sponsor.db"),
        StorageKey::new(sigil_crypto::Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap();
    a.note_to_self(&original, now, now).unwrap();
    let original = a
        .conversation_operation(
            [102; 32],
            Action::Post {
                body: Body::Text("temporary".into()),
                reply: None,
                thread: None,
                expires_at: Some(now + 600),
                view_once: false,
            },
        )
        .unwrap();
    let edit = a
        .conversation_operation(
            [103; 32],
            Action::Edit {
                target: Reference {
                    author,
                    message: original.id,
                },
                body: Body::Text("temporary edit".into()),
            },
        )
        .unwrap();
    a.note_to_self(&edit, now, now).unwrap();
    for _ in 0..3 {
        pump(&mut a, &mut b, now);
    }
    a.note_to_self(&original, now, now).unwrap();
    for _ in 0..5 {
        pump(&mut a, &mut b, now);
    }
    assert!(
        matches!(b.conversation_message(conv,Reference{author,message:[100;32]},now).unwrap().body,Some(Body::Text(v)) if v=="ordinary edit")
    );
    let all = entries(
        &b.db,
        &b.key,
        "SELECT id,state FROM conversation_ops WHERE scope=?1",
        &scope(&b.key, &conv).unwrap(),
    )
    .unwrap();
    assert!(all
        .iter()
        .all(|e| e.operation.id != [102; 32] && e.operation.id != [103; 32]));
    assert_eq!(
        a.db.query_row("SELECT count(*) FROM conversation_sync_deferred", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn automatic_receipts_survive_queue_failure_and_restart_without_duplicates() {
    let (dir, _fixture, mut a, mut b, now) = pair();
    let (alice, peer) = trust(&mut a, &mut b);
    let author = account(&mut a);
    let bob = account(&mut b);
    let conv = a.direct_conversation(peer).unwrap();
    let op = a
        .conversation_operation([104; 32], post("receipt"))
        .unwrap();
    deliver(&mut a, &mut b, peer, &op, now);
    b.db.execute_batch("CREATE TRIGGER fail_receipt BEFORE INSERT ON send_intents BEGIN SELECT RAISE(ABORT,'synthetic queue failure'); END;").unwrap();
    assert!(b.resume_conversation_receipts(now).is_err());
    b.db.execute_batch("DROP TRIGGER fail_receipt;").unwrap();
    drop(b);
    let mut b = ClientStore::open(
        &dir.path().join("bob.db"),
        StorageKey::new(sigil_crypto::Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap();
    assert_eq!(b.resume_conversation_receipts(now).unwrap(), 1);
    assert_eq!(b.resume_conversation_receipts(now).unwrap(), 0);
    for _ in 0..2 {
        for send in b.resume_send_intents_online(now).unwrap() {
            b.send_pending_online(send.result.unwrap(), now).unwrap();
        }
    }
    for item in a.receive_mailbox_online(now).unwrap() {
        assert!(item.result.is_ok());
    }
    a.acknowledge_incoming_online().unwrap();
    let target = Reference {
        author,
        message: op.id,
    };
    let view = a.conversation_message(conv, target.clone(), now).unwrap();
    assert_eq!(view.delivered, vec![bob]);
    assert!(view.read.is_empty());
    let off = b
        .conversation_operation(
            [105; 32],
            Action::Private {
                conversation: conv,
                value: Private::ReadReceipts(false),
            },
        )
        .unwrap();
    b.apply_private_operation(&off, now).unwrap();
    let read = b
        .conversation_operation([106; 32], Action::Receipt { target, read: true })
        .unwrap();
    assert!(matches!(
        b.queue_peer_operation(alice, &read, now, now),
        Err(Error::Obsolete)
    ));
}

#[test]
fn fragment_reordering_conflicts_and_commit_failure_preserve_atomicity() {
    let (_dir, _fixture, mut a, _, now) = pair();
    let own = a.own_device_binding().unwrap();
    let fields = peers::parse(&own).unwrap().binding;
    let author = event::account(&fields);
    let operation = a
        .conversation_operation([107; 32], post(&"x".repeat(20000)))
        .unwrap();
    let e = Entry {
        conversation: [6; 32],
        author,
        identity: fields.identity,
        timestamp: now,
        seen: now,
        operation,
    };
    let raw = serde_json::to_vec(&e).unwrap();
    let digest: Id = Sha256::digest(&raw).into();
    let total = raw.len().div_ceil(16384) as u32;
    let mut parts: Vec<_> = raw
        .chunks(16384)
        .enumerate()
        .map(|(i, p)| Operation {
            id: [108 + i as u8; 32],
            version: Version {
                device: [3; 32],
                counter: i as u64 + 1,
            },
            action: Action::SyncPart {
                transfer: [7; 32],
                digest,
                index: i as u32,
                total,
                payload: transport::hex(p),
            },
        })
        .collect();
    parts.reverse();
    let tx = a.db.transaction().unwrap();
    sync::receive(&tx, &a.key, &parts[0]).unwrap();
    tx.commit().unwrap();
    let mut changed = parts[0].clone();
    if let Action::SyncPart { payload, .. } = &mut changed.action {
        payload.replace_range(0..2, "ff");
    }
    let tx = a.db.transaction().unwrap();
    assert!(matches!(
        sync::receive(&tx, &a.key, &changed),
        Err(Error::Conflict)
    ));
    drop(tx);
    a.db.execute_batch("CREATE TRIGGER fail_fragment BEFORE INSERT ON conversation_ops BEGIN SELECT RAISE(ABORT,'synthetic completion failure'); END;").unwrap();
    let tx = a.db.transaction().unwrap();
    assert!(sync::receive(&tx, &a.key, &parts[1]).is_err());
    drop(tx);
    assert!(original(
        &a.db,
        &a.key,
        &e.conversation,
        &Reference {
            author,
            message: e.operation.id
        }
    )
    .unwrap()
    .is_none());
    a.db.execute_batch("DROP TRIGGER fail_fragment;").unwrap();
    let tx = a.db.transaction().unwrap();
    sync::receive(&tx, &a.key, &parts[1]).unwrap();
    sync::receive(&tx, &a.key, &parts[0]).unwrap();
    tx.commit().unwrap();
    assert!(original(
        &a.db,
        &a.key,
        &e.conversation,
        &Reference {
            author,
            message: e.operation.id
        }
    )
    .unwrap()
    .is_some());
}

#[test]
fn recovery_restores_conversation_deletions_drafts_and_counters_on_a_fresh_device() {
    let (dir, fixture, mut a, _, now) = pair();
    let own = peers::parse(&a.own_device_binding().unwrap())
        .unwrap()
        .binding;
    let author = event::account(&own);
    a.configure_recovery(
        &own.server,
        own.account,
        sigil_crypto::Secret32::from_bytes([7; 32]),
    )
    .unwrap();
    let first = a
        .conversation_operation([110; 32], post("retained history"))
        .unwrap();
    let conv = a.note_to_self(&first, now, now).unwrap();
    let second = a
        .conversation_operation([111; 32], post("deleted history"))
        .unwrap();
    a.note_to_self(&second, now, now).unwrap();
    let deleted = a
        .conversation_operation(
            [112; 32],
            Action::Delete {
                target: Reference {
                    author,
                    message: second.id,
                },
            },
        )
        .unwrap();
    a.note_to_self(&deleted, now, now).unwrap();
    let draft = a
        .conversation_operation(
            [113; 32],
            Action::Private {
                conversation: conv,
                value: Private::Draft {
                    text: "recover this draft".into(),
                    observed: vec![],
                },
            },
        )
        .unwrap();
    a.apply_private_operation(&draft, now).unwrap();
    let head = a.prepare_recovery_upload(now).unwrap();
    assert_eq!(a.upload_recovery_step().unwrap(), Some(head));
    let account = a.connection_session().unwrap().unwrap().account_id;
    let invite = sigil_server::store::Store::open(&dir.path().join("server.db"))
        .unwrap()
        .invite_reauthorization(&account, 60, now)
        .unwrap();
    let mut recovered = ClientStore::open(
        &dir.path().join("recovered.db"),
        StorageKey::new(sigil_crypto::Secret32::from_bytes([10; 32])).unwrap(),
    )
    .unwrap();
    recovered
        .prepare_enrollment(
            "chat.example",
            fixture.port(),
            &[crate::network::tests::CA.to_vec()],
            &invite.secret,
            "Synthetic recovery",
            true,
        )
        .unwrap();
    recovered.enroll_online().unwrap();
    recovered
        .configure_recovery(
            &own.server,
            own.account,
            sigil_crypto::Secret32::from_bytes([7; 32]),
        )
        .unwrap();
    let mut complete = false;
    for _ in 0..8 {
        if recovered.download_recovery_step(true).unwrap() == Some(head) {
            complete = true;
            break;
        }
    }
    assert!(complete);
    assert!(
        matches!(recovered.conversation_message(conv,Reference{author,message:first.id},now).unwrap().body,Some(Body::Text(v)) if v=="retained history")
    );
    assert!(
        recovered
            .conversation_message(
                conv,
                Reference {
                    author,
                    message: second.id
                },
                now
            )
            .unwrap()
            .deleted
    );
    let preferences = recovered.conversation_preferences(conv).unwrap();
    assert_eq!(preferences.drafts.len(), 1);
    assert_eq!(preferences.drafts[0].text, "recover this draft");
    for table in ["identity", "sessions", "peers", "prekeys"] {
        assert_eq!(
            recovered
                .db
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    let op = recovered
        .conversation_operation(
            [114; 32],
            Action::Private {
                conversation: conv,
                value: Private::Draft {
                    text: "resolved".into(),
                    observed: preferences.drafts.into_iter().map(|d| d.version).collect(),
                },
            },
        )
        .unwrap();
    assert!(op.version.counter > draft.version.counter);
    recovered.apply_private_operation(&op, now).unwrap();
    assert_eq!(
        recovered.conversation_preferences(conv).unwrap().drafts[0].text,
        "resolved"
    );
    assert!(recovered
        .search_conversations("deleted", None, now)
        .unwrap()
        .hits
        .is_empty());
    assert_eq!(
        recovered
            .search_conversations("HISTORY", None, now)
            .unwrap()
            .hits
            .len(),
        1
    );
}

#[test]
fn queued_posts_are_visible_offline_and_deleted_before_submission() {
    let (_dir, _fixture, mut a, mut b, now) = pair();
    let (_, peer) = trust(&mut a, &mut b);
    let author = account(&mut a);
    let op = a
        .conversation_operation([115; 32], post("offline"))
        .unwrap();
    a.queue_peer_operation(peer, &op, now, now).unwrap();
    assert_eq!(
        a.operation_delivery_state(peer, op.id).unwrap(),
        DeliveryState::Queued
    );
    assert_eq!(
        a.search_conversations("offline", None, now)
            .unwrap()
            .hits
            .len(),
        1
    );
    let delete = a
        .conversation_operation(
            [116; 32],
            Action::Delete {
                target: Reference {
                    author,
                    message: op.id,
                },
            },
        )
        .unwrap();
    a.queue_peer_operation(peer, &delete, now, now).unwrap();
    assert!(a
        .search_conversations("offline", None, now)
        .unwrap()
        .hits
        .is_empty());
    let sends = a.resume_send_intents_online(now).unwrap();
    assert!(sends
        .iter()
        .any(|s| matches!(s.result, Err(Error::Obsolete))));
    assert_eq!(
        a.operation_delivery_state(peer, op.id).unwrap(),
        DeliveryState::Cancelled
    );
    assert!(matches!(
        a.queue_peer_operation(peer, &op, now, now),
        Err(Error::Obsolete)
    ));
}

#[test]
fn conversation_cards_use_structured_permissions_and_deletion_guards() {
    use sigil_protocol::text::{action::Reference as CardReference, parse_card, Origin, Parsed};
    let (_dir, _fixture, mut a, _, now) = pair();
    let author = account(&mut a);
    let Parsed::Card(card) = parse_card(
        "poll::closed::Choose\n- One\n- Two;",
        Origin {
            message: [117; 32],
            creator: author,
            created_at: now,
            timezone: None,
        },
        Default::default(),
    )
    .unwrap()
    .content
    else {
        panic!()
    };
    let reference = CardReference::of(&card).unwrap();
    let bytes = card.to_bytes().unwrap();
    let post = a
        .conversation_operation(
            [117; 32],
            Action::Post {
                body: Body::Rich(bytes.clone()),
                reply: None,
                thread: None,
                expires_at: None,
                view_once: false,
            },
        )
        .unwrap();
    let conv = a.note_to_self(&post, now, now).unwrap();
    assert!(a.card_state(conv, reference).is_ok());
    assert_eq!(
        a.search_conversations("Choose", None, now)
            .unwrap()
            .hits
            .len(),
        1
    );
    let edit = a
        .conversation_operation(
            [118; 32],
            Action::Edit {
                target: Reference {
                    author,
                    message: post.id,
                },
                body: Body::Text("cannot replace a poll".into()),
            },
        )
        .unwrap();
    assert!(matches!(
        a.note_to_self(&edit, now, now),
        Err(Error::Obsolete)
    ));
    assert!(a
        .conversation_operation(
            [119; 32],
            Action::Edit {
                target: Reference {
                    author,
                    message: post.id
                },
                body: Body::Rich(bytes)
            }
        )
        .is_err());
    let delete = a
        .conversation_operation(
            [120; 32],
            Action::Delete {
                target: Reference {
                    author,
                    message: post.id,
                },
            },
        )
        .unwrap();
    a.note_to_self(&delete, now, now).unwrap();
    assert!(matches!(
        a.card_state(conv, reference),
        Err(Error::Obsolete)
    ));
}

#[test]
fn modified_lookup_indexes_cannot_redirect_authenticated_controls() {
    let (_dir, _fixture, mut a, _, now) = pair();
    let author = account(&mut a);
    let first = a.conversation_operation([121; 32], post("first")).unwrap();
    let conv = a.note_to_self(&first, now, now).unwrap();
    let second = a.conversation_operation([122; 32], post("second")).unwrap();
    a.note_to_self(&second, now, now).unwrap();
    let first = Reference {
        author,
        message: first.id,
    };
    let second = Reference {
        author,
        message: second.id,
    };
    let delete = a
        .conversation_operation(
            [123; 32],
            Action::Delete {
                target: first.clone(),
            },
        )
        .unwrap();
    a.note_to_self(&delete, now, now).unwrap();
    a.db.execute(
        "UPDATE conversation_ops SET target=?1 WHERE target=?2 AND kind=3",
        (
            target_index(&a.key, &conv, &second).unwrap().as_slice(),
            target_index(&a.key, &conv, &first).unwrap().as_slice(),
        ),
    )
    .unwrap();
    assert!(matches!(
        a.conversation_message(conv, second, now),
        Err(Error::InvalidStore)
    ));
    let private = a
        .conversation_operation(
            [124; 32],
            Action::Private {
                conversation: [1; 32],
                value: Private::Draft {
                    text: "private draft".into(),
                    observed: vec![],
                },
            },
        )
        .unwrap();
    a.apply_private_operation(&private, now).unwrap();
    a.db.execute(
        "UPDATE conversation_ops SET scope=?1 WHERE kind=1",
        [scope(&a.key, &conv).unwrap().as_slice()],
    )
    .unwrap();
    assert!(matches!(
        a.conversation_preferences(conv),
        Err(Error::InvalidStore)
    ));
}

#[test]
fn first_edit_does_not_compete_with_a_migrated_post_version() {
    let (_dir, _fixture, mut a, _, now) = pair();
    let own = a.own_device_binding().unwrap();
    let author = account(&mut a);
    let identity = peers::parse(&own).unwrap().binding.identity;
    let conv = Sha256::digest([b"Sigil/note-to-self/v0".as_slice(), &author].concat()).into();
    let reference = Reference {
        author,
        message: [127; 32],
    };
    let tx = a.db.transaction().unwrap();
    ingest_only(
        &tx,
        &a.key,
        &Entry {
            conversation: conv,
            author,
            identity,
            timestamp: now,
            seen: now,
            operation: Operation {
                id: reference.message,
                version: Version {
                    device: [255; 32],
                    counter: 1,
                },
                action: post("legacy"),
            },
        },
    )
    .unwrap();
    tx.commit().unwrap();
    let edit = a
        .conversation_operation(
            [128; 32],
            Action::Edit {
                target: reference.clone(),
                body: Body::Text("edited".into()),
            },
        )
        .unwrap();
    assert_eq!(edit.version.counter, 1);
    a.note_to_self(&edit, now, now).unwrap();
    assert!(
        matches!(a.conversation_message(conv,reference,now).unwrap().body,Some(Body::Text(v)) if v=="edited")
    );
}
