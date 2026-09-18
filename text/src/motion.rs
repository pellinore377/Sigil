use crate::Animation;

/// Version-1 rendering parameters, ported from the selected presentation recipes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Parameters {
    pub duration_ms: u16,
    pub mode: &'static str,
    /// Amplitude x1000: pixels against a 64px reference em, or a unitless ratio for pulse.
    pub amplitude_milli: u32,
    pub stagger_ms: u16,
    pub particles: u8,
    /// Shake and pulse move the whole run; the rest move glyph by glyph.
    pub line_scope: bool,
    pub caret: &'static str,
    pub substitutions: &'static str,
}

#[derive(serde::Serialize)]
pub struct Run {
    pub animation: Animation,
    pub parameters: Parameters,
    pub units: Vec<[u32; 2]>,
}

pub(crate) fn presentation(text: &crate::Text) -> Vec<Run> {
    use unicode_segmentation::UnicodeSegmentation;
    let graphemes: Vec<_> = text.body().graphemes(true).collect();
    let mut offsets = vec![0];
    for g in &graphemes {
        offsets.push(offsets.last().unwrap() + g.encode_utf16().count() as u32);
    }
    let mut budget = 192;
    let mut runs: Vec<Run> = Vec::new();
    for run in text.spans().iter().filter_map(|span| {
        let animation = span.effects.animation?;
        if budget == 0 || span.effects.reveal.is_some() {
            return None;
        }
        let source = graphemes[span.start as usize..span.end as usize].concat();
        let mut at = offsets[span.start as usize];
        let mut units = Vec::new();
        for word in source.split_word_bounds() {
            let end = at + word.encode_utf16().count() as u32;
            if !word.chars().all(char::is_whitespace) {
                // Keep joined scripts and emoji intact; everything else moves grapheme by grapheme.
                if word.is_ascii() {
                    units.extend((at..end).map(|i| [i, i + 1]));
                } else {
                    units.push([at, end]);
                }
            }
            at = end;
            if units.len() > budget {
                break;
            }
        }
        if units.len() > budget {
            return None;
        }
        if units.is_empty() {
            return None;
        }
        budget -= units.len();
        Some(Run {
            animation,
            parameters: animation.parameters(),
            units,
        })
    }) {
        if let Some(previous) = runs.iter_mut().find(|r| r.animation == run.animation) {
            previous.units.extend(run.units);
        } else {
            runs.push(run);
        }
    }
    // The staggered tail extends the timeline, exactly as the reference player measures it.
    for run in &mut runs {
        let n = run.units.len().saturating_sub(1) as u32;
        let tail = (n * u32::from(run.parameters.stagger_ms)).min(640);
        run.parameters.duration_ms = run.parameters.duration_ms.saturating_add(tail as u16);
    }
    runs
}
impl Animation {
    /// Flip is a static text transform and carries no timeline.
    pub fn parameters(self) -> Parameters {
        let mut p = Parameters {
            duration_ms: 0,
            mode: "",
            amplitude_milli: 0,
            stagger_ms: 0,
            particles: 0,
            line_scope: false,
            caret: "",
            substitutions: "",
        };
        match self {
            Self::Shake => {
                p.duration_ms = 1550;
                p.mode = "echo";
                p.amplitude_milli = 5_000;
                p.line_scope = true;
            }
            Self::Wave => {
                p.duration_ms = 1300;
                p.mode = "ribbon";
                p.amplitude_milli = 15_000;
                p.stagger_ms = 28;
            }
            Self::Pulse => {
                p.duration_ms = 1250;
                p.mode = "elastic";
                p.amplitude_milli = 100;
                p.line_scope = true;
            }
            Self::Glow => {
                p.duration_ms = 1300;
                p.mode = "travel";
                p.amplitude_milli = 24_000;
                p.stagger_ms = 48;
            }
            Self::Typewriter => {
                p.duration_ms = 1950;
                p.mode = "measured";
                p.caret = "line";
            }
            Self::Sparkle => {
                p.duration_ms = 1900;
                p.mode = "constellation";
                p.amplitude_milli = 11_000;
                p.particles = 9;
            }
            Self::Glitch => {
                p.duration_ms = 1700;
                p.mode = "fragment";
                p.amplitude_milli = 4_000;
                p.substitutions = "\u{2301}\u{2591}/\u{00a6}\u{2237}\u{2310}\u{2260}\u{2592}";
            }
            Self::Scatter => {
                p.duration_ms = 1800;
                p.mode = "burst";
                p.amplitude_milli = 94_000;
            }
            Self::Assemble => {
                p.duration_ms = 1600;
                p.mode = "sort";
                p.amplitude_milli = 74_000;
                p.stagger_ms = 35;
            }
            Self::Barrel => {
                p.duration_ms = 1200;
                p.mode = "ripple";
                p.amplitude_milli = 27_000;
                p.stagger_ms = 58;
            }
            // Static: the renderer turns each grapheme in place, over reversed text.
            Self::Flip => p.mode = "plain",
        }
        p
    }
}

/// Stable per-message/per-glyph randomness; renderers must not seed from wall time.
pub fn seed(message: &[u8; 32], glyph: u32, sample: u32) -> u64 {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(
        [
            b"Sigil/text-motion/v1".as_slice(),
            message,
            &glyph.to_be_bytes(),
            &sample.to_be_bytes(),
        ]
        .concat(),
    );
    u64::from_be_bytes(digest[..8].try_into().expect("SHA-256 prefix"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn presentation_preserves_words_emoji_and_secrets_with_bounded_work() {
        let text = crate::parse(
            "wave::office العربية 👩🏽‍💻 café; spoiler::shake::hidden;",
            Default::default(),
        )
        .unwrap();
        let wire = text.to_bytes().unwrap();
        let view = text.presentation();
        assert_eq!(view.motion.len(), 1);
        let utf16: Vec<_> = text.body().encode_utf16().collect();
        let words: Vec<_> = view.motion[0]
            .units
            .iter()
            .map(|r| String::from_utf16(&utf16[r[0] as usize..r[1] as usize]).unwrap())
            .collect();
        assert_eq!(&words[..6], ["o", "f", "f", "i", "c", "e"]);
        for word in ["العربية", "👩🏽‍💻", "café"] {
            assert!(words.iter().any(|v| v == word), "{words:?}");
        }
        assert!(!words.iter().any(|v| v.contains("hidden")));
        assert_eq!(wire, text.to_bytes().unwrap());
        let text = crate::parse(
            &format!("wave::{};", "word ".repeat(1000)),
            Default::default(),
        )
        .unwrap();
        let view = text.presentation();
        assert!(view.motion.iter().map(|r| r.units.len()).sum::<usize>() <= 192);
        assert!(view.motion.is_empty());
        let barrel = crate::parse("barrel::ab cd;", Default::default()).unwrap();
        assert_eq!(
            barrel.presentation().motion[0].units,
            vec![[0, 1], [1, 2], [3, 4], [4, 5]]
        );
        let flip = crate::parse("flip::abc;", Default::default()).unwrap();
        let run = &flip.presentation().motion[0];
        assert_eq!(run.parameters.duration_ms, 0);
        assert_eq!(run.units, vec![[0, 1], [1, 2], [2, 3]]);
        let assemble = crate::parse("assemble::ab;", Default::default()).unwrap();
        let run = &assemble.presentation().motion[0];
        assert_eq!(run.animation, crate::Animation::Assemble);
        assert_eq!(run.units, vec![[0, 1], [1, 2]]);
        assert!(run.parameters.amplitude_milli > 0 && run.parameters.stagger_ms > 0);
        // duration carries the staggered tail: 1600 + min(640, 1 * 35)
        assert_eq!(run.parameters.duration_ms, 1635);
    }
}
