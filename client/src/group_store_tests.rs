use super::*;
use sigil_crypto::Secret32;
use std::path::Path;

fn open(path: &Path) -> ClientStore {
    ClientStore::open(
        path,
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap()
}
fn receipt(state: &State, bytes: &[u8], authority: &IdentityKey) -> Vec<u8> {
    let proposal = state.proposal_from_bytes(bytes).unwrap();
    Receipt::sign(
        state.group(),
        state.head(),
        proposal.head(),
        proposal.proposed_state().revision(),
        authority,
    )
    .unwrap()
    .to_bytes()
}
pub(crate) fn staged(
    alice: &mut ClientStore,
    bob: &mut ClientStore,
    authority: &IdentityKey,
) -> Id {
    let group = alice
        .create_group(authority_fingerprint(&authority.public_key()))
        .unwrap();
    let genesis = alice.group_genesis(group).unwrap();
    let (a, _) = crate::incoming::tests::trust(alice, bob);
    assert_eq!(bob.accept_group_genesis(a, &genesis).unwrap(), group);
    group
}
pub(crate) fn join(
    alice: &mut ClientStore,
    bob: &mut ClientStore,
    group: Id,
    role: Role,
    authority: &IdentityKey,
) -> (Vec<u8>, Vec<u8>) {
    let member = Member::new([2; 32], role, &[bob.own_device_binding().unwrap()]).unwrap();
    let bytes = alice
        .prepare_group_change(group, Change::Add(member))
        .unwrap();
    let bytes = bob.approve_group_proposal(group, &bytes).unwrap();
    let bytes = alice.approve_group_proposal(group, &bytes).unwrap();
    let receipt = receipt(&alice.group_status(group).unwrap().state, &bytes, authority);
    assert_eq!(
        alice
            .commit_group_proposal(group, &bytes, &receipt)
            .unwrap(),
        CommitResult::Applied
    );
    assert_eq!(
        bob.commit_group_proposal(group, &bytes, &receipt).unwrap(),
        CommitResult::Applied
    );
    (bytes, receipt)
}
fn count(store: &ClientStore, table: &str) -> i64 {
    store
        .db
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

#[test]
fn durable_join_requires_consent_and_ordering_and_rolls_back_partial_commit() {
    let (dir, _fixture, mut alice, mut bob, _) = crate::claims::tests::pair();
    let authority = IdentityKey::generate().unwrap();
    let group = staged(&mut alice, &mut bob, &authority);
    let member = Member::new([2; 32], Role::Member, &[bob.own_device_binding().unwrap()]).unwrap();
    let proposed = alice
        .prepare_group_change(group, Change::Add(member))
        .unwrap();
    let before = alice.group_status(group).unwrap().state;
    let head = before.proposal_from_bytes(&proposed).unwrap().head();
    // Ordering alone cannot supply Bob's absent approval.
    let ordered = receipt(&before, &proposed, &authority);
    assert!(alice
        .commit_group_proposal(group, &proposed, &ordered)
        .is_err());
    assert_eq!(count(&alice, "group_commits"), 0);
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    assert_eq!(alice.pending_group_proposals(group).unwrap(), vec![head]);
    assert_eq!(alice.pending_group_proposal(group, head).unwrap(), proposed);
    let approved = bob.approve_group_proposal(group, &proposed).unwrap();
    let approved = alice.approve_group_proposal(group, &approved).unwrap();
    // Repeating the initial incomplete request cannot erase durable approvals.
    assert_eq!(
        alice.approve_group_proposal(group, &proposed).unwrap(),
        approved
    );
    let foreign = IdentityKey::generate().unwrap();
    let wrong = receipt(&before, &approved, &foreign);
    assert!(alice
        .commit_group_proposal(group, &approved, &wrong)
        .is_err());
    assert!(!alice.group_status(group).unwrap().frozen);
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON groups BEGIN SELECT RAISE(ABORT,'synthetic group commit failure'); END;").unwrap();
    assert!(alice
        .commit_group_proposal(group, &approved, &ordered)
        .is_err());
    assert_eq!(count(&alice, "group_commits"), 0);
    assert_eq!(
        alice.group_status(group).unwrap().state.head(),
        before.head()
    );
    assert_eq!(alice.pending_group_proposal(group, head).unwrap(), approved);
    alice.db.execute_batch("DROP TRIGGER fail;").unwrap();
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    assert_eq!(
        alice
            .commit_group_proposal(group, &approved, &ordered)
            .unwrap(),
        CommitResult::Applied
    );
    assert_eq!(
        alice
            .commit_group_proposal(group, &approved, &ordered)
            .unwrap(),
        CommitResult::Duplicate
    );
    assert_eq!(count(&alice, "group_commits"), 1);
    assert_eq!(count(&alice, "group_proposals"), 0);
    assert_eq!(
        bob.commit_group_proposal(group, &approved, &ordered)
            .unwrap(),
        CommitResult::Applied
    );
    assert_eq!(
        bob.group_status(group).unwrap().state.head(),
        alice.group_status(group).unwrap().state.head()
    );
    assert_eq!(alice.group_status(group).unwrap().state.epoch(), 1);
    let genesis = alice.group_genesis(group).unwrap();
    let creator = bob
        .observe_peer_binding(&alice.own_device_binding().unwrap())
        .unwrap();
    bob.accept_group_genesis(creator.id, &genesis).unwrap();
    assert_eq!(bob.group_status(group).unwrap().state.revision(), 1);
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    assert_eq!(
        bob.commit_group_proposal(group, &approved, &ordered)
            .unwrap(),
        CommitResult::Duplicate
    );
}

#[test]
fn authenticated_equivocation_freezes_across_restart_without_merging_members() {
    let (dir, _fixture, mut alice, mut bob, _) = crate::claims::tests::pair();
    let authority = IdentityKey::generate().unwrap();
    let group = staged(&mut alice, &mut bob, &authority);
    join(&mut alice, &mut bob, group, Role::Admin, &authority);
    let before = alice.group_status(group).unwrap().state;
    let alice_id = before
        .members()
        .iter()
        .find(|m| m.id() != [2; 32])
        .unwrap()
        .id();
    let a = alice
        .prepare_group_change(group, Change::Remove([2; 32]))
        .unwrap();
    let b = bob
        .prepare_group_change(group, Change::Remove(alice_id))
        .unwrap();
    let a_receipt = receipt(&before, &a, &authority);
    let b_receipt = receipt(&before, &b, &authority);
    assert_eq!(
        alice.commit_group_proposal(group, &a, &a_receipt).unwrap(),
        CommitResult::Applied
    );
    let accepted = alice.group_status(group).unwrap().state.head();
    let mut forged = b_receipt.clone();
    forged[207] ^= 1;
    assert!(alice.commit_group_proposal(group, &b, &forged).is_err());
    assert!(!alice.group_status(group).unwrap().frozen);
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON groups BEGIN SELECT RAISE(ABORT,'synthetic freeze failure'); END;").unwrap();
    assert!(alice.commit_group_proposal(group, &b, &b_receipt).is_err());
    assert_eq!(count(&alice, "group_forks"), 0);
    assert!(!alice.group_status(group).unwrap().frozen);
    alice.db.execute_batch("DROP TRIGGER fail;").unwrap();
    assert_eq!(
        alice.commit_group_proposal(group, &b, &b_receipt).unwrap(),
        CommitResult::Frozen
    );
    assert_eq!(alice.group_status(group).unwrap().state.head(), accepted);
    assert_eq!(alice.group_status(group).unwrap().state.members().len(), 1);
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    assert!(alice.group_status(group).unwrap().frozen);
    assert!(alice
        .prepare_group_change(group, Change::EarlierHistory(true))
        .is_err());
    assert!(alice.approve_group_proposal(group, &b).is_err());
    assert_eq!(
        alice.commit_group_proposal(group, &a, &a_receipt).unwrap(),
        CommitResult::Frozen
    );
    // Losing the evidence record cannot turn a persisted freeze back into trust.
    alice
        .db
        .execute(
            "DELETE FROM group_forks WHERE group_id=?1",
            [group.as_slice()],
        )
        .unwrap();
    assert!(alice.group_status(group).is_err());
}

#[test]
fn historical_receipts_and_contradictory_revision_are_still_equivocation() {
    let (dir, _fixture, mut alice, mut bob, _) = crate::claims::tests::pair();
    let authority = IdentityKey::generate().unwrap();
    let group = staged(&mut alice, &mut bob, &authority);
    let (joined, first) = join(&mut alice, &mut bob, group, Role::Member, &authority);
    let history = alice
        .prepare_group_change(group, Change::EarlierHistory(true))
        .unwrap();
    let ordered = receipt(
        &alice.group_status(group).unwrap().state,
        &history,
        &authority,
    );
    alice
        .commit_group_proposal(group, &history, &ordered)
        .unwrap();
    let current = alice.group_status(group).unwrap().state.head();
    let old = Receipt::from_bytes(&first, authority_fingerprint(&authority.public_key())).unwrap();
    let inconsistent = Receipt::sign(
        group,
        old.predecessor,
        old.head,
        old.revision + 1,
        &authority,
    )
    .unwrap()
    .to_bytes();
    assert_eq!(
        alice
            .commit_group_proposal(group, &joined, &inconsistent)
            .unwrap(),
        CommitResult::Frozen
    );
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    assert!(alice.group_status(group).unwrap().frozen);
    assert_eq!(alice.group_status(group).unwrap().state.head(), current);
}

#[test]
fn genesis_does_not_bypass_contact_trust_or_join_consent_and_checkpoints_are_bound() {
    let (_dir, _fixture, mut alice, mut bob, _) = crate::claims::tests::pair();
    let authority = IdentityKey::generate().unwrap();
    let group = alice
        .create_group(authority_fingerprint(&authority.public_key()))
        .unwrap();
    let genesis = alice.group_genesis(group).unwrap();
    let creator = bob
        .observe_peer_binding(&alice.own_device_binding().unwrap())
        .unwrap();
    assert!(bob.accept_group_genesis(creator.id, &genesis).is_err());
    bob.confirm_peer(creator.id, creator.fingerprint).unwrap();
    bob.accept_group_genesis(creator.id, &genesis).unwrap();
    assert!(bob
        .prepare_group_change(group, Change::EarlierHistory(true))
        .is_err());
    let another = alice
        .create_group(authority_fingerprint(&authority.public_key()))
        .unwrap();
    let sealed: Vec<u8> = alice
        .db
        .query_row(
            "SELECT state FROM groups WHERE id=?1",
            [group.as_slice()],
            |r| r.get(0),
        )
        .unwrap();
    alice
        .db
        .execute(
            "UPDATE groups SET state=?1 WHERE id=?2",
            (sealed, another.as_slice()),
        )
        .unwrap();
    assert!(alice.group_status(another).is_err());
    assert!(!alice.group_status(group).unwrap().frozen);
}

#[test]
fn schema_42_migration_preserves_existing_identity_and_creates_empty_group_journal() {
    let (dir, _fixture, alice, _bob, _) = crate::claims::tests::pair();
    let expected = alice.connection_session().unwrap().unwrap();
    crate::test_schema::rewind(&alice.db, 42);
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    assert_eq!(
        alice.connection_session().unwrap().unwrap().device_id,
        expected.device_id
    );
    assert_eq!(count(&alice, "groups"), 0);
    let authority = IdentityKey::generate().unwrap();
    let group = alice
        .create_group(authority_fingerprint(&authority.public_key()))
        .unwrap();
    assert_eq!(alice.group_status(group).unwrap().state.revision(), 0);
    assert_eq!(
        alice
            .db
            .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        68
    );
}

#[test]
fn large_device_rosters_and_approvals_survive_storage_and_restart() {
    let (dir, _fixture, mut alice, _bob, _) = crate::claims::tests::pair();
    let authority = IdentityKey::generate().unwrap();
    let group = alice
        .create_group(authority_fingerprint(&authority.public_key()))
        .unwrap();
    let mut keys = Vec::new();
    let mut bindings = Vec::new();
    for n in 0u32..256 {
        let key = IdentityKey::generate().unwrap();
        let mut device = [0; 32];
        device[..4].copy_from_slice(&n.to_be_bytes());
        let binding = sigil_protocol::device::Binding {
            server: format!("{}.example", "s".repeat(63)),
            username: "groupfixture".into(),
            account: [88; 32],
            device,
            identity: key.public_key(),
        };
        let signature = key.sign(&binding.signing_bytes().unwrap()).unwrap();
        let signed = SignedBinding { binding, signature }.to_bytes().unwrap();
        keys.push((key, device_fingerprint(&signed).unwrap()));
        bindings.push(signed);
    }
    let member = Member::new([88; 32], Role::Member, &bindings).unwrap();
    let unsigned = alice
        .prepare_group_change(group, Change::Add(member))
        .unwrap();
    let state = alice.group_status(group).unwrap().state;
    let mut proposal = state.proposal_from_bytes(&unsigned).unwrap();
    for (key, fingerprint) in keys {
        proposal.sign(fingerprint, &key).unwrap();
    }
    let signed = proposal.to_bytes().unwrap();
    assert!(signed.len() > sigil_crypto::storage::MAX_RECORD);
    let signed = alice.approve_group_proposal(group, &signed).unwrap();
    assert!(
        proposal.proposed_state().checkpoint().unwrap().len() > sigil_crypto::storage::MAX_RECORD
    );
    let ordered = receipt(&state, &signed, &authority);
    assert_eq!(
        alice
            .commit_group_proposal(group, &signed, &ordered)
            .unwrap(),
        CommitResult::Applied
    );
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    let current = alice.group_status(group).unwrap();
    assert_eq!(current.state.head(), proposal.head());
    assert_eq!(
        current
            .state
            .members()
            .iter()
            .map(|m| m.device_fingerprints().unwrap().len())
            .sum::<usize>(),
        257
    );
    assert_eq!(
        alice
            .commit_group_proposal(group, &signed, &ordered)
            .unwrap(),
        CommitResult::Duplicate
    );
}

#[test]
fn pending_capacity_is_reusable_without_claiming_remote_approval_revocation() {
    let (_dir, _fixture, mut alice, mut bob, _) = crate::claims::tests::pair();
    let authority = IdentityKey::generate().unwrap();
    let group = staged(&mut alice, &mut bob, &authority);
    let binding = bob.own_device_binding().unwrap();
    let mut retained = Vec::new();
    for n in 0u8..16 {
        let member = Member::new([n; 32], Role::Member, std::slice::from_ref(&binding)).unwrap();
        retained.push(
            alice
                .prepare_group_change(group, Change::Add(member))
                .unwrap(),
        );
    }
    assert_eq!(alice.pending_group_proposals(group).unwrap().len(), 16);
    assert!(matches!(
        alice.prepare_group_change(group, Change::EarlierHistory(true)),
        Err(Error::Limit)
    ));
    let state = alice.group_status(group).unwrap().state;
    let head = state.proposal_from_bytes(&retained[0]).unwrap().head();
    assert!(alice.discard_local_group_proposal(group, head).unwrap());
    assert!(!alice.discard_local_group_proposal(group, head).unwrap());
    alice
        .prepare_group_change(group, Change::EarlierHistory(true))
        .unwrap();
    assert_eq!(alice.pending_group_proposals(group).unwrap().len(), 16);
    // A previously shared signature wasn't cryptographically revoked by local
    // cleanup. Bob can still consent and the authority can commit that proposal.
    let completed = bob.approve_group_proposal(group, &retained[0]).unwrap();
    let ordered = receipt(&state, &completed, &authority);
    assert_eq!(
        alice
            .commit_group_proposal(group, &completed, &ordered)
            .unwrap(),
        CommitResult::Applied
    );
    assert!(alice.pending_group_proposals(group).unwrap().is_empty());
    assert_eq!(alice.group_status(group).unwrap().state.revision(), 1);
}
