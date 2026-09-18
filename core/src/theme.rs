fn mix(a: u32, b: u32, amount: u32) -> u32 {
    [16, 8, 0].into_iter().fold(0, |rgb, shift| {
        rgb | (((((a >> shift) & 255) * (100 - amount) + ((b >> shift) & 255) * amount) / 100)
            << shift)
    })
}

fn luminance(rgb: u32) -> f64 {
    [16, 8, 0]
        .into_iter()
        .zip([0.2126, 0.7152, 0.0722])
        .map(|(s, w)| {
            let c = f64::from((rgb >> s) & 255) / 255.0;
            w * if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        })
        .sum()
}

fn contrast(a: u32, b: u32) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn ink(surface: u32) -> u32 {
    if contrast(surface, 0x000000) >= 4.5 {
        0x000000
    } else {
        0xffffff
    }
}

// background, ink, surface, ink, accent, ink, tint, ink, outline, outgoing, ink.
/// Neutral anchors take a hint of the accent; the sent bubble is the accent itself, deepened
/// until white reads on it, and every other tone stays where it sat whatever hue is chosen.
pub fn palette(accent: u32, dark: bool) -> [u32; 11] {
    let accent = accent & 0xffffff;
    let base = if dark { 0x0f0f0f } else { 0xfafafa };
    let background = mix(base, accent, 3);
    let surface = mix(if dark { 0x1e1e1e } else { 0xeeeeee }, accent, if dark { 5 } else { 3 });
    let tint = mix(base, accent, if dark { 35 } else { 19 });
    let mut outgoing = mix(accent, if dark { 0x1e1e1e } else { 0x4a4a4a }, if dark { 55 } else { 50 });
    for _ in 0..100 {
        if contrast(outgoing, 0xffffff) >= 4.5 {
            break;
        }
        outgoing = mix(outgoing, 0x000000, 5);
    }
    let mut primary = accent;
    let target = if dark { 0xffffff } else { 0x000000 };
    for _ in 0..100 {
        if contrast(primary, background) >= 4.5 && contrast(primary, surface) >= 4.5 {
            break;
        }
        primary = mix(primary, target, 10);
    }
    let outline = mix(base, ink(base), 55);
    [
        background,
        ink(background),
        surface,
        ink(surface),
        primary,
        ink(primary),
        tint,
        ink(tint),
        outline,
        outgoing,
        0xffffff,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn derived_palettes_keep_readable_roles_across_rgb_cube() {
        for dark in [false, true] {
            for r in (0..=255).step_by(17) {
                for g in (0..=255).step_by(17) {
                    for b in (0..=255).step_by(17) {
                        let p = palette(r << 16 | g << 8 | b, dark);
                        for (fg, bg) in [(1, 0), (3, 2), (5, 4), (7, 6), (10, 9), (4, 0), (4, 2)] {
                            assert!(contrast(p[fg], p[bg]) >= 4.5, "{p:x?}");
                        }
                        assert!(contrast(p[8], p[0]) >= 3.0);
                    }
                }
            }
        }
    }
    #[test]
    fn default_is_achromatic() {
        for dark in [false, true] {
            for c in palette(0x555555, dark) {
                assert_eq!(c & 255, (c >> 8) & 255);
                assert_eq!(c & 255, (c >> 16) & 255);
            }
        }
    }
}
