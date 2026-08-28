//! In-app document previews, in-process: no LibreOffice, no pandoc — each format is read by
//! a Rust crate into one shape. Input is untrusted, so every reader is bounded and reports
//! early stops in `truncated`.

pub mod office;
pub mod raster;
pub mod pdf;
pub mod sheet;
pub mod text;

use std::path::Path;

use serde_json::{json, Value};

/// Ceilings, applied by every reader.
pub const MAX_FILE_BYTES: u64 = 200 * 1024 * 1024;
pub const MAX_BLOCKS: usize = 4_000;
pub const MAX_ROWS: usize = 2_000;
pub const MAX_COLS: usize = 64;
pub const MAX_CELL_CHARS: usize = 512;
pub const MAX_TEXT_CHARS: usize = 2_000_000;

/// One piece of a document — deliberately coarse, a preview, not a layout engine.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// `level` 1..6 for headings, 0 for ordinary text.
    Para { text: String, level: u8, bullet: bool },
    Table { rows: Vec<Vec<String>> },
    /// A page (PDF) or slide (PPTX) boundary, with its own title if it has one.
    Section { title: String },
}

impl Block {
    fn to_json(&self) -> Value {
        match self {
            Block::Para { text, level, bullet } =>
                json!({"t": "p", "text": text, "level": level, "bullet": bullet}),
            Block::Table { rows } => json!({"t": "table", "rows": rows}),
            Block::Section { title } => json!({"t": "section", "title": title}),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Preview {
    /// "pdf" | "document" | "sheet" | "slides" | "markdown" | "text"
    pub kind: String,
    pub title: String,
    pub blocks: Vec<Block>,
    /// Sheets, for spreadsheets: (name, rows).
    pub sheets: Vec<(String, Vec<Vec<String>>)>,
    /// Rendered markup, for Markdown only — already sanitised.
    pub html: String,
    pub pages: usize,
    pub truncated: bool,
    pub note: String,
}

impl Preview {
    pub fn to_json(&self) -> Value {
        json!({
            "kind": self.kind,
            "title": self.title,
            "blocks": self.blocks.iter().map(Block::to_json).collect::<Vec<_>>(),
            "sheets": self.sheets.iter().map(|(n, rows)| json!({"name": n, "rows": rows})).collect::<Vec<_>>(),
            "html": self.html,
            "pages": self.pages,
            "truncated": self.truncated,
            "note": self.note,
        })
    }

    pub fn push(&mut self, b: Block) -> bool {
        if self.blocks.len() >= MAX_BLOCKS { self.truncated = true; return false }
        self.blocks.push(b);
        true
    }
}

/// Extension first (what the sender named it), then mime, which clients get wrong.
pub fn kind_of(filename: &str, mime: Option<&str>) -> &'static str {
    let lower = filename.to_ascii_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    match ext {
        "pdf" => return "pdf",
        "docx" => return "docx",
        "odt" => return "odt",
        "pptx" => return "pptx",
        "odp" => return "odp",
        "xlsx" | "xlsm" | "xlsb" => return "xlsx",
        "xls" => return "xls",
        "ods" => return "ods",
        "csv" => return "csv",
        "tsv" | "tab" => return "tsv",
        "md" | "markdown" | "mdown" => return "md",
        "txt" | "log" | "rst" | "adoc" => return "txt",
        _ => {}
    }
    match mime.unwrap_or("") {
        "application/pdf" => "pdf",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => "docx",
        "application/vnd.oasis.opendocument.text" => "odt",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => "pptx",
        "application/vnd.oasis.opendocument.presentation" => "odp",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "xlsx",
        "application/vnd.ms-excel" => "xls",
        "application/vnd.oasis.opendocument.spreadsheet" => "ods",
        "text/csv" => "csv",
        "text/tab-separated-values" => "tsv",
        "text/markdown" | "text/x-markdown" => "md",
        "text/plain" => "txt",
        _ => "",
    }
}

/// Can we preview this at all? The view asks before offering the button.
pub fn previewable(filename: &str, mime: Option<&str>) -> bool {
    !kind_of(filename, mime).is_empty()
}

pub fn preview(path: &Path, filename: &str, mime: Option<&str>) -> anyhow::Result<Preview> {
    let meta = std::fs::metadata(path)?;
    if meta.len() > MAX_FILE_BYTES {
        anyhow::bail!("that file is too large to preview ({})", crate::timeline::fmt::bytes(meta.len()))
    }
    let kind = kind_of(filename, mime);
    let title = filename.rsplit('/').next().unwrap_or(filename).to_string();
    let mut p = match kind {
        "pdf" => pdf::read(path)?,
        "docx" => office::read_docx(path)?,
        "odt" => office::read_odt(path)?,
        "pptx" => office::read_pptx(path)?,
        "odp" => office::read_odp(path)?,
        "xlsx" | "xls" | "ods" => sheet::read_workbook(path, kind)?,
        "csv" => sheet::read_separated(path, b',')?,
        "tsv" => sheet::read_separated(path, b'\t')?,
        "md" => text::read_markdown(path)?,
        "txt" => text::read_plain(path)?,
        _ => anyhow::bail!("no preview for this kind of file"),
    };
    p.title = title;
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_wins_over_a_wrong_mime() {
        // Clients label .docx as octet-stream all the time.
        assert_eq!(kind_of("Report.DOCX", Some("application/octet-stream")), "docx");
        assert_eq!(kind_of("notes.md", None), "md");
        assert_eq!(kind_of("data.tsv", None), "tsv");
    }

    #[test]
    fn mime_is_the_fallback_when_there_is_no_useful_extension() {
        assert_eq!(kind_of("download", Some("application/pdf")), "pdf");
        assert_eq!(
            kind_of("sheet", Some("application/vnd.oasis.opendocument.spreadsheet")),
            "ods"
        );
    }

    #[test]
    fn unknown_things_are_not_previewable() {
        assert!(!previewable("archive.tar.gz", Some("application/gzip")));
        assert!(!previewable("photo.jpg", Some("image/jpeg")));
        assert!(previewable("q3.xlsx", None));
    }
}
