//! SPE, the Sigil Packed Encoding: the only serialisation the protocol
//! uses. It is deterministic and has no schema language.
//!
//! - integers: little-endian, fixed width (`u8`, `u16`, `u32`, `u64`)
//! - fixed-size byte arrays: raw
//! - variable bytes and strings: `u32` little-endian length, then bytes
//! - structures: fields concatenated in the order the spec lists them
//!
//! Nothing is self-describing; the reader must know the layout.

#[derive(Default)]
pub struct Writer(Vec<u8>);

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn u8(mut self, v: u8) -> Self {
        self.0.push(v);
        self
    }
    pub fn u16(mut self, v: u16) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn u32(mut self, v: u32) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn u64(mut self, v: u64) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    /// Fixed-size field: written raw, no length.
    pub fn fixed(mut self, v: &[u8]) -> Self {
        self.0.extend_from_slice(v);
        self
    }
    /// Variable-size field: `u32` length prefix.
    pub fn bytes(mut self, v: &[u8]) -> Self {
        self.0.extend_from_slice(&(v.len() as u32).to_le_bytes());
        self.0.extend_from_slice(v);
        self
    }
    pub fn str(self, v: &str) -> Self {
        self.bytes(v.as_bytes())
    }
    pub fn finish(self) -> Vec<u8> {
        self.0
    }
}

pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    fn take(&mut self, n: usize) -> crate::Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or(crate::Error::Malformed)?;
        if end > self.buf.len() {
            return Err(crate::Error::Malformed);
        }
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    pub fn u8(&mut self) -> crate::Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> crate::Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn u32(&mut self) -> crate::Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn u64(&mut self) -> crate::Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub fn fixed<const N: usize>(&mut self) -> crate::Result<[u8; N]> {
        Ok(self.take(N)?.try_into().unwrap())
    }
    pub fn bytes(&mut self) -> crate::Result<&'a [u8]> {
        let n = self.u32()? as usize;
        self.take(n)
    }
    pub fn str(&mut self) -> crate::Result<&'a str> {
        core::str::from_utf8(self.bytes()?).map_err(|_| crate::Error::Malformed)
    }
    pub fn rest(&mut self) -> &'a [u8] {
        let s = &self.buf[self.pos..];
        self.pos = self.buf.len();
        s
    }
    pub fn done(&self) -> crate::Result<()> {
        if self.pos == self.buf.len() {
            Ok(())
        } else {
            Err(crate::Error::Malformed)
        }
    }
}
