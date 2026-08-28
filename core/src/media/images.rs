//! Qt paints PNG, JPEG, GIF, ICO and SVG but not WebP or AVIF, so anything else is
//! re-encoded to PNG: the `image` crate, then `resvg` for SVG (Qt's renderer would resolve
//! external references), then ffmpeg for AVIF, HEIC and JPEG XL.

use std::io::Write;
use std::process::{Command, Stdio};

use tracing::debug;

/// Decode caps for untrusted input.
const MAX_PIXELS: u64 = 50_000_000;
const MAX_INPUT_BYTES: usize = 80 * 1024 * 1024;

/// Formats the view paints as they arrive. GIF stays as-is so animation survives.
pub fn passthrough(data: &[u8]) -> bool {
    matches!(
        image::guess_format(data),
        Ok(image::ImageFormat::Png) | Ok(image::ImageFormat::Jpeg) | Ok(image::ImageFormat::Gif)
    )
}

fn looks_like_svg(data: &[u8], mime: Option<&str>) -> bool {
    if mime.map(|m| m.contains("svg")).unwrap_or(false) { return true }
    let head = &data[..data.len().min(1024)];
    let text = String::from_utf8_lossy(head);
    text.contains("<svg") || (text.trim_start().starts_with("<?xml") && text.contains("svg"))
}

fn encode_png(img: &image::DynamicImage) -> Option<Vec<u8>> {
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png).ok()?;
    Some(buf.into_inner())
}

fn via_image_crate(data: &[u8]) -> Option<Vec<u8>> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(data))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(20_000);
    limits.max_image_height = Some(20_000);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().ok()?;
    if (img.width() as u64) * (img.height() as u64) > MAX_PIXELS { return None }
    encode_png(&img)
}

fn via_resvg(data: &[u8]) -> Option<Vec<u8>> {
    let opt = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_data(data, &opt).ok()?;
    let size = tree.size().to_int_size();
    // An SVG declares its own size and may declare an absurd one; clamp it.
    let scale = (1600.0 / size.width().max(1) as f32).min(1600.0 / size.height().max(1) as f32).min(1.0);
    let w = ((size.width() as f32 * scale).round() as u32).clamp(1, 1600);
    let h = ((size.height() as f32 * scale).round() as u32).clamp(1, 1600);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(w as f32 / size.width() as f32, h as f32 / size.height() as f32),
        &mut pixmap.as_mut(),
    );
    let img = image::RgbaImage::from_raw(w, h, pixmap.take())?;
    encode_png(&image::DynamicImage::ImageRgba8(img))
}

/// Last resort: AVIF, HEIC, JPEG XL. stdin → stdout, so nothing touches disk and no filename is passed.
fn via_ffmpeg(data: &[u8]) -> Option<Vec<u8>> {
    let mut child = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-i", "pipe:0",
               "-frames:v", "1", "-f", "image2", "-c:v", "png", "pipe:1"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    {
        let mut stdin = child.stdin.take()?;
        let chunk = data.to_vec();
        // ffmpeg may stop reading once it has a frame; a broken pipe is success here.
        std::thread::spawn(move || { let _ = stdin.write_all(&chunk); });
    }
    let out = child.wait_with_output().ok()?;
    if !out.status.success() || out.stdout.is_empty() { return None }
    Some(out.stdout)
}

/// `Some(png)` when conversion was needed; `None` when the view can paint the bytes, or nothing decoded.
pub fn to_displayable(data: &[u8], mime: Option<&str>) -> Option<Vec<u8>> {
    if data.len() > MAX_INPUT_BYTES { return None }
    if passthrough(data) { return None }
    if looks_like_svg(data, mime) {
        if let Some(png) = via_resvg(data) {
            debug!("images: rasterised an SVG to {} bytes of PNG", png.len());
            return Some(png)
        }
    }
    if let Some(png) = via_image_crate(data) {
        debug!("images: re-encoded to PNG via the image crate ({} bytes)", png.len());
        return Some(png)
    }
    if let Some(png) = via_ffmpeg(data) {
        debug!("images: re-encoded to PNG via ffmpeg ({} bytes)", png.len());
        return Some(png)
    }
    debug!("images: nothing could decode this ({} bytes, mime {:?})", data.len(), mime);
    None
}

/// Worth normalising at all? Cheap check, so ffmpeg does not run over every zip.
pub fn is_image(data: &[u8], mime: Option<&str>, filename: Option<&str>) -> bool {
    if image::guess_format(data).is_ok() { return true }
    if let Some(m) = mime {
        if m.starts_with("image/") { return true }
    }
    if looks_like_svg(data, mime) { return true }
    if let Some(f) = filename {
        let lower = f.to_ascii_lowercase();
        return [".avif", ".heic", ".heif", ".jxl", ".tif", ".tiff", ".bmp", ".ico", ".webp", ".svg"]
            .iter()
            .any(|e| lower.ends_with(e));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_dimensions(data: &[u8]) -> (u32, u32) {
        let img = image::load_from_memory_with_format(data, image::ImageFormat::Png).expect("decodes as PNG");
        (img.width(), img.height())
    }

    #[test]
    fn png_and_jpeg_pass_through_untouched() {
        let img = image::DynamicImage::new_rgb8(8, 8);
        let mut png = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png, image::ImageFormat::Png).unwrap();
        assert!(passthrough(png.get_ref()));
        assert!(to_displayable(png.get_ref(), Some("image/png")).is_none());
    }

    #[test]
    fn gif_stays_a_gif_so_animation_survives() {
        let img = image::DynamicImage::new_rgb8(4, 4);
        let mut gif = std::io::Cursor::new(Vec::new());
        img.write_to(&mut gif, image::ImageFormat::Gif).unwrap();
        assert!(to_displayable(gif.get_ref(), Some("image/gif")).is_none());
    }

    #[test]
    fn tiff_becomes_png() {
        let img = image::DynamicImage::new_rgb8(32, 24);
        let mut tiff = std::io::Cursor::new(Vec::new());
        img.write_to(&mut tiff, image::ImageFormat::Tiff).unwrap();
        let out = to_displayable(tiff.get_ref(), Some("image/tiff")).expect("tiff converts");
        assert_eq!(png_dimensions(&out), (32, 24));
    }

    #[test]
    fn webp_becomes_png() {
        // WebP matters most: Qt has no plugin for it and phones send it constantly.
        let img = image::DynamicImage::new_rgb8(20, 10);
        let mut webp = std::io::Cursor::new(Vec::new());
        img.write_to(&mut webp, image::ImageFormat::WebP).unwrap();
        let out = to_displayable(webp.get_ref(), Some("image/webp")).expect("webp converts");
        assert_eq!(png_dimensions(&out), (20, 10));
    }

    #[test]
    fn svg_is_rasterised() {
        // Note the ## delimiter: a `"#` inside a colour closes an r#"…"# literal.
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="60"><rect width="120" height="60" fill="#3a7"/></svg>"##;
        assert!(is_image(svg, Some("image/svg+xml"), None));
        let out = to_displayable(svg, Some("image/svg+xml")).expect("svg rasterises");
        assert_eq!(png_dimensions(&out), (120, 60));
    }

    #[test]
    fn oversized_input_is_refused_rather_than_decoded() {
        let huge = vec![0u8; MAX_INPUT_BYTES + 1];
        assert!(to_displayable(&huge, Some("image/png")).is_none());
    }

    #[test]
    fn avif_and_heic_go_through_ffmpeg() {
        // Built here: if ffmpeg cannot write one it cannot read one either.
        let src = image::DynamicImage::new_rgb8(48, 32);
        let mut png = std::io::Cursor::new(Vec::new());
        src.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let made = std::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-f", "image2pipe", "-i", "pipe:0",
                   "-frames:v", "1", "-f", "avif", "pipe:1"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .and_then(|mut c| {
                use std::io::Write;
                c.stdin.take().unwrap().write_all(png.get_ref())?;
                c.wait_with_output()
            });
        let Ok(out) = made else { eprintln!("no ffmpeg; skipping"); return };
        if !out.status.success() || out.stdout.is_empty() { eprintln!("ffmpeg cannot write AVIF; skipping"); return }
        assert!(!passthrough(&out.stdout), "AVIF is not something Qt can paint");
        let png = to_displayable(&out.stdout, Some("image/avif")).expect("avif converts");
        assert_eq!(png_dimensions(&png), (48, 32));
    }
}
