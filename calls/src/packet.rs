use crate::{hash, Error, Id, MediaKind};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

const CHUNK: usize = 1024;
const HEADER: usize = 44;
const MAX: usize = 1024 * 1024 + 128;
const DOMAIN: &[u8] = b"Sigil/call-packet/v1";

/// RTP payloads; the adapter supplies sequence numbers, timestamps and the final marker.
pub fn packetize(kind: MediaKind, encrypted: &[u8]) -> Result<Vec<Vec<u8>>, Error> {
    if encrypted.is_empty() || encrypted.len() > MAX {
        return Err(Error::Limit);
    }
    let id = hash(DOMAIN, encrypted);
    let count = encrypted.len().div_ceil(CHUNK);
    Ok(encrypted
        .chunks(CHUNK)
        .enumerate()
        .map(|(index, data)| {
            let mut packet = Vec::with_capacity(HEADER + CHUNK + 1);
            if kind != MediaKind::Audio {
                packet.push(if index == 0 { 0x10 } else { 0 });
            }
            packet.extend_from_slice(b"SCP\x01");
            packet.extend_from_slice(&id);
            packet.extend_from_slice(&(encrypted.len() as u32).to_be_bytes());
            packet.extend_from_slice(&(index as u16).to_be_bytes());
            packet.extend_from_slice(&(count as u16).to_be_bytes());
            packet.extend_from_slice(data);
            packet
        })
        .collect())
}
struct Pending {
    created: Instant,
    data: Vec<u8>,
    seen: Vec<bool>,
    received: usize,
}
#[derive(Default)]
pub struct Assembly {
    pending: BTreeMap<(Id, u8, Id), Pending>,
    bytes: usize,
}
impl Assembly {
    pub fn clear(&mut self) {
        self.pending.clear();
        self.bytes = 0;
    }
    /// Untrusted fragments are bounded; only the authenticated frame may reach a decoder.
    pub fn push(
        &mut self,
        sender: Id,
        kind: MediaKind,
        packet: &[u8],
        now: Instant,
    ) -> Result<Option<Vec<u8>>, Error> {
        self.pending.retain(|_, value| {
            now.saturating_duration_since(value.created) < Duration::from_secs(2)
        });
        self.bytes = self.pending.values().map(|v| v.data.len()).sum();
        let bytes = if kind == MediaKind::Audio {
            packet
        } else {
            packet.get(1..).ok_or(Error::Invalid)?
        };
        if bytes.len() <= HEADER || bytes.len() > HEADER + CHUNK || &bytes[..4] != b"SCP\x01" {
            return Err(Error::Invalid);
        }
        let id: Id = bytes[4..36].try_into().unwrap();
        let length = u32::from_be_bytes(bytes[36..40].try_into().unwrap()) as usize;
        let index = u16::from_be_bytes(bytes[40..42].try_into().unwrap()) as usize;
        let count = u16::from_be_bytes(bytes[42..44].try_into().unwrap()) as usize;
        if length == 0
            || length > MAX
            || count != length.div_ceil(CHUNK)
            || index >= count
            || bytes.len() - HEADER != (length - index * CHUNK).min(CHUNK)
            || kind != MediaKind::Audio && packet[0] != if index == 0 { 0x10 } else { 0 }
        {
            return Err(Error::Invalid);
        }
        let key = (sender, kind as u8, id);
        if !self.pending.contains_key(&key) {
            if self.pending.len() >= 16 || self.bytes + length > 8 * 1024 * 1024 {
                return Err(Error::Limit);
            }
            self.pending.insert(
                key,
                Pending {
                    created: now,
                    data: vec![0; length],
                    seen: vec![false; count],
                    received: 0,
                },
            );
            self.bytes += length;
        }
        let pending = self.pending.get_mut(&key).unwrap();
        if pending.data.len() != length {
            return Err(Error::Conflict);
        }
        let target = &mut pending.data[index * CHUNK..index * CHUNK + bytes.len() - HEADER];
        if pending.seen[index] {
            if target != &bytes[HEADER..] {
                return Err(Error::Conflict);
            }
            return Ok(None);
        }
        target.copy_from_slice(&bytes[HEADER..]);
        pending.seen[index] = true;
        pending.received += 1;
        if pending.received != count {
            return Ok(None);
        }
        let pending = self.pending.remove(&key).unwrap();
        self.bytes -= pending.data.len();
        if hash(DOMAIN, &pending.data) != id {
            return Err(Error::Authentication);
        }
        Ok(Some(pending.data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reordered_duplicate_missing_forged_and_oversized_fragments_stay_bounded() {
        let now = Instant::now();
        let bytes = vec![13; 1024 * 1024];
        let packets = packetize(MediaKind::Screen, &bytes).unwrap();
        let mut assembly = Assembly::default();
        for packet in packets.iter().skip(1).rev() {
            assert!(assembly
                .push([1; 32], MediaKind::Screen, packet, now)
                .unwrap()
                .is_none());
            assert!(assembly
                .push([1; 32], MediaKind::Screen, packet, now)
                .unwrap()
                .is_none());
        }
        let mut wrong = packets[0].clone();
        *wrong.last_mut().unwrap() ^= 1;
        assert_eq!(
            assembly.push([1; 32], MediaKind::Screen, &wrong, now),
            Err(Error::Authentication)
        );
        for packet in packets.iter().skip(1) {
            assembly
                .push([1; 32], MediaKind::Screen, packet, now)
                .unwrap();
        }
        assert_eq!(
            assembly
                .push([1; 32], MediaKind::Screen, &packets[0], now)
                .unwrap()
                .unwrap(),
            bytes
        );
        for sender in 1..=8 {
            assembly
                .push([sender; 32], MediaKind::Screen, &packets[0], now)
                .unwrap();
        }
        assert_eq!(
            assembly.push([9; 32], MediaKind::Screen, &packets[0], now),
            Err(Error::Limit)
        );
        assert!(assembly
            .push(
                [9; 32],
                MediaKind::Screen,
                &packets[0],
                now + Duration::from_secs(2)
            )
            .unwrap()
            .is_none());
        assert_eq!(assembly.pending.len(), 1);
        assert_eq!(assembly.bytes, bytes.len());
        for length in 0..packets[0].len() - 1 {
            assert!(assembly
                .push([1; 32], MediaKind::Screen, &packets[0][..length], now)
                .is_err());
        }
    }
}
