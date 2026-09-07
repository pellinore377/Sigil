use serde::{Deserialize, Serialize};

pub const MAX_PAYLOAD_HEX: usize = 134460;
pub const MAX_BODY: usize = 140 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Submit {
    pub recipient_device: String,
    /// Persist this fresh random identifier and the exact encrypted payload before sending.
    pub message_id: String,
    pub payload: String,
    /// Absolute expiry is part of retry identity; retries must not extend it.
    pub expires_at: u64,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Receipt {
    pub sequence: i64,
    pub expires_at: u64,
}

#[derive(Deserialize, Serialize)]
pub struct Delivery {
    pub sequence: i64,
    pub sender_device: String,
    pub message_id: String,
    pub payload: String,
    pub expires_at: u64,
}
