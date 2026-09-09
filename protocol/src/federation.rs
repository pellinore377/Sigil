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
    /// Opaque positive acceptance token, not a recipient cursor or ordering counter.
    /// The full request_hash identifies the request; this token need not be unique.
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
pub const MAX_LOOKUP_BODY: usize = 2 * crate::groups::MAX_BODY + 8192;
pub const MAX_LOOKUP_RESPONSE: usize = 2 * (1024 * 1024 + 84) + 8192;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Service {
    ContactDirectory {
        username: String,
    },
    ContactProfile {
        account: String,
    },
    ContactPhoto {
        account: String,
        hash: String,
    },
    ContactRequest {
        request: crate::contacts::RequestContact,
    },
    ContactStatus {
        recipient: String,
    },
    GroupAuthority,
    GroupCredential {
        authority: String,
        day: u32,
        binding: String,
    },
    GroupRequest {
        request: String,
    },
    CallConnect {
        request: String,
    },
    CallRelay {
        request: String,
    },
    CallUpdate {
        request: String,
    },
    AttachmentChunk {
        file: String,
        index: u32,
        access: String,
    },
}
impl Service {
    pub fn valid(&self) -> bool {
        match self {
            Self::ContactDirectory { username } => crate::accounts::valid_username(username),
            Self::ContactProfile { account } => crate::accounts::valid_credential(account),
            Self::ContactPhoto { account, hash } => {
                crate::accounts::valid_credential(account)
                    && crate::accounts::valid_credential(hash)
            }
            Self::ContactRequest { request } => request.valid(),
            Self::ContactStatus { recipient } => crate::accounts::valid_credential(recipient),
            Self::GroupAuthority => true,
            Self::GroupCredential {
                authority, binding, ..
            } => {
                crate::accounts::valid_credential(authority)
                    && (crate::device::Statement {
                        statement: binding.clone(),
                    })
                    .bytes()
                    .is_ok()
            }
            Self::GroupRequest { request } => {
                !request.is_empty() && request.len() <= crate::groups::MAX_BODY + 2048
            }
            Self::CallConnect { request } => !request.is_empty() && request.len() <= 100352,
            Self::CallRelay { request } => !request.is_empty() && request.len() <= 2048,
            Self::CallUpdate { request } => !request.is_empty() && request.len() <= 16384,
            Self::AttachmentChunk { file, access, .. } => {
                crate::accounts::valid_credential(file) && crate::accounts::valid_credential(access)
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Lookup {
    Account { username: String },
    Service { service: Service },
    Binding { device: String },
    Claim { device: String, request_id: String },
}
impl Lookup {
    pub fn anonymous(&self) -> bool {
        matches!(
            self,
            Self::Service {
                service: Service::GroupAuthority
                    | Service::GroupRequest { .. }
                    | Service::CallConnect { .. }
                    | Service::CallRelay { .. }
                    | Service::CallUpdate { .. }
                    | Service::AttachmentChunk { .. }
            }
        )
    }
    pub fn device(&self) -> Option<&str> {
        match self {
            Self::Binding { device } | Self::Claim { device, .. } => Some(device),
            Self::Service { .. } | Self::Account { .. } => None,
        }
    }
    pub fn valid(&self) -> bool {
        match self {
            Self::Account { username } => crate::accounts::valid_username(username),
            Self::Service { service } => service.valid(),
            Self::Binding { device } => crate::accounts::valid_credential(device),
            Self::Claim { device, request_id } => {
                crate::accounts::valid_credential(device)
                    && crate::accounts::valid_credential(request_id)
            }
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
    Account(crate::admin::FoundAccount),
    Service(String),
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
            (Self::Account(value), Lookup::Account { username }) => {
                value.valid_for(username, server)
            }
            (Self::Service(value), Lookup::Service { .. }) => value.len() <= MAX_LOOKUP_RESPONSE,
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
