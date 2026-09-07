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
