//! Internal bounded codec. Raw secret checkpoints never cross the crate API.
use crate::{Error, Secret32};
use zeroize::Zeroizing;

pub(crate) const MAX_CHECKPOINT: usize = 32768;
pub(crate) struct Writer(Zeroizing<Vec<u8>>, usize);
impl Writer {
    pub fn new() -> Self {
        Self(
            Zeroizing::new(Vec::with_capacity(MAX_CHECKPOINT)),
            MAX_CHECKPOINT,
        )
    }
    pub fn archive() -> Self {
        Self(
            Zeroizing::new(Vec::with_capacity(crate::storage::MAX_RECORD)),
            crate::storage::MAX_RECORD,
        )
    }
    pub fn put(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if bytes.len() > self.1 - self.0.len() {
            return Err(Error::Limit);
        }
        self.0.extend_from_slice(bytes);
        Ok(())
    }
    pub fn u8(&mut self, value: u8) -> Result<(), Error> {
        self.put(&[value])
    }
    pub fn u32(&mut self, value: u32) -> Result<(), Error> {
        self.put(&value.to_be_bytes())
    }
    pub fn u64(&mut self, value: u64) -> Result<(), Error> {
        self.put(&value.to_be_bytes())
    }
    pub fn blob(&mut self, value: &[u8]) -> Result<(), Error> {
        self.u32(value.len().try_into().map_err(|_| Error::Limit)?)?;
        self.put(value)
    }
    pub fn finish(self) -> Zeroizing<Vec<u8>> {
        self.0
    }
}

pub(crate) struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    pub fn archive(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.len() > crate::storage::MAX_RECORD {
            return Err(Error::Limit);
        }
        Ok(Self(bytes))
    }
    pub fn new(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_CHECKPOINT {
            return Err(Error::Limit);
        }
        Ok(Self(bytes))
    }
    fn slice(&mut self, len: usize) -> Result<&'a [u8], Error> {
        if len > self.0.len() {
            return Err(Error::Encoding);
        }
        let (head, tail) = self.0.split_at(len);
        self.0 = tail;
        Ok(head)
    }
    pub fn take<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        self.slice(N)?.try_into().map_err(|_| Error::Encoding)
    }
    pub fn secret(&mut self) -> Result<Secret32, Error> {
        Ok(Secret32::from_bytes(self.take()?))
    }
    pub fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.take::<1>()?[0])
    }
    pub fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_be_bytes(self.take()?))
    }
    pub fn u64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_be_bytes(self.take()?))
    }
    pub fn blob(&mut self, max: usize) -> Result<&'a [u8], Error> {
        let len = self.u32()? as usize;
        if len > max {
            return Err(Error::Limit);
        }
        self.slice(len)
    }
    pub fn finish(self) -> Result<(), Error> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(Error::Encoding)
        }
    }
}
