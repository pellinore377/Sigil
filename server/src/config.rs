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
    /// `invite` (default), `open`, or `oidc` (an ID token from `oidc_issuer`
    /// takes the place of the invite code).
    pub registration: String,
    /// OIDC gate: the issuer URL (Pocket ID, Authentik, Keycloak, …) and the
    /// client id registered there for Sigil. Both are needed for `oidc`.
    pub oidc_issuer: Option<String>,
    pub oidc_client_id: Option<String>,
    /// Where this server actually answers when that is not `hostname`
    /// (`sigil.example.com` for names `@…:example.com`). Served at
    /// `/.well-known/sigil`, which the bare domain must forward here.
    pub advertise: Option<String>,
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
            oidc_issuer: None,
            oidc_client_id: None,
            advertise: None,
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
        let mut cfg: Config = toml::from_str(&s)?;
        cfg.apply_env();
        Ok(cfg)
    }

    /// `SIGIL_*` variables win over the file, so a container can be
    /// configured from its environment alone (see server/docker-compose.yml).
    pub fn apply_env(&mut self) {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        let opt = |k: &str| std::env::var(k).ok().map(|v| v.trim().to_string());
        if let Some(v) = var("SIGIL_ROLE") {
            self.role = v;
        }
        if let Some(v) = var("SIGIL_HOSTNAME") {
            self.hostname = v;
        }
        if let Some(v) = var("SIGIL_LISTEN") {
            self.listen = v;
        }
        if let Some(v) = var("SIGIL_REGISTRATION") {
            self.registration = v;
        }
        // an empty value clears these, so a proxy or the gate can be turned off
        if let Some(v) = opt("SIGIL_OIDC_ISSUER") {
            self.oidc_issuer = Some(v).filter(|v| !v.is_empty());
        }
        if let Some(v) = opt("SIGIL_OIDC_CLIENT_ID") {
            self.oidc_client_id = Some(v).filter(|v| !v.is_empty());
        }
        if let Some(v) = opt("SIGIL_ADVERTISE") {
            self.advertise = Some(v).filter(|v| !v.is_empty());
        }
        if let Some(v) = opt("SIGIL_TLS_CERT") {
            self.tls_cert = Some(v).filter(|v| !v.is_empty()).map(PathBuf::from);
        }
        if let Some(v) = opt("SIGIL_TLS_KEY") {
            self.tls_key = Some(v).filter(|v| !v.is_empty()).map(PathBuf::from);
        }
        if let Some(v) = opt("SIGIL_MEDIA_PUBLIC") {
            self.media_public = Some(v).filter(|v| !v.is_empty());
        }
        if let Some(v) = var("SIGIL_MEDIA_UDP") {
            self.media_udp = v;
        }
        if let Some(v) = var("SIGIL_CALLS") {
            self.calls = matches!(v.trim(), "1" | "true" | "yes" | "on");
        }
        if let Some(v) = var("SIGIL_TOKENS_PER_DAY").and_then(|v| v.parse().ok()) {
            self.tokens_per_day = v;
        }
        if let Some(v) = var("SIGIL_JITTER_MAX_MS").and_then(|v| v.parse().ok()) {
            self.jitter_max_ms = v;
        }
        if let Some(v) = var("SIGIL_COVER_PER_MINUTE").and_then(|v| v.parse().ok()) {
            self.cover_per_minute = v;
        }
    }

    /// Where the gate is misconfigured, say so before anyone registers.
    pub fn check(&self) -> anyhow::Result<()> {
        match self.registration.as_str() {
            "invite" | "open" => Ok(()),
            "oidc" => {
                if self.oidc_issuer.is_none() || self.oidc_client_id.is_none() {
                    anyhow::bail!("registration = \"oidc\" needs oidc_issuer and oidc_client_id (SIGIL_OIDC_ISSUER and SIGIL_OIDC_CLIENT_ID)");
                }
                Ok(())
            }
            other => anyhow::bail!("registration = \"{other}\" is not one of invite, open, oidc"),
        }
    }
    pub fn is_home(&self) -> bool {
        self.role == "home" || self.role == "both"
    }
    pub fn is_envoy(&self) -> bool {
        self.role == "envoy" || self.role == "both"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_environment_wins_over_the_file_and_empty_clears() {
        let mut cfg: Config = toml::from_str(
            "hostname = \"file.example\"\nregistration = \"invite\"\nmedia_public = \"1.2.3.4:8444\"\n",
        )
        .unwrap();
        std::env::set_var("SIGIL_HOSTNAME", "env.example");
        std::env::set_var("SIGIL_REGISTRATION", "oidc");
        std::env::set_var("SIGIL_OIDC_ISSUER", "https://id.example/");
        std::env::set_var("SIGIL_OIDC_CLIENT_ID", "sigil");
        std::env::set_var("SIGIL_MEDIA_PUBLIC", "");
        std::env::set_var("SIGIL_CALLS", "false");
        cfg.apply_env();
        assert_eq!(cfg.hostname, "env.example");
        assert_eq!(cfg.registration, "oidc");
        assert_eq!(cfg.oidc_issuer.as_deref(), Some("https://id.example/"));
        assert_eq!(cfg.oidc_client_id.as_deref(), Some("sigil"));
        assert_eq!(cfg.media_public, None, "an empty value clears");
        assert!(!cfg.calls);
        assert!(cfg.check().is_ok());
        cfg.oidc_client_id = None;
        assert!(cfg.check().is_err(), "oidc without a client id is refused");
        for k in [
            "SIGIL_HOSTNAME",
            "SIGIL_REGISTRATION",
            "SIGIL_OIDC_ISSUER",
            "SIGIL_OIDC_CLIENT_ID",
            "SIGIL_MEDIA_PUBLIC",
            "SIGIL_CALLS",
        ] {
            std::env::remove_var(k);
        }
    }
}
