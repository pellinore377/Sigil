//! Native video sender adaptation: loss-based bitrate (as in the loss half of GCC) plus
//! thermal and battery limits. Browsers rely on their own congestion control. Android applies
//! only the bitrate: switching camera frame rate mid-call corrupted Pixel's AV1 encoder output.
use std::time::{Duration, Instant};

/// Android thermal status scale: none, light, moderate, severe, critical, emergency, shutdown.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Thermal(pub u8);

pub struct Conditions {
    /// Fraction of uplink packets lost in the newest receiver report, if one arrived.
    pub loss: Option<f32>,
    /// Keyframe requests from receivers since the previous update: loss beyond the uplink.
    pub key_requests: u32,
    pub thermal: Thermal,
    pub power_save: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Target {
    pub bitrate: u32,
    pub fps: u32,
}

pub struct RateControl {
    ceiling: u32,
    fps: u32,
    target: u32,
    raised: Instant,
    lowered: Instant,
}

const FLOOR: u32 = 300_000;
const RAISE_EVERY: Duration = Duration::from_secs(1);
/// Only one loss cut per report interval; reports arrive about once a second.
const LOWER_EVERY: Duration = Duration::from_millis(500);

impl RateControl {
    /// `ceiling` is the bitrate for the capture mode at `fps`; calls start there, as the
    /// sender has no estimate yet and most links carry it.
    pub fn new(ceiling: u32, fps: u32, now: Instant) -> Self {
        let ceiling = ceiling.max(FLOOR);
        Self { ceiling, fps: fps.max(1), target: ceiling, raised: now, lowered: now - LOWER_EVERY }
    }

    pub fn update(&mut self, conditions: &Conditions, now: Instant) -> Target {
        let lost = conditions.loss.unwrap_or(0.0).clamp(0.0, 1.0);
        let congested = lost > 0.1 || conditions.key_requests > 1;
        if congested && now.duration_since(self.lowered) >= LOWER_EVERY {
            let cut = if lost > 0.1 { 1.0 - 0.5 * lost } else { 0.85 };
            self.target = ((self.target as f32 * cut) as u32).max(FLOOR);
            self.lowered = now;
            self.raised = now;
        } else if conditions.loss.is_some_and(|l| l < 0.02) && conditions.key_requests == 0
            && now.duration_since(self.raised) >= RAISE_EVERY
        {
            self.target = ((self.target as f32 * 1.08) as u32).min(self.ceiling);
            self.raised = now;
        }
        // Heat and battery saver cap work: the camera, ISP and encoder scale with frame rate.
        // Moderate heat trims bits but keeps motion; only severe heat, or a battery saver the
        // user chose, costs frame rate. Unmanaged heat throttles the chip and stutters worse.
        let (share, fps) = match (conditions.thermal.0, conditions.power_save) {
            (0 | 1, false) => (1.0, self.fps),
            (2, false) => (0.85, self.fps),
            (0..=2, true) => (0.8, self.fps.min(30)),
            (3, _) => (0.6, self.fps.min(30)),
            _ => (0.35, self.fps.min(15)),
        };
        let bitrate = self.target.min((self.ceiling as f32 * share) as u32).max(FLOOR);
        // A starved stream spends its bits better on fewer, sharper frames.
        let fps = if bitrate < self.ceiling / 3 { fps.min(30) } else { fps };
        Target { bitrate, fps }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at(start: Instant, ms: u64) -> Instant { start + Duration::from_millis(ms) }
    fn clear(loss: Option<f32>) -> Conditions { Conditions { loss, key_requests: 0, thermal: Thermal(0), power_save: false } }

    #[test]
    fn loss_cuts_quickly_and_recovery_climbs_back_to_the_mode_ceiling() {
        let start = Instant::now();
        let mut rate = RateControl::new(7_000_000, 60, start);
        assert_eq!(rate.update(&clear(None), start), Target { bitrate: 7_000_000, fps: 60 });
        let cut = rate.update(&clear(Some(0.3)), at(start, 1000));
        assert_eq!(cut.bitrate, 5_950_000);
        // A second bad report inside the cut interval does not compound.
        assert_eq!(rate.update(&clear(Some(0.3)), at(start, 1200)).bitrate, 5_950_000);
        let mut last = cut.bitrate;
        for second in 2..40 {
            let next = rate.update(&clear(Some(0.0)), at(start, second * 1000)).bitrate;
            assert!(next >= last && next <= 7_000_000);
            last = next;
        }
        assert_eq!(last, 7_000_000);
    }

    #[test]
    fn repeated_keyframe_requests_count_as_congestion_beyond_the_uplink() {
        let start = Instant::now();
        let mut rate = RateControl::new(7_000_000, 60, start);
        let requests = Conditions { loss: Some(0.0), key_requests: 3, thermal: Thermal(0), power_save: false };
        assert_eq!(rate.update(&requests, at(start, 600)).bitrate, 5_950_000);
        // Moderate loss alone neither cuts nor raises.
        assert_eq!(rate.update(&clear(Some(0.05)), at(start, 3000)).bitrate, 5_950_000);
    }

    #[test]
    fn heat_and_battery_saver_lower_frame_rate_and_bitrate_then_release_them() {
        let start = Instant::now();
        let mut rate = RateControl::new(7_000_000, 60, start);
        let with = |thermal, power_save| Conditions { loss: None, key_requests: 0, thermal: Thermal(thermal), power_save };
        assert_eq!(rate.update(&with(1, false), start), Target { bitrate: 7_000_000, fps: 60 });
        assert_eq!(rate.update(&with(2, false), start), Target { bitrate: 5_950_000, fps: 60 });
        assert_eq!(rate.update(&with(0, true), start), Target { bitrate: 5_600_000, fps: 30 });
        assert_eq!(rate.update(&with(3, false), start), Target { bitrate: 4_200_000, fps: 30 });
        assert_eq!(rate.update(&with(4, false), start), Target { bitrate: 2_450_000, fps: 15 });
        assert_eq!(rate.update(&with(0, false), start), Target { bitrate: 7_000_000, fps: 60 });
    }

    #[test]
    fn a_starved_link_trades_frame_rate_for_detail_and_never_drops_below_the_floor() {
        let start = Instant::now();
        let mut rate = RateControl::new(7_000_000, 60, start);
        let mut target = rate.update(&clear(None), start);
        for step in 1..40 {
            target = rate.update(&clear(Some(0.5)), at(start, step * 600));
        }
        assert_eq!(target, Target { bitrate: FLOOR, fps: 30 });
    }
}
