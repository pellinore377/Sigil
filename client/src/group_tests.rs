use super::*;
use sigil_protocol::device::Binding;

struct Device {
    key: IdentityKey,
    binding: Vec<u8>,
    fingerprint: Id,
}
fn id(n: u32) -> Id {
    let mut bytes = [0; 32];
    bytes[..4].copy_from_slice(&n.to_be_bytes());
    bytes
}
impl Device {
    fn new(account: u32, device: u32) -> Self {
        let key = IdentityKey::generate().unwrap();
        let binding = Binding {
            server: "synthetic.example".into(),
            username: format!("user{account}"),
            account: id(account),
            device: id(device),
            identity: key.public_key(),
        };
        let signature = key.sign(&binding.signing_bytes().unwrap()).unwrap();
        let binding = SignedBinding { binding, signature }.to_bytes().unwrap();
        let fingerprint = device_fingerprint(&binding).unwrap();
        Self {
            key,
            binding,
            fingerprint,
        }
    }
    fn member(&self, member: u32, role: Role) -> Member {
        Member::new(id(member), role, std::slice::from_ref(&self.binding)).unwrap()
    }
    fn sign(&self, proposal: &mut Proposal) {
        proposal.sign(self.fingerprint, &self.key).unwrap();
    }
}
fn start(creator: &Device, nonce: u32) -> State {
    Genesis::create(id(nonce), id(900), id(1), &creator.binding, &creator.key)
        .unwrap()
        .accept(creator.fingerprint)
        .unwrap()
}
fn add(state: &State, admin: &Device, device: &Device, member: u32, role: Role) -> State {
    let mut proposal = state
        .propose(admin.fingerprint, Change::Add(device.member(member, role)))
        .unwrap();
    admin.sign(&mut proposal);
    device.sign(&mut proposal);
    state.authorize(&proposal).unwrap()
}

#[test]
fn genesis_requires_pinned_creator_and_possession_and_has_stable_fingerprints() {
    let alice = Device::new(1, 1);
    let bob = Device::new(2, 2);
    assert!(Genesis::create(id(1), id(900), id(1), &alice.binding, &bob.key).is_err());
    let genesis = Genesis::create(id(1), id(900), id(1), &alice.binding, &alice.key).unwrap();
    assert!(genesis.accept(bob.fingerprint).is_err());
    let state = genesis.accept(alice.fingerprint).unwrap();
    assert_eq!(state.revision(), 0);
    assert_eq!(state.epoch(), 0);
    assert!(!state.earlier_history());
    assert_ne!(state.group(), start(&alice, 2).group());
    let mut signed = SignedBinding::from_bytes(&alice.binding).unwrap();
    signed.signature = alice
        .key
        .sign(&signed.binding.signing_bytes().unwrap())
        .unwrap();
    let equivalent = Genesis::create(
        id(1),
        id(900),
        id(1),
        &signed.to_bytes().unwrap(),
        &alice.key,
    )
    .unwrap();
    assert_eq!(
        state.head(),
        equivalent.accept(alice.fingerprint).unwrap().head()
    );
    signed.signature[0] ^= 1;
    assert!(Member::new(id(1), Role::Admin, &[signed.to_bytes().unwrap()]).is_err());
}

#[test]
fn additions_require_admin_and_every_invited_device_consent() {
    let alice = Device::new(1, 1);
    let bob = Device::new(2, 2);
    let bob_phone = Device::new(2, 3);
    let eve = Device::new(3, 4);
    let state = start(&alice, 1);
    let member = Member::new(
        id(2),
        Role::Member,
        &[bob.binding.clone(), bob_phone.binding.clone()],
    )
    .unwrap();
    let mut proposal = state
        .propose(alice.fingerprint, Change::Add(member))
        .unwrap();
    assert!(state.authorize(&proposal).is_err());
    bob.sign(&mut proposal);
    bob_phone.sign(&mut proposal);
    assert!(state.authorize(&proposal).is_err());
    assert!(proposal.sign(eve.fingerprint, &eve.key).is_err());
    assert!(proposal.sign(alice.fingerprint, &eve.key).is_err());
    alice.sign(&mut proposal);
    let joined = state.authorize(&proposal).unwrap();
    assert_eq!(joined.members().len(), 2);
    assert_eq!(joined.epoch(), 1);
    assert!(joined
        .propose(bob.fingerprint, Change::Add(eve.member(3, Role::Member)))
        .is_err());
    assert!(joined
        .propose(
            bob.fingerprint,
            Change::SetRole {
                member: id(2),
                role: Role::Admin
            }
        )
        .is_err());
    assert!(joined
        .propose(bob.fingerprint, Change::EarlierHistory(true))
        .is_err());
}

#[test]
fn adding_account_device_requires_existing_member_and_new_device_not_just_admin() {
    let alice = Device::new(1, 1);
    let bob = Device::new(2, 2);
    let bob_phone = Device::new(2, 3);
    let wrong_account = Device::new(3, 4);
    let state = add(&start(&alice, 1), &alice, &bob, 2, Role::Member);
    assert!(state
        .propose(
            alice.fingerprint,
            Change::AddDevice {
                member: id(2),
                binding: wrong_account.binding
            }
        )
        .is_err());
    let mut proposal = state
        .propose(
            alice.fingerprint,
            Change::AddDevice {
                member: id(2),
                binding: bob_phone.binding.clone(),
            },
        )
        .unwrap();
    alice.sign(&mut proposal);
    bob_phone.sign(&mut proposal);
    assert!(state.authorize(&proposal).is_err());
    bob.sign(&mut proposal);
    let next = state.authorize(&proposal).unwrap();
    assert_eq!(next.epoch(), state.epoch() + 1);
    assert_eq!(next.members()[1].device_fingerprints().unwrap().len(), 2);
    assert!(next
        .propose(
            alice.fingerprint,
            Change::AddDevice {
                member: id(2),
                binding: bob_phone.binding
            }
        )
        .is_err());
}

#[test]
fn signatures_and_concurrent_changes_bind_exact_group_and_predecessor() {
    let alice = Device::new(1, 1);
    let bob = Device::new(2, 2);
    let charlie = Device::new(3, 3);
    let state = add(&start(&alice, 1), &alice, &bob, 2, Role::Admin);
    let mut removal = state
        .propose(alice.fingerprint, Change::Remove(id(2)))
        .unwrap();
    alice.sign(&mut removal);
    let mut addition = state
        .propose(
            bob.fingerprint,
            Change::Add(charlie.member(3, Role::Member)),
        )
        .unwrap();
    bob.sign(&mut addition);
    charlie.sign(&mut addition);
    let removed = state.authorize(&removal).unwrap();
    assert!(removed.authorize(&addition).is_err());
    assert!(removed
        .propose(bob.fingerprint, Change::EarlierHistory(true))
        .is_err());
    let alternative = state.authorize(&addition).unwrap();
    assert!(alternative.authorize(&removal).is_err());
    assert_ne!(alternative.head(), removed.head());
    // Both are authorized proposals; only the ordering layer can commit one.
    let other_group = add(&start(&alice, 2), &alice, &bob, 2, Role::Admin);
    assert!(other_group.authorize(&removal).is_err());
    let mut altered = state
        .propose(alice.fingerprint, Change::EarlierHistory(true))
        .unwrap();
    altered.signatures = removal.signatures.clone();
    assert!(state.authorize(&altered).is_err());
    assert_eq!(state.members().len(), 2);
    assert_eq!(state.revision(), 1);
}

#[test]
fn leave_role_changes_removal_and_closure_preserve_authority_and_epoch_rules() {
    let alice = Device::new(1, 1);
    let bob = Device::new(2, 2);
    let state = add(&start(&alice, 1), &alice, &bob, 2, Role::Member);
    assert!(state
        .propose(alice.fingerprint, Change::Remove(id(1)))
        .is_err());
    assert!(state
        .propose(
            alice.fingerprint,
            Change::SetRole {
                member: id(1),
                role: Role::Member
            }
        )
        .is_err());
    assert!(state
        .propose(
            bob.fingerprint,
            Change::RemoveDevice {
                member: id(2),
                fingerprint: bob.fingerprint
            }
        )
        .is_err());
    assert!(state
        .propose(bob.fingerprint, Change::Remove(id(1)))
        .is_err());
    let mut leave = state
        .propose(bob.fingerprint, Change::Remove(id(2)))
        .unwrap();
    bob.sign(&mut leave);
    let left = state.authorize(&leave).unwrap();
    assert_eq!(left.epoch(), state.epoch() + 1);
    let mut history = left
        .propose(alice.fingerprint, Change::EarlierHistory(true))
        .unwrap();
    alice.sign(&mut history);
    let allowed = left.authorize(&history).unwrap();
    assert!(allowed.earlier_history());
    assert_eq!(allowed.epoch(), left.epoch() + 1);
    assert_eq!(allowed.revision(), left.revision() + 1);
    let mut close = allowed.propose(alice.fingerprint, Change::Close).unwrap();
    alice.sign(&mut close);
    let closed = allowed.authorize(&close).unwrap();
    assert!(closed.is_closed());
    assert!(closed
        .propose(alice.fingerprint, Change::EarlierHistory(false))
        .is_err());
    assert!(closed.authorize(&close).is_err());
}

#[test]
fn membership_is_canonical_bounded_and_rejects_account_device_aliases() {
    let alice = Device::new(1, 1);
    let bob = Device::new(2, 2);
    let bob_phone = Device::new(2, 3);
    let state = start(&alice, 1);
    let forward = Member::new(
        id(2),
        Role::Member,
        &[bob.binding.clone(), bob_phone.binding.clone()],
    )
    .unwrap();
    let reverse = Member::new(
        id(2),
        Role::Member,
        &[bob_phone.binding.clone(), bob.binding.clone()],
    )
    .unwrap();
    let first = state
        .propose(alice.fingerprint, Change::Add(forward))
        .unwrap();
    let second = state
        .propose(alice.fingerprint, Change::Add(reverse))
        .unwrap();
    assert_eq!(first.head(), second.head());
    assert!(Member::new(
        id(2),
        Role::Member,
        &[bob.binding.clone(), bob.binding.clone()]
    )
    .is_err());
    assert!(Member::new(id(2), Role::Member, &[]).is_err());
    assert!(state
        .propose(alice.fingerprint, Change::Add(bob.member(1, Role::Member)))
        .is_err());
    assert!(state
        .propose(
            alice.fingerprint,
            Change::Add(alice.member(2, Role::Member))
        )
        .is_err());
    // Reach the 256-member target with real signed bindings and approved joins.
    let mut full = state;
    for n in 2..=MAX_MEMBERS as u32 {
        let device = Device::new(n, n);
        full = add(&full, &alice, &device, n, Role::Member);
    }
    assert_eq!(full.members().len(), MAX_MEMBERS);
    let extra = Device::new(257, 257);
    assert!(matches!(
        full.propose(
            alice.fingerprint,
            Change::Add(extra.member(257, Role::Member))
        ),
        Err(Error::Limit)
    ));
    assert_eq!(full.members().len(), MAX_MEMBERS);
}

#[test]
fn wire_roundtrip_reconstructs_authority_and_preserves_incomplete_approval() {
    let alice = Device::new(1, 1);
    let bob = Device::new(2, 2);
    let genesis = Genesis::create(id(1), id(900), id(1), &alice.binding, &alice.key).unwrap();
    let bytes = genesis.to_bytes().unwrap();
    let remote = Genesis::from_bytes(&bytes, alice.fingerprint).unwrap();
    assert_eq!(remote.to_bytes().unwrap(), bytes);
    assert!(Genesis::from_bytes(&bytes, bob.fingerprint).is_err());
    let state = remote.accept(alice.fingerprint).unwrap();
    let mut proposal = state
        .propose(alice.fingerprint, Change::Add(bob.member(2, Role::Member)))
        .unwrap();
    alice.sign(&mut proposal);
    let unsigned = proposal.to_bytes().unwrap();
    let mut remote = state.proposal_from_bytes(&unsigned).unwrap();
    assert!(state.authorize(&remote).is_err());
    let approval = bob.key.sign(&remote.signing_bytes()).unwrap();
    remote.approve(bob.fingerprint, approval).unwrap();
    remote.approve(bob.fingerprint, approval).unwrap();
    let complete = remote.to_bytes().unwrap();
    let joined = state
        .authorize(&state.proposal_from_bytes(&complete).unwrap())
        .unwrap();
    assert_eq!(joined.revision(), 1);
    assert!(joined.proposal_from_bytes(&complete).is_err());
    assert_eq!(
        state.head(),
        genesis.accept(alice.fingerprint).unwrap().head()
    );
    // Another group's identically shaped invitation cannot reuse this approval.
    let other = start(&alice, 2);
    let mut transplanted = other
        .propose(alice.fingerprint, Change::Add(bob.member(2, Role::Member)))
        .unwrap();
    assert!(transplanted.approve(bob.fingerprint, approval).is_err());
}

#[test]
fn control_decoders_reject_truncation_trailing_data_changes_and_duplicate_signers() {
    let alice = Device::new(1, 1);
    let genesis = Genesis::create(id(1), id(900), id(1), &alice.binding, &alice.key).unwrap();
    let encoded = genesis.to_bytes().unwrap();
    for n in 0..encoded.len() {
        assert!(Genesis::from_bytes(&encoded[..n], alice.fingerprint).is_err());
    }
    for n in [0, 8, 40, 72, encoded.len() - 1] {
        let mut bad = encoded.clone();
        bad[n] ^= 1;
        assert!(Genesis::from_bytes(&bad, alice.fingerprint).is_err());
    }
    let mut bad = encoded.clone();
    bad.push(0);
    assert!(Genesis::from_bytes(&bad, alice.fingerprint).is_err());
    let state = genesis.accept(alice.fingerprint).unwrap();
    let mut proposal = state
        .propose(alice.fingerprint, Change::EarlierHistory(true))
        .unwrap();
    alice.sign(&mut proposal);
    let encoded = proposal.to_bytes().unwrap();
    for n in 0..encoded.len() {
        assert!(state.proposal_from_bytes(&encoded[..n]).is_err());
    }
    for n in [0, 8, 40, 72, 104, 136, 137, encoded.len() - 1] {
        let mut bad = encoded.clone();
        bad[n] ^= 128;
        assert!(state.proposal_from_bytes(&bad).is_err());
    }
    let mut duplicate = encoded.clone();
    duplicate[139] = 2;
    duplicate.extend_from_slice(&encoded[140..]);
    assert!(state.proposal_from_bytes(&duplicate).is_err());
    let mut excessive = encoded.clone();
    excessive[138..140].copy_from_slice(&u16::MAX.to_be_bytes());
    assert!(matches!(
        state.proposal_from_bytes(&excessive),
        Err(Error::Limit)
    ));
    assert!(matches!(
        state.proposal_from_bytes(&vec![0; MAX_PROPOSAL_BYTES + 1]),
        Err(Error::Limit)
    ));
    let mut trailing = encoded;
    trailing.push(0);
    assert!(state.proposal_from_bytes(&trailing).is_err());
}
