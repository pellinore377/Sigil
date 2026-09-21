// Map source microseconds to a bounded presentation clock in local milliseconds.
#[derive(Default)]
pub(super) struct Clock(Option<(u64, f64)>);
impl Clock {
    pub(super) fn due(&mut self, stamp: u64, now: f64) -> f64 {
        if let Some((source, local)) = self.0 {
            if stamp >= source {
                let due = local + (stamp - source) as f64 / 1000.0;
                if due >= now - 80.0 && due <= now + 120.0 {
                    return due;
                }
            }
        }
        let due = now + 60.0;
        self.0 = Some((stamp, due));
        due
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_receive_burst_preserves_source_spacing() {
        let mut clock = Clock::default();
        assert_eq!(clock.due(0, 100.0), 160.0);
        assert_eq!(clock.due(16_000, 155.0), 176.0);
        assert_eq!(clock.due(32_000, 155.0), 192.0);
        assert_eq!(clock.due(48_000, 155.0), 208.0);
    }
    #[test]
    fn interrupted_or_restarted_capture_cannot_accumulate_latency() {
        let mut clock = Clock::default();
        clock.due(1_000_000, 100.0);
        assert_eq!(clock.due(1_016_000, 500.0), 560.0);
        assert_eq!(clock.due(0, 520.0), 580.0);
        assert_eq!(clock.due(10_000_000, 540.0), 600.0);
    }
}
