//! Per-conversation state this device keeps for itself: pinned, marked
//! unread, and snoozed-until.
//!
//! Sigil has no server-side room tags and no account data — a conversation's
//! replicated `Conversation` record is group policy, shared with everyone in
//! it, so none of this belongs there. These are facts about *this* device's
//! list, and they live where every other device-local setting lives: a
//! top-level key in `settings.json`, rewritten whole, exactly as
//! `notify::save_settings` and `sigil::save_shape` do it.

use std::collections::BTreeMap;

use serde_json::{json, Value};

/// "Always", as a deadline: snoozed until it is lifted by hand.
pub const FOREVER: i64 = i64::MAX;

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Flags {
    /// Pinned to the top of the list.
    #[serde(default)]
    pub favourite: bool,
    /// Marked unread by hand; cleared the next time the room is read.
    #[serde(default)]
    pub marked_unread: bool,
    /// Unix ms the snooze runs out at; 0 is not snoozed, `FOREVER` is
    /// "Always".
    #[serde(default)]
    pub snooze_until: i64,
    /// Let a mention through while snoozed (the dialog's checkbox).
    #[serde(default)]
    pub snooze_mentions: bool,
}

impl Flags {
    /// Snoozed as of now — an expired deadline is simply not snoozed, so a
    /// snooze needs no timer to end.
    pub fn snoozed(&self, now_ms: i64) -> bool {
        self.snooze_until == FOREVER || (self.snooze_until > 0 && self.snooze_until > now_ms)
    }
}

pub type Table = BTreeMap<String, Flags>;

pub fn load() -> Table {
    std::fs::read(crate::notify::settings_path())
        .ok()
        .and_then(|d| serde_json::from_slice::<Value>(&d).ok())
        .and_then(|v| serde_json::from_value(v.get("rooms").cloned()?).ok())
        .unwrap_or_default()
}

pub fn save(t: &Table) {
    let path = crate::notify::settings_path();
    let mut v: Value = std::fs::read(&path)
        .ok()
        .and_then(|d| serde_json::from_slice(&d).ok())
        .unwrap_or_else(|| json!({}));
    v["rooms"] = serde_json::to_value(t).unwrap_or(json!({}));
    let _ = std::fs::write(&path, serde_json::to_vec_pretty(&v).unwrap_or_default());
}

/// One conversation's flags; all-false when it has never been touched.
pub fn get(room_id: &str) -> Flags {
    load().get(room_id).cloned().unwrap_or_default()
}

/// Read-modify-write one conversation's flags. A room back at its defaults
/// drops out of the file rather than leaving an empty record behind.
pub fn update(room_id: &str, edit: impl FnOnce(&mut Flags)) {
    let mut t = load();
    let mut f = t.get(room_id).cloned().unwrap_or_default();
    edit(&mut f);
    if !f.favourite && !f.marked_unread && f.snooze_until == 0 {
        t.remove(room_id);
    } else {
        t.insert(room_id.to_string(), f);
    }
    save(&t);
}

/// How many conversations are pinned — the list's five-pin cap is checked
/// against this.
pub fn pinned_count() -> usize {
    load().values().filter(|f| f.favourite).count()
}

/// Leaving a conversation takes its local flags with it.
pub fn forget(room_id: &str) {
    let mut t = load();
    if t.remove(room_id).is_some() {
        save(&t);
    }
}
