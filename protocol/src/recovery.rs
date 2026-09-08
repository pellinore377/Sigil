use serde::{Deserialize, Serialize};

pub const MAX_OBJECT_BYTES: usize = 67266;
pub const MAX_BODY: usize = 140 * 1024;
pub const MAX_DELETE_OBJECTS: usize = 64;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StorageStatus {
    pub used_bytes: u64,
    pub quota_bytes: u64,
    pub recovery_objects: u64,
    pub recovery_object_limit: u64,
}
impl StorageStatus {
    pub fn nearly_full(&self) -> bool {
        self.used_bytes as u128 * 10 >= self.quota_bytes as u128 * 9
            || self.recovery_objects as u128 * 10 >= self.recovery_object_limit as u128 * 9
    }
    pub fn full(&self) -> bool {
        self.used_bytes >= self.quota_bytes || self.recovery_objects >= self.recovery_object_limit
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PutObject {
    pub ciphertext: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Head {
    pub generation: u64,
    pub manifest: Option<String>,
    /// Operator restore may have rolled back history/deletion state.
    pub restored_checkpoint: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublishHead {
    pub expected_generation: u64,
    pub expected_manifest: Option<String>,
    pub manifest: String,
    #[serde(default)]
    pub acknowledge_restored_checkpoint: bool,
    /// Advance beyond a newer client anchor while CAS remains bound to the
    /// restored server head. Only valid for an explicitly acknowledged restore.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore_generation: Option<u64>,
}

impl PublishHead {
    pub fn generation(&self) -> Option<u64> {
        if self.expected_generation >= i64::MAX as u64 {
            return None;
        }
        let next = self.expected_generation + 1;
        match self.restore_generation {
            Some(value)
                if self.acknowledge_restored_checkpoint
                    && self.expected_generation > 0
                    && value > next
                    && value <= i64::MAX as u64 =>
            {
                Some(value)
            }
            Some(_) => None,
            None => Some(next),
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteObjects {
    pub expected_generation: u64,
    pub expected_manifest: String,
    pub objects: Vec<String>,
}
