//! Secret storage / key backup recovery ("Security Key" from Element).
use futures_util::StreamExt;
use matrix_sdk::encryption::{backups::BackupState, recovery::RecoveryState};
use matrix_sdk::Client;
use serde_json::{json, Value};
use tracing::warn;

use crate::engine::SharedEngine;
use crate::ipc::wire::Reply;

fn recovery_str(s: RecoveryState) -> &'static str {
    match s {
        RecoveryState::Unknown => "unknown",
        RecoveryState::Enabled => "enabled",
        RecoveryState::Disabled => "disabled",
        RecoveryState::Incomplete => "incomplete",
    }
}

fn backup_str(s: BackupState) -> &'static str {
    match s {
        BackupState::Unknown => "unknown",
        BackupState::Creating => "creating",
        BackupState::Enabling => "enabling",
        BackupState::Enabled => "enabled",
        BackupState::Downloading => "downloading",
        BackupState::Disabling => "disabling",
        #[allow(unreachable_patterns)]
        _ => "other",
    }
}

pub fn status_json(engine: &crate::engine::Engine) -> Value {
    let (rec, bak, verified) = match engine.client() {
        Some(c) => {
            let e = c.encryption();
            (recovery_str(e.recovery().state()), backup_str(e.backups().state()), engine.state.lock().verified)
        }
        None => ("unknown", "unknown", false),
    };
    json!({"event":"recovery.status","recovery":rec,"backup":bak,"verified":verified})
}

pub fn watch(engine: SharedEngine, client: Client) {
    let e1 = engine.clone();
    let c1 = client.clone();
    tokio::spawn(async move {
        let mut s = c1.encryption().recovery().state_stream();
        while let Some(_st) = s.next().await {
            refresh_verified(&e1, &c1).await;
            e1.hub.broadcast(status_json(&e1));
        }
    });
    let e2 = engine.clone();
    let c2 = client.clone();
    tokio::spawn(async move {
        let mut s = c2.encryption().backups().state_stream();
        while let Some(_st) = s.next().await {
            e2.hub.broadcast(status_json(&e2));
        }
    });
    tokio::spawn(async move {
        refresh_verified(&engine, &client).await;
        engine.hub.broadcast(status_json(&engine));
    });
}

async fn refresh_verified(engine: &SharedEngine, client: &Client) {
    let verified = match client.encryption().get_own_device().await {
        Ok(Some(d)) => d.is_verified(),
        _ => false,
    };
    let changed = {
        let mut s = engine.state.lock();
        let c = s.verified != verified;
        s.verified = verified;
        c
    };
    if changed {
        engine.broadcast_status();
    }
}

pub async fn recover(engine: SharedEngine, key: String) -> Reply {
    let Some(client) = engine.client() else { return Reply::err("not_logged_in", "not logged in") };
    let key: String = key.split_whitespace().collect();
    if key.is_empty() {
        return Reply::err("bad_request", "key is required");
    }
    match client.encryption().recovery().recover(&key).await {
        Ok(()) => {
            refresh_verified(&engine, &client).await;
            let st = status_json(&engine);
            engine.hub.broadcast(st.clone());
            Reply::ok(st)
        }
        Err(e) => {
            warn!("recovery failed: {e}");
            let msg = e.to_string();
            let code = if msg.to_lowercase().contains("key") || msg.to_lowercase().contains("mac") { "recovery_key_invalid" } else { "network" };
            Reply::err(code, msg)
        }
    }
}
