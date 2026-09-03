//! Emoji as pictures. Slint's renderers draw text from outlines, and a
//! colour emoji font has none, so a reaction or a picker cell would come
//! out as a grey fallback glyph. The engine reads the bitmap out of the
//! system's colour emoji font (Noto Color Emoji ships one PNG per glyph)
//! and hands the frontend a file to show as an image. Nothing is fetched;
//! without such a font the reply says so and the frontend keeps the text.

use serde_json::{json, Value};
use std::path::PathBuf;

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

/// The scalar the font is asked for: variation selectors and joiners are
/// dropped, and a sequence falls back to its first picture.
fn first_scalar(text: &str) -> Option<char> {
    text.chars().find(|c| !matches!(*c, '\u{fe0f}' | '\u{fe0e}' | '\u{200d}' | '\u{20e3}'))
}

/// `emoji.render{text, size?}` → `{path, width, height}`; the file is a
/// PNG in the cache, made once per emoji.
pub fn render(p: &serde_json::Map<String, Value>) -> crate::ipc::wire::Reply {
    use crate::ipc::wire::Reply;
    let text = p.get("text").and_then(Value::as_str).unwrap_or("").trim();
    let Some(ch) = first_scalar(text) else {
        return Reply::err("bad_request", "nothing to draw");
    };
    let dir = crate::paths::cache_dir().join("emoji");
    let _ = std::fs::create_dir_all(&dir);
    let out = dir.join(format!("{:x}.png", ch as u32));
    if !out.is_file() {
        let Some(font) = font_path() else {
            return Reply::err("unavailable", "no colour emoji font on this device");
        };
        let Ok(bytes) = std::fs::read(&font) else {
            return Reply::err("unavailable", "the emoji font could not be read");
        };
        let Ok(face) = ttf_parser::Face::parse(&bytes, 0) else {
            return Reply::err("unavailable", "the emoji font could not be parsed");
        };
        let Some(gid) = face.glyph_index(ch) else {
            return Reply::err("not_found", "the font has no picture for that");
        };
        let Some(img) = face.glyph_raster_image(gid, u16::MAX) else {
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

    #[test]
    fn selectors_and_joiners_are_skipped() {
        assert_eq!(first_scalar("❤️"), Some('❤'));
        assert_eq!(first_scalar("\u{fe0f}"), None);
        assert_eq!(first_scalar("👍🏽"), Some('👍'));
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
}
