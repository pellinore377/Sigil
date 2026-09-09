use crate::{hash, Error, Id};
use serde::{Deserialize, Serialize};
use sigil_crypto::{verify_signature, IdentityKey};
pub const MAX_ROSTER: usize = 16384;
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Member {
    pub id: Id,
    pub key: Id,
}
impl Member {
    pub fn new(key: Id) -> Self {
        Self {
            id: hash(b"Sigil/call-member/v1", &key),
            key,
        }
    }
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Roster {
    pub version: u8,
    pub call: Id,
    pub server: String,
    pub owner: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller: Option<Id>,
    pub created: u64,
    pub expires: u64,
    pub revision: u64,
    pub previous: Option<Id>,
    pub members: Vec<Member>,
    pub closed: bool,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SignedRoster {
    pub roster: Roster,
    pub signature: Vec<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delegations: Vec<Delegation>,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Delegation {
    pub revision: u64,
    pub previous: Id,
    pub controller: Id,
    pub signature: Vec<u8>,
}
impl Delegation {
    fn digest(&self, roster: &Roster) -> Result<Id, Error> {
        if roster.version != 2
            || self.revision == 0
            || self.revision > 65535
            || self.previous == [0; 32]
            || self.controller == [0; 32]
        {
            return Err(Error::Invalid);
        }
        let bytes = serde_json::to_vec(&(
            roster.call,
            &roster.server,
            roster.owner,
            roster.created,
            roster.expires,
            self.revision,
            self.previous,
            self.controller,
        ))
        .map_err(|_| Error::Invalid)?;
        Ok(hash(b"Sigil/call-controller/v1", &bytes))
    }
    fn verify(&self, roster: &Roster, controller: &Id) -> Result<(), Error> {
        if *controller == self.controller {
            return Err(Error::Invalid);
        }
        verify_signature(controller, &self.digest(roster)?, &self.signature)
            .map_err(|_| Error::Authentication)
    }
}
impl Roster {
    pub fn controller(&self) -> Id {
        self.controller.unwrap_or(self.owner)
    }
    pub fn validate(&self) -> Result<(), Error> {
        if !matches!(self.version, 1 | 2)
            || (self.version == 1) != self.controller.is_none()
            || self.call == [0; 32]
            || self.owner == [0; 32]
            || !sigil_protocol::valid_server_name(&self.server)
            || self.created == 0
            || self.expires <= self.created
            || self.expires - self.created > 86400
            || self.expires > i64::MAX as u64
            || self.revision > 65535
            || self.previous.is_none() != (self.revision == 0)
            || self.members.is_empty()
            || self.members.len() > 8
            || !self.members.iter().any(|m| m.key == self.controller())
        {
            return Err(Error::Invalid);
        }
        for (i, m) in self.members.iter().enumerate() {
            if *m != Member::new(m.key)
                || m.key == [0; 32]
                || i > 0 && self.members[i - 1].id >= m.id
            {
                return Err(Error::Invalid);
            }
        }
        Ok(())
    }
    pub fn member(&self, id: Id) -> Result<&Member, Error> {
        self.members
            .iter()
            .find(|m| m.id == id)
            .ok_or(Error::Authentication)
    }
    pub fn active(&self, now: u64) -> Result<(), Error> {
        if self.closed || now < self.created || now >= self.expires {
            Err(Error::Expired)
        } else {
            Ok(())
        }
    }
    fn signing_bytes(&self) -> Result<Vec<u8>, Error> {
        self.validate()?;
        let raw = serde_json::to_vec(self).map_err(|_| Error::Invalid)?;
        if raw.len() > MAX_ROSTER - 64 {
            return Err(Error::Limit);
        }
        Ok([b"Sigil/call-roster/v1".as_slice(), &raw].concat())
    }
    pub fn digest(&self) -> Result<Id, Error> {
        Ok(hash(b"Sigil/call-roster-head/v1", &self.signing_bytes()?))
    }
    pub fn sign(self, owner: &IdentityKey) -> Result<SignedRoster, Error> {
        if owner.public_key() != self.owner || self.controller() != self.owner {
            return Err(Error::Authentication);
        }
        let signature = owner
            .sign(&self.signing_bytes()?)
            .map_err(|_| Error::Authentication)?
            .to_vec();
        Ok(SignedRoster {
            roster: self,
            signature,
            delegations: Vec::new(),
        })
    }
    pub fn successor(&self, next: &Self, consecutive: bool) -> Result<(), Error> {
        next.validate()?;
        if self.closed
            || self.version != next.version
            || self.call != next.call
            || self.server != next.server
            || self.owner != next.owner
            || self.created != next.created
            || self.expires != next.expires
            || next.revision <= self.revision
        {
            return Err(Error::Conflict);
        }
        if (consecutive || next.revision == self.revision + 1)
            && (next.revision != self.revision + 1 || next.previous != Some(self.digest()?))
        {
            return Err(Error::Conflict);
        }
        Ok(())
    }
}
impl SignedRoster {
    pub fn verify(&self) -> Result<(), Error> {
        self.roster.validate()?;
        if self.delegations.len() > 16 || self.roster.version == 1 && !self.delegations.is_empty() {
            return Err(Error::Limit);
        }
        let mut controller = self.roster.owner;
        let mut revision = 0;
        for delegation in &self.delegations {
            if delegation.revision <= revision || delegation.revision > self.roster.revision {
                return Err(Error::Conflict);
            }
            delegation.verify(&self.roster, &controller)?;
            controller = delegation.controller;
            revision = delegation.revision;
        }
        if controller != self.roster.controller() {
            return Err(Error::Authentication);
        }
        verify_signature(&controller, &self.roster.signing_bytes()?, &self.signature)
            .map_err(|_| Error::Authentication)
    }
    pub fn successor(&self, next: &Self, consecutive: bool) -> Result<(), Error> {
        self.verify()?;
        next.verify()?;
        self.roster.successor(&next.roster, consecutive)?;
        if !next.delegations.starts_with(&self.delegations) {
            return Err(Error::Conflict);
        }
        if let Some(change) = next.delegations.get(self.delegations.len()) {
            if change.revision <= self.roster.revision {
                return Err(Error::Conflict);
            }
            if change.revision == self.roster.revision + 1
                && (change.previous != self.roster.digest()?
                    || !self
                        .roster
                        .members
                        .iter()
                        .any(|m| m.key == change.controller))
            {
                return Err(Error::Conflict);
            }
        }
        Ok(())
    }
    pub fn update(&self, roster: Roster, key: &IdentityKey) -> Result<Self, Error> {
        if key.public_key() != self.roster.controller()
            || roster.controller() != self.roster.controller()
        {
            return Err(Error::Authentication);
        }
        let signature = key
            .sign(&roster.signing_bytes()?)
            .map_err(|_| Error::Authentication)?
            .to_vec();
        let next = Self {
            roster,
            signature,
            delegations: self.delegations.clone(),
        };
        self.successor(&next, true)?;
        Ok(next)
    }
    pub fn delegate(&self, controller: Id, key: &IdentityKey) -> Result<Delegation, Error> {
        self.verify()?;
        if self.roster.closed
            || key.public_key() != self.roster.controller()
            || controller == key.public_key()
            || !self.roster.members.iter().any(|m| m.key == controller)
        {
            return Err(Error::Authentication);
        }
        if self.delegations.len() == 16 {
            return Err(Error::Limit);
        }
        let mut proof = Delegation {
            revision: self.roster.revision.checked_add(1).ok_or(Error::Limit)?,
            previous: self.roster.digest()?,
            controller,
            signature: Vec::new(),
        };
        proof.signature = key
            .sign(&proof.digest(&self.roster)?)
            .map_err(|_| Error::Authentication)?
            .to_vec();
        Ok(proof)
    }
    pub fn verify_delegation(&self, proof: &Delegation) -> Result<(), Error> {
        self.verify()?;
        if self.roster.closed
            || self.delegations.len() == 16
            || proof.revision != self.roster.revision + 1
            || proof.previous != self.roster.digest()?
            || !self
                .roster
                .members
                .iter()
                .any(|m| m.key == proof.controller)
        {
            return Err(Error::Conflict);
        }
        proof.verify(&self.roster, &self.roster.controller())
    }
    pub fn transfer(&self, proof: Delegation, key: &IdentityKey) -> Result<Self, Error> {
        self.verify_delegation(&proof)?;
        if key.public_key() != proof.controller {
            return Err(Error::Authentication);
        }
        let mut roster = self.roster.clone();
        roster.revision = proof.revision;
        roster.previous = Some(proof.previous);
        roster.controller = Some(proof.controller);
        roster.members.retain(|m| m.key != self.roster.controller());
        let mut delegations = self.delegations.clone();
        delegations.push(proof);
        let signature = key
            .sign(&roster.signing_bytes()?)
            .map_err(|_| Error::Authentication)?
            .to_vec();
        let next = Self {
            roster,
            signature,
            delegations,
        };
        self.successor(&next, true)?;
        Ok(next)
    }
    pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        self.verify()?;
        let bytes = serde_json::to_vec(self).map_err(|_| Error::Invalid)?;
        if bytes.len() > MAX_ROSTER {
            return Err(Error::Limit);
        }
        Ok(bytes)
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_ROSTER {
            return Err(Error::Limit);
        }
        let result: Self = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
        result.verify()?;
        if result.to_bytes()? != bytes {
            return Err(Error::Invalid);
        }
        Ok(result)
    }
}
