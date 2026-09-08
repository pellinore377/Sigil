use serde::{Deserialize, Serialize};
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Start {
    pub request_id: String,
    pub secret: String,
    pub username: Option<String>,
    pub replace_devices: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Started {
    pub authorization_url: String,
    pub expires_at: u64,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Finish {
    pub request_id: String,
    pub secret: String,
    #[serde(default)]
    pub completion: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum Progress {
    Pending,
    Failed,
    Linked,
    Ready { reauthorize: bool, expires_at: u64 },
}
