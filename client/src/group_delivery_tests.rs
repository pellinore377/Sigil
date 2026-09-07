use super::*;
use crate::groups::store::tests::{join, staged};
use crate::incoming::tests::{next, start, trust};
use sigil_crypto::Secret32;
use std::path::Path;

fn open(path: &Path) -> ClientStore {
    ClientStore::open(
        path,
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap()
}
fn setup(
    alice: &mut ClientStore,
    bob: &mut ClientStore,
    authority: &IdentityKey,
    now: u64,
) -> (Id, Id) {
    let group = staged(alice, bob, authority);
    join(alice, bob, group, Role::Member, authority);
    let (_, b) = trust(alice, bob);
    start(alice, b, now);
    assert!(bob.sync_step_online(now).failure.is_none());
    (group, b)
}
fn fingerprint(client: &ClientStore) -> Id {
    let tx = client.db.unchecked_transaction().unwrap();
    device_fingerprint(&peers::own(&tx, &client.key).unwrap()).unwrap()
}
fn checkpoint(client: &ClientStore, table: &str) -> Vec<u8> {
    client
        .db
        .query_row(&format!("SELECT state FROM {table}"), [], |r| r.get(0))
        .unwrap()
}
fn count(client: &ClientStore, table: &str) -> i64 {
    client
        .db
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}
fn recovery(client: &mut ClientStore) {
    let own = peers::parse(&client.own_device_binding().unwrap())
        .unwrap()
        .binding;
    client
        .configure_recovery(&own.server, own.account, Secret32::from_bytes([7; 32]))
        .unwrap();
}
fn distribution(alice: &mut ClientStore, bob: &mut ClientStore, group: Id, b: Id, now: u64) {
    alice.prepare_group_distribution(group, b, now).unwrap();
    alice.send_pending_online([3; 32], now).unwrap();
    let step = bob.sync_step_online(now);
    assert!(step.failure.is_none(), "{:?}", step.failure);
    assert!(step
        .incoming
        .iter()
        .any(|r| matches!(r.result, Ok(crate::MailboxEvent::GroupDistribution(_)))));
}
fn request(client: &ClientStore, group: Id, message: Id, recipient: Id, now: u64) -> Submit {
    let own = fingerprint(client);
    let id = index(&client.key, &group, &own, &message).unwrap();
    let stored = load(&client.db, &client.key, &own, &id).unwrap().unwrap();
    let id = transport_id(
        &stored.message.context,
        &message,
        &fingerprint_id(client, recipient),
    );
    let job = read(&client.db, &client.key, &own, &id).unwrap();
    Submit {
        recipient_device: transport::hex(&job.target.device),
        message_id: transport::hex(&id),
        payload: transport::hex(
            &packet(&client.db, &client.key, &own, &job.message, &stored).unwrap(),
        ),
        expires_at: now + 604800,
    }
}
fn fingerprint_id(client: &ClientStore, peer: Id) -> Id {
    peers::known(&client.db, &client.key, &peer)
        .unwrap()
        .fingerprint
}

#[test]
fn group_file_handoff_commits_sender_keys_fanout_and_media_history_together() {
    let (dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    let authority = IdentityKey::generate().unwrap();
    let (group, b) = setup(&mut alice, &mut bob, &authority, now);
    recovery(&mut alice);
    recovery(&mut bob);
    distribution(&mut alice, &mut bob, group, b, now);
    let author = fingerprint(&alice);
    let mut sender = alice
        .open_attachment_cache(&dir.path().join("sender-cache.db"), 8 * 1024 * 1024)
        .unwrap();
    let file = sender
        .prepare_upload(
            5,
            crate::attachments::Metadata {
                name: "synthetic.bin".into(),
                media_type: "application/octet-stream".into(),
            },
            None,
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
    let before = checkpoint(&alice, "group_senders");
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON group_delivery BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(matches!(
        alice.queue_group_file(group, [99; 32], (&mut sender, file), now, now),
        Err(Error::Storage(_))
    ));
    assert_eq!(checkpoint(&alice, "group_senders"), before);
    assert_eq!(count(&alice, "group_messages"), 0);
    alice.db.execute_batch("DROP TRIGGER fail").unwrap();
    alice
        .queue_group_file(group, [99; 32], (&mut sender, file), now, now)
        .unwrap();
    let before = checkpoint(&alice, "group_senders");
    alice
        .queue_group_file(group, [99; 32], (&mut sender, file), now, now)
        .unwrap();
    assert_eq!(checkpoint(&alice, "group_senders"), before);
    let sent = alice.resume_group_outbound_online(now).unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].result.as_ref().unwrap(),
        &GroupDeliveryStatus::Accepted
    );
    let delivery = next(&bob);
    let before = checkpoint(&bob, "group_receivers");
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON archive_records BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(matches!(
        bob.accept_group_delivery(&delivery),
        Err(Error::Storage(_))
    ));
    assert_eq!(checkpoint(&bob, "group_receivers"), before);
    assert_eq!(count(&bob, "group_messages"), 0);
    bob.db.execute_batch("DROP TRIGGER fail").unwrap();
    let step = bob.receive_mailbox_online(now).unwrap().remove(0);
    let crate::MailboxEvent::GroupFile(received) = step.result.unwrap() else {
        panic!("expected group file")
    };
    assert!(received.text().is_err());
    let event = received.event().unwrap();
    assert_eq!(event.content.file().unwrap().name, "synthetic.bin");
    let history = group_event_history_id(&event);
    assert!(matches!(
        alice.recovery_record(history).unwrap().content,
        sigil_crypto::recovery::Content::File(_)
    ));
    assert!(matches!(
        bob.recovery_record(history).unwrap().content,
        sigil_crypto::recovery::Content::File(_)
    ));
    let mut receiver = bob
        .open_attachment_cache(&dir.path().join("receiver-cache.db"), 8 * 1024 * 1024)
        .unwrap();
    assert!(matches!(
        bob.prepare_received_group_file(&mut receiver, group, [0; 32], [99; 32], now),
        Err(Error::NotFound)
    ));
    assert_eq!(
        bob.prepare_received_group_file(&mut receiver, group, author, [99; 32], now)
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
    receiver.evict(file).unwrap();
    receiver.cleanup(file).unwrap();
    let mut deleted = bob.recovery_record(history).unwrap();
    deleted.revision = 2;
    deleted.content = sigil_crypto::recovery::Content::Deleted;
    bob.retain_recovery_record(&deleted).unwrap();
    assert!(matches!(
        bob.prepare_received_group_file(&mut receiver, group, author, [99; 32], now),
        Err(Error::Obsolete)
    ));
    let mut deleted = alice.recovery_record(history).unwrap();
    deleted.revision = 2;
    deleted.content = sigil_crypto::recovery::Content::Deleted;
    alice.retain_recovery_record(&deleted).unwrap();
    assert!(matches!(
        alice.queue_group_file(group, [99; 32], (&mut sender, file), now, now),
        Err(Error::Obsolete)
    ));
}

#[test]
fn real_worker_commits_counter_fanout_history_and_acknowledgement_across_failures_and_restart() {
    let (dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    let authority = IdentityKey::generate().unwrap();
    let (group, b) = setup(&mut alice, &mut bob, &authority, now);
    recovery(&mut alice);
    recovery(&mut bob);
    alice.prepare_group_distribution(group, b, now).unwrap();
    let before = checkpoint(&alice, "group_senders");
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON group_delivery BEGIN SELECT RAISE(ABORT,'synthetic fanout failure'); END;").unwrap();
    assert!(matches!(
        alice.queue_group_text(group, [50; 32], "group hello", now, now),
        Err(Error::Storage(_))
    ));
    assert_eq!(checkpoint(&alice, "group_senders"), before);
    assert_eq!(count(&alice, "group_messages"), 0);
    assert!(alice.recovery_records(None).unwrap().is_empty());
    alice.db.execute_batch("DROP TRIGGER fail;").unwrap();
    alice
        .queue_group_text(group, [50; 32], "group hello", now, now)
        .unwrap();
    let after = checkpoint(&alice, "group_senders");
    let frozen = serde_json::to_vec(&request(&alice, group, [50; 32], b, now)).unwrap();
    alice
        .queue_group_text(group, [50; 32], "group hello", now, now + 1)
        .unwrap();
    assert_eq!(checkpoint(&alice, "group_senders"), after);
    assert!(matches!(
        alice.queue_group_text(group, [50; 32], "changed", now, now),
        Err(Error::Conflict)
    ));
    assert_eq!(count(&alice, "group_delivery"), 1);
    // Message submission waits for durable server acceptance of its distribution.
    let blocked = alice.resume_group_outbound_online(now).unwrap();
    assert!(matches!(blocked[0].result, Err(Error::Unprepared)));
    assert!(bob
        .connected_client()
        .unwrap()
        .mailbox()
        .unwrap()
        .is_empty());
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    assert_eq!(
        serde_json::to_vec(&request(&alice, group, [50; 32], b, now)).unwrap(),
        frozen
    );
    // Cursor wrap takes an empty pass. The worker sends distribution first.
    assert!(alice.sync_step_online(now).failure.is_none());
    let step = alice.sync_step_online(now);
    assert!(step.failure.is_none(), "{:?}", step.failure);
    assert_eq!(step.group_outbound.len(), 1);
    assert_eq!(
        step.group_outbound[0].result.as_ref().unwrap(),
        &GroupDeliveryStatus::Accepted
    );
    let delivery_id = step.group_outbound[0].delivery;
    let deliveries = bob.connected_client().unwrap().mailbox().unwrap();
    assert_eq!(deliveries.len(), 2);
    bob.accept_delivery(&deliveries[0]).unwrap();
    bob.acknowledge_incoming_online().unwrap();
    let before = checkpoint(&bob, "group_receivers");
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON group_incoming BEGIN SELECT RAISE(ABORT,'synthetic group receipt failure'); END;").unwrap();
    assert!(matches!(
        bob.accept_group_delivery(&deliveries[1]),
        Err(Error::Storage(_))
    ));
    assert_eq!(checkpoint(&bob, "group_receivers"), before);
    assert_eq!(count(&bob, "group_messages"), 0);
    assert!(bob.recovery_records(None).unwrap().is_empty());
    bob.db.execute_batch("DROP TRIGGER fail;").unwrap();
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON group_incoming BEGIN SELECT RAISE(ABORT,'synthetic ack failure'); END;").unwrap();
    let step = bob.sync_step_online(now);
    assert!(matches!(
        step.failure,
        Some(crate::SyncFailure::Acknowledge(Error::Storage(_)))
    ));
    assert!(
        matches!(&step.incoming[0].result,Ok(crate::MailboxEvent::GroupText(m)) if m.text().unwrap().body=="group hello")
    );
    assert!(bob
        .connected_client()
        .unwrap()
        .mailbox()
        .unwrap()
        .is_empty());
    bob.db.execute_batch("DROP TRIGGER fail;").unwrap();
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    assert_eq!(bob.acknowledge_incoming_online().unwrap(), 1);
    assert!(bob.accept_group_delivery(&deliveries[1]).unwrap().duplicate);
    let a = fingerprint(&alice);
    assert_eq!(
        bob.group_message(group, a, [50; 32])
            .unwrap()
            .text()
            .unwrap()
            .body,
        "group hello"
    );
    assert_eq!(
        alice.group_delivery_status(delivery_id).unwrap(),
        GroupDeliveryStatus::Accepted
    );
    assert_eq!(count(&alice, "group_messages"), 1);
    assert_eq!(
        alice
            .db
            .query_row("SELECT count(packet) FROM group_messages", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let left = alice.recovery_records(None).unwrap();
    let right = bob.recovery_records(None).unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(right.len(), 1);
    assert_eq!(left[0].id, right[0].id);
}

#[test]
fn authenticated_reordering_duplicates_and_tampering_do_not_rewrite_history_or_receiver() {
    let (_dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    let authority = IdentityKey::generate().unwrap();
    let (group, b) = setup(&mut alice, &mut bob, &authority, now);
    distribution(&mut alice, &mut bob, group, b, now);
    alice
        .queue_group_text(group, [50; 32], "earlier", now, now)
        .unwrap();
    alice
        .queue_group_text(group, [51; 32], "later", now, now)
        .unwrap();
    let network = alice.connected_client().unwrap();
    network
        .submit(&request(&alice, group, [51; 32], b, now))
        .unwrap();
    network
        .submit(&request(&alice, group, [50; 32], b, now))
        .unwrap();
    let deliveries = bob.connected_client().unwrap().mailbox().unwrap();
    let before = checkpoint(&bob, "group_receivers");
    let mut bad: Delivery =
        serde_json::from_slice(&serde_json::to_vec(&deliveries[0]).unwrap()).unwrap();
    let last = bad.payload.len() - 2;
    let replacement = if &bad.payload[last..] == "00" {
        "01"
    } else {
        "00"
    };
    bad.payload.replace_range(last.., replacement);
    assert!(bob.accept_group_delivery(&bad).is_err());
    assert_eq!(checkpoint(&bob, "group_receivers"), before);
    assert_eq!(
        bob.accept_group_delivery(&deliveries[0])
            .unwrap()
            .text()
            .unwrap()
            .body,
        "later"
    );
    assert_eq!(
        bob.accept_group_delivery(&deliveries[1])
            .unwrap()
            .text()
            .unwrap()
            .body,
        "earlier"
    );
    let advanced = checkpoint(&bob, "group_receivers");
    assert!(bob.accept_group_delivery(&deliveries[0]).unwrap().duplicate);
    bad.sequence += 100;
    assert!(matches!(
        bob.accept_group_delivery(&bad),
        Err(Error::Conflict)
    ));
    assert_eq!(checkpoint(&bob, "group_receivers"), advanced);
    // A new server sequence cannot make an identical packet consume a key twice.
    bad.payload = deliveries[0].payload.clone();
    assert!(bob.accept_group_delivery(&bad).unwrap().duplicate);
    assert_eq!(checkpoint(&bob, "group_receivers"), advanced);
    assert_eq!(count(&bob, "group_messages"), 2);
    assert_eq!(count(&bob, "group_incoming"), 3);
}

#[test]
fn membership_cutover_rolls_back_failures_cancels_unsent_work_and_keeps_committed_history() {
    let (dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    let authority = IdentityKey::generate().unwrap();
    let (group, b) = setup(&mut alice, &mut bob, &authority, now);
    distribution(&mut alice, &mut bob, group, b, now);
    alice
        .queue_group_text(group, [50; 32], "before cutover", now, now)
        .unwrap();
    alice.resume_group_outbound_online(now).unwrap();
    let accepted = next(&bob);
    bob.accept_group_delivery(&accepted).unwrap();
    bob.acknowledge_incoming_online().unwrap();
    alice
        .queue_group_text(group, [51; 32], "not submitted", now, now)
        .unwrap();
    let unsent = request(&alice, group, [51; 32], b, now);
    let before = alice.group_status(group).unwrap().state;
    let proposal = alice
        .prepare_group_change(group, Change::EarlierHistory(true))
        .unwrap();
    let ordered = crate::groups::Receipt::sign(
        group,
        before.head(),
        before.proposal_from_bytes(&proposal).unwrap().head(),
        before.revision() + 1,
        &authority,
    )
    .unwrap()
    .to_bytes();
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON group_delivery BEGIN SELECT RAISE(ABORT,'synthetic cutover failure'); END;").unwrap();
    assert!(matches!(
        alice.commit_group_proposal(group, &proposal, &ordered),
        Err(Error::Storage(_))
    ));
    assert_eq!(
        alice.group_status(group).unwrap().state.head(),
        before.head()
    );
    assert_eq!(
        serde_json::to_vec(&request(&alice, group, [51; 32], b, now)).unwrap(),
        serde_json::to_vec(&unsent).unwrap()
    );
    alice.db.execute_batch("DROP TRIGGER fail;").unwrap();
    alice
        .commit_group_proposal(group, &proposal, &ordered)
        .unwrap();
    bob.commit_group_proposal(group, &proposal, &ordered)
        .unwrap();
    let id = decode_id(&unsent.message_id).unwrap();
    assert_eq!(
        alice.group_delivery_status(id).unwrap(),
        GroupDeliveryStatus::Cancelled
    );
    assert_eq!(count(&alice, "group_senders"), 0);
    assert_eq!(count(&bob, "group_receivers"), 0);
    assert!(alice.resume_group_outbound_online(now).unwrap().is_empty());
    // Simulate a request already in flight at cutover; submission cannot be recalled.
    alice.connected_client().unwrap().submit(&unsent).unwrap();
    assert!(matches!(
        bob.accept_group_delivery(&next(&bob)),
        Err(Error::Unprepared)
    ));
    assert!(bob.accept_group_delivery(&accepted).unwrap().duplicate);
    let a = fingerprint(&alice);
    assert_eq!(
        bob.group_message(group, a, [50; 32])
            .unwrap()
            .text()
            .unwrap()
            .body,
        "before cutover"
    );
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    assert_eq!(
        alice.group_delivery_status(id).unwrap(),
        GroupDeliveryStatus::Cancelled
    );
    // Retrying the old logical request never silently re-encrypts under the new epoch.
    alice
        .queue_group_text(group, [51; 32], "not submitted", now, now)
        .unwrap();
    assert_eq!(count(&alice, "group_senders"), 0);
}

#[test]
fn all_recipient_keys_precede_sends_and_partial_fanout_never_loses_shared_ciphertext() {
    use crate::connection::tests::{credential, prepare};
    use sigil_protocol::accounts::InviteRequest;
    let (dir, fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    let authority = IdentityKey::generate().unwrap();
    let group = staged(&mut alice, &mut bob, &authority);
    let (join_b, receipt_b) = join(&mut alice, &mut bob, group, Role::Member, &authority);
    let (_, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    assert!(bob.sync_step_online(now).failure.is_none());
    let mut server = sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap();
    let invitation = server
        .invite(
            InviteRequest {
                username: "charlie".into(),
                expires_in_seconds: 60,
            },
            now,
        )
        .unwrap();
    let mut charlie = open(&dir.path().join("charlie.db"));
    prepare(&mut charlie, &fixture, &invitation.secret);
    charlie.enroll_online().unwrap();
    let a_device = alice.connection_session().unwrap().unwrap().device_id;
    let c_device = charlie.connection_session().unwrap().unwrap().device_id;
    server
        .allow_sender(&credential(&charlie), &a_device, now)
        .unwrap();
    server
        .allow_sender(&credential(&alice), &c_device, now)
        .unwrap();
    let (a, c) = trust(&mut alice, &mut charlie);
    charlie
        .accept_group_genesis(a, &alice.group_genesis(group).unwrap())
        .unwrap();
    charlie
        .commit_group_proposal(group, &join_b, &receipt_b)
        .unwrap();
    let member = Member::new(
        [3; 32],
        Role::Member,
        &[charlie.own_device_binding().unwrap()],
    )
    .unwrap();
    let proposal = alice
        .prepare_group_change(group, Change::Add(member))
        .unwrap();
    let proposal = charlie.approve_group_proposal(group, &proposal).unwrap();
    let proposal = alice.approve_group_proposal(group, &proposal).unwrap();
    let before = alice.group_status(group).unwrap().state;
    let ordered = crate::groups::Receipt::sign(
        group,
        before.head(),
        before.proposal_from_bytes(&proposal).unwrap().head(),
        before.revision() + 1,
        &authority,
    )
    .unwrap()
    .to_bytes();
    for client in [&mut alice, &mut bob, &mut charlie] {
        client
            .commit_group_proposal(group, &proposal, &ordered)
            .unwrap();
    }
    charlie
        .prepare_prekey_publication([2; 32], true, 3600)
        .unwrap();
    charlie.publish_prekey_online([2; 32]).unwrap();
    alice.prepare_peer_claim([6; 32], c).unwrap();
    alice.claim_prekey_online([6; 32], now).unwrap();
    alice
        .start_claimed_text([6; 32], [6; 32], [7; 32], "charlie initial", now, now)
        .unwrap();
    alice.send_pending_online([6; 32], now).unwrap();
    assert!(charlie.sync_step_online(now).failure.is_none());
    alice.prepare_group_distribution(group, b, now).unwrap();
    assert!(!alice.group_sender_ready(group).unwrap());
    assert!(matches!(
        alice.queue_group_text(group, [50; 32], "three members", now, now),
        Err(Error::Unprepared)
    ));
    let first = alice
        .pending_deliveries([3; 32], now)
        .unwrap()
        .remove(0)
        .payload;
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON group_key_outbox BEGIN SELECT RAISE(ABORT,'synthetic second recipient failure'); END;").unwrap();
    assert!(matches!(
        alice.prepare_group_distribution(group, c, now),
        Err(Error::Storage(_))
    ));
    assert!(!alice.group_sender_ready(group).unwrap());
    assert_eq!(
        alice.pending_deliveries([3; 32], now).unwrap()[0].payload,
        first
    );
    alice.db.execute_batch("DROP TRIGGER fail;").unwrap();
    alice.prepare_group_distribution(group, c, now).unwrap();
    assert!(alice.group_sender_ready(group).unwrap());
    alice.send_pending_online([3; 32], now).unwrap();
    alice.send_pending_online([6; 32], now).unwrap();
    assert!(bob.sync_step_online(now).failure.is_none());
    assert!(charlie.sync_step_online(now).failure.is_none());
    alice
        .queue_group_text(group, [50; 32], "three members", now, now)
        .unwrap();
    let to_b = request(&alice, group, [50; 32], b, now);
    let to_c = request(&alice, group, [50; 32], c, now);
    assert_eq!(to_b.payload, to_c.payload);
    assert_ne!(to_b.message_id, to_c.message_id);
    assert_eq!(count(&alice, "group_messages"), 1);
    assert_eq!(count(&alice, "group_delivery"), 2);
    let c_id = decode_id(&to_c.message_id).unwrap();
    let b_id = decode_id(&to_b.message_id).unwrap();
    // An unauthenticated SQL status hint cannot authorize retry-ciphertext deletion.
    alice
        .db
        .execute(
            "UPDATE group_delivery SET status=3 WHERE id=?1",
            [c_id.as_slice()],
        )
        .unwrap();
    assert!(matches!(
        alice.send_group_delivery(b_id, now),
        Err(Error::InvalidStore)
    ));
    assert_eq!(
        alice.group_delivery_status(b_id).unwrap(),
        GroupDeliveryStatus::Pending
    );
    assert_eq!(
        alice
            .db
            .query_row("SELECT count(packet) FROM group_messages", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    alice
        .db
        .execute(
            "UPDATE group_delivery SET status=0 WHERE id=?1",
            [c_id.as_slice()],
        )
        .unwrap();
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    assert_eq!(
        alice.send_group_delivery(b_id, now).unwrap(),
        GroupDeliveryStatus::Accepted
    );
    assert_eq!(bob.connected_client().unwrap().mailbox().unwrap().len(), 1);
    assert_eq!(
        alice
            .db
            .query_row("SELECT count(packet) FROM group_messages", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        alice.send_group_delivery(c_id, now).unwrap(),
        GroupDeliveryStatus::Accepted
    );
    assert_eq!(
        alice
            .db
            .query_row("SELECT count(packet) FROM group_messages", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    for client in [&mut bob, &mut charlie] {
        let step = client.sync_step_online(now);
        assert!(step.failure.is_none());
        assert!(
            matches!(&step.incoming[0].result,Ok(crate::MailboxEvent::GroupText(m)) if m.text().unwrap().body=="three members")
        );
    }
}

#[test]
fn maximum_text_migrates_and_pending_capacity_is_reusable_without_counter_reset() {
    let (dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    let authority = IdentityKey::generate().unwrap();
    let (group, b) = setup(&mut alice, &mut bob, &authority, now);
    distribution(&mut alice, &mut bob, group, b, now);
    crate::test_schema::rewind(&alice.db, 44);
    alice.db.pragma_update(None, "user_version", 44).unwrap();
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    assert_eq!(
        alice
            .db
            .query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        55
    );
    let max = "x".repeat(GroupText::MAX_BODY);
    alice
        .queue_group_text(group, [50; 32], &max, now, now)
        .unwrap();
    let result = alice.resume_group_outbound_online(now).unwrap();
    assert_eq!(
        result[0].result.as_ref().unwrap(),
        &GroupDeliveryStatus::Accepted
    );
    assert_eq!(
        bob.accept_group_delivery(&next(&bob))
            .unwrap()
            .text()
            .unwrap()
            .body,
        max
    );
    bob.acknowledge_incoming_online().unwrap();
    for n in 0..16u8 {
        alice
            .queue_group_text(group, [n; 32], "pending", now, now)
            .unwrap();
    }
    let before = checkpoint(&alice, "group_senders");
    assert!(matches!(
        alice.queue_group_text(group, [17; 32], "full", now, now),
        Err(Error::Limit)
    ));
    assert_eq!(checkpoint(&alice, "group_senders"), before);
    let id = decode_id(&request(&alice, group, [0; 32], b, now).message_id).unwrap();
    assert_eq!(
        alice.send_group_delivery(id, now + 604800).unwrap(),
        GroupDeliveryStatus::Expired
    );
    alice
        .queue_group_text(group, [17; 32], "new capacity", now + 604800, now + 604800)
        .unwrap();
    let own = fingerprint(&alice);
    let state = alice.group_status(group).unwrap().state;
    let tx = alice.db.transaction().unwrap();
    assert_eq!(
        keys::load_sender(&tx, &alice.key, &own, &state)
            .unwrap()
            .unwrap()
            .0
            .counter(),
        18
    );
}

#[test]
fn sole_member_group_retains_local_history_without_network_jobs_or_seed_export() {
    let (_dir, _fixture, mut alice, _bob, now) = crate::claims::tests::pair();
    let authority = IdentityKey::generate().unwrap();
    let group = alice
        .create_group(authority_fingerprint(&authority.public_key()))
        .unwrap();
    recovery(&mut alice);
    for n in 0..20u8 {
        alice
            .queue_group_text(group, [n; 32], "local group", now, now)
            .unwrap();
    }
    assert!(alice.group_sender_ready(group).unwrap());
    assert_eq!(count(&alice, "group_delivery"), 0);
    assert_eq!(count(&alice, "group_key_outbox"), 0);
    assert_eq!(
        alice
            .db
            .query_row("SELECT count(packet) FROM group_messages", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let first = alice.recovery_records(None).unwrap();
    assert_eq!(first.len(), 16);
    assert_eq!(
        alice
            .recovery_records(Some(first.last().unwrap().id))
            .unwrap()
            .len(),
        4
    );
    assert!(alice.resume_group_outbound_online(now).unwrap().is_empty());
}

#[test]
fn authorized_refresh_uses_fresh_chain_and_equivocation_freezes_queued_group_messages() {
    let (_dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    let authority = IdentityKey::generate().unwrap();
    let (group, b) = setup(&mut alice, &mut bob, &authority, now);
    distribution(&mut alice, &mut bob, group, b, now);
    assert!(matches!(
        bob.prepare_group_change(group, Change::RefreshKeys),
        Err(Error::Unprepared)
    ));
    alice
        .queue_group_text(group, [50; 32], "old epoch", now, now)
        .unwrap();
    let own = fingerprint(&alice);
    let old_context = alice.group_message(group, own, [50; 32]).unwrap().context;
    let before = alice.group_status(group).unwrap().state;
    let proposal = alice
        .prepare_group_change(group, Change::RefreshKeys)
        .unwrap();
    let head = before.proposal_from_bytes(&proposal).unwrap().head();
    let receipt = crate::groups::Receipt::sign(
        group,
        before.head(),
        head,
        before.revision() + 1,
        &authority,
    )
    .unwrap()
    .to_bytes();
    for client in [&mut alice, &mut bob] {
        client
            .commit_group_proposal(group, &proposal, &receipt)
            .unwrap();
    }
    distribution(&mut alice, &mut bob, group, b, now);
    alice
        .queue_group_text(group, [51; 32], "fresh epoch", now, now)
        .unwrap();
    let new_context = alice.group_message(group, own, [51; 32]).unwrap().context;
    assert_eq!(new_context.epoch, old_context.epoch + 1);
    assert_ne!(new_context.chain, old_context.chain);
    assert_ne!(new_context.state, old_context.state);
    let attempts = alice.resume_group_outbound_online(now).unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(
        attempts[0].result.as_ref().unwrap(),
        &GroupDeliveryStatus::Accepted
    );
    assert_eq!(
        bob.accept_group_delivery(&next(&bob))
            .unwrap()
            .text()
            .unwrap()
            .body,
        "fresh epoch"
    );
    bob.acknowledge_incoming_online().unwrap();
    alice
        .queue_group_text(group, [52; 32], "freeze pending", now, now)
        .unwrap();
    let pending = request(&alice, group, [52; 32], b, now);
    let conflicting = crate::groups::Receipt::sign(
        group,
        before.head(),
        [89; 32],
        before.revision() + 1,
        &authority,
    )
    .unwrap()
    .to_bytes();
    assert_eq!(
        alice
            .commit_group_proposal(group, &proposal, &conflicting)
            .unwrap(),
        crate::groups::CommitResult::Frozen
    );
    assert_eq!(
        alice
            .group_delivery_status(decode_id(&pending.message_id).unwrap())
            .unwrap(),
        GroupDeliveryStatus::Cancelled
    );
    assert_eq!(count(&alice, "group_senders"), 0);
    assert!(matches!(
        alice.queue_group_text(group, [53; 32], "refused", now, now),
        Err(Error::Conflict)
    ));
}

#[test]
fn group_sigiltext_preserves_redaction_and_commits_ratchet_history_and_fanout_together() {
    let (_dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    let authority = IdentityKey::generate().unwrap();
    let (group, b) = setup(&mut alice, &mut bob, &authority, now);
    recovery(&mut alice);
    recovery(&mut bob);
    distribution(&mut alice, &mut bob, group, b, now);
    let text =
        sigil_protocol::text::parse("redact::SYNTHETIC_SECRET; red::👩🏽‍💻;", Default::default())
            .unwrap();
    let before = checkpoint(&alice, "group_senders");
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON group_delivery BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(matches!(
        alice.queue_group_sigiltext(group, [99; 32], &text, now, now),
        Err(Error::Storage(_))
    ));
    assert_eq!(checkpoint(&alice, "group_senders"), before);
    assert_eq!(count(&alice, "group_messages"), 0);
    alice.db.execute_batch("DROP TRIGGER fail").unwrap();
    alice
        .queue_group_sigiltext(group, [99; 32], &text, now, now)
        .unwrap();
    let before = checkpoint(&alice, "group_senders");
    alice
        .queue_group_sigiltext(group, [99; 32], &text, now, now)
        .unwrap();
    assert_eq!(checkpoint(&alice, "group_senders"), before);
    assert_eq!(
        *alice.resume_group_outbound_online(now).unwrap()[0]
            .result
            .as_ref()
            .unwrap(),
        GroupDeliveryStatus::Accepted
    );
    let delivery = next(&bob);
    let before = checkpoint(&bob, "group_receivers");
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON archive_records BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(matches!(
        bob.accept_group_delivery(&delivery),
        Err(Error::Storage(_))
    ));
    assert_eq!(checkpoint(&bob, "group_receivers"), before);
    bob.db.execute_batch("DROP TRIGGER fail").unwrap();
    let crate::MailboxEvent::GroupSigilText(message) = bob
        .receive_mailbox_online(now)
        .unwrap()
        .remove(0)
        .result
        .unwrap()
    else {
        panic!("lost rich group kind")
    };
    assert!(message.text().is_err());
    let event = message.event().unwrap();
    assert!(event.content.rich().unwrap() == text);
    assert!(!std::str::from_utf8(event.content.bytes())
        .unwrap()
        .contains("SYNTHETIC_SECRET"));
    let id = group_event_history_id(&event);
    for client in [&alice, &bob] {
        let sigil_crypto::recovery::Content::Rich(bytes) =
            client.recovery_record(id).unwrap().content
        else {
            panic!("lost rich archive kind")
        };
        assert!(sigil_protocol::text::Text::from_bytes(&bytes).unwrap() == text);
    }
    assert!(bob.accept_group_delivery(&delivery).unwrap().duplicate);
    let mut deleted = alice.recovery_record(id).unwrap();
    deleted.revision = 2;
    deleted.content = sigil_crypto::recovery::Content::Deleted;
    alice.retain_recovery_record(&deleted).unwrap();
    assert!(matches!(
        alice.queue_group_sigiltext(group, [99; 32], &text, now, now),
        Err(Error::Obsolete)
    ));
}

#[test]
fn group_cards_reject_authenticated_creator_forgery_then_accept_valid_skipped_counter() {
    let (_dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    let authority = IdentityKey::generate().unwrap();
    let (group, b) = setup(&mut alice, &mut bob, &authority, now);
    recovery(&mut alice);
    recovery(&mut bob);
    distribution(&mut alice, &mut bob, group, b, now);
    let a = fingerprint(&alice);
    let origin = sigil_protocol::text::Origin {
        message: [101; 32],
        creator: alice.account_reference().unwrap(),
        created_at: now,
        timezone: Some("America/Chicago"),
    };
    let sigil_protocol::text::Parsed::Card(card) = sigil_protocol::text::parse_card(
        "checklist::recurr::weekly::Tasks\n-r- One\n- Two;",
        origin,
        Default::default(),
    )
    .unwrap()
    .content
    else {
        panic!()
    };
    let mut forged = card.clone();
    forged.creator = bob.account_reference().unwrap();
    assert!(matches!(
        alice.queue_group_card(group, &forged, now),
        Err(Error::InvalidEvent)
    ));
    let body = forged.to_bytes().unwrap();
    let plaintext = Group {
        message: forged.id,
        group,
        sender: a,
        timestamp: now,
        content: Content::Rich(&body),
    }
    .to_bytes()
    .unwrap();
    // Model a malicious but authenticated group member using its real Sender Key.
    let tx = alice.db.transaction().unwrap();
    let state = keys::current(&tx, &alice.key, &a, &group).unwrap();
    let (mut sender, _, _) = keys::load_sender(&tx, &alice.key, &a, &state)
        .unwrap()
        .unwrap();
    let packet = sender.seal(forged.id, &plaintext).unwrap();
    keys::save_sender(&tx, &alice.key, &a, &sender, true, None).unwrap();
    tx.commit().unwrap();
    let delivery = sigil_protocol::mailbox::Delivery {
        sequence: 400,
        sender_device: alice.connection_session().unwrap().unwrap().device_id,
        message_id: transport::hex(&transport_id(
            &packet.context(),
            &forged.id,
            &fingerprint(&bob),
        )),
        payload: transport::hex(&packet.to_bytes()),
        expires_at: now + 60,
    };
    let before = checkpoint(&bob, "group_receivers");
    assert!(matches!(
        bob.accept_group_delivery(&delivery),
        Err(Error::InvalidEvent)
    ));
    assert_eq!(checkpoint(&bob, "group_receivers"), before);
    assert_eq!(count(&bob, "group_messages"), 0);
    let mut card = card;
    card.id = [102; 32];
    alice.queue_group_card(group, &card, now).unwrap();
    assert_eq!(
        *alice.resume_group_outbound_online(now).unwrap()[0]
            .result
            .as_ref()
            .unwrap(),
        GroupDeliveryStatus::Accepted
    );
    let incoming = bob.accept_group_delivery(&next(&bob)).unwrap();
    let event = incoming.event().unwrap();
    assert!(event.content.card().unwrap() == card);
    let id = group_event_history_id(&event);
    for client in [&alice, &bob] {
        let sigil_crypto::recovery::Content::Rich(bytes) =
            client.recovery_record(id).unwrap().content
        else {
            panic!()
        };
        assert!(sigil_protocol::text::structured::Card::from_bytes(&bytes).unwrap() == card);
    }
}

#[test]
fn group_checklist_actions_are_atomic_and_migrate_with_unarchived_cards() {
    for mode in 0..3 {
        group_list_actions(mode);
    }
}
fn group_list_actions(mode: u8) {
    use sigil_protocol::text::{
        action::{Action, Change, Reference},
        structured::Construct,
        Origin, Parsed,
    };
    let (dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    let authority = IdentityKey::generate().unwrap();
    let (group, b) = setup(&mut alice, &mut bob, &authority, now);
    distribution(&mut alice, &mut bob, group, b, now);
    let Parsed::Card(card) = sigil_protocol::text::parse_card(
        if mode == 2 {
            "checklist::recurr::weekly::Things\n-r- One\n- Two;"
        } else if mode == 1 {
            "checklist::task::Things\n- One\n- Two;"
        } else {
            "checklist::Things\n- One\n- Two;"
        },
        Origin {
            message: [103; 32],
            creator: alice.account_reference().unwrap(),
            created_at: now,
            timezone: if mode == 2 { Some("UTC") } else { None },
        },
        Default::default(),
    )
    .unwrap()
    .content
    else {
        panic!()
    };
    let reference = Reference::of(&card).unwrap();
    alice.queue_group_card(group, &card, now).unwrap();
    alice
        .resume_group_outbound_online(now)
        .unwrap()
        .remove(0)
        .result
        .unwrap();
    bob.accept_group_delivery(&next(&bob)).unwrap();
    bob.acknowledge_incoming_online().unwrap();
    // Reconstruct the actual pre-action schema, including unarchived group content.
    crate::test_schema::rewind(&bob.db, 51);
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    assert!(bob.card_state(group, reference).unwrap().card == card);
    let Construct::Checklist(list) = &card.content else {
        panic!()
    };
    let action = Action {
        card: reference,
        actor: alice.account_reference().unwrap(),
        created_at: now,
        previous: None,
        change: if mode == 2 {
            Change::RecurringCheck {
                item: list.items[0].id,
                period: now,
            }
        } else if mode == 1 {
            Change::Complete {
                item: list.items[0].id,
            }
        } else {
            Change::Check {
                item: list.items[0].id,
                checked: true,
            }
        },
    };
    alice.queue_group_action(group, &action, now).unwrap();
    let before = checkpoint(&alice, "group_senders");
    alice.queue_group_action(group, &action, now).unwrap();
    assert_eq!(checkpoint(&alice, "group_senders"), before);
    alice
        .resume_group_outbound_online(now)
        .unwrap()
        .remove(0)
        .result
        .unwrap();
    let delivery = next(&bob);
    let before = checkpoint(&bob, "group_receivers");
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON structured_heads BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(matches!(
        bob.accept_group_delivery(&delivery),
        Err(Error::Storage(_))
    ));
    assert_eq!(checkpoint(&bob, "group_receivers"), before);
    let checked = |client: &ClientStore| {
        let state = client.card_state(group, reference).unwrap();
        if mode == 2 {
            client
                .recurring_state(group, reference, now)
                .unwrap()
                .checks[0]
                .checked
        } else if mode == 1 {
            state.tasks[0].completed
        } else {
            state.checks[0].checked
        }
    };
    assert!(!checked(&bob));
    bob.db.execute_batch("DROP TRIGGER fail").unwrap();
    let accepted = bob.accept_group_delivery(&delivery).unwrap();
    assert!(accepted.event().unwrap().content.action().unwrap() == action);
    assert!(checked(&bob));
    assert!(bob.accept_group_delivery(&delivery).unwrap().duplicate);
    let forged = Action {
        actor: bob.account_reference().unwrap(),
        previous: if mode == 2 {
            None
        } else {
            Some(action.id().unwrap())
        },
        change: if mode == 2 {
            Change::RecurringCheck {
                item: list.items[0].id,
                period: now,
            }
        } else if mode == 1 {
            Change::Undo {
                item: list.items[0].id,
                completion: action.id().unwrap(),
            }
        } else {
            Change::Check {
                item: list.items[0].id,
                checked: false,
            }
        },
        ..action
    };
    let body = forged.to_bytes().unwrap();
    let message = forged.id().unwrap();
    let own = fingerprint(&alice);
    let bytes = Group {
        message,
        group,
        sender: own,
        timestamp: now,
        content: Content::Rich(&body),
    }
    .to_bytes()
    .unwrap();
    let tx = alice.db.unchecked_transaction().unwrap();
    let state = keys::current(&tx, &alice.key, &own, &group).unwrap();
    let (mut sender, _, _) = keys::load_sender(&tx, &alice.key, &own, &state)
        .unwrap()
        .unwrap();
    let packet = sender.seal(message, &bytes).unwrap();
    tx.commit().unwrap();
    let forged = sigil_protocol::mailbox::Delivery {
        sequence: 999,
        sender_device: alice.connection_session().unwrap().unwrap().device_id,
        message_id: transport::hex(&transport_id(
            &packet.context(),
            &message,
            &fingerprint(&bob),
        )),
        payload: transport::hex(&packet.to_bytes()),
        expires_at: now + 60,
    };
    let before = checkpoint(&bob, "group_receivers");
    assert!(matches!(
        bob.accept_group_delivery(&forged),
        Err(Error::InvalidEvent)
    ));
    assert_eq!(checkpoint(&bob, "group_receivers"), before);
}
