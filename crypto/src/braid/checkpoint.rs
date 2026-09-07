use super::*;
use crate::checkpoint::{Reader, Writer};

fn write_key(key: &Key, out: &mut Writer) -> Result<(), Error> {
    out.put(key.seed.as_ref())
}
fn read_key(input: &mut Reader<'_>) -> Result<Box<Key>, Error> {
    Ok(Box::new(Key::from_seed(&Zeroizing::new(input.take()?))))
}
fn write_encaps(
    encaps: &Encapsulation,
    ct1: &[u8; CT1_LEN],
    out: &mut Writer,
) -> Result<(), Error> {
    out.put(&encaps.header)?;
    out.put(encaps.randomness.as_ref())?;
    out.put(ct1)
}
fn read_encaps(input: &mut Reader<'_>) -> Result<(Box<Encapsulation>, Box<[u8; CT1_LEN]>), Error> {
    let header = input.take()?;
    let randomness = Zeroizing::new(input.take()?);
    let ct1 = Box::new(input.take()?);
    let (encaps, expected, _) = Encapsulation::with_randomness(&header, &randomness)?;
    if *ct1 != expected {
        return Err(Error::State);
    }
    Ok((Box::new(encaps), ct1))
}
fn matches_data(encoder: &Encoder, expected: &[u8]) -> Result<(), Error> {
    if encoder.data() == expected {
        Ok(())
    } else {
        Err(Error::State)
    }
}

impl Scka {
    pub(crate) fn write(&self, out: &mut Writer) -> Result<(), Error> {
        out.u64(self.epoch)?;
        out.put(self.auth.root.as_ref())?;
        out.put(self.auth.mac.as_ref())?;
        match &self.stage {
            Stage::KeysUnsampled => out.u8(0)?,
            Stage::KeysSampled { key, header } => {
                out.u8(1)?;
                write_key(key, out)?;
                header.write(out)?;
            }
            Stage::HeaderSent { key, ct1, vector } => {
                out.u8(2)?;
                write_key(key, out)?;
                ct1.write(out)?;
                vector.write(out)?;
            }
            Stage::Ct1Received { key, ct1, vector } => {
                out.u8(3)?;
                write_key(key, out)?;
                out.put(ct1.as_slice())?;
                vector.write(out)?;
            }
            Stage::EkSentCt1Received { key, ct1, ct2 } => {
                out.u8(4)?;
                write_key(key, out)?;
                out.put(ct1.as_slice())?;
                ct2.write(out)?;
            }
            Stage::NoHeaderReceived { header } => {
                out.u8(5)?;
                header.write(out)?;
            }
            Stage::HeaderReceived { header } => {
                out.u8(6)?;
                out.put(header)?;
            }
            Stage::Ct1Sampled {
                encaps,
                ct1,
                encoder,
                vector,
            } => {
                out.u8(7)?;
                write_encaps(encaps, ct1, out)?;
                encoder.write(out)?;
                vector.write(out)?;
            }
            Stage::EkReceivedCt1Sampled {
                encaps,
                ct1,
                vector,
                encoder,
            } => {
                out.u8(8)?;
                write_encaps(encaps, ct1, out)?;
                out.put(vector.as_slice())?;
                encoder.write(out)?;
            }
            Stage::Ct1Acknowledged {
                encaps,
                ct1,
                vector,
            } => {
                out.u8(9)?;
                write_encaps(encaps, ct1, out)?;
                vector.write(out)?;
            }
            Stage::Ct2Sampled { encoder } => {
                out.u8(10)?;
                encoder.write(out)?;
            }
        }
        Ok(())
    }

    pub(crate) fn read(input: &mut Reader<'_>) -> Result<Self, Error> {
        let epoch = input.u64()?;
        if epoch == 0 {
            return Err(Error::State);
        }
        let auth = Auth {
            root: Zeroizing::new(input.take()?),
            mac: Zeroizing::new(input.take()?),
        };
        let stage = match input.u8()? {
            0 => Stage::KeysUnsampled,
            1 => {
                let key = read_key(input)?;
                let header = Encoder::read(input, HEADER_LEN + MAC_LEN)?;
                let public = key.header();
                let tag = auth.tag(b":ekheader", epoch, &[&public])?;
                matches_data(&header, &[public.as_slice(), &tag].concat())?;
                Stage::KeysSampled { key, header }
            }
            2 => {
                let key = read_key(input)?;
                let ct1 = Decoder::read(input, CT1_LEN)?;
                let vector = Encoder::read(input, VECTOR_LEN)?;
                matches_data(&vector, &key.vector())?;
                Stage::HeaderSent { key, ct1, vector }
            }
            3 => {
                let key = read_key(input)?;
                let ct1 = Box::new(input.take()?);
                let vector = Encoder::read(input, VECTOR_LEN)?;
                matches_data(&vector, &key.vector())?;
                Stage::Ct1Received { key, ct1, vector }
            }
            4 => Stage::EkSentCt1Received {
                key: read_key(input)?,
                ct1: Box::new(input.take()?),
                ct2: Decoder::read(input, CT2_LEN + MAC_LEN)?,
            },
            5 => Stage::NoHeaderReceived {
                header: Decoder::read(input, HEADER_LEN + MAC_LEN)?,
            },
            6 => Stage::HeaderReceived {
                header: input.take()?,
            },
            7 => {
                let (encaps, ct1) = read_encaps(input)?;
                let encoder = Encoder::read(input, CT1_LEN)?;
                matches_data(&encoder, ct1.as_slice())?;
                Stage::Ct1Sampled {
                    encaps,
                    ct1,
                    encoder,
                    vector: Decoder::read(input, VECTOR_LEN)?,
                }
            }
            8 => {
                let (encaps, ct1) = read_encaps(input)?;
                let vector = Box::new(input.take()?);
                encaps.validate(&vector)?;
                let encoder = Encoder::read(input, CT1_LEN)?;
                matches_data(&encoder, ct1.as_slice())?;
                Stage::EkReceivedCt1Sampled {
                    encaps,
                    ct1,
                    vector,
                    encoder,
                }
            }
            9 => {
                let (encaps, ct1) = read_encaps(input)?;
                Stage::Ct1Acknowledged {
                    encaps,
                    ct1,
                    vector: Decoder::read(input, VECTOR_LEN)?,
                }
            }
            10 => Stage::Ct2Sampled {
                encoder: Encoder::read(input, CT2_LEN + MAC_LEN)?,
            },
            _ => return Err(Error::Encoding),
        };
        Ok(Self { epoch, auth, stage })
    }

    pub(crate) fn validate_epoch(&self, epoch: u64, bob: bool) -> Result<(), Error> {
        let encapsulator = matches!(
            self.stage,
            Stage::NoHeaderReceived { .. }
                | Stage::HeaderReceived { .. }
                | Stage::Ct1Sampled { .. }
                | Stage::EkReceivedCt1Sampled { .. }
                | Stage::Ct1Acknowledged { .. }
                | Stage::Ct2Sampled { .. }
        );
        if encapsulator != (bob == (self.epoch % 2 == 1)) {
            return Err(Error::State);
        }
        let produced = matches!(
            self.stage,
            Stage::Ct1Sampled { .. }
                | Stage::EkReceivedCt1Sampled { .. }
                | Stage::Ct1Acknowledged { .. }
                | Stage::Ct2Sampled { .. }
        );
        let expected = if produced { self.epoch } else { self.epoch - 1 };
        if epoch != expected {
            return Err(Error::State);
        }
        Ok(())
    }
}
