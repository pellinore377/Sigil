//! The *roots* of a room's threads. Opening one is `timeline::open_thread`.

use matrix_sdk::room::ListThreadsOptions;
use ruma::events::AnySyncTimelineEvent;
use serde_json::{json, Value};

use crate::engine::SharedEngine;
use crate::ipc::wire::Reply;

/// `threads.list {roomId}` → `{threads: [{rootId, sender, senderName, avatarPath, body, ts}]}`, server-sorted.
pub async fn list(engine: SharedEngine, room_id: String) -> Reply {
    let Some(client) = engine.client() else { return Reply::err("not_logged_in", "not logged in") };
    let Ok(rid) = ruma::RoomId::parse(&room_id) else { return Reply::err("bad_request", "invalid roomId") };
    let Some(room) = client.get_room(&rid) else { return Reply::err("unknown_room", "unknown room") };

    let roots = match room.list_threads(ListThreadsOptions::default()).await {
        Ok(r) => r,
        Err(e) => return Reply::err("network", e.to_string()),
    };

    let mut out = Vec::new();
    for ev in roots.chunk {
        // matrix-sdk-ui's `thread_summary` stays None against Synapse, so read the
        // server's bundled aggregation (`unsigned.m.relations.m.thread`) it is built from.
        let raw: serde_json::Value = ev.raw().deserialize_as::<serde_json::Value>().unwrap_or(Value::Null);
        let agg = raw.pointer("/unsigned/m.relations/m.thread");
        let count = agg.and_then(|a| a.get("count")).and_then(Value::as_u64).unwrap_or(0);
        let latest = agg.and_then(|a| a.get("latest_event"));
        let latest_sender = latest
            .and_then(|l| l.get("sender"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        // A photo's `body` is its filename and a sticker has none, so name media instead.
        let latest_body = match latest.and_then(|l| l.pointer("/content/msgtype")).and_then(Value::as_str) {
            Some("m.image") => "📷 Photo".to_string(),
            Some("m.video") => "🎥 Video".to_string(),
            Some("m.audio") => "🎤 Audio".to_string(),
            Some("m.file") => "📎 File".to_string(),
            Some("m.location") => "📍 Location".to_string(),
            _ => latest
                .and_then(|l| l.pointer("/content/body"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        };
        let Ok(any) = ev.raw().deserialize() else { continue };
        let AnySyncTimelineEvent::MessageLike(m) = &any else { continue };
        let sender = any.sender().to_string();
        let member = room.get_member_no_sync(any.sender()).await.ok().flatten();
        let name = member
            .as_ref()
            .and_then(|m| m.display_name().map(|s| s.to_string()))
            .unwrap_or_else(|| any.sender().localpart().to_string());
        let mxc = member.as_ref().and_then(|m| m.avatar_url().map(|a| a.to_string())).unwrap_or_default();
        out.push(json!({
            "rootId": any.event_id().to_string(),
            "sender": sender,
            "senderName": name,
            "avatarPath": crate::media::cached_avatar_path(&engine, &mxc).await,
            "body": body_of(m),
            "ts": u64::from(any.origin_server_ts().0),
            "count": count,
            "latestSender": latest_sender,
            "latestBody": latest_body,
        }));
    }
    Reply::ok(json!({ "roomId": room_id, "threads": out }))
}

/// A one-line plain-text description of the root, for the list row.
fn body_of(m: &ruma::events::AnySyncMessageLikeEvent) -> String {
    use ruma::events::room::message::MessageType;
    use ruma::events::AnySyncMessageLikeEvent as E;
    match m {
        E::RoomMessage(ev) => match ev.as_original() {
            Some(o) => match &o.content.msgtype {
                MessageType::Text(t) => t.body.clone(),
                MessageType::Emote(t) => t.body.clone(),
                MessageType::Notice(t) => t.body.clone(),
                MessageType::Image(_) => "Photo".into(),
                MessageType::Video(_) => "Video".into(),
                MessageType::Audio(_) => "Audio".into(),
                MessageType::File(_) => "File".into(),
                MessageType::Location(_) => "Location".into(),
                _ => String::new(),
            },
            None => "Message deleted".into(),
        },
        _ => String::new(),
    }
}
