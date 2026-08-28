//! Open timelines: subscribe → `timeline.reset` + `timeline.diff` pushes; actions.
pub mod contacts;
pub mod vcard;
pub mod palette;
pub mod motion;
pub mod effects;
pub mod code;
pub mod extras;
pub mod fmt;
pub mod html;
pub mod actions;
pub mod beacon;
pub mod items;
pub mod pins;
pub mod threads;

use std::collections::HashMap;
use std::sync::Arc;

use futures_util::{pin_mut, StreamExt};
use matrix_sdk::Room;
use matrix_sdk_ui::timeline::{Timeline, TimelineBuilder, TimelineReadReceiptTracking};
use serde_json::{json, Value};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use ruma::events::AnySyncTimelineEvent;

use crate::engine::SharedEngine;
use crate::ipc::wire::Reply;

pub struct OpenTimeline {
    pub timeline: Arc<Timeline>,
    pub room: Room,
    tasks: Vec<JoinHandle<()>>,
    _typing_guard: Option<matrix_sdk::event_handler::EventHandlerDropGuard>,
}

impl Drop for OpenTimeline {
    fn drop(&mut self) {
        for t in self.tasks.drain(..) {
            t.abort();
        }
    }
}

#[derive(Default)]
pub struct OpenTimelines {
    pub map: HashMap<String, Arc<OpenTimeline>>,
}

pub fn get(engine: &SharedEngine, room_id: &str) -> Option<Arc<OpenTimeline>> {
    engine.state.lock().timelines.map.get(room_id).cloned()
}


/// Read receipts per member: items' own `read_receipts()` comes back empty, so read the room's.

/// How many rows an op adds or removes, so the pump can report its length.
fn op_delta(op: &Value) -> i64 {
    match op.get("op").and_then(Value::as_str).unwrap_or("") {
        "append" => op.get("items").and_then(Value::as_array).map(|a| a.len() as i64).unwrap_or(0),
        "pushBack" | "pushFront" | "insert" => 1,
        "popBack" | "popFront" | "remove" => -1,
        "clear" => i64::MIN / 4,      // caller clamps at zero
        "reset" => i64::MIN / 4,
        _ => 0,                        // "set" and friends keep the length
    }
}

pub async fn broadcast_receipts(engine: &SharedEngine, room: &Room) {
    use ruma::events::receipt::{ReceiptThread, ReceiptType};
    let me = engine.client().and_then(|c| c.user_id().map(|u| u.to_owned()));
    let Ok(members) = room.members_no_sync(matrix_sdk::RoomMemberships::JOIN).await else { return };
    let mut out: Vec<Value> = Vec::new();
    for m in members.into_iter().take(32) {
        let uid = m.user_id().to_owned();
        if me.as_deref() == Some(uid.as_ref()) { continue }
        // Public receipts only; a private one is not ours to show.
        let Ok(Some((event_id, receipt))) = room
            .load_user_receipt(ReceiptType::Read, ReceiptThread::Unthreaded, &uid)
            .await
        else { continue };
        let mxc = m.avatar_url().map(|a| a.to_string()).unwrap_or_default();
        let avatar = crate::media::cached_avatar_path(engine, &mxc).await;
        out.push(json!({
            "userId": uid.to_string(),
            "displayName": m.display_name().map(|s| s.to_string()).unwrap_or_else(|| uid.localpart().to_string()),
            "avatarPath": avatar,
            "eventId": event_id.to_string(),
            "ts": receipt.ts.map(|t| u64::from(t.0)),
        }));
    }
    engine.hub.broadcast(json!({"event": "room.receipts", "roomId": room.room_id().to_string(), "users": out}));
}

pub async fn close_all(engine: &SharedEngine) {
    let all: Vec<_> = engine.state.lock().timelines.map.drain().map(|(_, v)| v).collect();
    drop(all);
}

/// Which slice of a room a view shows; threads and pins are the same timeline, differently focused.
#[derive(Clone, Debug)]
pub enum ViewFocus {
    Live,
    Thread(ruma::OwnedEventId),
}

/// The key a view is filed under, passed back as `roomId` later. The room id stays a
/// prefix so one room can have its live timeline, its threads and its pins open at once.
pub fn view_key(room_id: &str, focus: &ViewFocus) -> String {
    match focus {
        ViewFocus::Live => room_id.to_string(),
        ViewFocus::Thread(root) => format!("{room_id}|thread:{root}"),
    }
}

/// The room a view key belongs to (the part before the first `|`).
pub fn room_of_key(key: &str) -> &str {
    key.split('|').next().unwrap_or(key)
}

pub async fn open(engine: SharedEngine, room_id: String, initial_items: usize) -> Reply {
    open_view(engine, room_id, ViewFocus::Live, initial_items).await
}

/// `thread.open {roomId, rootId, initialItems}`
pub async fn open_thread(engine: SharedEngine, room_id: String, root_id: String, initial_items: usize) -> Reply {
    let Ok(root) = ruma::EventId::parse(root_id.as_str()) else {
        return Reply::err("bad_request", "invalid rootId");
    };
    open_view(engine, room_id, ViewFocus::Thread(root), initial_items).await
}

/// Whether a state event belongs in a conversation. Namespaced state is machinery, bar two
/// exceptions judged by who posted: `m.room.pinned_events` (out) and `beacon_info` (in).
fn keeps_state(t: &str) -> bool {
    if t == "m.room.pinned_events" { return false }
    if t.contains("beacon_info") { return true }
    !(t.starts_with("org.matrix.")
        || t.starts_with("io.element.")
        || t.starts_with("im.vector.")
        || t.starts_with("m.call.")
        || t.starts_with("m.rtc."))
}

pub async fn open_view(engine: SharedEngine, room_id: String, focus: ViewFocus, initial_items: usize) -> Reply {
    let key = view_key(&room_id, &focus);
    let live = matches!(focus, ViewFocus::Live);
    let Some(client) = engine.client() else { return Reply::err("not_logged_in", "not logged in") };
    let Some(rid) = crate::sync::members::parse_room_id(&room_id) else { return Reply::err("bad_request", "invalid roomId") };
    let Some(room) = client.get_room(&rid) else { return Reply::err("unknown_room", format!("unknown room {room_id}")) };
    if let Some(existing) = get(&engine, &key) {
        let items = existing.timeline.items().await;
        let json_items = items_json(&engine, &room, &items).await;
        engine.hub.broadcast(json!({"event":"timeline.reset","roomId":key,"items":json_items,"len":items.len()}));
        if live { broadcast_receipts(&engine, &room).await; }
        crate::presence::refresh(&engine);
        return Reply::ok(json!({"reopened": true, "key": key}));
    }
    let mut builder = TimelineBuilder::new(&room)
        .track_read_marker_and_receipts(TimelineReadReceiptTracking::MessageLikeEvents)
        // Filter at the SOURCE: diff ops are index-based, so dropping items downstream desyncs the model.
        .event_filter(|ev, rules| {
            use matrix_sdk_ui::timeline::default_event_filter;
            if !default_event_filter(ev, rules) { return false }
            match ev {
                AnySyncTimelineEvent::State(st) => keeps_state(&st.event_type().to_string()),
                _ => true,
            }
        });
    builder = match &focus {
        // Threaded replies belong to their thread; the root carries a summary.
        ViewFocus::Live => builder.with_focus(matrix_sdk_ui::timeline::TimelineFocus::Live { hide_threaded_events: true }),
        ViewFocus::Thread(root) => builder.with_focus(matrix_sdk_ui::timeline::TimelineFocus::Thread { root_event_id: root.clone() }),
    };
    let timeline = match builder
        .build()
        .await
    {
        Ok(t) => Arc::new(t),
        Err(e) => return Reply::err("internal", format!("timeline build failed: {e}")),
    };
    let (items, stream) = timeline.subscribe().await;
    let start_len = items.len();
    let json_items = items_json(&engine, &room, &items).await;
    engine.hub.broadcast(json!({"event":"timeline.reset","roomId":key,"items":json_items,"len":start_len}));

    let mut tasks = Vec::new();
    // Diff pump.
    {
        let engine = engine.clone();
        let room = room.clone();
        let room_id = key.clone();
        tasks.push(tokio::spawn(async move {
            pin_mut!(stream);
            // The view applies ops by index, so a disagreement over length lands them on the wrong rows.
            let mut len = start_len as i64;
            while let Some(diffs) = stream.next().await {
                let mut ops = Vec::with_capacity(diffs.len());
                for d in diffs {
                    let op = items::diff_json(&engine, &room, d).await;
                    len += op_delta(&op);
                    ops.push(op);
                }
                engine.hub.broadcast(json!({"event":"timeline.diff","roomId":room_id,"ops":ops,"len":len.max(0)}));
            }
            debug!("timeline stream ended for {room_id}");
        }));
    }
    // Receipts and typing belong to the ROOM, so only the live timeline runs them.
    if live {
        // Receipts arrive as room account data, not timeline items, so poll them.
        let engine = engine.clone();
        let room = room.clone();
        tasks.push(tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                broadcast_receipts(&engine, &room).await;
            }
        }));
    }

    let (typing_guard, mut typing_rx) = room.subscribe_to_typing_notifications();
    if live {
        let engine = engine.clone();
        let room = room.clone();
        let room_id = room_id.clone();
        tasks.push(tokio::spawn(async move {
            loop {
                match typing_rx.recv().await {
                    Ok(users) => {
                        let mut list = Vec::new();
                        // The typing EDU echoes our own notice back.
                        let me = engine.client().map(|c| c.user_id().map(|u| u.to_string()).unwrap_or_default()).unwrap_or_default();
                        for u in users {
                            if !me.is_empty() && u.as_str() == me { continue }
                            let member = room.get_member_no_sync(&u).await.ok().flatten();
                            let name = member.as_ref().and_then(|m| m.display_name().map(|s| s.to_string()))
                                .unwrap_or_else(|| u.localpart().to_string());
                            let mxc = member.as_ref().and_then(|m| m.avatar_url().map(|a| a.to_string())).unwrap_or_default();
                            let avatar = crate::media::cached_avatar_path(&engine, &mxc).await;
                            list.push(json!({"userId": u.to_string(), "displayName": name, "avatarPath": avatar}));
                        }
                        engine.hub.broadcast(json!({"event":"room.typing","roomId":room_id,"users":list}));
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        }));
    }
    // Initial backfill so the view has something to show.
    if items.len() < initial_items {
        let tl = timeline.clone();
        let want = (initial_items - items.len()).clamp(20, 100) as u16;
        let engine2 = engine.clone();
        let rid2 = key.clone();
        tasks.push(tokio::spawn(async move {
            engine2.hub.broadcast(json!({"event":"timeline.paginationState","roomId":rid2,"state":"paginating"}));
            let hit = tl.paginate_backwards(want).await.unwrap_or(false);
            engine2.hub.broadcast(json!({"event":"timeline.paginationState","roomId":rid2,"state": if hit {"timelineStart"} else {"idle"}}));
        }));
    }
    // Live shares: items only mark where a share sits, so beacons need following.
    let open = Arc::new(OpenTimeline { timeline, room, tasks, _typing_guard: Some(typing_guard) });
    engine.state.lock().timelines.map.insert(key.clone(), open);
    // Members are only "of interest" to presence once the room is open.
    crate::presence::refresh(&engine);
    Reply::ok(json!({"key": key}))
}

pub async fn close(engine: SharedEngine, room_id: String) -> Reply {
    let removed = engine.state.lock().timelines.map.remove(&room_id);
    if removed.is_none() {
        warn!("room.close for a room that was not open");
    }
    Reply::ok(json!({}))
}

pub async fn paginate(engine: SharedEngine, room_id: String, count: u16) -> Reply {
    let Some(open) = get(&engine, &room_id) else { return Reply::err("bad_request", "room is not open") };
    engine.hub.broadcast(json!({"event":"timeline.paginationState","roomId":room_id,"state":"paginating"}));
    match open.timeline.paginate_backwards(count.max(1)).await {
        Ok(hit_start) => {
            engine.hub.broadcast(json!({"event":"timeline.paginationState","roomId":room_id,"state": if hit_start {"timelineStart"} else {"idle"}}));
            Reply::ok(json!({"hitStart": hit_start}))
        }
        Err(e) => {
            engine.hub.broadcast(json!({"event":"timeline.paginationState","roomId":room_id,"state":"idle"}));
            Reply::err("network", e.to_string())
        }
    }
}

async fn items_json(engine: &SharedEngine, room: &Room, items: &matrix_sdk_ui::eyeball_im::Vector<Arc<matrix_sdk_ui::timeline::TimelineItem>>) -> Vec<Value> {
    let mut out = Vec::with_capacity(items.len());
    for it in items.iter() {
        out.push(items::item_json(engine, room, it).await);
    }
    out
}


#[cfg(test)]
mod tests {
    use super::keeps_state;

    #[test]
    fn machinery_stays_out_of_the_conversation() {
        for t in ["org.matrix.msc3401.call.member", "io.element.widget",
                  "im.vector.modular.widgets", "m.call.invite", "m.rtc.member"] {
            assert!(!keeps_state(t), "{t} is machinery and should be filtered");
        }
    }

    #[test]
    fn pinning_a_message_is_not_a_message() {
        // Unnamespaced, so the blanket rule would keep it.
        assert!(!keeps_state("m.room.pinned_events"));
    }

    #[test]
    fn a_live_location_share_survives_its_namespace() {
        // Namespaced while the MSC is unstable, so the blanket rule swallowed it.
        assert!(keeps_state("org.matrix.msc3672.beacon_info"));
        assert!(keeps_state("m.beacon_info"));
    }

    #[test]
    fn ordinary_room_state_still_shows() {
        for t in ["m.room.member", "m.room.name", "m.room.topic", "m.room.avatar"] {
            assert!(keeps_state(t), "{t} is something a person did");
        }
    }
}
