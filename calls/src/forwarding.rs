use std::time::Instant;
#[derive(Default)]
pub(crate) struct Queue {
    sampled: Option<Instant>,
    bytes: usize,
    packets: usize,
}
impl Queue {
    pub fn admit(&mut self, sample: Option<(Instant, usize, usize)>, bytes: usize) -> bool {
        let (queued_bytes, queued_packets) = if let Some((at, bytes, packets)) = sample {
            if self.sampled != Some(at) {
                self.sampled = Some(at);
                self.bytes = 0;
                self.packets = 0;
            }
            (bytes, packets)
        } else {
            (0, 0)
        };
        if queued_bytes
            .saturating_add(self.bytes)
            .saturating_add(bytes)
            > 65536
            || queued_packets.saturating_add(self.packets) >= 128
        {
            return false;
        }
        self.bytes += bytes;
        self.packets += 1;
        true
    }
}
#[derive(Default)]
pub(crate) struct Rewrite {
    current: Option<Segment>,
}
struct Segment {
    generation: u64,
    ssrc: u32,
    origin: u64,
    output: u64,
    highest: u64,
    timestamp: u32,
    output_timestamp: u32,
    highest_timestamp: u32,
}
impl Rewrite {
    pub fn packet(
        &mut self,
        generation: u64,
        ssrc: u32,
        sequence: u64,
        timestamp: u32,
        step: u32,
    ) -> Option<(u64, u32)> {
        if self
            .current
            .as_ref()
            .is_none_or(|s| s.generation != generation || s.ssrc != ssrc)
        {
            let (output, output_timestamp) = match &self.current {
                Some(old) => (
                    old.highest.checked_add(1)?,
                    old.highest_timestamp.wrapping_add(step),
                ),
                None => (sequence, timestamp),
            };
            self.current = Some(Segment {
                generation,
                ssrc,
                origin: sequence,
                output,
                highest: output,
                timestamp,
                output_timestamp,
                highest_timestamp: output_timestamp,
            });
        }
        let segment = self.current.as_mut()?;
        let sequence = segment
            .output
            .checked_add(sequence.checked_sub(segment.origin)?)?;
        let timestamp = segment
            .output_timestamp
            .wrapping_add(timestamp.wrapping_sub(segment.timestamp));
        if sequence > segment.highest {
            segment.highest = sequence;
            segment.highest_timestamp = timestamp;
        }
        Some((sequence, timestamp))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_restarts_and_wraps_preserve_output_indices_and_reordered_packets() {
        let mut rewrite = Rewrite::default();
        assert_eq!(
            rewrite.packet(1, 10, 65534, u32::MAX - 959, 960),
            Some((65534, u32::MAX - 959))
        );
        assert_eq!(rewrite.packet(1, 10, 65536, 960, 960), Some((65536, 960)));
        assert_eq!(rewrite.packet(1, 10, 65535, 0, 960), Some((65535, 0)));
        assert_eq!(rewrite.packet(2, 20, 0, 0, 960), Some((65537, 1920)));
        assert_eq!(rewrite.packet(2, 20, 2, 1920, 960), Some((65539, 3840)));
        assert_eq!(rewrite.packet(2, 20, 1, 960, 960), Some((65538, 2880)));
        assert_eq!(rewrite.packet(2, 21, 4000, 99, 960), Some((65540, 4800)));
        assert_eq!(rewrite.packet(2, 21, 3999, 98, 960), None);
    }
    #[test]
    fn unpolled_and_stale_queue_samples_cannot_bypass_byte_or_packet_limits() {
        let mut queue = Queue::default();
        for _ in 0..64 {
            assert!(queue.admit(None, 1024));
        }
        assert!(!queue.admit(None, 1));
        let at = Instant::now();
        assert!(!queue.admit(Some((at, 65536, 64)), 1));
        assert!(queue.admit(Some((at, 0, 0)), 1024));
        assert!(queue.admit(Some((at, 0, 0)), 1024));
        assert!(!queue.admit(Some((at, 64000, 127)), 1024));
        let later = at + std::time::Duration::from_millis(1);
        for _ in 0..128 {
            assert!(queue.admit(Some((later, 0, 0)), 0));
        }
        assert!(!queue.admit(Some((later, 0, 0)), 0));
    }
}
