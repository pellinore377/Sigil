use crate::{formats::Format, Content, Error, Preview, Request, MAX_INPUT};
use std::{
    io::{Cursor, Read},
    path::Path,
};
mod audio;
mod av;
mod mesh;
mod office;
mod pdf;
mod table;

pub fn render(path: &Path, format: Format, request: &Request) -> Result<Preview, Error> {
    request.validate()?;
    if std::fs::metadata(path)?.len() > MAX_INPUT {
        return Err(Error::Limit);
    }
    let preview = match (format, request) {
        (Format::Pdf, Request::Page { index, width }) => pdf::render(path, *index, *width)?,
        (
            Format::Csv | Format::Tsv | Format::Spreadsheet,
            Request::Table { sheet, row, column },
        ) => table::render(path, format, *sheet, *row, *column)?,
        (Format::Text | Format::Contact, Request::Text { offset }) => text(path, *offset)?,
        (Format::Document | Format::Presentation, Request::Page { index, width }) => {
            office::render(path, *index, *width)?
        }
        (
            Format::Audio | Format::Video,
            Request::Audio {
                start_ms,
                duration_ms,
            },
        ) => match audio::render(path, *start_ms, *duration_ms) {
            Ok(value) => value,
            Err(Error::Unsupported) => av::audio(path, *start_ms, *duration_ms)?,
            Err(error) => return Err(error),
        },
        (Format::Image | Format::Gif | Format::Video, Request::Frame { at_ms, width }) => {
            av::frame(path, *at_ms, *width)?
        }
        (Format::Stl | Format::ThreeMf, Request::Mesh { yaw, pitch, width }) => {
            mesh::render(path, format, *yaw, *pitch, *width)?
        }
        _ => return Err(Error::Unsupported),
    };
    preview.validate()?;
    Ok(preview)
}
fn text(path: &Path, offset: u64) -> Result<Preview, Error> {
    use std::io::{Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    let total = file.metadata()?.len();
    if offset > total {
        return Err(Error::Invalid);
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    file.take(65536).read_to_end(&mut bytes)?;
    let value = match std::str::from_utf8(&bytes) {
        Ok(s) => s,
        Err(e) if e.error_len().is_none() && offset + (bytes.len() as u64) < total => {
            std::str::from_utf8(&bytes[..e.valid_up_to()]).map_err(|_| Error::Invalid)?
        }
        Err(_) => return Err(Error::Invalid),
    };
    let end = offset + value.len() as u64;
    Ok(Preview {
        content: Content::Text {
            text: value.into(),
            next: (end < total).then_some(end),
        },
        bytes: Vec::new(),
    })
}
pub(super) fn check_zip(path: &Path) -> Result<(), Error> {
    let mut file = std::fs::File::open(path)?;
    let mut magic = [0; 4];
    if file.read(&mut magic)? != 4 || &magic != b"PK\x03\x04" {
        return Ok(());
    }
    let mut archive =
        zip::ZipArchive::new(std::fs::File::open(path)?).map_err(|_| Error::Invalid)?;
    if archive.len() > 8192 {
        return Err(Error::Limit);
    }
    let mut total = 0u64;
    let mut buffer = [0u8; 8192];
    for i in 0..archive.len() {
        let mut part = archive.by_index(i).map_err(|_| Error::Invalid)?;
        if part.enclosed_name().is_none() || part.is_symlink() || part.encrypted() {
            return Err(Error::Invalid);
        }
        total = total.checked_add(part.size()).ok_or(Error::Limit)?;
        if total > 256 * 1024 * 1024 || part.size() > 64 * 1024 * 1024 {
            return Err(Error::Limit);
        }
        let mut expanded = 0u64;
        loop {
            let count = part.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            expanded += count as u64;
            if expanded > part.size() {
                return Err(Error::Invalid);
            }
        }
    }
    Ok(())
}
