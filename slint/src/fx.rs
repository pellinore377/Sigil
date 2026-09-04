//! SigilText, glyph by glyph — the port of RichText.qml. Slint has no flow
//! layout and cannot animate one character of a Text, so every effect
//! message is laid out here: each character measured with the bundled
//! fonts (the same files the UI draws with), wrapped at word ends to the
//! bubble's width, and handed to the bubble as absolute positions. The
//! bubble then draws one Text per glyph with the span's colour, weight,
//! motion, mark, underline, strike or spoiler cover.

use serde_json::Value;
use ttf_parser::Face;

static REGULAR: &[u8] = include_bytes!("../../shared/fonts/Roboto-Regular.ttf");
static BOLD: &[u8] = include_bytes!("../../shared/fonts/Roboto-Bold.ttf");
static MONO: &[u8] = include_bytes!("../../shared/fonts/RobotoMono-Regular.ttf");

/// Line height as a multiple of the font size; matches what Slint's Text
/// gives Roboto at the body size.
pub const LINE: f32 = 1.3;
/// Longer than this and a message is laid out as plain text: per-glyph
/// animation burns a core.
const MAX_CHARS: usize = 600;

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

#[derive(Clone, Debug, Default)]
pub struct Glyph {
    pub ch: String,
    pub idx: i32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub size: f32,
    pub color: Option<String>,
    pub anim: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub mono: bool,
    pub mark: bool,
    pub mark_color: Option<String>,
    pub spoiler: bool,
}

pub struct Layout {
    pub glyphs: Vec<Glyph>,
    pub width: f32,
    pub height: f32,
    pub has_spoiler: bool,
}

struct Fonts<'a> {
    regular: Face<'a>,
    bold: Face<'a>,
    mono: Face<'a>,
}

impl<'a> Fonts<'a> {
    fn load() -> Option<Fonts<'a>> {
        Some(Fonts {
            regular: Face::parse(REGULAR, 0).ok()?,
            bold: Face::parse(BOLD, 0).ok()?,
            mono: Face::parse(MONO, 0).ok()?,
        })
    }

    /// The advance of one character at `size` pixels. A glyph the font
    /// lacks (an emoji) is given a square.
    fn advance(&self, ch: char, size: f32, bold: bool, mono: bool) -> f32 {
        let face = if mono {
            &self.mono
        } else if bold {
            &self.bold
        } else {
            &self.regular
        };
        let upem = face.units_per_em() as f32;
        match face.glyph_index(ch).and_then(|g| face.glyph_hor_advance(g)) {
            Some(a) => a as f32 / upem * size,
            None => size,
        }
    }
}

/// Lay a message out, or None when it has no effects (plain text draws it).
/// `fresh` gates the one-shot typewriter reveal: an old message arrives typed.
pub fn layout(
    body: &str,
    effects: &Value,
    fresh: bool,
    base_px: f32,
    max_w: f32,
) -> Option<Layout> {
    let list = effects.as_array()?;
    if list.is_empty() {
        return None;
    }
    let chars: Vec<char> = body.chars().collect();
    if chars.is_empty() || chars.len() > MAX_CHARS {
        return None;
    }
    let fonts = Fonts::load()?;
    let (colors, _) = crate::rows::effect_char_colors(&chars, effects)
        .unwrap_or((vec![None; chars.len()], false));

    // Per-character attributes from the spans (RichText.qml's `glyphs`).
    let mut glyphs: Vec<Glyph> = chars
        .iter()
        .enumerate()
        .map(|(i, c)| Glyph {
            ch: c.to_string(),
            idx: i as i32,
            size: base_px,
            color: colors[i].clone(),
            ..Default::default()
        })
        .collect();
    let mut any = false;
    for e in list {
        let start = e["start"].as_u64().unwrap_or(0) as usize;
        let end = (e["end"].as_u64().unwrap_or(0) as usize).min(chars.len());
        if start >= end {
            continue;
        }
        any = true;
        let anim = e["animation"]
            .as_str()
            .filter(|a| ANIMS.contains(a))
            .unwrap_or("");
        let anim = if anim == "typewriter" && !fresh {
            ""
        } else {
            anim
        };
        let scale = e["sizeScale"].as_f64().unwrap_or(1.0).clamp(0.7, 1.6) as f32;
        let mark_color = e["markRgb"]["dark"].as_str().map(str::to_string);
        for g in &mut glyphs[start..end] {
            if !anim.is_empty() {
                g.anim = anim.to_string();
            }
            if e["bold"].as_bool().unwrap_or(false) {
                g.bold = true;
            }
            if e["italic"].as_bool().unwrap_or(false) {
                g.italic = true;
            }
            if e["underline"].as_bool().unwrap_or(false) {
                g.underline = true;
            }
            if e["strike"].as_bool().unwrap_or(false) {
                g.strike = true;
            }
            if e["mono"].as_bool().unwrap_or(false) {
                g.mono = true;
            }
            if e["spoiler"].as_bool().unwrap_or(false) {
                g.spoiler = true;
            }
            if e["mark"].as_bool().unwrap_or(false) {
                g.mark = true;
                g.mark_color = mark_color.clone();
            }
            if scale != 1.0 {
                g.size = (base_px * scale).round();
            }
        }
    }
    if !any {
        return None;
    }
    // reverseRun: a flip span reads back to front (the glyphs turn over).
    let mut i = 0;
    while i < glyphs.len() {
        if glyphs[i].anim == "flip" {
            let mut j = i;
            while j < glyphs.len() && glyphs[j].anim == "flip" {
                j += 1;
            }
            let chs: Vec<String> = glyphs[i..j].iter().rev().map(|g| g.ch.clone()).collect();
            for (g, ch) in glyphs[i..j].iter_mut().zip(chs) {
                g.ch = ch;
            }
            i = j;
        } else {
            i += 1;
        }
    }
    for g in &mut glyphs {
        let ch = g.ch.chars().next().unwrap_or(' ');
        g.w = fonts.advance(ch, g.size, g.bold, g.mono);
    }

    // Words: a space stays with the word before it; a newline ends a line.
    let line_h = base_px * LINE;
    let max_w = max_w.max(base_px * 4.0);
    let (mut x, mut line) = (0.0f32, 0usize);
    let mut width = 0.0f32;
    let mut word_start = 0;
    let n = glyphs.len();
    let mut idx = 0;
    while idx < n {
        // find the word end: through the next space, or a newline
        let mut end = idx;
        while end < n && glyphs[end].ch != " " && glyphs[end].ch != "\n" {
            end += 1;
        }
        let breaks = end < n && glyphs[end].ch == "\n";
        if end < n && glyphs[end].ch == " " {
            end += 1;
        }
        let word_w: f32 = glyphs[idx..end].iter().map(|g| g.w).sum();
        if x > 0.0
            && x + word_w > max_w
            && !(end > idx && glyphs[end - 1].ch == " " && x + word_w - glyphs[end - 1].w <= max_w)
        {
            line += 1;
            x = 0.0;
        }
        let _ = word_start;
        word_start = idx;
        for g in &mut glyphs[idx..end] {
            // a word wider than the line breaks wherever it must
            if x > 0.0 && x + g.w > max_w {
                line += 1;
                x = 0.0;
            }
            g.x = x;
            g.y = line as f32 * line_h;
            x += g.w;
            if g.ch != " " {
                width = width.max(x);
            }
        }
        idx = end;
        if breaks {
            // the newline itself takes no room
            if idx < n {
                glyphs[idx].x = 0.0;
                glyphs[idx].y = (line + 1) as f32 * line_h;
                glyphs[idx].w = 0.0;
                idx += 1;
            }
            line += 1;
            x = 0.0;
        }
    }
    let _ = word_start;
    let height = (line + 1) as f32 * line_h;
    let has_spoiler = glyphs.iter().any(|g| g.spoiler);
    Some(Layout {
        glyphs,
        width: width.max(1.0).min(max_w),
        height,
        has_spoiler,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn words_wrap_at_the_width_and_never_squash() {
        let body = "solid red, a gradient, and a rainbow";
        let fx = json!([{"start": 0, "end": 9, "color": {"type": "solid", "rgb": {"dark": "#ff4444", "light": "#aa0000"}}}]);
        let lay = layout(body, &fx, false, 12.0, 90.0).expect("laid out");
        assert!(lay.glyphs.iter().all(|g| g.w > 0.0 || g.ch == "\n"));
        // every glyph fits inside the width it was given
        assert!(
            lay.glyphs
                .iter()
                .all(|g| g.x + g.w <= 90.0 + 0.01 || g.ch == " "),
            "{:?}",
            lay.glyphs
        );
        assert!(lay.height > 12.0 * LINE * 1.5, "wrapped onto several lines");
        // glyphs on one line never overlap
        let mut by_line: std::collections::BTreeMap<i64, Vec<&Glyph>> = Default::default();
        for g in &lay.glyphs {
            by_line.entry((g.y * 10.0) as i64).or_default().push(g);
        }
        for gs in by_line.values() {
            for w in gs.windows(2) {
                assert!(w[1].x >= w[0].x + w[0].w - 0.01);
            }
        }
        // The dark swatch, lifted to the legibility floor for a dark bubble.
        assert_eq!(lay.glyphs[0].color.as_deref(), Some("#ff7474"));
    }

    #[test]
    fn spans_set_weight_size_and_spoilers_and_flip_reverses() {
        let fx = json!([
            {"start": 0, "end": 3, "bold": true, "sizeScale": 1.4, "spoiler": true},
            {"start": 4, "end": 7, "animation": "flip"}
        ]);
        let lay = layout("abc def", &fx, true, 12.0, 400.0).unwrap();
        assert!(lay.glyphs[0].bold && lay.glyphs[0].spoiler && lay.has_spoiler);
        assert_eq!(lay.glyphs[0].size, 17.0);
        let flipped: String = lay.glyphs[4..7].iter().map(|g| g.ch.as_str()).collect();
        assert_eq!(flipped, "fed");
        assert!(lay.glyphs[1].x > lay.glyphs[0].x);
    }

    #[test]
    fn a_newline_starts_a_line() {
        let fx = json!([{"start": 0, "end": 2, "animation": "wave"}]);
        let lay = layout("ab\ncd", &fx, true, 12.0, 400.0).unwrap();
        assert_eq!(lay.glyphs[3].x, 0.0);
        assert!(lay.glyphs[3].y > 0.0);
    }
}
