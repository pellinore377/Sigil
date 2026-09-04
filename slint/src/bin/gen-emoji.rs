//! Regenerate assets/emojis.json from Unicode's emoji-test.txt: every
//! fully-qualified emoji in the standard's order (so the picker's category
//! marks land at the group starts), base glyphs only, with its name as the
//! search key. The desktop shell's list has the same shape.
//!
//!   curl -O https://unicode.org/Public/emoji/16.0/emoji-test.txt
//!   cargo run --bin gen-emoji -- emoji-test.txt > assets/emojis.json
use std::io::Write;

fn main() -> std::io::Result<()> {
    let path = std::env::args().nth(1).expect("path to emoji-test.txt");
    let text = std::fs::read_to_string(path)?;
    let mut out = std::io::BufWriter::new(std::io::stdout());
    let mut in_component = false;
    let mut first = true;
    out.write_all(b"[")?;
    for line in text.lines() {
        if let Some(g) = line.strip_prefix("# group:") {
            in_component = g.trim() == "Component";
            continue;
        }
        if in_component || line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        // "1F600  ; fully-qualified  # 😀 E1.0 grinning face"
        let Some((_, rest)) = line.split_once(';') else { continue };
        let Some((status, comment)) = rest.split_once('#') else { continue };
        if status.trim() != "fully-qualified" {
            continue;
        }
        let mut words = comment.split_whitespace();
        let Some(glyph) = words.next() else { continue };
        let _version = words.next();
        let name: String = words.collect::<Vec<_>>().join(" ").to_lowercase();
        if name.contains("skin tone") {
            continue;
        }
        if !first {
            out.write_all(b",")?;
        }
        first = false;
        let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        write!(out, "{{\"e\":\"{}\",\"k\":\"{}\"}}", esc(glyph), esc(&name))?;
    }
    out.write_all(b"]\n")?;
    Ok(())
}
