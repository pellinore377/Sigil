#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
pub use sigil_text as text;
pub mod admin;
pub mod attachments;
pub mod contacts;
pub mod device;
pub mod discovery;
pub mod event;
pub mod federation;
pub mod file;
pub mod groups;
pub mod initial;
pub mod link;
pub mod login;
pub mod mailbox;
pub mod oidc;
pub mod prekeys;
pub mod push;
pub mod recovery;
pub mod retry;
pub mod services;

pub const ADMIN_VERSION: u16 = 0;
pub const DEFAULT_QUOTA: u64 = 10 * 1024 * 1024 * 1024;
pub const DEFAULT_ATTACHMENT_LIMIT: u64 = 1024 * 1024 * 1024;
pub const MAX_ADMIN_BODY: usize = 8 * 1024;

#[derive(Debug, Serialize)]
pub struct Capabilities {
    pub admin: &'static [u16],
    pub enrollment: &'static [u16],
    pub reauthorization: &'static [u16],
    pub prekeys: &'static [u16],
    pub device_binding: &'static [u16],
    pub device_link: &'static [u16],
    pub delivery: &'static [u16],
    pub admission: &'static [u16],
    pub contacts: &'static [u16],
    pub client: &'static [u16],
    pub federation: &'static [u16],
    pub encrypted_event: &'static [u16],
    pub recovery: &'static [u16],
    pub recovery_storage: &'static [u16],
    pub attachment_storage: &'static [u16],
    pub push: &'static [u16],
    pub maps: &'static [u16],
    pub services: &'static [u16],
}

pub const CAPABILITIES: Capabilities = Capabilities {
    admin: &[ADMIN_VERSION],
    enrollment: &[0],
    reauthorization: &[0],
    prekeys: &[0],
    device_binding: &[0],
    device_link: &[1],
    delivery: &[0],
    admission: &[0],
    contacts: &[0],
    client: &[0],
    federation: &[],
    encrypted_event: &[],
    recovery: &[],
    recovery_storage: &[0],
    attachment_storage: &[0],
    push: &[0],
    maps: &[0],
    services: &[0],
};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub server_name: String,
    #[serde(default = "default_quota")]
    pub default_quota_bytes: u64,
    #[serde(default = "default_attachment_limit")]
    pub max_attachment_bytes: u64,
}

fn default_quota() -> u64 {
    DEFAULT_QUOTA
}
fn default_attachment_limit() -> u64 {
    DEFAULT_ATTACHMENT_LIMIT
}

impl Settings {
    pub fn validate(&self) -> Result<(), &'static str> {
        let name = &self.server_name;
        if !valid_server_name(name) {
            return Err(
                "server_name must be a lowercase ASCII DNS name without a scheme, port, or path",
            );
        }
        if !(1024 * 1024..=1024_u64.pow(5)).contains(&self.default_quota_bytes) {
            return Err("default_quota_bytes must be between 1 MiB and 1 PiB");
        }
        if self.max_attachment_bytes == 0 || self.max_attachment_bytes > self.default_quota_bytes {
            return Err("max_attachment_bytes must be positive and no larger than the quota");
        }
        Ok(())
    }
}

pub fn valid_server_name(name: &str) -> bool {
    name.len() <= 253
        && name.contains('.')
        && name.parse::<std::net::IpAddr>().is_err()
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Configure {
    pub expected_revision: u64,
    pub settings: Settings,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub revision: u64,
    pub settings: Option<Settings>,
}

#[derive(Serialize)]
pub struct ApiError {
    pub code: &'static str,
    pub message: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_ambiguous_hosts_and_invalid_storage_limits() {
        let mut settings = Settings {
            server_name: "chat.example".into(),
            default_quota_bytes: DEFAULT_QUOTA,
            max_attachment_bytes: DEFAULT_ATTACHMENT_LIMIT,
        };
        assert!(settings.validate().is_ok());
        for name in [
            "",
            "localhost",
            "https://chat.example",
            "chat.example:443",
            "a..example",
            "-a.example",
            "a_.example",
            "A.example",
            "a.example.",
            "127.0.0.1",
            "a/evil.example",
            "é.example",
        ] {
            settings.server_name = name.into();
            assert!(settings.validate().is_err(), "{name}");
        }
        settings.server_name = "chat.example".into();
        settings.max_attachment_bytes = DEFAULT_QUOTA + 1;
        assert!(settings.validate().is_err());
        settings.max_attachment_bytes = 0;
        assert!(settings.validate().is_err());
    }
}

pub mod accounts;
pub mod conversation;
pub mod profile;
