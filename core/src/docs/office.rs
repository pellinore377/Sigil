//! DOCX, PPTX, ODT and ODP: zip archives of XML, all read the same way — pull the body
//! part, stream it with `quick-xml`, turn text-bearing elements into blocks. Headings and
//! bullets are kept because they change meaning. Zip entries are read by name and capped.

use std::io::Read;
use std::path::Path;

use quick_xml::events::Event;
use quick_xml::Reader;

use super::{Block, Preview, MAX_CELL_CHARS, MAX_TEXT_CHARS};

/// A single part, read by exact name, with a ceiling on what it may inflate to.
fn part(path: &Path, name: &str) -> anyhow::Result<Option<String>> {
    let file = std::fs::File::open(path)?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file))?;
    let Ok(mut entry) = zip.by_name(name) else { return Ok(None) };
    let mut buf = String::new();
    let mut limited = entry.by_ref().take(MAX_TEXT_CHARS as u64);
    limited.read_to_string(&mut buf)?;
    Ok(Some(buf))
}

/// Numbered part names, in order (PPTX slides, ODP pages).
fn slide_parts(path: &Path, prefix: &str, suffix: &str) -> anyhow::Result<Vec<String>> {
    let file = std::fs::File::open(path)?;
    let zip = zip::ZipArchive::new(std::io::BufReader::new(file))?;
    let mut names: Vec<String> = zip
        .file_names()
        .filter(|n| n.starts_with(prefix) && n.ends_with(suffix))
        .map(str::to_string)
        .collect();
    // slide2 must not sort after slide10.
    names.sort_by_key(|n| {
        let digits: String = n.chars().filter(|c| c.is_ascii_digit()).collect();
        (digits.parse::<u64>().unwrap_or(0), n.clone())
    });
    Ok(names)
}

fn clip(s: &str) -> String {
    let t = s.trim();
    if t.chars().count() <= MAX_CELL_CHARS { return t.to_string() }
    t.chars().take(MAX_CELL_CHARS).collect::<String>() + "…"
}

/// Strip the namespace prefix: `w:p` and `text:p` are the same element here.
fn local_name(raw: &str) -> String {
    raw.rsplit(':').next().unwrap_or("").to_ascii_lowercase()
}

/// The shared walker: `para` ends a paragraph, `text` holds a run, `on_start` sees every element.
fn walk<F>(xml: &str, para: &[&str], text: &[&str], mut on_start: F) -> Vec<Block>
where
    F: FnMut(&str, &quick_xml::events::BytesStart) -> Option<Block>,
{
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut blocks: Vec<Block> = Vec::new();
    let mut buf_text = String::new();
    let mut in_text = 0usize;
    let mut pending: Option<Block> = None;
    let mut heading: u8 = 0;
    let mut bullet = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref());
                if text.contains(&name.as_str()) { in_text += 1 }
                if let Some(b) = on_start(&name, &e) {
                    match b {
                        Block::Para { level, bullet: bl, .. } => { heading = level; bullet = bl }
                        other => pending = Some(other),
                    }
                }
            }
            Ok(Event::Empty(e)) => {
                let name = local_name(e.name().as_ref());
                if let Some(b) = on_start(&name, &e) {
                    match b {
                        Block::Para { level, bullet: bl, .. } => { heading = level; bullet = bl }
                        other => pending = Some(other),
                    }
                }
                // Word writes breaks and tabs as empty elements.
                if name == "tab" { buf_text.push('\t') }
                if name == "br" || name == "cr" { buf_text.push('\n') }
            }
            Ok(Event::Text(t)) => {
                if in_text > 0 {
                    buf_text.push_str(t.as_ref())
                }
            }
            Ok(Event::End(e)) => {
                let name = local_name(e.name().as_ref());
                if text.contains(&name.as_str()) { in_text = in_text.saturating_sub(1) }
                if para.contains(&name.as_str()) {
                    if let Some(b) = pending.take() { blocks.push(b) }
                    let line = buf_text.trim().to_string();
                    buf_text.clear();
                    if !line.is_empty() {
                        blocks.push(Block::Para { text: clip(&line), level: heading, bullet });
                    }
                    heading = 0;
                    bullet = false;
                    if blocks.len() >= super::MAX_BLOCKS { break }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    if let Some(b) = pending.take() { blocks.push(b) }
    let line = buf_text.trim().to_string();
    if !line.is_empty() { blocks.push(Block::Para { text: clip(&line), level: 0, bullet: false }) }
    blocks
}

fn attr<'a>(e: &'a quick_xml::events::BytesStart, want: &str) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        let key = local_name(a.key.as_ref());
        if key == want { Some(a.value.as_ref().to_string()) } else { None }
    })
}

pub fn read_docx(path: &Path) -> anyhow::Result<Preview> {
    let Some(xml) = part(path, "word/document.xml")? else {
        anyhow::bail!("this .docx has no document body")
    };
    // `w:pStyle val="Heading2"` marks a heading; `w:numPr` marks a list item.
    let blocks = walk(&xml, &["p"], &["t"], |name, e| match name {
        "pstyle" => {
            let v = attr(e, "val").unwrap_or_default().to_ascii_lowercase();
            let level = v.strip_prefix("heading").and_then(|n| n.trim().parse::<u8>().ok()).unwrap_or(0);
            if v == "title" { Some(Block::Para { text: String::new(), level: 1, bullet: false }) }
            else if level > 0 { Some(Block::Para { text: String::new(), level: level.min(6), bullet: false }) }
            else { None }
        }
        "numpr" => Some(Block::Para { text: String::new(), level: 0, bullet: true }),
        _ => None,
    });
    Ok(Preview { kind: "document".into(), truncated: blocks.len() >= super::MAX_BLOCKS, blocks, ..Default::default() })
}

pub fn read_odt(path: &Path) -> anyhow::Result<Preview> {
    let Some(xml) = part(path, "content.xml")? else {
        anyhow::bail!("this .odt has no content")
    };
    // ODF puts the style on the paragraph and uses `text:h` with an outline-level attribute.
    let blocks = walk(&xml, &["p", "h"], &["p", "h", "span", "a"], |name, e| match name {
        "h" => {
            let level = attr(e, "outline-level").and_then(|v| v.parse::<u8>().ok()).unwrap_or(1);
            Some(Block::Para { text: String::new(), level: level.clamp(1, 6), bullet: false })
        }
        "list-item" => Some(Block::Para { text: String::new(), level: 0, bullet: true }),
        _ => None,
    });
    Ok(Preview { kind: "document".into(), truncated: blocks.len() >= super::MAX_BLOCKS, blocks, ..Default::default() })
}

pub fn read_pptx(path: &Path) -> anyhow::Result<Preview> {
    let names = slide_parts(path, "ppt/slides/slide", ".xml")?;
    if names.is_empty() { anyhow::bail!("this .pptx has no slides") }
    let mut out = Preview { kind: "slides".into(), pages: names.len(), ..Default::default() };
    for (i, name) in names.iter().enumerate() {
        if !out.push(Block::Section { title: format!("Slide {}", i + 1) }) { break }
        let Some(xml) = part(path, name)? else { continue };
        // `a:p` is a paragraph, `a:t` a run of text — the drawing namespace.
        for b in walk(&xml, &["p"], &["t"], |_, _| None) {
            if !out.push(b) { break }
        }
    }
    Ok(out)
}

pub fn read_odp(path: &Path) -> anyhow::Result<Preview> {
    let Some(xml) = part(path, "content.xml")? else {
        anyhow::bail!("this .odp has no content")
    };
    let mut out = Preview { kind: "slides".into(), ..Default::default() };
    let mut page = 0usize;
    // ODP keeps every page in one content.xml; `draw:page` opens each.
    let blocks = walk(&xml, &["p", "h"], &["p", "h", "span", "a"], |name, _| {
        if name == "page" {
            page += 1;
            Some(Block::Section { title: format!("Slide {page}") })
        } else {
            None
        }
    });
    out.pages = page;
    for b in blocks { if !out.push(b) { break } }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_with(parts: &[(&str, &str)]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("sigil-docs-{}-{:p}", std::process::id(), parts));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("doc.zip");
        let f = std::fs::File::create(&path).unwrap();
        let mut w = zip::ZipWriter::new(f);
        let opts: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, body) in parts {
            w.start_file(*name, opts).unwrap();
            w.write_all(body.as_bytes()).unwrap();
        }
        w.finish().unwrap();
        path
    }

    #[test]
    fn docx_paragraphs_headings_and_bullets() {
        let xml = r#"<?xml version="1.0"?>
        <w:document xmlns:w="x"><w:body>
          <w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>The Title</w:t></w:r></w:p>
          <w:p><w:r><w:t>Hello </w:t></w:r><w:r><w:t>world.</w:t></w:r></w:p>
          <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/></w:numPr></w:pPr><w:r><w:t>a bullet</w:t></w:r></w:p>
        </w:body></w:document>"#;
        let p = read_docx(&zip_with(&[("word/document.xml", xml)])).unwrap();
        assert_eq!(p.kind, "document");
        assert_eq!(p.blocks[0], Block::Para { text: "The Title".into(), level: 1, bullet: false });
        // Runs inside one paragraph join into one line rather than three.
        assert_eq!(p.blocks[1], Block::Para { text: "Hello world.".into(), level: 0, bullet: false });
        assert_eq!(p.blocks[2], Block::Para { text: "a bullet".into(), level: 0, bullet: true });
    }

    #[test]
    fn odt_headings_carry_their_outline_level() {
        let xml = r#"<?xml version="1.0"?>
        <office:document-content xmlns:office="o" xmlns:text="t">
          <office:body><office:text>
            <text:h text:outline-level="2">Second level</text:h>
            <text:p>Body text <text:span>with a span</text:span>.</text:p>
          </office:text></office:body>
        </office:document-content>"#;
        let p = read_odt(&zip_with(&[("content.xml", xml)])).unwrap();
        assert_eq!(p.blocks[0], Block::Para { text: "Second level".into(), level: 2, bullet: false });
        assert_eq!(p.blocks[1], Block::Para { text: "Body text with a span.".into(), level: 0, bullet: false });
    }

    #[test]
    fn pptx_slides_are_sectioned_and_ordered_numerically() {
        let slide = |t: &str| format!(
            r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree>
                 <p:sp><p:txBody><a:p><a:r><a:t>{t}</a:t></a:r></a:p></p:txBody></p:sp>
               </p:spTree></p:cSld></p:sld>"#);
        let s1 = slide("First");
        let s2 = slide("Second");
        let s10 = slide("Tenth");
        let p = read_pptx(&zip_with(&[
            ("ppt/slides/slide10.xml", s10.as_str()),
            ("ppt/slides/slide1.xml", s1.as_str()),
            ("ppt/slides/slide2.xml", s2.as_str()),
        ])).unwrap();
        assert_eq!(p.kind, "slides");
        assert_eq!(p.pages, 3);
        // slide10 must come last, not second.
        let texts: Vec<String> = p.blocks.iter().filter_map(|b| match b {
            Block::Para { text, .. } => Some(text.clone()),
            _ => None,
        }).collect();
        assert_eq!(texts, vec!["First", "Second", "Tenth"]);
    }

    #[test]
    fn a_missing_body_part_is_an_error_not_an_empty_preview() {
        let path = zip_with(&[("some/other/part.xml", "<x/>")]);
        assert!(read_docx(&path).is_err());
    }
}
