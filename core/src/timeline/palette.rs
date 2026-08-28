//! The SigilText palette: what `red`, `big2` and `rainbow` actually look like.
//!
//! This is part of the format, not of any one frontend. A client that picked its
//! own values would render the same message differently, which would make
//! SigilText a suggestion rather than a standard — so the values live here and
//! ship resolved in the effect spans. Frontends draw what they are handed.

/// Base RGB for each hue, i.e. the `2` (mid) variant.
const HUES: &[(&str, [f64; 3])] = &[
    ("red",    [0.85, 0.28, 0.26]),
    ("orange", [0.88, 0.52, 0.22]),
    ("yellow", [0.87, 0.74, 0.25]),
    ("green",  [0.40, 0.72, 0.36]),
    ("cyan",   [0.30, 0.74, 0.76]),
    ("blue",   [0.33, 0.55, 0.86]),
    ("purple", [0.60, 0.44, 0.82]),
    ("pink",   [0.88, 0.45, 0.68]),
    ("gray",   [0.60, 0.60, 0.62]),
];

/// Level 1 is the lightest variant, 3 the darkest, by scaling the HSV value.
const LIGHTEN: f64 = 1.35;
const DARKEN: f64 = 1.45;

/// The base hues are tuned against a dark ground. On a light ground the same
/// value is too pale to read, so it is scaled down and saturated slightly —
/// derived rather than hand-typed so the two sets cannot drift apart.
const LIGHT_GROUND_VALUE: f64 = 0.72;
const LIGHT_GROUND_SATURATION: f64 = 1.12;

/// Which ground the text is drawn on. The frontend knows this; the engine does
/// not, so both are shipped and the frontend picks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ground {
    Dark,
    Light,
}

/// `rainbow` cycles hue across the run at fixed saturation and lightness.
pub const RAINBOW_SATURATION: f64 = 0.62;
pub const RAINBOW_LIGHTNESS: f64 = 0.62;

/// The highlight behind `mark` when the span sets no colour of its own.
pub const MARK_DEFAULT: &str = "yellow2";

/// `small3`..`big3` as a font-size multiplier, indexed by step -3..=3.
const SIZE_SCALE: [f64; 7] = [0.7, 0.8, 0.9, 1.0, 1.2, 1.4, 1.6];

pub fn size_scale(step: i8) -> f64 {
    SIZE_SCALE[(step.clamp(-3, 3) + 3) as usize]
}

fn scale_value(rgb: [f64; 3], factor: f64) -> [f64; 3] {
    let (r, g, b) = (rgb[0], rgb[1], rgb[2]);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let v = (max * factor).clamp(0.0, 1.0);
    if max <= 0.0 {
        return [v, v, v];
    }
    // Preserve hue and saturation: scale the value, keep chroma proportional.
    let s = (max - min) / max;
    let new_min = v * (1.0 - s);
    let span = max - min;
    if span <= 0.0 {
        return [v, v, v];
    }
    let lerp = |c: f64| new_min + (c - min) / span * (v - new_min);
    [lerp(r), lerp(g), lerp(b)]
}

fn hex(rgb: [f64; 3]) -> String {
    let ch = |c: f64| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02X}{:02X}{:02X}", ch(rgb[0]), ch(rgb[1]), ch(rgb[2]))
}

fn saturate(rgb: [f64; 3], factor: f64) -> [f64; 3] {
    let max = rgb[0].max(rgb[1]).max(rgb[2]);
    [0, 1, 2].map(|i| (max - (max - rgb[i]) * factor).clamp(0.0, 1.0))
}

/// `red`, `red1`, `gray3` → `#RRGGBB` for one ground. Unknown names give `None`
/// so the caller can fall back to the surrounding text colour.
pub fn resolve_on(name: &str, ground: Ground) -> Option<String> {
    let (base_name, level) = match name.chars().last() {
        Some(c @ '1'..='3') => (&name[..name.len() - 1], c.to_digit(10).unwrap() as u8),
        _ => (name, 2),
    };
    let mut base = HUES.iter().find(|(n, _)| *n == base_name)?.1;
    if ground == Ground::Light {
        base = saturate(scale_value(base, LIGHT_GROUND_VALUE), LIGHT_GROUND_SATURATION);
    }
    Some(hex(match level {
        1 => scale_value(base, LIGHTEN),
        3 => scale_value(base, 1.0 / DARKEN),
        _ => base,
    }))
}

/// Both grounds, as the wire carries them.
pub fn resolve(name: &str) -> Option<(String, String)> {
    Some((resolve_on(name, Ground::Dark)?, resolve_on(name, Ground::Light)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_hue_resolves_at_every_level() {
        for (name, _) in HUES {
            for suffix in ["", "1", "2", "3"] {
                let n = format!("{name}{suffix}");
                assert!(resolve(&n).is_some(), "{n} did not resolve");
            }
        }
    }

    #[test]
    fn a_bare_name_is_the_mid_variant() {
        assert_eq!(resolve("red"), resolve("red2"));
    }

    fn lum(h: &str) -> u32 {
        (1..7).step_by(2).map(|i| u32::from_str_radix(&h[i..i + 2], 16).unwrap()).sum()
    }

    #[test]
    fn one_is_lightest_and_three_is_darkest() {
        for g in [Ground::Dark, Ground::Light] {
            let l = |n: &str| lum(&resolve_on(n, g).unwrap());
            assert!(l("green1") > l("green2"), "{g:?}: 1 must be lighter than 2");
            assert!(l("green3") < l("green2"), "{g:?}: 3 must be darker than 2");
        }
    }

    #[test]
    fn the_light_ground_set_is_darker_than_the_dark_ground_set() {
        for h in ["red", "green", "blue", "yellow"] {
            let d = lum(&resolve_on(h, Ground::Dark).unwrap());
            let l = lum(&resolve_on(h, Ground::Light).unwrap());
            assert!(l < d, "{h}: light-ground {l} should be darker than dark-ground {d}");
        }
    }

    #[test]
    fn an_unknown_name_does_not_resolve() {
        assert_eq!(resolve("chartreuse"), None);
        assert_eq!(resolve(""), None);
    }

    #[test]
    fn size_steps_run_from_dot_seven_to_one_point_six() {
        assert_eq!(size_scale(-3), 0.7);
        assert_eq!(size_scale(0), 1.0);
        assert_eq!(size_scale(3), 1.6);
        assert_eq!(size_scale(-9), 0.7, "out of range clamps");
        assert_eq!(size_scale(9), 1.6);
    }
}


