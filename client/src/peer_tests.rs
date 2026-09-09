use super::*;
use crate::connection::tests::{credential, prepare, setup};
use sigil_crypto::{IdentityKey, Secret32};
use sigil_server::store::Store;
fn open(path: &Path) -> ClientStore {
    ClientStore::open(
        path,
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap()
}
fn review(store: &ClientStore) -> Result<Vec<DeviceReview>, Error> {
    let mut cursor = None;
    let mut all = std::collections::BTreeMap::<Id, DeviceReview>::new();
    loop {
        let page = store.review_devices_online(cursor.as_ref())?;
        for mut entry in page.devices {
            if let Some(previous) = all.remove(&entry.device) {
                entry.inventory = entry.inventory.or(previous.inventory);
                entry.peer = entry.peer.or(previous.peer);
            }
            all.insert(entry.device, entry);
        }
        cursor = page.next;
        if cursor.is_none() {
            return Ok(all.into_values().collect());
        }
    }
}
fn changed(original: &[u8], key: &IdentityKey) -> Vec<u8> {
    let mut signed = parse(original).unwrap();
    signed.binding.identity = key.public_key();
    signed.signature = key.sign(&signed.binding.signing_bytes().unwrap()).unwrap();
    signed.to_bytes().unwrap()
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "bulk cryptographic boundary acceptance; run cargo test --release -p sigil-client --lib"
)]
fn superseded_peer_history_turnover_preserves_approval_and_retained_text() {
    let (dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    let old_bytes = bob.own_device_binding().unwrap();
    let old = alice.observe_peer_binding(&old_bytes).unwrap();
    alice.confirm_peer(old.id, old.fingerprint).unwrap();
    crate::incoming::tests::start(&mut alice, old.id, now);
    let history = alice.outgoing_message([3; 32], [4; 32]).unwrap();
    let tx = alice.db.transaction().unwrap();
    for n in 0u32..4096 {
        let identity = IdentityKey::generate().unwrap();
        let mut signed = parse(&old_bytes).unwrap();
        signed.binding.device = [235; 32];
        signed.binding.device[..4].copy_from_slice(&n.to_be_bytes());
        signed.binding.identity = identity.public_key();
        signed.signature = identity
            .sign(&signed.binding.signing_bytes().unwrap())
            .unwrap();
        let reference = peer_id(&signed.binding);
        save(
            &tx,
            &alice.key,
            &reference,
            &Record {
                trusted: true,
                suspended: false,
                signed,
                candidate: None,
                verified: true,
                blocked: false,
                replacement: Some((old.id, old.fingerprint)),
            },
        )
        .unwrap();
    }
    tx.commit().unwrap();
    crate::test_schema::rewind(&alice.db, 41);
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    let identity = IdentityKey::generate().unwrap();
    let mut signed = parse(&old_bytes).unwrap();
    signed.binding.device = [236; 32];
    signed.binding.identity = identity.public_key();
    signed.signature = identity
        .sign(&signed.binding.signing_bytes().unwrap())
        .unwrap();
    let new = alice
        .observe_peer_binding(&signed.to_bytes().unwrap())
        .unwrap();
    alice
        .approve_peer_replacement(old.id, new.id, old.fingerprint, new.fingerprint)
        .unwrap();
    alice.observe_peer_binding(&old_bytes).unwrap();
    alice.block_peer(old.id, false).unwrap();
    assert!(!alice.peer(old.id).unwrap().trusted);
    assert!(alice.confirm_peer(old.id, old.fingerprint).is_err());
    assert!(alice.peer(new.id).unwrap().trusted);
    assert_eq!(alice.outgoing_message([3; 32], [4; 32]).unwrap(), history);
    assert_eq!(
        alice
            .db
            .query_row("SELECT count(*) FROM peers WHERE obsolete=0", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    // Paginated local review also passes historical peers beyond the old bound.
    assert_eq!(review(&alice).unwrap().len(), 1);
}

#[test]
fn binding_signature_covers_every_byte_and_restarts_keep_exact_publication() {
    let (dir, fixture, invitation, _) = setup();
    let path = dir.path().join("client.db");
    let mut client = open(&path);
    prepare(&mut client, &fixture, &invitation.secret);
    client.enroll_online().unwrap();
    let bytes = client.own_device_binding().unwrap();
    let fingerprint = device_fingerprint(&bytes).unwrap();
    for n in 0..bytes.len() {
        let mut bad = bytes.clone();
        bad[n] ^= 1;
        assert!(device_fingerprint(&bad).is_err(), "field {n}");
    }
    client.publish_device_binding_online().unwrap();
    drop(client);
    let mut client = open(&path);
    assert_eq!(client.own_device_binding().unwrap(), bytes);
    assert_eq!(
        device_fingerprint(&client.own_device_binding().unwrap()).unwrap(),
        fingerprint
    );
    client.publish_device_binding_online().unwrap();
    let mut server = Store::open(&dir.path().join("server.db")).unwrap();
    let session = client.connection_session().unwrap().unwrap();
    let stored = client
        .connected_client()
        .unwrap()
        .device_binding(&session.device_id)
        .unwrap()
        .bytes()
        .unwrap();
    assert_eq!(stored, bytes);
    let altered = changed(&bytes, &IdentityKey::generate().unwrap());
    assert!(server
        .publish_device_binding(
            &credential(&client),
            Statement {
                statement: transport::hex(&altered)
            },
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        )
        .is_err());
}

#[test]
fn explicit_confirmation_and_key_change_quarantine_survive_restart() {
    let (dir, fixture, invitation, _) = setup();
    let path = dir.path().join("client.db");
    let mut client = open(&path);
    prepare(&mut client, &fixture, &invitation.secret);
    client.enroll_online().unwrap();
    let bytes = client.own_device_binding().unwrap();
    let peer = client.observe_peer_binding(&bytes).unwrap();
    assert!(!peer.trusted);
    assert!(client.prepare_peer_claim([1; 32], peer.id).is_err());
    assert!(matches!(
        client.confirm_peer(peer.id, [8; 32]),
        Err(Error::Conflict)
    ));
    client
        .confirm_peer(peer.id, device_fingerprint(&bytes).unwrap())
        .unwrap();
    client.prepare_peer_claim([1; 32], peer.id).unwrap();
    assert!(matches!(
        client.prepare_prekey_claim([1; 32], peer.binding.device, peer.binding.identity),
        Err(Error::Conflict)
    ));
    client.block_peer(peer.id, true).unwrap();
    assert!(!client.peer(peer.id).unwrap().trusted);
    assert!(client.claim_prekey_online([1; 32], 1000).is_err());
    client.block_peer(peer.id, false).unwrap();
    assert!(client.peer(peer.id).unwrap().trusted);
    let replacement = changed(&bytes, &IdentityKey::generate().unwrap());
    let changed = client.observe_peer_binding(&replacement).unwrap();
    assert!(!changed.trusted);
    assert_eq!(changed.fingerprint, peer.fingerprint);
    assert_eq!(
        changed.changed_fingerprint,
        Some(device_fingerprint(&replacement).unwrap())
    );
    assert!(client
        .confirm_peer(peer.id, device_fingerprint(&replacement).unwrap())
        .is_err());
    assert!(client.confirm_peer(peer.id, peer.fingerprint).is_err());
    assert!(client.claim_prekey_online([1; 32], 1000).is_err());
    assert!(client
        .start_claimed_initial([1; 32], [2; 32], [3; 32], b"blocked", 1000)
        .is_err());
    client.observe_peer_binding(&bytes).unwrap();
    drop(client);
    let client = open(&path);
    assert!(!client.peer(peer.id).unwrap().trusted);
    assert!(client.peer(peer.id).unwrap().changed_fingerprint.is_some());
}

#[test]
fn confirmation_storage_failure_does_not_grant_trust_and_history_migration_invents_none() {
    let (dir, fixture, invitation, _) = setup();
    let path = dir.path().join("client.db");
    let mut client = open(&path);
    prepare(&mut client, &fixture, &invitation.secret);
    client.enroll_online().unwrap();
    let bytes = client.own_device_binding().unwrap();
    crate::test_schema::rewind(&client.db, 12);
    drop(client);
    let mut client = open(&path);
    assert_eq!(
        client
            .db
            .query_row("SELECT count(*) FROM peers", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    let peer = client.observe_peer_binding(&bytes).unwrap();
    client.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON peers BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(client.confirm_peer(peer.id, peer.fingerprint).is_err());
    assert!(!client.peer(peer.id).unwrap().trusted);
    client.db.execute_batch("DROP TRIGGER fail;").unwrap();
    client.confirm_peer(peer.id, peer.fingerprint).unwrap();
    let mut signed = parse(&bytes).unwrap();
    let key = IdentityKey::generate().unwrap();
    signed.binding.identity = key.public_key();
    signed.binding.device = [7; 32];
    signed.signature = key.sign(&signed.binding.signing_bytes().unwrap()).unwrap();
    let new = client
        .observe_peer_binding(&signed.to_bytes().unwrap())
        .unwrap();
    assert_ne!(new.id, peer.id);
    assert!(!new.trusted);
    assert!(client.peer(peer.id).unwrap().trusted);
}

#[test]
fn two_peers_verify_from_independent_material_before_claiming_and_recheck_before_initial() {
    let (_dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    let alice_bytes = alice.own_device_binding().unwrap();
    let bob_bytes = bob.own_device_binding().unwrap();
    alice.publish_device_binding_online().unwrap();
    bob.publish_device_binding_online().unwrap();
    let a = bob
        .fetch_peer_online(parse(&alice_bytes).unwrap().binding.device)
        .unwrap();
    let b = alice
        .fetch_peer_online(parse(&bob_bytes).unwrap().binding.device)
        .unwrap();
    assert!(!a.trusted && !b.trusted);
    assert!(alice.prepare_peer_claim([1; 32], b.id).is_err());
    alice
        .confirm_peer(b.id, device_fingerprint(&bob_bytes).unwrap())
        .unwrap();
    bob.confirm_peer(a.id, device_fingerprint(&alice_bytes).unwrap())
        .unwrap();
    alice.prepare_peer_claim([1; 32], b.id).unwrap();
    alice.claim_prekey_online([1; 32], now).unwrap();
    alice.block_peer(b.id, true).unwrap();
    assert!(alice
        .start_claimed_initial([1; 32], [3; 32], [4; 32], b"hello", now)
        .is_err());
    alice.block_peer(b.id, false).unwrap();
    let packet = alice
        .start_claimed_initial([1; 32], [3; 32], [4; 32], b"hello", now)
        .unwrap();
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON sessions WHEN NEW.peer IS NOT NULL BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(bob
        .accept_peer_initial(a.id, [3; 32], [4; 32], &packet)
        .is_err());
    assert_eq!(
        bob.db
            .query_row("SELECT count(state) FROM prekeys", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        bob.db
            .query_row("SELECT count(*) FROM inbox", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    bob.db.execute_batch("DROP TRIGGER fail;").unwrap();
    bob.accept_peer_initial(a.id, [3; 32], [4; 32], &packet)
        .unwrap();
    let reply = bob.send([3; 32], [5; 32], b"verified reply").unwrap();
    assert_eq!(
        alice.receive([3; 32], [5; 32], &reply).unwrap().as_slice(),
        b"verified reply"
    );
    alice.send([3; 32], [6; 32], b"pending").unwrap();
    alice.block_peer(b.id, true).unwrap();
    assert!(alice.send([3; 32], [7; 32], b"blocked").is_err());
    assert!(alice.pending([3; 32]).is_err());
    assert!(alice.pending_deliveries([3; 32], now).is_err());
    assert!(alice.send_pending_online([3; 32], now).is_err());
    assert_eq!(
        alice.message([3; 32], [5; 32]).unwrap().as_slice(),
        b"verified reply"
    );
    alice.block_peer(b.id, false).unwrap();
    assert_eq!(alice.pending([3; 32]).unwrap().len(), 2);
    assert!(alice
        .prepare_delivery([3; 32], [6; 32], [9; 32], now + 60, now)
        .is_err());
    alice
        .prepare_delivery([3; 32], [6; 32], b.binding.device, now + 60, now)
        .unwrap();
    // Removing or substituting the clear lookup cannot downgrade a bound checkpoint.
    for replacement in [None, Some([9; 32].as_slice())] {
        alice
            .db
            .execute("UPDATE sessions SET peer=?1", [replacement])
            .unwrap();
        assert!(alice.pending([3; 32]).is_err());
        assert!(alice.send([3; 32], [7; 32], b"tampered route").is_err());
    }
    alice
        .db
        .execute("UPDATE sessions SET peer=?1", [b.id.as_slice()])
        .unwrap();
    assert_eq!(alice.pending([3; 32]).unwrap().len(), 2);
    let replacement = changed(&bob_bytes, &IdentityKey::generate().unwrap());
    alice.observe_peer_binding(&replacement).unwrap();
    assert!(alice.send([3; 32], [7; 32], b"changed identity").is_err());
    assert!(alice.pending_deliveries([3; 32], now).is_err());
}

#[test]
fn concurrent_confirmation_and_key_change_cannot_leave_the_peer_verified() {
    let (dir, fixture, invitation, _) = setup();
    let path = dir.path().join("client.db");
    let mut client = open(&path);
    prepare(&mut client, &fixture, &invitation.secret);
    client.enroll_online().unwrap();
    let bytes = client.own_device_binding().unwrap();
    let peer = client.observe_peer_binding(&bytes).unwrap();
    let replacement = changed(&bytes, &IdentityKey::generate().unwrap());
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let confirm = scope.spawn(|| {
            let mut store = open(&path);
            barrier.wait();
            store.confirm_peer(peer.id, peer.fingerprint)
        });
        let observe = scope.spawn(|| {
            let mut store = open(&path);
            barrier.wait();
            store.observe_peer_binding(&replacement).unwrap()
        });
        let _ = confirm.join().unwrap();
        assert!(!observe.join().unwrap().trusted);
    });
    let current = open(&path).peer(peer.id).unwrap();
    assert!(!current.trusted);
    assert_eq!(
        current.changed_fingerprint,
        Some(device_fingerprint(&replacement).unwrap())
    );
}

#[test]
fn retired_sessions_free_peer_slots_even_when_peer_is_blocked() {
    let (dir, fixture, invitation, _) = setup();
    let mut client = open(&dir.path().join("sessions.db"));
    prepare(&mut client, &fixture, &invitation.secret);
    client.enroll_online().unwrap();
    let bytes = client.own_device_binding().unwrap();
    let peer = client.observe_peer_binding(&bytes).unwrap();
    client
        .confirm_peer(peer.id, device_fingerprint(&bytes).unwrap())
        .unwrap();
    for n in 1..=9 {
        let dh = sigil_crypto::DhKey::generate().unwrap();
        let state =
            Session::initiator(Secret32::from_bytes([7; 32]), dh.public_key(), [8; 32]).unwrap();
        client.insert_session([n; 32], state).unwrap();
        let tx = client
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let result = bind_session(&tx, &client.key, &[n; 32], &peer.id);
        if n <= 8 {
            result.unwrap();
            tx.commit().unwrap();
        } else {
            assert!(matches!(result, Err(Error::Limit)));
        }
    }
    client.block_peer(peer.id, true).unwrap();
    client.retire_session([1; 32]).unwrap();
    client.retire_session([1; 32]).unwrap();
    client.block_peer(peer.id, false).unwrap();
    let tx = client
        .db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    bind_session(&tx, &client.key, &[9; 32], &peer.id).unwrap();
    tx.commit().unwrap();
    assert_eq!(
        client
            .db
            .query_row(
                "SELECT count(*) FROM sessions WHERE peer=?1 AND retired=0",
                [peer.id.as_slice()],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        8
    );
    assert_eq!(
        client
            .db
            .query_row(
                "SELECT count(*) FROM sessions WHERE peer=?1",
                [peer.id.as_slice()],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        9
    );
}

#[test]
fn account_device_review_keeps_unknown_missing_blocked_and_changed_devices_distinct() {
    let (dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    let own = parse(&alice.own_device_binding().unwrap()).unwrap();
    let bob_bytes = bob.own_device_binding().unwrap();
    let bob_peer = alice.observe_peer_binding(&bob_bytes).unwrap();
    alice
        .confirm_peer(bob_peer.id, bob_peer.fingerprint)
        .unwrap();
    let server = rusqlite::Connection::open(dir.path().join("server.db")).unwrap();
    // Synthetic account devices exercise multiple pages without claiming a link
    // protocol. No server authorization is used as encryption verification.
    for n in 1..=300_u16 {
        let mut device = [n as u8; 32];
        if n > 35 {
            device = [234; 32];
            device[..2].copy_from_slice(&n.to_be_bytes());
        }
        server.execute("INSERT INTO devices(id,account_id,label,expires_at,revoked) VALUES(?1,?2,'Synthetic inventory',?3,?4)",
            (transport::hex(&device), transport::hex(&own.binding.account), (now + 3600) as i64, n == 3)).unwrap();
    }
    let signed = |n: u8| {
        let key = IdentityKey::generate().unwrap();
        let mut binding = own.binding.clone();
        binding.device = [n; 32];
        binding.identity = key.public_key();
        let signature = key.sign(&binding.signing_bytes().unwrap()).unwrap();
        SignedBinding { binding, signature }.to_bytes().unwrap()
    };
    let bytes = signed(1);
    let verified = alice.observe_peer_binding(&bytes).unwrap();
    alice
        .confirm_peer(verified.id, verified.fingerprint)
        .unwrap();
    let blocked = alice.observe_peer_binding(&signed(2)).unwrap();
    alice.confirm_peer(blocked.id, blocked.fingerprint).unwrap();
    alice.block_peer(blocked.id, true).unwrap();
    let changed = alice.observe_peer_binding(&signed(3)).unwrap();
    alice.confirm_peer(changed.id, changed.fingerprint).unwrap();
    alice.observe_peer_binding(&signed(3)).unwrap();
    let missing = alice.observe_peer_binding(&signed(40)).unwrap();
    alice.confirm_peer(missing.id, missing.fingerprint).unwrap();
    let before: Vec<Vec<u8>> = alice
        .db
        .prepare("SELECT state FROM peers ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    drop(alice);
    let alice = open(&dir.path().join("alice.db"));
    let review = self::review(&alice).unwrap();
    assert_eq!(review.len(), 302);
    assert_eq!(review.iter().filter(|entry| entry.is_current).count(), 1);
    let entry = |n: u8| review.iter().find(|entry| entry.device == [n; 32]).unwrap();
    assert!(entry(1).peer.as_ref().unwrap().trusted);
    assert!(entry(2).peer.as_ref().unwrap().blocked);
    assert!(!entry(2).peer.as_ref().unwrap().trusted);
    assert!(entry(3).inventory.as_ref().unwrap().revoked);
    assert!(entry(3)
        .peer
        .as_ref()
        .unwrap()
        .changed_fingerprint
        .is_some());
    assert!(!entry(3).peer.as_ref().unwrap().trusted);
    assert!(entry(4).peer.is_none());
    assert!(entry(40).inventory.is_none());
    assert!(entry(40).peer.as_ref().unwrap().trusted);
    assert!(!review
        .iter()
        .any(|entry| entry.device == bob_peer.binding.device));
    let after: Vec<Vec<u8>> = alice
        .db
        .prepare("SELECT state FROM peers ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(before, after);
    alice
        .db
        .execute(
            "UPDATE peers SET state=zeroblob(length(state)) WHERE id=?1",
            [bob_peer.id.as_slice()],
        )
        .unwrap();
    assert!(self::review(&alice).is_err());
}

#[test]
fn account_device_review_rejects_inventory_conflicting_with_verified_account_binding() {
    let (dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    let peer = alice
        .observe_peer_binding(&bob.own_device_binding().unwrap())
        .unwrap();
    alice.confirm_peer(peer.id, peer.fingerprint).unwrap();
    let session = alice.connection_session().unwrap().unwrap();
    // Simulate a server moving a known device into the caller's inventory.
    let server = rusqlite::Connection::open(dir.path().join("server.db")).unwrap();
    server
        .execute(
            "UPDATE devices SET account_id=?1,expires_at=?2 WHERE id=?3",
            (
                &session.account_id,
                (now + 3600) as i64,
                transport::hex(&peer.binding.device),
            ),
        )
        .unwrap();
    assert!(matches!(review(&alice), Err(Error::Conflict)));
    assert!(alice.peer(peer.id).unwrap().trusted);
    assert_eq!(alice.peer(peer.id).unwrap().fingerprint, peer.fingerprint);
}

#[test]
fn replacement_approval_is_atomic_and_old_trust_cannot_be_revived() {
    let (dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    let old_bytes = bob.own_device_binding().unwrap();
    let old = alice.observe_peer_binding(&old_bytes).unwrap();
    alice.confirm_peer(old.id, old.fingerprint).unwrap();
    crate::incoming::tests::start(&mut alice, old.id, now);
    let queued = alice
        .send_peer_text(old.id, [80; 32], "pending old device", now, now)
        .unwrap();
    let history = alice.outgoing_message(queued.0, [80; 32]).unwrap();
    let key = IdentityKey::generate().unwrap();
    let mut replacement = parse(&old_bytes).unwrap();
    replacement.binding.device = [90; 32];
    replacement.binding.identity = key.public_key();
    replacement.signature = key
        .sign(&replacement.binding.signing_bytes().unwrap())
        .unwrap();
    let replacement_bytes = replacement.to_bytes().unwrap();
    let new = alice.observe_peer_binding(&replacement_bytes).unwrap();
    assert!(alice
        .approve_peer_replacement(old.id, new.id, old.fingerprint, [0; 32])
        .is_err());
    alice.db.execute_batch("CREATE TRIGGER fail_replacement BEFORE UPDATE ON peers BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(alice
        .approve_peer_replacement(old.id, new.id, old.fingerprint, new.fingerprint)
        .is_err());
    alice
        .db
        .execute_batch("DROP TRIGGER fail_replacement;")
        .unwrap();
    assert!(alice.peer(old.id).unwrap().trusted);
    assert_eq!(alice.active_session(old.id).unwrap(), Some([3; 32]));
    assert!(!alice.peer(new.id).unwrap().trusted);
    alice
        .approve_peer_replacement(old.id, new.id, old.fingerprint, new.fingerprint)
        .unwrap();
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    alice
        .approve_peer_replacement(old.id, new.id, old.fingerprint, new.fingerprint)
        .unwrap();
    assert!(alice.peer(new.id).unwrap().trusted);
    assert_eq!(alice.peer(old.id).unwrap().replaced_by, Some(new.id));
    alice.observe_peer_binding(&old_bytes).unwrap();
    alice.block_peer(old.id, false).unwrap();
    assert!(!alice.peer(old.id).unwrap().trusted);
    assert!(alice.confirm_peer(old.id, old.fingerprint).is_err());
    assert!(alice.prepare_peer_claim([99; 32], old.id).is_err());
    assert!(alice.send_pending_online(queued.0, now).is_err());
    assert!(alice
        .send_text(queued.0, [81; 32], "must not send", now, now)
        .is_err());
    assert_eq!(alice.outgoing_message(queued.0, [80; 32]).unwrap(), history);
    assert!(alice
        .approve_peer_replacement(new.id, old.id, new.fingerprint, old.fingerprint)
        .is_err());
}

#[test]
fn reviewed_link_endorsement_requires_existing_trust_and_preserves_quarantine() {
    let (dir, _fixture, mut contact, mut sponsor, now) = crate::claims::tests::pair();
    let mut joining = open(&dir.path().join("endorsement-joining.db"));
    let sponsor_bytes = sponsor.own_device_binding().unwrap();
    let joining_bytes = joining
        .sign_joining_device_binding(&sponsor_bytes, [91; 32])
        .unwrap();
    let sponsor_peer = contact.observe_peer_binding(&sponsor_bytes).unwrap();
    let transcript = sigil_protocol::link::Transcript {
        sponsor: device_fingerprint(&sponsor_bytes).unwrap(),
        joining: device_fingerprint(&joining_bytes).unwrap(),
        sponsor_challenge: [92; 32],
        joining_challenge: [93; 32],
        provisioning_key: sigil_crypto::DhKey::generate().unwrap().public_key(),
        credential_commitment: [94; 32],
        created_at: now,
        expires_at: now + 600,
    };
    let digest = crate::link::confirmation(&transcript).unwrap();
    let proof = sigil_protocol::link::Proof {
        sponsor_signature: sponsor
            .sign_device_link_consent(&transcript, &sponsor_bytes, &joining_bytes, digest, now)
            .unwrap(),
        joining_signature: joining
            .sign_device_link_consent(&transcript, &sponsor_bytes, &joining_bytes, digest, now)
            .unwrap(),
        transcript,
        sponsor: parse(&sponsor_bytes).unwrap(),
        joining: parse(&joining_bytes).unwrap(),
    };
    assert!(contact
        .accept_linked_peer(&proof, sponsor_peer.id, now)
        .is_err());
    contact
        .confirm_peer(sponsor_peer.id, sponsor_peer.fingerprint)
        .unwrap();
    assert!(contact
        .accept_linked_peer(&proof, sponsor_peer.id, now + 600)
        .is_err());
    let child = reference(&proof.joining.binding.server, &proof.joining.binding.device);
    contact.db.execute_batch("CREATE TRIGGER fail_endorsement BEFORE INSERT ON peers BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(contact
        .accept_linked_peer(&proof, sponsor_peer.id, now)
        .is_err());
    assert!(matches!(contact.peer(child), Err(Error::NotFound)));
    contact
        .db
        .execute_batch("DROP TRIGGER fail_endorsement;")
        .unwrap();
    assert_eq!(
        contact
            .accept_linked_peer(&proof, sponsor_peer.id, now)
            .unwrap(),
        child
    );
    assert!(contact.peer(child).unwrap().trusted);
    contact.block_peer(child, true).unwrap();
    assert!(contact
        .accept_linked_peer(&proof, sponsor_peer.id, now)
        .is_err());
    assert!(!contact.peer(child).unwrap().trusted);
    contact.block_peer(child, false).unwrap();
    contact.block_peer(sponsor_peer.id, true).unwrap();
    assert!(contact
        .accept_linked_peer(&proof, sponsor_peer.id, now)
        .is_err());
    contact.block_peer(sponsor_peer.id, false).unwrap();
    let other = IdentityKey::generate().unwrap();
    let mut changed = proof.sponsor.clone();
    changed.binding.identity = other.public_key();
    changed.signature = other
        .sign(&changed.binding.signing_bytes().unwrap())
        .unwrap();
    contact
        .observe_peer_binding(&changed.to_bytes().unwrap())
        .unwrap();
    assert!(contact
        .accept_linked_peer(&proof, sponsor_peer.id, now)
        .is_err());
}

fn directory(bindings: &[Vec<u8>]) -> sigil_protocol::admin::ContactDirectory {
    let parsed: Vec<_> = bindings.iter().map(|v| parse(v).unwrap()).collect();
    let owner = &parsed[0].binding;
    let mut devices: Vec<_> = parsed
        .iter()
        .map(|s| transport::hex(&s.binding.device))
        .collect();
    devices.sort();
    sigil_protocol::admin::ContactDirectory {
        account: sigil_protocol::admin::FoundAccount {
            address: format!("@{}:{}", owner.username, owner.server),
            account: transport::hex(&owner.account),
            devices,
        },
        bindings: bindings.iter().map(|v| transport::hex(v)).collect(),
        links: vec![],
    }
}
#[test]
fn directory_trust_is_not_manual_verification_and_replacements_require_exact_review() {
    let (dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    let original = bob.own_device_binding().unwrap();
    let binding = parse(&original).unwrap().binding;
    let first = directory(std::slice::from_ref(&original));
    let apply = |store: &mut ClientStore,
                 dir: &sigil_protocol::admin::ContactDirectory,
                 accepted,
                 approval| {
        store.reconcile_contact_trust(
            dir,
            (&binding.server, &binding.username, binding.account),
            accepted,
            approval,
            now,
        )
    };
    assert_eq!(apply(&mut alice, &first, false, None).unwrap(), None);
    let old = reference(&binding.server, &binding.device);
    assert!(!alice.peer(old).unwrap().trusted);
    apply(&mut alice, &first, true, None).unwrap();
    assert!(alice.peer(old).unwrap().trusted);
    assert!(!alice.peer(old).unwrap().verified);
    crate::incoming::tests::start(&mut alice, old, now);
    let queued = alice
        .send_peer_text(old, [161; 32], "Retained synthetic draft", now, now)
        .unwrap();
    let history = alice.outgoing_message(queued.0, [161; 32]).unwrap();
    let key = IdentityKey::generate().unwrap();
    let mut signed = parse(&changed(&original, &key)).unwrap();
    signed.binding.device = [162; 32];
    signed.signature = key.sign(&signed.binding.signing_bytes().unwrap()).unwrap();
    let replacement = directory(&[signed.to_bytes().unwrap()]);
    let review = apply(&mut alice, &replacement, true, None)
        .unwrap()
        .unwrap();
    assert!(!alice.peer(old).unwrap().trusted);
    assert!(alice.send_pending_online(queued.0, now).is_err());
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    assert!(!alice.peer(old).unwrap().trusted);
    assert!(apply(&mut alice, &replacement, true, Some([0; 32])).is_err());
    apply(&mut alice, &replacement, true, Some(review)).unwrap();
    let new = reference(&binding.server, &signed.binding.device);
    assert!(alice.peer(new).unwrap().trusted);
    assert!(!alice.peer(new).unwrap().verified);
    assert_eq!(alice.peer(old).unwrap().replaced_by, Some(new));
    assert_eq!(
        alice.outgoing_message(queued.0, [161; 32]).unwrap(),
        history
    );
    assert!(apply(&mut alice, &first, true, None).is_err());
    alice.block_peer(new, true).unwrap();
    apply(&mut alice, &replacement, true, None).unwrap();
    assert!(!alice.peer(new).unwrap().trusted);
}
#[test]
fn directory_link_endorsement_survives_consent_expiry_but_rejects_tampering() {
    let (dir, _fixture, mut contact, mut sponsor, now) = crate::claims::tests::pair();
    let mut joining = open(&dir.path().join("directory-child.db"));
    let sponsor_bytes = sponsor.own_device_binding().unwrap();
    let child_bytes = joining
        .sign_joining_device_binding(&sponsor_bytes, [171; 32])
        .unwrap();
    let binding = parse(&sponsor_bytes).unwrap().binding;
    let apply = |store: &mut ClientStore, dir: &sigil_protocol::admin::ContactDirectory| {
        store.reconcile_contact_trust(
            dir,
            (&binding.server, &binding.username, binding.account),
            true,
            None,
            now + 1000,
        )
    };
    apply(
        &mut contact,
        &directory(std::slice::from_ref(&sponsor_bytes)),
    )
    .unwrap();
    let transcript = sigil_protocol::link::Transcript {
        sponsor: device_fingerprint(&sponsor_bytes).unwrap(),
        joining: device_fingerprint(&child_bytes).unwrap(),
        sponsor_challenge: [172; 32],
        joining_challenge: [173; 32],
        provisioning_key: sigil_crypto::DhKey::generate().unwrap().public_key(),
        credential_commitment: [174; 32],
        created_at: now,
        expires_at: now + 600,
    };
    let digest = crate::link::confirmation(&transcript).unwrap();
    let proof = sigil_protocol::link::Proof {
        sponsor_signature: sponsor
            .sign_device_link_consent(&transcript, &sponsor_bytes, &child_bytes, digest, now)
            .unwrap(),
        joining_signature: joining
            .sign_device_link_consent(&transcript, &sponsor_bytes, &child_bytes, digest, now)
            .unwrap(),
        transcript,
        sponsor: parse(&sponsor_bytes).unwrap(),
        joining: parse(&child_bytes).unwrap(),
    };
    let mut next = directory(&[child_bytes]);
    let mut tampered = proof.clone();
    tampered.sponsor_signature[0] ^= 1;
    next.links = vec![transport::hex(&tampered.to_bytes().unwrap())];
    assert!(apply(&mut contact, &next).is_err());
    next.links = vec![transport::hex(&proof.to_bytes().unwrap())];
    assert_eq!(apply(&mut contact, &next).unwrap(), None);
    let child = contact
        .peer(reference(&binding.server, &[171; 32]))
        .unwrap();
    assert!(child.trusted);
    assert!(!child.verified);
    assert!(
        !contact
            .peer(reference(&binding.server, &binding.device))
            .unwrap()
            .active
    );
}

#[test]
fn recycled_username_requires_approval_and_cannot_restore_the_old_account() {
    let (_dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    let original = bob.own_device_binding().unwrap();
    let old = parse(&original).unwrap().binding;
    let first = directory(&[original]);
    alice
        .reconcile_contact_trust(
            &first,
            (&old.server, &old.username, old.account),
            true,
            None,
            now,
        )
        .unwrap();
    let key = IdentityKey::generate().unwrap();
    let mut binding = old.clone();
    binding.account = [181; 32];
    binding.device = [182; 32];
    binding.identity = key.public_key();
    let signature = key.sign(&binding.signing_bytes().unwrap()).unwrap();
    let next = directory(&[SignedBinding {
        binding: binding.clone(),
        signature,
    }
    .to_bytes()
    .unwrap()]);
    let review = alice
        .reconcile_contact_trust(
            &next,
            (&binding.server, &binding.username, binding.account),
            true,
            None,
            now,
        )
        .unwrap()
        .unwrap();
    let old_id = reference(&old.server, &old.device);
    assert!(!alice.peer(old_id).unwrap().trusted);
    alice
        .reconcile_contact_trust(
            &next,
            (&binding.server, &binding.username, binding.account),
            true,
            Some(review),
            now,
        )
        .unwrap();
    let new_id = reference(&binding.server, &binding.device);
    assert!(alice.peer(new_id).unwrap().trusted);
    assert!(!alice.peer(new_id).unwrap().verified);
    assert_eq!(alice.peer(old_id).unwrap().replaced_by, Some(new_id));
    assert!(alice
        .reconcile_contact_trust(
            &first,
            (&old.server, &old.username, old.account),
            true,
            None,
            now
        )
        .is_err());
    assert!(alice.peer(new_id).unwrap().trusted);
}
