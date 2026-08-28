//! Cover art and duration for a music file. A voice note carries its own MSC3245 waveform;
//! a track carries neither, so both are worked out here once and cached against the event.

use std::path::{Path, PathBuf};


#[derive(Debug, Default, Clone)]
pub struct Track {
    pub art: Option<PathBuf>,
    /// Accent colour taken from the art; empty when there is none.
    pub accent: String,
    pub duration_ms: u64,
}

/// Everything the player needs about one file. No waveform: nothing draws one for a track.
pub fn analyse(file: &Path, art_out: &Path) -> Track {
    let probe = super::av::probe(file);
    let art = super::av::cover(file, (600, 600), art_out);
    Track {
        accent: art.as_deref().and_then(accent_of).unwrap_or_default(),
        art,
        duration_ms: probe.map(|p| p.duration_ms).unwrap_or(0),
    }
}

/// Accent colour for the chrome under the art: the most saturated pixel family, not the
/// average (which comes out as mud); the average only for genuinely greyscale art.
pub fn accent_of(art: &Path) -> Option<String> {
    let img = image::open(art).ok()?;
    // Small enough to be free, large enough to keep the picture's colours.
    let small = img.thumbnail(48, 48).to_rgb8();
    let mut best: Option<([u8; 3], f32)> = None;
    let (mut sr, mut sg, mut sb, mut n) = (0u64, 0u64, 0u64, 0u64);
    for px in small.pixels() {
        let [r, g, b] = px.0;
        sr += r as u64; sg += g as u64; sb += b as u64; n += 1;
        let (max, min) = (r.max(g).max(b) as f32, r.min(g).min(b) as f32);
        if max <= 0.0 { continue }
        let sat = (max - min) / max;
        let val = max / 255.0;
        // Prefer real colour, neither so dark nor so bright that white text cannot sit on it.
        let score = sat * (1.0 - (val - 0.55).abs());
        if best.map(|(_, s)| score > s).unwrap_or(true) { best = Some(([r, g, b], score)) }
    }
    let [r, g, b] = match best {
        Some((c, s)) if s > 0.08 => c,
        _ if n > 0 => [(sr / n) as u8, (sg / n) as u8, (sb / n) as u8],
        _ => return None,
    };
    Some(format!("#{r:02x}{g:02x}{b:02x}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_accent_prefers_colour_over_the_average() {
        let dir = std::env::temp_dir().join(format!("sigil-audio-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Mostly black with a patch of orange: the average would be near-black.
        let mut img = image::RgbImage::new(40, 40);
        for y in 0..12 { for x in 0..12 { img.put_pixel(x, y, image::Rgb([230, 120, 40])) } }
        let p = dir.join("art.png");
        img.save(&p).unwrap();
        let hex = accent_of(&p).expect("an image has a colour");
        let r = u8::from_str_radix(&hex[1..3], 16).unwrap();
        let b = u8::from_str_radix(&hex[5..7], 16).unwrap();
        assert!(r > b, "warm, not muddy: {hex}");
        assert!(r > 150, "not the near-black average: {hex}");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn greyscale_art_still_gets_a_colour() {
        let dir = std::env::temp_dir().join(format!("sigil-audio-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut img = image::RgbImage::new(20, 20);
        for px in img.pixels_mut() { *px = image::Rgb([90, 90, 90]) }
        let p = dir.join("grey.png");
        img.save(&p).unwrap();
        // Near the source value, not an exact hex: the downscale resamples.
        let hex = accent_of(&p).expect("even a flat grey has a colour");
        let c: Vec<u8> = (0..3).map(|i| u8::from_str_radix(&hex[1 + i * 2..3 + i * 2], 16).unwrap()).collect();
        assert!(c.iter().all(|v| v.abs_diff(90) <= 3), "close to the source grey: {hex}");
        assert!(c[0].abs_diff(c[1]) <= 1 && c[1].abs_diff(c[2]) <= 1, "still neutral: {hex}");
        let _ = std::fs::remove_file(&p);
    }
}
