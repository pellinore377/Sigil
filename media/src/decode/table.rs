use super::*;
use calamine::Reader;
pub(super) fn render(
    input: &File,
    format: Format,
    sheet: u32,
    row: u32,
    column: u32,
) -> Result<Preview, Error> {
    if format == Format::Spreadsheet {
        check_zip(input)?;
        let mut bytes = Vec::new();
        source(input)?.take(MAX_INPUT + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_INPUT {
            return Err(Error::Limit);
        }
        let bytes: std::sync::Arc<[u8]> = bytes.into();
        let mut file =
            calamine::open_workbook_auto_from_rs(Cursor::new(bytes)).map_err(|_| Error::Invalid)?;
        let sheets = file.sheet_names().to_vec();
        let name = sheets.get(sheet as usize).ok_or(Error::Invalid)?;
        let range = file.worksheet_range(name).map_err(|_| Error::Invalid)?;
        let (height, width) = range
            .end()
            .map_or((0, 0), |(r, c)| (r as usize + 1, c as usize + 1));
        if height > 1_048_576 || width > 16_384 {
            return Err(Error::Limit);
        }
        let cells = (row..(height as u32).min(row + 128))
            .map(|r| {
                (column..(width as u32).min(column + 32))
                    .map(|c| {
                        range
                            .get_value((r, c))
                            .map_or_else(String::new, ToString::to_string)
                    })
                    .collect()
            })
            .collect();
        return Ok(Preview {
            content: Content::Table {
                sheets,
                sheet,
                row,
                column,
                total_rows: height as u32,
                total_columns: width as u32,
                cells,
            },
            bytes: Vec::new(),
        });
    }
    if sheet != 0 {
        return Err(Error::Invalid);
    }
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(if format == Format::Tsv { b'\t' } else { b',' })
        .from_reader(source(input)?);
    let mut total_rows = 0;
    let mut total_columns = 0;
    let mut cells = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|_| Error::Invalid)?;
        if record.len() > 16384 || total_rows == 1_048_576 {
            return Err(Error::Limit);
        }
        total_columns = total_columns.max(record.len() as u32);
        if total_rows >= row && cells.len() < 128 {
            cells.push(
                record
                    .iter()
                    .skip(column as usize)
                    .take(32)
                    .map(str::to_owned)
                    .collect(),
            );
        }
        total_rows += 1;
    }
    Ok(Preview {
        content: Content::Table {
            sheets: vec!["Data".into()],
            sheet,
            row,
            column,
            total_rows,
            total_columns,
            cells,
        },
        bytes: Vec::new(),
    })
}
