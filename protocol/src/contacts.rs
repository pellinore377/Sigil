use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateContactInvite {
    /// Generate and persist 32 random bytes as lowercase hex before submission.
    pub secret: String,
    pub expires_at: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RedeemContactInvite {
    pub secret: String,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ContactInvite {
    pub id: String,
    pub device_id: String,
    pub expires_at: u64,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ContactPeer {
    pub device_id: String,
}
