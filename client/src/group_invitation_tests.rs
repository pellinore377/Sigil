use super::*;
use crate::groups::service::tests::enable;
use sigil_crypto::Secret32;
fn open(path: &std::path::Path) -> ClientStore {
    ClientStore::open(
        path,
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap()
}
fn recovery(client: &mut ClientStore) {
    let binding = peers::parse(&client.own_device_binding().unwrap())
        .unwrap()
        .binding;
    client
        .configure_recovery(
            &binding.server,
            binding.account,
            Secret32::from_bytes([7; 32]),
        )
        .unwrap();
}
#[test]
fn invitation_requires_explicit_consent_and_survives_initial_queue_failure_and_approval_retry() {
    let (dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    alice.publish_device_binding_online().unwrap();
    bob.publish_device_binding_online().unwrap();
    let (profile, authority) = enable(&dir.path().join("server.db"));
    let group = alice.create_group(authority).unwrap();
    alice
        .pin_group_service(group, &profile, Zeroizing::new([37; 32]))
        .unwrap();
    alice.prepare_group_service_request(group, None).unwrap();
    alice.submit_group_service_online(group, now).unwrap();
    let (a, b) = crate::incoming::tests::trust(&mut alice, &mut bob);
    recovery(&mut alice);
    recovery(&mut bob);
    let id = [81; 32];
    alice
        .prepare_group_invitation(group, b, id, Role::Member, now + 300, now)
        .unwrap();
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON group_invitation_packets BEGIN SELECT RAISE(ABORT,'synthetic invitation packet failure'); END;").unwrap();
    assert!(alice.advance_group_invitation_online(id, now).is_err());
    assert_eq!(
        alice
            .db
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    alice.db.execute_batch("DROP TRIGGER fail").unwrap();
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    alice.advance_group_invitation_online(id, now).unwrap();
    let before: Vec<u8> = alice
        .db
        .query_row("SELECT packet FROM outbox", [], |r| r.get(0))
        .unwrap();
    alice.advance_group_invitation_online(id, now).unwrap();
    assert_eq!(
        alice
            .db
            .query_row("SELECT packet FROM outbox", [], |r| r.get::<_, Vec<u8>>(0))
            .unwrap(),
        before
    );
    assert!(alice
        .resume_outbound_online(now)
        .unwrap()
        .iter()
        .all(|v| v.result.is_ok()));
    let offer = crate::incoming::tests::next(&bob);
    let event = bob.accept_delivery_online(&offer, now).unwrap();
    assert_eq!(reference(&event.plaintext).unwrap(), Some(id));
    assert_eq!(event.plaintext.len(), RECEIPT);
    bob.acknowledge_incoming_online().unwrap();
    assert_eq!(
        bob.group_invitation(id).unwrap().status,
        InvitationStatus::Offered
    );
    bob.resume_group_invitations_online(now).unwrap();
    assert!(matches!(bob.group_status(group), Err(Error::NotFound)));
    assert!(bob.recovery_records(None).unwrap().is_empty());
    bob.accept_group_invitation(id, now).unwrap();
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    bob.advance_group_invitation_online(id, now).unwrap();
    assert_eq!(bob.group_status(group).unwrap().state.members().len(), 1);
    let own = device_fingerprint(&bob.own_device_binding().unwrap()).unwrap();
    let frozen = load(&bob.db, &bob.key, &own, &id)
        .unwrap()
        .approval
        .unwrap();
    bob.advance_group_invitation_online(id, now).unwrap();
    assert_eq!(
        load(&bob.db, &bob.key, &own, &id)
            .unwrap()
            .approval
            .unwrap(),
        frozen
    );
    assert!(bob
        .resume_outbound_online(now)
        .unwrap()
        .iter()
        .all(|v| v.result.is_ok()));
    let approved = alice.receive_mailbox_online(now).unwrap();
    assert!(approved
        .iter()
        .any(|v| matches!(v.result,Ok(crate::MailboxEvent::GroupInvitation(found)) if found==id)));
    alice.acknowledge_incoming_online().unwrap();
    alice.advance_group_invitation_online(id, now).unwrap();
    bob.advance_group_invitation_online(id, now).unwrap();
    alice.advance_group_invitation_online(id, now).unwrap();
    assert_eq!(
        bob.group_invitation(id).unwrap().status,
        InvitationStatus::Joined
    );
    assert_eq!(
        alice.group_invitation(id).unwrap().status,
        InvitationStatus::Joined
    );
    assert_eq!(
        bob.group_status(group).unwrap().state.head(),
        alice.group_status(group).unwrap().state.head()
    );
    assert_eq!(bob.group_status(group).unwrap().state.members().len(), 2);
    assert!(alice.recovery_records(None).unwrap().is_empty());
    assert!(bob.recovery_records(None).unwrap().is_empty());
    assert!(alice.peer(b).unwrap().verified && bob.peer(a).unwrap().verified);
}

#[test]
fn cancelled_offer_survives_restart_and_accepts_an_in_flight_receipt() {
    let (dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    alice.publish_device_binding_online().unwrap();
    bob.publish_device_binding_online().unwrap();
    let (profile, authority) = enable(&dir.path().join("server.db"));
    let group = alice.create_group(authority).unwrap();
    alice
        .pin_group_service(group, &profile, Zeroizing::new([38; 32]))
        .unwrap();
    alice.prepare_group_service_request(group, None).unwrap();
    alice.submit_group_service_online(group, now).unwrap();
    let (_, peer) = crate::incoming::tests::trust(&mut alice, &mut bob);
    let id = [82; 32];
    alice
        .prepare_group_invitation(group, peer, id, Role::Member, now + 300, now)
        .unwrap();
    alice.advance_group_invitation_online(id, now).unwrap();
    let own = device_fingerprint(&alice.own_device_binding().unwrap()).unwrap();
    let message = Capsule::parse(&load(&alice.db, &alice.key, &own, &id).unwrap().capsule)
        .unwrap()
        .message();
    let session: Vec<u8> = alice
        .db
        .query_row(
            "SELECT session FROM deliveries WHERE id=?1",
            [message.as_slice()],
            |r| r.get(0),
        )
        .unwrap();
    let session: Id = session.try_into().unwrap();
    let (requests, _) = alice.outgoing_batch(session, now, 16).unwrap();
    let receipt = alice
        .connected_client()
        .unwrap()
        .submit(&requests[0])
        .unwrap();
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON outbox BEGIN SELECT RAISE(ABORT,'synthetic cancellation failure'); END;").unwrap();
    assert!(alice.cancel_group_invitation(id).is_err());
    assert_eq!(
        alice.group_invitation(id).unwrap().status,
        InvitationStatus::Waiting
    );
    alice.db.execute_batch("DROP TRIGGER fail").unwrap();
    assert_eq!(alice.cancel_group_invitation(id).unwrap(), 1);
    assert!(matches!(
        alice.check_group_invitation_send(session, message, now),
        Err(Error::Cancelled)
    ));
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    assert!(alice
        .outgoing_batch(session, now + 301, 16)
        .unwrap()
        .0
        .is_empty());
    alice.acknowledge_sent(session, message, &receipt).unwrap();
    alice.advance_group_invitation_online(id, now).unwrap();
    assert!(alice.resume_outbound_online(now).unwrap().is_empty());
    let offer = crate::incoming::tests::next(&bob);
    bob.accept_delivery_online(&offer, now).unwrap();
    bob.acknowledge_incoming_online().unwrap();
    bob.accept_group_invitation(id, now).unwrap();
    assert!(matches!(
        bob.advance_group_invitation_online(id, now),
        Err(Error::Network(crate::network::Error::Status {
            code: 403,
            ..
        }))
    ));
    assert_eq!(bob.group_status(group).unwrap().state.members().len(), 1);
    let bob_fingerprint = device_fingerprint(&bob.own_device_binding().unwrap()).unwrap();
    assert!(load(&bob.db, &bob.key, &bob_fingerprint, &id)
        .unwrap()
        .approval
        .is_none());
}

#[test]
fn declined_offer_never_stages_membership_and_cannot_be_reopened() {
    let (dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    alice.publish_device_binding_online().unwrap();
    bob.publish_device_binding_online().unwrap();
    let (profile, authority) = enable(&dir.path().join("server.db"));
    let group = alice.create_group(authority).unwrap();
    alice
        .pin_group_service(group, &profile, Zeroizing::new([39; 32]))
        .unwrap();
    alice.prepare_group_service_request(group, None).unwrap();
    alice.submit_group_service_online(group, now).unwrap();
    let (_, peer) = crate::incoming::tests::trust(&mut alice, &mut bob);
    let id = [83; 32];
    alice
        .prepare_group_invitation(group, peer, id, Role::Member, now + 300, now)
        .unwrap();
    alice.advance_group_invitation_online(id, now).unwrap();
    alice.resume_outbound_online(now).unwrap();
    let offer = crate::incoming::tests::next(&bob);
    bob.accept_delivery_online(&offer, now).unwrap();
    bob.acknowledge_incoming_online().unwrap();
    bob.cancel_group_invitation(id).unwrap();
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    bob.advance_group_invitation_online(id, now).unwrap();
    bob.advance_group_invitation_online(id, now).unwrap();
    assert!(matches!(bob.group_status(group), Err(Error::NotFound)));
    assert!(matches!(
        bob.accept_group_invitation(id, now),
        Err(Error::Cancelled)
    ));
    alice.advance_group_invitation_online(id, now).unwrap();
    assert_eq!(
        alice.group_invitation(id).unwrap().status,
        InvitationStatus::Cancelled
    );
    assert_eq!(alice.group_status(group).unwrap().state.members().len(), 1);
}

#[test]
fn noncreator_admin_invites_and_concurrent_head_rebases_only_the_approved_join() {
    use crate::connection::tests::{credential, prepare};
    let (dir, fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    alice.publish_device_binding_online().unwrap();
    bob.publish_device_binding_online().unwrap();
    let (profile, authority) = enable(&dir.path().join("server.db"));
    let group = alice.create_group(authority).unwrap();
    let (a, _) = crate::incoming::tests::trust(&mut alice, &mut bob);
    bob.accept_group_genesis(a, &alice.group_genesis(group).unwrap())
        .unwrap();
    for client in [&mut alice, &mut bob] {
        client
            .pin_group_service(group, &profile, Zeroizing::new([40; 32]))
            .unwrap();
    }
    alice.prepare_group_service_request(group, None).unwrap();
    alice.submit_group_service_online(group, now).unwrap();
    let add = alice
        .prepare_group_change(
            group,
            Change::Add(
                Member::new([41; 32], Role::Admin, &[bob.own_device_binding().unwrap()]).unwrap(),
            ),
        )
        .unwrap();
    let add = bob.approve_group_proposal(group, &add).unwrap();
    let add = alice.approve_group_proposal(group, &add).unwrap();
    alice
        .prepare_group_service_request(group, Some(&add))
        .unwrap();
    alice.submit_group_service_online(group, now).unwrap();
    bob.sync_group_service_online(group, now).unwrap();
    let mut server = sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap();
    let invite = server
        .invite(
            sigil_protocol::accounts::InviteRequest {
                username: "carol".into(),
                expires_in_seconds: 60,
            },
            now,
        )
        .unwrap();
    let mut carol = open(&dir.path().join("carol.db"));
    prepare(&mut carol, &fixture, &invite.secret);
    carol.enroll_online().unwrap();
    carol.publish_device_binding_online().unwrap();
    carol
        .prepare_prekey_publication([42; 32], true, 3600)
        .unwrap();
    carol.publish_prekey_online([42; 32]).unwrap();
    server
        .allow_sender(
            &credential(&carol),
            &bob.connection_session().unwrap().unwrap().device_id,
            now,
        )
        .unwrap();
    server
        .allow_sender(
            &credential(&bob),
            &carol.connection_session().unwrap().unwrap().device_id,
            now,
        )
        .unwrap();
    let (_, c) = crate::incoming::tests::trust(&mut bob, &mut carol);
    let id = [84; 32];
    bob.prepare_group_invitation(group, c, id, Role::Member, now + 300, now)
        .unwrap();
    bob.advance_group_invitation_online(id, now).unwrap();
    bob.resume_outbound_online(now).unwrap();
    let offer = crate::incoming::tests::next(&carol);
    carol.accept_delivery_online(&offer, now).unwrap();
    carol.acknowledge_incoming_online().unwrap();
    assert!(matches!(
        carol.accept_group_invitation(id, 0),
        Err(Error::Expired)
    ));
    carol.accept_group_invitation(id, now).unwrap();
    carol.advance_group_invitation_online(id, now).unwrap();
    let own = device_fingerprint(&carol.own_device_binding().unwrap()).unwrap();
    let old = load(&carol.db, &carol.key, &own, &id)
        .unwrap()
        .approval
        .unwrap();
    let change = alice
        .prepare_group_change(group, Change::EarlierHistory(true))
        .unwrap();
    alice
        .prepare_group_service_request(group, Some(&change))
        .unwrap();
    alice.submit_group_service_online(group, now).unwrap();
    carol.resume_outbound_online(now).unwrap();
    let approved = bob.receive_mailbox_online(now).unwrap();
    assert!(approved.iter().all(|r| r.result.is_ok()));
    bob.acknowledge_incoming_online().unwrap();
    bob.advance_group_invitation_online(id, now).unwrap();
    assert_eq!(bob.group_status(group).unwrap().state.members().len(), 2);
    carol.advance_group_invitation_online(id, now).unwrap();
    let new = load(&carol.db, &carol.key, &own, &id)
        .unwrap()
        .approval
        .unwrap();
    assert_ne!(
        Approval::parse(&old).unwrap().head,
        Approval::parse(&new).unwrap().head
    );
    // The fair outbound cursor wraps with an empty pass before revisiting this session.
    for _ in 0..2 {
        assert!(carol
            .resume_outbound_online(now)
            .unwrap()
            .iter()
            .all(|r| r.result.is_ok()));
    }
    let delivered = bob.receive_mailbox_online(now).unwrap();
    assert!(!delivered.is_empty());
    assert!(delivered.iter().all(|r| r.result.is_ok()));
    bob.acknowledge_incoming_online().unwrap();
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON groups BEGIN SELECT RAISE(ABORT,'synthetic join acknowledgement failure'); END;").unwrap();
    assert!(bob.advance_group_invitation_online(id, now).is_err());
    bob.db.execute_batch("DROP TRIGGER fail").unwrap();
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    bob.advance_group_invitation_online(id, now).unwrap();
    carol.advance_group_invitation_online(id, now).unwrap();
    assert_eq!(
        carol.group_invitation(id).unwrap().status,
        InvitationStatus::Joined
    );
    assert_eq!(bob.group_status(group).unwrap().state.members().len(), 3);
    assert_eq!(
        carol.group_status(group).unwrap().state.head(),
        bob.group_status(group).unwrap().state.head()
    );
    assert_eq!(
        carol
            .db
            .query_row("SELECT count(*) FROM peers", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        1
    );
}

#[test]
fn invitation_context_is_bound_to_inviter_recipient_and_signed_policy() {
    let (dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    alice.publish_device_binding_online().unwrap();
    bob.publish_device_binding_online().unwrap();
    let (profile, authority) = enable(&dir.path().join("server.db"));
    let group = alice.create_group(authority).unwrap();
    alice
        .pin_group_service(group, &profile, Zeroizing::new([44; 32]))
        .unwrap();
    alice.prepare_group_service_request(group, None).unwrap();
    alice.submit_group_service_online(group, now).unwrap();
    let (a, b) = crate::incoming::tests::trust(&mut alice, &mut bob);
    let id = [85; 32];
    alice
        .prepare_group_invitation(group, b, id, Role::Member, now + 300, now)
        .unwrap();
    let own = device_fingerprint(&alice.own_device_binding().unwrap()).unwrap();
    let record = load(&alice.db, &alice.key, &own, &id).unwrap();
    let recipient = device_fingerprint(&bob.own_device_binding().unwrap()).unwrap();
    let peer = peers::verified(&bob.db, &bob.key, &a).unwrap();
    let capsule = Capsule::parse(&record.capsule).unwrap();
    capsule.verify(&peer, &recipient, now).unwrap();
    assert!(capsule.verify(&peer, &[0; 32], now).is_err());
    assert!(matches!(
        capsule.verify(&peer, &recipient, now + 301),
        Err(Error::Expired)
    ));
    let mut changed = Capsule::parse(&record.capsule).unwrap();
    changed.role = Role::Admin;
    assert!(changed.verify(&peer, &recipient, now).is_err());
    let mut changed = Capsule::parse(&record.capsule).unwrap();
    changed.context[0] ^= 1;
    assert!(changed.verify(&peer, &recipient, now).is_err());
    for n in [0, 7, 8, 168, record.capsule.len() - 1] {
        assert!(Capsule::parse(&record.capsule[..n]).is_err());
    }
    let mut extended = record.capsule.to_vec();
    extended.push(0);
    assert!(Capsule::parse(&extended).is_err());
    // Live-store ciphertext is bound to this device and invitation ID.
    let sealed: Vec<u8> = alice
        .db
        .query_row(
            "SELECT state FROM group_invitations WHERE id=?1",
            [id.as_slice()],
            |r| r.get(0),
        )
        .unwrap();
    bob.db
        .execute(
            "INSERT INTO group_invitations VALUES(?1,?2,?3)",
            (id.as_slice(), group.as_slice(), &sealed),
        )
        .unwrap();
    assert!(bob.group_invitation(id).is_err());
}

fn linked_device(
    sponsor: &mut ClientStore,
    path: &std::path::Path,
    fixture: &crate::network::tests::Fixture,
    now: u64,
) -> ClientStore {
    let mut joining = open(path);
    let offer = joining.prepare_device_link_offer([101; 32], now).unwrap();
    let (proposal, digest) = sponsor
        .prepare_sponsored_link([102; 32], &crate::link::offer_qr(&offer).unwrap(), now)
        .unwrap();
    assert_eq!(
        joining
            .accept_link_proposal([101; 32], &proposal, now)
            .unwrap(),
        digest
    );
    let response = joining
        .confirm_link_proposal([101; 32], digest, now)
        .unwrap();
    sponsor
        .confirm_sponsored_link([102; 32], &response, digest, now)
        .unwrap();
    sponsor.authorize_sponsored_link_online([102; 32]).unwrap();
    joining
        .finish_device_link_online(
            [101; 32],
            fixture.port(),
            &[crate::network::tests::CA.to_vec()],
        )
        .unwrap();
    joining.publish_device_binding_online().unwrap();
    joining
        .prepare_prekey_publication([103; 32], true, 3600)
        .unwrap();
    joining.publish_prekey_online([103; 32]).unwrap();
    joining
}
#[track_caller]
fn deliver_controls(source: &mut ClientStore, target: &mut ClientStore, now: u64) {
    for _ in 0..2 {
        for sent in source.resume_outbound_online(now).unwrap() {
            assert!(sent.result.is_ok(), "{:?}", sent.result);
        }
    }
    for received in target.receive_mailbox_online(now).unwrap() {
        if let Err(error) = received.result {
            panic!("{error:?}");
        }
    }
    target.acknowledge_incoming_online().unwrap();
}
#[test]
fn linked_nonadmin_device_replays_membership_and_requires_administrator_ordering() {
    let (dir, fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    alice.publish_device_binding_online().unwrap();
    bob.publish_device_binding_online().unwrap();
    let (profile, authority) = enable(&dir.path().join("server.db"));
    let group = alice.create_group(authority).unwrap();
    let (a, _) = crate::incoming::tests::trust(&mut alice, &mut bob);
    bob.accept_group_genesis(a, &alice.group_genesis(group).unwrap())
        .unwrap();
    for client in [&mut alice, &mut bob] {
        client
            .pin_group_service(group, &profile, Zeroizing::new([104; 32]))
            .unwrap();
    }
    alice.prepare_group_service_request(group, None).unwrap();
    alice.submit_group_service_online(group, now).unwrap();
    let mut bindings = vec![bob.own_device_binding().unwrap()];
    let mut devices = Vec::new();
    for n in 180..192 {
        let key = IdentityKey::generate().unwrap();
        let mut binding = peers::parse(&bindings[0]).unwrap().binding;
        binding.device = [n; 32];
        binding.identity = key.public_key();
        let signature = key.sign(&binding.signing_bytes().unwrap()).unwrap();
        let bytes = SignedBinding { binding, signature }.to_bytes().unwrap();
        devices.push((device_fingerprint(&bytes).unwrap(), key));
        bindings.push(bytes);
    }
    let change = alice
        .prepare_group_change(
            group,
            Change::Add(Member::new([105; 32], Role::Member, &bindings).unwrap()),
        )
        .unwrap();
    let change = bob.approve_group_proposal(group, &change).unwrap();
    let mut proposal = alice
        .group_status(group)
        .unwrap()
        .state
        .proposal_from_bytes(&change)
        .unwrap();
    for (device, key) in devices {
        proposal.sign(device, &key).unwrap();
    }
    let change = proposal.to_bytes().unwrap();
    assert!(change.len() > 3072);
    alice
        .prepare_group_service_request(group, Some(&change))
        .unwrap();
    alice.submit_group_service_online(group, now).unwrap();
    bob.sync_group_service_online(group, now).unwrap();
    let path = dir.path().join("joining.db");
    let mut joining = linked_device(&mut bob, &path, &fixture, now);
    let joining_binding = joining.own_device_binding().unwrap();
    let fp = device_fingerprint(&joining_binding).unwrap();
    let peer = bob.observe_peer_binding(&joining_binding).unwrap().id;
    let request = [106; 32];
    bob.prepare_group_device_invitation(group, peer, request, now + 600, now)
        .unwrap();
    bob.advance_group_invitation_online(request, now).unwrap();
    deliver_controls(&mut bob, &mut joining, now);
    assert!(matches!(joining.group_status(group), Err(Error::NotFound)));
    joining.accept_group_invitation(request, now).unwrap();
    let step = joining.sync_step_online(now);
    assert!(step.failure.is_none(), "{:?}", step.failure);
    let genesis_head = joining.group_status(group).unwrap().state.head();
    for pass in 0..12 {
        joining
            .advance_group_invitation_online(request, now)
            .unwrap();
        deliver_controls(&mut joining, &mut bob, now);
        bob.advance_group_invitation_online(request, now).unwrap();
        deliver_controls(&mut bob, &mut joining, now);
        if pass == 0 {
            assert_eq!(
                joining.group_status(group).unwrap().state.head(),
                genesis_head
            );
            drop(joining);
            joining = open(&path);
        }
    }
    assert!(!joining.group_status(group).unwrap().state.members()[0]
        .device_fingerprints()
        .unwrap()
        .contains(&fp));
    assert_eq!(
        joining.group_invitation(request).unwrap().status,
        InvitationStatus::Accepted
    );
    let proposals = alice
        .group_relay_proposals_online(group, None, now)
        .unwrap();
    assert_eq!(proposals.len(), 1);
    // A competing policy change consumes the predecessor; durable joining consent rebases.
    let policy = alice
        .prepare_group_change(group, Change::EarlierHistory(true))
        .unwrap();
    alice
        .prepare_group_service_request(group, Some(&policy))
        .unwrap();
    alice.submit_group_service_online(group, now).unwrap();
    drop(joining);
    let mut joining = open(&path);
    for _ in 0..12 {
        joining
            .advance_group_invitation_online(request, now)
            .unwrap();
        deliver_controls(&mut joining, &mut bob, now);
        bob.advance_group_invitation_online(request, now).unwrap();
        deliver_controls(&mut bob, &mut joining, now);
    }
    let proposals = alice
        .group_relay_proposals_online(group, None, now)
        .unwrap();
    assert_eq!(proposals.len(), 1);
    alice
        .prepare_group_relay_commit(group, &proposals[0])
        .unwrap();
    alice.submit_group_service_online(group, now).unwrap();
    for _ in 0..12 {
        bob.advance_group_invitation_online(request, now).unwrap();
        deliver_controls(&mut bob, &mut joining, now);
        joining
            .advance_group_invitation_online(request, now)
            .unwrap();
        deliver_controls(&mut joining, &mut bob, now);
    }
    assert_eq!(
        joining.group_invitation(request).unwrap().status,
        InvitationStatus::Joined
    );
    assert_eq!(
        bob.group_invitation(request).unwrap().status,
        InvitationStatus::Joined
    );
    let status = joining.group_status(group).unwrap();
    assert_eq!(
        status.state.head(),
        alice.group_status(group).unwrap().state.head()
    );
    assert_eq!(status.state.members().len(), 2);
    let bob_member = status
        .state
        .members()
        .iter()
        .find(|m| m.id() == [105; 32])
        .unwrap();
    assert!(bob_member.role() == Role::Member);
    assert!(bob_member.device_fingerprints().unwrap().contains(&fp));
    let server = rusqlite::Connection::open(dir.path().join("server.db")).unwrap();
    assert_eq!(
        server
            .query_row("SELECT count(*) FROM private_group_invitations", [], |r| {
                r.get::<_, u32>(0)
            })
            .unwrap(),
        0
    );
    recovery(&mut joining);
    assert!(joining.recovery_records(None).unwrap().is_empty());
}

#[test]
fn demotion_blocks_queued_invitations_after_restart_with_stale_or_current_membership() {
    use crate::connection::tests::{credential, prepare};
    for synced in [false, true] {
        let (dir, fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
        alice.publish_device_binding_online().unwrap();
        bob.publish_device_binding_online().unwrap();
        let (profile, authority) = enable(&dir.path().join("server.db"));
        let group = alice.create_group(authority).unwrap();
        let (a, _) = crate::incoming::tests::trust(&mut alice, &mut bob);
        bob.accept_group_genesis(a, &alice.group_genesis(group).unwrap())
            .unwrap();
        for client in [&mut alice, &mut bob] {
            client
                .pin_group_service(group, &profile, Zeroizing::new([40; 32]))
                .unwrap();
        }
        alice.prepare_group_service_request(group, None).unwrap();
        alice.submit_group_service_online(group, now).unwrap();
        let add = alice
            .prepare_group_change(
                group,
                Change::Add(
                    Member::new([41; 32], Role::Admin, &[bob.own_device_binding().unwrap()])
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
        let mut server = sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap();
        let invite = server
            .invite(
                sigil_protocol::accounts::InviteRequest {
                    username: "carol".into(),
                    expires_in_seconds: 60,
                },
                now,
            )
            .unwrap();
        let mut carol = open(&dir.path().join("carol.db"));
        prepare(&mut carol, &fixture, &invite.secret);
        carol.enroll_online().unwrap();
        carol.publish_device_binding_online().unwrap();
        carol
            .prepare_prekey_publication([42; 32], true, 3600)
            .unwrap();
        carol.publish_prekey_online([42; 32]).unwrap();
        server
            .allow_sender(
                &credential(&carol),
                &bob.connection_session().unwrap().unwrap().device_id,
                now,
            )
            .unwrap();
        let (_, c) = crate::incoming::tests::trust(&mut bob, &mut carol);
        let id = [84; 32];
        bob.prepare_group_invitation(group, c, id, Role::Member, now + 300, now)
            .unwrap();
        bob.advance_group_invitation_online(id, now).unwrap();
        let change = alice
            .prepare_group_change(
                group,
                Change::SetRole {
                    member: [41; 32],
                    role: Role::Member,
                },
            )
            .unwrap();
        alice
            .prepare_group_service_request(group, Some(&change))
            .unwrap();
        alice.submit_group_service_online(group, now).unwrap();
        if synced {
            bob.sync_group_service_online(group, now).unwrap();
        }
        let db = rusqlite::Connection::open(dir.path().join("server.db")).unwrap();
        assert_eq!(
            db.query_row("SELECT status FROM private_group_invitations", [], |r| r
                .get::<_, u8>(0))
                .unwrap(),
            2
        );
        drop(bob);
        let mut bob = open(&dir.path().join("bob.db"));
        let sent = bob.resume_outbound_online(now).unwrap();
        assert!(!sent.is_empty());
        assert!(
            sent.iter()
                .all(|a| matches!(a.result, Err(Error::Unprepared))),
            "{sent:?}"
        );
        assert!(carol.receive_mailbox_online(now).unwrap().is_empty());
        assert_eq!(
            carol
                .db
                .query_row("SELECT count(*) FROM group_invitations", [], |r| r
                    .get::<_, u32>(0))
                .unwrap(),
            0
        );
    }
}
