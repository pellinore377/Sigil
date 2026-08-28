//! Spreadsheets and separated values, via `calamine` (XLSX, XLSB, legacy XLS, ODS). Cells
//! are rendered to strings here, not in the view: a date is not a float, an error is not empty.

use std::path::Path;

use calamine::{Data, Reader};

use super::{Preview, MAX_CELL_CHARS, MAX_COLS, MAX_ROWS};

fn cell_text(d: &Data) -> String {
    let s = match d {
        Data::Empty => String::new(),
        Data::String(s) => s.clone(),
        Data::Float(f) => {
            // 3 rather than 3.0: spreadsheets store every integer as a float.
            if f.fract() == 0.0 && f.abs() < 1e15 { format!("{}", *f as i64) } else { format!("{f}") }
        }
        Data::Int(i) => i.to_string(),
        Data::Bool(b) => if *b { "TRUE".into() } else { "FALSE".into() },
        Data::Error(e) => format!("#{e:?}"),
        // calamine hands back an Excel serial; fall back to the raw number if it is not a date.
        Data::DateTime(dt) => match dt.as_datetime() {
            Some(d) => d.format("%Y-%m-%d %H:%M").to_string(),
            None => dt.as_f64().to_string(),
        },
        Data::DateTimeIso(s) => s.clone(),
        Data::DurationIso(s) => s.clone(),
    };
    if s.chars().count() <= MAX_CELL_CHARS { s } else { s.chars().take(MAX_CELL_CHARS).collect::<String>() + "…" }
}

/// Trailing empty columns are common and would otherwise fill the preview.
fn trim_trailing_empty(rows: &mut Vec<Vec<String>>) {
    let width = rows.iter().map(|r| r.iter().rposition(|c| !c.is_empty()).map(|i| i + 1).unwrap_or(0)).max().unwrap_or(0);
    for r in rows.iter_mut() { r.truncate(width) }
    while rows.last().map(|r| r.iter().all(|c| c.is_empty())).unwrap_or(false) { rows.pop(); }
}

pub fn read_workbook(path: &Path, kind: &str) -> anyhow::Result<Preview> {
    let mut wb = calamine::open_workbook_auto(path)
        .map_err(|e| anyhow::anyhow!("could not open the workbook: {e}"))?;
    let names = wb.sheet_names().to_vec();
    if names.is_empty() { anyhow::bail!("that workbook has no sheets") }
    let mut out = Preview { kind: "sheet".into(), pages: names.len(), ..Default::default() };
    out.note = match kind {
        "xls" => "Legacy Excel workbook".into(),
        "ods" => "OpenDocument spreadsheet".into(),
        _ => String::new(),
    };
    for name in names {
        let Ok(range) = wb.worksheet_range(&name) else { continue };
        let mut rows: Vec<Vec<String>> = Vec::new();
        for row in range.rows().take(MAX_ROWS) {
            rows.push(row.iter().take(MAX_COLS).map(cell_text).collect());
            if row.len() > MAX_COLS { out.truncated = true }
        }
        if range.rows().count() > MAX_ROWS { out.truncated = true }
        trim_trailing_empty(&mut rows);
        out.sheets.push((name, rows));
    }
    Ok(out)
}

pub fn read_separated(path: &Path, delim: u8) -> anyhow::Result<Preview> {
    let bytes = std::fs::read(path)?;
    // CSVs arrive in any encoding; Windows-1252 is what "exported from Excel" means.
    let text = match std::str::from_utf8(&bytes) {
        Ok(s) => s.to_string(),
        Err(_) => encoding_rs::WINDOWS_1252.decode(&bytes).0.into_owned(),
    };
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delim)
        .flexible(true)          // ragged rows are data, not a parse failure
        .has_headers(false)
        .from_reader(text.as_bytes());
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut truncated = false;
    for rec in rdr.records() {
        let Ok(rec) = rec else { continue };
        if rows.len() >= MAX_ROWS { truncated = true; break }
        let mut row: Vec<String> = rec.iter().take(MAX_COLS).map(|c| {
            if c.chars().count() <= MAX_CELL_CHARS { c.to_string() }
            else { c.chars().take(MAX_CELL_CHARS).collect::<String>() + "…" }
        }).collect();
        if rec.len() > MAX_COLS { truncated = true }
        // Pad so every row is the same width; the view draws a grid.
        row.resize(row.len().max(1), String::new());
        rows.push(row);
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    for r in rows.iter_mut() { r.resize(width, String::new()) }
    trim_trailing_empty(&mut rows);
    Ok(Preview {
        kind: "sheet".into(),
        sheets: vec![(if delim == b'\t' { "TSV".into() } else { "CSV".into() }, rows)],
        pages: 1,
        truncated,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str, body: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("sigil-sheet-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn csv_becomes_one_sheet_of_rows() {
        let p = tmp("a.csv", b"name,qty\nbolts,3\nnuts,12\n");
        let out = read_separated(&p, b',').unwrap();
        assert_eq!(out.kind, "sheet");
        assert_eq!(out.sheets.len(), 1);
        assert_eq!(out.sheets[0].1, vec![
            vec!["name".to_string(), "qty".to_string()],
            vec!["bolts".to_string(), "3".to_string()],
            vec!["nuts".to_string(), "12".to_string()],
        ]);
    }

    #[test]
    fn quoted_fields_with_commas_and_newlines_survive() {
        let p = tmp("q.csv", b"a,b\n\"one, two\",\"line\nbreak\"\n");
        let out = read_separated(&p, b',').unwrap();
        assert_eq!(out.sheets[0].1[1][0], "one, two");
        assert_eq!(out.sheets[0].1[1][1], "line\nbreak");
    }

    #[test]
    fn tsv_uses_tabs_and_says_so() {
        let p = tmp("a.tsv", b"x\ty\n1\t2\n");
        let out = read_separated(&p, b'\t').unwrap();
        assert_eq!(out.sheets[0].0, "TSV");
        assert_eq!(out.sheets[0].1[1], vec!["1".to_string(), "2".to_string()]);
    }

    #[test]
    fn ragged_rows_are_padded_rather_than_dropped() {
        let p = tmp("ragged.csv", b"a,b,c\n1\n2,3\n");
        let out = read_separated(&p, b',').unwrap();
        let rows = &out.sheets[0].1;
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|r| r.len() == 3), "every row padded to the widest");
        assert_eq!(rows[1], vec!["1".to_string(), String::new(), String::new()]);
    }

    #[test]
    fn latin1_csv_does_not_come_back_as_replacement_characters() {
        // "café" in Windows-1252 is not valid UTF-8.
        let p = tmp("latin.csv", b"name\ncaf\xe9\n");
        let out = read_separated(&p, b',').unwrap();
        assert_eq!(out.sheets[0].1[1][0], "café");
    }

    #[test]
    fn integers_do_not_come_out_as_floats() {
        assert_eq!(cell_text(&Data::Float(3.0)), "3");
        assert_eq!(cell_text(&Data::Float(3.5)), "3.5");
        assert_eq!(cell_text(&Data::Bool(true)), "TRUE");
        assert_eq!(cell_text(&Data::Empty), "");
    }


    /// A minimal XLSX built here: the one test that actually exercises calamine.
    fn minimal_xlsx() -> std::path::PathBuf {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("sigil-xlsx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("book.xlsx");
        let f = std::fs::File::create(&path).unwrap();
        let mut w = zip::ZipWriter::new(f);
        let o: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        let parts: Vec<(&str, String)> = vec![
            ("[Content_Types].xml", r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>"#.to_string()),
            ("_rels/.rels", r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#.to_string()),
            ("xl/_rels/workbook.xml.rels", r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#.to_string()),
            ("xl/workbook.xml", r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<sheets><sheet name="Q3" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#.to_string()),
            ("xl/worksheets/sheet1.xml", r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<sheetData>
<row r="1"><c r="A1" t="inlineStr"><is><t>Widget</t></is></c><c r="B1"><v>12</v></c></row>
<row r="2"><c r="A2" t="inlineStr"><is><t>Sprocket</t></is></c><c r="B2"><v>7.5</v></c></row>
</sheetData>
</worksheet>"#.to_string()),
        ];
        for (name, body) in parts {
            w.start_file(name, o).unwrap();
            w.write_all(body.as_bytes()).unwrap();
        }
        w.finish().unwrap();
        path
    }

    #[test]
    fn a_real_xlsx_reads_through_calamine() {
        let p = minimal_xlsx();
        let out = read_workbook(&p, "xlsx").unwrap();
        assert_eq!(out.kind, "sheet");
        assert_eq!(out.sheets.len(), 1);
        assert_eq!(out.sheets[0].0, "Q3", "the sheet keeps its name");
        assert_eq!(out.sheets[0].1[0], vec!["Widget".to_string(), "12".to_string()]);
        assert_eq!(out.sheets[0].1[1], vec!["Sprocket".to_string(), "7.5".to_string()]);
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn a_real_ods_reads_through_calamine() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("sigil-ods-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("book.ods");
        let f = std::fs::File::create(&path).unwrap();
        let mut w = zip::ZipWriter::new(f);
        let stored: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        // ODF requires mimetype first and uncompressed.
        w.start_file("mimetype", stored).unwrap();
        w.write_all(b"application/vnd.oasis.opendocument.spreadsheet").unwrap();
        let o: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        // calamine insists on the ODF manifest, and a real .ods always has one.
        w.start_file("META-INF/manifest.xml", o).unwrap();
        w.write_all(br#"<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2">
<manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/>
<manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>
</manifest:manifest>"#).unwrap();
        w.start_file("content.xml", o).unwrap();
        // No whitespace between table elements: calamine's ODS reader rejects a text node where it expects a cell.
        w.write_all(br#"<?xml version="1.0" encoding="UTF-8"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:spreadsheet><table:table table:name="Prices"><table:table-row><table:table-cell office:value-type="string"><text:p>Bolt</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="3"><text:p>3</text:p></table:table-cell></table:table-row></table:table></office:spreadsheet></office:body></office:document-content>"#).unwrap();
        w.finish().unwrap();

        let out = read_workbook(&path, "ods").unwrap();
        assert_eq!(out.sheets[0].0, "Prices");
        assert_eq!(out.sheets[0].1[0][0], "Bolt");
        assert_eq!(out.sheets[0].1[0][1], "3");
        assert_eq!(out.note, "OpenDocument spreadsheet");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn trailing_empty_columns_are_trimmed() {
        let mut rows = vec![
            vec!["a".to_string(), String::new(), String::new()],
            vec!["b".to_string(), String::new(), String::new()],
        ];
        trim_trailing_empty(&mut rows);
        assert!(rows.iter().all(|r| r.len() == 1));
    }
}
