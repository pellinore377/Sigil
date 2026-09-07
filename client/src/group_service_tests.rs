use super::*;

fn open(path: &std::path::Path) -> ClientStore {
    ClientStore::open(
        path,
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap()
}
fn enable(path: &std::path::Path) -> (Vec<u8>, Id) {
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
