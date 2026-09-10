use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::fmt::Write;
use unicode_segmentation::UnicodeSegmentation;

// UTF-16 start/end, hidden-prefix/suffix lengths, style id, optional style argument.
// Source is never rewritten by presentation. Unsupported constructs stay visible.
pub fn analyze(source: &str) -> String {
    if source.len() > sigil_text::Limits::default().source_bytes {
        return String::new();
    }
    markdown(source, &utf16_offsets(source), false)
}
fn utf16_offsets(source: &str) -> Vec<usize> {
    let mut offsets = vec![0; source.len() + 1];
    let mut length = 0;
    for (at, ch) in source.char_indices() {
        offsets[at] = length;
        length += ch.len_utf16();
    }
    offsets[source.len()] = length;
    offsets
}
fn markdown(source: &str, offsets: &[usize], escapes: bool) -> String {
    let mut result = String::new();
    let mut code = false;
    for (event, range) in Parser::new_ext(source, Options::ENABLE_STRIKETHROUGH).into_offset_iter()
    {
        let raw = &source[range.clone()];
        if matches!(event, Event::Start(Tag::CodeBlock(_))) {
            code = true;
        }
        if matches!(event, Event::End(TagEnd::CodeBlock)) {
            code = false;
        }
        if escapes
            && !code
            && matches!(event, Event::Text(_))
            && raw.as_bytes().first().is_some_and(u8::is_ascii_punctuation)
            && source.as_bytes()[..range.start]
                .iter()
                .rev()
                .take_while(|&&b| b == b'\\')
                .count()
                % 2
                == 1
        {
            writeln!(
                result,
                "{},{},1,0,0",
                offsets[range.start - 1],
                offsets[range.start + 1]
            )
            .unwrap();
        }
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
        let start = offsets[range.start];
        let end = offsets[range.end];
        writeln!(result, "{start},{end},{prefix},{suffix},{style}").unwrap();
    }
    result
}

pub fn editor(source: &str) -> String {
    let Ok(spans) = sigil_text::editor_spans(source) else {
        return String::new();
    };
    let offsets = utf16_offsets(source);
    let mut result = markdown(source, &offsets, true);
    for span in spans {
        let start = offsets[span.start];
        let end = offsets[span.end];
        let prefix = offsets[span.start + span.prefix] - start;
        let suffix = end - offsets[span.end - span.suffix];
        let mut write = |style: u8, argument: Option<String>| {
            write!(result, "{start},{end},{prefix},{suffix},{style}").unwrap();
            if let Some(argument) = argument {
                write!(result, ",{argument}").unwrap();
            }
            result.push('\n');
        };
        let e = span.effects;
        for (enabled, id) in [
            (e.bold, 1),
            (e.italic, 2),
            (e.strike, 3),
            (e.mono || e.code, 4),
            (e.underline, 5),
            (e.mark && e.paint.is_none(), 6),
        ] {
            if enabled {
                write(id, None);
            }
        }
        if let Some(reveal) = e.reveal {
            write(
                if reveal == sigil_text::Reveal::Spoiler {
                    7
                } else {
                    8
                },
                None,
            );
        }
        if let Some(size) = e.size {
            write(9, Some(size.to_string()));
        }
        if let Some(paint) = e.paint {
            let mut colors = match paint {
                sigil_text::Paint::Solid { color } => String::from(color),
                sigil_text::Paint::Gradient { stops } => stops
                    .into_iter()
                    .map(String::from)
                    .collect::<Vec<_>>()
                    .join(":"),
                sigil_text::Paint::Rainbow => {
                    "red2:orange2:yellow2:green2:cyan2:blue2:purple2".into()
                }
                sigil_text::Paint::Theme => continue,
            };
            if colors.contains(':') && !e.mark {
                let body_start = span.start + span.prefix;
                let body_end = span.end - span.suffix;
                let mut boundaries: Vec<_> = source[body_start..body_end]
                    .grapheme_indices(true)
                    .map(|(at, _)| offsets[body_start + at])
                    .collect();
                boundaries.push(offsets[body_end]);
                let count = (boundaries.len() - 1).min(256);
                colors.push('|');
                for index in 0..=count {
                    if index > 0 {
                        colors.push(':');
                    }
                    write!(
                        colors,
                        "{}",
                        boundaries[index * (boundaries.len() - 1) / count]
                    )
                    .unwrap();
                }
            }
            write(if e.mark { 11 } else { 10 }, Some(colors));
        }
        if let Some(animation) = e.animation {
            let name = match animation {
                sigil_text::Animation::Shake => "shake",
                sigil_text::Animation::Wave => "wave",
                sigil_text::Animation::Pulse => "pulse",
                sigil_text::Animation::Glow => "glow",
                sigil_text::Animation::Typewriter => "typewriter",
                sigil_text::Animation::Sparkle => "sparkle",
                sigil_text::Animation::Glitch => "glitch",
                sigil_text::Animation::Scatter => "scatter",
                sigil_text::Animation::Flip => "flip",
                sigil_text::Animation::Barrel => "barrel",
            };
            write(12, Some(name.into()));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{analyze, editor};
    #[test]
    fn editor_uses_named_sigiltext_without_reinterpreting_plain_messages() {
        let source = "👩🏽‍💻 underline::bold::שלום; red-blue::café; big1::wide;";
        assert_eq!(analyze(source), "");
        assert_eq!(
            editor(source),
            "8,30,17,1,1\n8,30,17,1,5\n31,46,10,1,10,red2:blue2|41:42:43:44:45\n47,58,6,1,9,1\n"
        );
        assert_eq!(editor("mark::green::leaf;"), "0,18,13,1,11,green2\n");
        assert_eq!(editor("spoiler::secret;"), "0,16,9,1,7\n");
        assert_eq!(editor("||secret||"), "0,10,2,2,7\n");
    }
    #[test]
    fn editor_respects_code_escapes_redaction_and_parser_limits() {
        assert_eq!(editor("`underline::literal;`"), "0,21,1,1,4\n");
        assert_eq!(editor("```\nunderline::literal;\n```"), "");
        assert_eq!(editor(r"\underline::literal;"), "");
        assert_eq!(editor("underline::redact::secret;"), "");
        assert_eq!(editor("unknown::literal;"), "");
        assert!(editor(&"x".repeat(sigil_text::Limits::default().source_bytes + 1)).is_empty());
    }
    #[test]
    fn editable_gradients_are_bounded_and_never_split_combining_or_joined_characters() {
        assert_eq!(
            editor("red-blue::a👩🏽‍💻e\u{301};"),
            "0,21,10,1,10,red2:blue2|10:11:18:20\n"
        );
        let wire = editor(&format!("rainbow::{};", "a".repeat(16_000)));
        let bounds: Vec<usize> = wire
            .trim()
            .split_once('|')
            .unwrap()
            .1
            .split(':')
            .map(|n| n.parse().unwrap())
            .collect();
        assert_eq!(bounds.len(), 257);
        assert_eq!(bounds[0], 9);
        assert_eq!(bounds[256], 16_009);
        assert!(bounds.windows(2).all(|pair| pair[0] < pair[1]));
    }
    #[test]
    fn editor_distinguishes_escaped_punctuation_from_code() {
        assert_eq!(editor(r"\[REDACTED\]"), "0,2,1,0,0\n10,12,1,0,0\n");
        assert_eq!(editor(r"\\x"), "0,2,1,0,0\n");
        assert_eq!(editor("```\n\\*\n```"), "");
    }
    #[test]
    fn editor_ranges_remain_utf16_boundaries_for_nested_and_incomplete_modifiers() {
        for source in [
            "underline::👩🏽‍💻",
            "bold::a italic::b;",
            "bold::**a *b***;",
            "red::x blue::y;",
            "small3::",
            "mono::a `b;c`;",
            "||**a**||",
            "mark::red-blue::a;",
            "shake::bold::a;",
        ] {
            let utf16: Vec<_> = source.encode_utf16().collect();
            for line in editor(source).lines() {
                let values: Vec<usize> = line
                    .split(',')
                    .take(5)
                    .map(|n| n.parse().unwrap())
                    .collect();
                let [start, end, prefix, suffix, _] = values[..] else {
                    panic!()
                };
                assert!(
                    start + prefix < end - suffix && end <= utf16.len(),
                    "{source}: {line}"
                );
                for offset in [start, end, start + prefix, end - suffix] {
                    assert!(
                        offset == utf16.len() || !(0xdc00..=0xdfff).contains(&utf16[offset]),
                        "{source}: {line}"
                    );
                }
            }
        }
    }
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
