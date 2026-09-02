//! Local client state, one JSON file next to the MLS store. Secrets live
//! here in Phase 2; the engine's keystore takes them over later.

use serde::{Deserialize, Serialize};
use sigil_protocol::identity::Identity;
use sigil_protocol::names;
use std::path::{Path, PathBuf};

#[derive(Default, Serialize, Deserialize, Clone)]
pub struct Conversation {
    pub group_id: String,
    /// Usernames of the other people (not devices) in it.
    pub peers: Vec<String>,
    /// Server hosting the slots.
    pub slot_server: String,
    /// Highest slot sequence seen per epoch address (hex → seq).
    pub cursors: std::collections::BTreeMap<String, u64>,
    /// What this device sent, keyed `"<address hex>:<seq>"`. MLS cannot
    /// decrypt one's own application messages, so they are kept here.
    #[serde(default)]
    pub sent: std::collections::BTreeMap<String, String>,
}

#[derive(Default, Serialize, Deserialize)]
pub struct State {
    pub identity_seed: String,
    /// Device signing key (Ed25519 secret) for MLS leaves.
    pub device_seed: String,
    pub username: String,
    pub envoy: String,
    pub device_id: String,
    pub credential: Option<String>,
    pub tokens: Vec<String>,
    pub conversations: Vec<Conversation>,
    /// Pending requests: hex-encoded welcome events not yet accepted.
    pub requests: Vec<PendingRequest>,
    #[serde(skip)]
    pub path: PathBuf,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct PendingRequest {
    pub from: String,
    pub from_card: String,
    pub welcome: String,
    pub first_message: String,
}

impl State {
    pub fn create(path: &Path, username: &str, envoy: &str) -> anyhow::Result<State> {
        names::parse_username(username).map_err(|_| anyhow::anyhow!("bad username"))?;
        let st = State {
            identity_seed: hex::encode(rand::random::<[u8; 32]>()),
            device_seed: hex::encode(rand::random::<[u8; 32]>()),
            username: username.to_string(),
            envoy: envoy.to_string(),
            device_id: hex::encode(rand::random::<[u8; 32]>()),
            path: path.to_path_buf(),
            ..Default::default()
        };
        st.save()?;
        Ok(st)
    }
    pub fn load(path: &Path) -> anyhow::Result<State> {
        let mut st: State = serde_json::from_str(&std::fs::read_to_string(path)?)?;
        st.path = path.to_path_buf();
        Ok(st)
    }
    pub fn save(&self) -> anyhow::Result<()> {
        Ok(std::fs::write(
            &self.path,
            serde_json::to_string_pretty(self)?,
        )?)
    }
    pub fn mls_path(&self) -> PathBuf {
        self.path.with_extension("mls.json")
    }
    pub fn identity(&self) -> Identity {
        Identity::from_seed(
            &hex::decode(&self.identity_seed)
                .unwrap()
                .try_into()
                .unwrap(),
        )
    }
    pub fn device_key(&self) -> ed25519_dalek::SigningKey {
        ed25519_dalek::SigningKey::from_bytes(
            &hex::decode(&self.device_seed).unwrap().try_into().unwrap(),
        )
    }
    pub fn server(&self) -> String {
        names::parse_username(&self.username)
            .map(|(_, s)| s.to_string())
            .unwrap()
    }
    pub fn take_token(&mut self) -> anyhow::Result<Vec<u8>> {
        let t = self
            .tokens
            .pop()
            .ok_or_else(|| anyhow::anyhow!("out of tokens; run `tokens`"))?;
        self.save()?; // a token is gone the moment it leaves the wallet
        Ok(hex::decode(t)?)
    }
}
