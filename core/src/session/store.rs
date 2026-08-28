//! session.json / store.key persistence (0600, atomic rename).
use std::path::Path;

use anyhow::Context;
use matrix_sdk::authentication::oauth::{ClientId, OAuthSession, UserSession};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct SavedSession {
    pub homeserver: String,
    pub client_id: String,
    pub user: UserSession,
}

impl SavedSession {
    pub fn from_oauth(homeserver: String, s: OAuthSession) -> Self {
        SavedSession { homeserver, client_id: s.client_id.as_str().to_owned(), user: s.user }
    }
    pub fn into_oauth(self) -> OAuthSession {
        OAuthSession { client_id: ClientId::new(self.client_id), user: self.user }
    }
}

fn write_private(path: &Path, data: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = path.with_extension("tmp");
    {
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn store_key(state_dir: &Path) -> anyhow::Result<String> {
    let p = state_dir.join("store.key");
    if let Ok(s) = std::fs::read_to_string(&p) {
        let s = s.trim().to_string();
        if !s.is_empty() {
            return Ok(s);
        }
    }
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let key = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    write_private(&p, key.as_bytes()).context("writing store.key")?;
    Ok(key)
}

pub fn session_path(state_dir: &Path) -> std::path::PathBuf {
    state_dir.join("session.json")
}

pub fn load_session(state_dir: &Path) -> anyhow::Result<Option<SavedSession>> {
    let p = session_path(state_dir);
    if !p.exists() {
        return Ok(None);
    }
    let data = std::fs::read(&p).context("reading session.json")?;
    Ok(Some(serde_json::from_slice(&data).context("parsing session.json")?))
}

pub fn save_session(state_dir: &Path, s: &SavedSession) -> anyhow::Result<()> {
    let data = serde_json::to_vec_pretty(s)?;
    write_private(&session_path(state_dir), &data).context("writing session.json")
}

pub fn remove_session(state_dir: &Path) {
    let _ = std::fs::remove_file(session_path(state_dir));
}
