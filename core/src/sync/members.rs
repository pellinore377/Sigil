//! Room management requests: members, join/leave/create/invite, DMs, tags, user search.
use matrix_sdk::{RoomMemberships, RoomState};
use ruma::{OwnedRoomId, OwnedRoomOrAliasId, OwnedUserId, RoomId, UserId};
use serde_json::{json, Value};

use crate::engine::SharedEngine;
use crate::ipc::wire::Reply;

fn room_of(engine: &SharedEngine, id: &str) -> Result<matrix_sdk::Room, Reply> {
    let client = engine.client().ok_or_else(|| Reply::err("not_logged_in", "not logged in"))?;
    let rid = RoomId::parse(id).map_err(|_| Reply::err("bad_request", "invalid roomId"))?;
    client.get_room(&rid).ok_or_else(|| Reply::err("unknown_room", format!("unknown room {id}")))
}

pub async fn members(engine: SharedEngine, room_id: String) -> Reply {
    let room = match room_of(&engine, &room_id) { Ok(r) => r, Err(e) => return e };
    match room.members(RoomMemberships::JOIN | RoomMemberships::INVITE).await {
        Ok(list) => {
            let mut out = Vec::with_capacity(list.len());
            for m in list {
                let avatar_url = m.avatar_url().map(|u| u.to_string()).unwrap_or_default();
                out.push(json!({
                    "userId": m.user_id().to_string(),
                    "displayName": m.display_name().map(|s| s.to_string()).unwrap_or_else(|| m.user_id().localpart().to_string()),
                    "avatarUrl": avatar_url,
                    "avatarPath": crate::media::cached_avatar_path(&engine, &avatar_url).await,
                    "powerLevel": power_level_i64(m.power_level()),
                    "membership": format!("{:?}", m.membership()).to_lowercase(),
                    "isNameAmbiguous": m.name_ambiguous(),
                }));
            }
            out.sort_by(|a, b| b["powerLevel"].as_i64().cmp(&a["powerLevel"].as_i64()).then_with(|| a["displayName"].as_str().unwrap_or("").to_lowercase().cmp(&b["displayName"].as_str().unwrap_or("").to_lowercase())));
            Reply::ok(json!({"members": out}))
        }
        Err(e) => Reply::err("network", e.to_string()),
    }
}

pub async fn join(engine: SharedEngine, id_or_alias: String) -> Reply {
    let Some(client) = engine.client() else { return Reply::err("not_logged_in", "not logged in") };
    let target: OwnedRoomOrAliasId = match id_or_alias.trim().parse() {
        Ok(t) => t,
        Err(_) => return Reply::err("bad_request", "expected a room id or alias"),
    };
    match client.join_room_by_id_or_alias(&target, &[]).await {
        Ok(room) => Reply::ok(json!({"roomId": room.room_id().to_string()})),
        Err(e) => Reply::err("network", e.to_string()),
    }
}

pub async fn leave(engine: SharedEngine, room_id: String) -> Reply {
    let room = match room_of(&engine, &room_id) { Ok(r) => r, Err(e) => return e };
    match room.leave().await {
        Ok(()) => Reply::ok(json!({})),
        Err(e) => Reply::err("network", e.to_string()),
    }
}

pub async fn invite(engine: SharedEngine, room_id: String, user: String) -> Reply {
    let room = match room_of(&engine, &room_id) { Ok(r) => r, Err(e) => return e };
    let uid: OwnedUserId = match user.trim().parse() { Ok(u) => u, Err(_) => return Reply::err("bad_request", "invalid userId") };
    match room.invite_user_by_id(&uid).await {
        Ok(()) => Reply::ok(json!({})),
        Err(e) => Reply::err("network", e.to_string()),
    }
}

pub async fn create(engine: SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    use ruma::api::client::room::{create_room::v3::{CreationContent, Request, RoomPreset}, Visibility};
    use ruma::events::room::encryption::RoomEncryptionEventContent;
    use ruma::events::InitialStateEvent;
    let Some(client) = engine.client() else { return Reply::err("not_logged_in", "not logged in") };
    let name = p.get("name").and_then(Value::as_str).unwrap_or("").trim().to_string();
    let topic = p.get("topic").and_then(Value::as_str).unwrap_or("").trim().to_string();
    let private = p.get("private").and_then(Value::as_bool).unwrap_or(true);
    let encrypted = p.get("encrypted").and_then(Value::as_bool).unwrap_or(private);
    let invites: Vec<OwnedUserId> = p.get("invite").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str()?.parse().ok()).collect()).unwrap_or_default();
    let mut req = Request::new();
    if !name.is_empty() { req.name = Some(name); }
    if !topic.is_empty() { req.topic = Some(topic); }
    req.visibility = if private { Visibility::Private } else { Visibility::Public };
    req.preset = Some(if private { RoomPreset::PrivateChat } else { RoomPreset::PublicChat });
    req.invite = invites;
    // `type: m.space` in the creation content makes a space; never encrypted. Accept a
    // string as well as a bool: the CLI passes every param as a string.
    let is_space = p.get("space")
        .and_then(|v| v.as_bool().or_else(|| v.as_str().map(|x| x == "true")))
        .unwrap_or(false);
    let mut cc = CreationContent::new();
    if is_space { cc.room_type = Some(ruma::room::RoomType::Space); }
    req.creation_content = Some(ruma::serde::Raw::new(&cc).unwrap());
    if encrypted && !is_space {
        let enc = RoomEncryptionEventContent::with_recommended_defaults();
        req.initial_state = vec![InitialStateEvent::new(ruma::events::EmptyStateKey, enc).to_raw_any()];
    }
    match client.create_room(req).await {
        Ok(room) => Reply::ok(json!({"roomId": room.room_id().to_string()})),
        Err(e) => Reply::err("network", e.to_string()),
    }
}

pub async fn create_dm(engine: SharedEngine, user: String) -> Reply {
    let Some(client) = engine.client() else { return Reply::err("not_logged_in", "not logged in") };
    let uid: OwnedUserId = match user.trim().parse() { Ok(u) => u, Err(_) => return Reply::err("bad_request", "invalid userId") };
    if let Some(room) = client.get_dm_room(&uid) {
        if room.state() == RoomState::Joined {
            return Reply::ok(json!({"roomId": room.room_id().to_string(), "existing": true}));
        }
    }
    match client.create_dm(&uid).await {
        Ok(room) => Reply::ok(json!({"roomId": room.room_id().to_string(), "existing": false})),
        Err(e) => Reply::err("network", e.to_string()),
    }
}

/// Everyone visible in a joined room, deduplicated: Synapse's user directory only indexes
/// users who share a room or are in a public room. Store-only, it runs on every keystroke.
async fn roster(engine: &SharedEngine) -> Vec<Value> {
    let Some(client) = engine.client() else { return Vec::new() };
    let me = client.user_id().map(|u| u.to_string()).unwrap_or_default();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: Vec<(bool, String, Value)> = Vec::new();
    for room in client.joined_rooms() {
        let Ok(members) = room.members_no_sync(RoomMemberships::JOIN | RoomMemberships::INVITE).await else { continue };
        for m in members {
            let id = m.user_id().to_string();
            if id == me || !seen.insert(id.clone()) { continue }
            // DM-partner is a fact about the person, not the room they were first seen in.
            let is_dm = client.get_dm_room(m.user_id()).is_some();
            let name = m.display_name().map(|s| s.to_string()).unwrap_or_else(|| m.user_id().localpart().to_string());
            let avatar_url = m.avatar_url().map(|u| u.to_string()).unwrap_or_default();
            let entry = json!({
                "userId": id,
                "displayName": name.clone(),
                "avatarUrl": avatar_url,
                "avatarPath": crate::media::cached_avatar_path(engine, &avatar_url).await,
            });
            out.push((is_dm, name.to_lowercase(), entry));
        }
    }
    // DM partners first.
    out.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    out.into_iter().map(|(_, _, v)| v).collect()
}

fn matches(entry: &Value, needle: &str) -> bool {
    let id = entry["userId"].as_str().unwrap_or("");
    entry["displayName"].as_str().unwrap_or("").to_lowercase().contains(needle)
        || id.to_lowercase().contains(needle)
}

/// `users.search {query, limit}` — an exact user id, then the local roster, then the server directory.
pub async fn search_users(engine: SharedEngine, query: String, limit: u64) -> Reply {
    let Some(client) = engine.client() else { return Reply::err("not_logged_in", "not logged in") };
    let q = query.trim();
    let cap = limit.clamp(1, 100) as usize;
    let mut users: Vec<Value> = Vec::new();
    let mut push = |v: Value, users: &mut Vec<Value>| {
        if users.len() >= cap { return }
        if users.iter().any(|x| x["userId"] == v["userId"]) { return }
        users.push(v);
    };

    if q.is_empty() {
        for e in roster(&engine).await { push(e, &mut users) }
        return Reply::ok(json!({"users": users}));
    }

    // A fully-qualified id stands alone: the person may share no room with anyone here.
    if let Ok(uid) = <&UserId>::try_from(q) {
        push(json!({"userId": uid.to_string(), "displayName": uid.localpart(), "avatarUrl": "", "avatarPath": ""}), &mut users);
    }
    let needle = q.to_lowercase();
    for e in roster(&engine).await {
        if matches(&e, &needle) { push(e, &mut users) }
    }
    // The directory reaches people no shared room knows about; its failure is not fatal.
    match client.search_users(q, limit).await {
        Ok(resp) => {
            for u in resp.results {
                let avatar_url = u.avatar_url.map(|a| a.to_string()).unwrap_or_default();
                let avatar_path = crate::media::cached_avatar_path(&engine, &avatar_url).await;
                push(json!({
                    "userId": u.user_id.to_string(),
                    "displayName": u.display_name.unwrap_or_else(|| u.user_id.localpart().to_string()),
                    "avatarUrl": avatar_url,
                    "avatarPath": avatar_path,
                }), &mut users);
            }
        }
        Err(e) => tracing::debug!("user directory search failed, local roster stands: {e}"),
    }
    Reply::ok(json!({"users": users}))
}

/// `space.addRoom` / `space.removeRoom` — `m.space.child` state on the SPACE, keyed by child room id.
pub async fn space_set_child(engine: SharedEngine, p: &serde_json::Map<String, Value>, add: bool) -> Reply {
    use ruma::events::space::child::SpaceChildEventContent;
    use ruma::events::EmptyStateKey;
    let _ = EmptyStateKey;
    let space_id = p.get("spaceId").and_then(Value::as_str).unwrap_or("");
    let room_id = p.get("roomId").and_then(Value::as_str).unwrap_or("");
    let space = match room_of(&engine, space_id) { Ok(r) => r, Err(e) => return e };
    let Ok(child) = RoomId::parse(room_id) else { return Reply::err("bad_request", "invalid roomId") };
    if !space.is_space() { return Reply::err("bad_request", "that room is not a space") }

    // `via` is what makes a child real; an entry with none means "not here".
    let result = if add {
        let mut via: Vec<ruma::OwnedServerName> = Vec::new();
        if let Some(s) = child.server_name() { via.push(s.to_owned()); }
        if let Some(s) = space.room_id().server_name() {
            if !via.iter().any(|v| v.as_str() == s.as_str()) { via.push(s.to_owned()); }
        }
        space.send_state_event_for_key(&child, SpaceChildEventContent::new(via)).await.map(|_| ())
    } else {
        // Removal needs EMPTY content, not `{"via": []}`: an empty array still reads as a child.
        space
            .send_state_event_raw("m.space.child", child.as_str(), json!({}))
            .await
            .map(|_| ())
    };
    match result {
        Ok(()) => { engine.request_rooms_refresh(); Reply::ok(json!({})) }
        Err(e) => Reply::err("network", e.to_string()),
    }
}

pub async fn set_favourite(engine: SharedEngine, room_id: String, on: bool) -> Reply {
    let room = match room_of(&engine, &room_id) { Ok(r) => r, Err(e) => return e };
    match room.set_is_favourite(on, None).await {
        // A tag change does not make the room-list stream emit on its own.
        Ok(()) => { engine.request_rooms_refresh(); Reply::ok(json!({})) }
        Err(e) => Reply::err("network", e.to_string()),
    }
}

pub async fn set_low_priority(engine: SharedEngine, room_id: String, on: bool) -> Reply {
    let room = match room_of(&engine, &room_id) { Ok(r) => r, Err(e) => return e };
    match room.set_is_low_priority(on, None).await {
        Ok(()) => { engine.request_rooms_refresh(); Reply::ok(json!({})) }
        Err(e) => Reply::err("network", e.to_string()),
    }
}

pub async fn set_unread(engine: SharedEngine, room_id: String, on: bool) -> Reply {
    let room = match room_of(&engine, &room_id) { Ok(r) => r, Err(e) => return e };
    match room.set_unread_flag(on).await { Ok(()) => Reply::ok(json!({})), Err(e) => Reply::err("network", e.to_string()) }
}

pub async fn typing(engine: SharedEngine, room_id: String, on: bool) -> Reply {
    let room = match room_of(&engine, &room_id) { Ok(r) => r, Err(e) => return e };
    match room.typing_notice(on).await { Ok(()) => Reply::ok(json!({})), Err(e) => Reply::err("network", e.to_string()) }
}

fn power_level_i64(p: ruma::events::room::power_levels::UserPowerLevel) -> i64 {
    use ruma::events::room::power_levels::UserPowerLevel;
    match p {
        UserPowerLevel::Int(i) => i64::from(i),
        UserPowerLevel::Infinite => i64::MAX,
        #[allow(unreachable_patterns)]
        _ => 0,
    }
}

pub fn parse_room_id(s: &str) -> Option<OwnedRoomId> {
    RoomId::parse(s).ok()
}
