//! Presentation strings for timestamps and sizes, in the local zone. `short` goes stale
//! when the local date rolls over, which is why the room list is re-broadcast at midnight.

use chrono::{Datelike, Local, TimeZone};

/// `HH:MM` for a millisecond epoch, empty for 0.
pub fn clock(ts_ms: i64) -> String {
    if ts_ms <= 0 { return String::new(); }
    match Local.timestamp_millis_opt(ts_ms).single() {
        Some(t) => t.format("%H:%M").to_string(),
        None => String::new(),
    }
}

/// Room-list stamp: `HH:MM`, `Yesterday`, `Tue` this week, `3 Feb`, then `3 Feb 24` past a year.
pub fn short(ts_ms: i64, now_ms: i64) -> String {
    if ts_ms <= 0 { return String::new(); }
    let (Some(t), Some(now)) = (
        Local.timestamp_millis_opt(ts_ms).single(),
        Local.timestamp_millis_opt(now_ms).single(),
    ) else { return String::new() };

    let days = now.date_naive().signed_duration_since(t.date_naive()).num_days();
    if days == 0 { return t.format("%H:%M").to_string(); }
    if days == 1 { return "Yesterday".to_string(); }
    if days < 7 { return t.format("%a").to_string(); }
    if t.year() == now.year() { return t.format("%-d %b").to_string(); }
    t.format("%-d %b %y").to_string()
}

/// `m:ss`, or `h:mm:ss` past an hour.
pub fn duration(ms: u64) -> String {
    let s = ms / 1000;
    let (h, m, sec) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{sec:02}") } else { format!("{m}:{sec:02}") }
}

/// Human file size.
pub fn bytes(n: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    let f = n as f64;
    if f < KB { format!("{n} B") }
    else if f < MB { format!("{:.1} KB", f / KB) }
    else if f < GB { format!("{:.1} MB", f / MB) }
    else { format!("{:.2} GB", f / GB) }
}

pub fn now_ms() -> i64 { Local::now().timestamp_millis() }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_durations() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(1023), "1023 B");
        assert_eq!(bytes(1536), "1.5 KB");
        assert_eq!(bytes(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(duration(0), "0:00");
        assert_eq!(duration(1_000), "0:01");
        assert_eq!(duration(61_000), "1:01");
        assert_eq!(duration(3_661_000), "1:01:01");
    }

    #[test]
    fn empty_for_missing_timestamps() {
        assert_eq!(clock(0), "");
        assert_eq!(short(0, now_ms()), "");
    }

    #[test]
    fn relative_days() {
        let now = Local::now();
        let ms = |d: chrono::DateTime<Local>| d.timestamp_millis();
        let yesterday = now - chrono::Duration::days(1);
        assert_eq!(short(ms(yesterday), ms(now)), "Yesterday");
        let last_year = now - chrono::Duration::days(400);
        assert!(short(ms(last_year), ms(now)).len() > 5);   // "3 Feb 24"
    }
}
