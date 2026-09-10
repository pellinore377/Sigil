use std::{
    collections::{BTreeMap, VecDeque},
    time::{Duration, Instant},
};

const MAX_BYTES: usize = 8 * 1024 * 1024;
#[derive(Default)]
pub(super) struct ReadyFrames {
    streams: BTreeMap<u32, VecDeque<(Instant, Vec<u8>)>>,
    bytes: usize,
}
impl ReadyFrames {
    pub fn clear(&mut self, stream: u32) {
        if let Some(frames) = self.streams.remove(&stream) {
            self.bytes -= frames.iter().map(|(_, bytes)| bytes.len()).sum::<usize>();
        }
    }
    pub fn push(&mut self, stream: u32, audio: bool, bytes: Vec<u8>, now: Instant) -> bool {
        let dropped = self
            .streams
            .get(&stream)
            .is_some_and(|v| v.len() >= if audio { 8 } else { 3 });
        if dropped {
            self.clear(stream);
        }
        if self.bytes + bytes.len() > MAX_BYTES
            || !self.streams.contains_key(&stream) && self.streams.len() >= 32
        {
            self.clear(stream);
            return true;
        }
        self.bytes += bytes.len();
        self.streams
            .entry(stream)
            .or_default()
            .push_back((now, bytes));
        dropped
    }
    pub fn pop(&mut self, stream: u32, now: Instant) -> (Option<Vec<u8>>, bool) {
        let Some(frames) = self.streams.get_mut(&stream) else {
            return (None, false);
        };
        let mut dropped = false;
        let mut result = None;
        while let Some((created, bytes)) = frames.pop_front() {
            self.bytes -= bytes.len();
            if now.saturating_duration_since(created) >= Duration::from_secs(2) {
                dropped = true;
            } else {
                result = Some(bytes);
                break;
            }
        }
        if frames.is_empty() {
            self.streams.remove(&stream);
        }
        (result, dropped)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queued_ciphertext_has_bounded_memory_age_and_per_stream_backlog() {
        let now = Instant::now();
        let mut queue = ReadyFrames::default();
        for id in 0..3 {
            assert!(!queue.push(1, false, vec![id; 1024], now));
        }
        assert!(queue.push(1, false, vec![3; 1024], now));
        assert_eq!(queue.bytes, 1024);
        assert_eq!(queue.pop(1, now), (Some(vec![3; 1024]), false));
        assert_eq!(queue.bytes, 0);
        assert!(!queue.push(2, true, vec![0; 1024], now));
        assert_eq!(queue.pop(2, now + Duration::from_secs(2)), (None, true));
        assert_eq!(queue.bytes, 0);
        for id in 0..8 {
            assert!(!queue.push(id, false, vec![0; 1024 * 1024], now));
        }
        assert!(queue.push(9, false, vec![0; 1024], now));
        assert_eq!(queue.bytes, MAX_BYTES);
        queue.clear(4);
        assert!(!queue.push(9, false, vec![0; 1024], now));
        assert_eq!(queue.bytes, MAX_BYTES - 1024 * 1024 + 1024);
    }
}
