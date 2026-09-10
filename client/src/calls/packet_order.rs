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
    pub fn push(&mut self, sequence: u16, value: T, now: Instant) {
        let next = *self.next.get_or_insert(sequence);
        if (sequence.wrapping_sub(next) as i16) < 0 || self.pending.len() >= 128 {
            return;
        }
        self.pending.entry(sequence).or_insert((now, value));
    }
    pub fn pop(&mut self, now: Instant) -> Option<(T, bool)> {
        let mut next = self.next?;
        let mut gap = false;
        if !self.pending.contains_key(&next) {
            let (&sequence, (arrived, _)) = self
                .pending
                .iter()
                .min_by_key(|(sequence, _)| sequence.wrapping_sub(next))?;
            if now.saturating_duration_since(*arrived) < Duration::from_millis(40)
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
        assert_eq!(queue.pop(now), Some((1, false)));
        assert_eq!(queue.pop(now), None);
        queue.push(65535, 2, now);
        queue.push(65535, 99, now);
        assert_eq!(queue.pop(now), Some((2, false)));
        assert_eq!(queue.pop(now), Some((3, false)));
        queue.push(65535, 99, now);
        queue.push(2, 5, now);
        assert_eq!(queue.pop(now + Duration::from_millis(39)), None);
        assert_eq!(queue.pop(now + Duration::from_millis(40)), Some((5, true)));
        for sequence in 3..1000 {
            queue.push(sequence, 0, now);
        }
        assert_eq!(queue.pending.len(), 128);
    }
}
