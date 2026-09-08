use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    #[default]
    Member,
    Auditor,
    Operator,
    Administrator,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Registration {
    Closed,
    #[default]
    Invitations,
    Oidc,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub revision: u64,
    pub registration: Registration,
    pub max_accounts: u32,
    pub registrations_per_day: u32,
    pub public_origin: Option<String>,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            revision: 0,
            registration: Registration::Invitations,
            max_accounts: 10000,
            registrations_per_day: 100,
            public_origin: None,
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Account {
    pub id: String,
    pub username: String,
    pub revision: u64,
    pub role: Role,
    pub disabled: bool,
    #[serde(default)]
    pub deleted: bool,
    pub discoverable: bool,
    pub quota_bytes: u64,
    pub used_bytes: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AccountUpdate {
    pub expected_revision: u64,
    pub role: Role,
    pub disabled: bool,
    pub quota_bytes: Option<u64>,
    pub confirm: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AccountPage {
    pub accounts: Vec<Account>,
    pub next_after: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryPreference {
    pub revision: u64,
    pub discoverable: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FoundAccount {
    pub address: String,
    pub account: String,
    pub devices: Vec<String>,
}
impl FoundAccount {
    pub fn valid_for(&self, username: &str, server: &str) -> bool {
        self.address == format!("@{username}:{server}")
            && crate::accounts::valid_credential(&self.account)
            && self.devices.len() <= 64
            && self.devices.windows(2).all(|w| w[0] < w[1])
            && self
                .devices
                .iter()
                .all(|d| crate::accounts::valid_credential(d))
    }
}
