//! TimelineItem → JSON (protocol §3.6) and VectorDiff → op JSON.
use std::sync::Arc;

use matrix_sdk_ui::eyeball_im::VectorDiff;
use matrix_sdk::Room;
use matrix_sdk_base::latest_event::{LatestEventValue, RemoteLatestEventValue};
use ruma::events::{AnyMessageLikeEventContent, AnySyncMessageLikeEvent, AnySyncTimelineEvent, sticker::StickerMediaSource};
use matrix_sdk_ui::timeline::{
    EventSendState, EventTimelineItem, MembershipChange, MsgLikeKind, TimelineDetails, TimelineItem,
    TimelineItemContent, TimelineItemKind, VirtualTimelineItem,
};
use ruma::events::room::message::MessageType;
use ruma::events::room::MediaSource;
use serde_json::{json, Value};

use crate::engine::SharedEngine;

pub async fn diff_json(engine: &SharedEngine, room: &Room, d: VectorDiff<Arc<TimelineItem>>) -> Value {
    match d {
        VectorDiff::Append { values } => {
            let mut items = Vec::with_capacity(values.len());
            for v in values.iter() { items.push(item_json(engine, room, v).await); }
            json!({"op":"append","items":items})
        }
        VectorDiff::Clear => json!({"op":"clear"}),
        VectorDiff::PushFront { value } => json!({"op":"pushFront","item":item_json(engine, room, &value).await}),
        VectorDiff::PushBack { value } => json!({"op":"pushBack","item":item_json(engine, room, &value).await}),
        VectorDiff::PopFront => json!({"op":"popFront"}),
        VectorDiff::PopBack => json!({"op":"popBack"}),
        VectorDiff::Insert { index, value } => json!({"op":"insert","index":index,"item":item_json(engine, room, &value).await}),
        VectorDiff::Set { index, value } => json!({"op":"set","index":index,"item":item_json(engine, room, &value).await}),
        VectorDiff::Remove { index } => json!({"op":"remove","index":index}),
        VectorDiff::Truncate { length } => json!({"op":"truncate","len":length}),
        VectorDiff::Reset { values } => {
            let mut items = Vec::with_capacity(values.len());
            for v in values.iter() { items.push(item_json(engine, room, v).await); }
            json!({"op":"reset","items":items})
        }
    }
}

/// `geo:lat,lon[;u=accuracy]`; `;u=` is an accuracy radius in metres.
/// Public face of `parse_geo` for the map compositor.
pub fn geo_of(uri: &str) -> Option<(f64, f64)> {
    match parse_geo(uri) {
        (Some(lat), Some(lon), _) => Some((lat, lon)),
        _ => None,
    }
}

fn parse_geo(uri: &str) -> (Option<f64>, Option<f64>, Option<f64>) {
    let raw = uri.trim_start_matches("geo:");
    let (coords, uncertainty) = match raw.split_once(';') {
        Some((a, rest)) => (a, rest.strip_prefix("u=").and_then(|v| v.parse::<f64>().ok())),
        None => (raw, None),
    };
    match coords.split_once(',') {
        Some((a, b)) => (a.trim().parse().ok(), b.trim().parse().ok(), uncertainty),
        None => (None, None, uncertainty),
    }
}

/// A vCard by name or by mime; some senders send a `.vcf` as octet-stream.
pub fn is_vcard(filename: &str, mime: Option<&str>) -> bool {
    let lower = filename.to_ascii_lowercase();
    if lower.ends_with(".vcf") || lower.ends_with(".vcard") { return true }
    matches!(mime, Some(m) if m.eq_ignore_ascii_case("text/vcard") || m.eq_ignore_ascii_case("text/x-vcard"))
}

fn v_has_html(v: &Value) -> bool {
    v.get("html").and_then(Value::as_str).map(|h| !h.is_empty()).unwrap_or(false)
}

pub async fn item_json(engine: &SharedEngine, room: &Room, item: &TimelineItem) -> Value {
    let id = item.unique_id().0.clone();
    match item.kind() {
        TimelineItemKind::Virtual(v) => match v {
            VirtualTimelineItem::DateDivider(ts) => json!({"id":id,"kind":"dayDivider","ts":u64::from(ts.0)}),
            VirtualTimelineItem::ReadMarker => json!({"id":id,"kind":"readMarker"}),
            VirtualTimelineItem::TimelineStart => json!({"id":id,"kind":"timelineStart"}),
            #[allow(unreachable_patterns)]
            _ => json!({"id":id,"kind":"unsupported"}),
        },
        TimelineItemKind::Event(ev) => event_json(engine, room, id, ev).await,
    }
}

fn profile_json(p: &TimelineDetails<matrix_sdk_ui::timeline::Profile>, fallback: &str) -> (String, String) {
    match p {
        TimelineDetails::Ready(p) => (
            p.display_name.clone().unwrap_or_else(|| fallback.to_string()),
            p.avatar_url.as_ref().map(|u| u.to_string()).unwrap_or_default(),
        ),
        _ => (fallback.to_string(), String::new()),
    }
}

async fn event_json(engine: &SharedEngine, room: &Room, id: String, ev: &EventTimelineItem) -> Value {
    let room_id = room.room_id().to_string();
    let event_id_s = ev.event_id().map(|e| e.to_string());
    let sender = ev.sender().to_string();
    let (sender_name, sender_avatar) = profile_json(ev.sender_profile(), ev.sender().localpart());
    let sender_avatar_path = crate::media::cached_avatar_path(engine, &sender_avatar).await;
    let send_state = match ev.send_state() {
        None => ("sent", String::new()),
        Some(EventSendState::NotSentYet { .. }) => ("sending", String::new()),
        Some(EventSendState::SendingFailed { error, .. }) => ("failed", error.to_string()),
        Some(EventSendState::Sent { .. }) => ("sent", String::new()),
        #[allow(unreachable_patterns)]
        Some(_) => ("sending", String::new()),
    };
    // Read receipts draw avatars rather than a second tick.
    let mut read_by: Vec<Value> = Vec::new();
    for (u, r) in ev.read_receipts().iter() {
        let member = room.get_member_no_sync(u).await.ok().flatten();
        let name = member.as_ref().and_then(|m| m.display_name().map(|s| s.to_string()))
            .unwrap_or_else(|| u.localpart().to_string());
        let mxc = member.as_ref().and_then(|m| m.avatar_url().map(|a| a.to_string())).unwrap_or_default();
        let avatar = crate::media::cached_avatar_path(engine, &mxc).await;
        read_by.push(json!({
            "userId": u.to_string(),
            "displayName": name,
            "avatarPath": avatar,
            "ts": r.ts.map(|t| u64::from(t.0)),
        }));
    }
    let mut v = json!({
        "id": id,
        "eventId": ev.event_id().map(|e| e.to_string()),
        "txnId": ev.transaction_id().map(|t| t.to_string()),
        "sender": sender,
        "senderName": sender_name,
        "senderAvatarUrl": sender_avatar,
        "senderAvatarPath": sender_avatar_path,
        "ts": u64::from(ev.timestamp().0),
        "clock": super::fmt::clock(u64::from(ev.timestamp().0) as i64),
        "isOwn": ev.is_own(),
        "isHighlighted": ev.is_highlighted(),
        "sendState": send_state.0,
        "sendError": send_state.1,
        "readBy": read_by,
        "can": {"edit": ev.is_editable(), "reply": ev.can_be_replied_to(), "redact": ev.is_own(), "react": ev.event_id().is_some()},
    });
    let obj = v.as_object_mut().unwrap();
    match ev.content() {
        TimelineItemContent::MsgLike(m) => {
            if let Some(r) = reactions_json(ev) { obj.insert("reactions".into(), r); }
            if let Some(reply) = &m.in_reply_to {
                let details = match &reply.event {
                    TimelineDetails::Ready(e) => {
                        let (n, _) = profile_json(&e.sender_profile, e.sender.localpart());
                        let (kind, body) = brief_content(&e.content);
                        json!({"eventId": reply.event_id.to_string(), "sender": e.sender.to_string(), "senderName": n, "kind": kind, "body": body})
                    }
                    _ => json!({"eventId": reply.event_id.to_string(), "sender": "", "senderName": "", "kind": "pending", "body": ""}),
                };
                obj.insert("replyTo".into(), details);
            }
            if let Some(t) = &m.thread_root { obj.insert("threadRoot".into(), json!(t.to_string())); }
            // "N replies" chip; the latest reply may not be fetched, so it degrades to the count.
            if let Some(sum) = &m.thread_summary {
                let mut t = serde_json::Map::new();
                t.insert("count".into(), json!(sum.num_replies));
                if let TimelineDetails::Ready(latest) = &sum.latest_event {
                    t.insert("sender".into(), json!(latest.sender.to_string()));
                    if let TimelineDetails::Ready(p) = &latest.sender_profile {
                        if let Some(n) = &p.display_name { t.insert("senderName".into(), json!(n)); }
                    }
                    t.insert("body".into(), json!(embedded_text(latest)));
                }
                obj.insert("threadSummary".into(), Value::Object(t));
            }
            match &m.kind {
                MsgLikeKind::Message(msg) => {
                    message_fields(engine, &room_id, event_id_s.as_deref(), obj, msg).await;
                }
                MsgLikeKind::Sticker(s) => {
                    let c = s.content();
                    obj.insert("kind".into(), json!("sticker"));
                    obj.insert("body".into(), json!(c.body));
                    let (mxc, encrypted, src) = sticker_source(&c.source);
                    let thumb = match &src { Some(s) => crate::media::thumbnail_path_or_fetch(engine, &room_id, event_id_s.as_deref(), s, (512, 512), c.info.mimetype.clone()).await, None => String::new() };
                    obj.insert("media".into(), json!({"mxc": mxc, "encrypted": encrypted, "mime": c.info.mimetype, "width": c.info.width.map(u64::from), "height": c.info.height.map(u64::from), "size": c.info.size.map(u64::from), "thumbnailPath": thumb}));
                }
                MsgLikeKind::Poll(p) => {
                    obj.insert("kind".into(), json!("poll"));
                    obj.insert("body".into(), json!(p.fallback_text().unwrap_or_default()));
                    let me = engine.client().and_then(|c| c.user_id().map(|u| u.to_string()));
                    obj.insert("poll".into(), poll_json(p, me.as_deref()));
                }
                // MSC3489: the SDK aggregates every `beacon` here, so positions arrive as `Set` diffs.
                MsgLikeKind::LiveLocation(l) => {
                    let latest = l.latest_location();
                    // Ending a share posts a second `beacon_info` with its own empty
                    // item; drawing it puts a blank map under every finished share.
                    if !l.is_live() && latest.is_none() {
                        obj.insert("kind".into(), json!("liveLocationEnd"));
                        obj.insert("body".into(), json!("Live location ended"));
                    } else {
                        let started = u64::from(l.ts().0);
                        let expires = started + l.timeout().as_millis() as u64;
                        let (lat, lon, uncertainty) = match latest {
                            Some(b) => parse_geo(b.geo_uri()),
                            None => (None, None, None),
                        };
                        let asset = match l.asset_type() {
                            ruma::events::location::AssetType::Self_ => "m.self",
                            _ => "m.pin",
                        };
                        obj.insert("kind".into(), json!("liveLocation"));
                        obj.insert("body".into(), json!(l.description().unwrap_or("Live location")));
                        obj.insert("location".into(), json!({
                            "geoUri": latest.map(|b| b.geo_uri().to_string()).unwrap_or_default(),
                            "lat": lat, "lon": lon, "uncertainty": uncertainty,
                            "description": l.description().unwrap_or_default(),
                            "asset": asset,
                        }));
                        obj.insert("liveShare".into(), json!({
                            "live": l.is_live(),
                            "startedAt": started,
                            "expiresAt": expires,
                            "at": latest.map(|b| u64::from(b.ts().0)).unwrap_or(0),
                            "updates": l.locations().len(),
                        }));
                    }
                }
                MsgLikeKind::Redacted => {
                    obj.insert("kind".into(), json!("redacted"));
                    obj.insert("body".into(), json!("Message deleted"));
                }
                MsgLikeKind::UnableToDecrypt(e) => {
                    obj.insert("kind".into(), json!("utd"));
                    obj.insert("body".into(), json!("Unable to decrypt"));
                    obj.insert("utdReason".into(), json!(format!("{e:?}")));
                }
                #[allow(unreachable_patterns)]
                _ => { obj.insert("kind".into(), json!("unsupported")); }
            }
        }
        TimelineItemContent::MembershipChange(c) => {
            obj.insert("kind".into(), json!("membership"));
            let who = c.display_name().unwrap_or_else(|| c.user_id().localpart().to_string());
            let text = match c.change() {
                Some(MembershipChange::Joined) => format!("{who} joined the room"),
                Some(MembershipChange::Left) => format!("{who} left the room"),
                Some(MembershipChange::Banned) => format!("{who} was banned"),
                Some(MembershipChange::Unbanned) => format!("{who} was unbanned"),
                Some(MembershipChange::Kicked) => format!("{who} was removed"),
                Some(MembershipChange::Invited) => format!("{who} was invited"),
                Some(MembershipChange::KickedAndBanned) => format!("{who} was removed and banned"),
                Some(MembershipChange::InvitationAccepted) => format!("{who} accepted the invitation"),
                Some(MembershipChange::InvitationRejected) => format!("{who} declined the invitation"),
                Some(MembershipChange::InvitationRevoked) => format!("{who}'s invitation was revoked"),
                Some(MembershipChange::Knocked) => format!("{who} requested to join"),
                Some(MembershipChange::KnockAccepted) => format!("{who}'s request to join was accepted"),
                Some(MembershipChange::KnockRetracted) => format!("{who} withdrew their request to join"),
                Some(MembershipChange::KnockDenied) => format!("{who}'s request to join was denied"),
                Some(MembershipChange::None) => format!("{who} made no membership change"),
                Some(MembershipChange::Error) => format!("{who}: membership error"),
                Some(MembershipChange::NotImplemented) => format!("{who}: membership change"),
                None => format!("{who}: membership change"),
                #[allow(unreachable_patterns)]
                Some(_) => format!("{who}: membership change"),
            };
            obj.insert("stateText".into(), json!(text));
            obj.insert("body".into(), json!(text));
        }
        TimelineItemContent::ProfileChange(c) => {
            obj.insert("kind".into(), json!("profile"));
            let who = sender_name_of(obj);
            let text = if let Some(ch) = c.displayname_change() {
                format!("{} changed their display name to {}", ch.old.clone().unwrap_or_else(|| who.clone()), ch.new.clone().unwrap_or_default())
            } else if c.avatar_url_change().is_some() {
                format!("{who} changed their avatar")
            } else {
                format!("{who} updated their profile")
            };
            obj.insert("stateText".into(), json!(text));
            obj.insert("body".into(), json!(text));
        }
        TimelineItemContent::OtherState(s) => {
            obj.insert("kind".into(), json!("state"));
            let who = sender_name_of(obj);
            let text = format!("{who} changed {}", s.content().event_type().to_string().trim_start_matches("m.room.").replace('_', " "));
            obj.insert("stateText".into(), json!(text));
            obj.insert("body".into(), json!(text));
        }
        TimelineItemContent::FailedToParseMessageLike { event_type, .. } => {
            obj.insert("kind".into(), json!("unsupported"));
            obj.insert("body".into(), json!(format!("Unsupported event {event_type}")));
        }
        TimelineItemContent::FailedToParseState { event_type, .. } => {
            obj.insert("kind".into(), json!("state"));
            obj.insert("body".into(), json!(format!("Unsupported state event {event_type}")));
            obj.insert("stateText".into(), json!(format!("Unsupported state event {event_type}")));
        }
        TimelineItemContent::CallInvite => {
            obj.insert("kind".into(), json!("call"));
            obj.insert("body".into(), json!("Call"));
        }
        TimelineItemContent::RtcNotification { .. } => {
            obj.insert("kind".into(), json!("rtcNotification"));
            obj.insert("body".into(), json!("started a call"));
            obj.insert("stateText".into(), json!(format!("{} started a call", sender_name_of(obj))));
        }
        #[allow(unreachable_patterns)]
        _ => { obj.insert("kind".into(), json!("unsupported")); obj.insert("body".into(), json!("")); }
    }
    // Sigil styling is a custom field ruma does not model, so read it from raw JSON.
    if let Some(raw) = ev.original_json() {
        if let Ok(v) = serde_json::from_str::<Value>(raw.json().get()) {
            if let Some(fx) = v.pointer(&format!("/content/{}", crate::timeline::actions::EFFECTS_KEY)) {
                let parsed = super::effects::from_json(fx);
                if !parsed.is_empty() {
                    obj.insert("effects".into(), super::effects::to_json(&parsed));
                }
            }
            // A shared contact rides in a custom field too; `body` is the fallback.
            if let Some(c) = v.pointer(&format!("/content/{}", crate::timeline::actions::CONTACT_KEY)) {
                if c.get("user_id").and_then(Value::as_str).map(|s| !s.is_empty()).unwrap_or(false) {
                    let mut c = c.clone();
                    // The *contact's* picture, not the sender's.
                    let mxc = c.get("avatar_url").and_then(Value::as_str).unwrap_or("").to_string();
                    let path = crate::media::cached_avatar_path(engine, &mxc).await;
                    if let Some(o) = c.as_object_mut() { o.insert("avatarPath".into(), json!(path)); }
                    obj.insert("contact".into(), c);
                }
            }
        }
    }

    // Bodies render as rich text, so newlines must become <br>.
    if !v_has_html(&v) {
        if let Some(body) = v.get("body").and_then(Value::as_str).filter(|b| !b.is_empty()) {
            let linked = super::html::linkify(body);
            if let Some(o) = v.as_object_mut() { o.insert("html".into(), json!(linked)); }
        }
    }

    v
}

fn sender_name_of(obj: &serde_json::Map<String, Value>) -> String {
    obj.get("senderName").and_then(Value::as_str).unwrap_or("").to_string()
}

fn reactions_json(ev: &EventTimelineItem) -> Option<Value> {
    let own = ev.is_own();
    let _ = own;
    let r = ev.content().reactions()?;
    let mut out = Vec::new();
    for (key, senders) in r.iter() {
        out.push(json!({
            "key": key,
            "count": senders.len(),
            "senders": senders.keys().map(|u| u.to_string()).collect::<Vec<_>>(),
        }));
    }
    Some(Value::Array(out))
}

fn sticker_source(src: &StickerMediaSource) -> (String, bool, Option<MediaSource>) {
    match src {
        StickerMediaSource::Plain(u) => (u.to_string(), false, Some(MediaSource::Plain(u.clone()))),
        StickerMediaSource::Encrypted(f) => (f.url.to_string(), true, Some(MediaSource::Encrypted(f.clone()))),
        #[allow(unreachable_patterns)]
        _ => (String::new(), false, None),
    }
}

fn cached_file_value(src: &MediaSource) -> Value {
    let f = crate::media::file_path_if_cached(src);
    if f.is_empty() { Value::Null } else { json!(f) }
}

fn media_source(src: &MediaSource) -> (String, bool) {
    match src {
        MediaSource::Plain(u) => (u.to_string(), false),
        MediaSource::Encrypted(f) => (f.url.to_string(), true),
    }
}

async fn message_fields(engine: &SharedEngine, room_id: &str, event_id: Option<&str>, obj: &mut serde_json::Map<String, Value>, msg: &matrix_sdk_ui::timeline::Message) {
    obj.insert("isEdited".into(), json!(msg.is_edited()));
    let mut html: Option<String> = None;
    let (kind, body, media) = match msg.msgtype() {
        MessageType::Text(t) => { html = t.formatted.as_ref().filter(|f| f.format == ruma::events::room::message::MessageFormat::Html).map(|f| f.body.clone()); ("text", t.body.clone(), None) }
        MessageType::Notice(t) => { html = t.formatted.as_ref().map(|f| f.body.clone()); ("notice", t.body.clone(), None) }
        MessageType::Emote(t) => { html = t.formatted.as_ref().map(|f| f.body.clone()); ("emote", t.body.clone(), None) }
        MessageType::Image(c) => {
            let (mxc, enc) = media_source(&c.source);
            let info = c.info.as_ref();
            let thumb = crate::media::thumbnail_path_or_fetch(engine, room_id, event_id, &c.source, (800, 600), info.and_then(|i| i.mimetype.clone())).await;
            ("image", c.body.clone(), Some(json!({"mxc": mxc, "encrypted": enc, "filename": c.filename.clone().unwrap_or_else(|| c.body.clone()), "mime": info.and_then(|i| i.mimetype.clone()), "width": info.and_then(|i| i.width).map(u64::from), "height": info.and_then(|i| i.height).map(u64::from), "size": info.and_then(|i| i.size).map(u64::from), "blurhash": info.and_then(|i| i.blurhash.clone()), "thumbnailPath": thumb, "sizeLabel": info.and_then(|i| i.size).map(|v| super::fmt::bytes(u64::from(v))), "path": cached_file_value(&c.source)})))
        }
        MessageType::Video(c) => {
            let (mxc, enc) = media_source(&c.source);
            let info = c.info.as_ref();
            // Poster frame from the event's own thumbnail source.
            let thumb = match info.and_then(|i| i.thumbnail_source.clone()) {
                Some(src) => crate::media::thumbnail_path_or_fetch(engine, room_id, event_id, &src, (800, 600), info.and_then(|i| i.thumbnail_info.as_ref().and_then(|t| t.mimetype.clone()))).await,
                // Many senders attach none; take a frame from the clip if cached.
                None => crate::media::poster_if_cached(&c.source),
            };
            ("video", c.body.clone(), Some(json!({"mxc": mxc, "encrypted": enc, "filename": c.filename.clone().unwrap_or_else(|| c.body.clone()), "mime": info.and_then(|i| i.mimetype.clone()), "width": info.and_then(|i| i.width).map(u64::from), "height": info.and_then(|i| i.height).map(u64::from), "size": info.and_then(|i| i.size).map(u64::from), "duration": info.and_then(|i| i.duration).map(|d| d.as_millis() as u64), "thumbnailPath": thumb, "sizeLabel": info.and_then(|i| i.size).map(|v| super::fmt::bytes(u64::from(v))), "durationLabel": info.and_then(|i| i.duration).map(|d| super::fmt::duration(d.as_millis() as u64)), "path": cached_file_value(&c.source)})))
        }
        MessageType::Audio(c) => {
            let (mxc, enc) = media_source(&c.source);
            let info = c.info.as_ref();
            // MSC3245 waveform amplitudes are 0..1024; the UI draws 0..1.
            let waveform: Option<Vec<f64>> = c.audio.as_ref().map(|a| {
                a.waveform.iter().map(|amp| u64::from(amp.get()) as f64 / 1024.0).collect()
            }).filter(|w: &Vec<f64>| !w.is_empty());
            (if c.voice.is_some() { "voice" } else { "audio" }, c.body.clone(), Some(json!({"mxc": mxc, "encrypted": enc, "filename": c.filename.clone().unwrap_or_else(|| c.body.clone()), "mime": info.and_then(|i| i.mimetype.clone()), "size": info.and_then(|i| i.size).map(u64::from), "sizeLabel": info.and_then(|i| i.size).map(|v| super::fmt::bytes(u64::from(v))), "duration": info.and_then(|i| i.duration).map(|d| d.as_millis() as u64), "durationLabel": info.and_then(|i| i.duration).map(|d| super::fmt::duration(d.as_millis() as u64)), "waveform": waveform, "path": cached_file_value(&c.source)})))
        }
        MessageType::File(c) => {
            let (mxc, enc) = media_source(&c.source);
            let info = c.info.as_ref();
            let fname = c.filename.clone().unwrap_or_else(|| c.body.clone());
            let mime = info.and_then(|i| i.mimetype.clone());
            ("file", c.body.clone(), Some(json!({"mxc": mxc, "encrypted": enc, "filename": fname.clone(), "mime": mime.clone(), "size": info.and_then(|i| i.size).map(u64::from), "sizeLabel": info.and_then(|i| i.size).map(|v| super::fmt::bytes(u64::from(v))), "path": cached_file_value(&c.source), "previewable": crate::docs::previewable(&fname, mime.as_deref()), "docKind": crate::docs::kind_of(&fname, mime.as_deref()), "vcard": is_vcard(&fname, mime.as_deref())})))
        }
        MessageType::Location(c) => {
            let (lat, lon, uncertainty) = parse_geo(&c.geo_uri);
            // MSC3488: `m.self` is the sender's own position (their face); anything else gets the pin.
            let asset = match c.asset.as_ref().map(|a| &a.type_) {
                Some(ruma::events::location::AssetType::Self_) => "m.self",
                _ => "m.pin",
            };
            let geo = json!({
                "geoUri": c.geo_uri,
                "lat": lat,
                "lon": lon,
                "uncertainty": uncertainty,
                "description": c.body,
                "asset": asset,
            });
            ("location", c.body.clone(), Some(geo))
        }
        MessageType::ServerNotice(c) => ("notice", c.body.clone(), None),
        MessageType::VerificationRequest(c) => ("text", c.body.clone(), None),
        other => ("text", other.body().to_string(), None),
    };
    obj.insert("kind".into(), json!(kind));
    obj.insert("body".into(), json!(body));
    // Remote markup is sanitized here, not in the view.
    if let Some(h) = html {
        obj.insert("html".into(), json!(super::html::to_rich_text(&h)));
        // A fenced block is laid out as parts so the code can fill the bubble.
        if let Some(parts) = super::html::to_parts(&h) {
            obj.insert("parts".into(), json!(parts));
        }
    }
    if let Some(m) = media {
        obj.insert(if kind == "location" { "location".into() } else { "media".into() }, m);
    }
}

/// MSC3381 poll state, flattened for the view. An undisclosed poll withholds the
/// responses until it ends, so `disclosed` lets the view say so rather than draw 0% bars.
fn poll_json(p: &matrix_sdk_ui::timeline::PollState, me: Option<&str>) -> Value {
    let r = p.results();
    let disclosed = matches!(r.kind, ruma::events::poll::start::PollKind::Disclosed);
    let ended = r.end_time.is_some();
    // Every voter counted once: a response replaces the sender's previous one.
    let mut voters: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for ids in r.votes.values() {
        for v in ids { voters.insert(v.as_str()); }
    }
    let answers: Vec<Value> = r
        .answers
        .iter()
        .map(|a| {
            let ids = r.votes.get(&a.id);
            let count = ids.map(|v| v.len()).unwrap_or(0);
            let mine = match (ids, me) { (Some(v), Some(me)) => v.iter().any(|u| u == me), _ => false };
            json!({"id": a.id, "text": a.text, "votes": count, "mine": mine})
        })
        .collect();
    json!({
        "question": r.question,
        "answers": answers,
        "maxSelections": r.max_selections,
        "disclosed": disclosed,
        "ended": ended,
        "endedAt": r.end_time.map(|t| u64::from(t.0)),
        "voters": voters.len(),
        "edited": r.has_been_edited,
    })
}

fn brief_content(c: &TimelineItemContent) -> (&'static str, String) {
    match c {
        TimelineItemContent::MsgLike(m) => match &m.kind {
            MsgLikeKind::Message(msg) => (match msg.msgtype() {
                MessageType::Image(_) => "image",
                MessageType::File(_) => "file",
                MessageType::Video(_) => "video",
                MessageType::Audio(_) => "audio",
                _ => "text",
            }, msg.body().to_string()),
            MsgLikeKind::Sticker(s) => ("sticker", s.content().body.clone()),
            MsgLikeKind::Poll(p) => ("poll", p.fallback_text().unwrap_or_default()),
            MsgLikeKind::Redacted => ("redacted", "Message deleted".into()),
            MsgLikeKind::UnableToDecrypt(_) => ("utd", "Unable to decrypt".into()),
            #[allow(unreachable_patterns)]
            _ => ("unsupported", String::new()),
        },
        _ => ("state", String::new()),
    }
}

/// How long a room-list preview may be before it is cut.
const PREVIEW_CHARS: usize = 120;

/// One line, always: newlines collapse and a fenced block is dropped (`hasCode` flags it),
/// or a pasted block makes the room-list row as tall as the paste.
pub fn preview_body(body: &str) -> String {
    let (before, fenced) = match body.split_once("```") {
        Some((head, rest)) => (head, rest.split_once("```").map(|(_, tail)| tail).unwrap_or("")),
        None => (body, ""),
    };
    let has_code = body.contains("```");
    let mut text = if has_code {
        let around = format!("{} {}", before.trim(), fenced.trim());
        let around = around.trim();
        around.to_string()
    } else {
        body.to_string()
    };
    text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() > PREVIEW_CHARS {
        text = text.chars().take(PREVIEW_CHARS).collect::<String>().trim_end().to_string();
        text.push('…');
    }
    text
}

pub fn remote_latest_json(r: &RemoteLatestEventValue) -> Value {
    let Ok(ev) = r.raw().deserialize() else { return json!({"kind":"unsupported","body":""}) };
    let sender = ev.sender().to_string();
    let sender_name = sender.trim_start_matches('@').split(':').next().unwrap_or("").to_string();
    // Flags a fenced block so the room list can mark it with an icon.
    let mut has_fence = false;
    let (kind, body) = match &ev {
        AnySyncTimelineEvent::MessageLike(m) => match m {
            AnySyncMessageLikeEvent::RoomMessage(msg) => match msg.as_original() {
                Some(o) => {
                    has_fence = o.content.msgtype.body().contains("```");
                    (msg_kind(&o.content.msgtype), preview_body(o.content.msgtype.body()))
                }
                None => ("redacted", "Message deleted".to_string()),
            },
            AnySyncMessageLikeEvent::RoomEncrypted(_) => ("utd", "Encrypted message".to_string()),
            AnySyncMessageLikeEvent::Sticker(_) => ("sticker", "Sticker".to_string()),
            // Calls arrive as legacy m.call.* or MSC4075 / MSC3401 events; ruma maps neither to a message.
            other => {
                let t = other.event_type().to_string();
                if is_call_event(&t) { ("call", "Call".to_string()) } else { ("other", String::new()) }
            }
        },
        AnySyncTimelineEvent::State(st) => {
            let t = st.event_type().to_string();
            if is_call_event(&t) { ("call", "Call".to_string()) } else { ("state", String::new()) }
        }
    };
    json!({"kind": kind, "sender": sender, "senderName": sender_name,
           "body": body, "hasCode": has_fence})
}

/// Legacy VoIP, MatrixRTC membership and RTC notification event types.
fn is_call_event(t: &str) -> bool {
    t.starts_with("m.call.")
        || t.starts_with("m.rtc.")
        || t.contains("msc3401.call")
        || t.contains("msc4075")
        || t.contains("rtc.notification")
        || t.contains("rtc.member")
}

fn msg_kind(m: &MessageType) -> &'static str {
    match m {
        MessageType::Image(_) => "image",
        MessageType::File(_) => "file",
        MessageType::Video(_) => "video",
        MessageType::Audio(_) => "audio",
        MessageType::Notice(_) => "notice",
        MessageType::Emote(_) => "emote",
        _ => "text",
    }
}

fn local_content_body(c: &matrix_sdk_base::store::SerializableEventContent) -> String {
    match c.deserialize() {
        Ok(AnyMessageLikeEventContent::RoomMessage(m)) => m.msgtype.body().to_string(),
        Ok(AnyMessageLikeEventContent::Sticker(_)) => "Sticker".into(),
        _ => String::new(),
    }
}

pub fn local_latest_body(v: &LatestEventValue) -> String {
    match v {
        LatestEventValue::LocalIsSending(l) | LatestEventValue::LocalCannotBeSent(l) => local_content_body(&l.content),
        LatestEventValue::LocalHasBeenSent { value, .. } => local_content_body(&value.content),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_geo_uri_yields_coordinates_and_an_accuracy_radius() {
        let (lat, lon, u) = parse_geo("geo:48.8583736,2.2944813;u=15.256");
        assert_eq!(lat, Some(48.8583736));
        assert_eq!(lon, Some(2.2944813));
        assert_eq!(u, Some(15.256));
    }

    #[test]
    fn the_accuracy_radius_is_optional_and_never_read_as_a_coordinate() {
        let (lat, lon, u) = parse_geo("geo:51.5,-0.12");
        assert_eq!((lat, lon), (Some(51.5), Some(-0.12)));
        assert_eq!(u, None, "a missing ;u= is not an accuracy of zero");
    }

    #[test]
    fn nonsense_is_refused_rather_than_placed_in_the_gulf_of_guinea() {
        // A partial parse yielding 0,0 would draw a confident pin in the ocean.
        assert_eq!(parse_geo("geo:").0, None);
        assert_eq!(parse_geo("geo:not,here").0, None);
        assert_eq!(parse_geo("").1, None);
    }
}

#[cfg(test)]
mod preview_tests {
    use super::preview_body;

    #[test]
    fn newlines_never_reach_the_room_list() {
        let out = preview_body("line one\nline two\n\nline three");
        assert_eq!(out, "line one line two line three");
        assert!(!out.contains('\n'));
    }

    #[test]
    fn a_fence_leaves_only_what_was_written_around_it() {
        assert_eq!(preview_body("```rust\nfn main() {}\n```"), "");
        assert_eq!(
            preview_body("Here is the reaper:\n```rust\nfn main() {}\n```"),
            "Here is the reaper:"
        );
        assert_eq!(
            preview_body("before\n```\ncode\n```\nafter"),
            "before after"
        );
    }

    #[test]
    fn a_long_line_is_cut_with_an_ellipsis() {
        let out = preview_body(&"word ".repeat(200));
        assert!(out.chars().count() <= 121, "got {}", out.chars().count());
        assert!(out.ends_with('…'));
    }

    #[test]
    fn ordinary_messages_are_left_alone() {
        assert_eq!(preview_body("hello there"), "hello there");
        assert_eq!(preview_body(""), "");
    }
}

/// One line describing a thread's latest reply, for the chip on the root bubble.
fn embedded_text(e: &matrix_sdk_ui::timeline::EmbeddedEvent) -> String {
    use matrix_sdk_ui::timeline::TimelineItemContent as C;
    match &e.content {
        C::MsgLike(m) => match &m.kind {
            MsgLikeKind::Message(msg) => msg.body().to_string(),
            MsgLikeKind::Sticker(_) => "Sticker".into(),
            MsgLikeKind::Poll(_) => "Poll".into(),
            MsgLikeKind::Redacted => "Message deleted".into(),
            MsgLikeKind::UnableToDecrypt(_) => "Waiting for keys…".into(),
            _ => String::new(),
        },
        _ => String::new(),
    }
}
