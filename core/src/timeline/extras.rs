//! Sends that are not plain messages: polls, locations and stickers. All go through
//! the open `Timeline`, so they get local echo and the ordinary send queue.

use ruma::events::poll::start::PollKind;
use ruma::events::poll::unstable_end::UnstablePollEndEventContent;
use ruma::events::poll::unstable_response::UnstablePollResponseEventContent;
use ruma::events::poll::unstable_start::{
    NewUnstablePollStartEventContent, UnstablePollAnswer, UnstablePollAnswers,
    UnstablePollStartContentBlock,
};
use ruma::events::room::message::{LocationMessageEventContent, MessageType, RoomMessageEventContent};
use ruma::events::room::ImageInfo;
use ruma::events::sticker::StickerEventContent;
use ruma::events::AnyMessageLikeEventContent;
use serde_json::{json, Value};

use crate::engine::SharedEngine;
use crate::ipc::wire::Reply;


/// The CLI passes every parameter as a string, so accept both shapes.
fn num(p: &serde_json::Map<String, Value>, key: &str) -> Option<f64> {
    match p.get(key) {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn string_list(p: &serde_json::Map<String, Value>, key: &str) -> Vec<String> {
    let clean = |v: &Value| v.as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    match p.get(key) {
        Some(Value::Array(a)) => a.iter().filter_map(clean).collect(),
        Some(Value::String(s)) => serde_json::from_str::<Vec<String>>(s)
            .map(|v| v.into_iter().map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// `poll.create {roomId, question, options[], closed}` — `closed` picks the MSC3381
/// kind: undisclosed hides the tally until the poll ends, disclosed shows it live.
pub async fn create_poll(engine: SharedEngine, room_id: String, p: &serde_json::Map<String, Value>) -> Reply {
    let question = p.get("question").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if question.is_empty() { return Reply::err("bad_request", "poll needs a question") }
    let options: Vec<String> = string_list(p, "options");
    if options.len() < 2 { return Reply::err("bad_request", "poll needs at least two options") }
    if options.len() > 20 { return Reply::err("bad_request", "poll allows at most twenty options") }

    let answers: Vec<UnstablePollAnswer> = options
        .iter()
        .enumerate()
        .map(|(i, text)| UnstablePollAnswer::new(format!("{i}"), text.clone()))
        .collect();
    let Ok(answers) = UnstablePollAnswers::try_from(answers) else {
        return Reply::err("bad_request", "poll options rejected")
    };

    let mut block = UnstablePollStartContentBlock::new(question.clone(), answers);
    let closed = match p.get("closed") { Some(Value::Bool(b)) => *b, Some(Value::String(s)) => s == "true", _ => false };
    block.kind = if closed {
        PollKind::Undisclosed
    } else {
        PollKind::Disclosed
    };

    // Plain-text fallback for clients that do not understand polls.
    let fallback = std::iter::once(question)
        .chain(options.iter().enumerate().map(|(i, o)| format!("{}. {o}", i + 1)))
        .collect::<Vec<_>>()
        .join("\n");
    let content = NewUnstablePollStartEventContent::plain_text(fallback, block);

    let Some(open) = crate::timeline::get(&engine, &room_id) else {
        return Reply::err("bad_request", "room is not open")
    };
    match open.timeline.send(AnyMessageLikeEventContent::UnstablePollStart(content.into())).await {
        Ok(_) => Reply::ok(json!({})),
        Err(e) => Reply::err("send_failed", e.to_string()),
    }
}

/// `poll.vote {roomId, eventId, answers[]}` — MSC3381; replaces the previous response, so `[]` retracts.
pub async fn vote_poll(engine: SharedEngine, room_id: String, p: &serde_json::Map<String, Value>) -> Reply {
    let event_id = p.get("eventId").and_then(Value::as_str).unwrap_or("").trim().to_string();
    let Ok(poll_id) = ruma::EventId::parse(event_id.as_str()) else {
        return Reply::err("bad_request", "vote needs the poll's eventId")
    };
    let answers = string_list(p, "answers");
    let Some(open) = crate::timeline::get(&engine, &room_id) else {
        return Reply::err("room_not_open", "open the room first")
    };
    let content = UnstablePollResponseEventContent::new(answers, poll_id);
    match open.timeline.send(AnyMessageLikeEventContent::UnstablePollResponse(content)).await {
        Ok(_) => Reply::ok(json!({})),
        Err(e) => Reply::err("send_failed", format!("vote failed: {e}")),
    }
}

/// `poll.end {roomId, eventId}` — MSC3381. Permanent, needs the redact power level.
pub async fn end_poll(engine: SharedEngine, room_id: String, p: &serde_json::Map<String, Value>) -> Reply {
    let event_id = p.get("eventId").and_then(Value::as_str).unwrap_or("").trim().to_string();
    let Ok(poll_id) = ruma::EventId::parse(event_id.as_str()) else {
        return Reply::err("bad_request", "ending a poll needs its eventId")
    };
    let Some(open) = crate::timeline::get(&engine, &room_id) else {
        return Reply::err("room_not_open", "open the room first")
    };
    let content = UnstablePollEndEventContent::new("The poll has closed.", poll_id);
    match open.timeline.send(AnyMessageLikeEventContent::UnstablePollEnd(content)).await {
        Ok(_) => Reply::ok(json!({})),
        Err(e) => Reply::err("send_failed", format!("ending the poll failed: {e}")),
    }
}

/// `location.send {roomId, lat, lon, description?}` → an `m.location` message.
pub async fn send_location(engine: SharedEngine, room_id: String, p: &serde_json::Map<String, Value>) -> Reply {
    let (Some(lat), Some(lon)) = (num(p, "lat"), num(p, "lon")) else {
        return Reply::err("bad_request", "location needs lat and lon")
    };
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return Reply::err("bad_request", "coordinates out of range")
    }
    let desc = p.get("description").and_then(Value::as_str).unwrap_or("").trim().to_string();
    let geo = format!("geo:{lat},{lon}");
    let body = if desc.is_empty() { format!("Location {lat}, {lon}") } else { desc };
    // MSC3488 `m.self` = "where I am". Set on **both** paths: ruma's `AssetType` defaults
    // to `Self_`, so leaving it alone makes every dropped place claim to be the sender.
    let is_self = p.get("selfLocation").and_then(|v| v.as_bool().or_else(|| v.as_str().map(|s| s == "true"))).unwrap_or(false);
    let loc = LocationMessageEventContent::new(body, geo).with_asset_type(if is_self {
        ruma::events::location::AssetType::Self_
    } else {
        ruma::events::location::AssetType::Pin
    });
    let content = RoomMessageEventContent::new(MessageType::Location(
        loc,
    ));

    let Some(open) = crate::timeline::get(&engine, &room_id) else {
        return Reply::err("bad_request", "room is not open")
    };
    match open.timeline.send(AnyMessageLikeEventContent::RoomMessage(content)).await {
        Ok(_) => Reply::ok(json!({})),
        Err(e) => Reply::err("send_failed", e.to_string()),
    }
}

/// `sticker.send {roomId, url, body, width, height}` → an `m.sticker` event.
pub async fn send_sticker(engine: SharedEngine, room_id: String, p: &serde_json::Map<String, Value>) -> Reply {
    let url = p.get("url").and_then(Value::as_str).unwrap_or("").to_string();
    if !url.starts_with("mxc://") { return Reply::err("bad_request", "sticker needs an mxc url") }
    let Ok(uri) = ruma::OwnedMxcUri::try_from(url) else {
        return Reply::err("bad_request", "bad mxc url")
    };
    let body = p.get("body").and_then(Value::as_str).unwrap_or("Sticker").to_string();
    let mut info = ImageInfo::new();
    if let Some(w) = p.get("width").and_then(Value::as_u64) { info.width = ruma::UInt::new(w) }
    if let Some(h) = p.get("height").and_then(Value::as_u64) { info.height = ruma::UInt::new(h) }
    let content = StickerEventContent::new(body, info, uri);

    let Some(open) = crate::timeline::get(&engine, &room_id) else {
        return Reply::err("bad_request", "room is not open")
    };
    match open.timeline.send(AnyMessageLikeEventContent::Sticker(content)).await {
        Ok(_) => Reply::ok(json!({})),
        Err(e) => Reply::err("send_failed", e.to_string()),
    }
}

/// `stickers.list` → MSC2545 image packs from account data, as `{packs: [{name, stickers[]}]}`.
pub async fn list_stickers(engine: SharedEngine) -> Reply {
    let Some(client) = engine.client() else { return Reply::err("not_logged_in", "no session") };
    let mut packs: Vec<Value> = Vec::new();
    let ty = ruma::events::GlobalAccountDataEventType::from("im.ponies.user_emotes");
    if let Ok(Some(raw)) = client.account().fetch_account_data(ty).await {
        if let Ok(v) = raw.deserialize_as::<Value>() {
            let mut pack = pack_json("Personal", &v);
            // The first pass returns empty paths and warms the cache.
            if let Some(list) = pack["stickers"].as_array_mut() {
                for st in list.iter_mut() {
                    let url = st["url"].as_str().unwrap_or("").to_string();
                    let path = crate::media::cached_avatar_path(&engine, &url).await;
                    st["path"] = json!(path);
                }
            }
            packs.push(pack);
        }
    }
    let packs: Vec<Value> = packs
        .into_iter()
        .filter(|p| p["stickers"].as_array().map(|a| !a.is_empty()).unwrap_or(false))
        .collect();
    Reply::ok(json!({ "packs": packs }))
}

/// One MSC2545 pack object → `{name, stickers[]}`.
fn pack_json(default_name: &str, v: &Value) -> Value {
    let name = v
        .get("pack")
        .and_then(|p| p.get("display_name"))
        .and_then(Value::as_str)
        .unwrap_or(default_name)
        .to_string();
    let mut out: Vec<Value> = Vec::new();
    if let Some(images) = v.get("images").and_then(Value::as_object) {
        for (key, img) in images {
            let Some(url) = img.get("url").and_then(Value::as_str) else { continue };
            if !url.starts_with("mxc://") { continue }
            let body = img.get("body").and_then(Value::as_str).unwrap_or(key).to_string();
            let info = img.get("info");
            out.push(json!({
                "url": url,
                "body": body,
                "width": info.and_then(|i| i.get("w")).and_then(Value::as_u64),
                "height": info.and_then(|i| i.get("h")).and_then(Value::as_u64),
            }));
        }
    }
    json!({ "name": name, "stickers": out })
}

#[cfg(test)]
mod tests {
    use ruma::events::location::AssetType;
    use ruma::events::room::message::LocationMessageEventContent;

    fn asset_of(c: &LocationMessageEventContent) -> String {
        c.asset.as_ref().map(|a| a.type_.to_string()).unwrap_or_default()
    }

    #[test]
    fn a_dropped_pin_does_not_claim_to_be_the_sender() {
        // ruma's `AssetType` defaults to `m.self`, so *not* setting it is not neutral.
        let bare = LocationMessageEventContent::new("x".into(), "geo:1,2".into());
        assert_eq!(asset_of(&bare), "m.self", "ruma's default, not ours");

        let pin = LocationMessageEventContent::new("x".into(), "geo:1,2".into())
            .with_asset_type(AssetType::Pin);
        assert_eq!(asset_of(&pin), "m.pin");

        let me = LocationMessageEventContent::new("x".into(), "geo:1,2".into())
            .with_asset_type(AssetType::Self_);
        assert_eq!(asset_of(&me), "m.self");
    }
}
