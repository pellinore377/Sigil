use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Methods {
    pub server_name: String,
    pub sso: bool,
    pub password: bool,
    pub invitation: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PasswordPolicy {
    pub revision: u64,
    pub enabled: bool,
}
