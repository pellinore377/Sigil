//! Sigil's modifier syntax: `modifier[::modifier…]::content;`, run after pulldown-cmark
//! over `Text` events only. Effect ranges are **character** offsets into the plain body —
//! a byte range would split a codepoint. A token is a modifier only if it names a known
//! animation or colour, which is what keeps `std::vector` text.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Colour names, mid variant first. Suffixes 1..3 set brightness; bare = the `2` variant.
const HUES: &[&str] = &["red", "orange", "yellow", "green", "cyan", "blue", "purple", "pink", "gray"];

#[cfg_attr(not(test), allow(dead_code))]
const ANIMATIONS: &[&str] = &[
    "shake", "wave", "pulse", "glow", "typewriter",
    "sparkle", "glitch", "blur", "flip", "barrel",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ColorSpec {
    Solid { name: String },
    Gradient { stops: Vec<String> },
    Rainbow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Animation {
    Shake,
    Wave,
    Pulse,
    Glow,
    Typewriter,
    Sparkle,
    Glitch,
    Blur,
    Flip,
    Barrel,
}

impl Animation {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "shake" => Animation::Shake,
            "wave" => Animation::Wave,
            "pulse" => Animation::Pulse,
            "glow" => Animation::Glow,
            "typewriter" => Animation::Typewriter,
            "sparkle" => Animation::Sparkle,
            "glitch" => Animation::Glitch,
            "blur" => Animation::Blur,
            "flip" => Animation::Flip,
            "barrel" => Animation::Barrel,
            _ => return None,
        })
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Animation::Shake => "shake",
            Animation::Wave => "wave",
            Animation::Pulse => "pulse",
            Animation::Glow => "glow",
            Animation::Typewriter => "typewriter",
            Animation::Sparkle => "sparkle",
            Animation::Glitch => "glitch",
            Animation::Blur => "blur",
            Animation::Flip => "flip",
            Animation::Barrel => "barrel",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TextEffect {
    /// Character offsets into the plain body.
    pub start: usize,
    pub end: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub animation: Option<Animation>,
    /// `||spoiler||` or the `spoiler` keyword. Also emitted in the HTML.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub spoiler: bool,
    /// `underline`. Emits `<u>`, outside the Matrix HTML whitelist; strict clients may strip it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub underline: bool,
    /// `mark`. Highlight behind the text; composes with the colour set.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mark: bool,
    /// `mono`. Monospace styling only — not a code span, nothing is literal.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mono: bool,
    /// The Markdown keywords, so the two syntaxes compose without mixing.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bold: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub italic: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub strike: bool,
    /// `small1..3` / `big1..3` as -3..=3. Clamped at the parser.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub size: i8,
}

fn is_zero(n: &i8) -> bool { *n == 0 }

/// Largest step either way. `small4` fails classification and stays literal.
pub const MAX_SIZE_STEP: i8 = 3;

impl TextEffect {
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

fn is_colour(tok: &str) -> bool {
    if tok == "rainbow" { return true }
    let base = tok.strip_suffix(['1', '2', '3']).unwrap_or(tok);
    // `red12` must not pass: only one suffix digit is allowed.
    if base.len() + 1 < tok.len() { return false }
    HUES.contains(&base)
}

/// Canonical colour form: a bare hue means its `2` variant.
fn canon_colour(tok: &str) -> String {
    if tok == "rainbow" { return tok.to_string() }
    if tok.ends_with(['1', '2', '3']) { return tok.to_string() }
    format!("{tok}2")
}

/// What one `::`-separated token means, if anything.
enum Modifier {
    Colour(ColorSpec),
    Anim(Animation),
    Underline,
    Mark,
    Mono,
    Bold,
    Italic,
    Strike,
    Spoiler,
    /// -3..=3, never zero.
    Size(i8),
}

/// `small1..3` / `big1..3`, and the bare aliases meaning step 2.
fn size_step(tok: &str) -> Option<i8> {
    let (word, sign) = if let Some(r) = tok.strip_prefix("small") { (r, -1i8) }
        else if let Some(r) = tok.strip_prefix("big") { (r, 1i8) }
        else { return None };
    if word.is_empty() { return Some(2 * sign) }
    let n: i8 = word.parse().ok()?;
    // Out of range is not a modifier; saturating would restyle deliberate text.
    if !(1..=MAX_SIZE_STEP).contains(&n) { return None }
    Some(n * sign)
}

fn classify(tok: &str) -> Option<Modifier> {
    if tok.is_empty() { return None }
    if let Some(a) = Animation::parse(tok) { return Some(Modifier::Anim(a)) }
    match tok {
        "underline" => return Some(Modifier::Underline),
        "mark" => return Some(Modifier::Mark),
        "mono" => return Some(Modifier::Mono),
        "bold" => return Some(Modifier::Bold),
        "italic" => return Some(Modifier::Italic),
        "strike" => return Some(Modifier::Strike),
        "spoiler" => return Some(Modifier::Spoiler),
        _ => {}
    }
    if let Some(n) = size_step(tok) { return Some(Modifier::Size(n)) }
    if tok == "rainbow" { return Some(Modifier::Colour(ColorSpec::Rainbow)) }
    if is_colour(tok) {
        return Some(Modifier::Colour(ColorSpec::Solid { name: canon_colour(tok) }))
    }
    // A gradient is hyphen-joined colours, so no colour name may contain a hyphen.
    if tok.contains('-') {
        let parts: Vec<&str> = tok.split('-').collect();
        if parts.len() >= 2 && parts.iter().all(|p| is_colour(p) && *p != "rainbow") {
            return Some(Modifier::Colour(ColorSpec::Gradient {
                stops: parts.iter().map(|p| canon_colour(p)).collect(),
            }))
        }
    }
    None
}

/// Everything one pass over a `Text` event produced.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Parsed {
    /// The text with the modifier syntax removed.
    pub text: String,
    /// Ranges within `text`, in characters, relative to its start.
    pub effects: Vec<TextEffect>,
}

/// Parse one run of plain text; `base` is its start in the whole body, so ranges are absolute.
pub fn parse_run(input: &str, base: usize) -> Parsed {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut effects: Vec<TextEffect> = Vec::new();
    let mut i = 0usize;
    let mut col = 0usize;

    while i < chars.len() {
        // An escape passes the next character through, so `\red::` no longer classifies.
        if chars[i] == '\\' && i + 1 < chars.len() {
            out.push(chars[i + 1]);
            col += 1;
            i += 2;
            continue
        }

        if chars[i] == '|' && i + 1 < chars.len() && chars[i + 1] == '|' {
            if let Some((body, next)) = take_spoiler(&chars, i) {
                let inner = parse_run(&body, base + col);
                let start = base + col;
                out.push_str(&inner.text);
                col += inner.text.chars().count();
                effects.push(TextEffect { start, end: base + col, spoiler: true, ..Default::default() });
                effects.extend(inner.effects);
                i = next;
                continue
            }
        }

        if let Some((mods, content_start)) = take_modifiers(&chars, i) {
            let (content, next, _terminated) = take_content(&chars, content_start);
            // Empty content is a no-op rather than a zero-width span.
            if content.is_empty() { i = next; continue }

            let inner = parse_run(&content, base + col);
            let start = base + col;
            out.push_str(&inner.text);
            col += inner.text.chars().count();

            let mut fx = TextEffect { start, end: base + col, ..Default::default() };
            for m in mods {
                match m {
                    // Last animation wins, and last size: `big3::small1::x;` is small1.
                    Modifier::Anim(a) => fx.animation = Some(a),
                    Modifier::Colour(c) => fx.color = Some(c),
                    Modifier::Size(n) => fx.size = n,
                    Modifier::Underline => fx.underline = true,
                    Modifier::Mark => fx.mark = true,
                    Modifier::Mono => fx.mono = true,
                    Modifier::Bold => fx.bold = true,
                    Modifier::Italic => fx.italic = true,
                    Modifier::Strike => fx.strike = true,
                    Modifier::Spoiler => fx.spoiler = true,
                }
            }
            fx.end = base + col;
            effects.push(fx);
            effects.extend(inner.effects);
            i = next;
            continue
        }

        out.push(chars[i]);
        col += 1;
        i += 1;
    }

    Parsed { text: out, effects }
}

/// Read `mod::mod::` at `i`. `None` unless every token classifies — that keeps `std::vector` text.
fn take_modifiers(chars: &[char], i: usize) -> Option<(Vec<Modifier>, usize)> {
    let mut mods = Vec::new();
    let mut j = i;
    loop {
        let start = j;
        while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '-') { j += 1 }
        if j == start { return None }
        if j + 1 >= chars.len() || chars[j] != ':' || chars[j + 1] != ':' { return None }
        let tok: String = chars[start..j].iter().collect::<String>().to_ascii_lowercase();
        mods.push(classify(&tok)?);
        j += 2;
        let peek_start = j;
        let mut k = j;
        while k < chars.len() && (chars[k].is_ascii_alphanumeric() || chars[k] == '-') { k += 1 }
        let another = k > peek_start && k + 1 < chars.len() && chars[k] == ':' && chars[k + 1] == ':';
        if !another { return Some((mods, j)) }
    }
}

/// Content up to the first unescaped `;` (escapes left in place for the recursive parse),
/// where to resume, and whether a terminator was found. Unterminated runs to end of line.
fn take_content(chars: &[char], from: usize) -> (String, usize, bool) {
    let mut body = String::new();
    let mut i = from;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            body.push(chars[i]);
            body.push(chars[i + 1]);
            i += 2;
            continue
        }
        if chars[i] == ';' { return (body, i + 1, true) }
        if chars[i] == '\n' { return (body, i, false) }
        body.push(chars[i]);
        i += 1;
    }
    (body, i, false)
}

/// `||…||`, returning the inner text and where to resume.
fn take_spoiler(chars: &[char], i: usize) -> Option<(String, usize)> {
    let mut body = String::new();
    let mut j = i + 2;
    while j < chars.len() {
        if chars[j] == '\\' && j + 1 < chars.len() {
            body.push(chars[j]);
            body.push(chars[j + 1]);
            j += 2;
            continue
        }
        if chars[j] == '|' && j + 1 < chars.len() && chars[j + 1] == '|' {
            if body.is_empty() { return None }
            return Some((body, j + 2))
        }
        body.push(chars[j]);
        j += 1;
    }
    None
}

/// A composed message: `body`, `formatted_body`, and `com.sigil.text_effects`.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Composed {
    /// Plain text. Effect offsets index into this.
    pub body: String,
    /// Markdown-derived HTML. Colours and animations stay out — they are Sigil's own field.
    pub html: String,
    pub effects: Vec<TextEffect>,
}

impl Composed {
    pub fn has_effects(&self) -> bool { !self.effects.is_empty() }
}

fn esc(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}

/// Turn typed input into `body`, `formatted_body` and the effects field.
pub fn compose(src: &str) -> Composed {
    use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);

    let mut body = String::new();
    let mut effects: Vec<TextEffect> = Vec::new();
    let mut events: Vec<Event> = Vec::new();
    let mut in_code = false;

    for ev in Parser::new_ext(src, opts) {
        match ev {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(_) | CodeBlockKind::Indented)) => {
                in_code = true;
                events.push(ev);
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code = false;
                events.push(ev);
            }
            // Inside a fence the text is the code: pass it through untouched.
            Event::Text(ref t) if in_code => {
                body.push_str(t);
                events.push(ev.clone());
            }
            Event::Text(t) => {
                let base = body.chars().count();
                let parsed = parse_run(&t, base);
                body.push_str(&parsed.text);

                let mut frag = String::new();
                let mut cursor = 0usize;
                let chars: Vec<char> = parsed.text.chars().collect();
                // Spoiler, underline and mark also go in the HTML; colours, sizes and animations do not.
                let mut spoilers: Vec<&TextEffect> =
                    parsed.effects.iter().filter(|e| e.spoiler || e.underline || e.mark).collect();
                spoilers.sort_by_key(|e| e.start);
                for sp in spoilers {
                    let (s, e) = (sp.start - base, sp.end - base);
                    if s < cursor || e > chars.len() { continue }
                    esc(&chars[cursor..s].iter().collect::<String>(), &mut frag);
                    let mut close = String::new();
                    if sp.spoiler { frag.push_str("<span data-mx-spoiler>"); close.insert_str(0, "</span>") }
                    if sp.mark {
                        // MSC2530 background colour; the effect's own colour if it has one.
                        let hue = match &sp.color {
                            Some(ColorSpec::Solid { name }) => name.trim_end_matches(['1','2','3']).to_string(),
                            _ => "yellow".to_string(),
                        };
                        frag.push_str(&format!("<span data-mx-bg-color=\"{hue}\">"));
                        close.insert_str(0, "</span>");
                    }
                    if sp.underline { frag.push_str("<u>"); close.insert_str(0, "</u>") }
                    esc(&chars[s..e].iter().collect::<String>(), &mut frag);
                    frag.push_str(&close);
                    cursor = e;
                }
                if cursor == 0 {
                    events.push(Event::Text(parsed.text.clone().into()));
                } else {
                    esc(&chars[cursor..].iter().collect::<String>(), &mut frag);
                    events.push(Event::Html(frag.into()));
                }
                effects.extend(parsed.effects);
            }
            // A code span's contents are literal by construction.
            Event::Code(ref c) => { body.push_str(c); events.push(ev.clone()); }
            Event::SoftBreak | Event::HardBreak => { body.push('\n'); events.push(ev); }
            Event::End(TagEnd::Paragraph)
            | Event::End(TagEnd::Heading(_))
            | Event::End(TagEnd::Item)
            | Event::End(TagEnd::BlockQuote(_)) => {
                body.push('\n');
                events.push(ev);
            }
            other => events.push(other),
        }
    }

    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, events.into_iter());

    Composed {
        body: body.trim_end().to_string(),
        html: html.trim_end().to_string(),
        effects,
    }
}

/// Effects as the JSON that rides on the event, with the palette already applied.
///
/// Frontends get `#RRGGBB` and a size multiplier rather than a token to look up:
/// a client resolving `red1` itself would drift from every other client, and
/// SigilText would stop meaning the same thing everywhere. See `palette`.
pub fn to_json(effects: &[TextEffect]) -> Value {
    json!(effects.iter().map(|e| resolved_json(e)).collect::<Vec<_>>())
}

/// Both grounds ride along; the frontend knows which one it is drawing on.
fn ground_pair((dark, light): (String, String)) -> Value {
    json!({ "dark": dark, "light": light })
}

fn resolved_json(e: &TextEffect) -> Value {
    let mut v = e.to_json();
    let Some(obj) = v.as_object_mut() else { return v };

    if e.size != 0 {
        obj.insert("sizeScale".into(), json!(super::palette::size_scale(e.size)));
    }

    // `mark` paints a highlight, which needs a colour even when the span sets none.
    if e.mark {
        let own = match &e.color {
            Some(ColorSpec::Solid { name }) => super::palette::resolve(name),
            Some(ColorSpec::Gradient { stops }) => stops.first().and_then(|s| super::palette::resolve(s)),
            _ => None,
        };
        let pair = own.or_else(|| super::palette::resolve(super::palette::MARK_DEFAULT));
        if let Some(p) = pair { obj.insert("markRgb".into(), ground_pair(p)); }
    }

    let Some(c) = obj.get_mut("color").and_then(Value::as_object_mut) else { return v };
    match c.get("type").and_then(Value::as_str) {
        Some("solid") => {
            let p = c.get("name").and_then(Value::as_str).and_then(super::palette::resolve);
            if let Some(p) = p { c.insert("rgb".into(), ground_pair(p)); }
        }
        Some("gradient") => {
            let stops: Vec<Value> = c.get("stops").and_then(Value::as_array).map(|a| {
                a.iter()
                    .map(|s| s.as_str().and_then(super::palette::resolve).map_or(Value::Null, ground_pair))
                    .collect()
            }).unwrap_or_default();
            c.insert("rgb".into(), json!(stops));
        }
        Some("rainbow") => {
            c.insert("saturation".into(), json!(super::palette::RAINBOW_SATURATION));
            c.insert("lightness".into(), json!(super::palette::RAINBOW_LIGHTNESS));
        }
        _ => {}
    }
    v
}

/// Read them back off a received event.
pub fn from_json(v: &Value) -> Vec<TextEffect> {
    v.as_array()
        .map(|a| a.iter().filter_map(|e| serde_json::from_value(e.clone()).ok()).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Parsed { parse_run(s, 0) }

    fn solid(name: &str) -> Option<ColorSpec> {
        Some(ColorSpec::Solid { name: name.to_string() })
    }

    #[test]
    fn the_wire_carries_resolved_colour_not_just_a_token() {
        let p = parse("red::text;");
        let v = to_json(&p.effects);
        let c = &v[0]["color"];
        assert_eq!(c["name"], "red2");
        assert!(c["rgb"]["dark"].as_str().is_some_and(|h| h.starts_with('#')), "{c}");
        assert!(c["rgb"]["light"].as_str().is_some_and(|h| h.starts_with('#')), "{c}");
    }

    #[test]
    fn gradient_stops_and_rainbow_constants_are_resolved_too() {
        let g = to_json(&parse("red1-blue3::x;").effects);
        let stops = g[0]["color"]["rgb"].as_array().expect("gradient rgb array");
        assert_eq!(stops.len(), 2);
        assert!(stops.iter().all(|s| s["dark"].as_str().is_some_and(|h| h.starts_with('#'))), "{stops:?}");

        let r = to_json(&parse("rainbow::x;").effects);
        assert_eq!(r[0]["color"]["saturation"], 0.62);
        assert_eq!(r[0]["color"]["lightness"], 0.62);
    }

    #[test]
    fn a_size_step_ships_its_multiplier() {
        let v = to_json(&parse("big2::x;").effects);
        assert_eq!(v[0]["sizeScale"], 1.4);
        assert!(to_json(&parse("plain").effects).get(0).is_none());
    }

    #[test]
    fn a_plain_colour_span() {
        let p = parse("red::text;");
        assert_eq!(p.text, "text");
        assert_eq!(p.effects.len(), 1);
        assert_eq!(p.effects[0].start, 0);
        assert_eq!(p.effects[0].end, 4);
        assert_eq!(p.effects[0].color, solid("red2"));
    }

    #[test]
    fn brightness_variants_survive_and_bare_names_normalise() {
        assert_eq!(parse("red1::x;").effects[0].color, solid("red1"));
        assert_eq!(parse("red3::x;").effects[0].color, solid("red3"));
        assert_eq!(parse("gray::x;").effects[0].color, solid("gray2"));
    }

    #[test]
    fn adjacent_spans_are_independent() {
        let p = parse("red::A;blue::B;");
        assert_eq!(p.text, "AB");
        assert_eq!(p.effects.len(), 2);
        assert_eq!((p.effects[0].start, p.effects[0].end), (0, 1));
        assert_eq!((p.effects[1].start, p.effects[1].end), (1, 2));
        assert_eq!(p.effects[1].color, solid("blue2"));
    }

    #[test]
    fn the_specs_rainbow_example() {
        let p = parse("red::R;orange::A;yellow::I;");
        assert_eq!(p.text, "RAI");
        assert_eq!(p.effects.len(), 3);
        assert_eq!(p.effects[2].color, solid("yellow2"));
    }

    #[test]
    fn gradients_take_two_or_more_stops() {
        let p = parse("red1-blue3::text;");
        assert_eq!(p.text, "text");
        assert_eq!(
            p.effects[0].color,
            Some(ColorSpec::Gradient { stops: vec!["red1".into(), "blue3".into()] })
        );
        let three = parse("red-yellow-green::x;");
        assert_eq!(
            three.effects[0].color,
            Some(ColorSpec::Gradient { stops: vec!["red2".into(), "yellow2".into(), "green2".into()] })
        );
    }

    #[test]
    fn rainbow_is_a_colour_of_its_own() {
        assert_eq!(parse("rainbow::RAINBOW;").effects[0].color, Some(ColorSpec::Rainbow));
        let one = parse("rainbow::A;");
        assert_eq!(one.text, "A");
        assert_eq!(one.effects[0].color, Some(ColorSpec::Rainbow));
    }

    #[test]
    fn animations_combine_with_colour_in_either_order() {
        let a = parse("shake::red::text;");
        let b = parse("red::shake::text;");
        assert_eq!(a.text, "text");
        assert_eq!(a.effects[0].animation, Some(Animation::Shake));
        assert_eq!(a.effects[0].color, solid("red2"));
        assert_eq!(a.effects[0], b.effects[0]);
    }

    #[test]
    fn the_last_animation_wins() {
        let p = parse("shake::wave::x;");
        assert_eq!(p.effects[0].animation, Some(Animation::Wave));
    }

    #[test]
    fn a_shaking_gradient() {
        let p = parse("shake::red1-blue3::text;");
        assert_eq!(p.text, "text");
        assert_eq!(p.effects[0].animation, Some(Animation::Shake));
        assert!(matches!(p.effects[0].color, Some(ColorSpec::Gradient { .. })));
    }

    #[test]
    fn a_non_colour_word_is_left_completely_alone() {
        let p = parse("std::vector<int> v;");
        assert_eq!(p.text, "std::vector<int> v;");
        assert!(p.effects.is_empty());
        assert!(parse("http://example.com").effects.is_empty());
        assert_eq!(parse("Foo::Bar::baz;").text, "Foo::Bar::baz;");
    }

    #[test]
    fn an_unterminated_span_runs_to_the_end_of_the_line() {
        let p = parse("red::text");
        assert_eq!(p.text, "text");
        assert_eq!((p.effects[0].start, p.effects[0].end), (0, 4));
        let two = parse("red::text\nplain");
        assert_eq!(two.text, "text\nplain");
        assert_eq!(two.effects[0].end, 4);
    }

    #[test]
    fn empty_content_is_a_no_op() {
        let p = parse("red::;");
        assert_eq!(p.text, "");
        assert!(p.effects.is_empty());
        let around = parse("a red::; b");
        assert_eq!(around.text, "a  b");
        assert!(around.effects.is_empty());
    }

    #[test]
    fn escapes() {
        let p = parse("red::a\\;b;");
        assert_eq!(p.text, "a;b");
        assert_eq!((p.effects[0].start, p.effects[0].end), (0, 3));

        // The spec's worked example.
        let ex = parse("shake::red1-blue3::text\\; lorem ipsum;");
        assert_eq!(ex.text, "text; lorem ipsum");
        assert_eq!(ex.effects[0].end, 17);

        let opener = parse("\\red::text;");
        assert_eq!(opener.text, "red::text;");
        assert!(opener.effects.is_empty());

        assert_eq!(parse("a\\\\b").text, "a\\b");
        assert_eq!(parse("\\*not bold\\*").text, "*not bold*");
        assert_eq!(parse("\\|\\|not a spoiler\\|\\|").text, "||not a spoiler||");
    }

    #[test]
    fn spoilers() {
        let p = parse("||hidden||");
        assert_eq!(p.text, "hidden");
        assert_eq!(p.effects.len(), 1);
        assert!(p.effects[0].spoiler);
        assert_eq!((p.effects[0].start, p.effects[0].end), (0, 6));

        let both = parse("||red::secret;||");
        assert_eq!(both.text, "secret");
        assert!(both.effects.iter().any(|e| e.spoiler));
        assert!(both.effects.iter().any(|e| e.color == solid("red2")));

        assert_eq!(parse("||oops").text, "||oops");
        assert_eq!(parse("||||").text, "||||");
    }

    #[test]
    fn offsets_are_characters_not_bytes() {
        let p = parse("🎉🎉red::ok;");
        assert_eq!(p.text, "🎉🎉ok");
        assert_eq!((p.effects[0].start, p.effects[0].end), (2, 4));
        let inside = parse("red::🎉ok;");
        assert_eq!((inside.effects[0].start, inside.effects[0].end), (0, 3));
    }

    #[test]
    fn text_around_a_span_keeps_its_place() {
        let p = parse("before red::mid; after");
        assert_eq!(p.text, "before mid after");
        assert_eq!((p.effects[0].start, p.effects[0].end), (7, 10));
        assert_eq!(&p.text[7..10], "mid");
    }

    #[test]
    fn nonsense_modifiers_are_refused_by_lookup() {
        assert!(classify("wobble").is_none());
        assert!(classify("red-wobble").is_none());
        assert!(classify("red12").is_none());
        assert!(classify("").is_none());
        for h in HUES {
            assert!(classify(h).is_some(), "{h}");
            for n in 1..=3 { assert!(classify(&format!("{h}{n}")).is_some(), "{h}{n}") }
        }
        for a in ANIMATIONS { assert!(classify(a).is_some(), "{a}") }
    }

    #[test]
    fn the_wire_format_round_trips() {
        let p = parse("shake::red1-blue3::text;");
        let v = to_json(&p.effects);
        let back = from_json(&v);
        assert_eq!(back, p.effects);
        let first = &v[0];
        assert_eq!(first["start"], 0);
        assert_eq!(first["end"], 4);
        assert_eq!(first["color"]["type"], "gradient");
        assert_eq!(first["color"]["stops"][0], "red1");
        assert_eq!(first["animation"], "shake");
    }

    #[test]
    fn compose_strips_the_syntax_from_the_body_and_keeps_offsets_aligned() {
        let c = compose("hello red::world;!");
        assert_eq!(c.body, "hello world!");
        assert_eq!(c.effects.len(), 1);
        let (s, e) = (c.effects[0].start, c.effects[0].end);
        assert_eq!(c.body.chars().skip(s).take(e - s).collect::<String>(), "world");
    }

    #[test]
    fn markdown_still_renders_for_everyone_else() {
        let c = compose("**bold** and *italic* and `code`");
        assert!(c.html.contains("<strong>bold</strong>"), "{}", c.html);
        assert!(c.html.contains("<em>italic</em>"), "{}", c.html);
        assert!(c.html.contains("<code>code</code>"), "{}", c.html);
        assert_eq!(c.body, "bold and italic and code");
        assert!(c.effects.is_empty());
    }

    #[test]
    fn a_modifier_inside_a_code_span_stays_literal() {
        let c = compose("try `red::foo;` here");
        assert_eq!(c.body, "try red::foo; here");
        assert!(c.effects.is_empty(), "{:?}", c.effects);
        assert!(c.html.contains("<code>red::foo;</code>"), "{}", c.html);
    }

    #[test]
    fn a_modifier_inside_a_fenced_block_stays_literal() {
        let c = compose("```\nred::foo;\nstd::vector\n```");
        assert!(c.effects.is_empty(), "{:?}", c.effects);
        assert!(c.body.contains("red::foo;"), "{}", c.body);
        assert!(c.html.contains("<pre>"), "{}", c.html);
    }

    #[test]
    fn spoilers_reach_other_clients_as_the_spec_says() {
        let c = compose("psst ||the butler did it||");
        assert_eq!(c.body, "psst the butler did it");
        assert!(c.html.contains("<span data-mx-spoiler>"), "{}", c.html);
        assert!(c.html.contains("the butler did it"), "{}", c.html);
        assert!(c.effects.iter().any(|e| e.spoiler));
    }

    #[test]
    fn colours_never_leak_into_the_html() {
        let c = compose("rainbow::party;");
        assert!(!c.html.contains("rainbow"), "{}", c.html);
        assert!(!c.html.contains("color"), "{}", c.html);
        assert_eq!(c.body, "party");
        assert_eq!(c.effects[0].color, Some(ColorSpec::Rainbow));
    }

    #[test]
    fn offsets_survive_multiple_blocks() {
        let c = compose("first line\n\nsecond red::styled; line");
        let e = &c.effects[0];
        let got: String = c.body.chars().skip(e.start).take(e.end - e.start).collect();
        assert_eq!(got, "styled");
    }

    #[test]
    fn mark_highlights_and_takes_the_colour_with_it() {
        let p = parse("mark::yellow::text;");
        assert_eq!(p.text, "text");
        assert!(p.effects[0].mark);
        assert_eq!(p.effects[0].color, solid("yellow2"));

        let bare = parse("mark::text;");
        assert!(bare.effects[0].mark);
        assert_eq!(bare.effects[0].color, None);
    }

    #[test]
    fn the_markdown_keywords_compose_with_everything_else() {
        let p = parse("shake::bold::red::text;");
        assert_eq!(p.text, "text");
        assert!(p.effects[0].bold);
        assert_eq!(p.effects[0].color, solid("red2"));
        assert_eq!(p.effects[0].animation, Some(Animation::Shake));

        let three = parse("underline::bold::red::text;");
        assert!(three.effects[0].underline && three.effects[0].bold);
        assert_eq!(three.effects[0].color, solid("red2"));

        for (tok, get) in [
            ("italic", (|e: &TextEffect| e.italic) as fn(&TextEffect) -> bool),
            ("strike", |e| e.strike),
            ("spoiler", |e| e.spoiler),
            ("mono", |e| e.mono),
        ] {
            let p = parse(&format!("{tok}::x;"));
            assert!(get(&p.effects[0]), "{tok} should set its flag");
        }
    }

    #[test]
    fn sizes_step_both_ways_and_the_last_one_wins() {
        assert_eq!(parse("small1::x;").effects[0].size, -1);
        assert_eq!(parse("small3::x;").effects[0].size, -3);
        assert_eq!(parse("big1::x;").effects[0].size, 1);
        assert_eq!(parse("big3::x;").effects[0].size, 3);
        assert_eq!(parse("small::x;").effects[0].size, -2);
        assert_eq!(parse("big::x;").effects[0].size, 2);
        assert_eq!(parse("big3::small1::x;").effects[0].size, -1);
        assert_eq!(parse("small1::big3::x;").effects[0].size, 3);
    }

    #[test]
    fn an_out_of_range_step_is_not_a_modifier_at_all() {
        let p = parse("small4::text;");
        assert_eq!(p.text, "small4::text;");
        assert!(p.effects.is_empty());
        assert!(classify("small0").is_none());
        assert!(classify("big9").is_none());
        assert!(classify("smallish").is_none());
    }

    #[test]
    fn the_new_animations_all_resolve_and_the_last_still_wins() {
        for a in ["sparkle", "glitch", "blur", "flip", "barrel"] {
            assert!(classify(a).is_some(), "{a}");
        }
        assert_eq!(parse("barrel::glitch::x;").effects[0].animation, Some(Animation::Glitch));
        assert_eq!(parse("glitch::barrel::x;").effects[0].animation, Some(Animation::Barrel));
    }

    #[test]
    fn underline_and_mark_reach_other_clients_through_the_html() {
        let c = compose("underline::hello;");
        assert!(c.html.contains("<u>"), "{}", c.html);
        assert_eq!(c.body, "hello");

        let m = compose("mark::yellow::hi;");
        assert!(m.html.contains("data-mx-bg-color=\"yellow\""), "{}", m.html);

        let s = compose("big3::glitch::hi;");
        assert!(!s.html.contains("big"), "{}", s.html);
        assert!(!s.html.contains("glitch"), "{}", s.html);
    }

    #[test]
    fn the_new_fields_round_trip_on_the_wire() {
        let p = parse("underline::mark::bold::big2::sparkle::red::x;");
        let back = from_json(&to_json(&p.effects));
        assert_eq!(back, p.effects);
        let v = to_json(&p.effects);
        assert_eq!(v[0]["underline"], true);
        assert_eq!(v[0]["mark"], true);
        assert_eq!(v[0]["bold"], true);
        assert_eq!(v[0]["size"], 2);
        assert_eq!(v[0]["animation"], "sparkle");
        let plain = to_json(&parse("red::x;").effects);
        assert!(plain[0].get("underline").is_none());
        assert!(plain[0].get("size").is_none());
    }
}
