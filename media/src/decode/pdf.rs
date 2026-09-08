use super::*;
use pdfium_render::prelude::*;
pub(super) fn render(path: &Path, index: u32, width: u32) -> Result<Preview, Error> {
    let library = std::env::var_os("SIGIL_PDFIUM").ok_or(Error::Unavailable)?;
    let bindings = Pdfium::bind_to_library(library).map_err(|_| Error::Unavailable)?;
    let pdfium = Pdfium::new(bindings);
    let document = pdfium
        .load_pdf_from_file(path, None)
        .map_err(|_| Error::Invalid)?;
    let pages = document.pages().len() as u32;
    if pages == 0 || index >= pages {
        return Err(Error::Invalid);
    }
    let page = document
        .pages()
        .get(index.try_into().map_err(|_| Error::Limit)?)
        .map_err(|_| Error::Invalid)?;
    let bitmap = page
        .render_with_config(
            &PdfRenderConfig::new()
                .set_target_width(width as i32)
                .set_maximum_height(2048),
        )
        .map_err(|_| Error::Invalid)?;
    Ok(Preview {
        content: Content::Image {
            width: bitmap.width() as u32,
            height: bitmap.height() as u32,
            pages,
            index,
        },
        bytes: bitmap.as_rgba_bytes(),
    })
}
