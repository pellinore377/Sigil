//! Per-participant frame keys exchanged over Olm-encrypted to-device events.
use std::collections::HashMap;

use base64::Engine;
use matrix_sdk::encryption::identities::Device;
use matrix_sdk::{Client, Room, RoomMemberships};
use matrix_sdk_base::crypto::CollectStrategy;
use ruma::events::AnyToDeviceEventContent;
use ruma::UserId;
use tracing::{debug, warn};

pub struct KeyState {
    pub own_key: Vec<u8>,
    pub own_index: u8,
    /// Peer keys keyed by Matrix user id → index → key.
    pub peer: HashMap<String, HashMap<i32, Vec<u8>>>,
}

impl KeyState {
    pub fn new() -> Self {
        KeyState { own_key: gen_key(), own_index: 0, peer: HashMap::new() }
    }
    pub fn rotate(&mut self) -> (u8, Vec<u8>) {
        self.own_key = gen_key();
        self.own_index = self.own_index.wrapping_add(1);
        (self.own_index, self.own_key.clone())
    }
    pub fn own_b64(&self) -> String {
        base64::engine::general_purpose::STANDARD.encode(&self.own_key)
    }
}

fn gen_key() -> Vec<u8> {
    use rand::RngCore;
    let mut k = vec![0u8; 32];
    rand::rng().fill_bytes(&mut k);
    k
}

pub fn decode_key(b64: &str) -> Option<Vec<u8>> {
    let e = base64::engine::general_purpose::STANDARD;
    e.decode(b64).ok().or_else(|| base64::engine::general_purpose::STANDARD_NO_PAD.decode(b64).ok()).filter(|k| !k.is_empty())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

/// Send our current key to every device of `user`.
pub async fn send_key_to_user(client: &Client, room_id: &str, identity: &str, key_b64: &str, index: u8, user: &UserId) -> anyhow::Result<()> {
    let our_device = client.device_id().map(|d| d.to_string()).unwrap_or_default();
    let content = serde_json::json!({
        "keys": { "key": key_b64, "index": index },
        "room_id": room_id,
        "member": { "claimed_device_id": our_device, "id": identity },
        "session": { "call_id": "", "application": "m.call", "scope": "m.room" },
        "sent_ts": now_ms(),
    });
    let raw: ruma::serde::Raw<AnyToDeviceEventContent> = ruma::serde::Raw::from_json(serde_json::value::to_raw_value(&content)?);
    let mut ud = client.encryption().get_user_devices(user).await?;
    if ud.devices().next().is_none() {
        let _ = client.encryption().request_user_identity(user).await;
        ud = client.encryption().get_user_devices(user).await?;
    }
    let devices: Vec<Device> = ud.devices().collect();
    if devices.is_empty() {
        warn!("e2ee: no devices for a member; key not delivered");
        return Ok(());
    }
    let refs: Vec<&Device> = devices.iter().collect();
    let failures = client.encryption().encrypt_and_send_raw_to_device(refs, "io.element.call.encryption_keys", raw, CollectStrategy::AllDevices).await?;
    if !failures.is_empty() {
        warn!("e2ee: key delivery failed for {} device(s)", failures.len());
    } else {
        debug!("e2ee: key index {index} sent to {user}");
    }
    Ok(())
}

/// Send our key to every joined member of the room (except us).
pub async fn broadcast_key(client: &Client, room: &Room, identity: &str, key_b64: &str, index: u8) {
    let Ok(members) = room.members(RoomMemberships::JOIN).await else { return };
    let own = room.own_user_id();
    for m in members {
        if m.user_id() == own { continue; }
        if let Err(e) = send_key_to_user(client, room.room_id().as_str(), identity, key_b64, index, m.user_id()).await {
            warn!("e2ee: key broadcast to a member failed: {e:#}");
        }
    }
}
