//! Push is an untrusted wake-up hint. Only authenticated mailbox sync delivers events.
use serde::{Deserialize, Serialize};

pub const MAX_BODY: usize = 8 * 1024;
pub const MAX_TOKEN: usize = 4096;
pub const MAX_ENDPOINT: usize = 1000;
pub fn valid_token(token: &str) -> bool {
    !token.is_empty() && token.len() <= MAX_TOKEN && token.bytes().all(|b| b.is_ascii_graphic())
}
const PREFIX: &[u8; 8] = b"SGPW\0\0\0\0";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Providers {
    pub unified_push: bool,
    pub fcm: bool,
    pub vapid_public_key: Option<String>,
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "provider", rename_all = "snake_case", deny_unknown_fields)]
pub enum Target {
    Fcm {
        token: String,
    },
    UnifiedPush {
        endpoint: String,
        public_key: String,
        auth_secret: String,
        vapid_key: String,
    },
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Register {
    pub expected_revision: u64,
    pub target: Target,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Confirm {
    pub revision: u64,
    pub channel: String,
    pub proof: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Disabled,
    Pending,
    Active,
    Invalid,
    Expired,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Status {
    pub revision: u64,
    pub state: State,
    pub channel: Option<String>,
    pub expires_at: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Disable {
    pub expected_revision: u64,
}

/// A connector may echo a challenge only for its own durable pending channel.
/// Wake carries no message or account identifiers. Parsing grants no authority.
#[derive(PartialEq, Eq)]
pub enum Payload<'a> {
    Wake,
    Challenge {
        channel: &'a [u8; 32],
        proof: &'a [u8; 32],
    },
}
impl<'a> Payload<'a> {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(73);
        bytes.extend_from_slice(PREFIX);
        match self {
            Self::Wake => bytes.push(0),
            Self::Challenge { channel, proof } => {
                bytes.push(1);
                bytes.extend_from_slice(*channel);
                bytes.extend_from_slice(*proof);
            }
        }
        bytes
    }
    pub fn from_bytes(bytes: &'a [u8]) -> Result<Self, &'static str> {
        if bytes.len() < 9 || &bytes[..8] != PREFIX {
            return Err("invalid push framing");
        }
        match (bytes[8], bytes.len()) {
            (0, 9) => Ok(Self::Wake),
            (1, 73) => Ok(Self::Challenge {
                channel: bytes[9..41].try_into().map_err(|_| "invalid channel")?,
                proof: bytes[41..73].try_into().map_err(|_| "invalid proof")?,
            }),
            _ => Err("invalid push framing"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_push_has_no_optional_message_fields() {
        for payload in [
            Payload::Wake,
            Payload::Challenge {
                channel: &[3; 32],
                proof: &[4; 32],
            },
        ] {
            let bytes = payload.to_bytes();
            assert!(Payload::from_bytes(&bytes).unwrap() == payload);
            for end in 0..bytes.len() {
                assert!(Payload::from_bytes(&bytes[..end]).is_err());
            }
            assert!(Payload::from_bytes(&[bytes.as_slice(), &[0]].concat()).is_err());
            for offset in 0..9 {
                let mut changed = bytes.clone();
                changed[offset] ^= 2;
                assert!(Payload::from_bytes(&changed).is_err());
            }
        }
        assert_eq!(Payload::Wake.to_bytes(), b"SGPW\0\0\0\0\0");
    }
}
