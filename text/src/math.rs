use crate::Error;
use pulldown_latex::{
    config::{DisplayMode, RenderConfig},
    Parser, Storage,
};
use std::io::{self, Write};

/// Bounded, macro-free LaTeX rendered without source annotations or error markup.
pub fn html(source: &str, block: bool) -> Result<String, Error> {
    if source.trim().is_empty()
        || source.len() > 8192
        || source
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(Error::Invalid);
    }
    let mut chars = source.chars().peekable();
    let mut depth = 0u8;
    while let Some(c) = chars.next() {
        match c {
            '%' => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '\\' => {
                let command: String = std::iter::from_fn(|| {
                    chars
                        .peek()
                        .is_some_and(char::is_ascii_alphabetic)
                        .then(|| chars.next().unwrap())
                })
                .collect();
                if command.is_empty() {
                    chars.next();
                }
                if matches!(
                    command.as_str(),
                    "def" | "let" | "futurelet" | "newcommand" | "renewcommand" | "providecommand"
                ) {
                    return Err(Error::Invalid);
                }
            }
            '{' => {
                depth += 1;
                if depth > 32 {
                    return Err(Error::Limit);
                }
            }
            '}' => {
                depth = depth.checked_sub(1).ok_or(Error::Invalid)?;
            }
            _ => (),
        }
    }
    if depth != 0 {
        return Err(Error::Invalid);
    }
    let storage = Storage::new();
    let mut events = Vec::new();
    for event in Parser::new(source, &storage) {
        if events.len() == 8192 {
            return Err(Error::Limit);
        }
        events.push(event.map_err(|_| Error::Invalid)?);
    }
    let mut output = Bounded(Vec::new());
    pulldown_latex::mathml::write_mathml(
        &mut output,
        events.into_iter().map(Ok::<_, io::Error>),
        RenderConfig {
            display_mode: if block {
                DisplayMode::Block
            } else {
                DisplayMode::Inline
            },
            ..Default::default()
        },
    )
    .map_err(|_| Error::Limit)?;
    String::from_utf8(output.0).map_err(|_| Error::Invalid)
}

struct Bounded(Vec<u8>);
impl Write for Bounded {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if data.len() > crate::MAX_WIRE_BYTES.saturating_sub(self.0.len()) {
            return Err(io::Error::other("math output limit"));
        }
        self.0.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn valid_math_has_structure_and_untrusted_text_is_escaped() {
        let output = html(r"\frac{x^2}{\sqrt{y}}", true).unwrap();
        assert!(output.contains("<mfrac>"));
        assert!(output.contains("<msqrt>"));
        let output = html(r"\text{<script>alert(1)</script>}", false).unwrap();
        assert!(!output.contains("<script>"));
        assert!(output.contains("&lt;script&gt;"));
        assert!(!output.contains("annotation"));
        assert!(html(r"\{x\}", false).is_ok());
    }
    #[test]
    fn malformed_math_and_macro_expansion_are_rejected() {
        for value in [
            r"\frac{a}{",
            r"\unknown",
            r"\def\x{\x}\x",
            r"\let\x=\def",
            r"\futurelet\x\def",
            r"\newcommand{\x}{x}",
            r"\renewcommand{\x}{x}",
            r"\providecommand{\x}{x}",
            r"\href{https://example.org}{x}",
        ] {
            assert!(html(value, false).is_err(), "{value}");
        }
        assert!(html(&format!("{}x{}", "{".repeat(33), "}".repeat(33)), false).is_err());
        assert!(html(&"x".repeat(8193), false).is_err());
    }
}
