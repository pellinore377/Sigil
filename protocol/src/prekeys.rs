use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PrekeyInventory {
    /// Unclaimed, unexpired public bundles for the authenticated device only.
    pub available: u32,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublishPrekey {
    /// Lowercase hex encoding of the experimental public PQXDH bundle.
    pub bundle: String,
    pub expires_in_seconds: u32,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PublishedPrekey {
    pub prekey_id: String,
    /// The original expiry. Exact publication retries never extend it.
    pub expires_at: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimPrekey {
    /// Persist a fresh random 256-bit request ID before sending the claim.
    pub request_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ClaimedPrekey {
    pub device_id: String,
    pub prekey_id: String,
    pub bundle: String,
    pub expires_at: u64,
}
