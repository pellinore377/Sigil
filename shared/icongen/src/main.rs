//! Turns `shared/icons.json` into the per-platform icon tables.
//!
//! One emitter per target: add Swift, Kotlin or TypeScript by writing another
//! `emit_*` and another `Target` in `TARGETS`. QML and Slint exist so far. Run it from anywhere:
//!
//!     cargo run --manifest-path shared/icongen/Cargo.toml

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Two traps that cost real debugging; they ride along into every generated file.
const TRAPS: [&str; 4] = [
    "QML has no `font.families`: an element names ONE family, so an icon cannot share a",
    "string with words — split it into an IconLabel plus a Text.",
    "These are private-use codepoints that Nerd Fonts also claim, so the wrong family",
    "silently draws a DIFFERENT icon rather than tofu.",
];

struct Icon {
    name: String,
    codepoint: String,
    material: String,
    group: String,
}

struct Target {
    /// Written relative to the repo root.
    path: &'static str,
    render: fn(&[Icon]) -> String,
}

const TARGETS: [Target; 2] = [
    Target { path: "omarchy/components/Icons.qml", render: emit_qml },
    Target { path: "slint/ui/icons.slint", render: emit_slint },
];

fn main() {
    let root = repo_root();
    let icons = load(&root.join("shared/icons.json"));

    for target in TARGETS {
        let path = root.join(target.path);
        std::fs::write(&path, (target.render)(&icons))
            .unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
        println!("wrote {} ({} icons)", target.path, icons.len());
    }
}

/// `shared/icongen` -> repo root, or `argv[1]` if given.
fn repo_root() -> PathBuf {
    match std::env::args().nth(1) {
        Some(arg) => PathBuf::from(arg),
        None => Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("crate lives at <root>/shared/icongen")
            .to_path_buf(),
    }
}

fn load(path: &Path) -> Vec<Icon> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let table: serde_json::Value = serde_json::from_str(&text).expect("icons.json is not valid JSON");
    let table = table.as_object().expect("icons.json must be an object keyed by icon name");

    table
        .iter()
        .map(|(name, value)| {
            let field = |key: &str| {
                value[key]
                    .as_str()
                    .unwrap_or_else(|| panic!("{name}.{key} must be a string"))
                    .to_string()
            };
            let codepoint = field("codepoint");
            assert!(
                codepoint.len() == 4 && codepoint.chars().all(|c| c.is_ascii_hexdigit() && !c.is_lowercase()),
                "{name}: codepoint must be 4 upper-case hex digits, got {codepoint:?}"
            );
            Icon { name: name.clone(), codepoint, material: field("material"), group: field("group") }
        })
        .collect()
}

fn header(comment: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{comment} Generated from `shared/icons.json` by `shared/icongen`. Do not hand-edit.");
    let _ = writeln!(out, "{comment} Add or change an icon there, then re-run the generator.");
    out
}

/// A QML singleton, registered as `singleton Icons Icons.qml` in `components/qmldir`.
fn emit_qml(icons: &[Icon]) -> String {
    let pad = icons.iter().map(|i| i.name.len()).max().unwrap_or(0);
    let mut out = header("//");
    out.push_str("\npragma Singleton\nimport QtQuick\n\n");
    out.push_str("// Every icon in Sigil, by what it means here rather than what the icon set calls it.\n");
    out.push_str("// Call sites say `Icons.back`, never a glyph literal. Draw them with `IconLabel`.\n");
    for trap in TRAPS {
        let _ = writeln!(out, "// {trap}");
    }
    out.push_str("// Trailing comments are the canonical Material Symbols names: fonts.google.com/icons.\n");
    out.push_str("QtObject {\n");

    let mut group = "";
    for icon in icons {
        if icon.group != group {
            group = &icon.group;
            let _ = writeln!(out, "\n  // {group}");
        }
        let _ = writeln!(
            out,
            "  readonly property string {:pad$} \"\\u{}\"  // {}",
            format!("{}:", icon.name),
            icon.codepoint,
            icon.material,
            pad = pad + 1
        );
    }
    out.push_str("}\n");
    out
}

/// A Slint global, imported as `import { Icons } from "icons.slint";`.
/// Slint has no per-element font fallback either, so the same two traps apply:
/// an icon never shares a `Text` with words, and the wrong family silently
/// draws a different glyph, not tofu.
fn emit_slint(icons: &[Icon]) -> String {
    let pad = icons.iter().map(|i| i.name.len()).max().unwrap_or(0);
    let mut out = header("//");
    out.push_str("\n// Every icon in Sigil, by what it means here rather than what the icon set calls it.\n");
    out.push_str("// Call sites say `Icons.back`, never a glyph literal. Draw them with `IconLabel`.\n");
    for trap in TRAPS {
        let _ = writeln!(out, "// {trap}");
    }
    out.push_str("// Trailing comments are the canonical Material Symbols names: fonts.google.com/icons.\n");
    out.push_str("export global Icons {\n");
    let mut group = "";
    for icon in icons {
        if icon.group != group {
            group = &icon.group;
            let _ = writeln!(out, "\n    // {group}");
        }
        let _ = writeln!(
            out,
            "    out property <string> {:pad$} \"\\u{{{}}}\";  // {}",
            format!("{}:", icon.name),
            icon.codepoint,
            icon.material,
            pad = pad + 1
        );
    }
    out.push_str("}\n");
    out
}
