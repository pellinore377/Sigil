use pulldown_cmark::{Event, Options, Parser, Tag};
use std::fmt::Write;

// Probe wire format: UTF-16 start/end, hidden-prefix/suffix lengths, style id.
// Source is never rewritten by presentation. Unsupported constructs stay visible.
pub fn analyze(source: &str) -> String {
    let mut result = String::new();
    for (event, range) in Parser::new_ext(source, Options::ENABLE_STRIKETHROUGH).into_offset_iter()
    {
        let raw = &source[range.clone()];
        let (prefix, suffix, style) = match event {
            Event::Start(Tag::Strong) => (2, 2, 1),
            Event::Start(Tag::Emphasis) => (1, 1, 2),
            Event::Start(Tag::Strikethrough) => (2, 2, 3),
            Event::Code(_) => {
                let n = raw.bytes().take_while(|&c| c == b'`').count();
                (n, n, 4)
            }
            _ => continue,
        };
        if raw.len() < prefix + suffix {
            continue;
        }
        let start = source[..range.start].encode_utf16().count();
        let end = start + raw.encode_utf16().count();
        writeln!(result, "{start},{end},{prefix},{suffix},{style}").unwrap();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::analyze;
    #[test]
    fn commonmark_nesting_and_utf16_offsets() {
        assert_eq!(analyze("👩🏽‍💻 **שלום**"), "8,16,2,2,1\n");
        assert_eq!(analyze("**bold *inner***"), "0,16,2,2,1\n7,14,1,1,2\n");
    }
    #[test]
    fn incomplete_and_escaped_syntax_remains_literal() {
        assert_eq!(analyze(r"\*\*literal\*\*"), "");
        assert_eq!(analyze("**incomplete"), "");
    }
    #[test]
    fn code_suppresses_markdown_styles() {
        assert_eq!(analyze("`**literal**`"), "0,13,1,1,4\n");
        assert_eq!(analyze("```\n**literal**\n```"), "");
    }
}
