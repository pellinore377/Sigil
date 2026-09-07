//! Experimental group membership, local ordering and same-server message delivery.
//! The private authority adapter is experimental; group-scoped channels remain open.
use crate::{device_fingerprint, peers, Error, Id};
use sha2::{Digest, Sha256};
use sigil_crypto::{verify_signature, IdentityKey};
use sigil_protocol::device::SignedBinding;
use std::collections::BTreeSet;

pub const MAX_MEMBERS: usize = 256;
pub const MAX_DEVICES: usize = 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Member,
    Admin,
}

#[derive(Clone)]
pub struct Member {
    id: Id,
    role: Role,
    devices: Vec<SignedBinding>,
}
impl Member {
    /// Every binding is signature-checked. This proves device-key possession,
    /// not independent contact verification or authority over a server account.
    pub fn new(id: Id, role: Role, bindings: &[Vec<u8>]) -> Result<Self, Error> {
        if bindings.is_empty() || bindings.len() > 256 {
            return Err(Error::Limit);
        }
        let mut devices = bindings
            .iter()
            .map(|bytes| peers::parse(bytes))
            .collect::<Result<Vec<_>, _>>()?;
        devices.sort_by_key(|v| v.binding.device);
        let first = &devices[0].binding;
        if devices.iter().any(|v| {
            v.binding.server != first.server
                || v.binding.account != first.account
                || v.binding.username != first.username
        }) || devices
            .windows(2)
            .any(|v| v[0].binding.device == v[1].binding.device)
        {
            return Err(Error::Conflict);
        }
        Ok(Self { id, role, devices })
    }
    pub fn id(&self) -> Id {
        self.id
    }
    pub fn role(&self) -> Role {
        self.role
    }
    pub fn device_fingerprints(&self) -> Result<Vec<Id>, Error> {
        self.devices
            .iter()
            .map(|v| peers::fingerprint(&v.binding))
            .collect()
    }
}

#[derive(Clone)]
pub enum Change {
    Add(Member),
    Remove(Id),
    SetRole { member: Id, role: Role },
    AddDevice { member: Id, binding: Vec<u8> },
    RemoveDevice { member: Id, fingerprint: Id },
    EarlierHistory(bool),
    RefreshKeys,
    Close,
}

// Plaintext membership deliberately does not implement Debug.
#[derive(Clone)]
pub struct State {
    group: Id,
    head: Id,
    revision: u64,
    epoch: u64,
    authority: Id,
    members: Vec<Member>,
    earlier_history: bool,
    closed: bool,
}

pub struct Genesis {
    nonce: Id,
    state: State,
    creator: Id,
    signature: [u8; 64],
}

pub struct Proposal {
    predecessor: Id,
    author: Id,
    change: Change,
    next: State,
    // Every set must contain at least one valid signer. This expresses both
    // mandatory new-device consent and an existing member's device approval.
    approvals: Vec<Vec<(Id, Id)>>,
    signatures: Vec<(Id, [u8; 64])>,
}

fn digest(domain: &[u8], parts: &[&[u8]]) -> Id {
    let mut hash = Sha256::new();
    hash.update(domain);
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    hash.finalize().into()
}

fn signers(member: &Member) -> Result<Vec<(Id, Id)>, Error> {
    member
        .devices
        .iter()
        .map(|v| Ok((peers::fingerprint(&v.binding)?, v.binding.identity)))
        .collect()
}

impl State {
    fn canonical(&self) -> Result<Vec<u8>, Error> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&self.group);
        bytes.extend_from_slice(&self.authority);
        bytes.extend_from_slice(&self.revision.to_be_bytes());
        bytes.extend_from_slice(&self.epoch.to_be_bytes());
        bytes.extend_from_slice(&[self.earlier_history as u8, self.closed as u8]);
        bytes.extend_from_slice(&(self.members.len() as u16).to_be_bytes());
        for member in &self.members {
            bytes.extend_from_slice(&member.id);
            bytes.push(u8::from(member.role == Role::Admin));
            bytes.extend_from_slice(&(member.devices.len() as u16).to_be_bytes());
            for device in &member.devices {
                // Randomized binding signatures do not change a state's ID.
                let binding = device
                    .binding
                    .signing_bytes()
                    .map_err(|_| Error::InvalidEvent)?;
                bytes.extend_from_slice(&(binding.len() as u16).to_be_bytes());
                bytes.extend_from_slice(&binding);
            }
        }
        Ok(bytes)
    }
    fn validate(&mut self) -> Result<(), Error> {
        if self.members.is_empty()
            || self.members.len() > MAX_MEMBERS
            || self.members.iter().map(|m| m.devices.len()).sum::<usize>() > MAX_DEVICES
        {
            return Err(Error::Limit);
        }
        self.members.sort_by_key(|m| m.id);
        let mut accounts = BTreeSet::new();
        let mut devices = BTreeSet::new();
        let mut identities = BTreeSet::new();
        if self.members.windows(2).any(|m| m[0].id == m[1].id)
            || !self.members.iter().any(|m| m.role == Role::Admin)
        {
            return Err(Error::Conflict);
        }
        for member in &self.members {
            let first = member.devices.first().ok_or(Error::Conflict)?;
            if !accounts.insert((&first.binding.server, first.binding.account)) {
                return Err(Error::Conflict);
            }
            for device in &member.devices {
                if !devices.insert((&device.binding.server, device.binding.device))
                    || !identities.insert(device.binding.identity)
                {
                    return Err(Error::Conflict);
                }
            }
        }
        Ok(())
    }
    pub fn group(&self) -> Id {
        self.group
    }
    pub fn head(&self) -> Id {
        self.head
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn members(&self) -> &[Member] {
        &self.members
    }
    pub fn earlier_history(&self) -> bool {
        self.earlier_history
    }
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    fn device(&self, fingerprint: Id) -> Result<(&Member, Id), Error> {
        for member in &self.members {
            for (id, identity) in signers(member)? {
                if id == fingerprint {
                    return Ok((member, identity));
                }
            }
        }
        Err(Error::Unprepared)
    }

    /// Produces an immutable approval request against this exact predecessor.
    /// No state change occurs until every required identity has signed.
    pub fn propose(&self, author: Id, change: Change) -> Result<Proposal, Error> {
        if self.closed {
            return Err(Error::Obsolete);
        }
        let (actor, identity) = self.device(author)?;
        let leaving = matches!(&change, Change::Remove(id) if *id == actor.id);
        let own_device = matches!(&change, Change::AddDevice { member, .. } | Change::RemoveDevice { member, .. } if *member == actor.id);
        if actor.role != Role::Admin && !leaving && !own_device {
            return Err(Error::Unprepared);
        }
        let mut next = self.clone();
        next.revision = next.revision.checked_add(1).ok_or(Error::Limit)?;
        let mut approvals = vec![vec![(author, identity)]];
        let frozen_change = change.clone();
        match change {
            Change::Add(member) => {
                // All invited devices consent to this specific group/state.
                for signer in signers(&member)? {
                    approvals.push(vec![signer]);
                }
                next.members.push(member);
            }
            Change::Remove(id) => {
                let i = next
                    .members
                    .iter()
                    .position(|m| m.id == id)
                    .ok_or(Error::NotFound)?;
                next.members.remove(i);
            }
            Change::SetRole { member, role } => {
                let target = next
                    .members
                    .iter_mut()
                    .find(|m| m.id == member)
                    .ok_or(Error::NotFound)?;
                if target.role == role {
                    return Err(Error::Conflict);
                }
                target.role = role;
            }
            Change::AddDevice { member, binding } => {
                let target = next
                    .members
                    .iter_mut()
                    .find(|m| m.id == member)
                    .ok_or(Error::NotFound)?;
                approvals.push(signers(target)?);
                let new_device = peers::parse(&binding)?;
                approvals.push(vec![(
                    device_fingerprint(&binding)?,
                    new_device.binding.identity,
                )]);
                let mut bindings = target
                    .devices
                    .iter()
                    .map(|v| v.to_bytes().map_err(|_| Error::InvalidEvent))
                    .collect::<Result<Vec<_>, _>>()?;
                bindings.push(binding);
                *target = Member::new(target.id, target.role, &bindings)?;
            }
            Change::RemoveDevice {
                member,
                fingerprint,
            } => {
                let target = next
                    .members
                    .iter_mut()
                    .find(|m| m.id == member)
                    .ok_or(Error::NotFound)?;
                let i = target
                    .device_fingerprints()?
                    .iter()
                    .position(|f| *f == fingerprint)
                    .ok_or(Error::NotFound)?;
                target.devices.remove(i);
            }
            Change::EarlierHistory(allowed) => {
                if next.earlier_history == allowed {
                    return Err(Error::Conflict);
                }
                next.earlier_history = allowed;
            }
            Change::Close => next.closed = true,
            Change::RefreshKeys => {}
        }
        // Every committed state has one unambiguous sender-key epoch, including
        // policy changes; distributions always bind the exact state digest.
        next.epoch = next.revision;
        next.validate()?;
        next.head = digest(
            b"Sigil/group-transition/v0",
            &[&self.head, &author, &next.canonical()?],
        );
        Ok(Proposal {
            predecessor: self.head,
            author,
            change: frozen_change,
            next,
            approvals,
            signatures: Vec::new(),
        })
    }

    /// Validates authorization only. A caller still needs authenticated ordering
    /// and a durable atomic commit before this state can enable group traffic.
    pub fn authorize(&self, proposal: &Proposal) -> Result<Self, Error> {
        if self.closed {
            return Err(Error::Obsolete);
        }
        if proposal.predecessor != self.head
            || proposal.next.group != self.group
            || proposal.next.revision != self.revision.checked_add(1).ok_or(Error::Limit)?
        {
            return Err(Error::Conflict);
        }
        // Proposal internals are private and can only be constructed by propose.
        // External deserialization must reconstruct through that method.
        let message = proposal.signing_bytes();
        for required in &proposal.approvals {
            let mut satisfied = false;
            for (fingerprint, identity) in required {
                if let Some((_, signature)) =
                    proposal.signatures.iter().find(|(id, _)| id == fingerprint)
                {
                    verify_signature(identity, &message, signature)?;
                    satisfied = true;
                }
            }
            if !satisfied {
                return Err(Error::Unprepared);
            }
        }
        Ok(proposal.next.clone())
    }
}

impl Proposal {
    pub fn head(&self) -> Id {
        self.next.head
    }
    pub fn predecessor(&self) -> Id {
        self.predecessor
    }
    pub fn proposed_state(&self) -> &State {
        &self.next
    }
    pub fn signing_bytes(&self) -> Vec<u8> {
        [
            b"Sigil/group-approval/v0".as_slice(),
            &self.next.group,
            &self.predecessor,
            &self.author,
            &self.next.head,
        ]
        .concat()
    }
    pub fn sign(&mut self, fingerprint: Id, key: &IdentityKey) -> Result<(), Error> {
        if !self
            .approvals
            .iter()
            .flatten()
            .any(|(id, identity)| *id == fingerprint && *identity == key.public_key())
        {
            return Err(Error::Unprepared);
        }
        if !self.signatures.iter().any(|(id, _)| *id == fingerprint) {
            self.signatures
                .push((fingerprint, key.sign(&self.signing_bytes())?));
        }
        Ok(())
    }
}

impl Genesis {
    /// Caller generates a fresh random nonce and pins an authority key digest.
    pub fn create(
        nonce: Id,
        authority: Id,
        member: Id,
        binding: &[u8],
        key: &IdentityKey,
    ) -> Result<Self, Error> {
        let (state, fingerprint) = Self::state(nonce, authority, member, binding)?;
        if state.members[0].devices[0].binding.identity != key.public_key() {
            return Err(Error::Unprepared);
        }
        let signature =
            key.sign(&[b"Sigil/group-genesis-approval/v0".as_slice(), &state.head].concat())?;
        Ok(Self {
            nonce,
            state,
            creator: fingerprint,
            signature,
        })
    }
    fn state(nonce: Id, authority: Id, member: Id, binding: &[u8]) -> Result<(State, Id), Error> {
        let creator = Member::new(member, Role::Admin, &[binding.to_vec()])?;
        let fingerprint = device_fingerprint(binding)?;
        let group = digest(
            b"Sigil/group-genesis-id/v0",
            &[&nonce, &authority, &member, &fingerprint],
        );
        let mut state = State {
            group,
            head: [0; 32],
            revision: 0,
            epoch: 0,
            authority,
            members: vec![creator],
            earlier_history: false,
            closed: false,
        };
        state.validate()?;
        state.head = digest(b"Sigil/group-genesis-state/v0", &[&state.canonical()?]);
        Ok((state, fingerprint))
    }
    /// Expected creator must come from independent verification/invitation trust,
    /// never merely from the untrusted genesis being checked.
    pub fn accept(&self, expected_creator: Id) -> Result<State, Error> {
        if self.creator != expected_creator {
            return Err(Error::Unprepared);
        }
        let (_, identity) = self.state.device(expected_creator)?;
        verify_signature(
            &identity,
            &[
                b"Sigil/group-genesis-approval/v0".as_slice(),
                &self.state.head,
            ]
            .concat(),
            &self.signature,
        )?;
        Ok(self.state.clone())
    }
}

#[path = "group_codec.rs"]
mod codec;
pub use codec::MAX_PROPOSAL_BYTES;
pub use sigil_crypto::group_receipt::{authority_fingerprint, Receipt};
#[path = "group_control.rs"]
mod control;
#[path = "group_storage_record.rs"]
mod storage_record;
pub use control::DistributionReceipt;
pub(crate) use control::{distribution_receipt, is_wire_control, retained_payload};
#[path = "group_keys.rs"]
mod keys;
pub(crate) use keys::MIGRATION as KEY_MIGRATION;
pub(crate) use keys::{install_distribution, validate_distribution_receipt};
#[path = "group_messages.rs"]
mod messages;
pub(crate) use messages::acknowledge as acknowledge_group;
pub(crate) use messages::migrate_structured;
pub(crate) use messages::MIGRATION as MESSAGE_MIGRATION;
pub use messages::{
    group_event_history_id, group_history_id, GroupDeliveryStatus, GroupMessage, GroupSendAttempt,
};
#[path = "group_service.rs"]
mod service;
#[path = "group_store.rs"]
mod store;
pub(crate) use service::MIGRATION as SERVICE_MIGRATION;
pub(crate) use store::MIGRATION;
pub use store::{CommitResult, GroupStatus};

#[cfg(test)]
#[path = "group_tests.rs"]
mod tests;
