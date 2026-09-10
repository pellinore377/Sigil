#![cfg(feature = "portable")]
use sigil_media::{Content, Error, Request, decode::render_portable, formats::Format};
use std::io::{Seek, SeekFrom, Write};

#[test]
fn delegated_unlinked_file_can_be_paged_without_path_access() {
    let mut file = tempfile::tempfile().unwrap();
    write!(file, "{}é<script>literal</script>", "x".repeat(65535)).unwrap();
    file.seek(SeekFrom::End(0)).unwrap();
    let first = render_portable(
        file.try_clone().unwrap(),
        Format::Text,
        &Request::Text { offset: 0 },
    )
    .unwrap();
    let Content::Text {
        text,
        next: Some(offset),
    } = first.content
    else {
        panic!()
    };
    assert_eq!(text.len(), 65535);
    assert_eq!(offset, 65535);
    let second = render_portable(file, Format::Text, &Request::Text { offset }).unwrap();
    assert_eq!(
        second.content,
        Content::Text {
            text: "é<script>literal</script>".into(),
            next: None
        }
    );
}

#[test]
fn portable_tables_preserve_literal_formulas_and_reject_bad_requests() {
    let mut file = tempfile::tempfile().unwrap();
    file.write_all(b"a,b\n1,\"line1\nline2\"\n=HYPERLINK(1),<script>\n")
        .unwrap();
    let table = render_portable(
        file.try_clone().unwrap(),
        Format::Csv,
        &Request::Table {
            sheet: 0,
            row: 2,
            column: 0,
        },
    )
    .unwrap();
    let Content::Table {
        cells,
        total_rows,
        total_columns,
        ..
    } = table.content
    else {
        panic!()
    };
    assert_eq!((total_rows, total_columns), (3, 2));
    assert_eq!(cells, vec![vec!["=HYPERLINK(1)", "<script>"]]);
    assert_eq!(
        render_portable(
            file.try_clone().unwrap(),
            Format::Csv,
            &Request::Table {
                sheet: 0,
                row: 3,
                column: 0
            }
        )
        .unwrap()
        .content,
        Content::Table {
            sheets: vec!["Data".into()],
            sheet: 0,
            row: 3,
            column: 0,
            total_rows: 3,
            total_columns: 2,
            cells: vec![]
        }
    );
    assert_eq!(
        render_portable(
            file.try_clone().unwrap(),
            Format::Csv,
            &Request::Table {
                sheet: 0,
                row: 4,
                column: 0
            }
        ),
        Err(Error::Limit)
    );
    assert_eq!(
        render_portable(
            file,
            Format::Pdf,
            &Request::Page {
                index: 0,
                width: 400
            }
        ),
        Err(Error::Unsupported)
    );
}
