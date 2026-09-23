use crate::Error;
use pulldown_latex::{
    config::{DisplayMode, RenderConfig},
    Parser, Storage,
};
use std::io::{self, Write};

mod typeset;

/// Typeset glyph outlines and rules for `source`, identical on every client.
pub fn typeset(source: &str, block: bool) -> Result<serde_json::Value, Error> {
    typeset::layout(&html(source, block)?, block)
}

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
            xml: true,
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
    fn typeset_math_uses_the_font_math_table() {
        let runs = |v: &serde_json::Value| v["runs"].as_array().unwrap().clone();
        let frac = typeset(r"\frac{1}{2}", true).unwrap();
        assert_eq!(frac["rules"].as_array().unwrap().len(), 1);
        assert_eq!(runs(&frac).len(), 2);
        assert!(frac["ascent"].as_f64().unwrap() > 0.5 && frac["descent"].as_f64().unwrap() > 0.3);
        // Scripts shrink by the font's ScriptPercentScaleDown.
        let script = runs(&typeset("x^2", false).unwrap());
        assert_eq!(script[0][3], 1.0);
        assert!(script[1][3].as_f64().unwrap() < 0.8);
        // A matrix's fences grow past the text-size parenthesis.
        let paren = runs(&typeset("(x)", true).unwrap())[0][0].clone();
        let matrix = typeset(r"\begin{pmatrix}1 & 2 \\ 3 & 4\end{pmatrix}", true).unwrap();
        assert_ne!(runs(&matrix)[0][0], paren);
        assert!(matrix["ascent"].as_f64().unwrap() > 1.0);
        // Display style picks the large integral variant.
        let inline = typeset(r"\int x", false).unwrap();
        let display = typeset(r"\int x", true).unwrap();
        assert_ne!(runs(&inline)[0][0], runs(&display)[0][0]);
        assert!(display["ascent"].as_f64().unwrap() > inline["ascent"].as_f64().unwrap());
        // Radicals carry their overbar, and every glyph run has an outline.
        let root = typeset(r"\sqrt{x}", true).unwrap();
        assert_eq!(root["rules"].as_array().unwrap().len(), 1);
        let ids: Vec<_> = root["glyphs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| g[0].clone())
            .collect();
        assert!(runs(&root).iter().all(|r| ids.contains(&r[0])));
        assert_eq!(
            typeset(r"\alpha + \frac{a}{b}", true).unwrap(),
            typeset(r"\alpha + \frac{a}{b}", true).unwrap()
        );
        assert!(typeset(r"\frac{a}{", true).is_err());
        // Script the font lacks leaves the formula untypeset, never drawn as replacement boxes.
        assert!(typeset(r"\text{面积} = \pi r^2", true).is_err());
        assert!(typeset(r"\text{Площадь} = \pi r^2", true).is_ok());
        // Matrix cells stand clear of their fences.
        let bare = typeset(r"\begin{matrix}1\end{matrix}", true).unwrap();
        let one = typeset("1", true).unwrap();
        assert!(bare["runs"][0][1].as_f64().unwrap() > 0.15);
        assert!(bare["width"].as_f64().unwrap() > one["width"].as_f64().unwrap() + 0.3);
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
