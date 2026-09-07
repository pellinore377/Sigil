//! Independently versioned homeserver transport. These keys never verify users.
use serde::{Deserialize, Serialize};
pub const VERSION: u16 = 0;
pub const DISCOVERY_PATH: &str = "/.well-known/sigil/federation";
pub const DELIVER_PATH: &str = "/federation/v0/deliver";
pub const PING_PATH: &str = "/federation/v0/ping";
pub const MAX_BODY: usize = 144 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Key {
    pub id: String,
    pub public_key: String,
    pub generation: u64,
    pub not_before: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rotation {
    pub previous: Key,
    pub signature: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Discovery {
    pub version: u16,
    pub server: String,
    pub current: Key,
    pub rotation: Option<Rotation>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Submit {
    pub sender_account: String,
    pub sender_device: String,
    pub recipient_device: String,
    pub message_id: String,
    pub payload: String,
    pub expires_at: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub request_hash: String,
    pub sequence: i64,
    pub expires_at: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteSender {
    pub server: String,
    pub account: String,
    pub device: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigureSender {
    pub expected_revision: u64,
    pub sender: RemoteSender,
    pub allowed: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SenderPermission {
    pub revision: u64,
    pub sender: RemoteSender,
    pub allowed: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Delivery {
    pub sequence: i64,
    pub sender: RemoteSender,
    pub message_id: String,
    pub payload: String,
    pub expires_at: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Queue {
    pub destination: String,
    pub recipient_device: String,
    pub message_id: String,
    pub payload: String,
    pub expires_at: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboundState {
    Pending,
    Accepted,
    Rejected,
    Expired,
    Revoked,
    Restored,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outbound {
    pub message_id: String,
    pub destination: String,
    pub request_hash: String,
    pub expires_at: u64,
    pub state: OutboundState,
    pub receipt: Option<Receipt>,
    pub not_before: u64,
    pub attempts: u32,
    pub error: Option<String>,
}

pub const LOOKUP_PATH: &str = "/federation/v0/lookup";
pub const MAX_LOOKUP_BODY: usize = 4096;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Lookup {
    Binding { device: String },
    Claim { device: String, request_id: String },
}
impl Lookup {
    pub fn device(&self) -> &str {
        match self {
            Self::Binding { device } | Self::Claim { device, .. } => device,
        }
    }
    pub fn valid(&self) -> bool {
        crate::accounts::valid_credential(self.device())
            && match self {
                Self::Binding { .. } => true,
                Self::Claim { request_id, .. } => crate::accounts::valid_credential(request_id),
            }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyLookup {
    pub destination: String,
    pub operation: Lookup,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerLookup {
    pub sender_account: String,
    pub sender_device: String,
    pub operation: Lookup,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum LookupValue {
    Binding(String),
    Prekey(crate::prekeys::ClaimedPrekey),
}
impl LookupValue {
    /// Shape/routing validation only. Peer signature and fingerprint verification
    /// remain mandatory in the encrypted client before handshake acceptance.
    pub fn valid_for(&self, operation: &Lookup, server: &str) -> bool {
        if !operation.valid() || !crate::valid_server_name(server) {
            return false;
        }
        match (self, operation) {
            (Self::Binding(statement), Lookup::Binding { device }) => {
                let Ok(bytes) = (crate::device::Statement {
                    statement: statement.clone(),
                })
                .bytes() else {
                    return false;
                };
                let Ok(signed) = crate::device::SignedBinding::from_bytes(&bytes) else {
                    return false;
                };
                signed.binding.server == server
                    && device
                        .as_bytes()
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .zip(signed.binding.device)
                        .all(|(pair, byte)| {
                            std::str::from_utf8(pair)
                                .ok()
                                .and_then(|p| u8::from_str_radix(p, 16).ok())
                                == Some(byte)
                        })
                    && device.len() == 64
            }
            (Self::Prekey(value), Lookup::Claim { device, .. }) => {
                value.device_id == *device
                    && crate::accounts::valid_credential(&value.prekey_id)
                    && matches!(value.bundle.len(), 3544 | 3610)
                    && value
                        .bundle
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    && value.expires_at > 0
                    && value.expires_at <= i64::MAX as u64
            }
            _ => false,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookupReply {
    pub request_hash: String,
    pub value: LookupValue,
}
