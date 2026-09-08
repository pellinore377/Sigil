use crate::Animation;

/// Version-1 rendering parameters: distances are thousandths of an em, angles degrees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
                p.rotation = 360;
                p.stagger_ms = 40;
            }
            Self::Barrel => {
                p.duration_ms = 900;
                p.rotation = 360;
            }
        }
        p
    }
}

/// Retain per-message playback state across recomposition; clocks use monotonic milliseconds.
pub struct Playback {
    elapsed: u64,
    last_tick: Option<u64>,
    running: bool,
    finished: bool,
}
impl Playback {
    pub fn new(new_content: bool) -> Self {
        Self {
            elapsed: 0,
            last_tick: None,
            running: false,
            finished: !new_content,
        }
    }
    pub fn replay(&mut self) {
        self.elapsed = 0;
        self.last_tick = None;
        self.running = false;
        self.finished = false;
    }
    /// Returns normalized progress, with 1000 the static final appearance.
    pub fn tick(
        &mut self,
        now: u64,
        visible: bool,
        reduced_motion: bool,
        animation: Animation,
    ) -> u16 {
        if reduced_motion {
            self.finished = true;
        }
        if self.finished {
            return 1000;
        }
        if self.running {
            self.elapsed = self
                .elapsed
                .saturating_add(now.saturating_sub(self.last_tick.unwrap_or(now)));
        }
        self.last_tick = Some(now);
        self.running = visible;
        let duration = u64::from(animation.parameters().duration_ms);
        if self.elapsed >= duration {
            self.finished = true;
            return 1000;
        }
        ((self.elapsed * 1000) / duration) as u16
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
    use super::*;
    #[test]
    fn rerender_visibility_and_reduced_motion_never_restart_playback() {
        let mut playback = Playback::new(true);
        assert_eq!(playback.tick(100, true, false, Animation::Wave), 0);
        assert_eq!(playback.tick(700, false, false, Animation::Wave), 500);
        assert_eq!(playback.tick(10000, true, false, Animation::Wave), 500);
        assert_eq!(playback.tick(10600, true, false, Animation::Wave), 1000);
        assert_eq!(playback.tick(10601, false, false, Animation::Wave), 1000);
        playback.replay();
        assert_eq!(playback.tick(11000, true, false, Animation::Wave), 0);
        assert_eq!(playback.tick(11001, true, true, Animation::Wave), 1000);
        assert_eq!(playback.tick(11002, true, false, Animation::Wave), 1000);
        assert_eq!(
            Playback::new(false).tick(1, true, false, Animation::Wave),
            1000
        );
        assert_ne!(seed(&[1; 32], 1, 1), seed(&[1; 32], 2, 1));
    }
}
