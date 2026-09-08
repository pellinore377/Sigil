use super::*;
use sigil_crypto::Secret32;
fn open(path: &std::path::Path) -> ClientStore {
    ClientStore::open(
        path,
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap()
}
fn recovery(client: &mut ClientStore) {
    let b = peers::parse(&client.own_device_binding().unwrap())
        .unwrap()
        .binding;
    client
        .configure_recovery(&b.server, b.account, Secret32::from_bytes([7; 32]))
        .unwrap();
}
fn setup() -> (
    tempfile::TempDir,
    crate::network::tests::Fixture,
    ClientStore,
    ClientStore,
    Id,
    Id,
    Id,
    u64,
) {
    let (dir, fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    alice.publish_device_binding_online().unwrap();
    bob.publish_device_binding_online().unwrap();
    let (profile, authority) = service::tests::enable(&dir.path().join("server.db"));
    let group = alice.create_group(authority).unwrap();
    let (a, _) = crate::incoming::tests::trust(&mut alice, &mut bob);
    bob.accept_group_genesis(a, &alice.group_genesis(group).unwrap())
        .unwrap();
    for client in [&mut alice, &mut bob] {
        client
            .pin_group_service(group, &profile, Zeroizing::new([120; 32]))
            .unwrap();
        recovery(client);
    }
    alice.prepare_group_service_request(group, None).unwrap();
    alice.submit_group_service_online(group, now).unwrap();
    let afp = device_fingerprint(&alice.own_device_binding().unwrap()).unwrap();
    let bfp = device_fingerprint(&bob.own_device_binding().unwrap()).unwrap();
    alice
        .queue_group_text(
            group,
            [121; 32],
            &"x".repeat(sigil_protocol::event::GroupText::MAX_BODY),
            now - 10,
            now,
        )
        .unwrap();
    alice
        .queue_group_text(group, [122; 32], "deleted before sharing", now - 9, now)
        .unwrap();
    alice
        .queue_group_text(group, [123; 32], "outside authorized range", now - 100, now)
        .unwrap();
    let original = alice.group_message(group, afp, [122; 32]).unwrap();
    let mut deleted = alice
        .recovery_record(messages::group_event_history_id(&original.event().unwrap()))
        .unwrap();
    deleted.revision += 1;
    deleted.content = sigil_crypto::recovery::Content::Deleted;
    alice.retain_recovery_record(&deleted).unwrap();
    let add = alice
        .prepare_group_change(
            group,
            Change::Add(
                Member::new(
                    [124; 32],
                    Role::Member,
                    &[bob.own_device_binding().unwrap()],
                )
                .unwrap(),
            ),
        )
        .unwrap();
    let add = bob.approve_group_proposal(group, &add).unwrap();
    alice
        .prepare_group_service_request(group, Some(&add))
        .unwrap();
    alice.submit_group_service_online(group, now).unwrap();
    bob.sync_group_service_online(group, now).unwrap();
    for client in [&mut alice, &mut bob] {
        for n in 130..138 {
            client
                .prepare_prekey_publication([n; 32], true, 3600)
                .unwrap();
            client.publish_prekey_online([n; 32]).unwrap();
        }
    }
    alice.db.execute("DELETE FROM peers", []).unwrap();
    bob.db.execute("DELETE FROM peers", []).unwrap();
    alice
        .observe_peer_binding(&bob.own_device_binding().unwrap())
        .unwrap();
    bob.observe_peer_binding(&alice.own_device_binding().unwrap())
        .unwrap();
    (dir, fixture, alice, bob, group, afp, bfp, now)
}
#[track_caller]
fn deliver(source: &mut ClientStore, target: &mut ClientStore, now: u64) {
    for _ in 0..2 {
        for attempt in source.resume_outbound_online(now).unwrap() {
            assert!(attempt.result.is_ok(), "{:?}", attempt.result);
        }
    }
    for attempt in target.receive_mailbox_online(now).unwrap() {
        if let Err(error) = attempt.result {
            panic!("{error:?}");
        }
    }
    target.acknowledge_incoming_online().unwrap();
}
fn enable(alice: &mut ClientStore, bob: &mut ClientStore, group: Id, now: u64) {
    let change = alice
        .prepare_group_change(group, Change::EarlierHistory(true))
        .unwrap();
    alice
        .prepare_group_service_request(group, Some(&change))
        .unwrap();
    alice.submit_group_service_online(group, now).unwrap();
    bob.sync_group_service_online(group, now).unwrap();
}
#[test]
fn history_transfer_is_explicit_bounded_restartable_and_excludes_deleted_and_out_of_range_content()
{
    let (dir, _fixture, mut alice, mut bob, group, afp, bfp, now) = setup();
    let range = HistoryRange {
        source_device: afp,
        recipient_device: bfp,
        from_timestamp: now - 20,
        until_timestamp: now,
        expires_at: now + 600,
    };
    let id = [140; 32];
    assert!(alice
        .prepare_group_history_share(group, id, range, now)
        .is_err());
    assert!(bob.shared_group_history(group, None).unwrap().is_empty());
    enable(&mut alice, &mut bob, group, now);
    alice
        .prepare_group_history_share(group, id, range, now)
        .unwrap();
    let grant = load(&alice.db, &alice.key, &afp, &id).unwrap().grant;
    alice
        .prepare_group_history_share(group, id, range, now)
        .unwrap();
    assert_eq!(grant, load(&alice.db, &alice.key, &afp, &id).unwrap().grant);
    alice.advance_group_history_online(id, now).unwrap();
    deliver(&mut alice, &mut bob, now);
    assert_eq!(
        bob.group_history_share_status(id).unwrap().0,
        HistoryShareStatus::Pending
    );
    let mut failed = false;
    for _ in 0..24 {
        alice.advance_group_history_online(id, now).unwrap();
        deliver(&mut alice, &mut bob, now);
        if !failed {
            bob.db.execute_batch("CREATE TRIGGER fail_import BEFORE INSERT ON group_shared_history BEGIN SELECT RAISE(ABORT,'synthetic history import failure'); END;").unwrap();
            let result = bob.advance_group_history_online(id, now);
            bob.db.execute_batch("DROP TRIGGER fail_import").unwrap();
            if result.is_err() {
                assert!(bob.shared_group_history(group, None).unwrap().is_empty());
                failed = true;
                drop(bob);
                bob = open(&dir.path().join("bob.db"));
                bob.advance_group_history_online(id, now).unwrap();
            }
        } else {
            bob.advance_group_history_online(id, now).unwrap();
        }
        deliver(&mut bob, &mut alice, now);
        if alice.group_history_share_status(id).unwrap().0 == HistoryShareStatus::Complete {
            break;
        }
    }
    assert!(failed);
    assert_eq!(
        alice.group_history_share_status(id).unwrap(),
        (HistoryShareStatus::Complete, 1)
    );
    assert_eq!(
        bob.group_history_share_status(id).unwrap(),
        (HistoryShareStatus::Complete, 1)
    );
    let history = bob.shared_group_history(group, None).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].source_device, afp);
    assert_eq!(history[0].grant, id);
    let event = Group::from_bytes(&history[0].plaintext).unwrap();
    assert_eq!(event.message, [121; 32]);
    assert_eq!(
        event.content.text().unwrap().len(),
        sigil_protocol::event::GroupText::MAX_BODY
    );
    assert!(matches!(
        bob.group_message(group, afp, [121; 32]),
        Err(Error::NotFound)
    ));
    assert!(bob.recovery_records(None).unwrap().is_empty());
    for client in [&alice, &bob] {
        assert_eq!(
            client
                .db
                .query_row("SELECT count(*) FROM group_key_outbox", [], |r| r
                    .get::<_, u32>(0))
                .unwrap(),
            0
        );
    }
    assert!(
        bob.db
            .query_row("SELECT count(*) FROM group_senders", [], |r| r
                .get::<_, u32>(0))
            .unwrap()
            == 0
    );
}
#[test]
fn unavailable_supplier_and_concurrent_policy_change_do_not_report_success() {
    let (_dir, _fixture, mut alice, mut bob, group, afp, bfp, now) = setup();
    enable(&mut alice, &mut bob, group, now);
    let id = [141; 32];
    let range = HistoryRange {
        source_device: bfp,
        recipient_device: afp,
        from_timestamp: now - 20,
        until_timestamp: now,
        expires_at: now + 300,
    };
    alice
        .prepare_group_history_share(group, id, range, now)
        .unwrap();
    alice.advance_group_history_online(id, now).unwrap();
    deliver(&mut alice, &mut bob, now);
    assert_eq!(
        alice.group_history_share_status(id).unwrap().0,
        HistoryShareStatus::Pending
    );
    bob.advance_group_history_online(id, now).unwrap();
    deliver(&mut bob, &mut alice, now);
    alice.advance_group_history_online(id, now).unwrap();
    assert_eq!(
        alice.group_history_share_status(id).unwrap().0,
        HistoryShareStatus::Unavailable
    );
    assert!(alice.shared_group_history(group, None).unwrap().is_empty());
    let id = [142; 32];
    let range = HistoryRange {
        source_device: afp,
        recipient_device: bfp,
        ..range
    };
    alice
        .prepare_group_history_share(group, id, range, now)
        .unwrap();
    alice.advance_group_history_online(id, now).unwrap();
    deliver(&mut alice, &mut bob, now);
    let change = alice
        .prepare_group_change(group, Change::EarlierHistory(false))
        .unwrap();
    alice
        .prepare_group_service_request(group, Some(&change))
        .unwrap();
    alice.submit_group_service_online(group, now).unwrap();
    assert_eq!(
        bob.advance_group_history_online(id, now).unwrap(),
        HistoryShareStatus::Revoked
    );
    assert_eq!(
        alice.advance_group_history_online(id, now).unwrap(),
        HistoryShareStatus::Revoked
    );
    assert!(bob.shared_group_history(group, None).unwrap().is_empty());
    enable(&mut alice, &mut bob, group, now);
    let id = [143; 32];
    alice
        .prepare_group_history_share(group, id, range, now)
        .unwrap();
    alice.advance_group_history_online(id, now).unwrap();
    deliver(&mut alice, &mut bob, now);
    assert_eq!(
        bob.advance_group_history_online(id, now + 301).unwrap(),
        HistoryShareStatus::Unavailable
    );
}
#[test]
fn grants_bind_all_authorization_fields_and_unknown_retention_encodings_fail_closed() {
    let (_dir, _fixture, mut alice, mut bob, group, afp, bfp, now) = setup();
    enable(&mut alice, &mut bob, group, now);
    let id = [144; 32];
    let range = HistoryRange {
        source_device: afp,
        recipient_device: bfp,
        from_timestamp: now - 20,
        until_timestamp: now,
        expires_at: now + 300,
    };
    assert!(bob
        .prepare_group_history_share(group, id, range, now)
        .is_err());
    alice
        .prepare_group_history_share(group, id, range, now)
        .unwrap();
    let bytes = load(&alice.db, &alice.key, &afp, &id).unwrap().grant;
    let state = alice.group_status(group).unwrap().state;
    for n in 8..bytes.len() {
        let mut changed = bytes.clone();
        changed[n] ^= 1;
        if let Ok(g) = Grant::parse(&changed) {
            assert!(g.verify(&state).is_err());
        }
    }
    for n in 0..bytes.len() {
        assert!(Grant::parse(&bytes[..n]).is_err());
    }
    let original = alice.group_message(group, afp, [121; 32]).unwrap();
    let mut unknown = original.plaintext.to_vec();
    unknown[9] = 255;
    assert!(Group::from_bytes(&unknown).is_err());
    let grant = Grant::parse(&bytes).unwrap();
    let descriptor = [
        sigil_protocol::file::DESCRIPTOR_PREFIX.as_slice(),
        &[1; 32],
        &5u64.to_be_bytes(),
        &(sigil_protocol::file::CHUNK_SIZE as u32).to_be_bytes(),
        &[2; 32],
        &[3; 32],
    ]
    .concat();
    let file = sigil_protocol::file::File {
        source: "chat.example",
        name: "expired.bin",
        media_type: "application/octet-stream",
        expires_at: Some(now - 1),
        access: &[4; 32],
        descriptor: &descriptor,
    }
    .to_bytes()
    .unwrap();
    let event = Group {
        message: [5; 32],
        group,
        sender: afp,
        timestamp: now - 10,
        content: Content::File(&file),
    };
    assert!(matches!(
        eligible(&event, &grant, now),
        Err(Error::Obsolete)
    ));
}

#[test]
fn shared_file_download_uses_retained_authorization_and_rejects_expiry() {
    let (dir, _fixture, mut alice, mut bob, group, afp, bfp, now) = setup();
    let peer = alice
        .observe_peer_binding(&bob.own_device_binding().unwrap())
        .unwrap()
        .id;
    alice
        .prepare_group_distribution_online(group, peer, now)
        .unwrap();
    let mut sender = alice
        .open_attachment_cache(&dir.path().join("upload.db"), 8 * 1024 * 1024)
        .unwrap();
    let file = sender
        .prepare_upload(
            5,
            crate::attachments::Metadata {
                name: "shared.bin".into(),
                media_type: "application/octet-stream".into(),
            },
            Some(now + 500),
        )
        .unwrap();
    sender.stage_chunk(file, 0, b"group").unwrap();
    sender.finish_staging(file).unwrap();
    for expected in [
        crate::attachments::UploadStep::Begun,
        crate::attachments::UploadStep::Chunk(0),
        crate::attachments::UploadStep::Published,
    ] {
        assert_eq!(
            alice.upload_attachment_step(&mut sender, file).unwrap(),
            expected
        );
    }
    alice
        .queue_group_file(group, [150; 32], (&mut sender, file), now - 2, now)
        .unwrap();
    enable(&mut alice, &mut bob, group, now);
    let id = [151; 32];
    alice
        .prepare_group_history_share(
            group,
            id,
            HistoryRange {
                source_device: afp,
                recipient_device: bfp,
                from_timestamp: now - 5,
                until_timestamp: now,
                expires_at: now + 600,
            },
            now,
        )
        .unwrap();
    for _ in 0..24 {
        alice.advance_group_history_online(id, now).unwrap();
        deliver(&mut alice, &mut bob, now);
        bob.advance_group_history_online(id, now).unwrap();
        deliver(&mut bob, &mut alice, now);
        if alice.group_history_share_status(id).unwrap().0 == HistoryShareStatus::Complete {
            break;
        }
    }
    assert_eq!(
        alice.group_history_share_status(id).unwrap(),
        (HistoryShareStatus::Complete, 1)
    );
    let history = bob.shared_group_history(group, None).unwrap();
    assert_eq!(history.len(), 1);
    let record = history[0].id;
    let mut receiver = bob
        .open_attachment_cache(&dir.path().join("download.db"), 8 * 1024 * 1024)
        .unwrap();
    assert!(matches!(
        bob.prepare_shared_group_file(&mut receiver, [0; 32], record, now),
        Err(Error::NotFound)
    ));
    assert_eq!(
        bob.prepare_shared_group_file(&mut receiver, group, record, now)
            .unwrap(),
        file
    );
    assert_eq!(
        bob.download_attachment_step(&mut receiver, file, now)
            .unwrap(),
        crate::attachments::DownloadStep::Chunk(0)
    );
    assert_eq!(
        bob.download_attachment_step(&mut receiver, file, now)
            .unwrap(),
        crate::attachments::DownloadStep::Complete
    );
    assert_eq!(
        receiver.completed_chunk(file, 0, now).unwrap().as_slice(),
        b"group"
    );
    assert!(matches!(
        bob.prepare_shared_group_file(&mut receiver, group, record, now + 501),
        Err(Error::Expired)
    ));
    assert!(bob.recovery_records(None).unwrap().is_empty());
}

#[test]
fn expired_history_work_releases_pending_terminal_packets_and_reuses_capacity() {
    let (_dir, _fixture, mut alice, mut bob, group, afp, bfp, now) = setup();
    enable(&mut alice, &mut bob, group, now);
    let range = HistoryRange {
        source_device: bfp,
        recipient_device: afp,
        from_timestamp: now - 20,
        until_timestamp: now,
        expires_at: now + 300,
    };
    let id = [160; 32];
    alice
        .prepare_group_history_share(group, id, range, now)
        .unwrap();
    alice.advance_group_history_online(id, now).unwrap();
    deliver(&mut alice, &mut bob, now);
    assert_eq!(
        bob.advance_group_history_online(id, now).unwrap(),
        HistoryShareStatus::Unavailable
    );
    assert!(load(&bob.db, &bob.key, &bfp, &id).unwrap().active());
    assert_eq!(
        bob.advance_group_history_online(id, now + 301).unwrap(),
        HistoryShareStatus::Unavailable
    );
    assert!(!load(&bob.db, &bob.key, &bfp, &id).unwrap().active());
    assert_eq!(
        bob.db
            .query_row("SELECT count(*) FROM group_key_outbox", [], |r| r
                .get::<_, u32>(0))
            .unwrap(),
        0
    );
    alice.advance_group_history_online(id, now + 301).unwrap();
    for n in 161..177 {
        alice
            .prepare_group_history_share(group, [n; 32], range, now)
            .unwrap();
    }
    assert!(matches!(
        alice.prepare_group_history_share(group, [177; 32], range, now),
        Err(Error::Limit)
    ));
    for n in 161..177 {
        alice
            .advance_group_history_online([n; 32], now + 301)
            .unwrap();
    }
    // Local expiry is simulated; HTTPS uses the fixture clock.
    alice
        .prepare_group_history_share(
            group,
            [177; 32],
            HistoryRange {
                expires_at: now + 600,
                ..range
            },
            now,
        )
        .unwrap();
}

#[test]
fn a_nonadmin_supplier_shares_only_under_the_admin_grant_without_advancing_sender_keys() {
    let (_dir, _fixture, mut alice, mut bob, group, afp, bfp, now) = setup();
    enable(&mut alice, &mut bob, group, now);
    let a = bob
        .observe_peer_binding(&alice.own_device_binding().unwrap())
        .unwrap()
        .id;
    bob.prepare_group_distribution_online(group, a, now)
        .unwrap();
    bob.queue_group_text(
        group,
        [145; 32],
        "history supplied by a member",
        now - 5,
        now,
    )
    .unwrap();
    let id = [146; 32];
    let range = HistoryRange {
        source_device: bfp,
        recipient_device: afp,
        from_timestamp: now - 20,
        until_timestamp: now,
        expires_at: now + 600,
    };
    alice
        .prepare_group_history_share(group, id, range, now)
        .unwrap();
    alice.advance_group_history_online(id, now).unwrap();
    deliver(&mut alice, &mut bob, now);
    for _ in 0..12 {
        bob.resume_group_history_online(now)
            .unwrap()
            .into_iter()
            .for_each(|v| {
                v.result.unwrap();
            });
        deliver(&mut bob, &mut alice, now);
        alice
            .resume_group_history_online(now)
            .unwrap()
            .into_iter()
            .for_each(|v| {
                v.result.unwrap();
            });
        deliver(&mut alice, &mut bob, now);
        if bob.group_history_share_status(id).unwrap().0 == HistoryShareStatus::Complete {
            break;
        }
    }
    assert_eq!(
        alice.group_history_share_status(id).unwrap(),
        (HistoryShareStatus::Complete, 1)
    );
    assert_eq!(
        bob.group_history_share_status(id).unwrap(),
        (HistoryShareStatus::Complete, 1)
    );
    let records = alice.shared_group_history(group, None).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].source_device, bfp);
    assert_eq!(
        Group::from_bytes(&records[0].plaintext)
            .unwrap()
            .content
            .text()
            .unwrap(),
        "history supplied by a member"
    );
    let state = bob.group_status(group).unwrap().state;
    let tx = bob.db.transaction().unwrap();
    assert_eq!(
        keys::load_sender(&tx, &bob.key, &bfp, &state)
            .unwrap()
            .unwrap()
            .0
            .counter(),
        1
    );
}
#[test]
fn deleting_a_message_mid_transfer_prevents_completion_of_its_partial_copy() {
    let (_dir, _fixture, mut alice, mut bob, group, afp, bfp, now) = setup();
    enable(&mut alice, &mut bob, group, now);
    let id = [147; 32];
    let range = HistoryRange {
        source_device: afp,
        recipient_device: bfp,
        from_timestamp: now - 20,
        until_timestamp: now,
        expires_at: now + 600,
    };
    alice
        .prepare_group_history_share(group, id, range, now)
        .unwrap();
    alice.advance_group_history_online(id, now).unwrap();
    deliver(&mut alice, &mut bob, now);
    for _ in 0..8 {
        alice.advance_group_history_online(id, now).unwrap();
        deliver(&mut alice, &mut bob, now);
        bob.advance_group_history_online(id, now).unwrap();
        deliver(&mut bob, &mut alice, now);
        if !load(&bob.db, &bob.key, &bfp, &id)
            .unwrap()
            .buffer
            .is_empty()
        {
            break;
        }
    }
    assert_eq!(
        load(&bob.db, &bob.key, &bfp, &id).unwrap().buffer.len(),
        CHUNK
    );
    assert!(bob.shared_group_history(group, None).unwrap().is_empty());
    let message = alice.group_message(group, afp, [121; 32]).unwrap();
    let mut record = alice
        .recovery_record(messages::group_event_history_id(&message.event().unwrap()))
        .unwrap();
    record.revision += 1;
    record.content = sigil_crypto::recovery::Content::Deleted;
    alice.retain_recovery_record(&record).unwrap();
    assert_eq!(
        alice.advance_group_history_online(id, now).unwrap(),
        HistoryShareStatus::Unavailable
    );
    deliver(&mut alice, &mut bob, now);
    assert_eq!(
        bob.advance_group_history_online(id, now).unwrap(),
        HistoryShareStatus::Unavailable
    );
    assert!(load(&bob.db, &bob.key, &bfp, &id)
        .unwrap()
        .buffer
        .is_empty());
    assert!(bob.shared_group_history(group, None).unwrap().is_empty());
}

#[test]
fn expired_distribution_preserves_pending_messages_for_a_working_receiver() {
    let (dir, _fixture, mut alice, mut bob, group, afp, _, now) = setup();
    let peer = alice
        .observe_peer_binding(&bob.own_device_binding().unwrap())
        .unwrap()
        .id;
    let original = alice
        .prepare_group_distribution_online(group, peer, now)
        .unwrap();
    deliver(&mut alice, &mut bob, now);
    assert!(bob.group_receiver_ready(group, afp).unwrap());
    alice
        .queue_group_text(group, [200; 32], "must survive recovery", now, now)
        .unwrap();
    let tx = alice.db.transaction().unwrap();
    let (session, _) = keys::job(&tx, &alice.key, &afp, &group, &original)
        .unwrap()
        .unwrap();
    let raw: Vec<u8> = tx
        .query_row(
            "SELECT metadata FROM deliveries WHERE id=?1",
            [original.as_slice()],
            |r| r.get(0),
        )
        .unwrap();
    let mut raw = alice
        .key
        .open(&raw, &crate::binding(7, &session, &original))
        .unwrap();
    raw[32..].copy_from_slice(&(now - 1).to_be_bytes());
    tx.execute(
        "UPDATE deliveries SET metadata=?1 WHERE id=?2",
        (
            alice
                .key
                .seal(&raw, &crate::binding(7, &session, &original))
                .unwrap(),
            original.as_slice(),
        ),
    )
    .unwrap();
    tx.commit().unwrap();
    alice
        .group_key_recovery_work_online(group, peer, now)
        .unwrap();
    let cancelled: u32 = alice
        .db
        .query_row(
            "SELECT count(*) FROM group_delivery WHERE status=3",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(bob.group_receiver_ready(group, afp).unwrap());
    assert!(!alice.group_status(group).unwrap().state.earlier_history());
    assert_eq!(cancelled,0,"automatic recovery cancelled a valid unexpired message although its recipient has the original receiver key");

    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    alice
        .group_key_recovery_work_online(group, peer, now)
        .unwrap();
    let sent = alice.resume_group_outbound_online(now).unwrap();
    assert!(!sent.is_empty());
    assert!(sent.iter().all(|a| a.result.is_ok()), "{sent:?}");
    let received = bob.receive_mailbox_online(now).unwrap();
    assert!(received.iter().any(|a| matches!(&a.result, Ok(crate::MailboxEvent::GroupText(m)) if m.text().unwrap().body == "must survive recovery")));
}

#[test]
fn history_import_rechecks_revocation_committed_after_preflight() {
    let (dir, _fixture, mut alice, mut bob, group, afp, bfp, now) = setup();
    enable(&mut alice, &mut bob, group, now);
    let id = [202; 32];
    alice
        .prepare_group_history_share(
            group,
            id,
            HistoryRange {
                source_device: afp,
                recipient_device: bfp,
                from_timestamp: now - 101,
                until_timestamp: now - 99,
                expires_at: now + 600,
            },
            now,
        )
        .unwrap();
    for _ in 0..6 {
        alice.advance_group_history_online(id, now).unwrap();
        deliver(&mut alice, &mut bob, now);
        if load(&bob.db, &bob.key, &bfp, &id)
            .unwrap()
            .incoming
            .is_some()
        {
            break;
        }
    }
    let incoming = load(&bob.db, &bob.key, &bfp, &id)
        .unwrap()
        .incoming
        .unwrap();
    assert_eq!(Frame::parse(&incoming).unwrap().kind, 1);
    assert!(bob.shared_group_history(group, None).unwrap().is_empty());
    let mut other = open(&dir.path().join("bob.db"));
    other
        .db
        .busy_timeout(std::time::Duration::from_secs(2))
        .unwrap();

    assert!(bob.sync_group_service_online(group, now).unwrap());
    let grant = Grant::parse(&load(&bob.db, &bob.key, &bfp, &id).unwrap().grant).unwrap();
    grant
        .verify(&bob.group_status(group).unwrap().state)
        .unwrap();
    let change = alice
        .prepare_group_change(group, Change::EarlierHistory(false))
        .unwrap();
    alice
        .prepare_group_service_request(group, Some(&change))
        .unwrap();
    alice.submit_group_service_online(group, now).unwrap();
    other.sync_group_service_online(group, now).unwrap();
    assert!(!other.group_status(group).unwrap().state.earlier_history());
    let work = bob.import_group_history(id, bfp, now).unwrap();
    assert_eq!(work.status, HistoryShareStatus::Revoked);
    assert!(work.incoming.is_none() && work.packet.is_none() && work.buffer.is_empty());
    assert!(bob.shared_group_history(group, None).unwrap().is_empty());
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    assert_eq!(
        bob.advance_group_history_online(id, now).unwrap(),
        HistoryShareStatus::Revoked
    );
    assert!(bob.shared_group_history(group, None).unwrap().is_empty());
}
