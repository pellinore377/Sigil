//! The extra kinds on the Sigil backend: pins, polls, threads, stickers,
//! contacts, places, and link previews. Each is a small event or a flag on
//! a file manifest (protocol spec 7, wire spec 13 and 16); the frontends
//! see the same item shapes they always did.
//!
//! Views: a conversation's history is one list. The main view hides thread
//! replies; a thread view (`room|thread:root`) is the root followed by its
//! replies. `emit_push` and `emit_set` translate a history index into the
//! index each open view sees, so a change lands in every view at the right
//! place.

use super::{now_ms, param, short_name, SigilSession};
use crate::engine::SharedEngine;
use crate::ipc::wire::Reply;
use serde_json::{json, Value};
use sigil_client::conversation;
use sigil_protocol::envelope::Kind;
use std::path::PathBuf;

/// Between live-location updates while a share runs.
const LIVE_UPDATE_SECS: u64 = 30;
/// Longest a live share may run.
const LIVE_MAX_MS: u64 = 8 * 60 * 60 * 1000;
/// A fetched page is read up to here; previews live in the head.
const PREVIEW_HTML_MAX: usize = 512 * 1024;
const PREVIEW_IMAGE_MAX: usize = 4 * 1024 * 1024;

/// `room|thread:root` → (room, root). A plain room id comes back alone.
pub(super) fn split_key(key: &str) -> (String, Option<String>) {
    match key.split_once("|thread:") {
        Some((room, root)) if !root.is_empty() => (room.to_string(), Some(root.to_string())),
        _ => (key.to_string(), None),
    }
}

pub(super) fn thread_key(room: &str, root: &str) -> String {
    format!("{room}|thread:{root}")
}

fn thread_root_of(item: &Value) -> Option<&str> {
    item.get("threadRoot").and_then(Value::as_str).filter(|r| !r.is_empty())
}

/// A text reference: "thread:<root>" or "thread:<root>|<replyTo>" or "<replyTo>".
pub(super) fn parse_reference(reference: &str) -> (Option<String>, Option<String>) {
    if let Some(rest) = reference.strip_prefix("thread:") {
        match rest.split_once('|') {
            Some((root, reply)) => (Some(root.to_string()), Some(reply.to_string()).filter(|r| !r.is_empty())),
            None => (Some(rest.to_string()), None),
        }
    } else if reference.is_empty() {
        (None, None)
    } else {
        (None, Some(reference.to_string()))
    }
}

pub(super) fn make_reference(thread_root: Option<&str>, reply_to: Option<&str>) -> String {
    match (thread_root, reply_to) {
        (Some(root), Some(r)) => format!("thread:{root}|{r}"),
        (Some(root), None) => format!("thread:{root}"),
        (None, Some(r)) => r.to_string(),
        (None, None) => String::new(),
    }
}

fn can_all(is_own: bool) -> Value {
    json!({"edit": false, "reply": true, "redact": is_own, "react": true})
}

fn stickers_dir() -> PathBuf {
    crate::paths::state_dir().join("stickers")
}

fn contacts_path() -> PathBuf {
    crate::paths::state_dir().join("contacts.json")
}

fn load_contacts() -> Vec<Value> {
    std::fs::read(contacts_path())
        .ok()
        .and_then(|d| serde_json::from_slice::<Vec<Value>>(&d).ok())
        .unwrap_or_default()
}

fn save_contacts(list: &[Value]) {
    let _ = std::fs::write(contacts_path(), serde_json::to_vec_pretty(list).unwrap_or_default());
}

/// The poll's public face from its stored votes.
fn poll_view(poll: &mut serde_json::Map<String, Value>, me: &str) {
    let ended = poll.get("ended").and_then(Value::as_bool).unwrap_or(false);
    let closed = poll.get("closed").and_then(Value::as_bool).unwrap_or(false);
    let votes = poll.get("votesBy").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut answers = poll.get("answers").and_then(Value::as_array).cloned().unwrap_or_default();
    for a in answers.iter_mut() {
        let id = a.get("id").and_then(Value::as_str).unwrap_or("").to_string();
        let n = votes
            .values()
            .filter(|ids| ids.as_array().map(|v| v.iter().any(|x| x.as_str() == Some(id.as_str()))).unwrap_or(false))
            .count();
        let mine = votes
            .get(me)
            .and_then(Value::as_array)
            .map(|v| v.iter().any(|x| x.as_str() == Some(id.as_str())))
            .unwrap_or(false);
        a["votes"] = json!(n);
        a["mine"] = json!(mine);
    }
    let voters = votes.values().filter(|v| v.as_array().map(|a| !a.is_empty()).unwrap_or(false)).count();
    poll.insert("answers".into(), json!(answers));
    poll.insert("voters".into(), json!(voters));
    // a closed poll shows its numbers only once ended
    poll.insert("disclosed".into(), json!(!closed || ended));
}

fn location_item(id: &str, sender: &str, ts: i64, is_own: bool, v: &Value) -> Value {
    let lat = v["lat"].as_f64().unwrap_or(0.0);
    let lon = v["lon"].as_f64().unwrap_or(0.0);
    let description = v["description"].as_str().unwrap_or("").to_string();
    let until = v["until"].as_u64().unwrap_or(0);
    let live = until > 0;
    let is_self = v["self"].as_bool().unwrap_or(true);
    let body = if !description.is_empty() {
        description.clone()
    } else if live {
        "Live location".to_string()
    } else {
        "Location".to_string()
    };
    json!({
        "id": id,
        "kind": if live { "liveLocation" } else { "location" },
        "eventId": id,
        "txnId": Value::Null,
        "sender": sender,
        "senderName": short_name(sender),
        "senderAvatarPath": "",
        "ts": ts,
        "isOwn": is_own,
        "isHighlighted": false,
        "body": body,
        "isEdited": false,
        "reactions": [],
        "sendState": "sent",
        "sendError": "",
        "readBy": [],
        "can": can_all(is_own),
        "location": {
            "geoUri": format!("geo:{lat},{lon}"),
            "lat": lat, "lon": lon,
            "description": description,
            "self": is_self,
        },
        "liveShare": if live {
            json!({"live": true, "expiresAt": until, "lat": lat, "lon": lon, "updatedTs": ts, "ended": false})
        } else { Value::Null },
    })
}

impl SigilSession {
    // ------------------------------------------------------------ views

    /// The index an item at history position `idx` has in the main view,
    /// or None when the item is a thread reply (the main view hides those).
    fn main_index(items: &[Value], idx: usize) -> Option<usize> {
        if thread_root_of(&items[idx]).is_some() {
            return None;
        }
        Some(items[..idx].iter().filter(|i| thread_root_of(i).is_none()).count())
    }

    fn main_len(items: &[Value]) -> usize {
        items.iter().filter(|i| thread_root_of(i).is_none()).count()
    }

    /// Everything the main view shows.
    pub(super) fn main_items(items: &[Value]) -> Vec<Value> {
        items.iter().filter(|i| thread_root_of(i).is_none()).cloned().collect()
    }

    /// The thread view for `root`: the root, then its replies.
    fn thread_items(items: &[Value], root: &str) -> Vec<Value> {
        let mut out: Vec<Value> = items
            .iter()
            .filter(|i| i.get("eventId").and_then(Value::as_str) == Some(root))
            .cloned()
            .collect();
        out.extend(items.iter().filter(|i| thread_root_of(i) == Some(root)).cloned());
        out
    }

    /// A new item at the end of history: to the main view, or to its thread's
    /// view, whichever is open.
    pub(super) fn emit_push(&self, engine: &SharedEngine, room_id: &str, item: &Value) {
        let open = self.open.lock();
        match thread_root_of(item) {
            None => {
                if open.contains(room_id) {
                    let len = self.history.lock().get(room_id).map(|i| Self::main_len(i)).unwrap_or(0);
                    engine.hub.broadcast(json!({"event":"timeline.diff","roomId":room_id,"ops":[{"op":"pushBack","item":item}],"len":len}));
                }
            }
            Some(root) => {
                let key = thread_key(room_id, root);
                if open.contains(&key) {
                    let len = self.history.lock().get(room_id).map(|i| Self::thread_items(i, root).len()).unwrap_or(0);
                    engine.hub.broadcast(json!({"event":"timeline.diff","roomId":key,"ops":[{"op":"pushBack","item":item}],"len":len}));
                }
            }
        }
    }

    /// History item `idx` changed: a `set` to every open view that shows it.
    pub(super) fn emit_set(&self, engine: &SharedEngine, room_id: &str, idx: usize) {
        let h = self.history.lock();
        let Some(items) = h.get(room_id) else { return };
        let Some(item) = items.get(idx) else { return };
        let open = self.open.lock();
        if let Some(mi) = Self::main_index(items, idx) {
            if open.contains(room_id) {
                engine.hub.broadcast(json!({"event":"timeline.diff","roomId":room_id,"ops":[{"op":"set","index":mi,"item":item}],"len":Self::main_len(items)}));
            }
        }
        // in a thread view: as the root (index 0) or as a reply
        let ev = item.get("eventId").and_then(Value::as_str).unwrap_or("");
        let root = thread_root_of(item).map(str::to_string).or_else(|| {
            let k = thread_key(room_id, ev);
            open.contains(&k).then(|| ev.to_string())
        });
        if let Some(root) = root {
            let key = thread_key(room_id, &root);
            if open.contains(&key) {
                let view = Self::thread_items(items, &root);
                if let Some(ti) = view.iter().position(|i| i.get("eventId").and_then(Value::as_str) == Some(ev)) {
                    engine.hub.broadcast(json!({"event":"timeline.diff","roomId":key,"ops":[{"op":"set","index":ti,"item":item}],"len":view.len()}));
                }
            }
        }
    }

    /// Drop one history item and tell the views.
    pub(super) fn remove_item(&self, engine: &SharedEngine, room_id: &str, event_id: &str) -> bool {
        let removed = {
            let mut h = self.history.lock();
            let Some(items) = h.get_mut(room_id) else { return false };
            let Some(idx) = items.iter().position(|i| i.get("eventId").and_then(Value::as_str) == Some(event_id)) else {
                return false;
            };
            let main = Self::main_index(items, idx);
            let root = thread_root_of(&items[idx]).map(str::to_string);
            let thread_pos = root.as_ref().and_then(|r| {
                Self::thread_items(items, r).iter().position(|i| i.get("eventId").and_then(Value::as_str) == Some(event_id))
            });
            items.remove(idx);
            let main_len = Self::main_len(items);
            let thread_len = root.as_ref().map(|r| Self::thread_items(items, r).len()).unwrap_or(0);
            (main, root, thread_pos, main_len, thread_len)
        };
        self.save_history();
        let (main, root, thread_pos, main_len, thread_len) = removed;
        let open = self.open.lock();
        if let Some(i) = main {
            if open.contains(room_id) {
                engine.hub.broadcast(json!({"event":"timeline.diff","roomId":room_id,"ops":[{"op":"remove","index":i}],"len":main_len}));
            }
        }
        if let (Some(root), Some(ti)) = (root, thread_pos) {
            let key = thread_key(room_id, &root);
            if open.contains(&key) {
                engine.hub.broadcast(json!({"event":"timeline.diff","roomId":key,"ops":[{"op":"remove","index":ti}],"len":thread_len}));
            }
        }
        true
    }

    /// Change one history item in place and tell the views.
    pub(super) fn update_item(
        &self,
        engine: &SharedEngine,
        room_id: &str,
        event_id: &str,
        f: impl FnOnce(&mut serde_json::Map<String, Value>),
    ) -> bool {
        let idx = {
            let mut h = self.history.lock();
            let Some(items) = h.get_mut(room_id) else { return false };
            let Some(idx) = items.iter().position(|i| i.get("eventId").and_then(Value::as_str) == Some(event_id)) else {
                return false;
            };
            f(items[idx].as_object_mut().unwrap());
            idx
        };
        self.save_history();
        self.emit_set(engine, room_id, idx);
        true
    }

    // ------------------------------------------------------------ sending

    /// One event into a conversation; the sent id comes back.
    async fn send_raw(&self, engine: &SharedEngine, room_id: &str, kind: Kind, reference: &str, body: &[u8]) -> Result<String, Reply> {
        self.top_up().await;
        let Some(conv) = self.conversation(room_id).await else {
            return Err(Reply::err("unknown_room", format!("unknown room {room_id}")));
        };
        let sent = {
            let mut g = self.inner.lock().await;
            let (a, p) = &mut *g;
            conversation::send_event(&self.link, a, p, &conv, kind, reference.as_bytes(), body).await
        };
        let sent = match sent {
            Ok(v) => v,
            Err(e) => return Err(Reply::err("network", format!("{e:#}"))),
        };
        self.ingest_caught(engine, &conv, sent.caught_up).await;
        Ok(format!("{}:{}", hex::encode(sent.address), sent.seq))
    }

    // ------------------------------------------------------------ pins

    pub(super) async fn set_pin(&self, engine: &SharedEngine, room_id: &str, event_id: &str, pin: bool) -> Reply {
        self.top_up().await;
        let Some(conv) = self.conversation(room_id).await else {
            return Reply::err("unknown_room", format!("unknown room {room_id}"));
        };
        if self.item_by_id(room_id, event_id).is_none() {
            return Reply::err("unknown_event", "no such message");
        }
        let mut pinned = conv.pinned.clone();
        pinned.retain(|p| p != event_id);
        if pin {
            pinned.push(event_id.to_string());
        }
        let r = {
            let mut g = self.inner.lock().await;
            let (a, p) = &mut *g;
            sigil_client::group::set_pinned(&self.link, a, p, &conv, pinned).await
        };
        if let Err(e) = r {
            return Reply::err("network", format!("{e:#}"));
        }
        self.broadcast_pinned(engine, room_id).await;
        Reply::ok(json!({}))
    }

    pub(super) async fn pins_list(&self, room_id: &str) -> Reply {
        let Some(conv) = self.conversation(room_id).await else {
            return Reply::err("unknown_room", format!("unknown room {room_id}"));
        };
        Reply::ok(json!({"events": conv.pinned}))
    }

    /// The pinned messages themselves, newest pin first, as timeline items.
    pub(super) async fn pins_items(&self, room_id: &str) -> Reply {
        let Some(conv) = self.conversation(room_id).await else {
            return Reply::err("unknown_room", format!("unknown room {room_id}"));
        };
        let items: Vec<Value> = conv
            .pinned
            .iter()
            .rev()
            .filter_map(|id| self.item_by_id(room_id, id))
            .collect();
        Reply::ok(json!({"items": items}))
    }

    pub(super) async fn broadcast_pinned(&self, engine: &SharedEngine, room_id: &str) {
        if let Some(conv) = self.conversation(room_id).await {
            engine.hub.broadcast(json!({"event":"room.pinned","roomId":room_id,"events":conv.pinned}));
        }
    }

    // ------------------------------------------------------------ polls

    pub(super) async fn poll_create(&self, engine: &SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
        let (room_id, _) = split_key(&param(p, "roomId"));
        let question = param(p, "question").trim().to_string();
        let options: Vec<String> = p
            .get("options")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
            .unwrap_or_default();
        if question.is_empty() || options.len() < 2 {
            return Reply::err("bad_request", "a poll needs a question and two options");
        }
        if options.len() > 20 {
            return Reply::err("bad_request", "at most twenty options");
        }
        let closed = p.get("closed").and_then(Value::as_bool).unwrap_or(false);
        let max = p.get("maxSelections").and_then(Value::as_u64).unwrap_or(1).clamp(1, options.len() as u64);
        let body = json!({
            "question": question,
            "options": options.iter().enumerate().map(|(i, t)| json!({"id": i.to_string(), "text": t})).collect::<Vec<_>>(),
            "closed": closed,
            "max": max,
        });
        let id = match self.send_raw(engine, &room_id, Kind::Poll, "", body.to_string().as_bytes()).await {
            Ok(id) => id,
            Err(r) => return r,
        };
        let item = self.poll_item(&id, &self.username, now_ms(), true, &body);
        self.append(engine, &room_id, item).await;
        Reply::ok(json!({"eventId": id}))
    }

    fn poll_item(&self, id: &str, sender: &str, ts: i64, is_own: bool, body: &Value) -> Value {
        let question = body["question"].as_str().unwrap_or("").to_string();
        let answers: Vec<Value> = body["options"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|o| json!({"id": o["id"].as_str().unwrap_or(""), "text": o["text"].as_str().unwrap_or(""), "votes": 0, "mine": false}))
                    .collect()
            })
            .unwrap_or_default();
        let mut poll = serde_json::Map::new();
        poll.insert("question".into(), json!(question));
        poll.insert("answers".into(), json!(answers));
        poll.insert("closed".into(), json!(body["closed"].as_bool().unwrap_or(false)));
        poll.insert("maxSelections".into(), json!(body["max"].as_u64().unwrap_or(1).max(1)));
        poll.insert("ended".into(), json!(false));
        poll.insert("votesBy".into(), json!({}));
        poll_view(&mut poll, &self.username);
        json!({
            "id": id,
            "kind": "poll",
            "eventId": id,
            "txnId": Value::Null,
            "sender": sender,
            "senderName": short_name(sender),
            "senderAvatarPath": "",
            "ts": ts,
            "isOwn": is_own,
            "isHighlighted": false,
            "body": question,
            "isEdited": false,
            "reactions": [],
            "sendState": "sent",
            "sendError": "",
            "readBy": [],
            "can": can_all(is_own),
            "poll": Value::Object(poll),
        })
    }

    pub(super) async fn poll_vote(&self, engine: &SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
        let (room_id, _) = split_key(&param(p, "roomId"));
        let event_id = param(p, "eventId");
        let ids: Vec<String> = p
            .get("answers")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default();
        let Some(item) = self.item_by_id(&room_id, &event_id) else {
            return Reply::err("unknown_event", "no such poll");
        };
        if item["poll"]["ended"].as_bool().unwrap_or(false) {
            return Reply::err("bad_request", "this poll has ended");
        }
        let body = json!({"ids": ids});
        if let Err(r) = self.send_raw(engine, &room_id, Kind::Vote, &event_id, body.to_string().as_bytes()).await {
            return r;
        }
        let me = self.username.clone();
        self.apply_vote(engine, &room_id, &event_id, &me, &body);
        Reply::ok(json!({}))
    }

    fn apply_vote(&self, engine: &SharedEngine, room_id: &str, poll_id: &str, sender: &str, body: &Value) {
        let me = self.username.clone();
        let sender = sender.to_string();
        self.update_item(engine, room_id, poll_id, |it| {
            let Some(poll) = it.get_mut("poll").and_then(Value::as_object_mut) else { return };
            if poll.get("ended").and_then(Value::as_bool).unwrap_or(false) {
                return;
            }
            let valid: Vec<String> = poll
                .get("answers")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|o| o["id"].as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            let max = poll.get("maxSelections").and_then(Value::as_u64).unwrap_or(1).max(1) as usize;
            let mut ids: Vec<String> = body["ids"]
                .as_array()
                .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).filter(|i| valid.contains(i)).collect())
                .unwrap_or_default();
            ids.dedup();
            ids.truncate(max);
            let votes = poll.entry("votesBy").or_insert_with(|| json!({}));
            if let Some(v) = votes.as_object_mut() {
                v.insert(sender, json!(ids));
            }
            poll_view(poll, &me);
        });
    }

    pub(super) async fn poll_end(&self, engine: &SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
        let (room_id, _) = split_key(&param(p, "roomId"));
        let event_id = param(p, "eventId");
        let Some(item) = self.item_by_id(&room_id, &event_id) else {
            return Reply::err("unknown_event", "no such poll");
        };
        if item["sender"].as_str() != Some(self.username.as_str()) {
            return Reply::err("forbidden", "only the person who asked can end a poll");
        }
        if let Err(r) = self.send_raw(engine, &room_id, Kind::PollEnd, &event_id, b"{}").await {
            return r;
        }
        let me = self.username.clone();
        self.apply_poll_end(engine, &room_id, &event_id, &me);
        Reply::ok(json!({}))
    }

    fn apply_poll_end(&self, engine: &SharedEngine, room_id: &str, poll_id: &str, sender: &str) {
        let me = self.username.clone();
        let sender = sender.to_string();
        self.update_item(engine, room_id, poll_id, |it| {
            // only the asker ends it
            if it.get("sender").and_then(Value::as_str) != Some(sender.as_str()) {
                return;
            }
            let Some(poll) = it.get_mut("poll").and_then(Value::as_object_mut) else { return };
            poll.insert("ended".into(), json!(true));
            poll_view(poll, &me);
        });
    }

    /// An incoming poll, vote or end (kinds 15 to 17).
    pub(super) async fn apply_poll_event(
        &self,
        engine: &SharedEngine,
        room_id: &str,
        id: &str,
        kind: u16,
        reference: &str,
        body: &str,
        sender: &str,
        ts: i64,
        is_own: bool,
    ) {
        let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        if kind == Kind::Poll as u16 {
            if v["question"].as_str().map(|q| !q.trim().is_empty()).unwrap_or(false)
                && v["options"].as_array().map(|a| a.len() >= 2).unwrap_or(false)
            {
                let item = self.poll_item(id, sender, ts, is_own, &v);
                self.append(engine, room_id, item).await;
            }
        } else if kind == Kind::Vote as u16 {
            self.apply_vote(engine, room_id, reference, sender, &v);
        } else if kind == Kind::PollEnd as u16 {
            self.apply_poll_end(engine, room_id, reference, sender);
        }
    }

    // ------------------------------------------------------------ threads

    /// Open a thread as its own view: `{key}`, then `timeline.reset` on it.
    pub(super) async fn thread_open(&self, engine: &SharedEngine, room_id: &str, root: &str) -> Reply {
        let (room_id, _) = split_key(room_id);
        if self.conversation(&room_id).await.is_none() {
            return Reply::err("unknown_room", format!("unknown room {room_id}"));
        }
        if self.item_by_id(&room_id, root).is_none() {
            return Reply::err("unknown_event", "no such message");
        }
        let key = thread_key(&room_id, root);
        self.open.lock().insert(key.clone());
        let items = self.history.lock().get(&room_id).map(|i| Self::thread_items(i, root)).unwrap_or_default();
        engine.hub.broadcast(json!({"event":"timeline.reset","roomId":key,"items":items,"len":items.len()}));
        engine.hub.broadcast(json!({"event":"timeline.paginationState","roomId":key,"state":"timelineStart"}));
        Reply::ok(json!({"key": key}))
    }

    /// Every thread in a conversation, latest activity first.
    pub(super) fn threads_list(&self, room_id: &str) -> Reply {
        let (room_id, _) = split_key(room_id);
        let h = self.history.lock();
        let items = h.get(&room_id).cloned().unwrap_or_default();
        let mut threads: Vec<Value> = items
            .iter()
            .filter(|i| i.get("threadSummary").map(|s| !s.is_null()).unwrap_or(false))
            .map(|i| {
                let s = &i["threadSummary"];
                json!({
                    "rootId": i["eventId"],
                    "sender": i["sender"],
                    "senderName": i["senderName"],
                    "body": i["body"],
                    "count": s["count"],
                    "ts": s["ts"],
                    "latestBody": s["body"],
                })
            })
            .collect();
        threads.sort_by_key(|t| std::cmp::Reverse(t["ts"].as_i64().unwrap_or(0)));
        Reply::ok(json!({"threads": threads}))
    }

    /// A reply landed in a thread: the root's chip counts it.
    pub(super) fn note_thread_reply(&self, engine: &SharedEngine, room_id: &str, root: &str, reply: &Value) {
        let (body, sender, ts) = (
            reply["body"].as_str().unwrap_or("").to_string(),
            reply["sender"].as_str().unwrap_or("").to_string(),
            reply["ts"].as_i64().unwrap_or(0),
        );
        let count = self
            .history
            .lock()
            .get(room_id)
            .map(|items| items.iter().filter(|i| thread_root_of(i) == Some(root)).count())
            .unwrap_or(0);
        self.update_item(engine, room_id, root, |it| {
            it.insert("threadSummary".into(), json!({"count": count, "body": body, "sender": sender, "ts": ts}));
        });
    }

    // ------------------------------------------------------------ stickers

    /// Stickers are local packs: one folder per pack under the state
    /// directory, any image inside is a sticker. Nothing is fetched.
    pub(super) fn stickers_list() -> Reply {
        let dir = stickers_dir();
        let _ = std::fs::create_dir_all(&dir);
        let mut packs: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()).collect())
            .unwrap_or_default();
        packs.sort();
        let mut out = Vec::new();
        for pack in packs {
            let pack_name = pack.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let mut files: Vec<PathBuf> = std::fs::read_dir(&pack)
                .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).collect())
                .unwrap_or_default();
            files.sort();
            for f in files {
                let ext = f.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
                if !matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif") {
                    continue;
                }
                let (w, h) = image::image_dimensions(&f).unwrap_or((0, 0));
                let body = f.file_stem().map(|s| s.to_string_lossy().replace(['_', '-'], " ")).unwrap_or_default();
                out.push(json!({
                    "path": f.to_string_lossy(),
                    "url": f.to_string_lossy(),
                    "body": body,
                    "pack": pack_name,
                    "width": w,
                    "height": h,
                }));
            }
        }
        Reply::ok(json!({"stickers": out, "dir": dir.to_string_lossy()}))
    }

    // ------------------------------------------------------------ contacts

    /// Share a contact: a username (a card is written for them) or a vCard
    /// file already on disk. The file travels like any other, flagged.
    pub(super) async fn contact_send(&self, engine: &SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
        let mut params = p.clone();
        let user_id = param(p, "userId");
        let path = param(p, "path");
        if path.is_empty() {
            if user_id.is_empty() {
                return Reply::err("bad_request", "a contact is a username or a vCard file");
            }
            let name = match param(p, "displayName") {
                s if s.is_empty() => short_name(&user_id),
                s => s,
            };
            let vcf = crate::timeline::vcard::to_vcf(&name, &user_id);
            let dir = crate::paths::cache_dir().join("contacts");
            let _ = std::fs::create_dir_all(&dir);
            let file = dir.join(format!("{}.vcf", short_name(&user_id).replace('/', "_")));
            if let Err(e) = std::fs::write(&file, vcf) {
                return Reply::err("internal", format!("could not write the card: {e}"));
            }
            params.insert("path".into(), json!(file.to_string_lossy()));
            params.insert("caption".into(), json!(name));
        }
        self.attachment_send(engine, &params, super::Extra { contact: true, ..Default::default() }).await
    }

    /// The cards in a shared contact, for the contact page.
    pub(super) async fn vcard_read(&self, p: &serde_json::Map<String, Value>) -> Reply {
        let (room_id, event_id) = (param(p, "roomId"), param(p, "eventId"));
        let (path, ..) = match self.locate(&split_key(&room_id).0, &event_id).await {
            Ok(v) => v,
            Err(r) => return r,
        };
        match std::fs::read_to_string(&path) {
            Ok(src) => Reply::ok(json!({"cards": crate::timeline::vcard::to_json(&crate::timeline::vcard::parse(&src))})),
            Err(e) => Reply::err("internal", format!("{e}")),
        }
    }

    /// `contact{displayName, userId}` for a downloaded vCard.
    pub(super) fn contact_summary(path: &std::path::Path, fallback_name: &str) -> Value {
        let cards = std::fs::read_to_string(path).map(|s| crate::timeline::vcard::parse(&s)).unwrap_or_default();
        let first = cards.first();
        let name = first.map(|c| c.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| fallback_name.to_string());
        let user_id = first.map(|c| c.matrix_id.clone()).unwrap_or_default();
        json!({"displayName": name, "userId": user_id, "cards": cards.len()})
    }

    pub(super) fn contacts_list() -> Reply {
        Reply::ok(json!({"contacts": load_contacts()}))
    }

    pub(super) fn contacts_save(p: &serde_json::Map<String, Value>) -> Reply {
        let user_id = param(p, "userId");
        if user_id.is_empty() {
            return Reply::err("bad_request", "a contact needs a username");
        }
        let name = match param(p, "displayName") {
            s if s.is_empty() => short_name(&user_id),
            s => s,
        };
        let mut list = load_contacts();
        list.retain(|c| c["userId"].as_str() != Some(user_id.as_str()));
        list.push(json!({"userId": user_id, "displayName": name, "savedTs": now_ms()}));
        list.sort_by_key(|c| c["displayName"].as_str().unwrap_or("").to_lowercase());
        save_contacts(&list);
        Reply::ok(json!({"contacts": list}))
    }

    pub(super) fn contacts_remove(p: &serde_json::Map<String, Value>) -> Reply {
        let user_id = param(p, "userId");
        let mut list = load_contacts();
        list.retain(|c| c["userId"].as_str() != Some(user_id.as_str()));
        save_contacts(&list);
        Reply::ok(json!({"contacts": list}))
    }

    // ------------------------------------------------------------ places

    /// A place, once. `lat`/`lon` given, or the device's current fix.
    pub(super) async fn location_send(&self, engine: &SharedEngine, p: &serde_json::Map<String, Value>, live_ms: Option<u64>) -> Reply {
        let (room_id, _) = split_key(&param(p, "roomId"));
        let (lat, lon) = match (p.get("lat").and_then(Value::as_f64), p.get("lon").and_then(Value::as_f64)) {
            (Some(lat), Some(lon)) => (lat, lon),
            _ => match crate::geo::fresh_fix(engine) {
                Some(f) => (f.lat, f.lon),
                None => return Reply::err("unavailable", "no position yet; try position.refresh"),
            },
        };
        if !crate::geo::valid_coords(lat, lon) {
            return Reply::err("bad_request", "that is not a place on Earth");
        }
        let description = param(p, "description");
        let is_self = p.get("self").and_then(Value::as_bool).unwrap_or(true);
        let mut body = json!({"lat": lat, "lon": lon, "description": description, "self": is_self});
        if let Some(ms) = live_ms {
            let ms = ms.clamp(60_000, LIVE_MAX_MS);
            body["until"] = json!(now_ms() as u64 + ms);
        }
        let id = match self.send_raw(engine, &room_id, Kind::Location, "", body.to_string().as_bytes()).await {
            Ok(id) => id,
            Err(r) => return r,
        };
        let item = location_item(&id, &self.username, now_ms(), true, &body);
        self.append(engine, &room_id, item).await;
        if live_ms.is_some() {
            self.run_live_share(engine.clone(), room_id.clone(), id.clone(), body["until"].as_u64().unwrap_or(0));
        }
        Reply::ok(json!({"eventId": id}))
    }

    /// While a live share runs, the device's fix goes out every half minute
    /// as an update referencing the share; the share ends on `stopLive` or
    /// when its window passes.
    fn run_live_share(&self, engine: SharedEngine, room_id: String, share_id: String, until: u64) {
        let Some(me) = engine.sigil.lock().clone() else { return };
        me.live_shares.lock().insert(room_id.clone(), share_id.clone());
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(LIVE_UPDATE_SECS)).await;
                if me.live_shares.lock().get(&room_id) != Some(&share_id) {
                    break;
                }
                if now_ms() as u64 >= until {
                    let _ = me.location_stop_live_inner(&engine, &room_id).await;
                    break;
                }
                crate::geo::refresh(&engine);
                let Some(f) = crate::geo::fresh_fix(&engine) else { continue };
                let body = json!({"lat": f.lat, "lon": f.lon, "until": until, "self": true, "update": true});
                if me.send_raw(&engine, &room_id, Kind::Location, &share_id, body.to_string().as_bytes()).await.is_ok() {
                    me.apply_live_update(&engine, &room_id, &share_id, &body, now_ms());
                }
            }
        });
    }

    pub(super) async fn location_stop_live(&self, engine: &SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
        let (room_id, _) = split_key(&param(p, "roomId"));
        self.location_stop_live_inner(engine, &room_id).await
    }

    async fn location_stop_live_inner(&self, engine: &SharedEngine, room_id: &str) -> Reply {
        let Some(share_id) = self.live_shares.lock().remove(room_id) else {
            return Reply::err("bad_request", "no live share running here");
        };
        let body = json!({"end": true});
        if let Err(r) = self.send_raw(engine, room_id, Kind::Location, &share_id, body.to_string().as_bytes()).await {
            return r;
        }
        self.apply_live_end(engine, room_id, &share_id);
        Reply::ok(json!({}))
    }

    fn apply_live_update(&self, engine: &SharedEngine, room_id: &str, share_id: &str, body: &Value, ts: i64) {
        let (lat, lon) = (body["lat"].as_f64().unwrap_or(0.0), body["lon"].as_f64().unwrap_or(0.0));
        self.update_item(engine, room_id, share_id, |it| {
            if let Some(loc) = it.get_mut("location").and_then(Value::as_object_mut) {
                loc.insert("geoUri".into(), json!(format!("geo:{lat},{lon}")));
                loc.insert("lat".into(), json!(lat));
                loc.insert("lon".into(), json!(lon));
            }
            if let Some(live) = it.get_mut("liveShare").and_then(Value::as_object_mut) {
                live.insert("lat".into(), json!(lat));
                live.insert("lon".into(), json!(lon));
                live.insert("updatedTs".into(), json!(ts));
            }
        });
        engine.hub.broadcast(json!({"event":"location.live","roomId":room_id,"eventId":share_id,"lat":lat,"lon":lon,"ts":ts}));
    }

    fn apply_live_end(&self, engine: &SharedEngine, room_id: &str, share_id: &str) {
        self.update_item(engine, room_id, share_id, |it| {
            if let Some(live) = it.get_mut("liveShare").and_then(Value::as_object_mut) {
                live.insert("live".into(), json!(false));
                live.insert("ended".into(), json!(true));
            }
            if it.get("body").and_then(Value::as_str) == Some("Live location") {
                it.insert("body".into(), json!("Live location ended"));
            }
        });
        engine.hub.broadcast(json!({"event":"location.live","roomId":room_id,"eventId":share_id,"ended":true}));
    }

    /// An incoming place (kind 18): a new share, an update to one, or its end.
    pub(super) async fn apply_location_event(&self, engine: &SharedEngine, room_id: &str, id: &str, reference: &str, body: &str, sender: &str, ts: i64, is_own: bool) {
        let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        if reference.is_empty() {
            if crate::geo::valid_coords(v["lat"].as_f64().unwrap_or(f64::NAN), v["lon"].as_f64().unwrap_or(f64::NAN)) {
                self.append(engine, room_id, location_item(id, sender, ts, is_own, &v)).await;
            }
            return;
        }
        // only the sharer moves or ends their share
        let Some(share) = self.item_by_id(room_id, reference) else { return };
        if share["sender"].as_str() != Some(sender) {
            return;
        }
        if v["end"].as_bool().unwrap_or(false) {
            self.apply_live_end(engine, room_id, reference);
        } else if crate::geo::valid_coords(v["lat"].as_f64().unwrap_or(f64::NAN), v["lon"].as_f64().unwrap_or(f64::NAN)) {
            self.apply_live_update(engine, room_id, reference, &v, ts);
        }
    }
}

// ---------------------------------------------------------------- link previews

/// The card for a link, fetched by this device and nobody else, only when
/// the person has turned previews on. Sites learn the device's address (or
/// the proxy's) when they are fetched, which is why the switch is off by
/// default.
pub(super) async fn link_preview(url: &str) -> Reply {
    let shape = super::load_shape();
    if !shape.link_previews {
        return Reply::err("disabled", "link previews are off");
    }
    let url = url.trim().to_string();
    if !(url.starts_with("https://") || url.starts_with("http://")) || url.len() > 2048 {
        return Reply::err("bad_request", "only http(s) links are previewed");
    }
    let mut builder = reqwest::Client::builder()
        .user_agent("Sigil/1")
        .connect_timeout(std::time::Duration::from_secs(8))
        .timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::limited(4));
    // Direct, or through the proxy set in Sigil; a proxy from the
    // environment is not something the person chose here.
    builder = builder.no_proxy();
    if !shape.socks_proxy.is_empty() {
        match reqwest::Proxy::all(format!("socks5h://{}", shape.socks_proxy)) {
            Ok(px) => builder = builder.proxy(px),
            Err(e) => return Reply::err("bad_request", format!("proxy: {e}")),
        }
    }
    let client = match builder.build() {
        Ok(c) => c,
        Err(e) => return Reply::err("internal", format!("{e}")),
    };
    let html = match fetch_capped(&client, &url, PREVIEW_HTML_MAX).await {
        Ok((bytes, ct)) if ct.contains("html") || ct.is_empty() => String::from_utf8_lossy(&bytes).into_owned(),
        Ok((_, ct)) => return Reply::ok(json!({"url": url, "title": "", "description": "", "imagePath": "", "contentType": ct})),
        Err(e) => return Reply::err("network", e),
    };
    let title = meta(&html, "og:title").or_else(|| meta(&html, "twitter:title")).or_else(|| html_title(&html)).unwrap_or_default();
    let description = meta(&html, "og:description").or_else(|| meta(&html, "description")).unwrap_or_default();
    let site = meta(&html, "og:site_name").unwrap_or_default();
    let image = meta(&html, "og:image").or_else(|| meta(&html, "twitter:image")).unwrap_or_default();
    let is_video = meta(&html, "og:video").is_some() || meta(&html, "og:type").map(|t| t.contains("video")).unwrap_or(false);
    let mut image_path = String::new();
    let (mut iw, mut ih) = (0u32, 0u32);
    if !image.is_empty() {
        if let Some(abs) = absolute(&url, &image) {
            let dir = crate::paths::cache_dir().join("derived");
            let _ = std::fs::create_dir_all(&dir);
            let out = dir.join(format!("link-{}.img", hex::encode(&sigil_protocol::kdf::hash(abs.as_bytes())[..12])));
            if !out.exists() {
                if let Ok((bytes, _)) = fetch_capped(&client, &abs, PREVIEW_IMAGE_MAX).await {
                    let _ = std::fs::write(&out, bytes);
                }
            }
            if let Ok((w, h)) = image::image_dimensions(&out) {
                image_path = out.to_string_lossy().into_owned();
                (iw, ih) = (w, h);
            }
        }
    }
    Reply::ok(json!({
        "url": url,
        "title": title,
        "description": description,
        "siteName": site,
        "imagePath": image_path,
        "imageWidth": iw,
        "imageHeight": ih,
        "isVideo": is_video,
    }))
}

async fn fetch_capped(client: &reqwest::Client, url: &str, max: usize) -> Result<(Vec<u8>, String), String> {
    let resp = client.get(url).send().await.map_err(|e| format!("{e}"))?;
    if !resp.status().is_success() {
        return Err(format!("the site answered {}", resp.status()));
    }
    let ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let bytes = resp.bytes().await.map_err(|e| format!("{e}"))?;
    let mut v = bytes.to_vec();
    v.truncate(max);
    Ok((v, ct))
}

/// `<meta property="og:x" content="…">` or `name=`, either attribute order.
fn meta(html: &str, key: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find("<meta") {
        let start = from + i;
        let end = lower[start..].find('>').map(|e| start + e).unwrap_or(lower.len());
        let tag = &html[start..end];
        let tl = &lower[start..end];
        let names = |attr: &str| attr_value(tag, tl, attr);
        if names("property").as_deref() == Some(key) || names("name").as_deref() == Some(key) {
            if let Some(c) = attr_value(tag, tl, "content") {
                let c = unescape(c.trim());
                if !c.is_empty() {
                    return Some(c);
                }
            }
        }
        from = end.min(lower.len());
        if from >= lower.len() {
            break;
        }
    }
    None
}

fn attr_value(tag: &str, tag_lower: &str, attr: &str) -> Option<String> {
    let needle = format!("{attr}=");
    let mut from = 0;
    while let Some(i) = tag_lower[from..].find(&needle) {
        let at = from + i;
        // must be an attribute start: preceded by whitespace
        if at > 0 && !tag_lower.as_bytes()[at - 1].is_ascii_whitespace() {
            from = at + needle.len();
            continue;
        }
        let rest = &tag[at + needle.len()..];
        let rest = rest.trim_start();
        let val = if let Some(r) = rest.strip_prefix('"') {
            r.split('"').next().unwrap_or("")
        } else if let Some(r) = rest.strip_prefix('\'') {
            r.split('\'').next().unwrap_or("")
        } else {
            rest.split(|c: char| c.is_ascii_whitespace() || c == '>').next().unwrap_or("")
        };
        return Some(val.to_string());
    }
    None
}

fn html_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let s = lower.find("<title")?;
    let s = lower[s..].find('>')? + s + 1;
    let e = lower[s..].find("</title>")? + s;
    let t = unescape(html[s..e].trim());
    (!t.is_empty()).then_some(t)
}

fn unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
}

/// Resolve `href` against `base`: absolute, protocol-relative, root or relative.
fn absolute(base: &str, href: &str) -> Option<String> {
    let href = href.trim();
    if href.starts_with("http://") || href.starts_with("https://") {
        return Some(href.to_string());
    }
    let scheme_end = base.find("://")? + 3;
    let host_end = base[scheme_end..].find('/').map(|i| scheme_end + i).unwrap_or(base.len());
    let origin = &base[..host_end];
    if let Some(rest) = href.strip_prefix("//") {
        return Some(format!("{}//{rest}", &base[..scheme_end - 2]));
    }
    if href.starts_with('/') {
        return Some(format!("{origin}{href}"));
    }
    let dir_end = base.rfind('/').filter(|&i| i >= host_end).unwrap_or(base.len());
    Some(format!("{}/{href}", &base[..dir_end].trim_end_matches('/')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_round_trip() {
        assert_eq!(parse_reference("thread:r1"), (Some("r1".into()), None));
        assert_eq!(parse_reference("thread:r1|m2"), (Some("r1".into()), Some("m2".into())));
        assert_eq!(parse_reference("m2"), (None, Some("m2".into())));
        assert_eq!(parse_reference(""), (None, None));
        assert_eq!(make_reference(Some("r1"), Some("m2")), "thread:r1|m2");
        assert_eq!(split_key("abc|thread:r1"), ("abc".into(), Some("r1".into())));
        assert_eq!(split_key("abc"), ("abc".into(), None));
    }

    #[test]
    fn meta_tags_are_found_in_either_attribute_order() {
        let html = r#"<html><head><title>Fallback &amp; Co</title>
            <meta content="A page" property="og:title">
            <meta name="description" content='Says "hi"'>
            <meta property="og:image" content="/pic.png"></head></html>"#;
        assert_eq!(meta(html, "og:title").as_deref(), Some("A page"));
        assert_eq!(meta(html, "description").as_deref(), Some("Says \"hi\""));
        assert_eq!(meta(html, "og:image").as_deref(), Some("/pic.png"));
        assert_eq!(html_title(html).as_deref(), Some("Fallback & Co"));
        assert_eq!(absolute("https://x.org/a/b.html", "/pic.png").as_deref(), Some("https://x.org/pic.png"));
        assert_eq!(absolute("https://x.org/a/b.html", "c.png").as_deref(), Some("https://x.org/a/c.png"));
        assert_eq!(absolute("https://x.org", "c.png").as_deref(), Some("https://x.org/c.png"));
        assert_eq!(absolute("https://x.org/a", "//cdn.x/c.png").as_deref(), Some("https://cdn.x/c.png"));
    }

    #[test]
    fn a_poll_counts_votes_and_hides_a_closed_one_until_it_ends() {
        let mut poll = serde_json::Map::new();
        poll.insert("answers".into(), json!([{"id":"0","text":"a"},{"id":"1","text":"b"}]));
        poll.insert("closed".into(), json!(true));
        poll.insert("votesBy".into(), json!({"@me:s": ["1"], "@you:s": ["1"], "@her:s": []}));
        poll_view(&mut poll, "@me:s");
        assert_eq!(poll["answers"][1]["votes"], json!(2));
        assert_eq!(poll["answers"][1]["mine"], json!(true));
        assert_eq!(poll["answers"][0]["votes"], json!(0));
        assert_eq!(poll["voters"], json!(2));
        assert_eq!(poll["disclosed"], json!(false));
        poll.insert("ended".into(), json!(true));
        poll_view(&mut poll, "@me:s");
        assert_eq!(poll["disclosed"], json!(true));
    }
}
