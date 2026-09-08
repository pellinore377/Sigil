use crate::{Error, Text};
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BlockKind {
    Paragraph,
    Heading { level: u8 },
    Code { language: Option<String> },
    Quote,
    List { start: Option<u64> },
    Item,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Block {
    pub start: u32,
    pub end: u32,
    pub depth: u8,
    pub kind: BlockKind,
}

pub(crate) fn validate(text: &Text, blocks: &[Block]) -> Result<(), Error> {
    if blocks.len() > 1024 {
        return Err(Error::Limit);
    }
    let count = text.body().graphemes(true).count() as u32;
    let mut stack: Vec<&Block> = Vec::new();
    let mut previous = 0;
    for block in blocks {
        if block.start >= block.end
            || block.end > count
            || block.start < previous
            || block.depth > 31
            || usize::from(block.depth) > stack.len()
        {
            return Err(Error::Invalid);
        }
        while stack.len() > usize::from(block.depth) {
            let sibling = stack.pop().ok_or(Error::Invalid)?;
            if stack.len() == usize::from(block.depth) && sibling.end > block.start {
                return Err(Error::Invalid);
            }
        }
        if let Some(parent) = stack.last() {
            if block.end > parent.end
                || block.start < parent.start
                || !matches!(
                    parent.kind,
                    BlockKind::Quote | BlockKind::List { .. } | BlockKind::Item
                )
            {
                return Err(Error::Invalid);
            }
        }
        if matches!(block.kind, BlockKind::Item)
            != stack
                .last()
                .is_some_and(|parent| matches!(parent.kind, BlockKind::List { .. }))
        {
            return Err(Error::Invalid);
        }
        match &block.kind {
            BlockKind::Heading { level } if !(1..=6).contains(level) => return Err(Error::Invalid),
            BlockKind::Code {
                language: Some(language),
            } if language.is_empty()
                || language.len() > 32
                || !language
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') =>
            {
                return Err(Error::Invalid)
            }
            BlockKind::List { start: Some(n) } if *n > 1_000_000_000 => return Err(Error::Invalid),
            _ => (),
        }
        stack.push(block);
        previous = block.start;
    }
    Ok(())
}

pub(crate) fn html(text: &Text, blocks: &[Block]) -> String {
    let offsets: Vec<_> = text
        .body()
        .grapheme_indices(true)
        .map(|(i, _)| i)
        .chain(std::iter::once(text.body().len()))
        .collect();
    fn render(
        text: &Text,
        blocks: &[Block],
        offsets: &[usize],
        index: &mut usize,
        out: &mut String,
    ) {
        let block = &blocks[*index];
        *index += 1;
        let close = match &block.kind {
            BlockKind::Paragraph => {
                out.push_str("<p>");
                "</p>".into()
            }
            BlockKind::Heading { level } => {
                out.push_str(&format!("<h{level}>"));
                format!("</h{level}>")
            }
            BlockKind::Quote => {
                out.push_str("<blockquote>");
                "</blockquote>".into()
            }
            BlockKind::List { start: None } => {
                out.push_str("<ul>");
                "</ul>".into()
            }
            BlockKind::List { start: Some(n) } => {
                out.push_str(&format!("<ol start=\"{n}\">"));
                "</ol>".into()
            }
            BlockKind::Item => {
                out.push_str("<li>");
                "</li>".into()
            }
            BlockKind::Code { language } => {
                out.push_str("<pre><code");
                if let Some(language) = language {
                    out.push_str(&format!(" class=\"language-{language}\""));
                }
                out.push('>');
                "</code></pre>".into()
            }
        };
        let mut at = block.start;
        let gap = |out: &mut String, start: u32, end: u32| {
            if matches!(block.kind, BlockKind::List { .. }) {
                crate::model::attribute(
                    out,
                    &text.body()[offsets[start as usize]..offsets[end as usize]],
                );
            } else {
                out.push_str(&text.html_inline(start..end));
            }
        };
        let body = &text.body()[offsets[block.start as usize]..offsets[block.end as usize]];
        if (matches!(block.kind, BlockKind::Item) && body.starts_with("• "))
            || (matches!(block.kind, BlockKind::Quote) && body.starts_with("> "))
        {
            at += 2;
        }
        while *index < blocks.len() && blocks[*index].depth > block.depth {
            let child = &blocks[*index];
            if at < child.start {
                gap(out, at, child.start);
            }
            render(text, blocks, offsets, index, out);
            at = child.end;
        }
        if at < block.end {
            if matches!(block.kind, BlockKind::Code { .. }) {
                crate::model::attribute(
                    out,
                    &text.body()[offsets[at as usize]..offsets[block.end as usize]],
                );
            } else {
                gap(out, at, block.end);
            }
        }
        out.push_str(&close);
    }
    let mut out = String::new();
    let mut at = 0;
    let mut index = 0;
    while index < blocks.len() {
        let block = &blocks[index];
        if at < block.start {
            out.push_str(&text.html_inline(at..block.start));
        }
        render(text, blocks, &offsets, &mut index, &mut out);
        at = block.end;
    }
    if at as usize + 1 < offsets.len() {
        out.push_str(&text.html_inline(at..offsets.len() as u32 - 1));
    }
    out
}

pub(crate) struct Collector {
    pub marks: Vec<(BlockKind, usize, usize, u8)>,
    stack: Vec<usize>,
}
impl Collector {
    pub fn new() -> Self {
        Self {
            marks: Vec::new(),
            stack: Vec::new(),
        }
    }
    pub fn start(&mut self, kind: BlockKind, at: usize) -> Result<(), Error> {
        if self.marks.len() == 1024 || self.stack.len() == 32 {
            return Err(Error::Limit);
        }
        self.stack.push(self.marks.len());
        self.marks.push((kind, at, at, self.stack.len() as u8 - 1));
        Ok(())
    }
    pub fn end(&mut self, at: usize) -> Result<(), Error> {
        let index = self.stack.pop().ok_or(Error::Invalid)?;
        self.marks[index].2 = at;
        Ok(())
    }
    pub fn finish(self, text: &Text, raw: &str) -> Result<Vec<Block>, Error> {
        if !self.stack.is_empty() {
            return Err(Error::Invalid);
        }
        let offsets: Vec<_> = raw
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(raw.len()))
            .collect();
        let position = |at: usize| offsets.partition_point(|&i| i < at.min(raw.len())) as u32;
        let mut blocks = Vec::new();
        for (kind, start, end, depth) in self.marks {
            let start = offsets
                .partition_point(|&i| i <= start.min(raw.len()))
                .saturating_sub(1) as u32;
            let end = position(end);
            if start < end {
                blocks.push(Block {
                    start,
                    end,
                    depth,
                    kind,
                });
            }
        }
        validate(text, &blocks)?;
        Ok(blocks)
    }
}
