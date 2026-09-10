use crate::{effects::valid_link, model::Run, Effects, Error, Limits, Text};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::ops::Range;

struct SourceSpan {
    range: Range<usize>,
    effects: Effects,
}
/// Byte ranges in the unmodified authoring source, including delimiters.
pub struct EditorSpan {
    pub start: usize,
    pub end: usize,
    pub prefix: usize,
    pub suffix: usize,
    pub effects: Effects,
}

pub fn editor_spans(source: &str) -> Result<Vec<EditorSpan>, Error> {
    if source.len() > Limits::default().source_bytes { return Err(Error::Limit); }
    let syntax = syntax(source)?;
    let delimiter = |at| syntax.edits.get(syntax.edits.partition_point(|edit| edit.range.start < at))
        .filter(|edit| edit.range.start == at && edit.replacement.is_empty())
        .map_or(0, |edit| edit.range.end - edit.range.start);
    let mut result = Vec::new();
    for span in &syntax.spans {
        if syntax.redacted(&span.range) { continue; }
        let prefix = delimiter(span.range.start);
        let suffix = delimiter(span.range.end);
        if span.range.start + prefix >= span.range.end { continue; }
        result.push(EditorSpan { start:span.range.start, end:span.range.end+suffix, prefix, suffix, effects:span.effects.clone() });
    }
    Ok(result)
}
struct Edit {
    range: Range<usize>,
    replacement: &'static str,
}
struct Syntax<'a> {
    source: &'a str,
    eligible: Vec<bool>,
    code: Vec<bool>,
    spans: Vec<SourceSpan>,
    edits: Vec<Edit>,
}
fn options() -> Options {
    Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS
}
fn escaped(source: &str, index: usize) -> bool {
    source.as_bytes()[..index]
        .iter()
        .rev()
        .take_while(|&&b| b == b'\\')
        .count()
        % 2
        == 1
}
fn chain(
    source: &str,
    start: usize,
    end: usize,
    eligible: &[bool],
) -> Option<(usize, Effects, bool)> {
    let mut at = start;
    let mut effects = Effects::default();
    let mut redact = false;
    loop {
        let tail = &source[at..end];
        let count = tail
            .bytes()
            .take_while(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
            .take(81)
            .count();
        if count == 0
            || count > 80
            || !tail[count..].starts_with("::")
            || !(at..at + count + 2).all(|i| eligible[i])
        {
            break;
        }
        let token = &tail[..count];
        if token == "redact" {
            redact = true;
        } else if !effects.modifier(token) {
            break;
        }
        at += count + 2;
    }
    (at > start).then_some((at, effects, redact))
}
impl Syntax<'_> {
    fn edit(&mut self, range: Range<usize>, replacement: &'static str) -> Result<(), Error> {
        if self.edits.len() == 4096 {
            return Err(Error::Limit);
        }
        self.edits.push(Edit { range, replacement });
        Ok(())
    }
    fn redacted(&self, range: &Range<usize>) -> bool {
        self.edits
            .iter()
            .any(|e| !e.replacement.is_empty() && overlap(&e.range, range))
    }
    fn scan(&mut self, start: usize, end: usize, depth: usize) -> Result<(), Error> {
        if depth > 32 {
            return Err(Error::Limit);
        }
        let mut at = start;
        while at < end {
            let byte = self.source.as_bytes()[at];
            if self.source[at..end].starts_with("||")
                && self.eligible[at]
                && !escaped(self.source, at)
            {
                let line_end = self.source[at + 2..end]
                    .find('\n')
                    .map_or(end, |n| at + 2 + n);
                let close = (at + 2..line_end.saturating_sub(1)).find(|&i| {
                    self.source.as_bytes()[i..].starts_with(b"||")
                        && !escaped(self.source, i)
                        && self.eligible[i]
                        && self.eligible[i + 1]
                        && !self.code[i]
                });
                if let Some(close) = close {
                    self.edit(at..at + 2, "")?;
                    self.scan(at + 2, close, depth + 1)?;
                    self.edit(close..close + 2, "")?;
                    if at + 2 < close {
                        if self.spans.len() == 1024 {
                            return Err(Error::Limit);
                        }
                        self.spans.push(SourceSpan {
                            range: at..close,
                            effects: Effects {
                                reveal: Some(crate::Reveal::Spoiler),
                                ..Default::default()
                            },
                        });
                    }
                    at = close + 2;
                    continue;
                }
            }
            if byte == b'\\' && !escaped(self.source, at) && self.eligible[at] {
                if self.source.as_bytes().get(at + 1) == Some(&b'\\') {
                    at += 2;
                    continue;
                }
                if let Some((prefix, _, _)) = chain(self.source, at + 1, end, &self.eligible) {
                    self.edit(at..at + 1, "")?;
                    at = prefix;
                    continue;
                }
            }
            let boundary = at == 0
                || !matches!(self.source.as_bytes()[at-1],b'a'..=b'z'|b'A'..=b'Z'|b'0'..=b'9'|b'_'|b':'|b'-');
            let segment = at >= 2 && self.source.as_bytes()[at - 2..at] == *b"::";
            if (boundary || segment) && self.eligible[at] && !escaped(self.source, at) {
                if let Some((content, effects, redact)) =
                    chain(self.source, at, end, &self.eligible)
                {
                    if !boundary && !redact {
                        at = content;
                        continue;
                    }
                    let stop = (content..end)
                        .find(|&i| {
                            self.source.as_bytes()[i] == b'\n'
                                || (self.source.as_bytes()[i] == b';'
                                    && self.eligible[i]
                                    && !escaped(self.source, i)
                                    && !self.code[i])
                        })
                        .unwrap_or(end);
                    let after =
                        stop + usize::from(stop < end && self.source.as_bytes()[stop] == b';');
                    if redact {
                        self.edit(at..after, if content < stop { "[REDACTED]" } else { "" })?;
                    } else {
                        self.edit(at..content, "")?;
                        self.scan(content, stop, depth + 1)?;
                        if after > stop {
                            self.edit(stop..after, "")?;
                        }
                    }
                    if content < stop && effects != Effects::default() {
                        if self.spans.len() == 1024 {
                            return Err(Error::Limit);
                        }
                        // Outer styles resolve first; nested content can override them.
                        self.spans.push(SourceSpan {
                            range: at..stop,
                            effects,
                        });
                    }
                    at = after;
                    continue;
                }
            }
            let ch = self.source[at..].chars().next().ok_or(Error::Invalid)?;
            at += ch.len_utf8();
        }
        Ok(())
    }
}
fn syntax(source: &str) -> Result<Syntax<'_>, Error> {
    let mut result = Syntax {
        source,
        eligible: vec![false; source.len() + 1],
        code: vec![false; source.len() + 1],
        spans: Vec::new(),
        edits: Vec::new(),
    };
    let mut code = false;
    let mut depth = 0usize;
    for (index, (event, range)) in Parser::new_ext(source, options())
        .into_offset_iter()
        .enumerate()
    {
        if index > 32768 {
            return Err(Error::Limit);
        }
        match event {
            Event::Start(tag) => {
                depth += 1;
                if depth > 32 {
                    return Err(Error::Limit);
                }
                if matches!(tag, Tag::CodeBlock(_)) {
                    code = true;
                    result.code[range].fill(true)
                }
            }
            Event::End(tag) => {
                depth = depth.checked_sub(1).ok_or(Error::Invalid)?;
                if tag == TagEnd::CodeBlock {
                    code = false
                }
            }
            Event::Text(text) if !code && &source[range.clone()] == text.as_ref() => {
                result.eligible[range].fill(true)
            }
            Event::Code(_) => result.code[range].fill(true),
            _ => {}
        }
    }
    result.scan(0, source.len(), 0)?;
    result.edits.sort_by_key(|e| e.range.start);
    result.spans.sort_by(|a, b| {
        a.range
            .start
            .cmp(&b.range.start)
            .then(b.range.end.cmp(&a.range.end))
    });
    Ok(result)
}
fn overlap(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}
pub(crate) fn construct_positions(source: &str) -> Result<(Vec<bool>, Vec<bool>), Error> {
    let syntax = syntax(source)?;
    let ordinary: Vec<_> = (0..source.len())
        .map(|at| syntax.eligible[at] && !syntax.code[at] && !escaped(source, at))
        .collect();
    let openers = (0..source.len())
        .map(|at| ordinary[at] && !syntax.redacted(&(at..at + 1)))
        .collect();
    Ok((openers, ordinary))
}
struct Piece {
    text: String,
    effects: Effects,
}
fn push(pieces: &mut Vec<Piece>, text: &str, effects: Effects) -> Result<(), Error> {
    if text.is_empty() {
        return Ok(());
    }
    if let Some(last) = pieces.last_mut().filter(|p| p.effects == effects) {
        last.text.push_str(text)
    } else {
        if pieces.len() == 4096 {
            return Err(Error::Limit);
        }
        pieces.push(Piece {
            text: text.into(),
            effects,
        });
    }
    Ok(())
}
fn line(pieces: &mut Vec<Piece>) -> Result<(), Error> {
    if pieces.last().is_some_and(|p| !p.text.ends_with('\n')) {
        push(pieces, "\n", Effects::default())?;
    }
    Ok(())
}
pub fn parse(source: &str, limits: Limits) -> Result<Text, Error> {
    limits.validate()?;
    if source.len() > limits.source_bytes {
        return Err(Error::Limit);
    }
    let source = source.replace("\r\n", "\n").replace('\r', "\n");
    if source
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
    {
        return Err(Error::Invalid);
    }
    let syntax = syntax(&source)?;
    let mut pieces = Vec::new();
    let mut stack = Vec::<Effects>::new();
    let mut current = Effects::default();
    let mut code = false;
    let mut blocks = crate::blocks::Collector::new();
    for (index, (event, range)) in Parser::new_ext(&source, options())
        .into_offset_iter()
        .enumerate()
    {
        if index > 32768 {
            return Err(Error::Limit);
        }
        match event {
            Event::Start(tag) => {
                if stack.len() == 32 {
                    return Err(Error::Limit);
                }
                stack.push(current.clone());
                let kind = match &tag {
                    Tag::Paragraph => Some(crate::BlockKind::Paragraph),
                    Tag::Heading { level, .. } => Some(crate::BlockKind::Heading {
                        level: *level as u8,
                    }),
                    Tag::BlockQuote(_) => Some(crate::BlockKind::Quote),
                    Tag::Item => Some(crate::BlockKind::Item),
                    Tag::List(start) => Some(crate::BlockKind::List { start: *start }),
                    Tag::CodeBlock(kind) => {
                        let language = match kind {
                            pulldown_cmark::CodeBlockKind::Fenced(info) => info
                                .split_whitespace()
                                .next()
                                .filter(|s| {
                                    s.len() <= 32
                                        && s.bytes().all(|b| {
                                            b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
                                        })
                                })
                                .map(str::to_owned),
                            _ => None,
                        };
                        Some(crate::BlockKind::Code { language })
                    }
                    _ => None,
                };
                if let Some(kind) = kind {
                    if matches!(tag, Tag::CodeBlock(_) | Tag::BlockQuote(_) | Tag::Item) {
                        line(&mut pieces)?;
                    }
                    blocks.start(kind, pieces.iter().map(|p| p.text.len()).sum())?;
                }
                match tag {
                    Tag::Strong => current.bold = true,
                    Tag::Emphasis => current.italic = true,
                    Tag::Strikethrough => current.strike = true,
                    Tag::CodeBlock(_) => {
                        line(&mut pieces)?;
                        code = true;
                        current.code = true
                    }
                    Tag::Link { dest_url, .. }
                        if valid_link(&dest_url) && !syntax.redacted(&range) =>
                    {
                        current.link = Some(dest_url.into_string())
                    }
                    Tag::Item => {
                        line(&mut pieces)?;
                        push(&mut pieces, "• ", Effects::default())?;
                    }
                    Tag::BlockQuote(_) => {
                        line(&mut pieces)?;
                        push(&mut pieces, "> ", Effects::default())?;
                    }
                    _ => {}
                }
            }
            Event::End(tag) => {
                current = stack.pop().ok_or(Error::Invalid)?;
                if matches!(
                    tag,
                    TagEnd::Paragraph
                        | TagEnd::Heading(_)
                        | TagEnd::CodeBlock
                        | TagEnd::Item
                        | TagEnd::List(_)
                        | TagEnd::BlockQuote(_)
                ) {
                    blocks.end(pieces.iter().map(|p| p.text.len()).sum())?;
                }
                if tag == TagEnd::CodeBlock {
                    code = false;
                }
                if matches!(
                    tag,
                    TagEnd::Paragraph
                        | TagEnd::Heading(_)
                        | TagEnd::CodeBlock
                        | TagEnd::Item
                        | TagEnd::BlockQuote(_)
                ) {
                    line(&mut pieces)?;
                }
            }
            Event::Text(text) if !code => {
                let raw = &source[range.clone()];
                let mut boundaries = vec![range.start, range.end];
                if raw == text.as_ref() {
                    for span in &syntax.spans {
                        if overlap(&span.range, &range) {
                            boundaries.push(span.range.start.max(range.start));
                            boundaries.push(span.range.end.min(range.end));
                        }
                    }
                    for edit in &syntax.edits {
                        if overlap(&edit.range, &range) {
                            boundaries.push(edit.range.start.max(range.start));
                            boundaries.push(edit.range.end.min(range.end));
                        }
                    }
                }
                boundaries.sort_unstable();
                boundaries.dedup();
                for pair in boundaries.windows(2) {
                    let part = pair[0]..pair[1];
                    let mut effects = current.clone();
                    for span in &syntax.spans {
                        if overlap(&span.range, &part) {
                            effects.overlay(&span.effects)
                        }
                    }
                    let value = if let Some(edit) =
                        syntax.edits.iter().find(|e| overlap(&e.range, &part))
                    {
                        if part.start == edit.range.start {
                            edit.replacement
                        } else {
                            ""
                        }
                    } else if raw == text.as_ref() {
                        &source[part]
                    } else {
                        text.as_ref()
                    };
                    push(&mut pieces, value, effects)?;
                }
            }
            Event::Text(text) if !syntax.redacted(&range) => {
                push(&mut pieces, &text, current.clone())?
            }
            Event::Code(text) => {
                if syntax.redacted(&range) {
                    continue;
                }
                let mut e = current.clone();
                e.code = true;
                push(&mut pieces, &text, e)?;
            }
            Event::Html(text) | Event::InlineHtml(text) => {
                if syntax.redacted(&range) {
                    continue;
                }
                push(&mut pieces, &text, current.clone())?
            }
            Event::SoftBreak | Event::HardBreak => push(&mut pieces, "\n", Effects::default())?,
            Event::Rule => {
                line(&mut pieces)?;
                push(&mut pieces, "—\n", Effects::default())?;
            }
            Event::TaskListMarker(checked) => push(
                &mut pieces,
                if checked { "[x] " } else { "[ ] " },
                Effects::default(),
            )?,
            Event::Text(_) => {}
            _ => return Err(Error::Invalid),
        }
    }
    if let Some(last) = pieces
        .last_mut()
        .filter(|p| p.effects == Effects::default() && p.text.ends_with('\n'))
    {
        last.text.pop();
    }
    let runs: Vec<_> = pieces
        .iter()
        .map(|p| Run {
            text: &p.text,
            effects: p.effects.clone(),
        })
        .collect();
    let text = Text::from_runs(&runs, limits)?;
    let raw = pieces.iter().map(|p| p.text.as_str()).collect::<String>();
    let mut blocks = blocks.finish(&text, &raw)?;
    if blocks.len() == 1 && matches!(blocks[0].kind, crate::BlockKind::Paragraph) {
        blocks.clear();
    }
    text.with_blocks(blocks)
}
