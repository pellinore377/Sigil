use crate::{hash, Error, Id, KeyShare, Member, Roster, SignedRoster};
use serde::{Deserialize, Serialize};
use sigil_crypto::{verify_signature, IdentityKey};
use sigil_protocol::device::SignedBinding;
use zeroize::Zeroizing;

fn scope(roster: &Roster) -> Vec<u8> {
    [
        roster.call.as_slice(),
        roster.owner.as_slice(),
        roster.server.as_bytes(),
        &roster.created.to_be_bytes(),
        &roster.expires.to_be_bytes(),
    ]
    .concat()
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Attestation {
    pub member: Member,
    pub device: Vec<u8>,
    pub signature: Vec<u8>,
}
impl Attestation {
    fn digest(&self, roster: &Roster) -> Result<Id, Error> {
        if self.device.len() > 512 || self.member != Member::new(self.member.key) {
            return Err(Error::Invalid);
        }
        Ok(hash(
            b"Sigil/call-device/v1",
            &[scope(roster), self.member.key.to_vec(), self.device.clone()].concat(),
        ))
    }
    pub fn sign(
        roster: &Roster,
        member: Member,
        device: Vec<u8>,
        identity: &IdentityKey,
    ) -> Result<Self, Error> {
        let binding = SignedBinding::from_bytes(&device).map_err(|_| Error::Invalid)?;
        if binding.binding.identity != identity.public_key() {
            return Err(Error::Authentication);
        }
        let mut value = Self {
            member,
            device,
            signature: Vec::new(),
        };
        value.signature = identity
            .sign(&value.digest(roster)?)
            .map_err(|_| Error::Authentication)?
            .to_vec();
        value.verify(roster)?;
        Ok(value)
    }
    pub fn verify(&self, roster: &Roster) -> Result<SignedBinding, Error> {
        let binding = SignedBinding::from_bytes(&self.device).map_err(|_| Error::Invalid)?;
        verify_signature(
            &binding.binding.identity,
            &binding
                .binding
                .signing_bytes()
                .map_err(|_| Error::Invalid)?,
            &binding.signature,
        )
        .map_err(|_| Error::Authentication)?;
        verify_signature(
            &binding.binding.identity,
            &self.digest(roster)?,
            &self.signature,
        )
        .map_err(|_| Error::Authentication)?;
        Ok(binding)
    }
    pub fn fingerprint(&self) -> Result<Id, Error> {
        use sha2::{Digest, Sha256};
        let binding = SignedBinding::from_bytes(&self.device).map_err(|_| Error::Invalid)?;
        Ok(Sha256::digest(
            binding
                .binding
                .signing_bytes()
                .map_err(|_| Error::Invalid)?,
        )
        .into())
    }
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct Tracks {
    pub audio: bool,
    pub camera: bool,
    pub screen: bool,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Ready {
    pub member: Id,
    pub sequence: u64,
    pub challenge: Id,
    pub tracks: Tracks,
    pub signature: Vec<u8>,
}
impl Ready {
    fn digest(&self, roster: &Roster) -> Result<Id, Error> {
        if self.sequence == 0 || self.sequence > 65535 || self.challenge == [0; 32] {
            return Err(Error::Invalid);
        }
        Ok(hash(
            b"Sigil/call-ready/v1",
            &[
                scope(roster),
                self.member.to_vec(),
                self.sequence.to_be_bytes().to_vec(),
                self.challenge.to_vec(),
                vec![
                    self.tracks.audio.into(),
                    self.tracks.camera.into(),
                    self.tracks.screen.into(),
                ],
            ]
            .concat(),
        ))
    }
    pub fn sign(
        roster: &Roster,
        sequence: u64,
        challenge: Id,
        tracks: Tracks,
        key: &IdentityKey,
    ) -> Result<Self, Error> {
        let mut value = Self {
            member: Member::new(key.public_key()).id,
            sequence,
            challenge,
            tracks,
            signature: Vec::new(),
        };
        value.signature = key
            .sign(&value.digest(roster)?)
            .map_err(|_| Error::Authentication)?
            .to_vec();
        value.verify(roster)?;
        Ok(value)
    }
    pub fn verify(&self, roster: &Roster) -> Result<(), Error> {
        verify_signature(
            &roster.member(self.member)?.key,
            &self.digest(roster)?,
            &self.signature,
        )
        .map_err(|_| Error::Authentication)
    }
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub roster: SignedRoster,
    pub epoch: u64,
    pub participants: Vec<Attestation>,
    pub ready: Vec<Ready>,
    pub signature: Vec<u8>,
}
impl State {
    pub fn digest(&self) -> Result<Id, Error> {
        if self.epoch > 65535
            || self.epoch < self.roster.roster.revision
            || self.participants.len() > 8
            || self.ready.len() > 8
        {
            return Err(Error::Limit);
        }
        let bytes = serde_json::to_vec(&(
            &self.roster.roster,
            self.epoch,
            &self.participants,
            &self.ready,
        ))
        .map_err(|_| Error::Invalid)?;
        if bytes.len() > 49152 {
            return Err(Error::Limit);
        }
        Ok(hash(b"Sigil/call-state/v1", &bytes))
    }
    pub fn sign(mut self, owner: &IdentityKey) -> Result<Self, Error> {
        if owner.public_key() != self.roster.roster.owner {
            return Err(Error::Authentication);
        }
        self.signature = owner
            .sign(&self.digest()?)
            .map_err(|_| Error::Authentication)?
            .to_vec();
        self.verify()?;
        Ok(self)
    }
    pub fn verify(&self) -> Result<(), Error> {
        self.roster.verify()?;
        verify_signature(&self.roster.roster.owner, &self.digest()?, &self.signature)
            .map_err(|_| Error::Authentication)?;
        if self.participants.len() != self.roster.roster.members.len() {
            return Err(Error::Invalid);
        }
        let mut fingerprints = std::collections::BTreeSet::new();
        for (proof, member) in self.participants.iter().zip(&self.roster.roster.members) {
            if proof.member != *member || !fingerprints.insert(proof.fingerprint()?) {
                return Err(Error::Invalid);
            }
            proof.verify(&self.roster.roster)?;
        }
        let mut previous = None;
        for value in &self.ready {
            if previous.is_some_and(|p| p >= value.member) {
                return Err(Error::Invalid);
            }
            value.verify(&self.roster.roster)?;
            previous = Some(value.member);
        }
        Ok(())
    }
    pub fn successor(&self, next: &Self) -> Result<(), Error> {
        next.verify()?;
        if next.epoch <= self.epoch || self.roster.roster.closed {
            return Err(Error::Conflict);
        }
        if self.roster.roster != next.roster.roster {
            self.roster.roster.successor(&next.roster.roster, false)?;
        }
        for old in &self.participants {
            if next.participants.iter().any(|new| {
                new.fingerprint().ok() == old.fingerprint().ok() && new.member != old.member
            }) {
                return Err(Error::Authentication);
            }
        }
        for old in &self.ready {
            if let Some(new) = next.ready.iter().find(|r| r.member == old.member) {
                if new.sequence < old.sequence || (new.sequence == old.sequence && new != old) {
                    return Err(Error::Conflict);
                }
            } else if next.roster.roster.member(old.member).is_ok() {
                return Err(Error::Conflict);
            }
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Share {
    pub state: Id,
    pub generation: u64,
    pub key: KeyShare,
    pub signature: Vec<u8>,
}
impl Share {
    fn digest(&self) -> Result<Id, Error> {
        if self.generation == 0 || self.generation > 65535 {
            return Err(Error::Limit);
        }
        let bytes = Zeroizing::new(
            serde_json::to_vec(&(self.state, self.generation, &self.key))
                .map_err(|_| Error::Invalid)?,
        );
        Ok(hash(b"Sigil/call-share/v1", &bytes))
    }
    pub fn sign(
        state: &State,
        generation: u64,
        key: KeyShare,
        identity: &IdentityKey,
    ) -> Result<Self, Error> {
        if Member::new(identity.public_key()).id != key.context.sender {
            return Err(Error::Authentication);
        }
        let mut value = Self {
            state: state.digest()?,
            generation,
            key,
            signature: Vec::new(),
        };
        value.signature = identity
            .sign(&value.digest()?)
            .map_err(|_| Error::Authentication)?
            .to_vec();
        value.verify(state)?;
        Ok(value)
    }
    pub fn verify(&self, state: &State) -> Result<(), Error> {
        if self.state != state.digest()?
            || self.key.context.call != state.roster.roster.call
            || self.key.context.roster != state.roster.roster.digest()?
            || !state
                .ready
                .iter()
                .any(|r| r.member == self.key.context.sender)
        {
            return Err(Error::Conflict);
        }
        verify_signature(
            &state.roster.roster.member(self.key.context.sender)?.key,
            &self.digest()?,
            &self.signature,
        )
        .map_err(|_| Error::Authentication)
    }
}
