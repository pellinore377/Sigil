//! One TOML file, few keys, all with defaults.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// `home`, `envoy`, or `both`.
    pub role: String,
    /// Public hostname, the `server` half of every `@name:server` here.
    pub hostname: String,
    pub listen: String,
    pub data_dir: PathBuf,
    /// PEM paths. Absent means plain HTTP, which is for local testing only.
    pub tls_cert: Option<PathBuf>,
    pub tls_key: Option<PathBuf>,
    /// `invite` (default) or `open`.
    pub registration: String,
    /// Daily tokens per credential.
    pub tokens_per_day: u32,
    /// Hostname → base URL overrides for the Envoy role, for testing
    /// (`"sigil.example" = "http://127.0.0.1:8080"`). Default is `https://<hostname>`.
    pub servers: BTreeMap<String, String>,
    /// Envoy: seconds a bag is held before forwarding, upper bound of the jitter.
    pub jitter_max_ms: u64,
    /// Envoy: the clocked tier. Dummy bags per minute sent to each server
    /// this Envoy talks to, indistinguishable from real ones. 0 = off.
    pub cover_per_minute: u32,
    /// Home: grant authenticated Envoys a token credential for cover traffic.
    pub cover_credentials: bool,
    /// Home: run the call forwarding unit. Media arrives on `media_udp`.
    pub calls: bool,
    /// UDP address the forwarding unit binds. One port carries every call.
    pub media_udp: String,
    /// The address participants are told to send media to, when it differs
    /// from `media_udp` (a public IP in front of the container). Default is
    /// the hostname's address at the `media_udp` port.
    pub media_public: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            role: "both".into(),
            hostname: "localhost.localdomain".into(),
            listen: "0.0.0.0:8443".into(),
            data_dir: PathBuf::from("./data"),
            tls_cert: None,
            tls_key: None,
            registration: "invite".into(),
            tokens_per_day: 2000,
            servers: BTreeMap::new(),
            jitter_max_ms: 2000,
            cover_per_minute: 0,
            cover_credentials: true,
            calls: true,
            media_udp: "0.0.0.0:8444".into(),
            media_public: None,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Config> {
        let s = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&s)?)
    }
    pub fn is_home(&self) -> bool {
        self.role == "home" || self.role == "both"
    }
    pub fn is_envoy(&self) -> bool {
        self.role == "envoy" || self.role == "both"
    }
    pub fn base_url(&self, server: &str) -> String {
        self.servers
            .get(server)
            .cloned()
            .unwrap_or_else(|| format!("https://{server}"))
    }
}
