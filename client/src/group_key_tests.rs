use super::*;
use crate::groups::store::tests::{join, staged};
use sigil_crypto::Secret32;
use std::path::Path;

fn open(path: &Path) -> ClientStore {
    ClientStore::open(
        path,
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap()
}
fn channel(alice: &mut ClientStore, bob: &mut ClientStore, now: u64) -> (Id, Id, Id) {
    let (a, b) = crate::incoming::tests::trust(alice, bob);
    crate::incoming::tests::start(alice, b, now);
    let incoming = bob
        .accept_delivery(&crate::incoming::tests::next(bob))
        .unwrap();
    bob.acknowledge_incoming_online().unwrap();
    (a, b, incoming.session)
}
fn recovery(client: &mut ClientStore) {
    let own = peers::parse(&client.own_device_binding().unwrap())
        .unwrap()
        .binding;
    client
        .configure_recovery(&own.server, own.account, Secret32::from_bytes([7; 32]))
        .unwrap();
}
fn count(client: &ClientStore, table: &str) -> i64 {
    client
        .db
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}
fn session_state(client: &ClientStore, session: Id) -> Vec<u8> {
    client
        .db
        .query_row(
            "SELECT state FROM sessions WHERE id=?1",
            [session.as_slice()],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn normal_worker_installs_keys_atomically_without_retaining_secret_history_or_resetting_receiver() {
    let (dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    let authority = IdentityKey::generate().unwrap();
    let group = staged(&mut alice, &mut bob, &authority);
    join(&mut alice, &mut bob, group, Role::Member, &authority);
    let (_, b, bob_session) = channel(&mut alice, &mut bob, now);
    recovery(&mut alice);
    recovery(&mut bob);
    let a_fingerprint = device_fingerprint(&alice.own_device_binding().unwrap()).unwrap();
    let b_fingerprint = device_fingerprint(&bob.own_device_binding().unwrap()).unwrap();
    let id = alice.prepare_group_distribution(group, b, now).unwrap();
    assert_eq!(alice.prepare_group_distribution(group, b, now).unwrap(), id);
    assert!(alice.group_sender_ready(group).unwrap());
    assert_eq!(count(&alice, "group_key_outbox"), 1);
    assert!(alice
        .db
        .query_row(
            "SELECT seed IS NULL FROM group_senders WHERE group_id=?1",
            [group.as_slice()],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
    let retained = alice.outgoing_message([3; 32], id).unwrap();
    assert_eq!(
        distribution_receipt(&retained).unwrap().unwrap().message,
        id
    );
    assert!(!is_wire_control(&retained));
    alice.send_pending_online([3; 32], now).unwrap();
    let delivery = crate::incoming::tests::next(&bob);
    let before = session_state(&bob, bob_session);
    let packet: Vec<u8> = delivery
        .payload
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect();
    assert!(matches!(
        bob.receive(bob_session, id, &packet),
        Err(Error::Unprepared)
    ));
    assert_eq!(session_state(&bob, bob_session), before);
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON group_controls BEGIN SELECT RAISE(ABORT,'synthetic distribution journal failure'); END;").unwrap();
    assert!(bob.accept_delivery(&delivery).is_err());
    assert_eq!(session_state(&bob, bob_session), before);
    assert_eq!(count(&bob, "group_receivers"), 0);
    assert_eq!(count(&bob, "group_controls"), 0);
    assert_eq!(bob.acknowledge_incoming_online().unwrap(), 0);
    bob.db.execute_batch("DROP TRIGGER fail;").unwrap();
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    let step = bob.sync_step_online(now);
    assert!(step.failure.is_none(), "{:?}", step.failure);
    assert!(step.incoming.iter().any(|v| matches!(&v.result, Ok(crate::MailboxEvent::GroupDistribution(r)) if r.message == id && r.context.group == group)));
    assert!(bob.group_receiver_ready(group, a_fingerprint).unwrap());
    let retained = bob.message(bob_session, id).unwrap();
    assert_eq!(retained.len(), 240);
    assert!(distribution_receipt(&retained).unwrap().is_some());
    assert!(alice.recovery_records(None).unwrap().is_empty());
    assert!(bob.recovery_records(None).unwrap().is_empty());
    // Advance the installed in-memory engines and persist their checkpoints.
    // This exercises key setup, not an implemented group-message transport.
    let state = alice.group_status(group).unwrap().state;
    let packet = {
        let tx = alice.db.transaction().unwrap();
        let (mut sender, ready, _) = load_sender(&tx, &alice.key, &a_fingerprint, &state)
            .unwrap()
            .unwrap();
        assert!(ready);
        let packet = sender
            .seal([92; 32], b"synthetic key agreement check")
            .unwrap();
        save_sender(&tx, &alice.key, &a_fingerprint, &sender, true, None).unwrap();
        tx.commit().unwrap();
        packet
    };
    let state = bob.group_status(group).unwrap().state;
    {
        let tx = bob.db.transaction().unwrap();
        let (mut receiver, tag) = receiver(&tx, &bob.key, &b_fingerprint, &state, &a_fingerprint)
            .unwrap()
            .unwrap();
        assert_eq!(
            receiver.open(&packet).unwrap(),
            b"synthetic key agreement check"
        );
        save_receiver(&tx, &bob.key, &b_fingerprint, &receiver, &tag).unwrap();
        tx.commit().unwrap();
    }
    let duplicate = bob.accept_delivery(&delivery).unwrap();
    assert!(duplicate.distribution().unwrap().is_some());
    let tx = bob.db.transaction().unwrap();
    assert_eq!(
        receiver(&tx, &bob.key, &b_fingerprint, &state, &a_fingerprint)
            .unwrap()
            .unwrap()
            .0
            .counter(),
        1
    );
    drop(tx);
    assert_eq!(count(&bob, "group_controls"), 1);
    drop(alice);
    drop(bob);
    let mut alice = open(&dir.path().join("alice.db"));
    let mut bob = open(&dir.path().join("bob.db"));
    assert!(alice.group_sender_ready(group).unwrap());
    assert!(bob.group_receiver_ready(group, a_fingerprint).unwrap());
    assert_eq!(alice.prepare_group_distribution(group, b, now).unwrap(), id);
}

#[test]
fn cutover_cancels_unsent_distributions_and_erases_live_group_keys_without_breaking_direct_sessions(
) {
    let (_dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    let authority = IdentityKey::generate().unwrap();
    let group = staged(&mut alice, &mut bob, &authority);
    join(&mut alice, &mut bob, group, Role::Member, &authority);
    let (a, b, bob_session) = channel(&mut alice, &mut bob, now);
    let bob_fp = device_fingerprint(&bob.own_device_binding().unwrap()).unwrap();
    bob.prepare_group_distribution(group, a, now).unwrap();
    bob.send_pending_online(bob_session, now).unwrap();
    let step = alice.sync_step_online(now);
    assert!(step.failure.is_none(), "{:?}", step.failure);
    assert!(alice.group_receiver_ready(group, bob_fp).unwrap());
    let pending = alice.prepare_group_distribution(group, b, now).unwrap();
    let before = alice.group_status(group).unwrap().state;
    let remove = alice
        .prepare_group_change(group, Change::Remove([2; 32]))
        .unwrap();
    let next = before.proposal_from_bytes(&remove).unwrap();
    let receipt = Receipt::sign(
        group,
        before.head(),
        next.head(),
        next.proposed_state().revision(),
        &authority,
    )
    .unwrap()
    .to_bytes();
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE DELETE ON group_senders BEGIN SELECT RAISE(ABORT,'synthetic key cutover failure'); END;").unwrap();
    assert!(alice
        .commit_group_proposal(group, &remove, &receipt)
        .is_err());
    assert_eq!(
        alice.group_status(group).unwrap().state.head(),
        before.head()
    );
    assert!(alice
        .pending([3; 32])
        .unwrap()
        .iter()
        .any(|(id, _)| *id == pending));
    assert!(alice.group_receiver_ready(group, bob_fp).unwrap());
    alice.db.execute_batch("DROP TRIGGER fail;").unwrap();
    assert_eq!(
        alice
            .commit_group_proposal(group, &remove, &receipt)
            .unwrap(),
        CommitResult::Applied
    );
    assert_eq!(count(&alice, "group_senders"), 0);
    assert_eq!(count(&alice, "group_receivers"), 0);
    assert_eq!(count(&alice, "group_key_outbox"), 0);
    assert!(alice.pending([3; 32]).unwrap().is_empty());
    assert!(alice.prepare_group_distribution(group, b, now).is_err());
    bob.commit_group_proposal(group, &remove, &receipt).unwrap();
    assert_eq!(count(&bob, "group_senders"), 0);
    alice
        .send_peer_text(b, [91; 32], "direct session still works", now, now)
        .unwrap();
    alice.send_pending_online([3; 32], now).unwrap();
    assert!(bob.receive_mailbox_online(now).unwrap().iter().any(|v| matches!(&v.result, Ok(crate::MailboxEvent::Text(t)) if t.text().unwrap().body == "direct session still works")));
}

#[test]
fn pairwise_authentication_cannot_substitute_for_group_join_approval() {
    let (_dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    let authority = IdentityKey::generate().unwrap();
    let group = staged(&mut alice, &mut bob, &authority);
    let (_, b, bob_session) = channel(&mut alice, &mut bob, now);
    assert!(alice.prepare_group_distribution(group, b, now).is_err());
    let state = alice.group_status(group).unwrap().state;
    let alice_fp = device_fingerprint(&alice.own_device_binding().unwrap()).unwrap();
    let bob_binding = peers::parse(&bob.own_device_binding().unwrap()).unwrap();
    let bob_fp = peers::fingerprint(&bob_binding.binding).unwrap();
    let (_, distribution) = sk::Sender::new(group, state.head(), state.epoch(), alice_fp).unwrap();
    let control = wire(&distribution, &bob_fp).unwrap();
    let id = message_id(&distribution.context(), &bob_fp);
    alice.send([3; 32], id, &control).unwrap();
    alice
        .prepare_delivery([3; 32], id, bob_binding.binding.device, now + 300, now)
        .unwrap();
    alice.send_pending_online([3; 32], now).unwrap();
    let before = session_state(&bob, bob_session);
    assert!(matches!(
        bob.accept_delivery(&crate::incoming::tests::next(&bob)),
        Err(Error::Unprepared)
    ));
    assert_eq!(session_state(&bob, bob_session), before);
    assert_eq!(count(&bob, "group_receivers"), 0);
    assert_eq!(count(&bob, "group_controls"), 0);
}

#[test]
fn readiness_flag_rejects_malformed_or_missing_seed_state() {
    let (_dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    let authority = IdentityKey::generate().unwrap();
    let group = staged(&mut alice, &mut bob, &authority);
    join(&mut alice, &mut bob, group, Role::Member, &authority);
    let (_, b, _) = channel(&mut alice, &mut bob, now);
    alice.prepare_group_distribution(group, b, now).unwrap();
    assert!(alice.group_sender_ready(group).unwrap());
    alice
        .db
        .execute(
            "UPDATE group_senders SET seed=x'01' WHERE group_id=?1",
            [group.as_slice()],
        )
        .unwrap();
    assert!(matches!(
        alice.group_sender_ready(group),
        Err(Error::InvalidStore)
    ));
}
