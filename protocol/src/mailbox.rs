use serde::{Deserialize, Serialize};

pub const MAX_PAYLOAD_HEX: usize = 134460;
pub const MAX_BODY: usize = 140 * 1024;
pub const RECOVERY_HEADER: &str = "sigil-recovery-request";
/// "1" marks traffic that never yields a visible notification; servers skip push for it.
pub const SILENT_HEADER: &str = "sigil-silent";

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

/// Up to sixteen deliveries acknowledged in one request; ones the server no longer holds are simply skipped.
pub const MAX_ACKNOWLEDGE: usize = 16;
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Acknowledge {
    pub sequences: Vec<i64>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Receipt {
    pub sequence: i64,
    pub expires_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Delivery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<crate::federation::RemoteSender>,
    pub sequence: i64,
    pub sender_device: String,
    pub message_id: String,
    pub payload: String,
    pub expires_at: u64,
}
