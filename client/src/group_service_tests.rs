use super::*;

fn open(path: &std::path::Path) -> ClientStore {
    ClientStore::open(
        path,
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap()
}
pub(crate) fn enable(path: &std::path::Path) -> (Vec<u8>, Id) {
    let mut server = sigil_server::store::Store::open(path).unwrap();
    let config = server
        .configure_groups(sigil_protocol::groups::Configure {
            expected_revision: 0,
            enabled: true,
            storage_limit_bytes: 1024 * 1024,
        })
        .unwrap();
    let raw = decode(config.authority.as_deref().unwrap(), 431).unwrap();
    let fingerprint =
        authority_fingerprint(raw[raw.len() - 96..raw.len() - 64].try_into().unwrap());
    (raw, fingerprint)
}
#[test]
fn group_service_restarts_after_server_acceptance_and_local_acknowledgement_failure() {
    let (dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    alice.publish_device_binding_online().unwrap();
    bob.publish_device_binding_online().unwrap();
    let (profile, fingerprint) = enable(&dir.path().join("server.db"));
    let group = alice.create_group(fingerprint).unwrap();
    let genesis = alice.group_genesis(group).unwrap();
    let (a, _) = crate::incoming::tests::trust(&mut alice, &mut bob);
    bob.accept_group_genesis(a, &genesis).unwrap();
    for store in [&mut alice, &mut bob] {
        store
            .pin_group_service(group, &profile, Zeroizing::new([7; 32]))
            .unwrap();
    }
    alice.prepare_group_service_request(group, None).unwrap();
    alice.submit_group_service_online(group, now).unwrap();
    assert!(!alice.group_service_request_pending(group).unwrap());
    let member = Member::new([2; 32], Role::Admin, &[bob.own_device_binding().unwrap()]).unwrap();
    let proposal = alice
        .prepare_group_change(group, Change::Add(member))
        .unwrap();
    let proposal = bob.approve_group_proposal(group, &proposal).unwrap();
    let proposal = alice.approve_group_proposal(group, &proposal).unwrap();
    alice
        .prepare_group_service_request(group, Some(&proposal))
        .unwrap();
    // Real authority accepts first; local membership transaction then fails.
    alice.db.execute_batch("CREATE TRIGGER synthetic_failure BEFORE UPDATE ON groups BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(alice.submit_group_service_online(group, now).is_err());
    assert_eq!(alice.group_status(group).unwrap().state.revision(), 0);
    let server = rusqlite::Connection::open(dir.path().join("server.db")).unwrap();
    assert_eq!(
        server
            .query_row("SELECT revision FROM private_groups", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    alice
        .db
        .execute_batch("DROP TRIGGER synthetic_failure")
        .unwrap();
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    // This retry commits membership but loses its outbox acknowledgement.
    alice.db.execute_batch("CREATE TRIGGER synthetic_failure BEFORE UPDATE ON group_service_outbox BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(alice.submit_group_service_online(group, now).is_err());
    assert_eq!(alice.group_status(group).unwrap().state.revision(), 1);
    assert!(alice.group_service_request_pending(group).unwrap());
    alice
        .db
        .execute_batch("DROP TRIGGER synthetic_failure")
        .unwrap();
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    assert_eq!(
        alice.submit_group_service_online(group, now).unwrap(),
        CommitResult::Duplicate
    );
    assert!(!alice.group_service_request_pending(group).unwrap());
    assert!(bob.sync_group_service_online(group, now).unwrap());
    assert_eq!(
        bob.group_status(group).unwrap().state.head(),
        alice.group_status(group).unwrap().state.head()
    );
    assert_eq!(
        server
            .query_row("SELECT count(*) FROM private_group_commits", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    alice
        .prepare_group_service_request(group, Some(&proposal))
        .unwrap();
    assert!(!alice.group_service_request_pending(group).unwrap());
    let losing = alice
        .prepare_group_change(group, Change::EarlierHistory(true))
        .unwrap();
    alice
        .prepare_group_service_request(group, Some(&losing))
        .unwrap();
    let winning = bob
        .prepare_group_change(group, Change::RefreshKeys)
        .unwrap();
    bob.prepare_group_service_request(group, Some(&winning))
        .unwrap();
    bob.submit_group_service_online(group, now).unwrap();
    assert!(matches!(
        alice.submit_group_service_online(group, now),
        Err(Error::Network(crate::network::Error::Status {
            code: 409,
            ..
        }))
    ));
    assert!(alice.sync_group_service_online(group, now).unwrap());
    assert!(!alice.group_service_request_pending(group).unwrap());
    assert!(matches!(
        alice.submit_group_service_online(group, now),
        Err(Error::Obsolete)
    ));
    let replacement = alice
        .prepare_group_change(group, Change::EarlierHistory(true))
        .unwrap();
    alice
        .prepare_group_service_request(group, Some(&replacement))
        .unwrap();
    alice.submit_group_service_online(group, now).unwrap();
    assert_eq!(alice.group_status(group).unwrap().state.revision(), 3);
    assert!(bob.sync_group_service_online(group, now).unwrap());
    let (_, b) = crate::incoming::tests::trust(&mut alice, &mut bob);
    crate::incoming::tests::start(&mut alice, b, now);
    bob.accept_delivery(&crate::incoming::tests::next(&bob))
        .unwrap();
    bob.acknowledge_incoming_online().unwrap();
    alice.prepare_group_distribution(group, b, now).unwrap();
    let refresh = bob
        .prepare_group_change(group, Change::RefreshKeys)
        .unwrap();
    bob.prepare_group_service_request(group, Some(&refresh))
        .unwrap();
    bob.submit_group_service_online(group, now).unwrap();
    let mailbox_count = || {
        server
            .query_row("SELECT count(*) FROM mailbox", [], |r| r.get::<_, i64>(0))
            .unwrap()
    };
    let before = mailbox_count();
    assert!(matches!(
        alice.send_pending_online([3; 32], now),
        Err(Error::Obsolete)
    ));
    assert_eq!(mailbox_count(), before);
    assert_eq!(alice.group_status(group).unwrap().state.revision(), 4);
    alice.prepare_group_distribution(group, b, now).unwrap();
    alice.send_pending_online([3; 32], now).unwrap();
    let incoming = bob
        .accept_delivery(&crate::incoming::tests::next(&bob))
        .unwrap();
    assert!(incoming.distribution().unwrap().is_some());
    bob.acknowledge_incoming_online().unwrap();
    alice
        .queue_group_text(group, [81; 32], "synthetic offline message", now, now)
        .unwrap();
    let refresh = bob
        .prepare_group_change(group, Change::RefreshKeys)
        .unwrap();
    bob.prepare_group_service_request(group, Some(&refresh))
        .unwrap();
    bob.submit_group_service_online(group, now).unwrap();
    let before = mailbox_count();
    let attempts = alice.resume_group_outbound_online(now).unwrap();
    assert_eq!(attempts.len(), 1);
    assert!(matches!(
        attempts[0].result,
        Ok(GroupDeliveryStatus::Cancelled)
    ));
    assert_eq!(mailbox_count(), before);
}

#[test]
fn private_envelopes_survive_lost_receipts_migration_and_reject_metadata_substitution() {
    let (dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    alice.publish_device_binding_online().unwrap();
    bob.publish_device_binding_online().unwrap();
    let (profile, fingerprint) = enable(&dir.path().join("server.db"));
    let group = alice.create_group(fingerprint).unwrap();
    let (a, b) = crate::incoming::tests::trust(&mut alice, &mut bob);
    bob.accept_group_genesis(a, &alice.group_genesis(group).unwrap())
        .unwrap();
    for store in [&mut alice, &mut bob] {
        store
            .pin_group_service(group, &profile, Zeroizing::new([7; 32]))
            .unwrap();
    }
    alice.prepare_group_service_request(group, None).unwrap();
    alice.submit_group_service_online(group, now).unwrap();
    let proposal = alice
        .prepare_group_change(
            group,
            Change::Add(
                Member::new([2; 32], Role::Member, &[bob.own_device_binding().unwrap()]).unwrap(),
            ),
        )
        .unwrap();
    let proposal = bob.approve_group_proposal(group, &proposal).unwrap();
    alice
        .prepare_group_service_request(group, Some(&proposal))
        .unwrap();
    alice.submit_group_service_online(group, now).unwrap();
    bob.sync_group_service_online(group, now).unwrap();
    crate::incoming::tests::start(&mut alice, b, now);
    bob.accept_delivery(&crate::incoming::tests::next(&bob))
        .unwrap();
    bob.acknowledge_incoming_online().unwrap();
    alice.prepare_group_distribution(group, b, now).unwrap();
    alice.send_pending_online([3; 32], now).unwrap();
    bob.accept_delivery_online(&crate::incoming::tests::next(&bob), now)
        .unwrap();
    bob.acknowledge_incoming_online().unwrap();
    let body = "x".repeat(sigil_protocol::initial::MAX_PLAINTEXT - 118);
    alice
        .queue_group_text(group, [93; 32], &body, now, now)
        .unwrap();
    alice.db.execute_batch("CREATE TRIGGER fail_envelope_receipt BEFORE UPDATE ON group_delivery BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    let attempts = alice.resume_group_outbound_online(now).unwrap();
    assert!(attempts[0].result.is_err());
    let before = crate::incoming::tests::next(&bob);
    assert!(super::super::envelope::is_envelope(&before.payload));
    assert!(!before.payload.contains(&hex(&group)));
    assert!(before.payload.len() <= sigil_protocol::mailbox::MAX_PAYLOAD_HEX);
    alice
        .db
        .execute_batch("DROP TRIGGER fail_envelope_receipt")
        .unwrap();
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    let mut accepted = false;
    for _ in 0..3 {
        for attempt in alice.resume_group_outbound_online(now).unwrap() {
            if matches!(attempt.result, Ok(GroupDeliveryStatus::Accepted)) {
                accepted = true;
            }
        }
    }
    assert!(accepted);
    assert_eq!(crate::incoming::tests::next(&bob).payload, before.payload);
    assert_eq!(
        alice
            .db
            .query_row("SELECT count(*) FROM group_envelopes", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let unwrapped = bob.unwrap_group_delivery(&before).unwrap().1;
    assert!(matches!(
        bob.accept_group_delivery(&unwrapped),
        Err(Error::InvalidEvent)
    ));
    // Migrate an authenticated legacy pin; a corrupt pin rolls the migration back.
    crate::test_schema::rewind(&bob.db, 55);
    bob.db.pragma_update(None, "user_version", 55).unwrap();
    let saved: Vec<u8> = bob
        .db
        .query_row("SELECT state FROM group_service", [], |r| r.get(0))
        .unwrap();
    let mut corrupt = saved.clone();
    corrupt[0] ^= 1;
    bob.db
        .execute("UPDATE group_service SET state=?1", [corrupt])
        .unwrap();
    drop(bob);
    assert!(ClientStore::open(
        &dir.path().join("bob.db"),
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap()
    )
    .is_err());
    let db = rusqlite::Connection::open(dir.path().join("bob.db")).unwrap();
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        55
    );
    db.execute("UPDATE group_service SET state=?1", [saved])
        .unwrap();
    drop(db);
    let mut bob = open(&dir.path().join("bob.db"));
    let mut tampered = sigil_protocol::mailbox::Delivery {
        origin: None,
        sequence: before.sequence,
        sender_device: before.sender_device.clone(),
        message_id: before.message_id.clone(),
        payload: before.payload.clone(),
        expires_at: before.expires_at + 1,
    };
    assert!(bob.accept_group_delivery_online(&tampered, now).is_err());
    tampered.expires_at = before.expires_at;
    tampered.sender_device = "00".repeat(32);
    assert!(bob.accept_group_delivery_online(&tampered, now).is_err());
    tampered.sender_device = before.sender_device.clone();
    let last = tampered.payload.len() - 2;
    let byte = u8::from_str_radix(&tampered.payload[last..], 16).unwrap() ^ 1;
    tampered
        .payload
        .replace_range(last.., &format!("{byte:02x}"));
    assert!(bob.accept_group_delivery_online(&tampered, now).is_err());
    assert_eq!(
        bob.db
            .query_row("SELECT count(*) FROM group_messages", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let received = bob.receive_mailbox_online(now).unwrap();
    assert!(
        matches!(&received[0].result,Ok(crate::MailboxEvent::GroupText(message)) if message.event().unwrap().content.text().unwrap()==body)
    );
    assert_eq!(bob.acknowledge_incoming_online().unwrap(), 1);
    // Reproduce a schema-55 send accepted remotely but not acknowledged locally.
    // Migration must retry those original bytes, not replace them with a wrapper.
    alice
        .queue_group_text(group, [94; 32], "synthetic legacy retry", now, now)
        .unwrap();
    let (stored,packet,delivery):(Vec<u8>,Vec<u8>,Vec<u8>)=alice.db.query_row("SELECT m.id,m.packet,d.id FROM group_messages m JOIN group_delivery d ON d.message=m.id WHERE d.status=0",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    let own = device_fingerprint(&alice.own_device_binding().unwrap()).unwrap();
    let raw = alice
        .key
        .open(&packet, &crate::binding(51, &own, &stored))
        .unwrap();
    let request = sigil_protocol::mailbox::Submit {
        recipient_device: bob.connection_session().unwrap().unwrap().device_id,
        message_id: hex(&delivery),
        payload: hex(&raw),
        expires_at: now + 604740,
    };
    alice.connected_client().unwrap().submit(&request).unwrap();
    crate::test_schema::rewind(&alice.db, 55);
    alice.db.pragma_update(None, "user_version", 55).unwrap();
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    let mut accepted = false;
    for _ in 0..3 {
        for attempt in alice.resume_group_outbound_online(now).unwrap() {
            if matches!(attempt.result, Ok(GroupDeliveryStatus::Accepted)) {
                accepted = true;
            }
        }
    }
    assert!(accepted);
    let legacy = crate::incoming::tests::next(&bob);
    assert_eq!(legacy.payload, request.payload);
    assert!(bob.accept_group_delivery_online(&legacy, now).is_ok());
    bob.acknowledge_incoming_online().unwrap();
    let refresh = alice
        .prepare_group_change(group, Change::RefreshKeys)
        .unwrap();
    alice
        .prepare_group_service_request(group, Some(&refresh))
        .unwrap();
    alice.submit_group_service_online(group, now).unwrap();
    bob.sync_group_service_online(group, now).unwrap();
    alice.prepare_group_distribution(group, b, now).unwrap();
    alice.send_pending_online([3; 32], now).unwrap();
    bob.accept_delivery_online(&crate::incoming::tests::next(&bob), now)
        .unwrap();
    bob.acknowledge_incoming_online().unwrap();
    alice
        .queue_group_text(group, [95; 32], "synthetic private cutover", now, now)
        .unwrap();
    let mut accepted = false;
    for _ in 0..3 {
        for attempt in alice.resume_group_outbound_online(now).unwrap() {
            if matches!(attempt.result, Ok(GroupDeliveryStatus::Accepted)) {
                accepted = true;
            }
        }
    }
    assert!(accepted);
    let private = crate::incoming::tests::next(&bob);
    assert!(super::super::envelope::is_envelope(&private.payload));
    let stripped = bob.unwrap_group_delivery(&private).unwrap().1;
    assert!(matches!(
        bob.accept_group_delivery(&stripped),
        Err(Error::InvalidEvent)
    ));
    assert!(bob.accept_group_delivery_online(&private, now).is_ok());
}

#[test]
fn member_leave_relay_survives_restart_and_lost_local_receipt_after_removal() {
    let (dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    alice.publish_device_binding_online().unwrap();
    bob.publish_device_binding_online().unwrap();
    let (profile, fingerprint) = enable(&dir.path().join("server.db"));
    let group = alice.create_group(fingerprint).unwrap();
    let (a, _) = crate::incoming::tests::trust(&mut alice, &mut bob);
    bob.accept_group_genesis(a, &alice.group_genesis(group).unwrap())
        .unwrap();
    for store in [&mut alice, &mut bob] {
        store
            .pin_group_service(group, &profile, Zeroizing::new([7; 32]))
            .unwrap();
    }
    alice.prepare_group_service_request(group, None).unwrap();
    alice.submit_group_service_online(group, now).unwrap();
    let proposal = alice
        .prepare_group_change(
            group,
            Change::Add(
                Member::new([2; 32], Role::Member, &[bob.own_device_binding().unwrap()]).unwrap(),
            ),
        )
        .unwrap();
    let proposal = bob.approve_group_proposal(group, &proposal).unwrap();
    alice
        .prepare_group_service_request(group, Some(&proposal))
        .unwrap();
    alice.submit_group_service_online(group, now).unwrap();
    bob.sync_group_service_online(group, now).unwrap();
    let leave = bob
        .prepare_group_change(group, Change::Remove([2; 32]))
        .unwrap();
    bob.prepare_group_service_request(group, Some(&leave))
        .unwrap();
    assert!(!bob.submit_group_relay_online(group, now).unwrap());
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    assert!(!bob.submit_group_relay_online(group, now).unwrap());
    let proposals = alice
        .group_relay_proposals_online(group, None, now)
        .unwrap();
    assert_eq!(proposals.len(), 1);
    let mut invalid = proposals[0].clone();
    invalid.author = "00".repeat(64);
    assert!(alice.prepare_group_relay_commit(group, &invalid).is_err());
    alice
        .prepare_group_relay_commit(group, &proposals[0])
        .unwrap();
    assert_eq!(alice.group_status(group).unwrap().state.revision(), 1);
    alice.submit_group_service_online(group, now).unwrap();
    let refresh = alice
        .prepare_group_change(group, Change::RefreshKeys)
        .unwrap();
    alice
        .prepare_group_service_request(group, Some(&refresh))
        .unwrap();
    alice.submit_group_service_online(group, now).unwrap();
    assert!(bob.sync_group_service_online(group, now).is_err());
    bob.db.execute_batch("CREATE TRIGGER fail_relay_ack BEFORE UPDATE ON group_service_outbox BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(bob.submit_group_relay_online(group, now).is_err());
    assert_eq!(bob.group_status(group).unwrap().state.revision(), 2);
    bob.db.execute_batch("DROP TRIGGER fail_relay_ack").unwrap();
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    assert!(bob.submit_group_relay_online(group, now).unwrap());
    assert!(!bob.group_service_request_pending(group).unwrap());
    assert!(bob
        .queue_group_text(group, [91; 32], "synthetic removed member", now, now)
        .is_err());
    assert!(alice
        .group_relay_proposals_online(group, None, now)
        .unwrap()
        .is_empty());
}

#[test]
fn online_reception_refreshes_membership_without_committing_failed_preflight() {
    let (dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    alice.publish_device_binding_online().unwrap();
    bob.publish_device_binding_online().unwrap();
    let (profile, fingerprint) = enable(&dir.path().join("server.db"));
    let group = alice.create_group(fingerprint).unwrap();
    let (a, b) = crate::incoming::tests::trust(&mut alice, &mut bob);
    bob.accept_group_genesis(a, &alice.group_genesis(group).unwrap())
        .unwrap();
    for store in [&mut alice, &mut bob] {
        store
            .pin_group_service(group, &profile, Zeroizing::new([7; 32]))
            .unwrap();
    }
    alice.prepare_group_service_request(group, None).unwrap();
    alice.submit_group_service_online(group, now).unwrap();
    let member = Member::new([2; 32], Role::Member, &[bob.own_device_binding().unwrap()]).unwrap();
    let proposal = alice
        .prepare_group_change(group, Change::Add(member))
        .unwrap();
    let proposal = bob.approve_group_proposal(group, &proposal).unwrap();
    alice
        .prepare_group_service_request(group, Some(&proposal))
        .unwrap();
    alice.submit_group_service_online(group, now).unwrap();
    // Bob has approved the join but has not received the ordering receipt.
    assert_eq!(bob.group_status(group).unwrap().state.revision(), 0);
    crate::incoming::tests::start(&mut alice, b, now);
    bob.accept_delivery(&crate::incoming::tests::next(&bob))
        .unwrap();
    bob.acknowledge_incoming_online().unwrap();
    alice.prepare_group_distribution(group, b, now).unwrap();
    alice.send_pending_online([3; 32], now).unwrap();
    let delivery = crate::incoming::tests::next(&bob);
    let snapshots = |store: &ClientStore| {
        store
            .db
            .prepare("SELECT state FROM sessions ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get::<_, Vec<u8>>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    let before = snapshots(&bob);
    let mut server = sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap();
    let config = server.group_configuration().unwrap();
    let disabled = server
        .configure_groups(sigil_protocol::groups::Configure {
            expected_revision: config.revision,
            enabled: false,
            storage_limit_bytes: config.storage_limit_bytes,
        })
        .unwrap();
    assert!(bob.accept_delivery_online(&delivery, now).is_err());
    assert_eq!(snapshots(&bob), before);
    assert_eq!(
        bob.db
            .query_row("SELECT count(*) FROM group_controls", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    server
        .configure_groups(sigil_protocol::groups::Configure {
            expected_revision: disabled.revision,
            enabled: true,
            storage_limit_bytes: disabled.storage_limit_bytes,
        })
        .unwrap();
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    let received = bob.receive_mailbox_online(now).unwrap();
    assert!(matches!(
        received[0].result,
        Ok(crate::MailboxEvent::GroupDistribution(_))
    ));
    assert_eq!(bob.group_status(group).unwrap().state.revision(), 1);
    assert!(bob
        .group_receiver_ready(
            group,
            device_fingerprint(&alice.own_device_binding().unwrap()).unwrap()
        )
        .unwrap());
    bob.acknowledge_incoming_online().unwrap();
    alice
        .queue_group_text(group, [87; 32], "synthetic delayed old epoch", now, now)
        .unwrap();
    assert!(matches!(
        alice.resume_group_outbound_online(now).unwrap()[0].result,
        Ok(GroupDeliveryStatus::Accepted)
    ));
    let old = crate::incoming::tests::next(&bob);
    let refresh = alice
        .prepare_group_change(group, Change::RefreshKeys)
        .unwrap();
    alice
        .prepare_group_service_request(group, Some(&refresh))
        .unwrap();
    alice.submit_group_service_online(group, now).unwrap();
    assert!(matches!(
        bob.accept_group_delivery_online(&old, now),
        Err(Error::Unprepared)
    ));
    assert_eq!(bob.group_status(group).unwrap().state.revision(), 2);
    assert_eq!(
        bob.db
            .query_row("SELECT count(*) FROM group_messages", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        bob.db
            .query_row("SELECT count(*) FROM group_incoming", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn group_service_pins_reject_profile_forks_master_replacement_and_record_transplants() {
    let (_dir, _fixture, mut alice, _bob, _now) = crate::claims::tests::pair();
    let identity = IdentityKey::generate().unwrap();
    let issuer = sigil_crypto::private_credentials::Issuer::generate().unwrap();
    let profile = Authority::sign("chat.example", 1, issuer.public(), &identity).unwrap();
    let group = alice.create_group(profile.fingerprint()).unwrap();
    alice
        .pin_group_service(group, &profile.to_bytes(), Zeroizing::new([7; 32]))
        .unwrap();
    alice
        .pin_group_service(group, &profile.to_bytes(), Zeroizing::new([7; 32]))
        .unwrap();
    assert!(alice
        .pin_group_service(group, &profile.to_bytes(), Zeroizing::new([8; 32]))
        .is_err());
    let fork = Authority::sign(
        "chat.example",
        1,
        sigil_crypto::private_credentials::Issuer::generate()
            .unwrap()
            .public(),
        &identity,
    )
    .unwrap();
    assert!(alice
        .pin_group_service(group, &fork.to_bytes(), Zeroizing::new([7; 32]))
        .is_err());
    let rotation = Authority::sign("chat.example", 2, issuer.public(), &identity).unwrap();
    assert!(alice
        .pin_group_service(group, &rotation.to_bytes(), Zeroizing::new([7; 32]))
        .is_err());
    alice.prepare_group_service_request(group, None).unwrap();
    let before: Vec<u8> = alice
        .db
        .query_row(
            "SELECT state FROM group_service_outbox WHERE group_id=?1",
            [group.as_slice()],
            |r| r.get(0),
        )
        .unwrap();
    alice.prepare_group_service_request(group, None).unwrap();
    let after: Vec<u8> = alice
        .db
        .query_row(
            "SELECT state FROM group_service_outbox WHERE group_id=?1",
            [group.as_slice()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(before, after);
    let other = alice.create_group(profile.fingerprint()).unwrap();
    alice
        .pin_group_service(other, &profile.to_bytes(), Zeroizing::new([7; 32]))
        .unwrap();
    alice
        .db
        .execute(
            "INSERT INTO group_service_outbox VALUES(?1,?2)",
            (other.as_slice(), before),
        )
        .unwrap();
    assert!(alice.group_service_request_pending(other).is_err());
}

#[test]
fn background_group_work_grants_admission_installs_keys_and_orders_member_leave() {
    let (dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    alice.publish_device_binding_online().unwrap();
    bob.publish_device_binding_online().unwrap();
    let (profile, authority) = enable(&dir.path().join("server.db"));
    let group = alice.create_group(authority).unwrap();
    let genesis = alice.group_genesis(group).unwrap();
    let (a, b) = crate::incoming::tests::trust(&mut alice, &mut bob);
    bob.accept_group_genesis(a, &genesis).unwrap();
    for client in [&mut alice, &mut bob] {
        client
            .pin_group_service(group, &profile, Zeroizing::new([71; 32]))
            .unwrap();
    }
    alice.prepare_group_service_request(group, None).unwrap();
    alice.submit_group_service_online(group, now).unwrap();
    let member = Member::new([2; 32], Role::Member, &[bob.own_device_binding().unwrap()]).unwrap();
    let proposal = alice
        .prepare_group_change(group, Change::Add(member))
        .unwrap();
    let proposal = bob.approve_group_proposal(group, &proposal).unwrap();
    alice
        .prepare_group_service_request(group, Some(&proposal))
        .unwrap();
    alice.submit_group_service_online(group, now).unwrap();
    // Bob learns the join from ordinary background work, not a direct sync call.
    alice.db.execute("DELETE FROM peers", []).unwrap();
    bob.db.execute("DELETE FROM peers", []).unwrap();
    let server = rusqlite::Connection::open(dir.path().join("server.db")).unwrap();
    server.execute("DELETE FROM allowed_senders", []).unwrap();
    let clock = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    };
    for _ in 0..10 {
        alice.sync_step_online(clock());
        bob.sync_step_online(clock());
        if alice.group_sender_ready(group).unwrap_or(false)
            && bob.group_sender_ready(group).unwrap_or(false)
        {
            break;
        }
        // Exercise the production rate limiter at the normal worker cadence.
        std::thread::sleep(std::time::Duration::from_secs(5));
    }
    assert_eq!(bob.group_status(group).unwrap().state.revision(), 1);
    assert!(alice.group_sender_ready(group).unwrap());
    assert!(bob.group_sender_ready(group).unwrap());
    assert!(!alice.peer(b).unwrap().trusted && !bob.peer(a).unwrap().trusted);
    assert_eq!(
        server
            .query_row("SELECT count(*) FROM allowed_senders", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    alice
        .queue_group_text(group, [94; 32], "background group delivery", now, now)
        .unwrap();
    let mut delivered = false;
    for _ in 0..4 {
        std::thread::sleep(std::time::Duration::from_secs(5));
        alice.sync_step_online(clock());
        let step = bob.sync_step_online(clock());
        delivered |= step
            .incoming
            .iter()
            .any(|v| matches!(v.result, Ok(crate::MailboxEvent::GroupText(_))));
        if delivered {
            break;
        }
    }
    assert!(delivered);
    // Group membership does not authorize a direct initial message.
    alice
        .confirm_peer(b, alice.peer(b).unwrap().fingerprint)
        .unwrap();
    crate::incoming::tests::start(&mut alice, b, now);
    let direct = crate::incoming::tests::next(&bob);
    let sessions: i64 = bob
        .db
        .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert!(matches!(
        bob.accept_delivery_online(&direct, now),
        Err(Error::Unprepared)
    ));
    assert_eq!(
        bob.db
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        sessions
    );
    let leave = bob
        .prepare_group_change(group, Change::Remove([2; 32]))
        .unwrap();
    bob.prepare_group_service_request(group, Some(&leave))
        .unwrap();
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    for _ in 0..8 {
        std::thread::sleep(std::time::Duration::from_secs(5));
        bob.sync_step_online(clock());
        alice.sync_step_online(clock());
        if !bob.group_service_request_pending(group).unwrap() {
            break;
        }
    }
    assert_eq!(bob.group_status(group).unwrap().state.revision(), 2);
    assert!(!bob.group_service_request_pending(group).unwrap());
    assert_eq!(alice.group_status(group).unwrap().state.members().len(), 1);
    assert_eq!(
        bob.db
            .query_row("SELECT count(*) FROM group_senders", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(bob
        .queue_group_text(group, [95; 32], "removed", now, now)
        .is_err());
}

#[test]
fn daily_group_credential_cache_is_bound_to_device_profile_and_rejects_corruption() {
    let (dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    alice.publish_device_binding_online().unwrap();
    bob.publish_device_binding_online().unwrap();
    let (raw, fingerprint) = enable(&dir.path().join("server.db"));
    let binding = alice.own_device_binding().unwrap();
    let server = peers::parse(&binding).unwrap().binding.server;
    let profile = Authority::from_bytes(&raw, &server, fingerprint).unwrap();
    let client = alice.connected_client().unwrap();
    alice
        .cached_group_credential(&client, &profile, &binding, now)
        .unwrap();
    let cached: Vec<u8> = alice
        .db
        .query_row("SELECT state FROM group_credentials", [], |r| r.get(0))
        .unwrap();
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    for _ in 0..8 {
        alice
            .cached_group_credential(&client, &profile, &binding, now)
            .unwrap();
    }
    assert_eq!(
        alice
            .db
            .query_row("SELECT state FROM group_credentials", [], |r| r
                .get::<_, Vec<u8>>(0))
            .unwrap(),
        cached
    );
    bob.db
        .execute(
            "INSERT INTO group_credentials VALUES(?1,?2)",
            (profile.id().as_slice(), &cached),
        )
        .unwrap();
    let bob_binding = bob.own_device_binding().unwrap();
    assert!(bob
        .cached_group_credential(
            &bob.connected_client().unwrap(),
            &profile,
            &bob_binding,
            now
        )
        .is_err());
    alice
        .db
        .execute("UPDATE group_credentials SET state=zeroblob(312)", [])
        .unwrap();
    assert!(alice
        .cached_group_credential(&client, &profile, &binding, now)
        .is_err());
    alice
        .db
        .execute("UPDATE group_credentials SET state=?1", [cached])
        .unwrap();
    assert!(alice
        .cached_group_credential(&client, &profile, &binding, now - 86400)
        .is_err());
}
