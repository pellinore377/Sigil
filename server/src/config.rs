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
