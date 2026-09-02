//! Pure projections: engine JSON → the structs pages draw. One function per
//! shape, so the bridge's action code stays legible. Sources are named in the
//! WIRING-*.md contracts; formats follow Service.qml.

use serde_json::Value;

use crate::rows::{initials, tint_for};
use crate::{MemberRow, RoomSettingsModel, ThreadRow, TimelineRow, UserRow};

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}
fn b(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}
fn n(v: &Value, k: &str) -> i64 {
    v.get(k).and_then(Value::as_i64).unwrap_or(0)
}

pub fn member_row(v: &Value) -> MemberRow {
    let uid = s(v, "userId").to_string();
    let name = match s(v, "displayName") {
        "" => uid.clone(),
        d => d.to_string(),
    };
    let level = n(v, "powerLevel");
    MemberRow {
        user_id: uid.clone().into(),
        display_name: name.clone().into(),
        initials: initials(&name).into(),
        tint: tint_for(&uid),
        power_level: level as i32,
        role: if level >= 100 {
            "Admin"
        } else if level >= 50 {
            "Moderator"
        } else {
            ""
        }
        .into(),
        membership: s(v, "membership").into(),
        is_name_ambiguous: b(v, "isNameAmbiguous"),
        ..Default::default()
    }
}

pub fn user_row(v: &Value, saved: bool) -> UserRow {
    let uid = s(v, "userId").to_string();
    let name = match s(v, "displayName") {
        "" => uid
            .trim_start_matches('@')
            .split(':')
            .next()
            .unwrap_or("")
            .to_string(),
        d => d.to_string(),
    };
    UserRow {
        user_id: uid.clone().into(),
        display_name: name.clone().into(),
        initials: initials(&name).into(),
        tint: tint_for(&uid),
        saved,
        ..Default::default()
    }
}

pub fn thread_row(v: &Value) -> ThreadRow {
    let sender = s(v, "sender").to_string();
    let name = match s(v, "senderName") {
        "" => sender.clone(),
        d => d.to_string(),
    };
    ThreadRow {
        root_id: s(v, "rootId").into(),
        sender_name: name.clone().into(),
        initials: initials(&name).into(),
        tint: tint_for(&sender),
        body: s(v, "body").into(),
        reply_count: n(v, "count") as i32,
        last_ts_label: crate::rows::bubble_stamp(n(v, "ts")).into(),
        ..Default::default()
    }
}

/// room.settings reply + the room's rooms.list summary → the settings model.
pub fn settings_model(
    room_id: &str,
    settings: &Value,
    room: &Value,
    avatar: slint::Image,
) -> RoomSettingsModel {
    let name = match s(settings, "name") {
        "" => s(room, "name").to_string(),
        v => v.to_string(),
    };
    let can = &settings["can"];
    RoomSettingsModel {
        room_id: room_id.into(),
        name: name.clone().into(),
        topic: if s(settings, "topic").is_empty() {
            s(room, "topic")
        } else {
            s(settings, "topic")
        }
        .into(),
        canonical_alias: s(room, "canonicalAlias").into(),
        initials: initials(&name).into(),
        avatar,
        tint: tint_for(room_id),
        is_space: b(room, "isSpace"),
        is_dm: b(room, "isDm"),
        join_rule: s(settings, "joinRule").into(),
        history_visibility: s(settings, "historyVisibility").into(),
        encrypted: b(settings, "isEncrypted") || b(room, "isEncrypted"),
        notification_mode: s(settings, "notificationMode").into(),
        my_power_level: if b(settings, "isAdmin") {
            100
        } else {
            n(settings, "myPowerLevel") as i32
        },
        member_count: n(room, "joinedMembers") as i32,
        can_edit_info: b(can, "name") || b(can, "topic"),
        can_edit_permissions: b(can, "admins")
            || b(can, "setPowerLevels")
            || b(can, "stateDefault"),
        can_invite: b(can, "invite"),
        can_kick: b(can, "kick"),
        can_ban: b(can, "ban"),
        is_favourite: b(room, "isFavourite"),
        is_low_priority: b(room, "isLowPriority"),
        ..Default::default()
    }
}

/// Pins stamps: "Today · 2:41 PM" — the pins-page clock (WIRING-spaces).
pub fn pin_stamp(ts_ms: i64) -> String {
    use chrono::{Datelike, Local, TimeZone};
    let Some(t) = Local.timestamp_millis_opt(ts_ms).single() else {
        return String::new();
    };
    let now = Local::now();
    let days = now
        .date_naive()
        .signed_duration_since(t.date_naive())
        .num_days();
    let clock = t.format("%-I:%M %p").to_string();
    if days == 0 {
        return format!("Today · {clock}");
    }
    if days == 1 {
        return format!("Yesterday · {clock}");
    }
    if t.year() == now.year() {
        return format!("{} · {clock}", t.format("%-d %b"));
    }
    format!("{} · {clock}", t.format("%-d %b %Y"))
}

/// Session labels above bubbles: 12h stamps, Service.sessionLabelFor.
pub fn session_label(ts_ms: i64) -> String {
    use chrono::{Datelike, Local, TimeZone};
    let Some(t) = Local.timestamp_millis_opt(ts_ms).single() else {
        return String::new();
    };
    let now = Local::now();
    let days = now
        .date_naive()
        .signed_duration_since(t.date_naive())
        .num_days();
    let clock = t.format("%-I:%M %p").to_string();
    if days == 0 {
        return clock;
    }
    if days == 1 {
        return format!("Yesterday · {clock}");
    }
    if days < 7 {
        return format!("{} · {clock}", t.format("%A"));
    }
    if t.year() == now.year() {
        return format!("{} · {clock}", t.format("%-d %b"));
    }
    format!("{} · {clock}", t.format("%-d %b %Y"))
}

/// The kind chip words on pins cards.
pub fn kind_words(kind: &str) -> &'static str {
    match kind {
        "image" => "Photo",
        "video" => "Video",
        "audio" | "voice" => "Audio",
        "file" => "File",
        "location" | "liveLocation" => "Location",
        "sticker" => "Sticker",
        _ => "",
    }
}

/// SearchPage's collect(): text matches / images / links over a room's items.
pub struct SearchOut {
    pub results: Vec<TimelineRow>,
    pub images: Vec<TimelineRow>,
    pub links: Vec<TimelineRow>,
}

pub fn collect_search(
    items: &[Value],
    query: &str,
    mk: impl Fn(&Value) -> TimelineRow,
) -> SearchOut {
    let q = query.to_lowercase();
    let searching = q.chars().count() >= 2;
    let mut out = SearchOut {
        results: Vec::new(),
        images: Vec::new(),
        links: Vec::new(),
    };
    for item in items.iter().rev() {
        let kind = s(item, "kind");
        if searching {
            if kind != "image"
                && s(item, "body").to_lowercase().contains(&q)
                && out.results.len() < 40
            {
                out.results.push(mk(item));
            }
            continue;
        }
        if kind == "image" && out.images.len() < 12 {
            out.images.push(mk(item));
        }
        if out.links.len() < 10 {
            if let Some(url) = s(item, "body")
                .split_whitespace()
                .find(|w| w.starts_with("http://") || w.starts_with("https://"))
            {
                let mut row = mk(item);
                // ElideMiddle, approximated: Slint only right-elides, so a long
                // URL keeps its host AND its tail here (~52 chars fits the row
                // at phone width in the caption size).
                row.link_url = url.into();
                let chars: Vec<char> = url.chars().collect();
                row.body = if chars.len() > 52 {
                    let head: String = chars[..30].iter().collect();
                    let tail: String = chars[chars.len() - 19..].iter().collect();
                    format!("{head}…{tail}").into()
                } else {
                    url.into()
                };
                out.links.push(row);
            }
        }
    }
    out
}
