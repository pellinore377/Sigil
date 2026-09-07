//! Bounded experimental group control encoding, carried only inside encryption.
//! Parsing a proposal rebuilds predecessor authorization; serialized approval
//! requirements or a claimed next roster are never trusted.
use super::*;
const GENESIS: &[u8; 8] = b"SGGG\0\x01\0\0";
const PROPOSAL: &[u8; 8] = b"SGGP\0\x01\0\0";
pub const MAX_PROPOSAL_BYTES: usize = 160 * 1024;
pub(super) const MAX_CHECKPOINT_BYTES: usize = 560 * 1024;

struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, size: usize) -> Result<&'a [u8], Error> {
        let (value, rest) = self.0.split_at_checked(size).ok_or(Error::InvalidEvent)?;
        self.0 = rest;
        Ok(value)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        self.take(N)?.try_into().map_err(|_| Error::InvalidEvent)
    }
    fn byte(&mut self) -> Result<u8, Error> {
        Ok(self.array::<1>()?[0])
    }
    fn count(&mut self, max: usize) -> Result<usize, Error> {
        let count = u16::from_be_bytes(self.array()?) as usize;
        if count > max {
            return Err(Error::Limit);
        }
        Ok(count)
    }
    fn binding(&mut self) -> Result<Vec<u8>, Error> {
        let size = self.count(sigil_protocol::device::MAX_BYTES)?;
        Ok(self.take(size)?.to_vec())
    }
    fn role(&mut self) -> Result<Role, Error> {
        match self.byte()? {
            0 => Ok(Role::Member),
            1 => Ok(Role::Admin),
            _ => Err(Error::InvalidEvent),
        }
    }
    fn change(&mut self) -> Result<Change, Error> {
        Ok(match self.byte()? {
            0 => {
                let id = self.array()?;
                let role = self.role()?;
                let count = self.count(256)?;
                let bindings = (0..count)
                    .map(|_| self.binding())
                    .collect::<Result<Vec<_>, _>>()?;
                Change::Add(Member::new(id, role, &bindings)?)
            }
            1 => Change::Remove(self.array()?),
            2 => Change::SetRole {
                member: self.array()?,
                role: self.role()?,
            },
            3 => Change::AddDevice {
                member: self.array()?,
                binding: self.binding()?,
            },
            4 => Change::RemoveDevice {
                member: self.array()?,
                fingerprint: self.array()?,
            },
            5 => Change::EarlierHistory(match self.byte()? {
                0 => false,
                1 => true,
                _ => return Err(Error::InvalidEvent),
            }),
            6 => Change::Close,
            7 => Change::RefreshKeys,
            _ => return Err(Error::InvalidEvent),
        })
    }
}
fn binding(bytes: &mut Vec<u8>, value: &[u8]) -> Result<(), Error> {
    if value.len() > sigil_protocol::device::MAX_BYTES {
        return Err(Error::Limit);
    }
    bytes.extend_from_slice(&(value.len() as u16).to_be_bytes());
    bytes.extend_from_slice(value);
    Ok(())
}
fn change(bytes: &mut Vec<u8>, value: &Change) -> Result<(), Error> {
    match value {
        Change::Add(member) => {
            bytes.push(0);
            bytes.extend_from_slice(&member.id);
            bytes.push(u8::from(member.role == Role::Admin));
            bytes.extend_from_slice(&(member.devices.len() as u16).to_be_bytes());
            for device in &member.devices {
                binding(bytes, &device.to_bytes().map_err(|_| Error::InvalidEvent)?)?;
            }
        }
        Change::Remove(id) => {
            bytes.push(1);
            bytes.extend_from_slice(id);
        }
        Change::SetRole { member, role } => {
            bytes.push(2);
            bytes.extend_from_slice(member);
            bytes.push(u8::from(*role == Role::Admin));
        }
        Change::AddDevice {
            member,
            binding: device,
        } => {
            bytes.push(3);
            bytes.extend_from_slice(member);
            binding(bytes, device)?;
        }
        Change::RemoveDevice {
            member,
            fingerprint,
        } => {
            bytes.push(4);
            bytes.extend_from_slice(member);
            bytes.extend_from_slice(fingerprint);
        }
        Change::EarlierHistory(allowed) => {
            bytes.extend_from_slice(&[5, *allowed as u8]);
        }
        Change::Close => bytes.push(6),
        Change::RefreshKeys => bytes.push(7),
    }
    Ok(())
}
impl Proposal {
    /// Includes only the change and signatures, never a trusted next-state blob.
    pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let mut bytes = PROPOSAL.to_vec();
        for id in [
            &self.next.group,
            &self.predecessor,
            &self.author,
            &self.next.head,
        ] {
            bytes.extend_from_slice(id);
        }
        change(&mut bytes, &self.change)?;
        let mut signatures = self.signatures.clone();
        signatures.sort_by_key(|s| s.0);
        bytes.extend_from_slice(&(signatures.len() as u16).to_be_bytes());
        for (id, signature) in signatures {
            bytes.extend_from_slice(&id);
            bytes.extend_from_slice(&signature);
        }
        if bytes.len() > MAX_PROPOSAL_BYTES {
            return Err(Error::Limit);
        }
        Ok(bytes)
    }
    /// Collect a remote signature only after verifying its exact approval text.
    pub fn approve(&mut self, fingerprint: Id, signature: [u8; 64]) -> Result<(), Error> {
        let (_, identity) = self
            .approvals
            .iter()
            .flatten()
            .find(|(id, _)| *id == fingerprint)
            .ok_or(Error::Unprepared)?;
        verify_signature(identity, &self.signing_bytes(), &signature)?;
        if !self.signatures.iter().any(|(id, _)| *id == fingerprint) {
            self.signatures.push((fingerprint, signature));
        }
        Ok(())
    }
}
impl State {
    pub(super) fn checkpoint(&self) -> Result<Vec<u8>, Error> {
        let mut bytes = b"SGGS\0\x01\0\0".to_vec();
        for id in [&self.group, &self.head, &self.authority] {
            bytes.extend_from_slice(id);
        }
        bytes.extend_from_slice(&self.revision.to_be_bytes());
        bytes.extend_from_slice(&self.epoch.to_be_bytes());
        bytes.extend_from_slice(&[self.earlier_history as u8, self.closed as u8]);
        bytes.extend_from_slice(&(self.members.len() as u16).to_be_bytes());
        for member in &self.members {
            change(&mut bytes, &Change::Add(member.clone()))?;
        }
        if bytes.len() > MAX_CHECKPOINT_BYTES {
            return Err(Error::Limit);
        }
        Ok(bytes)
    }
    // Only storage-key-authenticated checkpoints can reach this helper. They
    // are live state, not a history import or rollback recovery format.
    pub(super) fn from_checkpoint(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_CHECKPOINT_BYTES {
            return Err(Error::Limit);
        }
        let mut reader = Reader(bytes);
        if reader.take(8)? != b"SGGS\0\x01\0\0" {
            return Err(Error::InvalidStore);
        }
        let group = reader.array()?;
        let head = reader.array()?;
        let authority = reader.array()?;
        let revision = u64::from_be_bytes(reader.array()?);
        let epoch = u64::from_be_bytes(reader.array()?);
        let flags = reader.array::<2>()?;
        if epoch > revision || flags.iter().any(|v| *v > 1) {
            return Err(Error::InvalidStore);
        }
        let count = reader.count(MAX_MEMBERS)?;
        let mut members = Vec::with_capacity(count);
        let mut devices = 0;
        for _ in 0..count {
            let Change::Add(member) = reader.change()? else {
                return Err(Error::InvalidStore);
            };
            devices += member.devices.len();
            if devices > MAX_DEVICES {
                return Err(Error::Limit);
            }
            members.push(member);
        }
        let mut state = Self {
            group,
            head,
            authority,
            revision,
            epoch,
            members,
            earlier_history: flags[0] == 1,
            closed: flags[1] == 1,
        };
        state.validate()?;
        if !reader.0.is_empty() || state.checkpoint()? != bytes {
            return Err(Error::InvalidStore);
        }
        Ok(state)
    }
    /// Reconstructs policy from this predecessor. A parsed proposal may still
    /// lack signatures; authorize() is the distinct complete-approval check.
    pub fn proposal_from_bytes(&self, bytes: &[u8]) -> Result<Proposal, Error> {
        if bytes.len() > MAX_PROPOSAL_BYTES {
            return Err(Error::Limit);
        }
        let mut reader = Reader(bytes);
        if reader.take(PROPOSAL.len())? != PROPOSAL
            || reader.array::<32>()? != self.group
            || reader.array::<32>()? != self.head
        {
            return Err(Error::Conflict);
        }
        let author = reader.array()?;
        let head = reader.array::<32>()?;
        let mut proposal = self.propose(author, reader.change()?)?;
        if proposal.head() != head {
            return Err(Error::Conflict);
        }
        let count = reader.count(258)?;
        for _ in 0..count {
            proposal.approve(reader.array()?, reader.array()?)?;
        }
        if !reader.0.is_empty() || proposal.to_bytes()? != bytes {
            return Err(Error::InvalidEvent);
        }
        Ok(proposal)
    }
}
impl Genesis {
    pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let mut bytes = GENESIS.to_vec();
        for id in [
            &self.nonce,
            &self.state.authority,
            &self.state.members[0].id,
        ] {
            bytes.extend_from_slice(id);
        }
        binding(
            &mut bytes,
            &self.state.members[0].devices[0]
                .to_bytes()
                .map_err(|_| Error::InvalidEvent)?,
        )?;
        bytes.extend_from_slice(&self.signature);
        Ok(bytes)
    }
    /// Checks the pinned creator and all signatures before returning any state.
    pub fn from_bytes(bytes: &[u8], expected_creator: Id) -> Result<Self, Error> {
        if bytes.len() > 682 {
            return Err(Error::Limit);
        }
        let mut reader = Reader(bytes);
        if reader.take(GENESIS.len())? != GENESIS {
            return Err(Error::InvalidEvent);
        }
        let nonce = reader.array()?;
        let authority = reader.array()?;
        let member = reader.array()?;
        let (state, creator) = Self::state(nonce, authority, member, &reader.binding()?)?;
        let signature = reader.array()?;
        if !reader.0.is_empty() {
            return Err(Error::InvalidEvent);
        }
        let genesis = Self {
            nonce,
            state,
            creator,
            signature,
        };
        genesis.accept(expected_creator)?;
        Ok(genesis)
    }
}
