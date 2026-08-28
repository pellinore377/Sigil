//! Room list snapshots (`rooms.list`) and spaces tree (`spaces.tree`).
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use matrix_sdk_ui::eyeball_im::Vector;
use futures_util::{pin_mut, StreamExt};
use matrix_sdk::{Client, Room, RoomState};
use matrix_sdk_base::latest_event::LatestEventValue;
use matrix_sdk_ui::room_list_service::{filters, RoomListItem};
use matrix_sdk_ui::spaces::SpaceService;
use matrix_sdk_ui::sync_service::SyncService;
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::engine::SharedEngine;

/// Space membership index: room id → parent space ids, plus the flat spaces list.
#[derive(Default, Clone)]
pub struct SpaceIndex {
    pub parents: HashMap<String, Vec<String>>,
    pub tree: Value,
}

pub fn start(engine: SharedEngine, client: Client, sync: Arc<SyncService>) {
    let e1 = engine.clone();
    let c1 = client.clone();
    tokio::spawn(async move { run_room_list(e1, c1, sync).await });
    let e2 = engine.clone();
    tokio::spawn(async move { run_spaces(e2, client).await });
}

async fn run_room_list(engine: SharedEngine, client: Client, sync: Arc<SyncService>) {
    let rls = sync.room_list_service();
    let list = match rls.all_rooms().await {
        Ok(l) => l,
        Err(e) => {
            engine.set_error(format!("room list unavailable: {e}"));
            return;
        }
    };
    let (stream, controller) = list.entries_with_dynamic_adapters(500);
    controller.set_filter(Box::new(filters::new_filter_non_left()));
    pin_mut!(stream);
    let mut rooms: Vector<RoomListItem> = Vector::new();
    let mut pages_added = 0;
    let debounce = std::time::Duration::from_millis(150);

    // Receiver for `request_rooms_refresh()`: changes the room-list stream never emits for.
    let (refresh_tx, mut refresh_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    engine.state.lock().rooms_refresh = Some(refresh_tx);

    loop {
        tokio::select! {
            next = stream.next() => {
                let Some(diffs) = next else { break };
                for d in diffs { d.apply(&mut rooms); }
            }
            got = refresh_rx.recv() => {
                if got.is_none() { break }
            }
        }
        // Keep asking for more pages until the list stops growing (all rooms).
        if rooms.len() >= 500 * (pages_added + 1) && pages_added < 20 {
            controller.add_one_page();
            pages_added += 1;
        }
        let mut more = true;
        while more {
            more = false;
            tokio::select! {
                next = stream.next() => {
                    if let Some(diffs) = next {
                        for d in diffs { d.apply(&mut rooms); }
                        more = true;
                    } else { break; }
                }
                _ = tokio::time::sleep(debounce) => {}
            }
        }
        let snapshot = build_snapshot(&engine, &client, &rooms).await;
        engine.state.lock().rooms_snapshot = snapshot.clone();
        engine.hub.broadcast(snapshot);
    }
    warn!("room list stream ended");
}

/// Rebuild the room list when the local date changes: stamps are relative ("Yesterday", "Tue").
pub async fn run_daily_refresh(engine: SharedEngine) {
    use chrono::Datelike;
    let mut day = chrono::Local::now().day();
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        let now = chrono::Local::now().day();
        if now == day { continue }
        day = now;
        let snapshot = { engine.state.lock().rooms_snapshot.clone() };
        let Some(rooms) = snapshot.get("rooms").and_then(|r| r.as_array()).cloned() else { continue };
        let now_ms = crate::timeline::fmt::now_ms();
        let refreshed: Vec<Value> = rooms.into_iter().map(|mut r| {
            let ts = r.get("lastActivityTs").and_then(Value::as_i64).unwrap_or(0);
            if let Some(o) = r.as_object_mut() {
                o.insert("stamp".into(), json!(crate::timeline::fmt::short(ts, now_ms)));
            }
            r
        }).collect();
        let out = json!({"event":"rooms.list","loaded":true,"rooms":refreshed});
        engine.state.lock().rooms_snapshot = out.clone();
        engine.hub.broadcast(out);
    }
}

async fn build_snapshot(engine: &SharedEngine, _client: &Client, rooms: &Vector<RoomListItem>) -> Value {
    let spaces = engine.state.lock().space_index.clone();
    let mut out = Vec::with_capacity(rooms.len());
    for item in rooms.iter() {
        let room: &Room = item;
        out.push(room_json(engine, room, &spaces).await);
    }
    // Pinned (the m.favourite tag) first, then highlights, unread, recency.
    out.sort_by(|a, b| {
        let key = |v: &Value| {
            (
                v["isFavourite"].as_bool().unwrap_or(false),
                v["highlights"].as_u64().unwrap_or(0) > 0,
                v["unread"].as_u64().unwrap_or(0) > 0 || v["unreadMessages"].as_u64().unwrap_or(0) > 0,
                v["lastActivityTs"].as_i64().unwrap_or(0),
            )
        };
        key(b).cmp(&key(a))
    });
    json!({"event":"rooms.list","loaded":true,"rooms":out})
}

pub async fn room_json(engine: &SharedEngine, room: &Room, spaces: &SpaceIndex) -> Value {
    let id = room.room_id().to_string();
    let name = match room.cached_display_name() {
        Some(n) => n.to_string(),
        None => room.display_name().await.map(|n| n.to_string()).unwrap_or_else(|_| id.clone()),
    };
    let state = room.state();
    let is_dm = room.is_direct().await.unwrap_or(false);
    let dm_user = room.direct_targets().into_iter().next().map(|u| u.to_string());
    let inviter = if state == RoomState::Invited {
        room.invite_details().await.ok().map(|i| i.inviter_id.to_string())
    } else {
        None
    };
    let counts = room.unread_notification_counts();
    let (last, last_ts) = latest_event_json(room);
    let mut avatar_url = room.avatar_url().map(|u| u.to_string()).unwrap_or_default();
    // DMs rarely carry a room avatar; fall back to the partner's (store-only, no network).
    if avatar_url.is_empty() && is_dm {
        if let Some(u) = dm_user.as_deref() {
            if let Ok(uid) = ruma::UserId::parse(u) {
                if let Ok(Some(m)) = room.get_member_no_sync(&uid).await {
                    if let Some(a) = m.avatar_url() { avatar_url = a.to_string(); }
                }
            }
        }
    }
    let avatar_path = crate::media::cached_avatar_path(engine, &avatar_url).await;
    json!({
        "id": id,
        "name": name,
        "topic": room.topic().unwrap_or_default(),
        "avatarUrl": avatar_url,
        "avatarPath": avatar_path,
        "canonicalAlias": room.canonical_alias().map(|a| a.to_string()).unwrap_or_default(),
        "isDm": is_dm,
        "dmUserId": dm_user,
        "isSpace": room.is_space(),
        "spaceParents": spaces.parents.get(room.room_id().as_str()).cloned().unwrap_or_default(),
        "isEncrypted": room.encryption_state().is_encrypted(),
        "isInvite": state == RoomState::Invited,
        "inviter": inviter,
        "isFavourite": room.is_favourite(),
        "isLowPriority": room.is_low_priority(),
        "joinedMembers": room.joined_members_count(),
        "unread": counts.notification_count,
        "highlights": counts.highlight_count,
        "unreadMessages": room.num_unread_messages(),
        "markedUnread": room.is_marked_unread(),
        "lastMessage": last,
        "lastActivityTs": last_ts,
        "stamp": crate::timeline::fmt::short(last_ts, crate::timeline::fmt::now_ms()),
        "hasActiveCall": room.has_active_room_call(),
        "callParticipants": room.active_room_call_participants().iter().map(|u| u.to_string()).collect::<Vec<_>>(),
    })
}

fn latest_event_json(room: &Room) -> (Value, i64) {
    let lev = room.latest_event();
    let ts = lev.timestamp().map(|t| t.0.into()).unwrap_or(0u64) as i64;
    let v = match &lev {
        LatestEventValue::None => Value::Null,
        LatestEventValue::Remote(r) => remote_latest_json(r),
        LatestEventValue::RemoteInvite { inviter, .. } => json!({"kind":"invite","sender":inviter.as_ref().map(|u| u.to_string()),"body":"Invitation"}),
        other => json!({"kind":"local","body": crate::timeline::items::local_latest_body(other)}),
    };
    (v, ts)
}

fn remote_latest_json(r: &matrix_sdk_base::latest_event::RemoteLatestEventValue) -> Value {
    crate::timeline::items::remote_latest_json(r)
}

async fn run_spaces(engine: SharedEngine, client: Client) {
    let svc = SpaceService::new(client.clone()).await;
    let (initial, stream) = svc.subscribe_to_space_filters().await;
    let _ = initial;
    pin_mut!(stream);
    loop {
        let filters = svc.space_filters().await;
        let mut parents: HashMap<String, Vec<String>> = HashMap::new();
        let mut spaces = Vec::new();
        let mut by_id: BTreeMap<String, Value> = BTreeMap::new();
        for f in &filters {
            let sid = f.space_room.room_id.to_string();
            for child in &f.descendants {
                parents.entry(child.to_string()).or_default().push(sid.clone());
            }
            let avatar_url = f.space_room.avatar_url.as_ref().map(|u| u.to_string()).unwrap_or_default();
            let avatar_path = crate::media::cached_avatar_path(&engine, &avatar_url).await;
            by_id.insert(sid.clone(), json!({
                "id": sid,
                "name": f.space_room.display_name,
                "avatarUrl": avatar_url,
                "avatarPath": avatar_path,
                "level": f.level,
                "children": f.descendants.iter().map(|r| r.to_string()).collect::<Vec<_>>(),
                "childrenCount": f.space_room.children_count,
            }));
        }
        for f in &filters {
            spaces.push(by_id[&f.space_room.room_id.to_string()].clone());
        }
        let tree = json!({"event":"spaces.tree","spaces":spaces});
        {
            let mut s = engine.state.lock();
            s.space_index = SpaceIndex { parents, tree: tree.clone() };
        }
        info!("spaces: {} entries", filters.len());
        engine.hub.broadcast(tree);
        // Re-broadcast rooms so spaceParents update.
        engine.request_rooms_refresh();
        if stream.next().await.is_none() {
            break;
        }
    }
}
