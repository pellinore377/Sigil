use crate::Animation;

/// Version-1 rendering parameters: distances are thousandths of an em, angles degrees.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Parameters {
    pub duration_ms: u16,
    pub easing: [u16; 4],
    pub cycles: u8,
    pub displacement: u16,
    pub rotation: u16,
    pub scale_per_mille: u16,
    pub stagger_ms: u16,
    pub particles: u8,
    pub particle_lifetime_ms: u16,
    pub spring_stiffness: u16,
    pub spring_damping: u16,
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
                // Keep joined scripts and emoji intact; barrel moves complete words.
                if word.is_ascii() && animation != Animation::Barrel {
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
    runs
}
impl Animation {
    pub fn parameters(self) -> Parameters {
        let mut p = Parameters {
            duration_ms: 900,
            easing: [220, 0, 200, 1000],
            cycles: 1,
            displacement: 0,
            rotation: 0,
            scale_per_mille: 1000,
            stagger_ms: 0,
            particles: 0,
            particle_lifetime_ms: 0,
            spring_stiffness: 180,
            spring_damping: 24,
            substitutions: "",
        };
        match self {
            Self::Shake => {
                p.duration_ms = 480;
                p.cycles = 4;
                p.displacement = 80;
            }
            Self::Wave => {
                p.duration_ms = 1200;
                p.displacement = 140;
                p.stagger_ms = 45;
            }
            Self::Pulse => {
                p.cycles = 2;
                p.scale_per_mille = 1080;
            }
            Self::Glow => {
                p.duration_ms = 1200;
                p.displacement = 180;
            }
            Self::Typewriter => {
                p.duration_ms = 1600;
                p.stagger_ms = 35;
            }
            Self::Sparkle => {
                p.duration_ms = 1100;
                p.particles = 12;
                p.particle_lifetime_ms = 650;
                p.displacement = 600;
            }
            Self::Glitch => {
                p.duration_ms = 420;
                p.cycles = 3;
                p.displacement = 60;
                p.substitutions = "#%&?<>[]";
            }
            Self::Scatter => {
                p.duration_ms = 1000;
                p.displacement = 750;
                p.rotation = 25;
            }
            Self::Flip => {
                p.duration_ms = 700;
                p.rotation = 180;
                p.stagger_ms = 40;
            }
            Self::Barrel => {
                p.duration_ms = 900;
                p.rotation = 360;
                p.displacement = 600;
            }
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
        let barrel = crate::parse("barrel::whole words;", Default::default()).unwrap();
        assert_eq!(barrel.presentation().motion[0].units, vec![[0, 5], [6, 11]]);
    }
}
