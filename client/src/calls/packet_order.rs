use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

pub(super) struct PacketOrder<T> {
    next: Option<u16>,
    pending: BTreeMap<u16, (Instant, T)>,
}
impl<T> Default for PacketOrder<T> {
    fn default() -> Self {
        Self {
            next: None,
            pending: BTreeMap::new(),
        }
    }
}
impl<T> PacketOrder<T> {
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
    pub fn push(&mut self, sequence: u16, value: T, now: Instant) {
        let next = *self.next.get_or_insert(sequence);
        if (sequence.wrapping_sub(next) as i16) < 0 || self.pending.len() >= 128 {
            return;
        }
        self.pending.entry(sequence).or_insert((now, value));
    }
    pub fn pop(&mut self, now: Instant, repair_wait: Duration) -> Option<(T, bool)> {
        let mut next = self.next?;
        let mut gap = false;
        if !self.pending.contains_key(&next) {
            let (&sequence, (arrived, _)) = self
                .pending
                .iter()
                .min_by_key(|(sequence, _)| sequence.wrapping_sub(next))?;
            if now.saturating_duration_since(*arrived) < repair_wait
                && self.pending.len() < 128
            {
                return None;
            }
            next = sequence;
            gap = true;
        }
        let (_, value) = self.pending.remove(&next)?;
        self.next = Some(next.wrapping_add(1));
        Some((value, gap))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reordering_wrap_duplicates_and_loss_are_bounded() {
        let now = Instant::now();
        let mut queue = PacketOrder::default();
        queue.push(65534, 1, now);
        queue.push(0, 3, now);
        assert_eq!(queue.pop(now, Duration::from_millis(120)), Some((1, false)));
        assert_eq!(queue.pop(now, Duration::from_millis(120)), None);
        queue.push(65535, 2, now);
        queue.push(65535, 99, now);
        assert_eq!(queue.pop(now, Duration::from_millis(120)), Some((2, false)));
        assert_eq!(queue.pop(now, Duration::from_millis(120)), Some((3, false)));
        queue.push(65535, 99, now);
        queue.push(2, 5, now);
        assert_eq!(queue.pop(now + Duration::from_millis(119), Duration::from_millis(120)), None);
        assert_eq!(queue.pop(now + Duration::from_millis(120), Duration::from_millis(120)), Some((5, true)));
        for sequence in 3..1000 {
            queue.push(sequence, 0, now);
        }
        assert_eq!(queue.pending.len(), 128);
    }
    #[test]
    fn a_retransmission_after_one_network_round_trip_repairs_the_frame() {
        let now = Instant::now();
        let mut queue = PacketOrder::default();
        queue.push(10, 10, now);
        queue.push(12, 12, now);
        assert_eq!(queue.pop(now, Duration::from_millis(120)), Some((10, false)));
        assert_eq!(queue.pop(now + Duration::from_millis(40), Duration::from_millis(120)), None);
        let repaired = now + Duration::from_millis(80);
        queue.push(11, 11, repaired);
        assert_eq!(queue.pop(repaired, Duration::from_millis(120)), Some((11, false)));
        assert_eq!(queue.pop(repaired, Duration::from_millis(120)), Some((12, false)));
    }
    #[test]
    fn audio_keeps_its_shorter_loss_deadline() {
        let now = Instant::now();
        let mut queue = PacketOrder::default();
        queue.push(1, 1, now);
        queue.push(3, 3, now);
        let wait = Duration::from_millis(40);
        assert_eq!(queue.pop(now, wait), Some((1, false)));
        assert_eq!(queue.pop(now + wait, wait), Some((3, true)));
    }
}
