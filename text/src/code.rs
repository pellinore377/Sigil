use serde::Serialize;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Token {
    pub start: u32,
    pub end: u32,
    pub role: &'static str,
}

pub(crate) fn highlight(source: &str, language: &str, base: u32) -> Vec<Token> {
    let language = language.to_ascii_lowercase();
    let words = match language.as_str() {
        "json" => "true false null",
        "rust" | "rs" => {
            "as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait true type unsafe use where while bool char str u8 u16 u32 u64 u128 usize i8 i16 i32 i64 i128 isize f32 f64"
        }
        "kotlin" | "kt" => {
            "as break class continue do else false for fun if in interface is null object package return super this throw true try typealias typeof val var when while by catch constructor delegate dynamic field file finally get import init param property receiver set setparam where actual abstract annotation companion const crossinline data enum expect external final infix inline inner internal lateinit noinline open operator out override private protected public reified sealed suspend tailrec vararg"
        }
        "c" | "cpp" | "c++" | "h" | "hpp" => {
            "alignas alignof asm auto bool break case catch char class const constexpr consteval constinit continue default delete do double else enum explicit export extern false float for friend goto if inline int long mutable namespace new noexcept nullptr operator private protected public register reinterpret_cast return short signed sizeof static static_assert static_cast struct switch template this thread_local throw true try typedef typeid typename union unsigned using virtual void volatile wchar_t while"
        }
        _ => return Vec::new(),
    };
    let rust = matches!(language.as_str(), "rust" | "rs");
    let json = language == "json";
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    let mut at = base;
    while i < bytes.len() {
        let start = i;
        let mut role = None;
        if !json && source[i..].starts_with("//") {
            i = source[i..].find('\n').map_or(bytes.len(), |n| i + n);
            role = Some("comment");
        } else if !json && source[i..].starts_with("/*") {
            i += 2;
            let mut depth = 1;
            while i < bytes.len() && depth > 0 {
                if bytes[i..].starts_with(b"*/") {
                    depth -= 1;
                    i += 2;
                } else if (rust || matches!(language.as_str(), "kotlin" | "kt"))
                    && bytes[i..].starts_with(b"/*")
                {
                    depth += 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            role = Some("comment");
        } else if rust
            && bytes[i] == b'r'
            && bytes.get(i + 1).is_some_and(|b| *b == b'#' || *b == b'"')
        {
            let hashes = bytes[i + 1..].iter().take_while(|&&b| b == b'#').count();
            if bytes.get(i + 1 + hashes) == Some(&b'"') {
                i += 2 + hashes;
                while i < bytes.len() {
                    if bytes[i] == b'"'
                        && bytes
                            .get(i + 1..i + 1 + hashes)
                            .is_some_and(|v| v.iter().all(|&b| b == b'#'))
                    {
                        i += 1 + hashes;
                        break;
                    }
                    i += 1;
                }
                role = Some("string");
            } else {
                i += 1;
            }
        } else if bytes[i] == b'"'
            || !json && bytes[i] == b'\'' && (!rust || char_literal(&source[i..]))
        {
            let quote = bytes[i];
            let triple = quote == b'"'
                && matches!(language.as_str(), "kotlin" | "kt")
                && bytes[i..].starts_with(b"\"\"\"");
            i += if triple { 3 } else { 1 };
            while i < bytes.len() {
                if triple {
                    if bytes[i..].starts_with(b"\"\"\"") {
                        i += 3;
                        break;
                    }
                    i += 1;
                } else if bytes[i] == b'\\' {
                    i = (i + 2).min(bytes.len());
                } else if bytes[i] == quote {
                    i += 1;
                    break;
                } else if bytes[i] == b'\n' {
                    break;
                } else {
                    i += 1;
                }
            }
            role = Some("string");
        } else if bytes[i].is_ascii_digit() {
            i += 1;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric()
                    || matches!(bytes[i], b'.' | b'_')
                    || matches!(bytes[i], b'+' | b'-')
                        && matches!(bytes[i - 1], b'e' | b'E' | b'p' | b'P'))
            {
                i += 1;
            }
            role = Some("number");
        } else {
            let ch = source[i..].chars().next().unwrap();
            i += ch.len_utf8();
            if ch.is_alphabetic() || ch == '_' {
                while i < bytes.len() {
                    let ch = source[i..].chars().next().unwrap();
                    if !ch.is_alphanumeric() && ch != '_' {
                        break;
                    }
                    i += ch.len_utf8();
                }
                if words
                    .split_ascii_whitespace()
                    .any(|word| word == &source[start..i])
                {
                    role = Some("keyword");
                }
            }
        }
        while !source.is_char_boundary(i) {
            i += 1;
        }
        let end = at + source[start..i].encode_utf16().count() as u32;
        if let Some(role) = role {
            tokens.push(Token {
                start: at,
                end,
                role,
            });
        }
        at = end;
    }
    tokens
}
fn char_literal(source: &str) -> bool {
    let rest = &source[1..];
    rest.starts_with('\\')
        || rest
            .chars()
            .next()
            .is_some_and(|ch| rest.as_bytes().get(ch.len_utf8()) == Some(&b'\''))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn presentation_tokens_use_utf16_and_do_not_enter_the_canonical_message() {
        let text =
            crate::parse("👋\n\n```rust\nlet code = \"👩🏽‍💻\";\n```", Default::default()).unwrap();
        let bytes = text.to_bytes().unwrap();
        let view = text.presentation();
        let units: Vec<_> = view.text.encode_utf16().collect();
        assert_eq!(
            String::from_utf16(
                &units[view.code_tokens[0].start as usize..view.code_tokens[0].end as usize]
            )
            .unwrap(),
            "let"
        );
        assert_eq!(view.code_tokens[1].role, "string");
        assert_eq!(
            String::from_utf16(
                &units[view.code_tokens[1].start as usize..view.code_tokens[1].end as usize]
            )
            .unwrap(),
            "\"👩🏽‍💻\""
        );
        assert!(!std::str::from_utf8(&bytes).unwrap().contains("code_tokens"));
        assert_eq!(
            crate::Text::from_bytes(&bytes).unwrap().to_bytes().unwrap(),
            bytes
        );
    }
    #[test]
    fn highlighting_preserves_unicode_literals_lifetimes_and_unknown_languages() {
        let source = "let glyph = r##\"👩🏽‍💻 // literal\"##; /* outer /* inner */ end */ &'static str";
        let tokens = highlight(source, "rust", 4);
        let utf16: Vec<_> = source.encode_utf16().collect();
        let text = |token: &Token| {
            String::from_utf16(&utf16[(token.start - 4) as usize..(token.end - 4) as usize])
                .unwrap()
        };
        assert_eq!(text(&tokens[0]), "let");
        assert_eq!(text(&tokens[1]), "r##\"👩🏽‍💻 // literal\"##");
        assert_eq!(text(&tokens[2]), "/* outer /* inner */ end */");
        assert_eq!(tokens.iter().filter(|t| t.role == "string").count(), 1);
        assert!(highlight(source, "unknown", 0).is_empty());
        for input in [
            "\"escaped\\👋\"",
            "'👋'",
            "\"unterminated\\",
            "r#identifier",
        ] {
            highlight(input, "rust", 0);
        }
        assert_eq!(highlight("true false null", "json", 0).len(), 3);
    }
}
