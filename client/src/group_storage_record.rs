//! Bounded chunk framing for local group records larger than one StorageKey
//! record. A fresh value identifier, total length and ordinal bind every chunk.
use super::*;
use sigil_crypto::storage::{StorageKey, MAX_RECORD};
use zeroize::Zeroizing;
const PREFIX: &[u8; 8] = b"SGGC\0\x01\0\0";
const HEADER: usize = 44;
const MAX: usize = codec::MAX_CHECKPOINT_BYTES + 1;

pub(super) const fn sealed_limit(length: usize) -> usize {
    HEADER + length + length.div_ceil(MAX_RECORD) * 36
}
pub(super) fn seal_record(
    key: &StorageKey,
    bytes: &[u8],
    context: &[u8],
) -> Result<Vec<u8>, Error> {
    if bytes.is_empty() || bytes.len() > MAX {
        return Err(Error::Limit);
    }
    let mut out = Vec::with_capacity(sealed_limit(bytes.len()));
    out.extend_from_slice(PREFIX);
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.resize(HEADER, 0);
    getrandom::fill(&mut out[12..HEADER])
        .map_err(|_| Error::Crypto(sigil_crypto::Error::Entropy))?;
    let header: [u8; HEADER] = out[..HEADER].try_into().map_err(|_| Error::InvalidStore)?;
    for (index, part) in bytes.chunks(MAX_RECORD).enumerate() {
        let aad = [context, header.as_slice(), &(index as u32).to_be_bytes()].concat();
        out.extend_from_slice(&key.seal(part, &aad)?);
    }
    Ok(out)
}
pub(super) fn open_record(
    key: &StorageKey,
    bytes: &[u8],
    context: &[u8],
) -> Result<Zeroizing<Vec<u8>>, Error> {
    if bytes.len() < HEADER || bytes.len() > sealed_limit(MAX) || &bytes[..8] != PREFIX {
        return Err(Error::InvalidStore);
    }
    let length =
        u32::from_be_bytes(bytes[8..12].try_into().map_err(|_| Error::InvalidStore)?) as usize;
    if length == 0 || length > MAX || bytes.len() != sealed_limit(length) {
        return Err(Error::InvalidStore);
    }
    let header = &bytes[..HEADER];
    let mut rest = &bytes[HEADER..];
    let mut output = Zeroizing::new(Vec::with_capacity(length));
    for index in 0..length.div_ceil(MAX_RECORD) {
        let size = (length - output.len()).min(MAX_RECORD);
        let (chunk, tail) = rest
            .split_at_checked(size + 36)
            .ok_or(Error::InvalidStore)?;
        let aad = [context, header, &(index as u32).to_be_bytes()].concat();
        let plain = key.open(chunk, &aad)?;
        if plain.len() != size {
            return Err(Error::InvalidStore);
        }
        output.extend_from_slice(&plain);
        rest = tail;
    }
    if !rest.is_empty() || output.len() != length {
        return Err(Error::InvalidStore);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sigil_crypto::Secret32;
    #[test]
    fn chunks_reject_cross_record_splicing_wrong_context_and_malformed_framing() {
        let key = StorageKey::new(Secret32::from_bytes([7; 32])).unwrap();
        let plain = vec![9; MAX_RECORD + 1];
        let a = seal_record(&key, &plain, b"synthetic group").unwrap();
        let b = seal_record(&key, &plain, b"synthetic group").unwrap();
        assert_eq!(
            open_record(&key, &a, b"synthetic group")
                .unwrap()
                .as_slice(),
            plain
        );
        assert_ne!(a, b);
        assert!(open_record(&key, &a, b"different group").is_err());
        let mut spliced = a.clone();
        let last = HEADER + MAX_RECORD + 36;
        spliced[last..].copy_from_slice(&b[last..]);
        assert!(open_record(&key, &spliced, b"synthetic group").is_err());
        for i in [0, 8, 12, HEADER, last, a.len() - 1] {
            let mut bad = a.clone();
            bad[i] ^= 1;
            assert!(open_record(&key, &bad, b"synthetic group").is_err());
        }
        for n in [0, 7, 12, HEADER, last, a.len() - 1] {
            assert!(open_record(&key, &a[..n], b"synthetic group").is_err());
        }
        let mut bad = a;
        bad.push(0);
        assert!(open_record(&key, &bad, b"synthetic group").is_err());
        assert!(seal_record(&key, &[], b"synthetic group").is_err());
        assert!(seal_record(&key, &vec![0; MAX + 1], b"synthetic group").is_err());
    }
}
