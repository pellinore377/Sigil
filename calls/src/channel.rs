use crate::{Error, Id, MediaKind};
pub const LABEL: &str = "sigil.media.v1";
const PREFIX: &[u8; 5] = b"SGDC\x01";
const HEADER: usize = 51;

pub struct Packet {
    pub sender: Id,
    pub kind: MediaKind,
    pub sequence: u64,
    pub timestamp: u32,
    pub marker: bool,
    pub payload: std::sync::Arc<[u8]>,
}
impl Packet {
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        if self.sender == [0; 32]
            || self.payload.is_empty()
            || self.payload.len() > 1500
            || self.sequence > i64::MAX as u64
        {
            return Err(Error::Invalid);
        }
        let mut bytes = Vec::with_capacity(HEADER + self.payload.len());
        bytes.extend_from_slice(PREFIX);
        bytes.push(self.kind as u8);
        bytes.extend_from_slice(&self.sender);
        bytes.extend_from_slice(&self.sequence.to_be_bytes());
        bytes.extend_from_slice(&self.timestamp.to_be_bytes());
        bytes.push(u8::from(self.marker));
        bytes.extend_from_slice(&self.payload);
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        if !(HEADER + 1..=HEADER + 1500).contains(&bytes.len())
            || &bytes[..5] != PREFIX
            || bytes[50] > 1
        {
            return Err(Error::Invalid);
        }
        let kind = match bytes[5] {
            0 => MediaKind::Audio,
            1 => MediaKind::Camera,
            2 => MediaKind::Screen,
            _ => return Err(Error::Invalid),
        };
        let sender = bytes[6..38].try_into().map_err(|_| Error::Invalid)?;
        let sequence = u64::from_be_bytes(bytes[38..46].try_into().map_err(|_| Error::Invalid)?);
        if sender == [0; 32] || sequence > i64::MAX as u64 {
            return Err(Error::Invalid);
        }
        Ok(Self {
            sender,
            kind,
            sequence,
            timestamp: u32::from_be_bytes(bytes[46..50].try_into().map_err(|_| Error::Invalid)?),
            marker: bytes[50] == 1,
            payload: std::sync::Arc::from(&bytes[HEADER..]),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn carrier_preserves_ciphertext_and_rejects_invalid_metadata() {
        let mut packet = Packet {
            sender: [1; 32],
            kind: MediaKind::Screen,
            sequence: 65537,
            timestamp: u32::MAX,
            marker: true,
            payload: vec![7; 1069].into(),
        };
        let encoded = packet.encode().unwrap();
        let decoded = Packet::decode(&encoded).unwrap();
        assert_eq!(decoded.sender, packet.sender);
        assert_eq!(decoded.kind, packet.kind);
        assert_eq!(decoded.sequence, packet.sequence);
        assert_eq!(decoded.timestamp, packet.timestamp);
        assert_eq!(decoded.marker, packet.marker);
        assert_eq!(decoded.payload, packet.payload);
        for length in 0..=HEADER {
            assert!(Packet::decode(&encoded[..length]).is_err());
        }
        for (index, value) in [(0, 0), (4, 2), (5, 3), (50, 2)] {
            let mut bad = encoded.clone();
            bad[index] = value;
            assert!(Packet::decode(&bad).is_err());
        }
        packet.payload = vec![0; 1501].into();
        assert!(packet.encode().is_err());
        packet.payload = Vec::new().into();
        assert!(packet.encode().is_err());
        packet.payload = vec![1].into();
        packet.sender = [0; 32];
        assert!(packet.encode().is_err());
    }
}
