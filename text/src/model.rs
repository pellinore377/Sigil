use crate::Effects;
use serde::{Deserialize, Serialize};
use unicode_normalization::{is_nfc, UnicodeNormalization};
use unicode_segmentation::UnicodeSegmentation;

pub const MAX_WIRE_BYTES: usize = 60 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Limit,
    Invalid,
    Version,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Limit => "SigilText limit exceeded",
            Self::Invalid => "invalid SigilText",
            Self::Version => "unsupported SigilText version",
        })
    }
}
impl std::error::Error for Error {}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub source_bytes: usize,
    pub body_bytes: usize,
    pub spans: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            source_bytes: 32768,
            body_bytes: 32768,
            spans: 1024,
        }
    }
}
impl Limits {
    pub(crate) fn validate(self) -> Result<(), Error> {
        let hard = Self::default();
        if self.source_bytes == 0
            || self.source_bytes > hard.source_bytes
            || self.body_bytes == 0
            || self.body_bytes > hard.body_bytes
            || self.spans == 0
            || self.spans > hard.spans
        {
            return Err(Error::Limit);
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Span {
    pub start: u32,
    pub end: u32,
    pub effects: Effects,
}
#[derive(Clone, PartialEq, Eq)]
pub struct Text {
    body: String,
    spans: Vec<Span>,
    blocks: Vec<crate::Block>,
    mentions: Vec<crate::contact::Mention>,
}
impl Serialize for Text {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        crate::structured::inline::serialize(self, serializer)
    }
}
impl<'de> Deserialize<'de> for Text {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        crate::structured::inline::deserialize(deserializer)
    }
}
/// Graphical creation uses the same normalization and resolved spans as parsing.
pub struct Run<'a> {
    pub text: &'a str,
    pub effects: Effects,
}
#[derive(Serialize)]
pub struct Presentation<'a> {
    pub text: &'a str,
    pub spans: Vec<Span>,
    pub blocks: Vec<crate::Block>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub code_tokens: Vec<crate::code::Token>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub motion: Vec<crate::motion::Run>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    body: String,
    format: String,
    formatted_body: String,
    sigil: Metadata,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    version: u8,
    unicode: String,
    text_spans: Vec<Span>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    blocks: Vec<crate::Block>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    mentions: Vec<crate::contact::Mention>,
}
impl Text {
    pub fn mentions(&self) -> &[crate::contact::Mention] {
        &self.mentions
    }
    pub fn with_mentions(mut self, mentions: Vec<crate::contact::Mention>) -> Result<Self, Error> {
        crate::contact::validate_mentions(&self, &mentions)?;
        self.mentions = mentions;
        Ok(self)
    }
    pub fn bind_mention(
        self,
        range: std::ops::Range<u32>,
        contact: &crate::contact::Contact,
    ) -> Result<Self, Error> {
        contact.validate()?;
        let display: String = self
            .body
            .graphemes(true)
            .skip(range.start as usize)
            .take(range.end.saturating_sub(range.start) as usize)
            .collect();
        let mut mentions = self.mentions.clone();
        mentions.push(crate::contact::Mention {
            start: range.start,
            end: range.end,
            user_id: contact.user_id,
            display,
            address: contact.address.clone(),
        });
        mentions.sort_by_key(|mention| mention.start);
        self.with_mentions(mentions)
    }
    pub fn blocks(&self) -> &[crate::Block] {
        &self.blocks
    }
    pub fn with_blocks(mut self, blocks: Vec<crate::Block>) -> Result<Self, Error> {
        crate::blocks::validate(&self, &blocks)?;
        self.blocks = blocks;
        Ok(self)
    }
    pub fn body(&self) -> &str {
        &self.body
    }
    pub fn spans(&self) -> &[Span] {
        &self.spans
    }
    /// Platform text layouts use UTF-16; canonical ranges remain grapheme-based.
    pub fn presentation(&self) -> Presentation<'_> {
        let mut at = 0;
        let mut offsets = Vec::new();
        for g in self.body.graphemes(true) {
            offsets.push(at);
            at += g.encode_utf16().count() as u32;
        }
        offsets.push(at);
        let mut spans = self.spans.clone();
        let mut blocks = self.blocks.clone();
        let mut code_tokens = Vec::new();
        if blocks
            .iter()
            .any(|b| matches!(b.kind, crate::BlockKind::Code { language: Some(_) }))
        {
            let bytes: Vec<_> = self
                .body
                .grapheme_indices(true)
                .map(|(i, _)| i)
                .chain(std::iter::once(self.body.len()))
                .collect();
            for block in &blocks {
                if let crate::BlockKind::Code {
                    language: Some(language),
                } = &block.kind
                {
                    code_tokens.extend(crate::code::highlight(
                        &self.body[bytes[block.start as usize]..bytes[block.end as usize]],
                        language,
                        offsets[block.start as usize],
                    ));
                }
            }
        }
        for span in &mut spans {
            span.start = offsets[span.start as usize];
            span.end = offsets[span.end as usize];
        }
        for block in &mut blocks {
            block.start = offsets[block.start as usize];
            block.end = offsets[block.end as usize];
        }
        Presentation {
            text: &self.body,
            spans,
            blocks,
            code_tokens,
            motion: crate::motion::presentation(self),
        }
    }
    /// Selection indices use the same graphemes as effects, never UTF-16 units.
    pub fn redact_range(&self, range: std::ops::Range<u32>, limits: Limits) -> Result<Self, Error> {
        let offsets: Vec<_> = self
            .body
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(self.body.len()))
            .collect();
        if range.start > range.end || range.end as usize >= offsets.len() {
            return Err(Error::Invalid);
        }
        if range.is_empty() {
            limits.validate()?;
            return Self::from_runs(&self.runs(&offsets), limits)?
                .with_blocks(self.blocks.clone())?
                .with_mentions(self.mentions.clone());
        }
        let start = offsets[range.start as usize];
        let end = offsets[range.end as usize];
        let links: std::collections::BTreeSet<_> = self
            .spans
            .iter()
            .filter(|s| s.start < range.end && range.start < s.end)
            .filter_map(|s| s.effects.link.as_deref())
            .collect();
        let original = self.runs(&offsets);
        let mut result = Vec::new();
        let mut at = 0;
        let mut inserted = false;
        for mut run in original {
            if run
                .effects
                .link
                .as_deref()
                .is_some_and(|url| links.contains(url))
            {
                run.effects.link = None;
            }
            let stop = at + run.text.len();
            if stop <= start || at >= end {
                result.push(run)
            } else {
                if at < start {
                    result.push(Run {
                        text: &self.body[at..start],
                        effects: run.effects.clone(),
                    });
                }
                if !inserted {
                    result.push(Run {
                        text: "[REDACTED]",
                        effects: run.effects.clone(),
                    });
                    inserted = true;
                }
                if stop > end {
                    result.push(Run {
                        text: &self.body[end..stop],
                        effects: run.effects,
                    });
                }
            }
            at = stop;
        }
        let result = Self::from_runs(&result, limits)?;
        let delta = result.body.graphemes(true).count() as i64 - (offsets.len() - 1) as i64;
        let shift = |index: u32| -> u32 { (i64::from(index) + delta) as u32 };
        let mentions = self
            .mentions
            .iter()
            .filter_map(|mention| {
                if mention.start < range.end && range.start < mention.end {
                    return None;
                }
                let mut mention = mention.clone();
                if mention.start >= range.end {
                    mention.start = shift(mention.start);
                    mention.end = shift(mention.end);
                }
                Some(mention)
            })
            .collect();
        let mut blocks = Vec::new();
        let mut skip = false;
        for block in &self.blocks {
            if block.depth == 0 {
                skip = block.start < range.end && range.start < block.end;
            }
            if skip {
                continue;
            }
            let mut block = block.clone();
            if block.start >= range.end {
                block.start = shift(block.start);
                block.end = shift(block.end);
            }
            blocks.push(block);
        }
        result.with_blocks(blocks)?.with_mentions(mentions)
    }
    fn runs(&self, offsets: &[usize]) -> Vec<Run<'_>> {
        let mut runs = Vec::with_capacity(self.spans.len() * 2 + 1);
        let mut at = 0;
        for span in &self.spans {
            let start = offsets[span.start as usize];
            let end = offsets[span.end as usize];
            if at < start {
                runs.push(Run {
                    text: &self.body[at..start],
                    effects: Effects::default(),
                });
            }
            runs.push(Run {
                text: &self.body[start..end],
                effects: span.effects.clone(),
            });
            at = end;
        }
        if at < self.body.len() {
            runs.push(Run {
                text: &self.body[at..],
                effects: Effects::default(),
            });
        }
        runs
    }
    pub fn plain(body: &str, limits: Limits) -> Result<Self, Error> {
        Self::from_runs(
            &[Run {
                text: body,
                effects: Effects::default(),
            }],
            limits,
        )
    }
    pub(crate) fn replace_plain_prefix(
        &self,
        end: usize,
        replacement: &str,
    ) -> Result<Option<Self>, Error> {
        if !self.body.is_char_boundary(end)
            || !self.body[..end].is_ascii()
            || !self.blocks.is_empty()
            || !self.mentions.is_empty()
            || self.spans.iter().any(|s| (s.start as usize) < end)
        {
            return Ok(None);
        }
        let offsets: Vec<_> = self
            .body
            .grapheme_indices(true)
            .map(|(at, _)| at)
            .chain(std::iter::once(self.body.len()))
            .collect();
        let mut runs = self.runs(&offsets);
        let mut consumed = 0;
        for run in &mut runs {
            let skip = (end - consumed).min(run.text.len());
            consumed += skip;
            run.text = &run.text[skip..];
            if consumed == end {
                break;
            }
        }
        runs.insert(
            0,
            Run {
                text: replacement,
                effects: Effects::default(),
            },
        );
        Self::from_runs(&runs, Default::default()).map(Some)
    }
    pub fn from_runs(runs: &[Run<'_>], limits: Limits) -> Result<Self, Error> {
        limits.validate()?;
        if runs.len() > 4096 {
            return Err(Error::Limit);
        }
        let mut raw = String::new();
        let mut ends = Vec::with_capacity(runs.len());
        for run in runs {
            run.effects.validate()?;
            if raw
                .len()
                .checked_add(run.text.len())
                .is_none_or(|n| n > limits.body_bytes)
            {
                return Err(Error::Limit);
            }
            if run
                .text
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
            {
                return Err(Error::Invalid);
            }
            raw.push_str(run.text);
            ends.push(raw.len());
        }
        let mut body = String::new();
        let mut spans: Vec<Span> = Vec::new();
        let mut piece = 0;
        for (index, (start, grapheme)) in raw.grapheme_indices(true).enumerate() {
            if grapheme.len() > 4096 {
                return Err(Error::Limit);
            }
            let end = start + grapheme.len();
            let mut effects = Effects::default();
            while piece < runs.len() && ends[piece] <= start {
                piece += 1;
            }
            let mut touched = piece;
            while touched < runs.len() {
                if !runs[touched].text.is_empty() {
                    effects.overlay(&runs[touched].effects);
                }
                if ends[touched] >= end {
                    break;
                }
                touched += 1;
            }
            body.extend(grapheme.nfc());
            if body.len() > limits.body_bytes {
                return Err(Error::Limit);
            }
            if effects != Effects::default() {
                if let Some(last) = spans
                    .last_mut()
                    .filter(|s| s.end == index as u32 && s.effects == effects)
                {
                    last.end += 1;
                } else {
                    if spans.len() == limits.spans {
                        return Err(Error::Limit);
                    }
                    spans.push(Span {
                        start: index as u32,
                        end: index as u32 + 1,
                        effects,
                    });
                }
            }
        }
        Ok(Self {
            body,
            spans,
            blocks: Vec::new(),
            mentions: Vec::new(),
        })
    }
    pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        if self.body.is_empty() {
            return Err(Error::Invalid);
        }
        let bytes = serde_json::to_vec(&Wire {
            body: self.body.clone(),
            format: "text/html".into(),
            formatted_body: self.html(),
            sigil: Metadata {
                version: 1,
                unicode: "17.0.0".into(),
                text_spans: self.spans.clone(),
                blocks: self.blocks.clone(),
                mentions: self.mentions.clone(),
            },
        })
        .map_err(|_| Error::Invalid)?;
        if bytes.len() > MAX_WIRE_BYTES {
            return Err(Error::Limit);
        }
        Ok(bytes)
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_WIRE_BYTES {
            return Err(Error::Limit);
        }
        let wire: Wire = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
        if wire.sigil.version != 1 || wire.sigil.unicode != "17.0.0" {
            return Err(Error::Version);
        }
        if wire.body.is_empty() || wire.format != "text/html" {
            return Err(Error::Invalid);
        }
        let text = Self::from_parts(wire.body, wire.sigil.text_spans)?
            .with_blocks(wire.sigil.blocks)?
            .with_mentions(wire.sigil.mentions)?;
        if text.html() != wire.formatted_body || text.to_bytes()?.as_slice() != bytes {
            return Err(Error::Invalid);
        }
        Ok(text)
    }
    pub(crate) fn from_parts(body: String, spans: Vec<Span>) -> Result<Self, Error> {
        let limits = Limits::default();
        if body.len() > limits.body_bytes || spans.len() > limits.spans || !is_nfc(&body) {
            return Err(Error::Invalid);
        }
        if body
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
        {
            return Err(Error::Invalid);
        }
        if body.graphemes(true).any(|g| g.len() > 4096) {
            return Err(Error::Limit);
        }
        let graphemes = body.graphemes(true).count() as u32;
        let mut previous: Option<&Span> = None;
        for span in &spans {
            span.effects.validate()?;
            if span.start >= span.end
                || span.end > graphemes
                || span.effects == Effects::default()
                || previous.is_some_and(|p| {
                    p.end > span.start || (p.end == span.start && p.effects == span.effects)
                })
            {
                return Err(Error::Invalid);
            }
            previous = Some(span);
        }
        Ok(Self {
            body,
            spans,
            blocks: Vec::new(),
            mentions: Vec::new(),
        })
    }
    pub fn html(&self) -> String {
        if !self.blocks.is_empty() {
            return crate::blocks::html(self, &self.blocks);
        }
        format!(
            "<p>{}</p>",
            self.html_inline(0..self.body.graphemes(true).count() as u32)
        )
    }
    pub(crate) fn html_inline(&self, range: std::ops::Range<u32>) -> String {
        let mut out = String::new();
        let boundaries: Vec<usize> = self
            .body
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(self.body.len()))
            .collect();
        let mut at = boundaries[range.start as usize];
        for span in &self.spans {
            let start = span.start.max(range.start);
            let end = span.end.min(range.end);
            if start >= end {
                continue;
            }
            escaped(&mut out, &self.body[at..boundaries[start as usize]]);
            let e = &span.effects;
            let mut closing = Vec::new();
            if let Some(url) = &e.link {
                out.push_str("<a rel=\"noreferrer noopener\" href=\"");
                attribute(&mut out, url);
                out.push_str("\">");
                closing.push("</a>");
            }
            for (enabled, open, close) in [
                (e.bold, "<strong>", "</strong>"),
                (e.italic, "<em>", "</em>"),
                (e.strike, "<del>", "</del>"),
                (e.underline, "<u>", "</u>"),
                (e.mono || e.code, "<code>", "</code>"),
                (e.mark, "<mark>", "</mark>"),
            ] {
                if enabled {
                    out.push_str(open);
                    closing.push(close);
                }
            }
            escaped(
                &mut out,
                &self.body[boundaries[start as usize]..boundaries[end as usize]],
            );
            for close in closing.into_iter().rev() {
                out.push_str(close)
            }
            at = boundaries[end as usize];
        }
        escaped(&mut out, &self.body[at..boundaries[range.end as usize]]);
        out
    }
}
pub(crate) fn attribute(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
}
fn escaped(out: &mut String, text: &str) {
    for line in text.split_inclusive('\n') {
        attribute(out, line.trim_end_matches('\n'));
        if line.ends_with('\n') {
            out.push_str("<br>")
        }
    }
}
