//! The link offer as a QR code, rendered straight into a Slint image: dark
//! modules on white with a four-module quiet zone, scaled up so the pixels
//! land crisply when the page draws it with `image-rendering: pixelated`.

use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

pub fn image(text: &str) -> Option<Image> {
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    let n = code.width();
    let quiet = 4;
    let scale = 4;
    let side = (n + 2 * quiet) * scale;
    let colors = code.to_colors();
    let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(side as u32, side as u32);
    let px = buf.make_mut_slice();
    for p in px.iter_mut() {
        *p = Rgba8Pixel {
            r: 255,
            g: 255,
            b: 255,
            a: 255,
        };
    }
    for y in 0..n {
        for x in 0..n {
            if colors[y * n + x] == qrcode::Color::Dark {
                for dy in 0..scale {
                    for dx in 0..scale {
                        let ix = (x + quiet) * scale + dx;
                        let iy = (y + quiet) * scale + dy;
                        px[iy * side + ix] = Rgba8Pixel {
                            r: 20,
                            g: 20,
                            b: 20,
                            a: 255,
                        };
                    }
                }
            }
        }
    }
    Some(Image::from_rgba8(buf))
}
