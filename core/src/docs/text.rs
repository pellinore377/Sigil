//! Markdown and plain text. Markdown goes through `pulldown-cmark` and then the same
//! sanitiser the timeline uses — a sent document is as untrusted as a sent message.

use std::path::Path;

use super::{Block, Preview, MAX_TEXT_CHARS};

fn read_text(path: &Path) -> anyhow::Result<String> {
    let bytes = std::fs::read(path)?;
    let text = match std::str::from_utf8(&bytes) {
        Ok(s) => s.to_string(),
        Err(_) => encoding_rs::WINDOWS_1252.decode(&bytes).0.into_owned(),
    };
    Ok(text.chars().take(MAX_TEXT_CHARS).collect())
}

pub fn read_markdown(path: &Path) -> anyhow::Result<Preview> {
    let src = read_text(path)?;
    let mut opts = pulldown_cmark::Options::empty();
    opts.insert(pulldown_cmark::Options::ENABLE_TABLES);
    opts.insert(pulldown_cmark::Options::ENABLE_STRIKETHROUGH);
    opts.insert(pulldown_cmark::Options::ENABLE_TASKLISTS);
    opts.insert(pulldown_cmark::Options::ENABLE_FOOTNOTES);
    let parser = pulldown_cmark::Parser::new_ext(&src, opts);
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, parser);
    // Same treatment as a remote formatted_body.
    let safe = crate::timeline::html::to_rich_text(&html);
    let mut out = Preview {
        kind: "markdown".into(),
        html: safe,
        pages: 1,
        truncated: src.chars().count() >= MAX_TEXT_CHARS,
        ..Default::default()
    };
    // A bubble lays out lines, not markup: walk the parser again rather than the HTML.
    markdown_blocks(&src, opts, &mut out);
    Ok(out)
}

/// Markdown as flat blocks, for anything that wants lines rather than markup.
fn markdown_blocks(src: &str, opts: pulldown_cmark::Options, out: &mut Preview) {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};
    let mut level = 0u8;
    let mut bullet = false;
    let mut buf = String::new();
    for ev in Parser::new_ext(src, opts) {
        match ev {
            Event::Start(Tag::Heading { level: l, .. }) => level = l as u8,
            Event::Start(Tag::Item) => bullet = true,
            Event::Text(t) => buf.push_str(&t),
            Event::Code(t) => buf.push_str(&t),
            Event::SoftBreak | Event::HardBreak => buf.push(' '),
            Event::End(TagEnd::Heading(_)) | Event::End(TagEnd::Paragraph) | Event::End(TagEnd::Item) => {
                let text = buf.trim().to_string();
                buf.clear();
                if !text.is_empty() && !out.push(Block::Para { text, level, bullet }) { return }
                level = 0;
                bullet = false;
            }
            _ => {}
        }
    }
    let text = buf.trim().to_string();
    if !text.is_empty() { out.push(Block::Para { text, level, bullet }); }
}

pub fn read_plain(path: &Path) -> anyhow::Result<Preview> {
    let src = read_text(path)?;
    let mut out = Preview { kind: "text".into(), pages: 1, ..Default::default() };
    out.truncated = src.chars().count() >= MAX_TEXT_CHARS;
    for line in src.lines() {
        if !out.push(Block::Para { text: line.to_string(), level: 0, bullet: false }) { break }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str, body: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("sigil-text-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn markdown_renders_headings_lists_and_tables() {
        let p = tmp("a.md", b"# Title\n\n- one\n- two\n\n| a | b |\n|---|---|\n| 1 | 2 |\n");
        let out = read_markdown(&p).unwrap();
        assert_eq!(out.kind, "markdown");
        assert!(out.html.contains("Title"));
        assert!(out.html.contains("<li>"), "lists survive: {}", out.html);
        assert!(out.html.contains("<table"), "tables survive: {}", out.html);
    }

    #[test]
    fn markdown_cannot_smuggle_script_through_raw_html() {
        // CommonMark passes raw HTML through; the sanitiser is what stops it.
        let p = tmp("x.md", b"Hello <script>alert(1)</script> <img src=x onerror=y>\n");
        let out = read_markdown(&p).unwrap();
        assert!(!out.html.contains("<script"), "script stripped: {}", out.html);
        assert!(!out.html.to_ascii_lowercase().contains("onerror"), "handler stripped: {}", out.html);
        assert!(out.html.contains("Hello"));
    }

    #[test]
    fn plain_text_becomes_one_block_per_line() {
        let p = tmp("a.txt", b"first\nsecond\n\nfourth\n");
        let out = read_plain(&p).unwrap();
        assert_eq!(out.kind, "text");
        assert_eq!(out.blocks.len(), 4);
        assert_eq!(out.blocks[0], Block::Para { text: "first".into(), level: 0, bullet: false });
        assert_eq!(out.blocks[2], Block::Para { text: String::new(), level: 0, bullet: false });
    }
}
