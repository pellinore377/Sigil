//! JSON from the engine → the row structs the UI draws. The mapping rules are
//! ports of mobile/HomePage.qml and BubbleDelegate.qml; where a rule exists on
//! both sides (avatar hue, preview marks) the QML file is the reference.

use serde_json::Value;

/// The icon strings a row can carry, fetched once from the generated `Icons`
/// global so the codepoints stay single-sourced in shared/icons.json.
#[derive(Clone, Default)]
pub struct IconSet {
    pub camera: slint::SharedString,
    pub video_on: slint::SharedString,
    pub mic_on: slint::SharedString,
    pub attach: slint::SharedString,
    pub phone: slint::SharedString,
    pub code_blocks: slint::SharedString,
    pub poll: slint::SharedString,
    pub sticker: slint::SharedString,
    pub location: slint::SharedString,
    pub person: slint::SharedString,
}

impl IconSet {
    /// The icon glyphs the bridge stamps into rows, read once from the
    /// `Icons` global so Rust never spells a codepoint.
    pub fn from_window(win: &crate::AppWindow) -> IconSet {
        use slint::ComponentHandle as _;
        let g = win.global::<crate::Icons>();
        IconSet {
            camera: g.get_camera(),
            video_on: g.get_videoOn(),
            mic_on: g.get_micOn(),
            attach: g.get_attach(),
            phone: g.get_phone(),
            code_blocks: g.get_codeBlocks(),
            poll: g.get_poll(),
            sticker: g.get_sticker(),
            location: g.get_location(),
            person: g.get_person(),
        }
    }
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}
fn b(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}
fn n(v: &Value, k: &str) -> i64 {
    v.get(k).and_then(Value::as_i64).unwrap_or(0)
}

/// Avatar.qml's hashed hue, exactly: h = (h*31 + code) >>> 0 over userId||name,
/// then a fixed hue table at HSL(?, 0.35, 0.55).
pub fn tint_for(key: &str) -> slint::Color {
    let mut h: u32 = 0;
    // JS charCodeAt is UTF-16; Matrix ids are ASCII in practice, chars() matches.
    for c in key.chars() {
        h = h.wrapping_mul(31).wrapping_add(c as u32);
    }
    const HUES: [f64; 10] = [0.00, 0.08, 0.16, 0.33, 0.50, 0.58, 0.66, 0.75, 0.83, 0.92];
    let hue = HUES[(h % 10) as usize];
    let (r, g, bl) = hsl_to_rgb(hue, 0.35, 0.55);
    slint::Color::from_rgb_u8(r, g, bl)
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h * 6.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    (
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}

/// Avatar.qml: one letter, uppercased, from the name with sigils stripped.
// Avatar.qml: strip the sigil, split on space/underscore/dash/dot, and take
// two letters when the name has two parts ("Jane Doe" → "JD"), else one.
pub fn initials(name: &str) -> String {
    let n = name.trim_start_matches(['@', '#', '!']).trim();
    if n.is_empty() {
        return "?".into();
    }
    let mut parts = n
        .split(|c: char| c.is_whitespace() || matches!(c, '_' | '-' | '.'))
        .filter(|p| !p.is_empty());
    let first = parts.next().and_then(|p| p.chars().next());
    let second = parts.next().and_then(|p| p.chars().next());
    match (first, second) {
        (Some(a), Some(b)) => format!("{}{}", a.to_uppercase(), b.to_uppercase()),
        (Some(a), None) => a.to_uppercase().to_string(),
        _ => "?".into(),
    }
}

/// HomePage.qml's preview line: typing > invite > call > last message,
/// with the leading icon mark in its own element.
pub struct Preview {
    pub icon: slint::SharedString,
    pub text: String,
    pub typing: bool,
}

pub fn preview_for(room: &Value, typing: &[Value], icons: &IconSet) -> Preview {
    if let Some(first) = typing.first() {
        let who = s(first, "displayName");
        return Preview {
            icon: Default::default(),
            text: format!("{who} is typing…"),
            typing: true,
        };
    }
    if b(room, "isInvite") {
        // A Sigil request carries the stranger's first message; show it, the
        // way the row would once the conversation exists.
        let first = s(&room["lastMessage"], "body");
        return Preview {
            icon: Default::default(),
            text: if first.is_empty() || first == "Invitation" {
                "Invitation — tap to respond".into()
            } else {
                first.to_string()
            },
            typing: false,
        };
    }
    if b(room, "hasActiveCall") {
        return Preview {
            icon: icons.phone.clone(),
            text: "Ongoing call".into(),
            typing: false,
        };
    }
    let lm = &room["lastMessage"];
    if lm.is_null() {
        return Preview {
            icon: Default::default(),
            text: String::new(),
            typing: false,
        };
    }
    let icon = if b(lm, "hasCode") {
        icons.code_blocks.clone()
    } else {
        match s(lm, "kind") {
            "image" => icons.camera.clone(),
            "video" => icons.video_on.clone(),
            "audio" | "voice" => icons.mic_on.clone(),
            "file" => icons.attach.clone(),
            "call" => icons.phone.clone(),
            _ => Default::default(),
        }
    };
    let sender = s(lm, "senderName");
    let body = match (s(lm, "kind"), s(lm, "body")) {
        ("call", "") => "Call",
        (_, body) => body,
    };
    let text = if b(room, "isDm") || sender.is_empty() {
        body.to_string()
    } else {
        format!("{sender}: {body}")
    };
    Preview {
        icon,
        text,
        typing: false,
    }
}

/// HomePage.qml's badge: highlights count when highlighted, else unread;
/// invites show "!". Server counts are 0 for E2EE rooms, so combine.
pub fn badge_for(room: &Value) -> (String, bool) {
    let unread = n(room, "unread").max(n(room, "unreadMessages"));
    let highlights = n(room, "highlights");
    if b(room, "isInvite") {
        return ("!".into(), true);
    }
    let count = if highlights > 0 { highlights } else { unread };
    if count == 0 {
        return (String::new(), false);
    }
    let label = if count > 99 {
        "99+".into()
    } else {
        count.to_string()
    };
    (label, highlights > 0)
}

/// timeline/fmt.rs `short()` runs engine-side and arrives as `stamp`;
/// this is only for in-bubble times.
pub fn bubble_stamp(ts_ms: i64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_millis_opt(ts_ms).single() {
        Some(t) => t.format("%H:%M").to_string(),
        None => String::new(),
    }
}

/// How long a live share has left, as the chip says it: `H:MMm` above an
/// hour, `M:SS` below one, and nothing once it has run out.
///
/// This is the ONE derivation. The bubble's chip and the location page's chip
/// are meant to be the same chip — MapPage.qml:295 says so in as many words —
/// and they were not: the page asked the clock and the bubble reconstructed
/// "now" as `boot-epoch-s + fx-clock`, where `fx-clock` is a 50ms accumulator
/// that only advances while a row with effects is on screen (chat.slint:402-409).
/// It is not elapsed time and never was: it stops whenever nothing is
/// animating and the page is away, so `now` fell behind real time by however
/// long that had been. The countdown was over by exactly that much, whatever
/// the share's length — 15m showing 23m, 8h showing 8h07m, the same seven or
/// eight minutes both times.
pub fn live_remaining(expires_ms: i64, now_ms: i64) -> String {
    let left = ((expires_ms - now_ms) as f64 / 1000.0).max(0.0) as u64;
    if left == 0 {
        return String::new();
    }
    if left >= 3600 {
        format!("{}h {:02}m", left / 3600, (left % 3600) / 60)
    } else {
        format!("{}:{:02}", left / 60, left % 60)
    }
}

pub fn day_divider_label(ts_ms: i64) -> String {
    use chrono::{Datelike, Local, TimeZone};
    let Some(t) = Local.timestamp_millis_opt(ts_ms).single() else {
        return String::new();
    };
    let now = Local::now();
    let days = now
        .date_naive()
        .signed_duration_since(t.date_naive())
        .num_days();
    if days == 0 {
        return "Today".into();
    }
    if days == 1 {
        return "Yesterday".into();
    }
    if t.year() == now.year() {
        return t.format("%A, %-d %B").to_string();
    }
    t.format("%-d %B %Y").to_string()
}

/// Which timeline kinds draw as bubbles, which as centred state lines, and
/// which do not draw at all. The shadow list keeps every item (filtering the
/// model the diffs index into is how views desynchronise — ui-conventions.md);
/// only this projection skips rows.
pub enum RowShape {
    Bubble {
        media_icon: slint::SharedString,
        body_override: Option<String>,
    },
    State(String),
    Divider(String),
    Marker,
    Skip,
}

pub fn shape_for(item: &Value, icons: &IconSet) -> RowShape {
    let kind = s(item, "kind");
    match kind {
        "text" | "notice" | "emote" => RowShape::Bubble {
            media_icon: Default::default(),
            body_override: None,
        },
        "image" => RowShape::Bubble {
            media_icon: icons.camera.clone(),
            body_override: media_body(item, "Photo"),
        },
        "video" => RowShape::Bubble {
            media_icon: icons.video_on.clone(),
            body_override: media_body(item, "Video"),
        },
        "audio" | "voice" => RowShape::Bubble {
            media_icon: icons.mic_on.clone(),
            body_override: media_body(item, "Audio"),
        },
        "file" => RowShape::Bubble {
            media_icon: icons.attach.clone(),
            body_override: media_body(item, "File"),
        },
        "sticker" => RowShape::Bubble {
            media_icon: icons.sticker.clone(),
            body_override: media_body(item, "Sticker"),
        },
        "poll" => RowShape::Bubble {
            media_icon: icons.poll.clone(),
            body_override: Some("Poll".into()),
        },
        "redacted" => RowShape::Bubble {
            media_icon: Default::default(),
            body_override: Some("Message deleted".into()),
        },
        "utd" => RowShape::Bubble {
            media_icon: Default::default(),
            body_override: Some("Waiting for this message…".into()),
        },
        "unsupported" => RowShape::Bubble {
            media_icon: Default::default(),
            body_override: Some("Unsupported message".into()),
        },
        "dayDivider" => RowShape::Divider(day_divider_label(n(item, "ts"))),
        "membership" | "profile" | "state" | "call" | "rtcNotification" => {
            let text = match s(item, "stateText") {
                "" => s(item, "body").to_string(),
                t => t.to_string(),
            };
            if text.is_empty() {
                RowShape::Skip
            } else {
                RowShape::State(text)
            }
        }
        "timelineStart" => RowShape::State("Beginning of conversation".into()),
        "location" | "liveLocation" => RowShape::Bubble {
            media_icon: icons.location.clone(),
            body_override: Some(match s(item, "body") {
                "" => "Location".into(),
                b => b.to_string(),
            }),
        },
        "contact" => RowShape::Bubble {
            media_icon: icons.person.clone(),
            body_override: None,
        },
        "readMarker" => RowShape::Marker,
        "liveLocationEnd" => RowShape::Skip, // protocol noise (BubbleDelegate.hiddenItem)
        _ => RowShape::Skip,
    }
}

fn media_body(item: &Value, fallback: &str) -> Option<String> {
    let body = s(item, "body");
    if !body.is_empty() {
        return Some(body.to_string());
    }
    let name = item.get("media").map(|m| s(m, "filename")).unwrap_or("");
    Some(if name.is_empty() {
        fallback.to_string()
    } else {
        name.to_string()
    })
}

pub fn same_day(a_ms: i64, b_ms: i64) -> bool {
    use chrono::{Local, TimeZone};
    match (
        Local.timestamp_millis_opt(a_ms).single(),
        Local.timestamp_millis_opt(b_ms).single(),
    ) {
        (Some(a), Some(b)) => a.date_naive() == b.date_naive(),
        _ => false,
    }
}

/// Bubble grouping: consecutive messages from one sender within five minutes
/// on the same day share small corners.
pub fn same_group(a: &Value, bv: &Value) -> bool {
    let bubble = |v: &Value| {
        !matches!(
            s(v, "kind"),
            "dayDivider"
                | "membership"
                | "profile"
                | "state"
                | "call"
                | "rtcNotification"
                | "readMarker"
                | "timelineStart"
                | "liveLocationEnd"
        )
    };
    if !bubble(a) || !bubble(bv) {
        return false;
    }
    if s(a, "sender") != s(bv, "sender") {
        return false;
    }
    let (ta, tb) = (n(a, "ts"), n(bv, "ts"));
    (ta - tb).abs() <= 5 * 60 * 1000
}

/// BubbleDelegate.resampleWave: fit a waveform to n bars, peak-preserving.
pub fn resample_wave(arr: &[f64], n: usize) -> Vec<f32> {
    if arr.is_empty() || n == 0 {
        return Vec::new();
    }
    (0..n)
        .map(|i| {
            let a = i * arr.len() / n;
            let b = ((i + 1) * arr.len() / n).max(a + 1).min(arr.len());
            arr[a..b].iter().cloned().fold(0.0f64, f64::max) as f32
        })
        .collect()
}

/// Flatten sanitized/highlighted markup to plain text: tags out, entities back.
pub fn strip_markup(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ")
        .trim_matches('\n')
        .to_string()
}

/// First http(s) URL in a body — the QML's /https?:\/\/[^\s<>"]+/ match.
pub fn first_url(body: &str) -> Option<String> {
    let at = body.find("https://").or_else(|| body.find("http://"))?;
    let tail = &body[at..];
    let end = tail
        .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"'))
        .unwrap_or(tail.len());
    let url = &tail[..end];
    // Trim trailing punctuation a sentence would add.
    let url = url.trim_end_matches(['.', ',', ')', ']', '!', '?']);
    if url.len() > 10 {
        Some(url.to_string())
    } else {
        None
    }
}

/// Host part of a URL, with any leading www. dropped.
pub fn domain_of(url: &str) -> String {
    url.split("://")
        .nth(1)
        .unwrap_or("")
        .split('/')
        .next()
        .unwrap_or("")
        .trim_start_matches("www.")
        .to_string()
}

/// The engine's sanitized rich-text subset (<b> <i> <code> <a href> <br> <font>)
/// → CommonMark for StyledText. Literal markdown characters in text runs are
/// escaped so message text never becomes accidental markup.
pub fn html_to_markdown(html: &str) -> String {
    let mut out = String::with_capacity(html.len() + 16);
    let mut rest = html;
    let mut in_code = false;
    while let Some(open) = rest.find('<') {
        push_md_escaped(&mut out, &rest[..open], in_code);
        let Some(close) = rest[open..].find('>').map(|p| open + p) else {
            push_md_escaped(&mut out, &rest[open..], in_code);
            return out;
        };
        let tag = &rest[open + 1..close];
        rest = &rest[close + 1..];
        let lower = tag.to_ascii_lowercase();
        let name = lower.trim_start_matches('/');
        let name: String = name
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        let closing = lower.starts_with('/');
        match (name.as_str(), closing) {
            ("b" | "strong", _) => out.push_str("**"),
            ("i" | "em", _) => out.push('*'),
            ("del" | "s" | "strike", _) => out.push_str("~~"),
            ("code", c) => {
                in_code = !c;
                out.push('`');
            }
            ("br", _) => out.push_str("  \n"),
            ("p", true) => out.push_str("\n\n"),
            // from_markdown rejects headings/blockquotes; downgrade to what it
            // accepts: bold paragraphs and italic quote lines.
            ("h1" | "h2" | "h3" | "h4" | "h5" | "h6", false) => out.push_str("\n\n**"),
            ("h1" | "h2" | "h3" | "h4" | "h5" | "h6", true) => out.push_str("**\n\n"),
            ("blockquote", false) => out.push_str("\n\n*❯ "),
            ("blockquote", true) => out.push_str("*\n\n"),
            ("ul" | "ol", true) => out.push_str("\n"),
            ("li", false) => out.push_str("\n• "),
            ("a", false) => {
                // <a href="X">text</a> → [text](X)
                let href = lower
                    .find("href=\"")
                    .map(|h| &tag[h + 6..])
                    .and_then(|t| t.find('"').map(|e| &t[..e]))
                    .unwrap_or("")
                    .to_string();
                out.push('[');
                // find the closing </a> and emit its inner text
                if let Some(end) = rest.to_ascii_lowercase().find("</a>") {
                    push_md_escaped(&mut out, &rest[..end], false);
                    rest = &rest[end + 4..];
                }
                out.push_str("](");
                out.push_str(&href);
                out.push(')');
            }
            _ => {} // font/span colouring and unknown tags: styling dropped, text kept
        }
    }
    push_md_escaped(&mut out, rest, in_code);
    out
}

fn push_md_escaped(out: &mut String, text: &str, in_code: bool) {
    for ch in text.chars() {
        if !in_code && matches!(ch, '*' | '_' | '`' | '[' | ']' | '\\' | '#' | '~') {
            out.push('\\');
        }
        out.push(ch);
    }
}

/// SigilText static colours: effect spans (char offsets, engine-resolved dark
/// hex) → markdown with `<font color>` runs for StyledText. Geometry
/// animations stay out (they need the glyph-run project); colour is 1:1.
pub fn effects_markdown(body: &str, effects: &Value) -> Option<String> {
    let chars: Vec<char> = body.chars().collect();
    let (colors, any) = effect_char_colors(&chars, effects)?;
    if !any {
        return None;
    }
    // Emit runs: consecutive characters sharing a colour share one font tag.
    let mut out = String::with_capacity(body.len() * 2);
    let mut i = 0;
    while i < chars.len() {
        let color = colors[i].clone();
        let mut j = i;
        while j < chars.len() && colors[j] == color {
            j += 1;
        }
        let run: String = chars[i..j].iter().collect();
        match color {
            Some(hex) => {
                out.push_str(&format!("<font color=\"{hex}\">"));
                push_md_escaped(&mut out, &run, false);
                out.push_str("</font>");
            }
            None => push_md_escaped(&mut out, &run, false),
        }
        i = j;
    }
    Some(out)
}

/// Per-character colour from effect spans, later effects painting over earlier.
/// Which of an effect's two swatches applies: the composer stores a dark and
/// a light hex per colour, and the phone follows the system setting.
pub static DARK_SCHEME: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

fn scheme_hex(rgb: &Value) -> Option<String> {
    let dark = DARK_SCHEME.load(std::sync::atomic::Ordering::Relaxed);
    let (first, other) = if dark { ("dark", "light") } else { ("light", "dark") };
    rgb[first]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| rgb[other].as_str().filter(|s| !s.is_empty()))
        .map(str::to_string)
}

fn luminance((r, g, b): (u8, u8, u8)) -> f32 {
    let lin = |c: u8| {
        let c = c as f32 / 255.0;
        if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

/// A colour picked for one scheme, kept readable on the bubble it lands on:
/// too dark for a dark ground is lifted toward white, too light for a light
/// ground is pulled toward black, hue kept. The floors are what a bubble's
/// container needs for 4.5:1 against the messenger's greys.
fn legible((r, g, b): (u8, u8, u8)) -> (u8, u8, u8) {
    let dark = DARK_SCHEME.load(std::sync::atomic::Ordering::Relaxed);
    let (floor, target) = if dark { (0.35, 255.0) } else { (0.15, 0.0) };
    let y = luminance((r, g, b));
    if (dark && y >= floor) || (!dark && y <= floor) {
        return (r, g, b);
    }
    let mix = |t: f32| {
        let m = |c: u8| (c as f32 + (target - c as f32) * t).round() as u8;
        (m(r), m(g), m(b))
    };
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..12 {
        let mid = (lo + hi) / 2.0;
        let y = luminance(mix(mid));
        if (dark && y < floor) || (!dark && y > floor) { lo = mid } else { hi = mid }
    }
    mix(hi)
}

fn hex_of((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

pub(crate) fn effect_char_colors(
    chars: &[char],
    effects: &Value,
) -> Option<(Vec<Option<String>>, bool)> {
    let list = effects.as_array()?;
    if list.is_empty() {
        return None;
    }
    let mut colors: Vec<Option<String>> = vec![None; chars.len()];
    let mut any = false;
    for e in list {
        let start = e["start"].as_u64().unwrap_or(0) as usize;
        let end = (e["end"].as_u64().unwrap_or(0) as usize).min(chars.len());
        if start >= end {
            continue;
        }
        let c = &e["color"];
        match c["type"].as_str() {
            Some("solid") => {
                let Some(hex) = scheme_hex(&c["rgb"]).and_then(|h| parse_hex(&h)).map(|rgb| hex_of(legible(rgb))) else {
                    continue;
                };
                for slot in &mut colors[start..end] {
                    *slot = Some(hex.clone());
                }
                any = true;
            }
            Some("gradient") => {
                let stops: Vec<(u8, u8, u8)> = c["stops"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|s| scheme_hex(s).and_then(|h| parse_hex(&h)).map(legible))
                            .collect()
                    })
                    .unwrap_or_default();
                if stops.len() < 2 {
                    continue;
                }
                let n = (end - start).max(1) as f32;
                for (k, slot) in colors[start..end].iter_mut().enumerate() {
                    let t = k as f32 / (n - 1.0).max(1.0) * (stops.len() - 1) as f32;
                    let i = (t as usize).min(stops.len() - 2);
                    let f = t - i as f32;
                    let (a, b) = (stops[i], stops[i + 1]);
                    *slot = Some(format!(
                        "#{:02x}{:02x}{:02x}",
                        (a.0 as f32 + (b.0 as f32 - a.0 as f32) * f) as u8,
                        (a.1 as f32 + (b.1 as f32 - a.1 as f32) * f) as u8,
                        (a.2 as f32 + (b.2 as f32 - a.2 as f32) * f) as u8
                    ));
                }
                any = true;
            }
            Some("rainbow") => {
                let n = (end - start).max(1) as f32;
                for (k, slot) in colors[start..end].iter_mut().enumerate() {
                    *slot = Some(hex_of(legible(hue_rgb(k as f32 / n))));
                }
                any = true;
            }
            _ => {}
        }
    }
    Some((colors, any))
}

/// Per-character rows for an animated short run (per-glyph motion): each char
/// carries its colour and its span's animation. Flip runs come out reversed
/// (the spec's reverseRun). None when the body is long or nothing animates.
/// `fresh` gates the one-shot reveal: an old message arrives fully typed.
pub fn effect_fx_chars(
    body: &str,
    effects: &Value,
    fresh: bool,
) -> Option<Vec<(String, Option<String>, String, i32)>> {
    const ANIMS: &[&str] = &[
        "wave",
        "shake",
        "pulse",
        "glow",
        "barrel",
        "flip",
        "typewriter",
        "glitch",
        "sparkle",
        "blur",
    ];
    let chars: Vec<char> = body.chars().collect();
    if chars.is_empty() || chars.len() > 48 || body.contains('\n') {
        return None;
    }
    let list = effects.as_array()?;
    let mut anims: Vec<&str> = vec![""; chars.len()];
    let mut animated = false;
    for e in list {
        let Some(a) = e["animation"].as_str() else {
            continue;
        };
        if !ANIMS.contains(&a) {
            continue;
        }
        if a == "typewriter" && !fresh {
            continue;
        }
        let start = e["start"].as_u64().unwrap_or(0) as usize;
        let end = (e["end"].as_u64().unwrap_or(0) as usize).min(chars.len());
        for slot in &mut anims[start..end.max(start)] {
            *slot = a;
        }
        animated = start < end || animated;
    }
    if !animated {
        return None;
    }
    let (colors, _) =
        effect_char_colors(&chars, effects).unwrap_or((vec![None; chars.len()], false));
    let mut out: Vec<(String, Option<String>, String, i32)> = chars
        .iter()
        .enumerate()
        .map(|(i, c)| {
            (
                c.to_string(),
                colors[i].clone(),
                anims[i].to_string(),
                i as i32,
            )
        })
        .collect();
    // reverseRun: a flip span reads back to front.
    let mut i = 0;
    while i < out.len() {
        if out[i].2 == "flip" {
            let mut j = i;
            while j < out.len() && out[j].2 == "flip" {
                j += 1;
            }
            out[i..j].reverse();
            i = j;
        } else {
            i += 1;
        }
    }
    Some(out)
}

fn parse_hex(hex: &str) -> Option<(u8, u8, u8)> {
    let h = hex.trim_start_matches('#');
    if h.len() < 6 {
        return None;
    }
    Some((
        u8::from_str_radix(&h[0..2], 16).ok()?,
        u8::from_str_radix(&h[2..4], 16).ok()?,
        u8::from_str_radix(&h[4..6], 16).ok()?,
    ))
}

fn hue_rgb(t: f32) -> (u8, u8, u8) {
    let h = (t.fract() * 6.0).abs();
    let x = (1.0 - (h % 2.0 - 1.0).abs()) * 255.0;
    let (r, g, b) = match h as u32 {
        0 => (255.0, x, 0.0),
        1 => (x, 255.0, 0.0),
        2 => (0.0, 255.0, x),
        3 => (0.0, x, 255.0),
        4 => (x, 0.0, 255.0),
        _ => (255.0, 0.0, x),
    };
    (r as u8, g as u8, b as u8)
}

/// "#rrggbb" → a slint colour.
pub fn hex_color(hex: &str) -> Option<slint::Color> {
    let (r, g, b) = parse_hex(hex)?;
    Some(slint::Color::from_rgb_u8(r, g, b))
}

/// `@name:server` → `name`; anything else unchanged.
pub fn localpart(user: &str) -> String {
    user.trim_start_matches('@')
        .split(':')
        .next()
        .unwrap_or(user)
        .to_string()
}

/// HomePage.qml fmtTime: HH:mm today, Yesterday, the weekday inside a week,
/// then "d MMM".
pub fn home_stamp(ts_ms: i64) -> String {
    use chrono::{Local, TimeZone};
    if ts_ms <= 0 {
        return String::new();
    }
    let Some(t) = Local.timestamp_millis_opt(ts_ms).single() else {
        return String::new();
    };
    let now = Local::now();
    let days = now
        .date_naive()
        .signed_duration_since(t.date_naive())
        .num_days();
    if days == 0 {
        return t.format("%H:%M").to_string();
    }
    if days == 1 {
        return "Yesterday".to_string();
    }
    if days < 7 {
        return t.format("%a").to_string();
    }
    t.format("%-d %b").to_string()
}

/// A filename without its extension (BubbleDelegate.qml:1408-1412).
pub fn strip_extension(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 => name[..i].to_string(),
        _ => name.to_string(),
    }
}

/// Keep the head and tail of a long name with "…" between, the stand-in for
/// Text.ElideMiddle (Slint only elides at the end and cannot measure text).
pub fn elide_middle(name: &str, max_chars: usize) -> String {
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= max_chars {
        return name.to_string();
    }
    let tail = 12.min(max_chars / 3);
    let head = max_chars.saturating_sub(tail + 1);
    format!(
        "{}…{}",
        chars[..head].iter().collect::<String>(),
        chars[chars.len() - tail..].iter().collect::<String>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixed timestamps, no clock: the three durations the attach sheet
    /// offers, seen seven and a half minutes after the share began — which is
    /// the case the bubble got wrong, and it got it wrong by exactly that
    /// seven and a half minutes.
    #[test]
    fn the_countdown_counts_down_from_now_not_from_when_the_app_started() {
        let start = 1_764_000_000_000i64; // some fixed instant
        let elapsed = 7 * 60 * 1000 + 30 * 1000; // 7m30s in
        let now = start + elapsed;
        // 15 minutes: 7m30s left, not 22m30s.
        assert_eq!(live_remaining(start + 900_000, now), "7:30");
        // 1 hour: 52m30s left, not 1h07m.
        assert_eq!(live_remaining(start + 3_600_000, now), "52:30");
        // 8 hours: 7h52m left, not 8h07m.
        assert_eq!(live_remaining(start + 28_800_000, now), "7h 52m");
    }

    #[test]
    fn an_hour_is_where_the_chip_changes_shape() {
        let now = 0i64;
        // Under the hour it is minutes and seconds …
        assert_eq!(live_remaining(3_599_000, now), "59:59");
        // … and from the hour it is hours and padded minutes.
        assert_eq!(live_remaining(3_600_000, now), "1h 00m");
        assert_eq!(live_remaining(3_660_000, now), "1h 01m");
        assert_eq!(live_remaining(28_800_000, now), "8h 00m");
    }

    #[test]
    fn seconds_are_padded_and_a_finished_share_says_nothing() {
        let now = 0i64;
        assert_eq!(live_remaining(65_000, now), "1:05");
        assert_eq!(live_remaining(9_000, now), "0:09");
        assert_eq!(live_remaining(60_000, now), "1:00");
        // Run out, and run out a while ago: no negative clocks.
        assert_eq!(live_remaining(0, now), "");
        assert_eq!(live_remaining(999, now), "");
        assert_eq!(live_remaining(-500_000, now), "");
    }

    /// The bubble draws its own chip in Slint from `expires-s − now-epoch-s`,
    /// so the two must agree at every second, not merely look alike. Walking
    /// a whole share second by second against the same arithmetic the Slint
    /// expression performs is the cheapest way to know they do.
    #[test]
    fn the_bubbles_arithmetic_and_this_one_agree_at_every_second() {
        let expires_ms = 8 * 3_600_000i64;
        for now_s in (0..8 * 3600).step_by(7) {
            let left = (expires_ms / 1000 - now_s).max(0);
            // Exactly what bubble.slint computes from `live-remaining`.
            let slint = if left >= 3600 {
                format!("{}h {}{}m", left / 3600, if (left % 3600) / 60 < 10 { "0" } else { "" }, (left % 3600) / 60)
            } else {
                format!("{}:{}{}", left / 60, if left % 60 < 10 { "0" } else { "" }, left % 60)
            };
            assert_eq!(live_remaining(expires_ms, now_s * 1000), slint, "at {now_s}s");
        }
    }
}
