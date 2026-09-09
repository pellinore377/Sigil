use super::*;
use crate::connection::tests::{credential, prepare, setup};
use crate::network::tests::Fixture;
use sigil_crypto::Secret32;
use sigil_protocol::accounts::InviteRequest;
use sigil_server::store::Store;
const CLAIM: Id = [1; 32];
const SLOT: Id = [2; 32];
const SESSION: Id = [3; 32];
const MESSAGE: Id = [4; 32];
fn open(path: &Path) -> ClientStore {
    ClientStore::open(
        path,
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap()
}
pub(crate) fn pair() -> (tempfile::TempDir, Fixture, ClientStore, ClientStore, u64) {
    pair_with_bob_key([9; 32])
}
pub(crate) fn pair_with_bob_key(
    bob_key: Id,
) -> (tempfile::TempDir, Fixture, ClientStore, ClientStore, u64) {
    let (dir, fixture, invitation, now) = setup();
    let mut alice = open(&dir.path().join("alice.db"));
    prepare(&mut alice, &fixture, &invitation.secret);
    let enrolled = alice.enroll_online().unwrap();
    let mut server = Store::open(&dir.path().join("server.db")).unwrap();
    let invitation = server
        .invite(
            InviteRequest {
                username: "bob".into(),
                expires_in_seconds: 60,
            },
            now,
        )
        .unwrap();
    let mut bob = ClientStore::open(
        &dir.path().join("bob.db"),
        StorageKey::new(Secret32::from_bytes(bob_key)).unwrap(),
    )
    .unwrap();
    prepare(&mut bob, &fixture, &invitation.secret);
    bob.enroll_online().unwrap();
    server
        .allow_sender(&credential(&bob), &enrolled.device_id, now)
        .unwrap();
    server
        .allow_sender(&credential(&alice), &transport::hex(&device(&bob)), now)
        .unwrap();
    bob.prepare_prekey_publication(SLOT, true, 3600).unwrap();
    bob.publish_prekey_online(SLOT).unwrap();
    (dir, fixture, alice, bob, now)
}
fn device(store: &ClientStore) -> Id {
    let value = store.connection_session().unwrap().unwrap().device_id;
    let mut id = [0; 32];
    for (output, pair) in id.iter_mut().zip(value.as_bytes().as_chunks::<2>().0) {
        *output = u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap();
    }
    id
}

#[test]
fn lost_claim_response_retries_same_assignment_and_initial_state_commits_atomically() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    alice
        .prepare_prekey_claim(CLAIM, device(&bob), bob.identity().unwrap())
        .unwrap();
    let identity = alice.identity().unwrap();
    let before = load(&alice.db, &alice.key, &CLAIM, &identity).unwrap().1;
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON prekey_claims BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(alice.claim_prekey_online(CLAIM, now).is_err());
    assert_eq!(
        load(&alice.db, &alice.key, &CLAIM, &identity).unwrap().1,
        before
    );
    let server = rusqlite::Connection::open(dir.path().join("server.db")).unwrap();
    assert_eq!(
        server
            .query_row(
                "SELECT count(*) FROM prekeys WHERE claimant IS NOT NULL",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    alice.db.execute_batch("DROP TRIGGER fail;").unwrap();
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    let response = alice.claim_prekey_online(CLAIM, now).unwrap();
    assert_eq!(alice.claim_prekey_online(CLAIM, now + 1).unwrap(), response);
    for table in ["outbox", "deliveries"] {
        alice.db.execute_batch(&format!("CREATE TRIGGER fail BEFORE INSERT ON {table} BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;")).unwrap();
        assert!(alice
            .start_claimed_initial(CLAIM, SESSION, MESSAGE, b"hello", now)
            .is_err());
        for table in ["sessions", "outbox", "deliveries", "initiations"] {
            assert_eq!(
                alice
                    .db
                    .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
        alice.db.execute_batch("DROP TRIGGER fail;").unwrap();
    }
    let packet = alice
        .start_claimed_initial(CLAIM, SESSION, MESSAGE, b"hello", now)
        .unwrap();
    assert_eq!(
        alice
            .start_claimed_initial(CLAIM, SESSION, MESSAGE, b"hello", response.expires_at)
            .unwrap(),
        packet
    );
    assert!(matches!(
        alice.start_claimed_initial(CLAIM, [9; 32], MESSAGE, b"new", response.expires_at),
        Err(Error::Expired)
    ));
    assert!(matches!(
        alice.claim_prekey_online(CLAIM, response.expires_at),
        Err(Error::Expired)
    ));
    let frozen = alice.pending_deliveries(SESSION, now).unwrap();
    assert_eq!(frozen[0].recipient_device, transport::hex(&device(&bob)));
    assert_eq!(frozen[0].expires_at, now + 604800);
    assert!(matches!(
        alice.prepare_delivery(SESSION, MESSAGE, [8; 32], now + 604800, now),
        Err(Error::Conflict)
    ));
    assert_eq!(alice.send_pending_online(SESSION, now).unwrap().accepted, 1);
    let deliveries = bob.connected_client().unwrap().mailbox().unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].payload, transport::hex(&packet));
    bob.accept_initial(SLOT, SESSION, MESSAGE, identity, &packet)
        .unwrap();
    bob.connected_client()
        .unwrap()
        .acknowledge_delivery(deliveries[0].sequence)
        .unwrap();
    let reply = bob.send(SESSION, [5; 32], b"immediate").unwrap();
    bob.prepare_delivery(SESSION, [5; 32], device(&alice), now + 60, now)
        .unwrap();
    assert_eq!(bob.send_pending_online(SESSION, now).unwrap().accepted, 1);
    let deliveries = alice.connected_client().unwrap().mailbox().unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].payload, transport::hex(&reply));
    assert_eq!(
        alice.receive(SESSION, [5; 32], &reply).unwrap().as_slice(),
        b"immediate"
    );
    alice
        .connected_client()
        .unwrap()
        .acknowledge_delivery(deliveries[0].sequence)
        .unwrap();
}

#[test]
fn malicious_bundle_or_kem_identifier_never_becomes_a_ready_claim() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let expected = bob.identity().unwrap();
    alice
        .prepare_prekey_claim(CLAIM, device(&bob), expected)
        .unwrap();
    let server = rusqlite::Connection::open(dir.path().join("server.db")).unwrap();
    let (id, bundle): (String, Vec<u8>) = server
        .query_row("SELECT id,bundle FROM prekeys", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    let mut changed = bundle.clone();
    changed[90] ^= 1;
    server
        .execute("UPDATE prekeys SET bundle=?1", [&changed])
        .unwrap();
    assert!(alice.claim_prekey_online(CLAIM, now).is_err());
    server
        .execute(
            "UPDATE prekeys SET bundle=?1,id=?2",
            (&bundle, transport::hex(&[8; 32])),
        )
        .unwrap();
    assert!(alice.claim_prekey_online(CLAIM, now).is_err());
    let identity = alice.identity().unwrap();
    assert!(matches!(
        load(&alice.db, &alice.key, &CLAIM, &identity)
            .unwrap()
            .0
            .phase,
        Phase::Pending
    ));
    assert!(matches!(
        alice.prepare_prekey_claim(CLAIM, device(&bob), [8; 32]),
        Err(Error::Conflict)
    ));
    server.execute("UPDATE prekeys SET id=?1", [&id]).unwrap();
    let response = alice.claim_prekey_online(CLAIM, now).unwrap();
    assert_eq!(response.prekey_id, id);
    alice
        .db
        .execute("UPDATE prekey_claims SET phase=0", [])
        .unwrap();
    assert!(matches!(
        alice.claim_prekey_online(CLAIM, now),
        Err(Error::InvalidStore)
    ));
}

#[test]
fn claim_limits_and_abandonment_preserve_request_tombstones_across_migration() {
    let (dir, _fixture, alice, mut bob, now) = pair();
    let expected = bob.identity().unwrap();
    let target = device(&bob);
    crate::test_schema::rewind(&alice.db, 10);
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    for n in 0..64 {
        alice
            .prepare_prekey_claim([n; 32], target, expected)
            .unwrap();
    }
    assert!(matches!(
        alice.prepare_prekey_claim([64; 32], target, expected),
        Err(Error::Limit)
    ));
    alice.abandon_prekey_claim([0; 32]).unwrap();
    alice.abandon_prekey_claim([0; 32]).unwrap();
    assert!(matches!(
        alice.prepare_prekey_claim([0; 32], target, expected),
        Err(Error::AlreadyDelivered)
    ));
    assert!(matches!(
        alice.claim_prekey_online([0; 32], now),
        Err(Error::AlreadyDelivered)
    ));
    alice
        .prepare_prekey_claim([64; 32], target, expected)
        .unwrap();
    assert_eq!(
        alice
            .db
            .query_row("SELECT count(*) FROM prekey_claims", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        65
    );
    assert!(alice
        .start_claimed_initial([0; 32], SESSION, MESSAGE, b"no", now)
        .is_err());
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "bulk cryptographic boundary acceptance; run cargo test --release -p sigil-client --lib"
)]
fn prekey_and_claim_turnover_crosses_retained_limits_and_preserves_consumption() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_, b) = crate::incoming::tests::trust(&mut alice, &mut bob);
    crate::incoming::tests::start(&mut alice, b, now);
    let original = crate::incoming::tests::next(&bob);
    let received = bob.accept_delivery(&original).unwrap();
    bob.acknowledge_incoming_online().unwrap();
    alice.retire_session([3; 32]).unwrap();
    let own = alice.identity().unwrap();
    let known = alice.peer(b).unwrap();
    let tx = alice.db.transaction().unwrap();
    let bob_tx = bob.db.transaction().unwrap();
    let server = rusqlite::Connection::open(dir.path().join("server.db")).unwrap();
    let server_tx = server.unchecked_transaction().unwrap();
    for n in 0u32..4095 {
        let mut id = [232; 32];
        id[..4].copy_from_slice(&n.to_be_bytes());
        let claim = Claim {
            route: None,
            recipient: known.binding.device,
            expected_identity: known.binding.identity,
            peer: Some(b),
            phase: Phase::Abandoned,
        };
        tx.execute(
            "INSERT INTO prekey_claims VALUES(?1,2,?2)",
            (id.as_slice(), seal(&alice.key, &id, &own, &claim).unwrap()),
        )
        .unwrap();
        bob_tx
            .execute("INSERT INTO prekeys VALUES(?1,NULL)", [id.as_slice()])
            .unwrap();
        server_tx.execute("INSERT INTO prekeys(id,device_id,bundle_hash,expires_at) VALUES(?1,?2,zeroblob(32),1)",(transport::hex(&id),transport::hex(&known.binding.device))).unwrap();
    }
    tx.commit().unwrap();
    bob_tx.commit().unwrap();
    server_tx.commit().unwrap();
    // Rebuild server accounting through the real upgrade path for this boundary fixture.
    server.execute_batch("DROP TABLE profile_shares; DROP TABLE profile_photos; DROP TABLE contact_requests; DROP TABLE contact_request_policy; DROP TABLE account_passwords; DROP TABLE password_policy; DROP TABLE web_oidc; DROP TABLE web_sessions; DROP TABLE oidc_fallback_ack; DROP TABLE oidc_transition; DROP TABLE account_profiles; DROP TABLE web_owner; DROP TABLE deleted_accounts; DROP TABLE operation_uploads; DROP TABLE operations; DROP TABLE operation_configuration; DROP TABLE oidc_grants; DROP TABLE oidc_bindings; DROP TABLE oidc_flows; DROP TABLE oidc_configuration; DROP TABLE registration_usage; DROP TABLE account_policy; DROP TABLE admin_policy; DROP TABLE call_connections; DROP TABLE calls; DROP TABLE call_configuration; DROP TABLE service_budgets; DROP TABLE service_configuration; DROP TABLE map_configuration; DROP TABLE private_group_invitations; DROP TABLE private_group_proposals; DROP TABLE private_group_nonces; DROP TABLE private_group_commits; DROP TABLE private_group_members; DROP TABLE private_groups; DROP TABLE group_credential_uids; DROP TABLE group_authority; DROP INDEX prekeys_available; DROP INDEX prekeys_remote_claim; ALTER TABLE prekeys DROP COLUMN remote_request; ALTER TABLE prekeys DROP COLUMN remote_device; ALTER TABLE prekeys DROP COLUMN remote_account; ALTER TABLE prekeys DROP COLUMN remote_server; CREATE INDEX prekeys_available ON prekeys(device_id,expires_at) WHERE bundle IS NOT NULL AND claimant IS NULL; DROP TABLE federation_outbox; DROP TABLE federation_revocations; DROP TABLE federation_senders; DROP TABLE federation_nonces; DROP TABLE federation_admission; DROP TABLE federation_usage; DROP TABLE federation_peers; DROP TABLE federation_configuration; DROP TABLE push_jobs; DROP TABLE push_channels; DROP TABLE push_configuration; DROP TABLE attachment_chunks; DROP TABLE attachments; DROP TABLE retained_storage; DROP INDEX mailbox_live_recipient; DROP INDEX prekeys_available; DROP INDEX devices_active_account; PRAGMA user_version=11;").unwrap();
    drop(Store::open(&dir.path().join("server.db")).unwrap());
    bob.prepare_prekey_publication([99; 32], true, 3600)
        .unwrap();
    bob.publish_prekey_online([99; 32]).unwrap();
    drop(alice);
    drop(bob);
    let mut alice = open(&dir.path().join("alice.db"));
    let mut bob = open(&dir.path().join("bob.db"));
    assert!(matches!(
        bob.create_prekey([2; 32], true),
        Err(Error::AlreadyDelivered)
    ));
    let mut retired = [232; 32];
    retired[..4].copy_from_slice(&0u32.to_be_bytes());
    assert!(matches!(
        alice.prepare_peer_claim(retired, b),
        Err(Error::AlreadyDelivered)
    ));
    alice
        .queue_peer_text(b, [98; 32], "after prekey turnover", now, now)
        .unwrap();
    let step = alice.sync_step_online(now);
    assert!(step.failure.is_none(), "{:?}", step.failure);
    assert!(step.sends[0].result.is_ok(), "{:?}", step.sends[0].result);
    let result = bob.receive_mailbox_online(now).unwrap();
    assert!(result.iter().any(|item|matches!(&item.result,Ok(MailboxEvent::Text(text)) if text.text().unwrap().body=="after prekey turnover")));
    bob.acknowledge_incoming_online().unwrap();
    assert!(bob.accept_delivery(&original).unwrap().duplicate);
    assert_eq!(
        bob.message(received.session, [4; 32]).unwrap(),
        received.plaintext
    );
    assert!(alice.claim_prekey_online([1; 32], now).is_ok());
    assert!(alice
        .start_claimed_initial([1; 32], [97; 32], [96; 32], b"cannot reuse", now)
        .is_err());
}
