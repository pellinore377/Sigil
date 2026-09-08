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
}
impl Roster {
    pub fn validate(&self) -> Result<(), Error> {
        if self.version != 1
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
            || !self.members.iter().any(|m| m.key == self.owner)
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
        if owner.public_key() != self.owner {
            return Err(Error::Authentication);
        }
        let signature = owner
            .sign(&self.signing_bytes()?)
            .map_err(|_| Error::Authentication)?
            .to_vec();
        Ok(SignedRoster {
            roster: self,
            signature,
        })
    }
    pub fn successor(&self, next: &Self, consecutive: bool) -> Result<(), Error> {
        next.validate()?;
        if self.closed
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
        verify_signature(
            &self.roster.owner,
            &self.roster.signing_bytes()?,
            &self.signature,
        )
        .map_err(|_| Error::Authentication)
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
