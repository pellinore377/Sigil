//! Private authority transport. Hex fields are strictly bounded by the service.
use serde::{Deserialize, Serialize};

pub const MAX_BODY: usize = 768 * 1024;
pub const MAX_CONTROL: usize = 192 * 1024;
pub const MAX_DEVICES: usize = 1024;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Configure {
    pub expected_revision: u64,
    pub enabled: bool,
    pub storage_limit_bytes: u64,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub revision: u64,
    pub enabled: bool,
    pub storage_limit_bytes: u64,
    pub used_bytes: u64,
    pub authority: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialRequest {
    pub authority: String,
    pub day: u32,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialResponse {
    pub authority: String,
    pub uid: String,
    pub day: u32,
    pub response: String,
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Member {
    pub ciphertext: String,
    pub admin: bool,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Create {
        public: String,
        head: String,
        control: String,
        members: Vec<Member>,
    },
    Advance {
        predecessor: String,
        head: String,
        revision: u64,
        control: String,
        members: Vec<Member>,
    },
    Read {
        from_revision: u64,
    },
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub authority: String,
    pub group: String,
    pub day: u32,
    pub nonce: String,
    pub expires_at: u64,
    pub operation: Operation,
    pub proof: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Commit {
    pub revision: u64,
    pub head: String,
    pub control: String,
    pub receipt: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    pub authority: String,
    pub group: String,
    pub revision: u64,
    pub head: String,
    pub restored: bool,
    pub commits: Vec<Commit>,
}
