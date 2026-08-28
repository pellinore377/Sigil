//! PDF page → picture with [`hayro`]: pure Rust, CPU only, no native library to ship. It
//! does not do encrypted files, blend modes or knockout groups, so failure is ordinary and
//! the caller falls back to text.

use std::path::{Path, PathBuf};

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::{RenderCache, RenderSettings};
use tracing::debug;

/// Refuse to walk a document with more pages than this; each page costs a render.
pub const MAX_PAGES: usize = 500;
/// Never rasterise wider than this, whatever the caller asks for.
pub const MAX_WIDTH: u32 = 2_000;

/// What a reader needs before it can lay pages out.
#[derive(Debug, Clone, Copy)]
pub struct PageInfo {
    pub count: usize,
    /// Page one's size in points; pages may differ, and each rendered page carries its own.
    pub width: f32,
    pub height: f32,
}

fn open(bytes: Vec<u8>) -> Option<Pdf> {
    Pdf::new(std::sync::Arc::new(bytes)).ok()
}

pub fn info(bytes: Vec<u8>) -> Option<PageInfo> {
    std::panic::catch_unwind(move || {
        let pdf = open(bytes)?;
        let pages = pdf.pages();
        let first = pages.first()?;
        let (w, h) = first.render_dimensions();
        Some(PageInfo { count: pages.len().min(MAX_PAGES), width: w, height: h })
    })
    .ok()
    .flatten()
}

/// Render one page to a PNG at `out`, `target_w` wide. `None` when hayro cannot draw the file.
pub fn render_page(bytes: Vec<u8>, index: usize, target_w: u32, out: &Path) -> Option<PathBuf> {
    if out.exists() { return Some(out.to_path_buf()) }
    let target_w = target_w.clamp(64, MAX_WIDTH);
    let out = out.to_path_buf();
    let written = std::panic::catch_unwind(move || {
        let pdf = open(bytes)?;
        let pages = pdf.pages();
        if index >= pages.len() || index >= MAX_PAGES { return None }
        let page = pages.get(index)?;
        let (pw, ph) = page.render_dimensions();
        if pw <= 0.0 || ph <= 0.0 { return None }
        let scale = target_w as f32 / pw;

        let settings = RenderSettings {
            x_scale: scale,
            y_scale: scale,
            // Paper, not transparency: a page composited onto a dark chat would be unreadable.
            bg_color: hayro::vello_cpu::color::palette::css::WHITE,
            ..Default::default()
        };
        let pixmap = hayro::render(&page, &RenderCache::new(), &InterpreterSettings::default(), &settings);
        let (w, h) = (pixmap.width() as u32, pixmap.height() as u32);
        // Unpremultiplied is what a PNG stores; the opaque background makes the
        // two identical here, but relying on that breaks a transparent page.
        let mut buf = Vec::with_capacity((w * h * 4) as usize);
        for px in pixmap.take_unpremultiplied() {
            buf.extend_from_slice(&[px.r, px.g, px.b, px.a]);
        }
        let img = image::RgbaImage::from_raw(w, h, buf)?;
        // Temp name then rename, so a reader never opens a half-written file.
        let tmp = out.with_extension("part.png");
        img.save(&tmp).ok()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp, &out).ok()?;
        Some(out)
    })
    .ok()
    .flatten();
    if written.is_none() { debug!("raster: could not draw page {index}") }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-page PDF with real text, assembled by hand — no toolchain needed.
    fn tiny_pdf() -> Vec<u8> {
        let stream = b"BT /F1 24 Tf 72 700 Td (Hello) Tj ET".to_vec();
        let objs: Vec<Vec<u8>> = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
            [format!("<< /Length {} >>\nstream\n", stream.len()).into_bytes(), stream.clone(), b"\nendstream".to_vec()].concat(),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offs = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offs.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(o);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
        for off in &offs { out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes()); }
        out.extend_from_slice(
            format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n", objs.len() + 1, xref).as_bytes(),
        );
        out
    }

    #[test]
    fn a_pdf_reports_its_page_count_and_shape() {
        let i = info(tiny_pdf()).expect("a well-formed PDF opens");
        assert_eq!(i.count, 1);
        // US Letter, in points.
        assert!((i.width - 612.0).abs() < 1.0, "width {}", i.width);
        assert!((i.height - 792.0).abs() < 1.0, "height {}", i.height);
    }

    #[test]
    fn a_page_renders_to_a_png_of_the_width_asked_for() {
        let dir = std::env::temp_dir().join(format!("sigil-raster-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("page0.png");
        let _ = std::fs::remove_file(&out);
        let made = render_page(tiny_pdf(), 0, 400, &out).expect("a page hayro can draw");
        assert_eq!(made, out);
        let (w, h) = image::image_dimensions(&out).expect("a readable PNG");
        assert_eq!(w, 400);
        assert!(h > w, "portrait, got {w}x{h}");
        assert!((h as f32 / w as f32 - 792.0 / 612.0).abs() < 0.02);
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn rubbish_is_refused_rather_than_panicking() {
        // Input comes from strangers; a young parser must not take the engine down.
        assert!(info(b"not a pdf at all".to_vec()).is_none());
        assert!(info(Vec::new()).is_none());
        let dir = std::env::temp_dir().join(format!("sigil-raster-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(render_page(b"%PDF-1.4\ngarbage".to_vec(), 0, 200, &dir.join("nope.png")).is_none());
        // A page that does not exist is a miss, not a crash.
        assert!(render_page(tiny_pdf(), 99, 200, &dir.join("nope2.png")).is_none());
    }
}
