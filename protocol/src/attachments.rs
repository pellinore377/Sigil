//! Opaque same-server storage. File keys and filenames are never server fields.
use serde::{Deserialize, Serialize};
pub const MAX_PARTS_PAGE: usize = 64;
pub const ACCESS_HEADER: &str = "sigil-attachment-access";

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Begin {
    pub plaintext_bytes: u64,
    pub access_token: String,
    /// None retains published ciphertext until explicit deletion or policy cleanup.
    pub expires_at: Option<u64>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Publish {
    pub root: String,
    #[serde(default)]
    pub acknowledge_restored_checkpoint: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Uploading,
    Published,
    Removed,
    Expired,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Status {
    pub state: State,
    pub plaintext_bytes: u64,
    pub chunks: u32,
    pub received_chunks: u32,
    pub upload_deadline: u64,
    pub expires_at: Option<u64>,
    pub root: Option<String>,
    pub restored_checkpoint: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Part {
    pub index: u32,
    pub hash: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Parts {
    pub chunks: Vec<Part>,
    pub next_after: Option<u32>,
}
