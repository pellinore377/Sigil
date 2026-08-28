//! Room and space settings: hierarchy, room state, notification mode, power levels.
//! A space IS a room (`m.space` in its creation content), so every request works on both.
use ruma::events::room::history_visibility::{HistoryVisibility, RoomHistoryVisibilityEventContent};
use ruma::events::room::join_rules::{JoinRule, RoomJoinRulesEventContent};
use ruma::events::room::power_levels::RoomPowerLevelsEventContent;
use ruma::events::{StateEventType, TimelineEventType};
use ruma::{Int, OwnedUserId, RoomId, UserId};
use serde_json::{json, Value};

use crate::engine::SharedEngine;
use crate::ipc::wire::Reply;

fn room_of(engine: &SharedEngine, id: &str) -> Result<matrix_sdk::Room, Reply> {
    let client = engine.client().ok_or_else(|| Reply::err("not_logged_in", "not logged in"))?;
    let rid = RoomId::parse(id).map_err(|_| Reply::err("bad_request", "invalid roomId"))?;
    client.get_room(&rid).ok_or_else(|| Reply::err("unknown_room", format!("unknown room {id}")))
}

fn str_of(p: &serde_json::Map<String, Value>, k: &str) -> String {
    p.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string()
}

/// The CLI passes every parameter as a string, so accept both forms.
fn i64_of_param(p: &serde_json::Map<String, Value>, k: &str) -> Option<i64> {
    let v = p.get(k)?;
    v.as_i64().or_else(|| v.as_str()?.trim().parse().ok())
}

fn u64_of_param(p: &serde_json::Map<String, Value>, k: &str) -> Option<u64> {
    let v = p.get(k)?;
    v.as_u64().or_else(|| v.as_str()?.trim().parse().ok())
}

fn bool_of_param(p: &serde_json::Map<String, Value>, k: &str) -> Option<bool> {
    let v = p.get(k)?;
    v.as_bool().or_else(|| match v.as_str()?.trim() { "true" => Some(true), "false" => Some(false), _ => None })
}

/// Join rules keep their Matrix wire strings; the UI speaks the same ones.
fn join_rule_name(r: &JoinRule) -> &'static str {
    match r {
        JoinRule::Public => "public",
        JoinRule::Invite => "invite",
        JoinRule::Knock => "knock",
        JoinRule::Restricted(_) => "restricted",
        JoinRule::KnockRestricted(_) => "knock_restricted",
        JoinRule::Private => "private",
        _ => "invite",
    }
}

fn history_name(h: &HistoryVisibility) -> &'static str {
    match h {
        HistoryVisibility::WorldReadable => "world_readable",
        HistoryVisibility::Shared => "shared",
        HistoryVisibility::Invited => "invited",
        HistoryVisibility::Joined => "joined",
        _ => "shared",
    }
}

fn history_from(name: &str) -> Option<HistoryVisibility> {
    Some(match name {
        "world_readable" => HistoryVisibility::WorldReadable,
        "shared" => HistoryVisibility::Shared,
        "invited" => HistoryVisibility::Invited,
        "joined" => HistoryVisibility::Joined,
        _ => return None,
    })
}

fn i64_of(i: Int) -> i64 {
    i64::from(i)
}

/// Every child of a space: an unjoined one is not in `rooms.list`, so only the hierarchy endpoint names it.
pub async fn hierarchy(engine: SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    use ruma::api::client::space::get_hierarchy;
    let Some(client) = engine.client() else { return Reply::err("not_logged_in", "not logged in") };
    let space_id = str_of(p, "spaceId");
    let Ok(sid) = RoomId::parse(&space_id) else { return Reply::err("bad_request", "invalid spaceId") };

    let mut req = get_hierarchy::v1::Request::new(sid.clone());
    let limit = u64_of_param(p, "limit").unwrap_or(100);
    req.limit = ruma::UInt::try_from(limit).ok();
    let resp = match client.send(req).await {
        Ok(r) => r,
        Err(e) => return Reply::err("network", e.to_string()),
    };

    let mut out = Vec::new();
    for chunk in resp.rooms {
        let s = &chunk.summary;
        let id = s.room_id.to_string();
        // The space is the first entry of its own hierarchy: the header, not a row.
        if id == space_id { continue }
        let avatar_url = s.avatar_url.as_ref().map(|u| u.to_string()).unwrap_or_default();
        let joined = client
            .get_room(&s.room_id)
            .map(|r| r.state() == matrix_sdk::RoomState::Joined)
            .unwrap_or(false);
        out.push(json!({
            "id": id,
            "name": s.name.clone().unwrap_or_default(),
            "topic": s.topic.clone().unwrap_or_default(),
            "avatarUrl": avatar_url,
            "avatarPath": crate::media::cached_avatar_path(&engine, &avatar_url).await,
            "memberCount": u64::from(s.num_joined_members),
            "isSpace": s.room_type.as_ref().map(|t| *t == ruma::room::RoomType::Space).unwrap_or(false),
            "canonicalAlias": s.canonical_alias.as_ref().map(|a| a.to_string()).unwrap_or_default(),
            "worldReadable": s.world_readable,
            "encrypted": s.encryption.is_some(),
            "joined": joined,
        }));
    }
    Reply::ok(json!({"spaceId": space_id, "rooms": out, "nextBatch": resp.next_batch}))
}

/// Everything the settings pages read, in one round trip: per-page fetches could disagree.
pub async fn settings(engine: SharedEngine, room_id: String) -> Reply {
    let room = match room_of(&engine, &room_id) { Ok(r) => r, Err(e) => return e };
    let Some(client) = engine.client() else { return Reply::err("not_logged_in", "not logged in") };
    let me = client.user_id().map(|u| u.to_owned());

    let pl = room.power_levels_or_default().await;
    let my_level = me
        .as_ref()
        .map(|u| pl.users.get(u).copied().map(i64_of).unwrap_or_else(|| i64_of(pl.users_default)))
        .unwrap_or(0);

    // The user-defined mode only; `null` is a real third state: follow the account default.
    let mode = client
        .notification_settings()
        .await
        .get_user_defined_room_notification_mode(room.room_id())
        .await
        .map(|m| match m {
            matrix_sdk_base::notification_settings::RoomNotificationMode::AllMessages => "all",
            matrix_sdk_base::notification_settings::RoomNotificationMode::MentionsAndKeywordsOnly => "mentions",
            matrix_sdk_base::notification_settings::RoomNotificationMode::Mute => "mute",
        });

    let avatar_url = room.avatar_url().map(|u| u.to_string()).unwrap_or_default();
    let join_rule = room.join_rule().map(|r| join_rule_name(&r).to_string()).unwrap_or_else(|| "invite".into());

    let ev = |t: TimelineEventType| -> i64 {
        pl.events.get(&t).copied().map(i64_of).unwrap_or_else(|| i64_of(pl.state_default))
    };

    let can = me.as_ref().map(|u| json!({
        "setName":              pl.user_can_send_state(u, StateEventType::RoomName),
        "setAvatar":            pl.user_can_send_state(u, StateEventType::RoomAvatar),
        "setTopic":             pl.user_can_send_state(u, StateEventType::RoomTopic),
        "setJoinRule":          pl.user_can_send_state(u, StateEventType::RoomJoinRules),
        "setHistoryVisibility": pl.user_can_send_state(u, StateEventType::RoomHistoryVisibility),
        "setEncryption":        pl.user_can_send_state(u, StateEventType::RoomEncryption),
        "setPowerLevels":       pl.user_can_send_state(u, StateEventType::RoomPowerLevels),
        "addChildren":          pl.user_can_send_state(u, StateEventType::SpaceChild),
        "invite":               pl.user_can_invite(u),
        "kick":                 pl.user_can_kick(u),
        "ban":                  pl.user_can_ban(u),
    })).unwrap_or_else(|| json!({}));

    Reply::ok(json!({
        "id": room.room_id().to_string(),
        "name": room.name().unwrap_or_default(),
        "topic": room.topic().unwrap_or_default(),
        "avatarUrl": avatar_url,
        "avatarPath": crate::media::cached_avatar_path(&engine, &avatar_url).await,
        "isSpace": room.is_space(),
        "isEncrypted": room.encryption_state().is_encrypted(),
        "isDirect": room.is_direct().await.unwrap_or(false),
        "canonicalAlias": room.canonical_alias().map(|a| a.to_string()).unwrap_or_default(),
        "memberCount": room.joined_members_count(),
        "joinRule": join_rule,
        "historyVisibility": history_name(&room.history_visibility_or_default()),
        "notificationMode": mode,
        "myPowerLevel": my_level,
        "can": can,
        "powerLevels": {
            "usersDefault":  i64_of(pl.users_default),
            "eventsDefault": i64_of(pl.events_default),
            "stateDefault":  i64_of(pl.state_default),
            "invite":        i64_of(pl.invite),
            "kick":          i64_of(pl.kick),
            "ban":           i64_of(pl.ban),
            "redact":        i64_of(pl.redact),
            "name":          ev(TimelineEventType::RoomName),
            "avatar":        ev(TimelineEventType::RoomAvatar),
            "topic":         ev(TimelineEventType::RoomTopic),
            "liveLocation":  pl.events.get(&"org.matrix.msc3672.beacon_info".into())
                                .or_else(|| pl.events.get(&"m.beacon_info".into()))
                                .copied().map(i64_of).unwrap_or_else(|| i64_of(pl.state_default)),
            "users": pl.users.iter().map(|(u, l)| (u.to_string(), json!(i64_of(*l)))).collect::<serde_json::Map<_, _>>(),
        },
    }))
}

/// Write whichever fields were supplied; absent means "leave alone".
pub async fn set_settings(engine: SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    let room_id = str_of(p, "roomId");
    let room = match room_of(&engine, &room_id) { Ok(r) => r, Err(e) => return e };
    let mut changed: Vec<&str> = Vec::new();

    if let Some(name) = p.get("name").and_then(Value::as_str) {
        if let Err(e) = room.set_name(name.trim().to_string()).await {
            return Reply::err("forbidden", format!("could not set name: {e}"));
        }
        changed.push("name");
    }
    if let Some(topic) = p.get("topic").and_then(Value::as_str) {
        if let Err(e) = room.set_room_topic(topic.trim()).await {
            return Reply::err("forbidden", format!("could not set topic: {e}"));
        }
        changed.push("topic");
    }
    if let Some(rule) = p.get("joinRule").and_then(Value::as_str) {
        let jr = match rule {
            "public" => JoinRule::Public,
            "invite" => JoinRule::Invite,
            "knock" => JoinRule::Knock,
            // The restriction names the parent space, so the caller must supply it.
            "restricted" => {
                let parent = str_of(p, "restrictedTo");
                let Ok(pid) = RoomId::parse(&parent) else {
                    return Reply::err("bad_request", "restricted join rule needs restrictedTo")
                };
                JoinRule::Restricted(ruma::room::Restricted::new(vec![
                    ruma::room::AllowRule::room_membership(pid),
                ]))
            }
            _ => return Reply::err("bad_request", format!("unknown join rule {rule}")),
        };
        if let Err(e) = room.send_state_event(RoomJoinRulesEventContent::new(jr)).await {
            return Reply::err("forbidden", format!("could not set join rule: {e}"));
        }
        changed.push("joinRule");
    }
    if let Some(hv) = p.get("historyVisibility").and_then(Value::as_str) {
        let Some(v) = history_from(hv) else {
            return Reply::err("bad_request", format!("unknown history visibility {hv}"))
        };
        if let Err(e) = room.send_state_event(RoomHistoryVisibilityEventContent::new(v)).await {
            return Reply::err("forbidden", format!("could not set history visibility: {e}"));
        }
        changed.push("historyVisibility");
    }
    // One-way: the spec has no way to turn encryption off.
    if bool_of_param(p, "encrypted").unwrap_or(false) {
        if !room.encryption_state().is_encrypted() {
            if let Err(e) = room.enable_encryption().await {
                return Reply::err("forbidden", format!("could not enable encryption: {e}"));
            }
            changed.push("encrypted");
        }
    }
    if p.contains_key("notificationMode") {
        use matrix_sdk_base::notification_settings::RoomNotificationMode as M;
        let Some(client) = engine.client() else { return Reply::err("not_logged_in", "not logged in") };
        let ns = client.notification_settings().await;
        let res = match p.get("notificationMode").and_then(Value::as_str) {
            Some("all") => ns.set_room_notification_mode(room.room_id(), M::AllMessages).await,
            Some("mentions") => ns.set_room_notification_mode(room.room_id(), M::MentionsAndKeywordsOnly).await,
            Some("mute") => ns.set_room_notification_mode(room.room_id(), M::Mute).await,
            // null / "default" clears the per-room rule so the account default applies.
            _ => ns.delete_user_defined_room_rules(room.room_id()).await,
        };
        if let Err(e) = res {
            return Reply::err("network", format!("could not set notifications: {e}"));
        }
        changed.push("notificationMode");
    }
    engine.request_rooms_refresh();
    Reply::ok(json!({"roomId": room_id, "changed": changed}))
}

/// Upload a local file as the room avatar; an empty path removes it.
pub async fn set_avatar(engine: SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    let room_id = str_of(p, "roomId");
    let room = match room_of(&engine, &room_id) { Ok(r) => r, Err(e) => return e };
    let path = str_of(p, "path");
    if path.is_empty() {
        return match room.remove_avatar().await {
            Ok(_) => Reply::ok(json!({"roomId": room_id, "avatarUrl": ""})),
            Err(e) => Reply::err("forbidden", format!("could not remove avatar: {e}")),
        };
    }
    let data = match std::fs::read(&path) {
        Ok(d) => d,
        Err(e) => return Reply::err("bad_request", format!("cannot read {path}: {e}")),
    };
    let mime = mime_guess::from_path(&path).first_or_octet_stream();
    match room.upload_avatar(&mime, data, None).await {
        Ok(_) => {
            engine.request_rooms_refresh();
            Reply::ok(json!({"roomId": room_id}))
        }
        Err(e) => Reply::err("forbidden", format!("could not set avatar: {e}")),
    }
}

/// Change one power-levels entry: `userId` for a person, `key` for one capability.
pub async fn set_power_level(engine: SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    let room_id = str_of(p, "roomId");
    let room = match room_of(&engine, &room_id) { Ok(r) => r, Err(e) => return e };
    let Some(level) = i64_of_param(p, "level") else {
        return Reply::err("bad_request", "level is required")
    };
    let Ok(level) = Int::try_from(level) else { return Reply::err("bad_request", "level out of range") };

    let user_id = str_of(p, "userId");
    if !user_id.is_empty() {
        let Ok(uid) = UserId::parse(&user_id) else { return Reply::err("bad_request", "invalid userId") };
        let uid: OwnedUserId = uid;
        return match room.update_power_levels(vec![(&uid, level)]).await {
            Ok(_) => Reply::ok(json!({"roomId": room_id, "userId": user_id, "level": i64_of(level)})),
            Err(e) => Reply::err("forbidden", format!("could not set power level: {e}")),
        };
    }

    let key = str_of(p, "key");
    if key.is_empty() { return Reply::err("bad_request", "userId or key is required") }
    let mut pl = match room.power_levels().await {
        Ok(p) => p,
        Err(e) => return Reply::err("network", format!("could not read power levels: {e}")),
    };
    match key.as_str() {
        "invite" => pl.invite = level,
        "kick" => pl.kick = level,
        "ban" => pl.ban = level,
        "redact" => pl.redact = level,
        "eventsDefault" => pl.events_default = level,
        "stateDefault" => pl.state_default = level,
        "usersDefault" => pl.users_default = level,
        "name" => { pl.events.insert(TimelineEventType::RoomName, level); }
        "avatar" => { pl.events.insert(TimelineEventType::RoomAvatar, level); }
        "topic" => { pl.events.insert(TimelineEventType::RoomTopic, level); }
        // Element writes the unstable beacon type; set both so the bar means the same.
        "liveLocation" => {
            pl.events.insert("org.matrix.msc3672.beacon_info".into(), level);
            pl.events.insert("m.beacon_info".into(), level);
        }
        other => return Reply::err("bad_request", format!("unknown permission key {other}")),
    }
    let content = match RoomPowerLevelsEventContent::try_from(pl) {
        Ok(c) => c,
        Err(e) => return Reply::err("bad_request", format!("invalid power levels: {e}")),
    };
    match room.send_state_event(content).await {
        Ok(_) => Reply::ok(json!({"roomId": room_id, "key": key, "level": i64_of(level)})),
        Err(e) => Reply::err("forbidden", format!("could not set permission: {e}")),
    }
}
