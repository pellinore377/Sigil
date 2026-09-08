use crate::{hash, Error, Id, MediaKind, Roster};
use serde::{Deserialize, Serialize};
use sigil_crypto::{verify_signature, IdentityKey};
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Track {
    pub sender: Id,
    pub kind: MediaKind,
    pub mid: String,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    pub uploads: [String; 3],
    pub downloads: Vec<Track>,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Connect {
    pub call: Id,
    pub roster: Id,
    pub participant: Id,
    pub sequence: u64,
    pub sdp: String,
    pub layout: Layout,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SignedConnect {
    pub request: Connect,
    pub signature: Vec<u8>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Answer {
    pub sdp: String,
    pub sequence: u64,
    pub roster: Id,
    pub participant: Id,
    pub streams: Vec<Downstream>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Downstream {
    pub track: Track,
    pub ssrc: u32,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Relay {
    pub urls: Vec<String>,
    pub username: String,
    pub credential: zeroize::Zeroizing<String>,
    pub expires: u64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelayRequest {
    pub call: Id,
    pub roster: Id,
    pub participant: Id,
    pub at: u64,
    pub signature: Vec<u8>,
}
impl RelayRequest {
    fn digest(&self) -> Id {
        hash(
            b"Sigil/call-relay/v1",
            &[
                self.call.as_slice(),
                self.roster.as_slice(),
                self.participant.as_slice(),
                self.at.to_be_bytes().as_slice(),
            ]
            .concat(),
        )
    }
    pub fn new(roster: &Roster, identity: &IdentityKey, at: u64) -> Result<Self, Error> {
        roster.active(at)?;
        let participant = crate::Member::new(identity.public_key()).id;
        roster.member(participant)?;
        let mut value = Self {
            call: roster.call,
            roster: roster.digest()?,
            participant,
            at,
            signature: Vec::new(),
        };
        value.signature = identity
            .sign(&value.digest())
            .map_err(|_| Error::Authentication)?
            .to_vec();
        Ok(value)
    }
    pub fn verify(&self, roster: &Roster, now: u64) -> Result<(), Error> {
        roster.active(now)?;
        if self.call != roster.call || self.roster != roster.digest()? {
            return Err(Error::Conflict);
        }
        if self.at.abs_diff(now) > 60 {
            return Err(Error::Expired);
        }
        verify_signature(
            &roster.member(self.participant)?.key,
            &self.digest(),
            &self.signature,
        )
        .map_err(|_| Error::Authentication)
    }
}
impl Layout {
    pub fn validate(&self, roster: &Roster, participant: Id) -> Result<(), Error> {
        roster.member(participant)?;
        if self.downloads.len() != 3 * (roster.members.len() - 1) {
            return Err(Error::Invalid);
        }
        let mut mids = std::collections::BTreeSet::new();
        for mid in self
            .uploads
            .iter()
            .chain(self.downloads.iter().map(|v| &v.mid))
        {
            if mid.is_empty()
                || mid.len() > 16
                || !mid.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                || !mids.insert(mid)
            {
                return Err(Error::Invalid);
            }
        }
        let mut routes = std::collections::BTreeSet::new();
        for track in &self.downloads {
            roster.member(track.sender)?;
            if track.sender == participant || !routes.insert((track.sender, track.kind as u8)) {
                return Err(Error::Invalid);
            }
        }
        Ok(())
    }
}
impl Connect {
    pub fn digest(&self) -> Result<Id, Error> {
        if self.sdp.len() > 65536 || self.sequence == 0 || self.sequence > i64::MAX as u64 {
            return Err(Error::Invalid);
        }
        let bytes = serde_json::to_vec(self).map_err(|_| Error::Invalid)?;
        if bytes.len() > 98304 {
            return Err(Error::Limit);
        }
        Ok(hash(b"Sigil/call-connect/v1", &bytes))
    }
    pub fn sign(self, identity: &IdentityKey) -> Result<SignedConnect, Error> {
        if crate::Member::new(identity.public_key()).id != self.participant {
            return Err(Error::Authentication);
        }
        let signature = identity
            .sign(&self.digest()?)
            .map_err(|_| Error::Authentication)?
            .to_vec();
        Ok(SignedConnect {
            request: self,
            signature,
        })
    }
}
impl SignedConnect {
    pub fn verify(&self, roster: &Roster, now: u64) -> Result<(), Error> {
        roster.active(now)?;
        if self.request.call != roster.call || self.request.roster != roster.digest()? {
            return Err(Error::Conflict);
        }
        self.request
            .layout
            .validate(roster, self.request.participant)?;
        verify_signature(
            &roster.member(self.request.participant)?.key,
            &self.request.digest()?,
            &self.signature,
        )
        .map_err(|_| Error::Authentication)
    }
}

pub fn validate_turn_url(url: &str) -> Result<(), Error> {
    let invalid = || Error::Invalid;
    if url.len() > 512
        || !url.is_ascii()
        || url
            .bytes()
            .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
    {
        return Err(invalid());
    }
    let (tls, rest) = if let Some(v) = url.strip_prefix("turns:") {
        (true, v)
    } else if let Some(v) = url.strip_prefix("turn:") {
        (false, v)
    } else {
        return Err(invalid());
    };
    let (authority, transport) = rest.split_once('?').ok_or_else(invalid)?;
    if !matches!(transport, "transport=udp" | "transport=tcp")
        || (tls && transport != "transport=tcp")
    {
        return Err(invalid());
    }
    let (host, port) = authority.rsplit_once(':').ok_or_else(invalid)?;
    let host_valid = if host.starts_with('[') && host.ends_with(']') {
        host[1..host.len() - 1]
            .parse::<std::net::Ipv6Addr>()
            .is_ok()
    } else {
        host.parse::<std::net::Ipv4Addr>().is_ok() || sigil_protocol::valid_server_name(host)
    };
    if !host_valid || !port.parse::<u16>().is_ok_and(|p| p > 0) {
        return Err(invalid());
    }
    Ok(())
}
