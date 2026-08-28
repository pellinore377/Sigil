//! message.* / attachment / receipts.
use matrix_sdk::room::edit::EditedContent;
use matrix_sdk_ui::timeline::TimelineEventItemId;
use ruma::events::room::message::{RoomMessageEventContent, RoomMessageEventContentWithoutRelation};
use ruma::{EventId, OwnedEventId};
use serde_json::{json, Value};

use crate::engine::SharedEngine;
use crate::ipc::wire::Reply;

fn text_content(body: &str, markdown: bool) -> RoomMessageEventContent {
    if markdown { RoomMessageEventContent::text_markdown(body) } else { RoomMessageEventContent::text_plain(body) }
}

fn text_content_nr(body: &str, markdown: bool) -> RoomMessageEventContentWithoutRelation {
    if markdown { RoomMessageEventContentWithoutRelation::text_markdown(body) } else { RoomMessageEventContentWithoutRelation::text_plain(body) }
}

fn event_id_of(p: &serde_json::Map<String, Value>) -> Result<OwnedEventId, Reply> {
    let s = p.get("eventId").and_then(Value::as_str).unwrap_or("");
    EventId::parse(s).map_err(|_| Reply::err("bad_request", "invalid eventId"))
}

/// Re-send a local echo that failed (SendHandle::unwedge), or drop it (abort).
async fn local_echo_action(engine: SharedEngine, room_id: String, p: &serde_json::Map<String, Value>, retry: bool) -> Reply {
    let Some(open) = super::get(&engine, &room_id) else { return Reply::err("bad_request", "room is not open") };
    let id = p.get("id").and_then(Value::as_str).unwrap_or("");
    let txn = p.get("txnId").and_then(Value::as_str).unwrap_or("");
    let want = if !txn.is_empty() { txn } else { id };
    if want.is_empty() { return Reply::err("bad_request", "missing txnId"); }
    let items = open.timeline.items().await;
    for it in items.iter() {
        let Some(ev) = it.as_event() else { continue };
        let Some(txn_id) = ev.transaction_id() else { continue };
        if txn_id.as_str() != want { continue; }
        let Some(handle) = ev.local_echo_send_handle() else { return Reply::err("bad_request", "no send handle") };
        return if retry {
            match handle.unwedge().await { Ok(()) => Reply::ok(json!({})), Err(e) => Reply::err("network", e.to_string()) }
        } else {
            match handle.abort().await { Ok(_) => Reply::ok(json!({})), Err(e) => Reply::err("network", e.to_string()) }
        };
    }
    Reply::err("bad_request", "message not found")
}

pub async fn retry(engine: SharedEngine, room_id: String, p: &serde_json::Map<String, Value>) -> Reply {
    local_echo_action(engine, room_id, p, true).await
}

pub async fn cancel_send(engine: SharedEngine, room_id: String, p: &serde_json::Map<String, Value>) -> Reply {
    local_echo_action(engine, room_id, p, false).await
}

/// The key Sigil's styling rides on; everything else stays ordinary Matrix for unaware clients.
pub const EFFECTS_KEY: &str = "com.sigil.text_effects";

/// A shared contact; `body` names the person and their MXID for unaware clients.
pub const CONTACT_KEY: &str = "com.sigil.contact";

/// `contact.send {roomId, userId, displayName, avatarUrl}`
pub async fn send_contact(engine: SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    let room_id = p.get("roomId").and_then(Value::as_str).unwrap_or("").to_string();
    let user_id = p.get("userId").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if user_id.is_empty() { return Reply::err("bad_request", "a contact needs a userId") }
    let name = p.get("displayName").and_then(Value::as_str).unwrap_or("").trim().to_string();
    let avatar = p.get("avatarUrl").and_then(Value::as_str).unwrap_or("").to_string();
    let shown = if name.is_empty() { user_id.clone() } else { name.clone() };

    let Some(client) = engine.client() else { return Reply::err("not_logged_in", "not logged in") };
    let Some(rid) = crate::sync::members::parse_room_id(&room_id) else {
        return Reply::err("bad_request", "invalid roomId")
    };
    let Some(room) = client.get_room(&rid) else {
        return Reply::err("unknown_room", format!("unknown room {room_id}"))
    };
    let content = json!({
        "msgtype": "m.text",
        // Element shows this verbatim, so it carries the id and stays readable.
        "body": format!("Contact: {shown} ({user_id})"),
        CONTACT_KEY: {
            "type": "matrix",
            "user_id": user_id,
            "display_name": name,
            "avatar_url": avatar,
        },
    });
    match room.send_raw("m.room.message", content).await {
        Ok(_) => Reply::ok(json!({})),
        Err(e) => Reply::err("network", e.to_string()),
    }
}

pub async fn send(engine: SharedEngine, room_id: String, p: &serde_json::Map<String, Value>) -> Reply {
    let Some(open) = super::get(&engine, &room_id) else { return Reply::err("bad_request", "room is not open") };
    let body = p.get("body").and_then(Value::as_str).unwrap_or("");
    if body.trim().is_empty() { return Reply::err("bad_request", "empty message"); }
    let markdown = p.get("markdown").and_then(Value::as_bool).unwrap_or(true);

    // Only this path needs a raw send: ruma's typed content has nowhere to hang a
    // custom field. An ordinary message keeps the timeline's local echo and send queue.
    if markdown {
        let composed = crate::timeline::effects::compose(body);
        if composed.has_effects() {
            let Some(client) = engine.client() else { return Reply::err("not_logged_in", "not logged in") };
            let Some(rid) = crate::sync::members::parse_room_id(&room_id) else {
                return Reply::err("bad_request", "invalid roomId")
            };
            let Some(room) = client.get_room(&rid) else {
                return Reply::err("unknown_room", format!("unknown room {room_id}"))
            };
            let content = json!({
                "msgtype": "m.text",
                "body": composed.body,
                "format": "org.matrix.custom.html",
                "formatted_body": composed.html,
                EFFECTS_KEY: crate::timeline::effects::to_json(&composed.effects),
            });
            return match room.send_raw("m.room.message", content).await {
                Ok(_) => Reply::ok(json!({"effects": composed.effects.len()})),
                Err(e) => Reply::err("network", e.to_string()),
            }
        }
    }

    match open.timeline.send(text_content(body, markdown).into()).await {
        Ok(_handle) => Reply::ok(json!({})),
        Err(e) => Reply::err("network", e.to_string()),
    }
}

pub async fn reply(engine: SharedEngine, room_id: String, p: &serde_json::Map<String, Value>) -> Reply {
    let Some(open) = super::get(&engine, &room_id) else { return Reply::err("bad_request", "room is not open") };
    let eid = match event_id_of(p) { Ok(e) => e, Err(r) => return r };
    let body = p.get("body").and_then(Value::as_str).unwrap_or("");
    if body.trim().is_empty() { return Reply::err("bad_request", "empty message"); }
    let markdown = p.get("markdown").and_then(Value::as_bool).unwrap_or(true);
    match open.timeline.send_reply(text_content_nr(body, markdown), eid).await {
        Ok(()) => Reply::ok(json!({})),
        Err(e) => Reply::err("network", e.to_string()),
    }
}

/// Edit (or clear) a media caption — its own edit kind; a plain edit would replace the media.
pub async fn edit_caption(engine: SharedEngine, room_id: String, p: &serde_json::Map<String, Value>) -> Reply {
    let Some(open) = super::get(&engine, &room_id) else { return Reply::err("bad_request", "room is not open") };
    let eid = match event_id_of(p) { Ok(e) => e, Err(r) => return r };
    let caption = p.get("body").and_then(Value::as_str).unwrap_or("").trim().to_string();
    let content = EditedContent::MediaCaption {
        caption: if caption.is_empty() { None } else { Some(caption) },
        formatted_caption: None,
        mentions: None,
    };
    match open.timeline.edit(&TimelineEventItemId::EventId(eid), content).await {
        Ok(()) => Reply::ok(json!({})),
        Err(e) => Reply::err("network", e.to_string()),
    }
}

pub async fn edit(engine: SharedEngine, room_id: String, p: &serde_json::Map<String, Value>) -> Reply {
    let Some(open) = super::get(&engine, &room_id) else { return Reply::err("bad_request", "room is not open") };
    let eid = match event_id_of(p) { Ok(e) => e, Err(r) => return r };
    let body = p.get("body").and_then(Value::as_str).unwrap_or("");
    if body.trim().is_empty() { return Reply::err("bad_request", "empty message"); }
    let markdown = p.get("markdown").and_then(Value::as_bool).unwrap_or(true);
    match open.timeline.edit(&TimelineEventItemId::EventId(eid), EditedContent::RoomMessage(text_content_nr(body, markdown))).await {
        Ok(()) => Reply::ok(json!({})),
        Err(e) => Reply::err("network", e.to_string()),
    }
}

pub async fn react(engine: SharedEngine, room_id: String, p: &serde_json::Map<String, Value>) -> Reply {
    let Some(open) = super::get(&engine, &room_id) else { return Reply::err("bad_request", "room is not open") };
    let eid = match event_id_of(p) { Ok(e) => e, Err(r) => return r };
    let key = p.get("key").and_then(Value::as_str).unwrap_or("");
    if key.is_empty() { return Reply::err("bad_request", "key is required"); }
    match open.timeline.toggle_reaction(&TimelineEventItemId::EventId(eid), key).await {
        Ok(added) => Reply::ok(json!({"added": added})),
        Err(e) => Reply::err("network", e.to_string()),
    }
}

pub async fn redact(engine: SharedEngine, room_id: String, p: &serde_json::Map<String, Value>) -> Reply {
    let Some(open) = super::get(&engine, &room_id) else { return Reply::err("bad_request", "room is not open") };
    let eid = match event_id_of(p) { Ok(e) => e, Err(r) => return r };
    let reason = p.get("reason").and_then(Value::as_str).filter(|s| !s.is_empty());
    match open.timeline.redact(&TimelineEventItemId::EventId(eid), reason).await {
        Ok(()) => Reply::ok(json!({})),
        Err(e) => Reply::err("network", e.to_string()),
    }
}

pub async fn mark_read(engine: SharedEngine, room_id: String) -> Reply {
    let Some(open) = super::get(&engine, &room_id) else {
        // Not open: fall back to the room's own mark-as-read.
        let Some(client) = engine.client() else { return Reply::err("not_logged_in", "not logged in") };
        let Some(rid) = crate::sync::members::parse_room_id(&room_id) else { return Reply::err("bad_request", "invalid roomId") };
        let Some(room) = client.get_room(&rid) else { return Reply::err("unknown_room", "unknown room") };
        return match room.set_unread_flag(false).await { Ok(()) => Reply::ok(json!({})), Err(e) => Reply::err("network", e.to_string()) };
    };
    // Both: the "Unread" divider follows m.fully_read, so without it it never moves.
    let _ = open.timeline.mark_as_read(ruma::api::client::receipt::create_receipt::v3::ReceiptType::FullyRead).await;
    match open.timeline.mark_as_read(ruma::api::client::receipt::create_receipt::v3::ReceiptType::Read).await {
        Ok(sent) => Reply::ok(json!({"sent": sent})),
        Err(e) => Reply::err("network", e.to_string()),
    }
}

pub async fn read_receipt(engine: SharedEngine, room_id: String, p: &serde_json::Map<String, Value>) -> Reply {
    let Some(open) = super::get(&engine, &room_id) else { return Reply::err("bad_request", "room is not open") };
    let eid = match event_id_of(p) { Ok(e) => e, Err(r) => return r };
    match open.timeline.send_single_receipt(ruma::api::client::receipt::create_receipt::v3::ReceiptType::Read, eid).await {
        Ok(sent) => Reply::ok(json!({"sent": sent})),
        Err(e) => Reply::err("network", e.to_string()),
    }
}
