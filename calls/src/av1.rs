//! Encrypted frames carried as one opaque AV1 frame OBU. Only an endpoint opens its payload.
//! RTP aggregation follows https://aomediacodec.github.io/av1-rtp-spec/.
use crate::Error;
const MAX: usize = 1024 * 1024 + 128;

fn length(mut n: usize, out: &mut Vec<u8>) {
    loop { let byte = (n & 127) as u8; n >>= 7; out.push(byte | if n == 0 { 0 } else { 128 }); if n == 0 { break; } }
}
fn read_length(bytes: &[u8], at: &mut usize) -> Result<usize, Error> {
    let mut n = 0usize;
    for shift in (0..28).step_by(7) {
        let byte = *bytes.get(*at).ok_or(Error::Invalid)?; *at += 1;
        n |= usize::from(byte & 127) << shift;
        if byte & 128 == 0 { return Ok(n); }
    }
    Err(Error::Invalid)
}
pub fn wrap(encrypted: &[u8]) -> Result<Vec<u8>, Error> {
    if encrypted.is_empty() || encrypted.len() > MAX { return Err(Error::Limit); }
    let mut bytes = vec![0x32]; // OBU_FRAME, with size.
    length(encrypted.len(), &mut bytes); bytes.extend_from_slice(encrypted); Ok(bytes)
}
pub fn unwrap(bytes: &[u8]) -> Result<&[u8], Error> {
    if bytes.first() != Some(&0x32) { return Err(Error::Invalid); }
    let mut at = 1;
    let size = read_length(bytes, &mut at)?;
    if size == 0 || size > MAX || bytes.len() - at != size { return Err(Error::Invalid); }
    Ok(&bytes[at..])
}
pub fn packetize(encrypted: &[u8], keyframe: bool) -> Result<Vec<Vec<u8>>, Error> {
    if encrypted.is_empty() || encrypted.len() > MAX { return Err(Error::Limit); }
    let mut obu = Vec::with_capacity(encrypted.len() + 1);
    obu.push(0x30); obu.extend_from_slice(encrypted);
    let count = obu.len().div_ceil(1100);
    Ok(obu.chunks(1100).enumerate().map(|(i, chunk)| {
        let mut packet = Vec::with_capacity(chunk.len() + 1);
        packet.push(0x10 | if i > 0 { 0x80 } else { 0 } | if i + 1 < count { 0x40 } else { 0 } | if i == 0 && keyframe { 8 } else { 0 });
        packet.extend_from_slice(chunk); packet
    }).collect())
}
#[derive(Default)]
pub struct Assembly { data: Vec<u8>, next: Option<u16>, timestamp: u32 }
impl Assembly {
    pub fn clear(&mut self) { self.data.clear(); self.next = None; }
    /// Caller orders packets and bounds their age. A gap discards the incomplete OBU.
    pub fn push(&mut self, seq: u16, timestamp: u32, marker: bool, bytes: &[u8]) -> Result<Option<Vec<u8>>, Error> {
        let result = self.append(seq, timestamp, marker, bytes);
        if result.is_err() { self.clear(); }
        result
    }
    fn append(&mut self, seq: u16, timestamp: u32, marker: bool, bytes: &[u8]) -> Result<Option<Vec<u8>>, Error> {
        let h = *bytes.first().ok_or(Error::Invalid)?;
        // One opaque OBU; reject aggregation, reserved bits, and inconsistent boundaries.
        if h & 7 != 0 || h & 0x30 > 0x10 || h & 0x88 == 0x88 || marker == (h & 0x40 != 0) { return Err(Error::Invalid); }
        let mut at = 1;
        let size = if h & 0x30 == 0 { read_length(bytes, &mut at)? } else { bytes.len() - at };
        if size == 0 || size != bytes.len() - at { return Err(Error::Invalid); }
        if h & 0x80 == 0 { self.clear(); self.timestamp = timestamp; }
        else if self.next != Some(seq) || self.timestamp != timestamp { return Err(Error::Invalid); }
        if self.data.len() + size > MAX + 8 { return Err(Error::Limit); }
        self.data.extend_from_slice(&bytes[at..]); self.next = Some(seq.wrapping_add(1));
        if !marker { return Ok(None); }
        let bytes = std::mem::take(&mut self.data); self.next = None;
        let payload = match bytes.first() {
            Some(0x30) if bytes.len() > 1 && bytes.len() <= MAX + 1 => &bytes[1..],
            Some(0x32) => unwrap(&bytes)?,
            _ => return Err(Error::Invalid),
        };
        Ok(Some(payload.to_vec()))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn encrypted_obu_round_trip_and_sequence_wrap() {
        for size in [1, 1100, 1101, 100_000, MAX] {
            let bytes = vec![91; size]; assert_eq!(unwrap(&wrap(&bytes).unwrap()).unwrap(), bytes);
            let packets = packetize(&bytes, true).unwrap(); let mut a = Assembly::default();
            for (i, p) in packets.iter().enumerate() {
                let result = a.push(65535u16.wrapping_add(i as u16), 42, i + 1 == packets.len(), p).unwrap();
                assert_eq!(result, (i + 1 == packets.len()).then(|| bytes.clone()));
            }
        }
    }
    #[test] fn gaps_truncation_and_malformed_boundaries_fail_closed() {
        let p = packetize(&vec![7; 4000], false).unwrap(); let mut a = Assembly::default();
        assert!(a.push(1, 42, false, &p[0]).unwrap().is_none());
        assert!(a.push(3, 42, false, &p[2]).is_err());
        assert!(a.push(4, 42, true, &p[3]).is_err());
        for bytes in [&[][..], &[0x10], &[0x50, 0x30, 7], &[0x20, 0x30, 7], &[0, 255]] { assert!(a.push(5, 43, true, bytes).is_err()); }
        assert_eq!(a.push(6, 43, true, &[0x10, 0x30, 8]).unwrap(), Some(vec![8]));
        assert_eq!(a.push(7, 44, true, &[0, 2, 0x30, 9]).unwrap(), Some(vec![9]));
    }
}
