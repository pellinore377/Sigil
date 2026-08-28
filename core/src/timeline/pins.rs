//! Pinned messages, as `m.room.pinned_events` room state. Only the id list lives here;
//! reading the events is `timeline::open_pins`.

use ruma::EventId;
use serde_json::{json, Map, Value};

use crate::engine::SharedEngine;
use crate::ipc::wire::Reply;

fn room_of(engine: &SharedEngine, id: &str) -> Result<matrix_sdk::Room, Reply> {
    let client = engine.client().ok_or_else(|| Reply::err("not_logged_in", "not logged in"))?;
    let rid = ruma::RoomId::parse(id).map_err(|_| Reply::err("bad_request", "invalid roomId"))?;
    client.get_room(&rid).ok_or_else(|| Reply::err("unknown_room", format!("unknown room {id}")))
}

/// Everyone's pinned ids, newest last — the state event's own order, as other clients show it.
pub async fn ids(engine: &SharedEngine, room_id: &str) -> Vec<String> {
    let Ok(room) = room_of(engine, room_id) else { return Vec::new() };
    if let Some(v) = room.pinned_event_ids() {
        return v.into_iter().map(|e| e.to_string()).collect();
    }
    room.load_pinned_events()
        .await
        .ok()
        .flatten()
        .map(|v| v.into_iter().map(|e| e.to_string()).collect())
        .unwrap_or_default()
}

/// `pins.items {roomId}`, newest first. Not `TimelineFocus::PinnedEvents`: its
/// `load_pinned_events()` requests a trailing slash some Synapse versions 404, and cannot paginate.
pub async fn items(engine: SharedEngine, room_id: String) -> Reply {
    use ruma::events::AnySyncTimelineEvent;
    let room = match room_of(&engine, &room_id) { Ok(r) => r, Err(e) => return e };
    let mut out = Vec::new();
    for id in ids(&engine, &room_id).await {
        let Ok(eid) = EventId::parse(id.as_str()) else { continue };
        let Ok(ev) = room.event(&eid, None).await else { continue };
        let Ok(any) = ev.raw().deserialize() else { continue };
        let sender = any.sender().to_owned();
        let member = room.get_member_no_sync(&sender).await.ok().flatten();
        let name = member
            .as_ref()
            .and_then(|m| m.display_name().map(|s| s.to_string()))
            .unwrap_or_else(|| sender.localpart().to_string());
        let mxc = member.as_ref().and_then(|m| m.avatar_url().map(|a| a.to_string())).unwrap_or_default();
        let (kind, body) = match &any {
            AnySyncTimelineEvent::MessageLike(m) => describe(m),
            // A pinned live-location share is a state event, so state events must be named or it draws empty.
            AnySyncTimelineEvent::State(st) => {
                let t = st.event_type().to_string();
                if t.contains("beacon_info") { ("liveLocation", "Live location".to_string()) }
                else { ("state", pretty_state(&t)) }
            }
        };
        out.push(json!({
            "eventId": id,
            "sender": sender.to_string(),
            "senderName": name,
            "avatarPath": crate::media::cached_avatar_path(&engine, &mxc).await,
            "kind": kind,
            "body": if body.trim().is_empty() { "Message".to_string() } else { body },
            "ts": u64::from(any.origin_server_ts().0),
            "isOwn": engine.client().and_then(|c| c.user_id().map(|u| u == sender)).unwrap_or(false),
        }));
    }
    // Newest first, matching the room's own ordering.
    out.sort_by_key(|v| std::cmp::Reverse(v["ts"].as_u64().unwrap_or(0)));
    Reply::ok(json!({ "roomId": room_id, "items": out }))
}

/// `org.matrix.msc3672.beacon_info` -> "Beacon info".
fn pretty_state(t: &str) -> String {
    let tail = t.rsplit('.').next().unwrap_or(t).replace('_', " ");
    let mut c = tail.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => "Room event".to_string(),
    }
}

/// What a pinned event is, in one word and one line.
fn describe(m: &ruma::events::AnySyncMessageLikeEvent) -> (&'static str, String) {
    use ruma::events::room::message::MessageType;
    use ruma::events::AnySyncMessageLikeEvent as E;
    match m {
        E::RoomMessage(ev) => match ev.as_original() {
            Some(o) => match &o.content.msgtype {
                MessageType::Text(t) => ("text", t.body.clone()),
                MessageType::Emote(t) => ("emote", t.body.clone()),
                MessageType::Notice(t) => ("notice", t.body.clone()),
                MessageType::Image(i) => ("image", if i.body.is_empty() { "Photo".into() } else { i.body.clone() }),
                MessageType::Video(v) => ("video", if v.body.is_empty() { "Video".into() } else { v.body.clone() }),
                MessageType::Audio(a) => ("audio", if a.body.is_empty() { "Audio".into() } else { a.body.clone() }),
                MessageType::File(f) => ("file", if f.body.is_empty() { "File".into() } else { f.body.clone() }),
                MessageType::Location(l) => ("location", if l.body.is_empty() { "Location".into() } else { l.body.clone() }),
                _ => ("text", String::new()),
            },
            None => ("redacted", "Message deleted".into()),
        },
        E::Sticker(_) => ("sticker", "Sticker".into()),
        _ => ("text", String::new()),
    }
}

/// `pins.list {roomId}`
pub async fn list(engine: SharedEngine, room_id: String) -> Reply {
    Reply::ok(json!({ "roomId": room_id, "events": ids(&engine, &room_id).await }))
}

/// Tell every client the set changed, so badges update without anyone reopening anything.
async fn broadcast(engine: &SharedEngine, room_id: &str) {
    engine.hub.broadcast(json!({
        "event": "room.pinned", "roomId": room_id, "events": ids(engine, room_id).await
    }));
}

/// `message.pin {roomId, eventId}`
pub async fn pin(engine: SharedEngine, room_id: String, p: &Map<String, Value>) -> Reply {
    let event_id = p.get("eventId").and_then(Value::as_str).unwrap_or("");
    let Ok(eid) = EventId::parse(event_id) else { return Reply::err("bad_request", "invalid eventId") };
    let room = match room_of(&engine, &room_id) { Ok(r) => r, Err(e) => return e };
    match room.pin_event(&eid).await {
        // `false` means already pinned, which is the state the caller wanted.
        Ok(_) => { broadcast(&engine, &room_id).await; Reply::ok(json!({"pinned": true})) }
        Err(e) => Reply::err("network", e.to_string()),
    }
}

/// `message.unpin {roomId, eventId}`
pub async fn unpin(engine: SharedEngine, room_id: String, p: &Map<String, Value>) -> Reply {
    let event_id = p.get("eventId").and_then(Value::as_str).unwrap_or("");
    let Ok(eid) = EventId::parse(event_id) else { return Reply::err("bad_request", "invalid eventId") };
    let room = match room_of(&engine, &room_id) { Ok(r) => r, Err(e) => return e };
    match room.unpin_event(&eid).await {
        Ok(_) => { broadcast(&engine, &room_id).await; Reply::ok(json!({"pinned": false})) }
        Err(e) => Reply::err("network", e.to_string()),
    }
}
