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
/// Dark anchors are fixed neutral steps, tinted toward the accent rather than mixed
/// from it, so the ramp keeps its spacing whatever hue is chosen.
pub fn palette(accent: u32, dark: bool) -> [u32; 11] {
    let base = if dark { 0x131313 } else { 0xfcfcfc };
    let surface = mix(if dark { 0x1a1a1a } else { 0xf1f1f1 }, accent, if dark { 8 } else { 5 });
    let background = mix(base, accent, 3);
    // The tonal container every secondary ink is measured against. It stays on the same
    // side of the ground it always sat on; only the bubble below moves.
    let tint = mix(base, accent, if dark { 35 } else { 19 });
    // The outgoing bubble steps away from the ground: lighter in dark, darker in light,
    // so its own ink flips with it.
    let outgoing = mix(if dark { 0x474747 } else { 0x5e5e5e }, accent, 14);
    let mut primary = accent & 0xffffff;
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
        ink(outgoing),
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
