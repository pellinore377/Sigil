//! How SigilText animations move.
//!
//! Timings, easings and displacements are part of the format, like the palette.
//! A frontend that picked its own would make the same message feel different
//! per platform, and unlike a colour that drift is invisible in a screenshot.
//!
//! Easings are cubic Bézier control points, not named curves: `InOutSine` in Qt,
//! `FastOutSlowInEasing` in Compose and `ease-in-out` in CSS are three different
//! shapes. Control points are the only encoding all three read the same way.

use serde_json::{json, Value};

/// Per-character start offset, in ms, before each effect's own reduction.
pub const STAGGER_MS: u32 = 90;

/// Cubic Bézier control points, the CSS `cubic-bezier(x1, y1, x2, y2)` order.
#[derive(Debug, Clone, Copy)]
pub struct Easing(pub f64, pub f64, pub f64, pub f64);

pub const LINEAR: Easing = Easing(0.0, 0.0, 1.0, 1.0);
/// Closest Bézier to Qt's `InOutSine`.
pub const IN_OUT_SINE: Easing = Easing(0.37, 0.0, 0.63, 1.0);
/// Closest Bézier to Qt's `InOutQuad`.
pub const IN_OUT_QUAD: Easing = Easing(0.45, 0.0, 0.55, 1.0);
/// Closest Bézier to Qt's `OutCubic`.
pub const OUT_CUBIC: Easing = Easing(0.33, 1.0, 0.68, 1.0);

impl Easing {
    fn json(&self) -> Value { json!([self.0, self.1, self.2, self.3]) }
}

/// One animation's full specification. `params` carries what only that
/// animation means; everything reading this should treat unknown keys as
/// additive rather than erroring.
fn spec(name: &str, steps: &[u32], easing: Easing, stagger_mod: Option<u32>, params: Value) -> Value {
    json!({
        "name": name,
        "steps": steps,
        "durationMs": steps.iter().sum::<u32>(),
        "easing": easing.json(),
        "staggerMs": STAGGER_MS,
        "staggerModulo": stagger_mod,
        "params": params,
    })
}

/// Every animation, as the frontends should drive them.
pub fn all() -> Value {
    json!({
        "staggerMs": STAGGER_MS,
        "animations": [
            spec("shake", &[80, 80, 80], LINEAR, Some(160),
                 json!({ "axis": "x", "amplitudePx": 0.8 })),
            spec("wave", &[520, 520, 260], IN_OUT_SINE, None,
                 json!({ "axis": "y", "amplitudePx": 1.8 })),
            spec("pulse", &[500, 500], IN_OUT_QUAD, Some(300),
                 json!({ "minScale": 1.0, "maxScale": 1.18 })),
            spec("glow", &[1600], IN_OUT_QUAD, None,
                 json!({ "minAlpha": 0.0, "maxAlpha": 1.0 })),
            spec("sparkle", &[900], IN_OUT_SINE, None,
                 json!({ "particles": 3, "minScale": 1.1, "maxScale": 1.5,
                         "minAlpha": 0.05, "maxAlpha": 0.45 })),
            spec("glitch", &[90, 70, 110, 260], LINEAR, Some(400),
                 json!({ "axis": "x", "leadPx": -2.0, "trailPx": 1.5,
                         "staggerStride": 53,
                         "leadRgb": "#FF2640", "trailRgb": "#26F2FF", "splitAlpha": 0.7 })),
            spec("typewriter", &[620], OUT_CUBIC, None,
                 json!({ "reveal": "character" })),
            spec("flip", &[0], LINEAR, None,
                 json!({ "rotationDeg": 180.0, "reverseRun": true })),
            spec("barrel", &[1200], LINEAR, None,
                 json!({ "rotationDeg": 360.0, "continuous": true })),
            spec("blur", &[900], IN_OUT_QUAD, None,
                 json!({ "maxRadiusPx": 6.0, "revealOn": "hover" })),
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every animation the parser accepts must have a spec, or a frontend will
    /// meet a name it has no numbers for and invent some.
    #[test]
    fn every_parsed_animation_has_a_spec() {
        let v = all();
        let specced: Vec<String> = v["animations"].as_array().unwrap().iter()
            .map(|a| a["name"].as_str().unwrap().to_string()).collect();
        for name in ["shake", "wave", "pulse", "glow", "typewriter",
                     "sparkle", "glitch", "blur", "flip", "barrel"] {
            assert!(specced.contains(&name.to_string()), "{name} has no motion spec");
        }
        assert_eq!(specced.len(), 10, "an animation was added without a spec");
    }

    #[test]
    fn durations_are_the_sum_of_their_steps() {
        for a in all()["animations"].as_array().unwrap() {
            let steps: u32 = a["steps"].as_array().unwrap().iter()
                .map(|s| s.as_u64().unwrap() as u32).sum();
            assert_eq!(a["durationMs"].as_u64().unwrap() as u32, steps, "{}", a["name"]);
        }
    }

    #[test]
    fn easings_are_control_points_not_names() {
        for a in all()["animations"].as_array().unwrap() {
            let e = a["easing"].as_array().expect("easing must be an array");
            assert_eq!(e.len(), 4, "{}: cubic bezier takes 4 control points", a["name"]);
            assert!(e.iter().all(|c| c.as_f64().is_some()));
        }
    }
}
