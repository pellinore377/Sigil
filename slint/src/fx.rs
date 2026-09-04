//! SigilText, glyph by glyph — the port of RichText.qml. Slint has no flow
//! layout and cannot animate one character of a Text, so every effect
//! message is laid out here: each character measured with the bundled
//! fonts (the same files the UI draws with), wrapped at word ends to the
//! bubble's width, and handed to the bubble as absolute positions. The
//! bubble then draws one Text per glyph with the span's colour, weight,
//! motion, mark, underline, strike or spoiler cover.
//!
//! The `glow` span is the one effect Slint cannot draw at all: there is no
//! blur, shadow or stroke on a Text. `glow()` below rasterises the run's
//! outlines out of the same font files, blurs the coverage the way frost.rs
//! blurs a snapshot, and hands the bubble one picture of light to lay behind
//! the words — so the glow is a real halo off the glyph edges rather than
//! offset copies of the text, and it costs nothing per frame (the bubble
//! only breathes the picture's alpha).

use serde_json::Value;
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
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

// ---------------------------------------------------------------------------
// The glow, as light rather than as copies.
//
// A glyph is filled from its outline into a coverage mask (cached per
// character, size and weight — a message reuses one mask for every 'e'), the
// masks are stamped into the run's own layout in their own colours, and the
// picture is box-blurred twice, which lands close enough to a Gaussian at
// this radius (frost.rs blurs the frosted page the same way). Two pictures
// come back: `ink` carries the glyphs that wear the bubble's foreground, as
// alpha alone, so the bubble tints it with `fg` and white text glows white;
// `hue` carries the glyphs that brought a colour of their own, and colour
// emoji, baked.
// ---------------------------------------------------------------------------

/// Blur radius as a share of the font size; the light reaches `PASSES` times
/// that far, which at the body size is about half a line height.
const GLOW_R: f32 = 0.24;
/// Box passes. Two leave the box's own square shoulders visible in the haze;
/// three are a quadratic B-spline and read as light.
const PASSES: usize = 3;
/// The brightest pixel the blurred halo is lifted to before the bubble's
/// breathing alpha scales it; normalising like this keeps one glowing word
/// and a glowing paragraph equally bright.
const GLOW_PEAK: f32 = 1.0;
/// A cap on that lift, so a nearly empty run is not amplified into mush.
const GLOW_GAIN_MAX: f32 = 14.0;
/// Sub-scanlines per pixel row when filling an outline.
const SUB: usize = 4;
/// Nothing wider or taller than this is glowed (a runaway allocation guard).
const GLOW_MAX_PX: usize = 4096;

/// One glow picture pair for a laid-out message.
#[derive(Clone, Default)]
pub struct Glow {
    /// Alpha-only: the bubble colorizes it with its own foreground.
    pub ink: Option<Image>,
    /// Already coloured: spans with a colour, and colour emoji.
    pub hue: Option<Image>,
    /// How far the light may spill past the text box, in layout pixels.
    pub pad: f32,
}

/// A filled glyph, in coverage, positioned from the pen (baseline origin).
struct Mask {
    w: usize,
    h: usize,
    left: i32,
    top: i32,
    a: Vec<f32>,
}

/// What decides a glyph's shape: the character, its size in bits, and bold,
/// italic, mono.
type MaskKey = (char, u32, bool, bool, bool);

thread_local! {
    /// char × size × weight → its coverage. A message of a hundred letters
    /// rasterises two dozen shapes; a redraw rasterises none.
    static MASKS: RefCell<HashMap<MaskKey, Option<Rc<Mask>>>> = RefCell::new(HashMap::new());
    /// The finished pictures, keyed by the layout that made them, so the
    /// timeline can be rebuilt without blurring anything again.
    static GLOWS: RefCell<HashMap<u64, Glow>> = RefCell::new(HashMap::new());
}

/// Collects an outline into device-pixel line segments, y down from the
/// baseline, with the synthetic slant Slint uses for an italic Text.
struct Pen {
    segs: Vec<[f32; 4]>,
    cur: (f32, f32),
    start: (f32, f32),
    scale: f32,
    shear: f32,
}

impl Pen {
    fn map(&self, x: f32, y: f32) -> (f32, f32) {
        (x * self.scale + y * self.scale * self.shear, -y * self.scale)
    }
    fn to(&mut self, p: (f32, f32)) {
        self.segs.push([self.cur.0, self.cur.1, p.0, p.1]);
        self.cur = p;
    }
    /// Curves become lines; a step of about a pixel is finer than the blur.
    fn steps(&self, pts: &[(f32, f32)]) -> usize {
        let mut d = 0.0;
        for w in pts.windows(2) {
            d += ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt();
        }
        (d.ceil() as usize).clamp(1, 24)
    }
}

impl ttf_parser::OutlineBuilder for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        let p = self.map(x, y);
        self.cur = p;
        self.start = p;
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.map(x, y);
        self.to(p);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (p0, p1, p2) = (self.cur, self.map(x1, y1), self.map(x, y));
        let n = self.steps(&[p0, p1, p2]);
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            self.to((
                u * u * p0.0 + 2.0 * u * t * p1.0 + t * t * p2.0,
                u * u * p0.1 + 2.0 * u * t * p1.1 + t * t * p2.1,
            ));
        }
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (p0, p1, p2, p3) = (self.cur, self.map(x1, y1), self.map(x2, y2), self.map(x, y));
        let n = self.steps(&[p0, p1, p2, p3]);
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            self.to((
                u * u * u * p0.0 + 3.0 * u * u * t * p1.0 + 3.0 * u * t * t * p2.0 + t * t * t * p3.0,
                u * u * u * p0.1 + 3.0 * u * u * t * p1.1 + 3.0 * u * t * t * p2.1 + t * t * t * p3.1,
            ));
        }
    }
    fn close(&mut self) {
        let s = self.start;
        if s != self.cur {
            self.to(s);
        }
    }
}

/// Coverage of one span of a sub-scanline, with the fraction at each end.
fn span(row: &mut [f32], a: f32, b: f32, amt: f32) {
    let w = row.len() as f32;
    let (a, b) = (a.max(0.0), b.min(w));
    if b <= a {
        return;
    }
    let (i0, i1) = (a.floor() as usize, (b.ceil() as usize).min(row.len()));
    for (px, cell) in row.iter_mut().enumerate().take(i1).skip(i0) {
        let (l, r) = ((px as f32).max(a), ((px + 1) as f32).min(b));
        if r > l {
            *cell += amt * (r - l);
        }
    }
}

/// Fill the segments (non-zero winding) into a coverage buffer whose top-left
/// pixel is at (ox, oy) in the segments' own space.
fn fill(segs: &[[f32; 4]], w: usize, h: usize, ox: f32, oy: f32) -> Vec<f32> {
    let mut cov = vec![0.0f32; w * h];
    let mut xs: Vec<(f32, i32)> = Vec::with_capacity(16);
    let amt = 1.0 / SUB as f32;
    for py in 0..h {
        for s in 0..SUB {
            let y = oy + py as f32 + (s as f32 + 0.5) / SUB as f32;
            xs.clear();
            for &[x0, y0, x1, y1] in segs {
                if (y0 <= y) == (y1 <= y) {
                    continue;
                }
                let t = (y - y0) / (y1 - y0);
                xs.push((x0 + t * (x1 - x0) - ox, if y1 > y0 { 1 } else { -1 }));
            }
            if xs.len() < 2 {
                continue;
            }
            xs.sort_by(|a, b| a.0.total_cmp(&b.0));
            let (mut wind, mut from) = (0i32, 0.0f32);
            let row = &mut cov[py * w..(py + 1) * w];
            for &(x, d) in xs.iter() {
                if wind == 0 {
                    from = x;
                }
                wind += d;
                if wind == 0 {
                    span(row, from, x, amt);
                }
            }
        }
    }
    cov
}

/// The coverage of one character, cached.
fn mask(ch: char, size: f32, bold: bool, italic: bool, mono: bool) -> Option<Rc<Mask>> {
    let key = (ch, size.to_bits(), bold, italic, mono);
    if let Some(hit) = MASKS.with(|m| m.borrow().get(&key).cloned()) {
        return hit;
    }
    let built = build_mask(ch, size, bold, italic, mono).map(Rc::new);
    MASKS.with(|m| m.borrow_mut().insert(key, built.clone()));
    built
}

fn build_mask(ch: char, size: f32, bold: bool, italic: bool, mono: bool) -> Option<Mask> {
    let bytes = if mono {
        MONO
    } else if bold {
        BOLD
    } else {
        REGULAR
    };
    let face = Face::parse(bytes, 0).ok()?;
    let gid = face.glyph_index(ch)?;
    let scale = size / face.units_per_em() as f32;
    let shear = if italic { 0.21 } else { 0.0 };
    let mut pen = Pen { segs: Vec::new(), cur: (0.0, 0.0), start: (0.0, 0.0), scale, shear };
    face.outline_glyph(gid, &mut pen)?;
    if pen.segs.is_empty() {
        return None;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for s in &pen.segs {
        x0 = x0.min(s[0]).min(s[2]);
        y0 = y0.min(s[1]).min(s[3]);
        x1 = x1.max(s[0]).max(s[2]);
        y1 = y1.max(s[1]).max(s[3]);
    }
    let (left, top) = (x0.floor() as i32 - 1, y0.floor() as i32 - 1);
    let w = (x1.ceil() as i32 + 1 - left).clamp(1, 512) as usize;
    let h = (y1.ceil() as i32 + 1 - top).clamp(1, 512) as usize;
    let a = fill(&pen.segs, w, h, left as f32, top as f32);
    Some(Mask { w, h, left, top, a })
}

/// Box blur with a running window, `ch` interleaved channels, in place. The
/// window is clipped at the edges and divided by what it actually covered,
/// so the picture does not darken towards its border.
fn blur_pass(src: &[f32], dst: &mut [f32], w: usize, h: usize, ch: usize, r: usize, vert: bool) {
    let (outer, inner, step) = if vert { (w, h, w * ch) } else { (h, w, ch) };
    let mut pre = vec![0.0f32; (inner + 1) * ch];
    for o in 0..outer {
        let base = if vert { o * ch } else { o * w * ch };
        pre[..ch].fill(0.0);
        for i in 0..inner {
            for c in 0..ch {
                pre[(i + 1) * ch + c] = pre[i * ch + c] + src[base + i * step + c];
            }
        }
        for i in 0..inner {
            let lo = i.saturating_sub(r);
            let hi = (i + r + 1).min(inner);
            let n = (hi - lo) as f32;
            for c in 0..ch {
                dst[base + i * step + c] = (pre[hi * ch + c] - pre[lo * ch + c]) / n;
            }
        }
    }
}

fn blur(buf: &mut [f32], w: usize, h: usize, ch: usize, r: usize) {
    let mut tmp = vec![0.0f32; buf.len()];
    for _ in 0..PASSES {
        blur_pass(buf, &mut tmp, w, h, ch, r, false);
        blur_pass(&tmp, buf, w, h, ch, r, true);
    }
}

/// Lift the blurred coverage to `GLOW_PEAK` and write it out premultiplied.
fn picture(buf: &[f32], w: usize, h: usize, ch: usize) -> Option<Image> {
    let peak = buf
        .chunks_exact(ch)
        .map(|p| p[ch - 1])
        .fold(0.0f32, f32::max);
    if peak <= 0.001 {
        return None;
    }
    let gain = (GLOW_PEAK / peak).min(GLOW_GAIN_MAX);
    let mut out = SharedPixelBuffer::<Rgba8Pixel>::new(w as u32, h as u32);
    for (px, p) in out.make_mut_slice().iter_mut().zip(buf.chunks_exact(ch)) {
        let a = (p[ch - 1] * gain).clamp(0.0, 1.0);
        let (r, g, b) = if ch == 1 {
            (a, a, a)
        } else {
            (
                (p[0] * gain).clamp(0.0, a),
                (p[1] * gain).clamp(0.0, a),
                (p[2] * gain).clamp(0.0, a),
            )
        };
        *px = Rgba8Pixel {
            r: (r * 255.0) as u8,
            g: (g * 255.0) as u8,
            b: (b * 255.0) as u8,
            a: (a * 255.0) as u8,
        };
    }
    Some(Image::from_rgba8_premultiplied(out))
}

/// A colour emoji's picture, handed in by the caller (the bridge reads the
/// engine's cached PNG): width, height and straight RGBA.
pub type EmojiPix<'a> = dyn FnMut(&str) -> Option<(u32, u32, Vec<u8>)> + 'a;

/// The glow pictures for a laid-out message, or None when nothing glows.
pub fn glow(lay: &Layout, emoji: &mut EmojiPix) -> Option<Glow> {
    if !lay.glyphs.iter().any(|g| g.anim == "glow") {
        return None;
    }
    use std::hash::{Hash, Hasher};
    let mut key = std::collections::hash_map::DefaultHasher::new();
    for g in lay.glyphs.iter().filter(|g| g.anim == "glow") {
        g.ch.hash(&mut key);
        (g.x.to_bits(), g.y.to_bits(), g.w.to_bits(), g.size.to_bits()).hash(&mut key);
        (g.bold, g.italic, g.mono, &g.color).hash(&mut key);
    }
    (lay.width.to_bits(), lay.height.to_bits()).hash(&mut key);
    let hit = key.finish();
    if let Some(g) = GLOWS.with(|c| c.borrow().get(&hit).cloned()) {
        return Some(g);
    }
    let built = build_glow(lay, emoji)?;
    // A run whose emoji pictures have not arrived yet is not worth keeping:
    // the glow is made again when they land.
    if built.settled {
        GLOWS.with(|c| {
            let mut c = c.borrow_mut();
            if c.len() > 64 {
                c.clear();
            }
            c.insert(hit, built.glow.clone());
        });
    }
    Some(built.glow)
}

struct Built {
    glow: Glow,
    settled: bool,
}

fn build_glow(lay: &Layout, emoji: &mut EmojiPix) -> Option<Built> {
    let lit: Vec<&Glyph> = lay
        .glyphs
        .iter()
        .filter(|g| g.anim == "glow" && g.ch != " " && g.ch != "\n")
        .collect();
    if lit.is_empty() {
        return None;
    }
    let size = lit.iter().fold(0.0f32, |m, g| m.max(g.size));
    let r = (size * GLOW_R).round().max(2.0) as usize;
    let pad = (PASSES * r + 2) as f32;
    let w = (lay.width.ceil() as usize + 2 * pad as usize).min(GLOW_MAX_PX);
    let h = (lay.height.ceil() as usize + 2 * pad as usize).min(GLOW_MAX_PX);
    if w < 4 || h < 4 {
        return None;
    }
    // The baseline inside a glyph cell: Slint centres the line box in the
    // cell's `size * LINE`, so the pen sits an ascent below that top.
    let face = Face::parse(REGULAR, 0).ok()?;
    let upem = face.units_per_em() as f32;
    let (asc, desc) = (face.ascender() as f32 / upem, face.descender() as f32 / upem);
    let base_of = |g: &Glyph| g.y + (g.size * LINE - g.size * (asc - desc)) / 2.0 + g.size * asc;

    let mut ink = vec![0.0f32; w * h];
    let mut hue = vec![0.0f32; w * h * 4];
    let (mut any_ink, mut any_hue, mut settled) = (false, false, true);
    for g in &lit {
        let (px, py) = (pad + g.x, pad + base_of(g));
        let tint = g.color.as_deref().and_then(hex_rgb);
        if g.ch.chars().next().is_some_and(is_emoji) {
            match emoji(&g.ch) {
                Some((ew, eh, rgba)) if ew > 0 && eh > 0 => {
                    stamp_emoji(&mut hue, w, h, g, pad, ew, eh, &rgba);
                    any_hue = true;
                }
                _ => settled = false,
            }
            continue;
        }
        let Some(m) = mask(g.ch.chars().next()?, g.size, g.bold, g.italic, g.mono) else {
            continue;
        };
        let (x0, y0) = ((px.round() as i32 + m.left), (py.round() as i32 + m.top));
        match tint {
            Some((cr, cg, cb)) => {
                any_hue = true;
                for my in 0..m.h {
                    let ty = y0 + my as i32;
                    if ty < 0 || ty as usize >= h {
                        continue;
                    }
                    for mx in 0..m.w {
                        let tx = x0 + mx as i32;
                        if tx < 0 || tx as usize >= w {
                            continue;
                        }
                        let a = m.a[my * m.w + mx];
                        if a <= 0.0 {
                            continue;
                        }
                        let o = (ty as usize * w + tx as usize) * 4;
                        hue[o] += cr * a;
                        hue[o + 1] += cg * a;
                        hue[o + 2] += cb * a;
                        hue[o + 3] += a;
                    }
                }
            }
            None => {
                any_ink = true;
                for my in 0..m.h {
                    let ty = y0 + my as i32;
                    if ty < 0 || ty as usize >= h {
                        continue;
                    }
                    for mx in 0..m.w {
                        let tx = x0 + mx as i32;
                        if tx < 0 || tx as usize >= w {
                            continue;
                        }
                        ink[ty as usize * w + tx as usize] += m.a[my * m.w + mx];
                    }
                }
            }
        }
    }
    if !any_ink && !any_hue {
        return None;
    }
    let mut out = Glow { ink: None, hue: None, pad };
    if any_ink {
        blur(&mut ink, w, h, 1, r);
        out.ink = picture(&ink, w, h, 1);
    }
    if any_hue {
        blur(&mut hue, w, h, 4, r);
        out.hue = picture(&hue, w, h, 4);
    }
    Some(Built { glow: out, settled })
}

/// A colour emoji's own light: its picture, shrunk into the glyph's cell and
/// added to the coloured buffer, so a yellow face glows yellow.
#[allow(clippy::too_many_arguments)]
fn stamp_emoji(
    hue: &mut [f32],
    w: usize,
    h: usize,
    g: &Glyph,
    pad: f32,
    ew: u32,
    eh: u32,
    rgba: &[u8],
) {
    let side = g.size;
    let x0 = pad + g.x + (g.w - side) / 2.0;
    let y0 = pad + g.y + (g.size * LINE - side) / 2.0;
    let n = side.ceil().max(1.0) as usize;
    for dy in 0..n {
        let ty = (y0 + dy as f32).round() as i32;
        if ty < 0 || ty as usize >= h {
            continue;
        }
        let sy = ((dy as f32 + 0.5) / n as f32 * eh as f32) as u32;
        for dx in 0..n {
            let tx = (x0 + dx as f32).round() as i32;
            if tx < 0 || tx as usize >= w {
                continue;
            }
            let sx = ((dx as f32 + 0.5) / n as f32 * ew as f32) as u32;
            let s = ((sy.min(eh - 1) * ew + sx.min(ew - 1)) * 4) as usize;
            if s + 3 >= rgba.len() {
                continue;
            }
            let a = rgba[s + 3] as f32 / 255.0;
            if a <= 0.0 {
                continue;
            }
            let o = (ty as usize * w + tx as usize) * 4;
            hue[o] += rgba[s] as f32 / 255.0 * a;
            hue[o + 1] += rgba[s + 1] as f32 / 255.0 * a;
            hue[o + 2] += rgba[s + 2] as f32 / 255.0 * a;
            hue[o + 3] += a;
        }
    }
}

/// A character the bundled text fonts have no shape for and the system draws
/// in colour; the same test the bridge uses to mark an FxChar an emoji.
pub fn is_emoji(c: char) -> bool {
    c as u32 >= 0x1F000 || (0x2600..=0x27BF).contains(&(c as u32))
}

fn hex_rgb(h: &str) -> Option<(f32, f32, f32)> {
    let h = h.trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some((
        ((v >> 16) & 0xff) as f32 / 255.0,
        ((v >> 8) & 0xff) as f32 / 255.0,
        (v & 0xff) as f32 / 255.0,
    ))
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

    /// The brightest pixel of a glow picture, as straight RGBA.
    fn brightest(img: &Image) -> (u8, u8, u8, u8) {
        let buf = img.to_rgba8().expect("readable");
        buf.as_slice()
            .iter()
            .max_by_key(|p| p.a)
            .map(|p| (p.r, p.g, p.b, p.a))
            .unwrap()
    }

    fn no_emoji(_: &str) -> Option<(u32, u32, Vec<u8>)> {
        None
    }

    #[test]
    fn a_plain_glow_is_light_around_the_words_and_not_a_copy_of_them() {
        let fx = json!([{"start": 0, "end": 5, "animation": "glow"}]);
        let lay = layout("light", &fx, false, 17.0, 400.0).unwrap();
        let t0 = std::time::Instant::now();
        let g = glow(&lay, &mut no_emoji).expect("a glow");
        eprintln!("glow built in {:?}", t0.elapsed());
        // Uncoloured text hands back alpha alone: the bubble tints it with
        // its own foreground, so white text glows white.
        let ink = g.ink.expect("an ink picture");
        assert!(g.hue.is_none());
        assert!(g.pad >= 6.0, "the light spills outside the text box");
        let (w, h) = (ink.size().width as f32, ink.size().height as f32);
        assert!(w >= lay.width + 2.0 * g.pad - 1.0 && h >= lay.height + 2.0 * g.pad - 1.0);
        let (r, gr, b, a) = brightest(&ink);
        assert!(a > 200, "the halo reaches full strength somewhere: {a}");
        assert_eq!((r, gr, b), (255, 255, 255), "untinted, for colorize");
        // Blurred, not stamped: no hard edge — the border stays dark.
        let buf = ink.to_rgba8().unwrap();
        let edge = buf.as_slice()[0].a;
        assert_eq!(edge, 0, "the very corner is unlit");
        // Built again from the same layout, it comes out of the cache.
        let t1 = std::time::Instant::now();
        let again = glow(&lay, &mut no_emoji).expect("a glow");
        assert!(again.ink.is_some());
        assert!(t1.elapsed() < t0.elapsed(), "the second is cached");
    }

    #[test]
    fn a_coloured_glow_carries_the_span_colour() {
        let fx = json!([{"start": 0, "end": 5, "animation": "glow",
                         "color": {"type": "solid", "rgb": {"dark": "#e06c75", "light": "#a03030"}}}]);
        let lay = layout("ember", &fx, false, 17.0, 400.0).unwrap();
        let g = glow(&lay, &mut no_emoji).expect("a glow");
        assert!(g.ink.is_none(), "nothing here wears the bubble's ink");
        let (r, gr, b, _) = brightest(&g.hue.expect("a coloured picture"));
        assert!(r > gr + 40 && r > b + 40, "red light: {r},{gr},{b}");
    }

    #[test]
    fn nothing_but_glow_spans_makes_a_picture() {
        let fx = json!([{"start": 0, "end": 5, "animation": "wave"}]);
        let lay = layout("waves", &fx, false, 17.0, 400.0).unwrap();
        assert!(glow(&lay, &mut no_emoji).is_none());
    }
}
