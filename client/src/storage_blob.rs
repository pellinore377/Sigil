use crate::{Error, StorageKey, Zeroizing};
const CHUNK: usize = 65536;
pub(crate) fn seal(key: &StorageKey, binding: &[u8], bytes: &[u8]) -> Result<Vec<u8>, Error> {
    if bytes.is_empty() || bytes.len() > 2 * 1024 * 1024 {
        return Err(Error::Limit);
    }
    let mut revision = [0; 32];
    getrandom::fill(&mut revision).map_err(|_| sigil_crypto::Error::Entropy)?;
    let mut framed = revision.to_vec();
    framed.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    let context = [binding, framed.as_slice()].concat();
    for (index, chunk) in bytes.chunks(CHUNK).enumerate() {
        framed.extend_from_slice(&key.seal(
            chunk,
            &[context.as_slice(), &(index as u32).to_be_bytes()].concat(),
        )?);
    }
    Ok(framed)
}
pub(crate) fn open(
    key: &StorageKey,
    binding: &[u8],
    framed: &[u8],
    limit: usize,
) -> Result<Zeroizing<Vec<u8>>, Error> {
    let header = framed.get(..40).ok_or(Error::InvalidStore)?;
    let length = usize::try_from(u64::from_be_bytes(
        header[32..40].try_into().map_err(|_| Error::InvalidStore)?,
    ))
    .map_err(|_| Error::InvalidStore)?;
    if length == 0
        || length > limit.min(2 * 1024 * 1024)
        || framed.len() != 40 + length + length.div_ceil(CHUNK) * 36
    {
        return Err(Error::InvalidStore);
    }
    let context = [binding, header].concat();
    let mut bytes = Zeroizing::new(Vec::with_capacity(length));
    for (index, chunk) in framed[40..].chunks(CHUNK + 36).enumerate() {
        bytes.extend_from_slice(&key.open(
            chunk,
            &[context.as_slice(), &(index as u32).to_be_bytes()].concat(),
        )?);
    }
    if bytes.len() != length {
        return Err(Error::InvalidStore);
    }
    Ok(bytes)
}
