use crate::checkpoint::{Reader, Writer};
use crate::Error;
use raptorq::{
    Decoder as RqDecoder, Encoder as RqEncoder, EncodingPacket, ObjectTransmissionInformation,
};
use std::collections::BTreeMap;

pub(super) const CHUNK_LEN: usize = 68;
const SYMBOL_LEN: usize = 64;
const MAX_SYMBOL_ID: u32 = 0x00ff_ffff;
const MAX_RECEIVED: usize = 64;

fn config(len: usize) -> Result<ObjectTransmissionInformation, Error> {
    // Protocol-fixed objects; no untrusted transmission parameters enter RaptorQ.
    if !matches!(len, 96 | 1536 | 1408 | 192) {
        return Err(Error::Encoding);
    }
    Ok(ObjectTransmissionInformation::new(
        len as u64,
        SYMBOL_LEN as u16,
        1,
        1,
        1,
    ))
}

#[derive(Clone)]
pub(super) struct Encoder {
    encoder: RqEncoder,
    data: Vec<u8>,
    next: u32,
}

impl Encoder {
    pub fn new(bytes: &[u8]) -> Result<Self, Error> {
        let encoder = RqEncoder::new(bytes, config(bytes.len())?);
        Ok(Self {
            encoder,
            data: bytes.to_vec(),
            next: 0,
        })
    }

    pub fn next(&mut self) -> Result<[u8; CHUNK_LEN], Error> {
        if self.next > MAX_SYMBOL_ID {
            return Err(Error::Limit);
        }
        let count = self.data.len().div_ceil(SYMBOL_LEN) as u32;
        let chunk = if self.next < count {
            let mut chunk = [0; CHUNK_LEN];
            chunk[..4].copy_from_slice(&self.next.to_be_bytes());
            let start = self.next as usize * SYMBOL_LEN;
            let end = (start + SYMBOL_LEN).min(self.data.len());
            chunk[4..4 + end - start].copy_from_slice(&self.data[start..end]);
            chunk
        } else {
            self.encoder.get_block_encoders()[0]
                .repair_packets(self.next - count, 1)
                .remove(0)
                .serialize()
                .try_into()
                .map_err(|_| Error::Encoding)?
        };
        self.next += 1;
        Ok(chunk)
    }

    pub fn write(&self, out: &mut Writer) -> Result<(), Error> {
        out.blob(&self.data)?;
        out.u32(self.next)
    }
    pub fn read(input: &mut Reader<'_>, len: usize) -> Result<Self, Error> {
        let data = input.blob(len)?;
        if data.len() != len {
            return Err(Error::Encoding);
        }
        let next = input.u32()?;
        if next > MAX_SYMBOL_ID + 1 {
            return Err(Error::Limit);
        }
        let mut encoder = Self::new(data)?;
        encoder.next = next;
        Ok(encoder)
    }
    pub fn data(&self) -> &[u8] {
        &self.data
    }
}

#[derive(Clone)]
pub(super) struct Decoder {
    decoder: RqDecoder,
    received: BTreeMap<u32, [u8; SYMBOL_LEN]>,
}

impl Decoder {
    pub fn new(len: usize) -> Result<Self, Error> {
        Ok(Self {
            decoder: RqDecoder::new(config(len)?),
            received: BTreeMap::new(),
        })
    }

    pub fn add(&mut self, chunk: &[u8; CHUNK_LEN]) -> Result<Option<Vec<u8>>, Error> {
        if chunk[0] != 0 {
            return Err(Error::Encoding);
        }
        let id = u32::from_be_bytes(chunk[..4].try_into().map_err(|_| Error::Encoding)?);
        let data = chunk[4..].try_into().map_err(|_| Error::Encoding)?;
        if let Some(prior) = self.received.get(&id) {
            return if prior == &data {
                Ok(None)
            } else {
                Err(Error::Authentication)
            };
        }
        if self.received.len() == MAX_RECEIVED {
            return Err(Error::Limit);
        }
        self.received.insert(id, data);
        Ok(self.decoder.decode(EncodingPacket::deserialize(chunk)))
    }

    pub fn write(&self, out: &mut Writer) -> Result<(), Error> {
        out.u32(self.received.len() as u32)?;
        for (id, data) in &self.received {
            out.u32(*id)?;
            out.put(data)?;
        }
        Ok(())
    }
    pub fn read(input: &mut Reader<'_>, len: usize) -> Result<Self, Error> {
        let count = input.u32()? as usize;
        if count > MAX_RECEIVED {
            return Err(Error::Limit);
        }
        let mut decoder = Self::new(len)?;
        for _ in 0..count {
            let chunk = input.take::<CHUNK_LEN>()?;
            let id = u32::from_be_bytes(chunk[..4].try_into().map_err(|_| Error::Encoding)?);
            if decoder.received.contains_key(&id) || decoder.add(&chunk)?.is_some() {
                return Err(Error::State);
            }
        }
        Ok(decoder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systematic_chunks_match_rfc_library_encoder() {
        for len in [96, 1536, 1408, 192] {
            let data: Vec<u8> = (0..len).map(|i| (i * 13) as u8).collect();
            let reference = RqEncoder::new(&data, config(len).unwrap());
            let mut encoder = Encoder::new(&data).unwrap();
            for packet in reference.get_block_encoders()[0].source_packets() {
                assert_eq!(encoder.next().unwrap().as_slice(), packet.serialize());
            }
        }
    }

    #[test]
    fn loss_reordering_and_duplicates_reconstruct_every_object() {
        for len in [96, 1536, 1408, 192] {
            let data: Vec<u8> = (0..len).map(|i| (i * 29) as u8).collect();
            let mut encoder = Encoder::new(&data).unwrap();
            let mut decoder = Decoder::new(len).unwrap();
            // Lose all systematic symbols and every third repair symbol.
            for _ in 0..len.div_ceil(SYMBOL_LEN) {
                encoder.next().unwrap();
            }
            let chunks: Vec<_> = (0..60)
                .filter_map(|i| {
                    let chunk = encoder.next().unwrap();
                    (i % 3 != 0).then_some(chunk)
                })
                .collect();
            let mut output = None;
            for chunk in chunks.iter().rev() {
                if let Some(decoded) = decoder.add(chunk).unwrap() {
                    output = Some(decoded);
                    break;
                }
                assert!(decoder.add(chunk).unwrap().is_none());
            }
            assert_eq!(output.unwrap(), data);
        }
    }

    #[test]
    fn malformed_conflicting_and_exhausted_chunks_are_bounded() {
        assert!(Encoder::new(&[0; 97]).is_err());
        let mut encoder = Encoder::new(&[0; 96]).unwrap();
        let mut decoder = Decoder::new(96).unwrap();
        let mut chunk = encoder.next().unwrap();
        chunk[0] = 1;
        assert_eq!(decoder.add(&chunk), Err(Error::Encoding));
        chunk[0] = 0;
        decoder.add(&chunk).unwrap();
        chunk[4] ^= 1;
        assert_eq!(decoder.add(&chunk), Err(Error::Authentication));
        for len in [96, 1536, 1408, 192] {
            let mut encoder = Encoder::new(&vec![0; len]).unwrap();
            encoder.next = MAX_SYMBOL_ID;
            assert_eq!(encoder.next().unwrap()[..4], [0, 255, 255, 255]);
            assert_eq!(encoder.next(), Err(Error::Limit));
        }
        // Exercise the admission bound without invoking a large decoder matrix.
        decoder.received = (0..MAX_RECEIVED as u32)
            .map(|id| (id, [0; SYMBOL_LEN]))
            .collect();
        assert_eq!(decoder.add(&[255; CHUNK_LEN]), Err(Error::Encoding));
        let mut chunk = [0; CHUNK_LEN];
        chunk[..4].copy_from_slice(&(MAX_RECEIVED as u32).to_be_bytes());
        assert_eq!(decoder.add(&chunk), Err(Error::Limit));
    }
}
