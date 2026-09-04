//! Emoji as pictures. Slint's renderers draw text from outlines, and a
//! colour emoji font has none, so a reaction or a picker cell would come
//! out as a grey fallback glyph. The engine reads the bitmap out of the
//! system's colour emoji font (Noto Color Emoji ships one PNG per glyph)
//! and hands the frontend a file to show as an image. Nothing is fetched;
//! without such a font the reply says so and the frontend keeps the text.
//!
//! Most emoji are more than one code point — a flag is two regional
//! indicators, a toned hand is a base and a modifier, a family is four
//! people sewn together with joiners — and the font keeps ONE picture for
//! each of those, on a glyph that no single code point maps to. Reaching it
//! is the small half of what a text shaper does: map each code point through
//! `cmap`, then fold the run with the font's own GSUB ligature rules until
//! one glyph is left, and cut that glyph's raster. A sequence the font does
//! not compose leaves more than one glyph behind and is refused, so the
//! caller never gets a fragment of what it asked for.

use serde_json::{json, Value};
use std::path::PathBuf;
use ttf_parser::{gsub::SubstitutionSubtable, Face, GlyphId, RasterGlyphImage, Tag};

/// Where a colour emoji font usually lives, in the order tried. The
/// `SIGIL_EMOJI_FONT` environment variable comes first.
const CANDIDATES: &[&str] = &[
    "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf",
    "/usr/share/fonts/noto/NotoColorEmoji.ttf",
    "/usr/share/fonts/google-noto-emoji/NotoColorEmoji.ttf",
    "/usr/share/fonts/TTF/NotoColorEmoji.ttf",
    "/system/fonts/NotoColorEmoji.ttf",
];

fn font_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("SIGIL_EMOJI_FONT") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        for rel in [".local/share/fonts/NotoColorEmoji.ttf", ".fonts/NotoColorEmoji.ttf"] {
            let p = PathBuf::from(&home).join(rel);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    CANDIDATES.iter().map(PathBuf::from).find(|p| p.is_file())
}

/// The presentation selectors: emoji (U+FE0F) and text (U+FE0E). They pick a
/// look rather than name a picture, so a leftover one is dropped.
const SELECTORS: [char; 2] = ['\u{fe0f}', '\u{fe0e}'];

/// Longer than any emoji: the widest standard sequence is a tag flag at
/// seven code points, and a toned family of four comes to eleven. A longer
/// string is a sentence, and is refused before the font is opened.
const MAX_CODE_POINTS: usize = 32;

/// A ligature can only shorten the run, so a pass count this size is reached
/// only by a cyclic rule in a broken font. It stops the walk either way.
const MAX_PASSES: usize = 8;

/// The features whose lookups compose a sequence. `ccmp` is the one Noto
/// Color Emoji uses for every flag, tone and joined sequence; `rlig` and
/// `liga` are here for the other emoji fonts that ligate under those.
/// Nothing optional (`dlig`, `hlig`) is applied: those are a typographer's
/// choice, off unless asked for, and would compose things nobody asked to
/// compose.
const COMPOSING: [&[u8; 4]; 3] = [b"ccmp", b"rlig", b"liga"];

/// Every code point of `text` as a glyph, before any substitution.
///
/// A presentation selector is folded into the character before it when the
/// font's `cmap` carries that pair (format 14, the same lookup this file has
/// always done, one step smaller); otherwise it stays as its own glyph,
/// because Noto ligates U+FE0F as a component of the keycaps and of several
/// joined faces. A code point the font has never heard of ends the walk:
/// there is no picture to be had.
fn to_glyphs(face: &Face, text: &str) -> Option<Vec<GlyphId>> {
    let mut out: Vec<GlyphId> = Vec::new();
    let mut cs = text.chars().peekable();
    while let Some(c) = cs.next() {
        if SELECTORS.contains(&c) {
            // A selector whose base did not want it. Keep it if the font has
            // a glyph for it, drop it if not.
            if let Some(g) = face.glyph_index(c) {
                out.push(g);
            }
            continue;
        }
        if let Some(&next) = cs.peek() {
            if SELECTORS.contains(&next) {
                if let Some(g) = face.glyph_variation_index(c, next) {
                    out.push(g);
                    cs.next();
                    continue;
                }
            }
        }
        out.push(face.glyph_index(c)?);
    }
    (!out.is_empty()).then_some(out)
}

/// The lookups the composing features name, in the order the font lists
/// them. Taken from the feature list rather than from every lookup in the
/// table, so a lookup that only a contextual rule reaches is not fired at a
/// run that no rule matched.
fn composing_lookups(face: &Face) -> Vec<u16> {
    let mut want: Vec<u16> = Vec::new();
    let Some(gsub) = face.tables().gsub else {
        return want;
    };
    for feature in gsub.features.into_iter() {
        if !COMPOSING.iter().any(|t| feature.tag == Tag::from_bytes(t)) {
            continue;
        }
        for i in feature.lookup_indices {
            if !want.contains(&i) {
                want.push(i);
            }
        }
    }
    want.sort_unstable();
    want
}

/// One pass of every ligature subtable in `lookups` over the run, left to
/// right. Returns whether anything was folded.
///
/// At each position the font offers a set of ligatures that start with the
/// glyph there; the longest whose components match the glyphs that follow
/// wins, which is the composed emoji when a shorter piece of it also exists
/// (the man and the woman inside a family, say).
fn ligate_once(face: &Face, glyphs: &mut Vec<GlyphId>, lookups: &[u16]) -> bool {
    let Some(gsub) = face.tables().gsub else {
        return false;
    };
    let mut folded = false;
    for &index in lookups {
        let Some(lookup) = gsub.lookups.get(index) else {
            continue;
        };
        for subtable in lookup.subtables.into_iter::<SubstitutionSubtable>() {
            let SubstitutionSubtable::Ligature(table) = subtable else {
                continue;
            };
            let mut at = 0;
            while at < glyphs.len() {
                let set = table
                    .coverage
                    .get(glyphs[at])
                    .and_then(|i| table.ligature_sets.get(i));
                let mut best: Option<(usize, GlyphId)> = None;
                if let Some(set) = set {
                    for i in 0..set.len() {
                        let Some(lig) = set.get(i) else { continue };
                        let n = usize::from(lig.components.len());
                        if at + 1 + n > glyphs.len() {
                            continue;
                        }
                        let fits = (0..n).all(|j| {
                            lig.components.get(j as u16) == Some(glyphs[at + 1 + j])
                        });
                        if fits && best.map(|(len, _)| n + 1 > len).unwrap_or(true) {
                            best = Some((n + 1, lig.glyph));
                        }
                    }
                }
                if let Some((len, glyph)) = best {
                    glyphs.splice(at..at + len, [glyph]);
                    folded = true;
                }
                at += 1;
            }
        }
    }
    folded
}

/// The one glyph the font composes `text` into, or `None` when it composes
/// it into several — a sequence this font does not know, which has no
/// picture of its own and must not be drawn as its opening piece.
pub fn glyph_of(face: &Face, text: &str) -> Option<GlyphId> {
    if text.is_empty() || text.chars().count() > MAX_CODE_POINTS {
        return None;
    }
    let mut glyphs = to_glyphs(face, text)?;
    let lookups = composing_lookups(face);
    for _ in 0..MAX_PASSES {
        if glyphs.len() < 2 || !ligate_once(face, &mut glyphs, &lookups) {
            break;
        }
    }
    if glyphs.len() > 1 {
        // A selector no rule consumed is decoration on a glyph that is
        // already the right one; the picture is the same either way.
        let marks: Vec<GlyphId> = SELECTORS.iter().filter_map(|c| face.glyph_index(*c)).collect();
        glyphs.retain(|g| !marks.contains(g));
    }
    match glyphs.as_slice() {
        [one] => Some(*one),
        _ => None,
    }
}

/// The raster the font keeps for `text`, whole.
fn picture<'f>(face: &'f Face, text: &str) -> Option<RasterGlyphImage<'f>> {
    let raster = |g| face.glyph_raster_image(g, u16::MAX);
    if let Some(img) = glyph_of(face, text).and_then(raster) {
        return Some(img);
    }
    // A text-default emoji written bare (a mountain, a tent) carries its
    // picture only on the glyph the emoji-presentation pair maps to, so that
    // is the second try — for a lone code point, which is the only shape
    // that question makes sense for.
    let mut cs = text.chars().filter(|c| !SELECTORS.contains(c));
    let one = cs.next()?;
    cs.next().is_none().then_some(())?;
    face.glyph_variation_index(one, '\u{fe0f}').and_then(raster)
}

/// Whether this face can draw `text` as one picture. The picker asks this of
/// every entry it is about to offer, so that it never offers a cell the
/// engine would answer `not_found` for.
pub fn drawable(face: &Face, text: &str) -> bool {
    picture(face, text).is_some()
}

/// The cache file's name: every code point in hex, joined. A lone code point
/// keeps the name it had, so pictures cut before this still count.
fn cache_name(text: &str) -> String {
    let mut name = String::new();
    for c in text.chars() {
        if !name.is_empty() {
            name.push('-');
        }
        name.push_str(&format!("{:x}", c as u32));
    }
    name.push_str(".png");
    name
}

/// `emoji.render{text, size?}` → `{path, width, height}`; the file is a
/// PNG in the cache, made once per emoji.
pub fn render(p: &serde_json::Map<String, Value>) -> crate::ipc::wire::Reply {
    use crate::ipc::wire::Reply;
    let text = p.get("text").and_then(Value::as_str).unwrap_or("").trim();
    if text.is_empty() || text.chars().count() > MAX_CODE_POINTS {
        return Reply::err("bad_request", "nothing to draw");
    }
    let dir = crate::paths::cache_dir().join("emoji");
    let _ = std::fs::create_dir_all(&dir);
    let out = dir.join(cache_name(text));
    if !out.is_file() {
        let Some(font) = font_path() else {
            return Reply::err("unavailable", "no colour emoji font on this device");
        };
        let Ok(bytes) = std::fs::read(&font) else {
            return Reply::err("unavailable", "the emoji font could not be read");
        };
        let Ok(face) = Face::parse(&bytes, 0) else {
            return Reply::err("unavailable", "the emoji font could not be parsed");
        };
        let Some(img) = picture(&face, text) else {
            return Reply::err("not_found", "the font has no picture for that");
        };
        if img.format != ttf_parser::RasterImageFormat::PNG {
            return Reply::err("unavailable", "the emoji font's pictures are not PNG");
        }
        if std::fs::write(&out, img.data).is_err() {
            return Reply::err("internal", "could not write the picture");
        }
    }
    match image::image_dimensions(&out) {
        Ok((w, h)) => Reply::ok(json!({"path": out.to_string_lossy(), "width": w, "height": h})),
        Err(_) => Reply::err("internal", "the picture could not be read back"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A font built here, so the substitution walk is tested against known
    /// rules rather than against whichever Noto this machine happens to
    /// carry. Four glyphs: 1 = A (U+0041), 2 = B (U+0042), 3 = the selector
    /// U+FE0F, 4 = the ligature of A+B, 5 = the ligature of A+B+FE0F.
    mod fake {
        /// `cmap` format 12, one group per code point, sorted.
        fn cmap(map: &[(u32, u16)]) -> Vec<u8> {
            let mut sub: Vec<u8> = Vec::new();
            sub.extend(12u16.to_be_bytes()); // format
            sub.extend(0u16.to_be_bytes()); // reserved
            sub.extend(((16 + 12 * map.len()) as u32).to_be_bytes()); // length
            sub.extend(0u32.to_be_bytes()); // language
            sub.extend((map.len() as u32).to_be_bytes());
            for (c, g) in map {
                sub.extend(c.to_be_bytes());
                sub.extend(c.to_be_bytes());
                sub.extend(u32::from(*g).to_be_bytes());
            }
            let mut t: Vec<u8> = Vec::new();
            t.extend(0u16.to_be_bytes()); // version
            t.extend(1u16.to_be_bytes()); // one subtable
            t.extend(3u16.to_be_bytes()); // Windows
            t.extend(10u16.to_be_bytes()); // full repertoire
            t.extend(12u32.to_be_bytes()); // offset to the subtable
            t.extend(sub);
            t
        }

        /// `GSUB` with one `ccmp` feature over one ligature lookup.
        /// Each entry is (first glyph, the rest, the ligature).
        fn gsub(ligs: &[(u16, &[u16], u16)]) -> Vec<u8> {
            // The ligature sets, one per covered first glyph, in order.
            let mut firsts: Vec<u16> = ligs.iter().map(|(f, _, _)| *f).collect();
            firsts.dedup();
            let mut sets: Vec<Vec<u8>> = Vec::new();
            for first in &firsts {
                let mine: Vec<&(u16, &[u16], u16)> =
                    ligs.iter().filter(|(f, _, _)| f == first).collect();
                let mut tables: Vec<Vec<u8>> = Vec::new();
                for (_, rest, lig) in &mine {
                    let mut t: Vec<u8> = Vec::new();
                    t.extend(lig.to_be_bytes());
                    t.extend(((rest.len() + 1) as u16).to_be_bytes());
                    for g in rest.iter() {
                        t.extend(g.to_be_bytes());
                    }
                    tables.push(t);
                }
                let mut set: Vec<u8> = Vec::new();
                set.extend((tables.len() as u16).to_be_bytes());
                let mut at = 2 + 2 * tables.len();
                for t in &tables {
                    set.extend((at as u16).to_be_bytes());
                    at += t.len();
                }
                for t in &tables {
                    set.extend(t);
                }
                sets.push(set);
            }
            // Coverage format 1 over the first glyphs.
            let mut cov: Vec<u8> = Vec::new();
            cov.extend(1u16.to_be_bytes());
            cov.extend((firsts.len() as u16).to_be_bytes());
            for f in &firsts {
                cov.extend(f.to_be_bytes());
            }
            // The ligature substitution subtable.
            let mut sub: Vec<u8> = Vec::new();
            sub.extend(1u16.to_be_bytes()); // format
            let head = 6 + 2 * sets.len();
            sub.extend((head as u16).to_be_bytes()); // coverage, straight after
            sub.extend((sets.len() as u16).to_be_bytes());
            let mut at = head + cov.len() + cov.len() % 2;
            for s in &sets {
                sub.extend((at as u16).to_be_bytes());
                at += s.len();
            }
            sub.extend(&cov);
            if cov.len() % 2 == 1 {
                sub.push(0);
            }
            for s in &sets {
                sub.extend(s);
            }
            // One lookup holding it.
            let mut lookup: Vec<u8> = Vec::new();
            lookup.extend(4u16.to_be_bytes()); // ligature substitution
            lookup.extend(0u16.to_be_bytes()); // flags
            lookup.extend(1u16.to_be_bytes()); // one subtable
            lookup.extend(8u16.to_be_bytes()); // its offset
            lookup.extend(sub);
            let mut lookups: Vec<u8> = Vec::new();
            lookups.extend(1u16.to_be_bytes());
            lookups.extend(4u16.to_be_bytes());
            lookups.extend(lookup);
            // One `ccmp` feature naming lookup 0.
            let mut feature: Vec<u8> = Vec::new();
            feature.extend(0u16.to_be_bytes()); // no params
            feature.extend(1u16.to_be_bytes()); // one lookup
            feature.extend(0u16.to_be_bytes()); // index 0
            let mut features: Vec<u8> = Vec::new();
            features.extend(1u16.to_be_bytes());
            features.extend(*b"ccmp");
            features.extend(8u16.to_be_bytes());
            features.extend(feature);
            // DFLT/dflt naming feature 0.
            let mut langsys: Vec<u8> = Vec::new();
            langsys.extend(0u16.to_be_bytes()); // lookup order
            langsys.extend(0xffffu16.to_be_bytes()); // no required feature
            langsys.extend(1u16.to_be_bytes());
            langsys.extend(0u16.to_be_bytes());
            let mut script: Vec<u8> = Vec::new();
            script.extend(4u16.to_be_bytes()); // default langsys offset
            script.extend(0u16.to_be_bytes()); // no other langsys
            script.extend(langsys);
            let mut scripts: Vec<u8> = Vec::new();
            scripts.extend(1u16.to_be_bytes());
            scripts.extend(*b"DFLT");
            scripts.extend(8u16.to_be_bytes());
            scripts.extend(script);

            let mut t: Vec<u8> = Vec::new();
            t.extend(1u16.to_be_bytes()); // major
            t.extend(0u16.to_be_bytes()); // minor
            let at = 10;
            t.extend((at as u16).to_be_bytes());
            t.extend(((at + scripts.len()) as u16).to_be_bytes());
            t.extend(((at + scripts.len() + features.len()) as u16).to_be_bytes());
            t.extend(scripts);
            t.extend(features);
            t.extend(lookups);
            t
        }

        fn head() -> Vec<u8> {
            let mut t = vec![0u8; 54];
            t[18..20].copy_from_slice(&1000u16.to_be_bytes()); // units per em
            t
        }

        fn hhea() -> Vec<u8> {
            vec![0u8; 36]
        }

        fn maxp(glyphs: u16) -> Vec<u8> {
            let mut t: Vec<u8> = Vec::new();
            t.extend(0x00005000u32.to_be_bytes());
            t.extend(glyphs.to_be_bytes());
            t
        }

        /// The whole face: the four tables above, in tag order.
        pub fn face(map: &[(u32, u16)], ligs: &[(u16, &[u16], u16)], glyphs: u16) -> Vec<u8> {
            let tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
                (b"GSUB", gsub(ligs)),
                (b"cmap", cmap(map)),
                (b"head", head()),
                (b"hhea", hhea()),
                (b"maxp", maxp(glyphs)),
            ];
            let mut out: Vec<u8> = Vec::new();
            out.extend(0x00010000u32.to_be_bytes()); // TrueType
            out.extend((tables.len() as u16).to_be_bytes());
            out.extend([0u8; 6]); // search range and friends, unread
            let mut at = 12 + 16 * tables.len();
            for (tag, data) in &tables {
                out.extend(**tag);
                out.extend(0u32.to_be_bytes()); // checksum, unread
                out.extend((at as u32).to_be_bytes());
                out.extend((data.len() as u32).to_be_bytes());
                at += data.len() + (4 - data.len() % 4) % 4;
            }
            for (_, data) in &tables {
                out.extend(data);
                out.extend(vec![0u8; (4 - data.len() % 4) % 4]);
            }
            out
        }
    }

    /// A, B, the selector, and ligatures for A+B and A+B+selector.
    fn built() -> Vec<u8> {
        fake::face(
            &[(0x41, 1), (0x42, 2), (0xfe0f, 3)],
            &[(1, &[2], 4), (1, &[2, 3], 5)],
            6,
        )
    }

    #[test]
    fn a_sequence_the_font_composes_comes_back_as_the_composed_glyph() {
        let bytes = built();
        let face = Face::parse(&bytes, 0).unwrap();
        assert_eq!(glyph_of(&face, "A"), Some(GlyphId(1)));
        assert_eq!(glyph_of(&face, "AB"), Some(GlyphId(4)));
        // The longer rule wins over the shorter one it contains.
        assert_eq!(glyph_of(&face, "AB\u{fe0f}"), Some(GlyphId(5)));
    }

    #[test]
    fn a_sequence_the_font_does_not_compose_fails_rather_than_giving_a_piece() {
        let bytes = built();
        let face = Face::parse(&bytes, 0).unwrap();
        // B then A is no ligature of this font's: two glyphs, no picture.
        assert_eq!(glyph_of(&face, "BA"), None);
        // Nor is a code point it has never mapped.
        assert_eq!(glyph_of(&face, "AZ"), None);
        assert_eq!(glyph_of(&face, "Z"), None);
        assert_eq!(glyph_of(&face, ""), None);
    }

    #[test]
    fn a_selector_no_rule_wanted_is_dropped() {
        // U+FE0E is mapped by neither cmap nor a ligature here, so a lone
        // glyph carrying it still resolves to the lone glyph.
        let bytes = built();
        let face = Face::parse(&bytes, 0).unwrap();
        assert_eq!(glyph_of(&face, "A\u{fe0e}"), Some(GlyphId(1)));
        // And U+FE0F, which IS mapped, is dropped when nothing ligates it.
        assert_eq!(glyph_of(&face, "B\u{fe0f}"), Some(GlyphId(2)));
    }

    #[test]
    fn a_run_longer_than_any_emoji_is_refused() {
        let bytes = built();
        let face = Face::parse(&bytes, 0).unwrap();
        assert_eq!(glyph_of(&face, &"A".repeat(MAX_CODE_POINTS + 1)), None);
    }

    #[test]
    fn the_cache_name_covers_the_whole_sequence() {
        assert_eq!(cache_name("👍"), "1f44d.png");
        assert_eq!(cache_name("🇬🇧"), "1f1ec-1f1e7.png");
        assert_eq!(cache_name("👍🏽"), "1f44d-1f3fd.png");
        assert_eq!(cache_name("1\u{fe0f}\u{20e3}"), "31-fe0f-20e3.png");
    }

    #[test]
    fn a_picture_comes_out_of_the_system_font_when_there_is_one() {
        if font_path().is_none() {
            eprintln!("skipping: no colour emoji font");
            return;
        }
        let mut p = serde_json::Map::new();
        p.insert("text".into(), json!("👍"));
        match render(&p) {
            crate::ipc::wire::Reply::Ok(v) => {
                assert!(v["width"].as_u64().unwrap_or(0) > 0);
                assert!(std::path::Path::new(v["path"].as_str().unwrap()).is_file());
            }
            crate::ipc::wire::Reply::Err(e) => panic!("{}: {}", e.code, e.message),
        }
    }

    /// The composed cases, on whatever emoji font this machine has. Skipped
    /// where there is none, and forgiving of a font too old to know a given
    /// sequence — what it must never do is hand back a picture for a
    /// sequence it could not compose.
    #[test]
    fn composed_sequences_render_on_the_system_font() {
        let Some(path) = font_path() else {
            eprintln!("skipping: no colour emoji font");
            return;
        };
        let bytes = std::fs::read(path).unwrap();
        let face = Face::parse(&bytes, 0).unwrap();
        if face.tables().gsub.is_none() {
            eprintln!("skipping: that font composes nothing");
            return;
        }
        for seq in ["🇬🇧", "👍🏽", "👨‍👩‍👧‍👦", "1\u{fe0f}\u{20e3}", "👩‍🚒"] {
            // A font too old to know a sequence composes nothing for it, and
            // that is the right answer; what it must not do is answer with a
            // piece of it, or with a glyph that has no picture.
            let Some(one) = glyph_of(&face, seq) else { continue };
            let opening = face.glyph_index(seq.chars().next().unwrap());
            assert_ne!(Some(one), opening, "{seq} came back as its opening glyph");
            assert!(drawable(&face, seq), "{seq} composed to a glyph with no picture");
        }
        // Two whole emoji in a row are not one picture, on any font.
        assert!(glyph_of(&face, "👍👍").is_none());
    }
}
