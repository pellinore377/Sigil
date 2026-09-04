//! The frosted page behind the message sheet, done the way the QML did it:
//! a snapshot of what is on screen, blurred. Slint has no backdrop blur, so
//! the window is captured (`take_snapshot`), the pressed bubble's rect
//! painted over with the ground around it, then shrunk, box-blurred twice
//! (which lands close to a Gaussian at the radius MultiEffect used), and
//! handed back as an image the sheet stretches over the page. The pressed
//! bubble's copy is a crop of a snapshot at full size, cut to its corners.

use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

/// A rect in the snapshot's pixels with its four corner radii (tl, tr, bl, br).
#[derive(Clone, Copy, Debug, Default)]
pub struct PixelRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub radii: [f32; 4],
}

impl PixelRect {
    fn bounds(&self, bw: u32, bh: u32) -> Option<(u32, u32, u32, u32)> {
        let (bw, bh) = (bw as i64, bh as i64);
        let x0 = (self.x.floor() as i64).clamp(0, bw);
        let y0 = (self.y.floor() as i64).clamp(0, bh);
        let x1 = ((self.x + self.w).ceil() as i64).clamp(0, bw);
        let y1 = ((self.y + self.h).ceil() as i64).clamp(0, bh);
        (x1 > x0 && y1 > y0).then(|| (x0 as u32, y0 as u32, x1 as u32, y1 as u32))
    }
}

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

    /// Paint over `r` with the ground around it (the mean of a 2px ring just
    /// outside), so the frost has a hole where the bubble stood without the
    /// page having to hide it first. Done before the blur, which smooths
    /// the seam away.
    pub fn mask(&mut self, r: PixelRect) {
        let (bw, bh) = (self.buf.width(), self.buf.height());
        let Some((x0, y0, x1, y1)) = r.bounds(bw, bh) else { return };
        let px = self.buf.make_mut_slice();
        let (mut sr, mut sg, mut sb, mut n) = (0u64, 0u64, 0u64, 0u64);
        let (rx0, ry0) = (x0.saturating_sub(2), y0.saturating_sub(2));
        let (rx1, ry1) = ((x1 + 2).min(bw), (y1 + 2).min(bh));
        for y in ry0..ry1 {
            for x in rx0..rx1 {
                if x >= x0 && x < x1 && y >= y0 && y < y1 {
                    continue;
                }
                let p = px[(y * bw + x) as usize];
                sr += p.r as u64;
                sg += p.g as u64;
                sb += p.b as u64;
                n += 1;
            }
        }
        if n == 0 {
            return;
        }
        let fill = Rgba8Pixel { r: (sr / n) as u8, g: (sg / n) as u8, b: (sb / n) as u8, a: 255 };
        for y in y0..y1 {
            px[(y * bw + x0) as usize..(y * bw + x1) as usize].fill(fill);
        }
    }

    /// A crop at full resolution: the pressed bubble as it was drawn, its
    /// corners cut to the bubble's radii (a pixel's coverage by the corner
    /// circle becomes its alpha, so the edge is smooth on any renderer).
    pub fn crop(&self, r: PixelRect) -> Option<Image> {
        let (bw, bh) = (self.buf.width(), self.buf.height());
        let (x0, y0, x1, y1) = r.bounds(bw, bh)?;
        let (cw, ch) = (x1 - x0, y1 - y0);
        let mut out = SharedPixelBuffer::<Rgba8Pixel>::new(cw, ch);
        let src = self.buf.as_slice();
        let dst = out.make_mut_slice();
        for row in 0..ch {
            let s = ((y0 + row) * bw + x0) as usize;
            let d = (row * cw) as usize;
            dst[d..d + cw as usize].copy_from_slice(&src[s..s + cw as usize]);
        }
        let (fw, fh) = (cw as f32, ch as f32);
        // corner centres: tl, tr, bl, br
        let corners = [
            (r.radii[0], r.radii[0], r.radii[0]),
            (fw - r.radii[1], r.radii[1], r.radii[1]),
            (r.radii[2], fh - r.radii[2], r.radii[2]),
            (fw - r.radii[3], fh - r.radii[3], r.radii[3]),
        ];
        for (cx, cy, rad) in corners {
            if rad <= 0.5 {
                continue;
            }
            let (xa, xb) = if cx < fw / 2.0 { (0.0, cx) } else { (cx, fw) };
            let (ya, yb) = if cy < fh / 2.0 { (0.0, cy) } else { (cy, fh) };
            for y in (ya.floor() as u32)..(yb.ceil() as u32).min(ch) {
                for x in (xa.floor() as u32)..(xb.ceil() as u32).min(cw) {
                    let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                    let cover = (rad - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0);
                    if cover < 1.0 {
                        let p = &mut dst[(y * cw + x) as usize];
                        p.a = (p.a as f32 * cover).round() as u8;
                    }
                }
            }
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
