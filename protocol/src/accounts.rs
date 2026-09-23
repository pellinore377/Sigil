use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InviteRequest {
    pub username: String,
    pub expires_in_seconds: u32,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReauthorizationRequest {
    pub expires_in_seconds: u32,
}

#[derive(Deserialize, Serialize)]
pub struct Invitation {
    pub id: String,
    pub secret: String,
    pub expires_at: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Enrollment {
    pub invitation: String,
    pub device_credential: String,
    pub device_label: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RotateCredential {
    pub device_credential: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Session {
    pub account_id: String,
    pub address: String,
    pub device_id: String,
    pub device_label: String,
    pub expires_at: u64,
    /// Signed in but not yet endorsed by the account key.
    #[serde(default)]
    pub pending: bool,
}
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DeviceSummary {
    pub id: String,
    pub label: String,
    pub expires_at: u64,
    pub revoked: bool,
}
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DevicePage {
    pub account_id: String,
    pub devices: Vec<DeviceSummary>,
    pub next_after: Option<String>,
}
pub const DEVICE_PAGE_SIZE: usize = 32;

pub fn valid_username(value: &str) -> bool {
    (1..=32).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

pub fn valid_credential(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Account public key (hex) and its recovery bundle (hex, sealed under the recovery secret).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AccountKey {
    pub public: String,
    pub bundle: Option<String>,
}
/// Publishes the account key with the caller's endorsement; `bundle` may be replaced later.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublishAccountKey {
    pub public: String,
    pub bundle: Option<String>,
    pub endorsement: String,
}
/// A pending device joins with its binding and an account-key endorsement of it.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Activate {
    pub statement: String,
    pub endorsement: String,
}
/// Replaces the account key; every other device is signed out.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResetIdentity {
    pub statement: String,
    pub key: PublishAccountKey,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Endorse {
    pub endorsement: String,
}
/// Recovery secret wrapped under one passkey's PRF output. Hex fields.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecoveryWrap {
    pub id: String,
    pub salt: String,
    pub wrapped: String,
    pub label: String,
    pub created: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecoveryWraps {
    pub wraps: Vec<RecoveryWrap>,
}
pub const MAX_RECOVERY_WRAPS: usize = 16;
/// What a pending device may read before recovery.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PendingState {
    pub session: Session,
    pub account_key: Option<AccountKey>,
    pub wraps: Vec<RecoveryWrap>,
}
