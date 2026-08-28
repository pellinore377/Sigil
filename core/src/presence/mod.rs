//! Simplified sliding sync does not carry presence, so this polls `GET /presence/{user}/status`
//! for the users the UI shows. "Busy" is not a server state (MSC3026 never landed) — it is
//! derived from live MatrixRTC membership.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use matrix_sdk::Client;
use ruma::api::client::presence::{get_presence, set_presence};
use ruma::presence::PresenceState;
use ruma::OwnedUserId;
use serde_json::{json, Value};
use tracing::debug;

use crate::engine::SharedEngine;

const POLL_EVERY: Duration = Duration::from_secs(30);
/// Presence costs one request per user, so never sweep a huge room.
const MAX_USERS: usize = 60;

fn state_name(s: &PresenceState) -> &'static str {
    if *s == PresenceState::Online {
        "online"
    } else if *s == PresenceState::Unavailable {
        // The spec's "unavailable" is what every client draws as idle/away.
        "away"
    } else {
        "offline"
    }
}

/// Direct-chat peers, plus the members of whatever room is on screen.
async fn users_of_interest(engine: &SharedEngine, client: &Client) -> BTreeSet<OwnedUserId> {
    let me = client.user_id().map(|u| u.to_owned());
    let open: BTreeSet<String> = engine.state.lock().timelines.map.keys().cloned().collect();
    let mut out: BTreeSet<OwnedUserId> = BTreeSet::new();
    for room in client.joined_rooms() {
        let is_open = open.contains(room.room_id().as_str());
        let targets = room.direct_targets();
        if !targets.is_empty() {
            for u in targets {
                if let Ok(uid) = OwnedUserId::try_from(u.to_string()) {
                    out.insert(uid);
                }
            }
        }
        if is_open {
            if let Ok(members) = room.members_no_sync(matrix_sdk::RoomMemberships::JOIN).await {
                for m in members {
                    out.insert(m.user_id().to_owned());
                }
            }
        }
        if out.len() >= MAX_USERS {
            break;
        }
    }
    if let Some(me) = me {
        out.remove(&me);
    }
    out.into_iter().take(MAX_USERS).collect()
}

/// Everyone holding a live MatrixRTC membership anywhere we can see.
async fn busy_users(client: &Client) -> BTreeSet<OwnedUserId> {
    let mut out = BTreeSet::new();
    for room in client.joined_rooms() {
        for (uid, _device) in crate::rtc::signaling::other_active_members(&room).await {
            out.insert(uid);
        }
    }
    out
}

async fn poll_once(engine: &SharedEngine, client: &Client) {
    let busy = busy_users(client).await;
    let users = users_of_interest(engine, client).await;
    let mut map: BTreeMap<String, Value> = BTreeMap::new();
    for uid in users {
        let req = get_presence::v3::Request::new(uid.clone());
        // 403 = presence withheld, 404 = unknown user; say nothing either way.
        let Ok(resp) = client.send(req).await else { continue };
        map.insert(
            uid.to_string(),
            json!({
                "state": state_name(&resp.presence),
                "busy": busy.contains(&uid),
                "currentlyActive": resp.currently_active.unwrap_or(false),
                "lastActiveAgo": resp.last_active_ago.map(|d| d.as_millis() as u64),
                "statusMsg": resp.status_msg.unwrap_or_default(),
            }),
        );
    }
    // The server never reports us as busy; we know that locally.
    if let Some(me) = client.user_id() {
        let mine = if engine.rtc.in_call() { "busy" } else { "online" };
        map.insert(
            me.to_string(),
            json!({"state": "online", "busy": mine == "busy", "currentlyActive": true, "lastActiveAgo": 0, "statusMsg": ""}),
        );
    }
    let payload = json!({"event": "presence.list", "users": map});
    engine.state.lock().presence_snapshot = payload.clone();
    engine.hub.broadcast(payload);
}

/// Tell the server we are here; it ages back to `unavailable` once we stop.
async fn publish_self(client: &Client) {
    let Some(me) = client.user_id() else { return };
    let req = set_presence::v3::Request::new(me.to_owned(), PresenceState::Online);
    if let Err(e) = client.send(req).await {
        debug!("presence: could not publish our own state: {e}");
    }
}

pub fn start(engine: SharedEngine, client: Client) {
    tokio::spawn(async move {
        loop {
            publish_self(&client).await;
            poll_once(&engine, &client).await;
            tokio::time::sleep(POLL_EVERY).await;
        }
    });
}

/// Re-poll now, so a freshly opened room's dots are not `POLL_EVERY` late.
pub fn refresh(engine: &SharedEngine) {
    let Some(client) = engine.client() else { return };
    let engine = engine.clone();
    tokio::spawn(async move {
        poll_once(&engine, &client).await;
    });
}
