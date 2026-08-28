//! PDF text extraction with `pdf-extract`: a *reading* preview. A scan has no text layer
//! and says so rather than previewing blank; page rasterising lives in `raster`.

use std::path::Path;

use super::{Block, Preview};

pub fn read(path: &Path) -> anyhow::Result<Preview> {
    let bytes = std::fs::read(path)?;
    // pdf-extract panics on some malformed files instead of returning an error.
    let extracted = std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem(&bytes))
        .map_err(|_| anyhow::anyhow!("this PDF could not be read"))?
        .map_err(|e| anyhow::anyhow!("this PDF could not be read: {e}"))?;

    let mut out = Preview { kind: "pdf".into(), ..Default::default() };
    // pdf-extract separates pages with a form feed.
    let pages: Vec<&str> = extracted.split('\u{c}').collect();
    out.pages = pages.len();
    let mut any_text = false;
    for (i, page) in pages.iter().enumerate() {
        if !out.push(Block::Section { title: format!("Page {}", i + 1) }) { break }
        for line in page.lines() {
            let line = line.trim();
            if line.is_empty() { continue }
            any_text = true;
            if !out.push(Block::Para { text: line.to_string(), level: 0, bullet: false }) { break }
        }
    }
    if !any_text {
        out.note = "No text layer — this looks like a scan or a drawing, so there is nothing to show here.".into();
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The smallest hand-written PDF with a text layer: one page, one Tj.
    fn tiny_pdf() -> Vec<u8> {
        let content = b"BT /F1 24 Tf 72 700 Td (Hello Sigil) Tj ET";
        let mut objs: Vec<String> = Vec::new();
        objs.push("<< /Type /Catalog /Pages 2 0 R >>".into());
        objs.push("<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into());
        objs.push("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".into());
        objs.push(format!("<< /Length {} >>\nstream\n{}\nendstream", content.len(), String::from_utf8_lossy(content)));
        objs.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into());

        let mut pdf = String::from("%PDF-1.4\n");
        let mut offsets = Vec::new();
        for (i, body) in objs.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.push_str(&format!("{} 0 obj\n{}\nendobj\n", i + 1, body));
        }
        let xref_at = pdf.len();
        pdf.push_str(&format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1));
        for off in &offsets { pdf.push_str(&format!("{off:010} 00000 n \n")); }
        pdf.push_str(&format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            objs.len() + 1,
            xref_at
        ));
        pdf.into_bytes()
    }

    fn tmp(name: &str, body: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("sigil-pdf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn extracts_the_text_of_a_page() {
        let p = tmp("hello.pdf", &tiny_pdf());
        let out = read(&p).unwrap();
        assert_eq!(out.kind, "pdf");
        assert!(out.pages >= 1);
        let text: String = out.blocks.iter().filter_map(|b| match b {
            Block::Para { text, .. } => Some(text.clone()),
            _ => None,
        }).collect::<Vec<_>>().join(" ");
        assert!(text.contains("Hello Sigil"), "got: {text}");
    }

    #[test]
    fn pages_are_sectioned() {
        let p = tmp("hello2.pdf", &tiny_pdf());
        let out = read(&p).unwrap();
        assert!(matches!(out.blocks.first(), Some(Block::Section { .. })));
    }

    #[test]
    fn rubbish_is_an_error_rather_than_a_panic() {
        let p = tmp("bad.pdf", b"%PDF-1.4\nthis is not a pdf at all");
        assert!(read(&p).is_err());
    }
}
