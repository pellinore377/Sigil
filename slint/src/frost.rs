//! The frosted page behind the message sheet, done the way the QML did it:
//! a snapshot of what is on screen, blurred. Slint has no backdrop blur, so
//! the window is captured (`take_snapshot`), shrunk, box-blurred twice
//! (which lands close to a Gaussian at the radius MultiEffect used), and
//! handed back as an image the sheet stretches over the page. The pressed
//! bubble's copy is a crop of the same snapshot at full size.

use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

/// Downscale factor before blurring; the page is drawn 55 % black on top
/// anyway, so nothing finer survives.
const SHRINK: u32 = 4;
const RADIUS: i32 = 6;

pub struct Snapshot {
    pub buf: SharedPixelBuffer<Rgba8Pixel>,
}

impl Snapshot {
    pub fn take(window: &slint::Window) -> Option<Snapshot> {
        let buf = window.take_snapshot().ok()?;
        if buf.width() == 0 || buf.height() == 0 {
            return None;
        }
        Some(Snapshot { buf })
    }

    /// The whole page, frosted.
    pub fn frosted(&self) -> Image {
        let (w, h) = (self.buf.width(), self.buf.height());
        let (sw, sh) = ((w / SHRINK).max(1), (h / SHRINK).max(1));
        let src = self.buf.as_slice();
        let mut small = vec![Rgba8Pixel::default(); (sw * sh) as usize];
        for y in 0..sh {
            for x in 0..sw {
                // average the SHRINK×SHRINK block
                let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
                for dy in 0..SHRINK {
                    for dx in 0..SHRINK {
                        let (px, py) = (x * SHRINK + dx, y * SHRINK + dy);
                        if px < w && py < h {
                            let p = src[(py * w + px) as usize];
                            r += p.r as u32;
                            g += p.g as u32;
                            b += p.b as u32;
                            n += 1;
                        }
                    }
                }
                let n = n.max(1);
                small[(y * sw + x) as usize] = Rgba8Pixel {
                    r: (r / n) as u8,
                    g: (g / n) as u8,
                    b: (b / n) as u8,
                    a: 255,
                };
            }
        }
        let mut tmp = small.clone();
        for _ in 0..2 {
            box_blur_h(&small, &mut tmp, sw as usize, sh as usize, RADIUS);
            box_blur_v(&tmp, &mut small, sw as usize, sh as usize, RADIUS);
        }
        let mut out = SharedPixelBuffer::<Rgba8Pixel>::new(sw, sh);
        out.make_mut_slice().copy_from_slice(&small);
        Image::from_rgba8(out)
    }

    /// A crop at full resolution: the pressed bubble as it was drawn.
    pub fn crop(&self, x: f32, y: f32, w: f32, h: f32) -> Option<Image> {
        let (bw, bh) = (self.buf.width() as i64, self.buf.height() as i64);
        let x0 = (x.floor() as i64).clamp(0, bw);
        let y0 = (y.floor() as i64).clamp(0, bh);
        let x1 = ((x + w).ceil() as i64).clamp(0, bw);
        let y1 = ((y + h).ceil() as i64).clamp(0, bh);
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let (cw, ch) = ((x1 - x0) as u32, (y1 - y0) as u32);
        let mut out = SharedPixelBuffer::<Rgba8Pixel>::new(cw, ch);
        let src = self.buf.as_slice();
        let dst = out.make_mut_slice();
        for row in 0..ch as i64 {
            let s = ((y0 + row) * bw + x0) as usize;
            let d = (row as u32 * cw) as usize;
            dst[d..d + cw as usize].copy_from_slice(&src[s..s + cw as usize]);
        }
        Some(Image::from_rgba8(out))
    }
}

fn box_blur_h(src: &[Rgba8Pixel], dst: &mut [Rgba8Pixel], w: usize, h: usize, r: i32) {
    for y in 0..h {
        let row = &src[y * w..(y + 1) * w];
        for x in 0..w {
            let (mut rr, mut gg, mut bb, mut n) = (0u32, 0u32, 0u32, 0u32);
            for dx in -r..=r {
                let xx = x as i32 + dx;
                if xx >= 0 && (xx as usize) < w {
                    let p = row[xx as usize];
                    rr += p.r as u32;
                    gg += p.g as u32;
                    bb += p.b as u32;
                    n += 1;
                }
            }
            dst[y * w + x] = Rgba8Pixel {
                r: (rr / n) as u8,
                g: (gg / n) as u8,
                b: (bb / n) as u8,
                a: 255,
            };
        }
    }
}

fn box_blur_v(src: &[Rgba8Pixel], dst: &mut [Rgba8Pixel], w: usize, h: usize, r: i32) {
    for x in 0..w {
        for y in 0..h {
            let (mut rr, mut gg, mut bb, mut n) = (0u32, 0u32, 0u32, 0u32);
            for dy in -r..=r {
                let yy = y as i32 + dy;
                if yy >= 0 && (yy as usize) < h {
                    let p = src[yy as usize * w + x];
                    rr += p.r as u32;
                    gg += p.g as u32;
                    bb += p.b as u32;
                    n += 1;
                }
            }
            dst[y * w + x] = Rgba8Pixel {
                r: (rr / n) as u8,
                g: (gg / n) as u8,
                b: (bb / n) as u8,
                a: 255,
            };
        }
    }
}
