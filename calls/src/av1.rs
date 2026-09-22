//! Encrypted frames carried as one opaque AV1 frame OBU. Only an endpoint opens its payload.
//! RTP aggregation follows https://aomediacodec.github.io/av1-rtp-spec/.
use crate::Error;
const MAX: usize = 1024 * 1024 + 128;

pub fn camera_size(width: u16, height: u16) -> bool {
    (16..=3840).contains(&width) && (16..=3840).contains(&height)
        && u32::from(width) * u32::from(height) <= 3840 * 2160
}

/// Authenticated renderer envelope: media marker, key flag, timestamp, then camera header.
pub fn camera_payload(bytes: &[u8]) -> Result<(u16, u16, u16, &[u8]), Error> {
    if bytes.len() <= 17 || bytes[0] != 1 || bytes[1] > 1 || bytes[10] != 2 { return Err(Error::Invalid); }
    let field = |at| u16::from_be_bytes([bytes[at], bytes[at + 1]]);
    let (rotation, width, height) = (field(11), field(13), field(15));
    if !matches!(rotation, 0 | 90 | 180 | 270) || !camera_size(width, height) {
        return Err(Error::Invalid);
    }
    Ok((rotation, width, height, &bytes[17..]))
}

/// Retain codec configuration so a later keyframe can initialize a fresh decoder.
#[derive(Default)]
pub struct Sequence(Vec<u8>);
impl Sequence {
    pub fn clear(&mut self) { use zeroize::Zeroize; self.0.zeroize(); self.0.clear(); }
    pub fn frame<'a>(&mut self, bytes: &'a [u8], key: bool) -> Result<std::borrow::Cow<'a, [u8]>, Error> {
        use std::borrow::Cow;
        if bytes.is_empty() || bytes.len() > MAX { return Err(Error::Limit); }
        let mut at = 0;
        let mut delimiter = 0;
        let mut sequence = None;
        while at < bytes.len() {
            let start = at;
            let header = bytes[at]; at += 1;
            if header & 0x81 != 0 || header & 2 == 0 { return Err(Error::Invalid); }
            if header & 4 != 0 {
                let extension = *bytes.get(at).ok_or(Error::Invalid)?; at += 1;
                if extension & 7 != 0 { return Err(Error::Invalid); }
            }
            let length = read_length(bytes, &mut at)?;
            at = at.checked_add(length).filter(|end| *end <= bytes.len()).ok_or(Error::Invalid)?;
            match (header >> 3) & 15 {
                1 => { if length == 0 || length > 65536 { return Err(Error::Limit); } sequence = Some(start..at); }
                2 if start == 0 => delimiter = at,
                _ => {}
            }
        }
        if let Some(range) = sequence {
            self.clear(); self.0.extend_from_slice(&bytes[range]);
            return Ok(Cow::Borrowed(bytes));
        }
        if !key { return Ok(Cow::Borrowed(bytes)); }
        if self.0.is_empty() { return Err(Error::Invalid); }
        if bytes.len() + self.0.len() > MAX { return Err(Error::Limit); }
        let mut independent = Vec::with_capacity(bytes.len() + self.0.len());
        independent.extend_from_slice(&bytes[..delimiter]);
        independent.extend_from_slice(&self.0);
        independent.extend_from_slice(&bytes[delimiter..]);
        Ok(Cow::Owned(independent))
    }
}
impl Drop for Sequence { fn drop(&mut self) { self.clear(); } }

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
pub fn wrap(encrypted: &[u8], keyframe: bool) -> Result<Vec<u8>, Error> {
    if encrypted.is_empty() || encrypted.len() > MAX { return Err(Error::Limit); }
    // Chromium sets RTP's new-sequence bit only when a keyframe begins with a sequence OBU.
    // Its payload stays opaque; the encoded transform restores real AV1 before decoding.
    let mut bytes = vec![if keyframe { 0x0a } else { 0x32 }];
    length(encrypted.len(), &mut bytes); bytes.extend_from_slice(encrypted); Ok(bytes)
}
pub fn unwrap(bytes: &[u8]) -> Result<&[u8], Error> {
    if !matches!(bytes.first(), Some(0x0a | 0x32)) { return Err(Error::Invalid); }
    let mut at = 1;
    let size = read_length(bytes, &mut at)?;
    if size == 0 || size > MAX || bytes.len() - at != size { return Err(Error::Invalid); }
    Ok(&bytes[at..])
}
pub fn packetize(encrypted: &[u8], keyframe: bool) -> Result<Vec<Vec<u8>>, Error> {
    if encrypted.is_empty() || encrypted.len() > MAX { return Err(Error::Limit); }
    let mut obu = Vec::with_capacity(encrypted.len() + 1);
    obu.push(if keyframe { 0x08 } else { 0x30 }); obu.extend_from_slice(encrypted);
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
            Some(0x08 | 0x30) if bytes.len() > 1 && bytes.len() <= MAX + 1 => &bytes[1..],
            Some(0x0a | 0x32) => unwrap(&bytes)?,
            _ => return Err(Error::Invalid),
        };
        Ok(Some(payload.to_vec()))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn native_camera_payload_preserves_encoded_bytes_and_rejects_bad_geometry() {
        let mut bytes = vec![1, 1]; bytes.extend_from_slice(&123u64.to_be_bytes()); bytes.push(2);
        bytes.extend_from_slice(&90u16.to_be_bytes()); bytes.extend_from_slice(&1920u16.to_be_bytes()); bytes.extend_from_slice(&1080u16.to_be_bytes());
        bytes.extend_from_slice(&[0x12, 0, 0x32, 7]);
        assert_eq!(camera_payload(&bytes).unwrap(), (90, 1920, 1080, &[0x12, 0, 0x32, 7][..]));
        for end in 0..=17 { assert!(camera_payload(&bytes[..end]).is_err()); }
        bytes[12] = 91; assert!(camera_payload(&bytes).is_err()); bytes[12] = 90;
        bytes[10] = 1; assert!(camera_payload(&bytes).is_err()); bytes[10] = 2;
        for (width, height) in [(3840u16, 2160u16), (2160, 3840)] {
            bytes[13..15].copy_from_slice(&width.to_be_bytes()); bytes[15..17].copy_from_slice(&height.to_be_bytes());
            assert_eq!(camera_payload(&bytes).unwrap(), (90, width, height, &[0x12, 0, 0x32, 7][..]));
        }
        for (width, height) in [(3840u16, 2161u16), (3841, 2160), (4096, 2048), (16, 65535), (15, 16)] {
            bytes[13..15].copy_from_slice(&width.to_be_bytes()); bytes[15..17].copy_from_slice(&height.to_be_bytes());
            assert!(camera_payload(&bytes).is_err());
        }
    }
    #[test] fn recovery_keyframes_repeat_configuration_after_the_delimiter() {
        let mut sequence = Sequence::default();
        let configured = [0x12, 0, 0x0a, 2, 7, 8, 0x32, 1, 9];
        let later = [0x12, 0, 0x32, 1, 10];
        assert!(sequence.frame(&later, true).is_err());
        assert_eq!(sequence.frame(&configured, true).unwrap().as_ref(), configured);
        assert_eq!(sequence.frame(&later, false).unwrap().as_ref(), later);
        assert_eq!(sequence.frame(&later, true).unwrap().as_ref(), [0x12, 0, 0x0a, 2, 7, 8, 0x32, 1, 10]);
        assert_eq!(sequence.frame(&[0x32, 1, 11], true).unwrap().as_ref(), [0x0a, 2, 7, 8, 0x32, 1, 11]);
        assert!(sequence.frame(&[0x0a, 4, 99], true).is_err());
        assert_eq!(sequence.frame(&later, true).unwrap().as_ref(), [0x12, 0, 0x0a, 2, 7, 8, 0x32, 1, 10]);
        sequence.frame(&[0x0a, 1, 42, 0x32, 1, 9], true).unwrap();
        assert_eq!(sequence.frame(&later, true).unwrap().as_ref(), [0x12, 0, 0x0a, 1, 42, 0x32, 1, 10]);
        sequence.clear();
        assert!(sequence.frame(&later, true).is_err());
    }
    #[test] fn sequence_rejects_malformed_obus_without_replacing_configuration() {
        let mut sequence = Sequence::default();
        let configured = [0x0e, 0, 1, 7, 0x32, 1, 9];
        assert!(matches!(sequence.frame(&configured, true).unwrap(), std::borrow::Cow::Borrowed(_)));
        for invalid in [&[0x8a, 0][..], &[0x0b, 0], &[0x08, 1, 7], &[0x0e], &[0x0e, 1, 1, 7], &[0x0a, 0x80], &[0x0a, 0xff, 0xff, 0xff, 0xff, 0], &[0x0a, 0]] {
            assert!(sequence.frame(invalid, true).is_err());
        }
        assert_eq!(sequence.frame(&[0x32, 1, 10], true).unwrap().as_ref(), [0x0e, 0, 1, 7, 0x32, 1, 10]);
    }
    #[test] fn encrypted_obu_round_trip_and_sequence_wrap() {
        for size in [1, 1100, 1101, 100_000, MAX] {
            let bytes = vec![91; size]; assert_eq!(unwrap(&wrap(&bytes, true).unwrap()).unwrap(), bytes);
            let packets = packetize(&bytes, true).unwrap(); let mut a = Assembly::default();
            for (i, p) in packets.iter().enumerate() {
                let result = a.push(65535u16.wrapping_add(i as u16), 42, i + 1 == packets.len(), p).unwrap();
                assert_eq!(result, (i + 1 == packets.len()).then(|| bytes.clone()));
            }
        }
    }
    #[test] fn browser_and_native_keyframes_preserve_the_new_sequence_marker() {
        for key in [false,true] {
            let body=vec![0xa5;4096];let browser=wrap(&body,key).unwrap();
            assert_eq!(browser[0],if key{0x0a}else{0x32});
            let packets=packetize(&body,key).unwrap();
            assert_eq!(packets[0][0]&8,if key{8}else{0});
            assert_eq!(packets[0][1],browser[0]&!2);
            assert!(packets.iter().skip(1).all(|p|p[0]&8==0));
            let mut assembly=Assembly::default();
            let mut single=vec![0x10|if key{8}else{0}];single.extend_from_slice(&browser);
            assert_eq!(assembly.push(0,90,true,&single).unwrap(),Some(body));
        }
        // Existing native peers used a frame OBU even when the RTP header marked a keyframe.
        assert_eq!(Assembly::default().push(0,90,true,&[0x18,0x30,7]).unwrap(),Some(vec![7]));
        assert_eq!(unwrap(&[0x32,1,7]).unwrap(),&[7]);
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
