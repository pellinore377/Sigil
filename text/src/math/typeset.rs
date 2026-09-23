//! MathML Core layout with STIX Two Math's MATH table, emitted as glyph outlines and rules in em.
use crate::Error;
use serde_json::{json, Value};
use std::cell::Cell;
use std::collections::BTreeMap;
use ttf_parser::{math, Face, GlyphId, OutlineBuilder};
use unicode_normalization::UnicodeNormalization;

static FONT: &[u8] = include_bytes!("../../assets/STIXTwoMath-Regular.otf");
const MU: f32 = 1.0 / 18.0;

pub(super) fn layout(mathml: &str, block: bool) -> Result<Value, Error> {
    let root = Xml { s: mathml, i: 0 }.element(0).ok_or(Error::Invalid)?;
    let face = Face::parse(FONT, 0).map_err(|_| Error::Invalid)?;
    let table = face.tables().math.ok_or(Error::Invalid)?;
    let t = Typesetter {
        upem: face.units_per_em() as f32,
        k: table.constants.ok_or(Error::Invalid)?,
        info: table.glyph_info,
        variants: table.variants,
        face,
        missing: Cell::new(false),
    };
    let ctx = Ctx {
        level: 0,
        display: block,
        cramped: false,
        color: None,
    };
    let mut frame = t.row(&root.kids, ctx);
    frame.w += frame.ic;
    // A glyph the font lacks falls back to the TeX source rather than a replacement box.
    if t.missing.get() {
        return Err(Error::Invalid);
    }
    t.emit(frame)
}

struct Node {
    tag: String,
    attrs: Vec<(String, String)>,
    kids: Vec<Node>,
    text: String,
}

impl Node {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
    fn class(&self, name: &str) -> bool {
        self.attr("class")
            .is_some_and(|c| c.split_whitespace().any(|v| v == name))
    }
    fn token(&self) -> bool {
        matches!(self.tag.as_str(), "mi" | "mn" | "mo" | "mtext" | "ms")
    }
    // An mrow wrapping one child counts as that child.
    fn single(&self) -> &Node {
        if self.tag == "mrow" && self.kids.len() == 1 && self.attrs.is_empty() {
            self.kids[0].single()
        } else {
            self
        }
    }
}

// Reads the writer's own output: elements, quoted attributes (spacing optional) and entities.
struct Xml<'a> {
    s: &'a str,
    i: usize,
}

impl Xml<'_> {
    fn rest(&self) -> &str {
        &self.s[self.i..]
    }
    fn eat(&mut self, c: char) -> Option<()> {
        self.rest().starts_with(c).then(|| self.i += c.len_utf8())
    }
    fn name(&mut self) -> String {
        let n = self
            .rest()
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == ':'))
            .unwrap_or(self.rest().len());
        let name = self.rest()[..n].to_string();
        self.i += n;
        name
    }
    fn space(&mut self) {
        self.i += self.rest().len() - self.rest().trim_start().len();
    }
    fn element(&mut self, depth: u32) -> Option<Node> {
        if depth > 96 {
            return None;
        }
        self.eat('<')?;
        let tag = self.name();
        let mut attrs = Vec::new();
        loop {
            self.space();
            if self.eat('/').is_some() {
                self.eat('>')?;
                return Some(Node {
                    tag,
                    attrs,
                    kids: Vec::new(),
                    text: String::new(),
                });
            }
            if self.eat('>').is_some() {
                break;
            }
            let key = self.name();
            if key.is_empty() {
                return None;
            }
            self.eat('=')?;
            self.eat('"')?;
            let end = self.rest().find('"')?;
            attrs.push((key, unescape(&self.rest()[..end])));
            self.i += end + 1;
        }
        let (mut kids, mut text) = (Vec::new(), String::new());
        loop {
            if self.rest().starts_with("</") {
                self.i += 2;
                if self.name() != tag {
                    return None;
                }
                self.eat('>')?;
                return Some(Node {
                    tag,
                    attrs,
                    kids,
                    text,
                });
            }
            if self.rest().starts_with('<') {
                kids.push(self.element(depth + 1)?);
                continue;
            }
            let end = self.rest().find('<')?;
            text.push_str(&unescape(&self.rest()[..end]));
            self.i += end;
        }
    }
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest.find(';') else { break };
        let entity = &rest[1..end];
        let c = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            _ => entity
                .strip_prefix("#x")
                .map(|h| u32::from_str_radix(h, 16).ok())
                .unwrap_or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[derive(Clone, Copy)]
struct Ctx {
    level: u8,
    display: bool,
    cramped: bool,
    color: Option<u32>,
}

impl Ctx {
    fn script(self) -> Self {
        Ctx {
            level: (self.level + 1).min(2),
            display: false,
            ..self
        }
    }
    fn cramp(self) -> Self {
        Ctx {
            cramped: true,
            ..self
        }
    }
    fn apply(mut self, n: &Node) -> Self {
        match n.attr("displaystyle") {
            Some("true") => self.display = true,
            Some("false") => self.display = false,
            _ => (),
        }
        if let Some(level) = n.attr("scriptlevel").and_then(|v| v.parse::<u8>().ok()) {
            self.level = level.min(2);
        }
        if let Some(rgb) = n.attr("style").and_then(|s| {
            s.split(';').find_map(|p| {
                p.trim()
                    .strip_prefix("color: rgb(")?
                    .strip_suffix(')')
                    .map(str::to_owned)
            })
        }) {
            let v: Vec<u32> = rgb
                .split_whitespace()
                .filter_map(|c| c.parse().ok())
                .collect();
            if v.len() == 3 {
                self.color = Some(v[0].min(255) << 16 | v[1].min(255) << 8 | v[2].min(255));
            }
        }
        self
    }
}

#[derive(Clone, Copy)]
enum Item {
    Glyph {
        id: u16,
        x: f32,
        y: f32,
        s: f32,
        c: Option<u32>,
    },
    Rule {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        c: Option<u32>,
    },
}

// Font units at scale 1, y up from the baseline; `ic` is italic correction kept outside `w`.
#[derive(Default)]
struct Frame {
    w: f32,
    a: f32,
    d: f32,
    ic: f32,
    items: Vec<Item>,
    glyph: Option<u16>,
    largeop: bool,
}

impl Frame {
    fn put(&mut self, other: Frame, dx: f32, dy: f32) {
        self.items
            .extend(other.items.into_iter().map(|item| match item {
                Item::Glyph { id, x, y, s, c } => Item::Glyph {
                    id,
                    x: x + dx,
                    y: y + dy,
                    s,
                    c,
                },
                Item::Rule { x, y, w, h, c } => Item::Rule {
                    x: x + dx,
                    y: y + dy,
                    w,
                    h,
                    c,
                },
            }));
    }
    fn shift(mut self, dy: f32) -> Self {
        for item in &mut self.items {
            match item {
                Item::Glyph { y, .. } | Item::Rule { y, .. } => *y += dy,
            }
        }
        self.a += dy;
        self.d -= dy;
        self
    }
}

struct Typesetter<'a> {
    face: Face<'a>,
    upem: f32,
    k: math::Constants<'a>,
    info: Option<math::GlyphInfo<'a>>,
    variants: Option<math::Variants<'a>>,
    missing: Cell<bool>,
}

enum Class {
    Rel,
    Bin,
    Punct,
    Op,
    Fence,
    Other,
}

fn class(text: &str) -> Class {
    let mut chars = text.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return if text.chars().count() > 1 && text.chars().all(|c| "=<>≤≥≠:!".contains(c)) {
            Class::Rel
        } else {
            Class::Other
        };
    };
    match c {
        '=' | '<' | '>' | ':' | '≤' | '≥' | '≠' | '≈' | '≡' | '∼' | '≃' | '≅' | '∝' | '≪' | '≫'
        | '≺' | '≻' | '⪯' | '⪰' | '≔' | '≐' | '≍' | '≮' | '≯' | '≰' | '≱' | '≢' | '≁' | '≉' => {
            Class::Rel
        }
        '∈' | '∉' | '∋' | '∌' | '⊂' | '⊃' | '⊆' | '⊇' | '⊊' | '⊋' | '⊄' | '⊅' | '⊈' | '⊉' | '∣'
        | '∤' | '∥' | '∦' | '⊥' | '⊢' | '⊣' | '⊨' | '⊩' | '⊏' | '⊐' | '⊑' | '⊒' | '⋈' | '⌣'
        | '⌢' => Class::Rel,
        '→' | '←' | '↔' | '⇒' | '⇐' | '⇔' | '↦' | '⟶' | '⟵' | '⟷' | '⟹' | '⟸' | '⟺' | '⟼' | '↑'
        | '↓' | '↕' | '⇑' | '⇓' | '↗' | '↘' | '↙' | '↖' | '↪' | '↩' | '⇀' | '↽' | '⇌' | '⇝' => {
            Class::Rel
        }
        '+' | '−' | '-' | '±' | '∓' | '×' | '÷' | '⋅' | '·' | '∗' | '∘' | '∙' | '∪' | '∩' | '∧'
        | '∨' | '⊕' | '⊗' | '⊖' | '⊙' | '⊘' | '∖' | '⋆' | '†' | '‡' | '⊎' | '⊔' | '⊓' | '◃'
        | '▹' | '≀' | '⨿' | '⋄' | '△' | '▽' | '⊲' | '⊳' | '⊴' | '⊵' | '⋉' | '⋊' | '⊞' | '⊟'
        | '⊠' | '⊡' => Class::Bin,
        ',' | ';' => Class::Punct,
        '∑' | '∏' | '∐' | '∫' | '∬' | '∭' | '∮' | '∯' | '∰' | '⨌' | '⋃' | '⋂' | '⋁' | '⋀' | '⨁'
        | '⨂' | '⨀' | '⨄' | '⨆' => Class::Op,
        '(' | ')' | '[' | ']' | '{' | '}' | '|' | '‖' | '⟨' | '⟩' | '⌊' | '⌋' | '⌈' | '⌉' | '⟦'
        | '⟧' | '/' | '\\' => Class::Fence,
        _ => Class::Other,
    }
}

// The operator an embellished operator (scripted or wrapped) spaces as.
fn embellished(n: &Node) -> Option<&Node> {
    match n.tag.as_str() {
        "mo" => Some(n),
        "msub" | "msup" | "msubsup" | "munder" | "mover" | "munderover" => {
            embellished(n.kids.first()?)
        }
        "mrow" if n.kids.len() == 1 => embellished(&n.kids[0]),
        _ => None,
    }
}

fn italic(c: char) -> char {
    let v = c as u32;
    char::from_u32(match c {
        'h' => 0x210E,
        'A'..='Z' => v - 'A' as u32 + 0x1D434,
        'a'..='z' => v - 'a' as u32 + 0x1D44E,
        'Α'..='Ρ' | 'Σ'..='Ω' => v - 'Α' as u32 + 0x1D6E2,
        'α'..='ω' => v - 'α' as u32 + 0x1D6FC,
        'µ' => 0x1D707,
        'ϑ' => 0x1D717,
        'ϕ' => 0x1D719,
        'ϖ' => 0x1D71B,
        'ϵ' => 0x1D716,
        'ϱ' => 0x1D71A,
        'ϰ' => 0x1D718,
        '∂' => 0x1D715,
        '∇' => 0x1D6FB,
        'ı' => 0x1D6A4,
        'ȷ' => 0x1D6A5,
        _ => v,
    })
    .unwrap_or(c)
}

// Spacing accents the writer emits as plain characters, mapped to the font's combining marks.
fn accent(c: char) -> Option<char> {
    Some(match c {
        '^' | 'ˆ' | '\u{302}' => '\u{302}',
        '~' | '˜' | '\u{303}' => '\u{303}',
        '¯' | '\u{304}' => '\u{304}',
        '˙' | '\u{307}' => '\u{307}',
        '¨' | '\u{308}' => '\u{308}',
        'ˇ' | '\u{30c}' => '\u{30c}',
        '˘' | '\u{306}' => '\u{306}',
        '´' | '\u{301}' => '\u{301}',
        '`' | '\u{300}' => '\u{300}',
        '˚' | '\u{30a}' => '\u{30a}',
        '→' | '\u{20d7}' => '\u{20d7}',
        '←' | '\u{20d6}' => '\u{20d6}',
        _ => return None,
    })
}

fn length(value: &str, em: f32) -> f32 {
    let split = value
        .find(|c: char| c.is_ascii_alphabetic() || c == '%')
        .unwrap_or(value.len());
    let n: f32 = value[..split].trim().parse().unwrap_or(0.0);
    let n = if n.is_finite() {
        n.clamp(-100.0, 100.0)
    } else {
        0.0
    };
    em * n
        * match &value[split..] {
            // The writer has already converted mu to em.
            "em" | "mu" | "" => 1.0,
            "ex" => 0.43,
            "pt" => 0.1,
            "pc" => 1.2,
            "mm" => 0.2845,
            "cm" => 2.845,
            "in" => 7.227,
            "px" => 0.0625,
            _ => 0.0,
        }
}

impl Typesetter<'_> {
    fn scale(&self, ctx: Ctx) -> f32 {
        match ctx.level {
            0 => 1.0,
            1 => self.k.script_percent_scale_down() as f32 / 100.0,
            _ => self.k.script_script_percent_scale_down() as f32 / 100.0,
        }
    }
    fn em(&self, ctx: Ctx) -> f32 {
        self.upem * self.scale(ctx)
    }
    fn v(&self, value: math::MathValue, ctx: Ctx) -> f32 {
        value.value as f32 * self.scale(ctx)
    }
    fn axis(&self, ctx: Ctx) -> f32 {
        self.v(self.k.axis_height(), ctx)
    }
    fn italic_correction(&self, id: u16) -> f32 {
        self.info
            .and_then(|i| i.italic_corrections)
            .and_then(|v| v.get(GlyphId(id)))
            .map_or(0.0, |v| v.value as f32)
    }
    fn attachment(&self, id: u16) -> Option<f32> {
        self.info
            .and_then(|i| i.top_accent_attachments)
            .and_then(|v| v.get(GlyphId(id)))
            .map(|v| v.value as f32)
    }

    fn glyph(&self, id: u16, s: f32, ctx: Ctx) -> Frame {
        let b = self.face.glyph_bounding_box(GlyphId(id));
        Frame {
            w: self.face.glyph_hor_advance(GlyphId(id)).unwrap_or(0) as f32 * s,
            a: b.map_or(0.0, |b| b.y_max as f32 * s),
            d: b.map_or(0.0, |b| -b.y_min as f32 * s),
            ic: self.italic_correction(id) * s,
            items: vec![Item::Glyph {
                id,
                x: 0.0,
                y: 0.0,
                s,
                c: ctx.color,
            }],
            glyph: Some(id),
            largeop: false,
        }
    }

    fn text(&self, text: &str, ctx: Ctx) -> Frame {
        let s = self.scale(ctx);
        let mut f = Frame::default();
        let mut count = 0;
        for c in text.nfc() {
            let id = self.face.glyph_index(c).map_or_else(
                || {
                    self.missing.set(true);
                    0
                },
                |g| g.0,
            );
            let g = self.glyph(id, s, ctx);
            f.a = f.a.max(g.a);
            f.d = f.d.max(g.d);
            f.ic = g.ic;
            let x = f.w;
            f.w += g.w;
            f.put(g, x, 0.0);
            f.glyph = Some(id);
            count += 1;
        }
        if count != 1 {
            f.glyph = None;
        }
        f
    }

    fn node(&self, n: &Node, ctx: Ctx) -> Frame {
        let ctx = ctx.apply(n);
        match n.tag.as_str() {
            "mi" => {
                let mut chars = n.text.chars();
                let upright = n.attr("mathvariant") == Some("normal");
                match (chars.next(), chars.next()) {
                    (Some(c), None) if !upright => self.text(&italic(c).to_string(), ctx),
                    _ => {
                        let mut f = self.text(&n.text, ctx);
                        f.ic = 0.0;
                        f
                    }
                }
            }
            "mn" | "mtext" | "ms" => {
                let mut f = self.text(&n.text, ctx);
                f.ic = 0.0;
                f
            }
            "mo" => self.operator(n, ctx, None),
            "mspace" => {
                let em = self.em(ctx);
                Frame {
                    w: n.attr("width").map_or(0.0, |v| length(v, em)),
                    a: n.attr("height").map_or(0.0, |v| length(v, em)),
                    d: n.attr("depth").map_or(0.0, |v| length(v, em)),
                    ..Frame::default()
                }
            }
            "mfrac" if n.kids.len() == 2 => self.fraction(n, ctx),
            "msqrt" => {
                let body = self.row(&n.kids, ctx.cramp());
                self.radical(body, None, ctx)
            }
            "mroot" if n.kids.len() == 2 => {
                let body = self.node(&n.kids[0], ctx.cramp());
                let index = self.node(
                    &n.kids[1],
                    Ctx {
                        level: 2,
                        display: false,
                        ..ctx
                    },
                );
                self.radical(body, Some(index), ctx)
            }
            "msub" | "msup" | "msubsup" if n.kids.len() >= 2 => self.scripts(n, ctx),
            "munder" | "mover" | "munderover" if n.kids.len() >= 2 => self.limits(n, ctx),
            "mtable" => self.table(n, ctx),
            "mphantom" => {
                let mut f = self.row(&n.kids, ctx);
                f.items.clear();
                f
            }
            _ if n.class("mop-negated") => {
                let mut f = self.row(&n.kids, ctx);
                let slash = self.text("\u{338}", ctx);
                let x = (f.w - slash.w) / 2.0;
                f.put(slash, x, 0.0);
                f
            }
            _ => self.row(&n.kids, ctx),
        }
    }

    fn operator(&self, n: &Node, ctx: Ctx, target: Option<(f32, f32)>) -> Frame {
        let text: String = n.text.nfc().collect();
        if text == "\u{2061}" || text == "\u{2062}" || text == "\u{2063}" {
            return Frame::default();
        }
        let s = self.scale(ctx);
        let mut chars = text.chars();
        let single = match (chars.next(), chars.next()) {
            (Some(c), None) => Some(c),
            _ => None,
        };
        let axis = self.axis(ctx);
        if let (Some(c), Class::Op, false) =
            (single, class(&text), n.attr("largeop") == Some("false"))
        {
            let base = self.face.glyph_index(c).map_or(0, |g| g.0);
            let id = if ctx.display {
                let min = self.k.display_operator_min_height() as f32;
                self.vertical(base)
                    .and_then(|v| {
                        v.variants
                            .into_iter()
                            .find(|v| v.advance_measurement as f32 >= min)
                            .or_else(|| v.variants.into_iter().last())
                    })
                    .map_or(base, |v| v.variant_glyph.0)
            } else {
                base
            };
            let g = self.glyph(id, s, ctx);
            let dy = axis - (g.a - g.d) / 2.0;
            let mut f = g.shift(dy);
            f.largeop = true;
            return f;
        }
        if let (Some(c), true) = (single, n.attr("stretchy") == Some("true")) {
            let em = self.em(ctx);
            let size = n.attr("minsize").map(|v| length(v, em)).or_else(|| {
                target.map(|(a, d)| {
                    let size = 2.0 * (a - axis).max(d + axis);
                    (size * 0.901).max(size - 0.5 * em)
                })
            });
            if let (Some(size), Some(id)) = (size, self.face.glyph_index(c)) {
                if self.vertical(id.0).is_some() {
                    let g = self.stretch(id.0, size, s, ctx, true);
                    let dy = axis - (g.a - g.d) / 2.0;
                    return g.shift(dy);
                }
            }
        }
        self.text(&text, ctx)
    }

    fn vertical(&self, id: u16) -> Option<math::GlyphConstruction<'_>> {
        self.variants?.vertical_constructions.get(GlyphId(id))
    }
    fn horizontal(&self, id: u16) -> Option<math::GlyphConstruction<'_>> {
        self.variants?.horizontal_constructions.get(GlyphId(id))
    }

    // The smallest variant covering `size`, else an assembly of parts.
    fn stretch(&self, id: u16, size: f32, s: f32, ctx: Ctx, vertical: bool) -> Frame {
        let base = self.glyph(id, s, ctx);
        let natural = if vertical { base.a + base.d } else { base.w };
        let construction = if vertical {
            self.vertical(id)
        } else {
            self.horizontal(id)
        };
        let Some(construction) = construction else {
            return base;
        };
        if natural >= size {
            return base;
        }
        let mut last = None;
        for v in construction.variants {
            last = Some(v.variant_glyph.0);
            if v.advance_measurement as f32 * s >= size {
                return self.glyph(v.variant_glyph.0, s, ctx);
            }
        }
        match construction.assembly {
            Some(assembly) => self.assemble(assembly, size, s, ctx, vertical),
            None => last.map_or(base, |id| self.glyph(id, s, ctx)),
        }
    }

    fn assemble(
        &self,
        assembly: math::GlyphAssembly,
        size: f32,
        s: f32,
        ctx: Ctx,
        vertical: bool,
    ) -> Frame {
        let overlap = self.variants.map_or(0, |v| v.min_connector_overlap) as f32;
        let parts: Vec<math::GlyphPart> = assembly.parts.into_iter().collect();
        let target = size / s;
        let mut sequence = Vec::new();
        for repeats in 1..=64 {
            sequence = parts
                .iter()
                .flat_map(|p| {
                    std::iter::repeat_n(*p, if p.part_flags.extender() { repeats } else { 1 })
                })
                .collect();
            let full: f32 = sequence.iter().map(|p| p.full_advance as f32).sum::<f32>()
                - overlap * (sequence.len().saturating_sub(1)) as f32;
            if full >= target {
                break;
            }
        }
        let joins = sequence.len().saturating_sub(1) as f32;
        let total: f32 = sequence.iter().map(|p| p.full_advance as f32).sum();
        let allowed = sequence
            .windows(2)
            .map(|w| w[0].end_connector_length.min(w[1].start_connector_length) as f32)
            .fold(f32::MAX, f32::min)
            .max(overlap);
        let o = if joins > 0.0 {
            ((total - target) / joins).clamp(overlap, allowed)
        } else {
            0.0
        };
        let mut f = Frame::default();
        let mut at = 0.0;
        for part in &sequence {
            let id = part.glyph_id.0;
            let b = self.face.glyph_bounding_box(part.glyph_id);
            let g = self.glyph(id, s, ctx);
            if vertical {
                let lift = at - b.map_or(0.0, |b| b.y_min as f32) * s;
                f.w = f.w.max(g.w);
                f.put(g, 0.0, lift);
            } else {
                f.a = f.a.max(g.a);
                f.d = f.d.max(g.d);
                f.put(g, at, 0.0);
            }
            at += (part.full_advance as f32 - o) * s;
        }
        let length = at + o * s;
        if vertical {
            f.a = length;
        } else {
            f.w = length;
        }
        f
    }

    fn row(&self, kids: &[Node], ctx: Ctx) -> Frame {
        let stretchy = |n: &Node| {
            n.tag == "mo" && n.attr("stretchy") == Some("true") && n.attr("minsize").is_none()
        };
        let mut frames: Vec<Option<Frame>> = kids
            .iter()
            .map(|n| (!stretchy(n)).then(|| self.node(n, ctx)))
            .collect();
        let (mut a, mut d) = (0f32, 0f32);
        for f in frames.iter().flatten() {
            a = a.max(f.a);
            d = d.max(f.d);
        }
        for (n, slot) in kids.iter().zip(frames.iter_mut()) {
            if slot.is_none() {
                *slot = Some(self.operator(n, ctx.apply(n), Some((a, d))));
            }
        }
        let em = self.em(ctx);
        let mut out = Frame::default();
        let last = kids.len().saturating_sub(1);
        let mut prior_op = true;
        for (i, (n, f)) in kids.iter().zip(frames).enumerate() {
            let f = f.unwrap_or_default();
            let core = embellished(n);
            let (left, right) = if let Some(n) = core {
                let text: String = n.text.nfc().collect();
                let wide = ctx.level == 0;
                match class(&text) {
                    Class::Rel if wide => (5.0, 5.0),
                    Class::Bin if wide && !prior_op && i != last => (4.0, 4.0),
                    Class::Punct if wide => (0.0, 3.0),
                    Class::Op => (3.0, 3.0),
                    _ => (0.0, 0.0),
                }
            } else {
                (0.0, 0.0)
            };
            prior_op = core.is_some_and(|n| !matches!(class(&n.text), Class::Fence | Class::Other));
            out.w += left * MU * em;
            let x = out.w;
            out.a = out.a.max(f.a);
            out.d = out.d.max(f.d);
            out.w += f.w + if i == last { 0.0 } else { f.ic };
            out.ic = if i == last { f.ic } else { 0.0 };
            if kids.len() == 1 {
                out.glyph = f.glyph;
                out.largeop = f.largeop;
            }
            out.put(f, x, 0.0);
            out.w += right * MU * em;
        }
        out
    }

    fn fraction(&self, n: &Node, ctx: Ctx) -> Frame {
        let inner = if ctx.display {
            Ctx {
                display: false,
                ..ctx
            }
        } else {
            ctx.script()
        };
        let num = self.node(&n.kids[0], inner);
        let den = self.node(&n.kids[1], inner.cramp());
        let k = &self.k;
        let d = ctx.display;
        let axis = self.axis(ctx);
        let em = self.em(ctx);
        let t = n
            .attr("linethickness")
            .map_or(self.v(k.fraction_rule_thickness(), ctx), |v| length(v, em));
        let (u, v) = if t > 0.0 {
            let u = self
                .v(
                    if d {
                        k.fraction_numerator_display_style_shift_up()
                    } else {
                        k.fraction_numerator_shift_up()
                    },
                    ctx,
                )
                .max(
                    axis + t / 2.0
                        + self.v(
                            if d {
                                k.fraction_num_display_style_gap_min()
                            } else {
                                k.fraction_numerator_gap_min()
                            },
                            ctx,
                        )
                        + num.d,
                );
            let v = self
                .v(
                    if d {
                        k.fraction_denominator_display_style_shift_down()
                    } else {
                        k.fraction_denominator_shift_down()
                    },
                    ctx,
                )
                .max(
                    den.a
                        + self.v(
                            if d {
                                k.fraction_denom_display_style_gap_min()
                            } else {
                                k.fraction_denominator_gap_min()
                            },
                            ctx,
                        )
                        + t / 2.0
                        - axis,
                );
            (u, v)
        } else {
            let mut u = self.v(
                if d {
                    k.stack_top_display_style_shift_up()
                } else {
                    k.stack_top_shift_up()
                },
                ctx,
            );
            let mut v = self.v(
                if d {
                    k.stack_bottom_display_style_shift_down()
                } else {
                    k.stack_bottom_shift_down()
                },
                ctx,
            );
            let min = self.v(
                if d {
                    k.stack_display_style_gap_min()
                } else {
                    k.stack_gap_min()
                },
                ctx,
            );
            let gap = (u - num.d) - (den.a - v);
            if gap < min {
                u += (min - gap) / 2.0;
                v += (min - gap) / 2.0;
            }
            (u, v)
        };
        let pad = if t > 0.0 { 0.12 * em } else { 0.0 };
        let w = (num.w + num.ic).max(den.w + den.ic);
        let mut f = Frame {
            w: w + 2.0 * pad,
            a: u + num.a,
            d: v + den.d,
            ..Frame::default()
        };
        if t > 0.0 {
            f.items.push(Item::Rule {
                x: pad,
                y: axis - t / 2.0,
                w,
                h: t,
                c: ctx.color,
            });
            f.a = f.a.max(axis + t / 2.0);
        }
        let (nx, dx) = (pad + (w - num.w) / 2.0, pad + (w - den.w) / 2.0);
        f.put(num, nx, u);
        f.put(den, dx, -v);
        f
    }

    fn radical(&self, body: Frame, index: Option<Frame>, ctx: Ctx) -> Frame {
        let k = &self.k;
        let s = self.scale(ctx);
        let t = self.v(k.radical_rule_thickness(), ctx);
        let mut gap = self.v(
            if ctx.display {
                k.radical_display_style_vertical_gap()
            } else {
                k.radical_vertical_gap()
            },
            ctx,
        );
        let extra = self.v(k.radical_extra_ascender(), ctx);
        let id = self.face.glyph_index('√').map_or(0, |g| g.0);
        let sign = self.stretch(id, body.a + body.d + gap + t, s, ctx, true);
        let height = sign.a + sign.d;
        let excess = (height - t) - (body.a + body.d + gap);
        if excess > 0.0 {
            gap += excess / 2.0;
        }
        let top = body.a + gap + t;
        let lift = top - sign.a;
        let bottom = top - height;
        let mut f = Frame::default();
        let mut x = 0.0;
        if let Some(index) = index {
            let before = self.v(k.radical_kern_before_degree(), ctx);
            let after = self.v(k.radical_kern_after_degree(), ctx);
            let raise = k.radical_degree_bottom_raise_percent() as f32 / 100.0 * height;
            let base = bottom + raise + index.d;
            f.a = f.a.max(base + index.a);
            let iw = index.w;
            f.put(index, before, base);
            x = (before + iw + after).max(0.0);
        }
        let sw = sign.w;
        f.put(sign, x, lift);
        x += sw;
        f.items.push(Item::Rule {
            x,
            y: top - t,
            w: body.w + body.ic,
            h: t,
            c: ctx.color,
        });
        f.a = f.a.max(top + extra);
        f.d = body.d.max(-bottom);
        f.w = x + body.w + body.ic;
        f.put(body, x, 0.0);
        f
    }

    fn scripts(&self, n: &Node, ctx: Ctx) -> Frame {
        let k = &self.k;
        let base = self.node(&n.kids[0], ctx);
        let token = n.kids[0].single().token() && !base.largeop;
        let script = ctx.script();
        let (sub, sup) = match n.tag.as_str() {
            "msub" => (Some(self.node(&n.kids[1], script.cramp())), None),
            "msup" => (None, Some(self.node(&n.kids[1], script))),
            _ => (
                Some(self.node(&n.kids[1], script.cramp())),
                n.kids.get(2).map(|m| self.node(m, script)),
            ),
        };
        let mut u = 0.0;
        let mut v = 0.0;
        if let Some(sup) = &sup {
            u = self
                .v(
                    if ctx.cramped {
                        k.superscript_shift_up_cramped()
                    } else {
                        k.superscript_shift_up()
                    },
                    ctx,
                )
                .max(sup.d + self.v(k.superscript_bottom_min(), ctx));
            if !token {
                u = u.max(base.a - self.v(k.superscript_baseline_drop_max(), ctx));
            }
        }
        if let Some(sub) = &sub {
            v = self
                .v(k.subscript_shift_down(), ctx)
                .max(sub.a - self.v(k.subscript_top_max(), ctx));
            if !token {
                v = v.max(base.d + self.v(k.subscript_baseline_drop_min(), ctx));
            }
        }
        if let (Some(sub), Some(sup)) = (&sub, &sup) {
            let gap = (u - sup.d) - (sub.a - v);
            let min = self.v(k.sub_superscript_gap_min(), ctx);
            if gap < min {
                let delta = min - gap;
                let lift = (self.v(k.superscript_bottom_max_with_subscript(), ctx) - (u - sup.d))
                    .clamp(0.0, delta);
                u += lift;
                v += delta - lift;
            }
        }
        let space = self.v(k.space_after_script(), ctx);
        let mut f = Frame {
            a: base.a,
            d: base.d,
            ..Frame::default()
        };
        // A large operator's slant moves its subscript left; a letter's moves its superscript right.
        let (bw, ic) = (base.w, base.ic);
        let (sup_x, sub_x) = if base.largeop {
            (bw, bw - ic)
        } else {
            (bw + ic, bw)
        };
        f.put(base, 0.0, 0.0);
        let mut w = sup_x.max(bw);
        if let Some(sup) = sup {
            f.a = f.a.max(u + sup.a);
            w = w.max(sup_x + sup.w + sup.ic);
            f.put(sup, sup_x, u);
        }
        if let Some(sub) = sub {
            f.d = f.d.max(v + sub.d);
            w = w.max(sub_x + sub.w);
            f.put(sub, sub_x, -v);
        }
        f.w = w + space;
        f
    }

    fn limits(&self, n: &Node, ctx: Ctx) -> Frame {
        let k = &self.k;
        let base = self.node(&n.kids[0], ctx);
        let (under, over) = match n.tag.as_str() {
            "munder" => (Some(&n.kids[1]), None),
            "mover" => (None, Some(&n.kids[1])),
            _ => (Some(&n.kids[1]), n.kids.get(2)),
        };
        let accent_of = |m: &Node| {
            let m = m.single();
            let mut chars = m.text.chars();
            match (m.token(), chars.next(), chars.next()) {
                (true, Some(c), None) if !base.largeop => {
                    Some((c, m.attr("stretchy") == Some("true")))
                }
                _ => None,
            }
        };
        let mut f = Frame {
            a: base.a,
            d: base.d,
            ..Frame::default()
        };
        let (bw, bic, bglyph) = (base.w, base.ic, base.glyph);
        let mut w = bw;
        let mut placed: Vec<(Frame, f32, f32)> = Vec::new();
        if let Some(m) = over {
            match accent_of(m) {
                Some(('‾' | '¯', true) | ('‾', false)) => {
                    let t = self.v(k.overbar_rule_thickness(), ctx);
                    let y = base.a + self.v(k.overbar_vertical_gap(), ctx);
                    f.items.push(Item::Rule {
                        x: 0.0,
                        y,
                        w: bw,
                        h: t,
                        c: ctx.color,
                    });
                    f.a = f.a.max(y + t + self.v(k.overbar_extra_ascender(), ctx));
                }
                Some((c, stretchy)) if accent(c).is_some() || stretchy => {
                    let mark = accent(c).unwrap_or(c);
                    let s = self.scale(ctx);
                    let id = self
                        .face
                        .glyph_index(mark)
                        .or_else(|| self.face.glyph_index(c))
                        .map_or(0, |g| g.0);
                    let mut g = self.glyph(id, s, ctx);
                    if stretchy {
                        g = self.stretch(id, bw, s, ctx, false);
                    }
                    let lift = (base.a - self.v(k.accent_base_height(), ctx)).max(0.0);
                    let anchor = bglyph
                        .and_then(|id| self.attachment(id))
                        .map_or(bw / 2.0 + bic / 2.0, |x| x * s);
                    let own = g
                        .glyph
                        .and_then(|id| self.attachment(id).map(|x| x * s))
                        .unwrap_or_else(|| {
                            let ink = g
                                .glyph
                                .and_then(|id| self.face.glyph_bounding_box(GlyphId(id)));
                            ink.map_or(g.w / 2.0, |b| (b.x_min + b.x_max) as f32 / 2.0 * s)
                        });
                    let ga = g.a;
                    let x = if g.glyph.is_some() {
                        anchor - own
                    } else {
                        (bw - g.w) / 2.0
                    };
                    f.a = f.a.max(ga + lift);
                    placed.push((g, x, lift));
                }
                _ => {
                    let m = self.node(m, ctx.script());
                    let rise = self
                        .v(k.upper_limit_baseline_rise_min(), ctx)
                        .max(self.v(k.upper_limit_gap_min(), ctx) + m.d);
                    let y = base.a + rise;
                    f.a = f.a.max(y + m.a);
                    w = w.max(m.w);
                    placed.push((m, f32::NAN, y));
                }
            }
        }
        if let Some(m) = under {
            match accent_of(m) {
                Some(('_' | '‾', _)) => {
                    let t = self.v(k.underbar_rule_thickness(), ctx);
                    let y = -base.d - self.v(k.underbar_vertical_gap(), ctx) - t;
                    f.items.push(Item::Rule {
                        x: 0.0,
                        y,
                        w: bw,
                        h: t,
                        c: ctx.color,
                    });
                    f.d = f.d.max(-y + self.v(k.underbar_extra_descender(), ctx));
                }
                _ => {
                    let m = self.node(m, ctx.script().cramp());
                    let drop = self
                        .v(k.lower_limit_baseline_drop_min(), ctx)
                        .max(self.v(k.lower_limit_gap_min(), ctx) + m.a);
                    let y = -base.d - drop;
                    f.d = f.d.max(-y + m.d);
                    w = w.max(m.w);
                    placed.push((m, f32::NAN, y));
                }
            }
        }
        let shift = (w - bw) / 2.0;
        for item in &mut f.items {
            if let Item::Rule { x, .. } = item {
                *x += shift;
            }
        }
        f.put(base, shift, 0.0);
        for (m, x, y) in placed {
            let x = if x.is_nan() {
                (w - m.w) / 2.0 + if y > 0.0 { bic / 2.0 } else { -bic / 2.0 }
            } else {
                x + shift
            };
            f.put(m, x, y);
        }
        f.w = w;
        f
    }

    fn table(&self, n: &Node, ctx: Ctx) -> Frame {
        let align_like = n.class("menv-alignlike");
        let cell_ctx = Ctx {
            display: align_like,
            ..ctx
        };
        let em = self.em(ctx);
        let rows: Vec<Vec<(Frame, &Node)>> = n
            .kids
            .iter()
            .filter(|r| r.tag == "mtr")
            .map(|r| {
                r.kids
                    .iter()
                    .filter(|c| c.tag == "mtd" && !c.class("menv-border-only"))
                    .map(|c| (self.row(&c.kids, cell_ctx.apply(c)), c))
                    .collect()
            })
            .collect();
        let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
        let mut widths = vec![0f32; columns];
        for row in &rows {
            for (i, (f, _)) in row.iter().enumerate() {
                widths[i] = widths[i].max(f.w + f.ic);
            }
        }
        let gap = |i: usize| -> f32 {
            if align_like {
                if i % 2 == 1 {
                    0.0
                } else {
                    2.0 * em
                }
            } else {
                em
            }
        };
        let (strut_a, strut_d) = (0.84 * em, 0.36 * em);
        let lead = if align_like { 0.3 * em } else { 0.0 };
        // Keeps cells off a surrounding fence, as \arraycolsep does.
        let pad = if align_like { 0.0 } else { em / 6.0 };
        let metrics: Vec<(f32, f32)> = rows
            .iter()
            .map(|r| {
                r.iter().fold((strut_a, strut_d), |(a, d), (f, _)| {
                    (a.max(f.a), d.max(f.d))
                })
            })
            .collect();
        let height: f32 = metrics.iter().map(|(a, d)| a + d).sum::<f32>()
            + lead * metrics.len().saturating_sub(1) as f32;
        let axis = self.axis(ctx);
        let mut f = Frame {
            a: axis + height / 2.0,
            d: height / 2.0 - axis,
            ..Frame::default()
        };
        let mut top = f.a;
        for (row, (a, d)) in rows.into_iter().zip(metrics) {
            let baseline = top - a;
            let mut x = pad;
            for (i, (cell, node)) in row.into_iter().enumerate() {
                let room = widths[i] - cell.w - cell.ic;
                let left = n.class("menv-cells-left")
                    || node.class("cell-left")
                    || (align_like && i % 2 == 1);
                let right = n.class("menv-cells-right")
                    || node.class("cell-right")
                    || (align_like && i % 2 == 0);
                let dx = if left {
                    0.0
                } else if right {
                    room
                } else {
                    room / 2.0
                };
                f.put(cell, x + dx, baseline);
                x += widths[i] + if i + 1 < columns { gap(i + 1) } else { 0.0 };
            }
            top = baseline - d - lead;
        }
        f.w = widths.iter().sum::<f32>() + (1..columns).map(gap).sum::<f32>() + 2.0 * pad;
        f
    }

    fn emit(&self, f: Frame) -> Result<Value, Error> {
        let em = self.upem;
        let r = |v: f32| (v / em * 10000.0).round() / 10000.0;
        let mut paths = BTreeMap::new();
        let mut runs = Vec::new();
        let mut rules = Vec::new();
        let mut bytes = 0usize;
        if f.items.len() > 8192 {
            return Err(Error::Limit);
        }
        for item in &f.items {
            match *item {
                Item::Glyph { id, x, y, s, c } => {
                    if let std::collections::btree_map::Entry::Vacant(slot) = paths.entry(id) {
                        let mut path = Path(String::new());
                        self.face.outline_glyph(GlyphId(id), &mut path);
                        bytes += path.0.len();
                        slot.insert(path.0);
                    }
                    let mut run = vec![
                        json!(id),
                        json!(r(x)),
                        json!(r(-y)),
                        json!((s * 1000.0).round() / 1000.0),
                    ];
                    if let Some(c) = c {
                        run.push(json!(c));
                    }
                    runs.push(Value::Array(run));
                }
                Item::Rule { x, y, w, h, c } => {
                    let mut rule = vec![json!(r(x)), json!(r(-(y + h))), json!(r(w)), json!(r(h))];
                    if let Some(c) = c {
                        rule.push(json!(c));
                    }
                    rules.push(Value::Array(rule));
                }
            }
            if bytes > crate::MAX_WIRE_BYTES {
                return Err(Error::Limit);
            }
        }
        Ok(json!({
            "units": em as u32,
            "width": r(f.w), "ascent": r(f.a), "descent": r(f.d),
            "glyphs": paths.into_iter().map(|(id, d)| json!([id, d])).collect::<Vec<_>>(),
            "runs": runs, "rules": rules,
        }))
    }
}

// SVG path data in font units, y up.
struct Path(String);
impl OutlineBuilder for Path {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0 += &format!("M{} {}", x.round(), y.round());
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0 += &format!("L{} {}", x.round(), y.round());
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.0 += &format!("Q{} {} {} {}", x1.round(), y1.round(), x.round(), y.round());
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0 += &format!(
            "C{} {} {} {} {} {}",
            x1.round(),
            y1.round(),
            x2.round(),
            y2.round(),
            x.round(),
            y.round()
        );
    }
    fn close(&mut self) {
        self.0.push('Z');
    }
}
