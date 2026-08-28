//! A saved contact list in `com.sigil.contacts` account data, read and written back as one
//! blob: `{"contacts": [{"user_id", "nickname", "favorite", "groups"}]}`. Unknown keys on a
//! contact MUST survive a write, or another client's data is lost on every edit.

use ruma::events::{AnyGlobalAccountDataEventContent, GlobalAccountDataEventType};
use ruma::serde::Raw;
use serde_json::{json, Map, Value};

use crate::engine::SharedEngine;
use crate::ipc::wire::Reply;

pub const ACCOUNT_DATA_TYPE: &str = "com.sigil.contacts";
/// A saved list is a convenience, not a directory.
pub const MAX_CONTACTS: usize = 500;

fn event_type() -> GlobalAccountDataEventType {
    GlobalAccountDataEventType::from(ACCOUNT_DATA_TYPE)
}

/// The stored list; a malformed blob reads as empty rather than as an error.
pub async fn load(engine: &SharedEngine) -> Vec<Value> {
    let Some(client) = engine.client() else { return Vec::new() };
    let Ok(Some(raw)) = client.account().fetch_account_data(event_type()).await else {
        return Vec::new()
    };
    let Ok(v) = raw.deserialize_as::<Value>() else { return Vec::new() };
    v.get("contacts")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter(|c| has_id(c)).cloned().collect())
        .unwrap_or_default()
}

fn has_id(c: &Value) -> bool {
    c.get("user_id").and_then(Value::as_str).map(|s| !s.trim().is_empty()).unwrap_or(false)
}

fn id_of(c: &Value) -> String {
    c.get("user_id").and_then(Value::as_str).unwrap_or("").to_string()
}

async fn store(engine: &SharedEngine, contacts: Vec<Value>) -> Result<(), String> {
    let Some(client) = engine.client() else { return Err("not logged in".into()) };
    let body = json!({ "contacts": contacts });
    let raw = Raw::<AnyGlobalAccountDataEventContent>::from_json_string(body.to_string())
        .map_err(|e| e.to_string())?;
    client
        .account()
        .set_account_data_raw(event_type(), raw)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// `contacts.list` → `{contacts: [...]}`
pub async fn list(engine: SharedEngine) -> Reply {
    Reply::ok(json!({ "contacts": load(&engine).await }))
}

/// `contacts.save {userId, nickname?, favorite?, groups?}` — upsert. Fields not sent are
/// left as they were, so a client that only toggles `favorite` cannot wipe a nickname.
pub async fn save(engine: SharedEngine, p: &Map<String, Value>) -> Reply {
    let user_id = p.get("userId").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if user_id.is_empty() { return Reply::err("bad_request", "a contact needs a userId") }

    let mut contacts = load(&engine).await;
    let existing = contacts.iter().position(|c| id_of(c) == user_id);
    if existing.is_none() && contacts.len() >= MAX_CONTACTS {
        return Reply::err("too_many", format!("a saved list holds at most {MAX_CONTACTS} contacts"))
    }

    // Start from what is stored, so unknown keys another client wrote survive.
    let mut entry = existing
        .map(|i| contacts[i].clone())
        .unwrap_or_else(|| json!({}));
    let obj = entry.as_object_mut().ok_or(()).ok();
    let Some(obj) = obj else { return Reply::err("internal", "stored contact is not an object") };
    obj.insert("user_id".into(), json!(user_id));
    if let Some(n) = p.get("nickname").and_then(Value::as_str) {
        obj.insert("nickname".into(), json!(n.trim()));
    }
    if let Some(f) = p.get("favorite").and_then(|v| v.as_bool().or_else(|| v.as_str().map(|s| s == "true"))) {
        obj.insert("favorite".into(), json!(f));
    }
    if let Some(g) = p.get("groups").and_then(Value::as_array) {
        let groups: Vec<String> = g.iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
        obj.insert("groups".into(), json!(groups));
    }

    match existing {
        Some(i) => contacts[i] = entry,
        None => contacts.push(entry),
    }
    match store(&engine, contacts.clone()).await {
        Ok(()) => Reply::ok(json!({ "contacts": contacts })),
        Err(e) => Reply::err("network", e),
    }
}

/// `contacts.remove {userId}`
pub async fn remove(engine: SharedEngine, p: &Map<String, Value>) -> Reply {
    let user_id = p.get("userId").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if user_id.is_empty() { return Reply::err("bad_request", "which contact?") }
    let mut contacts = load(&engine).await;
    let before = contacts.len();
    contacts.retain(|c| id_of(c) != user_id);
    if contacts.len() == before {
        // Already gone is the state the caller wanted.
        return Reply::ok(json!({ "contacts": contacts }))
    }
    match store(&engine, contacts.clone()).await {
        Ok(()) => Reply::ok(json!({ "contacts": contacts })),
        Err(e) => Reply::err("network", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The upsert, without a homeserver: the merge is the part with rules in it.
    fn upsert(mut contacts: Vec<Value>, patch: &Map<String, Value>) -> Vec<Value> {
        let user_id = patch.get("userId").and_then(Value::as_str).unwrap_or("").to_string();
        let existing = contacts.iter().position(|c| id_of(c) == user_id);
        let mut entry = existing.map(|i| contacts[i].clone()).unwrap_or_else(|| json!({}));
        let obj = entry.as_object_mut().unwrap();
        obj.insert("user_id".into(), json!(user_id));
        if let Some(n) = patch.get("nickname").and_then(Value::as_str) { obj.insert("nickname".into(), json!(n)); }
        if let Some(f) = patch.get("favorite").and_then(Value::as_bool) { obj.insert("favorite".into(), json!(f)); }
        match existing { Some(i) => contacts[i] = entry, None => contacts.push(entry) }
        contacts
    }

    fn patch(v: Value) -> Map<String, Value> { v.as_object().unwrap().clone() }

    #[test]
    fn saving_someone_new_appends_them() {
        let out = upsert(Vec::new(), &patch(json!({"userId": "@a:b.com", "nickname": "Alice"})));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["user_id"], "@a:b.com");
        assert_eq!(out[0]["nickname"], "Alice");
    }

    #[test]
    fn saving_someone_twice_edits_rather_than_duplicating() {
        let one = upsert(Vec::new(), &patch(json!({"userId": "@a:b.com", "nickname": "Alice"})));
        let two = upsert(one, &patch(json!({"userId": "@a:b.com", "favorite": true})));
        assert_eq!(two.len(), 1, "one entry per person");
        assert_eq!(two[0]["favorite"], true);
        // A client that only knows how to star someone must not wipe a nickname.
        assert_eq!(two[0]["nickname"], "Alice");
    }

    #[test]
    fn keys_this_build_does_not_understand_survive_an_edit() {
        let stored = vec![json!({"user_id": "@a:b.com", "someFutureField": {"x": 1}})];
        let out = upsert(stored, &patch(json!({"userId": "@a:b.com", "nickname": "Alice"})));
        assert_eq!(out[0]["someFutureField"]["x"], 1);
        assert_eq!(out[0]["nickname"], "Alice");
    }

    #[test]
    fn entries_without_an_id_are_not_contacts() {
        assert!(has_id(&json!({"user_id": "@a:b.com"})));
        assert!(!has_id(&json!({"user_id": ""})));
        assert!(!has_id(&json!({"nickname": "orphan"})));
        assert!(!has_id(&json!("not even an object")));
    }

    #[test]
    fn removal_is_idempotent() {
        let mut list = vec![json!({"user_id": "@a:b.com"}), json!({"user_id": "@c:d.com"})];
        list.retain(|c| id_of(c) != "@a:b.com");
        assert_eq!(list.len(), 1);
        list.retain(|c| id_of(c) != "@a:b.com");
        assert_eq!(list.len(), 1);
    }
}
