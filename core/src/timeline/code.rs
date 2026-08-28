//! Syntax highlighting for fenced code. Qt's rich text has no stylesheet, so every colour
//! is inline on a `<span>`. syntect must use `regex-fancy`, not the default `onig`:
//! oniguruma is C, and this engine stays pure Rust so it travels to Android and iOS.

use std::sync::OnceLock;

use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Style, Theme, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

/// Past this, escape and move on.
const MAX_CODE_BYTES: usize = 64 * 1024;

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

/// One dark theme, always: a per-room palette would mean re-highlighting on theme change.
fn theme() -> &'static Theme {
    static THEME: OnceLock<Theme> = OnceLock::new();
    THEME.get_or_init(|| {
        let mut set = ThemeSet::load_defaults();
        set.themes
            .remove("base16-ocean.dark")
            .or_else(|| set.themes.remove("base16-eighties.dark"))
            .unwrap_or_default()
    })
}

/// Fence tags with no bundled syntax and no close relative; these stay plain monospace.
pub const MISSING: &[&str] = &["swift", "zig", "nim", "elixir", "dart", "toml", "ini", "proto", "powershell"];

/// The languages this build can highlight, by name.
pub fn languages() -> Vec<String> {
    syntaxes().syntaxes().iter().map(|s| s.name.clone()).collect()
}

/// Resolve a fence tag to a syntax: token, then name, then extension. `None` is not an error.
fn syntax_for(tag: Option<&str>) -> Option<&'static syntect::parsing::SyntaxReference> {
    let ss = syntaxes();
    let tag = tag?.trim().to_ascii_lowercase();
    if tag.is_empty() { return None }
    // Spellings syntect lacks by token, plus near-relative stand-ins; hopeless tags are in `MISSING`.
    let tag = match tag.as_str() {
        "sh" | "shell" | "zsh" | "console" | "dockerfile" => "bash",
        "js" | "mjs" | "cjs" | "node" | "jsx" => "javascript",
        // No TypeScript bundled; JS gets all but the type annotations right.
        "ts" | "typescript" | "tsx" | "qml" | "vue" | "svelte" => "javascript",
        "py" => "python",
        "rs" => "rust",
        "yml" => "yaml",
        "hpp" | "h" | "cc" | "cxx" | "cpp" => "c++",
        // Kotlin is not bundled either; Java is the closest thing present.
        "kt" | "kotlin" => "java",
        "cs" | "csharp" => "c#",
        "objc" => "objective-c",
        "jsonc" | "json5" => "json",
        "htm" => "html",
        other => other,
    };
    ss.find_syntax_by_token(tag)
        .or_else(|| ss.find_syntax_by_name(tag))
        .or_else(|| ss.find_syntax_by_extension(tag))
}

fn escape(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
}

fn span_open(style: &Style, out: &mut String) {
    let c = style.foreground;
    out.push_str("<span style=\"color:#");
    out.push_str(&format!("{:02x}{:02x}{:02x}", c.r, c.g, c.b));
    if style.font_style.contains(FontStyle::BOLD) { out.push_str("; font-weight:bold") }
    if style.font_style.contains(FontStyle::ITALIC) { out.push_str("; font-style:italic") }
    out.push_str("\">");
}

/// Highlight `code` into HTML safe inside a `<pre>`, falling back to escaped-but-uncoloured text.
pub fn highlight(code: &str, tag: Option<&str>) -> String {
    let mut out = String::with_capacity(code.len() * 2);
    let Some(syntax) = syntax_for(tag) else {
        escape(code, &mut out);
        return out
    };
    if code.len() > MAX_CODE_BYTES {
        escape(code, &mut out);
        return out
    }
    let mut h = HighlightLines::new(syntax, theme());
    for line in LinesWithEndings::from(code) {
        let Ok(ranges) = h.highlight_line(line, syntaxes()) else {
            // Give up cleanly: what is highlighted stays, the rest is plain.
            escape(line, &mut out);
            continue
        };
        for (style, text) in ranges {
            span_open(&style, &mut out);
            escape(text, &mut out);
            out.push_str("</span>");
        }
    }
    out
}

/// Neutral dark grey (equal channels) so it works under a bubble of any colour.
pub const BACKGROUND: &str = "#242424";
pub fn background() -> String { BACKGROUND.to_string() }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_is_in_there() {
        assert!(syntax_for(Some("rust")).is_some(), "rust by token");
        assert!(syntax_for(Some("rs")).is_some(), "rust by the extension people type");
        assert_eq!(syntax_for(Some("rust")).unwrap().name, "Rust");
    }

    #[test]
    fn the_usual_suspects_resolve() {
        for tag in ["python", "py", "javascript", "js", "c", "c++", "cpp", "go", "java",
                    "bash", "sh", "json", "yaml", "yml", "html", "css", "sql", "xml",
                    "ruby", "php", "lua", "haskell", "markdown", "diff", "perl", "scala",
                    "clojure", "erlang", "ocaml", "r", "matlab", "tex", "makefile"] {
            assert!(syntax_for(Some(tag)).is_some(), "{tag} should resolve");
        }
        assert!(languages().len() >= 40, "got {} syntaxes", languages().len());
    }

    #[test]
    fn languages_the_bundle_lacks_borrow_a_relative_or_stay_plain() {
        assert_eq!(syntax_for(Some("ts")).unwrap().name, "JavaScript");
        assert_eq!(syntax_for(Some("typescript")).unwrap().name, "JavaScript");
        assert_eq!(syntax_for(Some("kotlin")).unwrap().name, "Java");
        assert_eq!(syntax_for(Some("dockerfile")).unwrap().name, "Bourne Again Shell (bash)");
        for tag in MISSING {
            assert!(syntax_for(Some(tag)).is_none(), "{tag} should not pretend");
        }
    }

    #[test]
    fn an_unknown_or_missing_tag_is_plain_but_still_escaped() {
        let out = highlight("if a < b && c > d {}", Some("brainfuck-ish"));
        assert!(!out.contains("<span"), "no colouring without a syntax");
        assert!(out.contains("&lt;") && out.contains("&gt;") && out.contains("&amp;"));
        let none = highlight("<script>", None);
        assert_eq!(none, "&lt;script&gt;");
    }

    #[test]
    fn rust_actually_gets_coloured_and_stays_escaped() {
        let out = highlight("fn main() { let v: Vec<u8> = vec![1 & 2]; }", Some("rust"));
        assert!(out.contains("<span style=\"color:#"), "coloured: {out}");
        // The entities land in separate spans, so look for them individually.
        assert!(out.contains("&lt;") && out.contains("&gt;"), "generics escaped: {out}");
        assert!(out.contains("&amp;"), "ampersand escaped: {out}");
        assert!(!out.contains("Vec<u8>"), "raw angle brackets must not survive");
    }

    #[test]
    fn a_huge_block_is_escaped_rather_than_parsed() {
        let big = "let x = 1;\n".repeat(MAX_CODE_BYTES / 8);
        let out = highlight(&big, Some("rust"));
        assert!(!out.contains("<span"), "oversized blocks skip the parse");
    }

    #[test]
    fn the_background_is_a_dark_neutral_the_view_can_use() {
        let bg = background();
        assert!(bg.starts_with('#') && bg.len() == 7, "got {bg}");
        let v = u32::from_str_radix(&bg[1..], 16).expect("a hex colour");
        let (r, g, b) = ((v >> 16) & 0xff, (v >> 8) & 0xff, v & 0xff);
        assert!(r < 0x50 && g < 0x50 && b < 0x50, "dark: {bg}");
        // Neutral: no channel far from the others, or it stops being grey.
        let (hi, lo) = (r.max(g).max(b), r.min(g).min(b));
        assert!(hi - lo <= 0x10, "grey, not tinted: {bg}");
    }
}
