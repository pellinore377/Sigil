//! Experimental ML-KEM Braid rev. 1 state machine. No enabled wire suite.
//! Every fragment must be bound into the outer ratchet's AEAD. The outer
//! receiver must stage this state and commit it only after AEAD verification.

mod checkpoint;
mod chunks;
mod incremental;
use crate::{Error, Secret32};
use chunks::{Decoder, Encoder, CHUNK_LEN};
use hkdf::Hkdf;
use hmac::{Hmac, KeyInit, Mac};
use incremental::{Encapsulation, Key, CT1_LEN, CT2_LEN, HEADER_LEN, VECTOR_LEN};
use sha2::Sha256;
use zeroize::Zeroizing;

const INFO: &[u8] = b"Sigil/experimental/braid/v0_MLKEM1024_SHA-256_RaptorQ64";
const MAC_LEN: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum Kind {
    None = 0,
    Hdr = 1,
    Ek = 2,
    EkCt1Ack = 3,
    Ct1 = 5,
    Ct2 = 6,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    epoch: u64,
    kind: Kind,
    chunk: Option<[u8; CHUNK_LEN]>,
}

impl Message {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(9 + CHUNK_LEN);
        bytes.extend_from_slice(&self.epoch.to_be_bytes());
        bytes.push(self.kind as u8);
        if let Some(chunk) = self.chunk {
            bytes.extend_from_slice(&chunk);
        }
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() != 9 && bytes.len() != 9 + CHUNK_LEN {
            return Err(Error::Encoding);
        }
        let epoch = u64::from_be_bytes(bytes[..8].try_into().map_err(|_| Error::Encoding)?);
        if epoch == 0 {
            return Err(Error::Encoding);
        }
        let kind = match bytes[8] {
            0 => Kind::None,
            1 => Kind::Hdr,
            2 => Kind::Ek,
            3 => Kind::EkCt1Ack,
            5 => Kind::Ct1,
            6 => Kind::Ct2,
            _ => return Err(Error::Encoding),
        };
        let chunk = match kind {
            Kind::None if bytes.len() == 9 => None,
            Kind::Hdr | Kind::Ek | Kind::EkCt1Ack | Kind::Ct1 | Kind::Ct2
                if bytes.len() == 9 + CHUNK_LEN =>
            {
                if bytes[9] != 0 {
                    return Err(Error::Encoding);
                }
                Some(bytes[9..].try_into().map_err(|_| Error::Encoding)?)
            }
            _ => return Err(Error::Encoding),
        };
        Ok(Self { epoch, kind, chunk })
    }

    fn data(&self) -> Result<&[u8; CHUNK_LEN], Error> {
        self.chunk.as_ref().ok_or(Error::Encoding)
    }
}

pub struct Output {
    pub epoch: u64,
    pub key: Secret32,
}

#[derive(Clone)]
struct Auth {
    root: Zeroizing<[u8; 32]>,
    mac: Zeroizing<[u8; 32]>,
}

fn info(label: &[u8], epoch: u64) -> Vec<u8> {
    [INFO, label, &epoch.to_be_bytes()].concat()
}

fn output_key(raw: Secret32, epoch: u64) -> Result<Secret32, Error> {
    let mut output = Zeroizing::new([0; 32]);
    Hkdf::<Sha256>::new(Some(&[0; 32]), raw.0.as_ref())
        .expand(&info(b":SCKA Key", epoch), output.as_mut())
        .map_err(|_| Error::State)?;
    Ok(Secret32(output))
}

impl Auth {
    fn new(shared: &Secret32) -> Result<Self, Error> {
        let mut auth = Self {
            root: Zeroizing::new([0; 32]),
            mac: Zeroizing::new([0; 32]),
        };
        auth.update(1, shared)?;
        Ok(auth)
    }

    fn update(&mut self, epoch: u64, key: &Secret32) -> Result<(), Error> {
        let mut output = Zeroizing::new([0; 64]);
        Hkdf::<Sha256>::new(Some(self.root.as_ref()), key.0.as_ref())
            .expand(&info(b":Authenticator Update", epoch), output.as_mut())
            .map_err(|_| Error::State)?;
        self.root.copy_from_slice(&output[..32]);
        self.mac.copy_from_slice(&output[32..]);
        Ok(())
    }

    fn tagger(&self, label: &[u8], epoch: u64, parts: &[&[u8]]) -> Result<Hmac<Sha256>, Error> {
        let mut mac =
            Hmac::<Sha256>::new_from_slice(self.mac.as_ref()).map_err(|_| Error::State)?;
        mac.update(&info(label, epoch));
        for part in parts {
            mac.update(part);
        }
        Ok(mac)
    }

    fn tag(&self, label: &[u8], epoch: u64, parts: &[&[u8]]) -> Result<[u8; MAC_LEN], Error> {
        Ok(self
            .tagger(label, epoch, parts)?
            .finalize()
            .into_bytes()
            .into())
    }

    fn verify(&self, label: &[u8], epoch: u64, parts: &[&[u8]], tag: &[u8]) -> Result<(), Error> {
        self.tagger(label, epoch, parts)?
            .verify_slice(tag)
            .map_err(|_| Error::Authentication)
    }
}

#[derive(Clone)]
enum Stage {
    KeysUnsampled,
    KeysSampled {
        key: Box<Key>,
        header: Encoder,
    },
    HeaderSent {
        key: Box<Key>,
        ct1: Decoder,
        vector: Encoder,
    },
    Ct1Received {
        key: Box<Key>,
        ct1: Box<[u8; CT1_LEN]>,
        vector: Encoder,
    },
    EkSentCt1Received {
        key: Box<Key>,
        ct1: Box<[u8; CT1_LEN]>,
        ct2: Decoder,
    },
    NoHeaderReceived {
        header: Decoder,
    },
    HeaderReceived {
        header: [u8; HEADER_LEN],
    },
    Ct1Sampled {
        encaps: Box<Encapsulation>,
        ct1: Box<[u8; CT1_LEN]>,
        encoder: Encoder,
        vector: Decoder,
    },
    EkReceivedCt1Sampled {
        encaps: Box<Encapsulation>,
        ct1: Box<[u8; CT1_LEN]>,
        vector: Box<[u8; VECTOR_LEN]>,
        encoder: Encoder,
    },
    Ct1Acknowledged {
        encaps: Box<Encapsulation>,
        ct1: Box<[u8; CT1_LEN]>,
        vector: Decoder,
    },
    Ct2Sampled {
        encoder: Encoder,
    },
}

pub struct Scka {
    epoch: u64,
    auth: Auth,
    stage: Stage,
}

impl Scka {
    pub fn alice(shared: &Secret32) -> Result<Self, Error> {
        Ok(Self {
            epoch: 1,
            auth: Auth::new(shared)?,
            stage: Stage::KeysUnsampled,
        })
    }

    pub fn bob(shared: &Secret32) -> Result<Self, Error> {
        Ok(Self {
            epoch: 1,
            auth: Auth::new(shared)?,
            stage: Stage::NoHeaderReceived {
                header: Decoder::new(HEADER_LEN + MAC_LEN)?,
            },
        })
    }

    pub(crate) fn candidate(&self) -> Self {
        Self {
            epoch: self.epoch,
            auth: self.auth.clone(),
            stage: self.stage.clone(),
        }
    }

    pub fn send(&mut self) -> Result<(Message, u64, Option<Output>), Error> {
        let mut candidate = self.candidate();
        let result = candidate.send_inner()?;
        *self = candidate;
        Ok(result)
    }

    fn send_inner(&mut self) -> Result<(Message, u64, Option<Output>), Error> {
        let mut output = None;
        if matches!(self.stage, Stage::KeysUnsampled) {
            let key = Box::new(Key::generate()?);
            let header = key.header();
            let tag = self.auth.tag(b":ekheader", self.epoch, &[&header])?;
            self.stage = Stage::KeysSampled {
                key,
                header: Encoder::new(&[header.as_slice(), &tag].concat())?,
            };
        } else if let Stage::HeaderReceived { header } = &self.stage {
            let (encaps, ct1, raw) = Encapsulation::start(header)?;
            let key = output_key(raw, self.epoch)?;
            self.auth.update(self.epoch, &key)?;
            self.stage = Stage::Ct1Sampled {
                encaps: Box::new(encaps),
                ct1: Box::new(ct1),
                encoder: Encoder::new(&ct1)?,
                vector: Decoder::new(VECTOR_LEN)?,
            };
            output = Some(Output {
                epoch: self.epoch,
                key,
            });
        }
        let (kind, chunk) = match &mut self.stage {
            Stage::KeysSampled { header, .. } => (Kind::Hdr, Some(header.next()?)),
            Stage::HeaderSent { vector, .. } => (Kind::Ek, Some(vector.next()?)),
            Stage::Ct1Received { vector, .. } => (Kind::EkCt1Ack, Some(vector.next()?)),
            Stage::Ct1Sampled { encoder, .. } | Stage::EkReceivedCt1Sampled { encoder, .. } => {
                (Kind::Ct1, Some(encoder.next()?))
            }
            Stage::Ct2Sampled { encoder } => (Kind::Ct2, Some(encoder.next()?)),
            Stage::EkSentCt1Received { .. }
            | Stage::NoHeaderReceived { .. }
            | Stage::Ct1Acknowledged { .. } => (Kind::None, None),
            Stage::KeysUnsampled | Stage::HeaderReceived { .. } => return Err(Error::State),
        };
        Ok((
            Message {
                epoch: self.epoch,
                kind,
                chunk,
            },
            self.epoch - 1,
            output,
        ))
    }

    pub fn receive(&mut self, message: &Message) -> Result<(u64, Option<Output>), Error> {
        let mut candidate = self.candidate();
        let result = candidate.receive_inner(message)?;
        *self = candidate;
        Ok(result)
    }

    fn finish_encapsulation(
        &self,
        encaps: Encapsulation,
        ct1: &[u8; CT1_LEN],
        vector: &[u8; VECTOR_LEN],
    ) -> Result<Stage, Error> {
        let ct2 = encaps.finish(vector)?;
        let tag = self.auth.tag(b":ciphertext", self.epoch, &[ct1, &ct2])?;
        Ok(Stage::Ct2Sampled {
            encoder: Encoder::new(&[ct2.as_slice(), &tag].concat())?,
        })
    }

    fn receive_inner(&mut self, msg: &Message) -> Result<(u64, Option<Output>), Error> {
        // The authenticated message carries its sending epoch. Returning the
        // current local epoch for delayed fragments would break SCKA epoch agreement.
        let receiving_epoch = msg.epoch.checked_sub(1).ok_or(Error::Encoding)?;
        if msg.epoch < self.epoch {
            return Ok((receiving_epoch, None));
        }
        if msg.epoch > self.epoch {
            if self.epoch.checked_add(1) == Some(msg.epoch)
                && matches!(self.stage, Stage::Ct2Sampled { .. })
            {
                self.epoch = msg.epoch;
                self.stage = Stage::KeysUnsampled;
                return Ok((receiving_epoch, None));
            }
            return Err(Error::State);
        }
        let mut output = None;
        self.stage = match std::mem::replace(&mut self.stage, Stage::KeysUnsampled) {
            Stage::KeysSampled { key, .. } if msg.kind == Kind::Ct1 => {
                let mut ct1 = Decoder::new(CT1_LEN)?;
                ct1.add(msg.data()?)?;
                let vector = Encoder::new(&key.vector())?;
                Stage::HeaderSent { key, ct1, vector }
            }
            Stage::HeaderSent {
                key,
                mut ct1,
                vector,
            } if msg.kind == Kind::Ct1 => {
                if let Some(bytes) = ct1.add(msg.data()?)? {
                    Stage::Ct1Received {
                        key,
                        ct1: bytes
                            .into_boxed_slice()
                            .try_into()
                            .map_err(|_| Error::Encoding)?,
                        vector,
                    }
                } else {
                    Stage::HeaderSent { key, ct1, vector }
                }
            }
            Stage::Ct1Received { key, ct1, .. } if msg.kind == Kind::Ct2 => {
                let mut ct2 = Decoder::new(CT2_LEN + MAC_LEN)?;
                ct2.add(msg.data()?)?;
                Stage::EkSentCt1Received { key, ct1, ct2 }
            }
            Stage::EkSentCt1Received { key, ct1, mut ct2 } if msg.kind == Kind::Ct2 => {
                if let Some(bytes) = ct2.add(msg.data()?)? {
                    let ciphertext = bytes[..CT2_LEN].try_into().map_err(|_| Error::Encoding)?;
                    let secret = output_key(key.decapsulate(&ct1, ciphertext), self.epoch)?;
                    self.auth.update(self.epoch, &secret)?;
                    self.auth.verify(
                        b":ciphertext",
                        self.epoch,
                        &[ct1.as_slice(), ciphertext],
                        &bytes[CT2_LEN..],
                    )?;
                    output = Some(Output {
                        epoch: self.epoch,
                        key: secret,
                    });
                    self.epoch = self.epoch.checked_add(1).ok_or(Error::Limit)?;
                    Stage::NoHeaderReceived {
                        header: Decoder::new(HEADER_LEN + MAC_LEN)?,
                    }
                } else {
                    Stage::EkSentCt1Received { key, ct1, ct2 }
                }
            }
            Stage::NoHeaderReceived { mut header } if msg.kind == Kind::Hdr => {
                if let Some(bytes) = header.add(msg.data()?)? {
                    self.auth.verify(
                        b":ekheader",
                        self.epoch,
                        &[&bytes[..HEADER_LEN]],
                        &bytes[HEADER_LEN..],
                    )?;
                    Stage::HeaderReceived {
                        header: bytes[..HEADER_LEN]
                            .try_into()
                            .map_err(|_| Error::Encoding)?,
                    }
                } else {
                    Stage::NoHeaderReceived { header }
                }
            }
            Stage::Ct1Sampled {
                encaps,
                ct1,
                encoder,
                mut vector,
            } if matches!(msg.kind, Kind::Ek | Kind::EkCt1Ack) => {
                if let Some(bytes) = vector.add(msg.data()?)? {
                    let vector: Box<[u8; VECTOR_LEN]> = bytes
                        .into_boxed_slice()
                        .try_into()
                        .map_err(|_| Error::Encoding)?;
                    encaps.validate(&vector)?;
                    if msg.kind == Kind::EkCt1Ack {
                        self.finish_encapsulation(*encaps, &ct1, &vector)?
                    } else {
                        Stage::EkReceivedCt1Sampled {
                            encaps,
                            ct1,
                            vector,
                            encoder,
                        }
                    }
                } else if msg.kind == Kind::EkCt1Ack {
                    Stage::Ct1Acknowledged {
                        encaps,
                        ct1,
                        vector,
                    }
                } else {
                    Stage::Ct1Sampled {
                        encaps,
                        ct1,
                        encoder,
                        vector,
                    }
                }
            }
            Stage::EkReceivedCt1Sampled {
                encaps,
                ct1,
                vector,
                ..
            } if msg.kind == Kind::EkCt1Ack => self.finish_encapsulation(*encaps, &ct1, &vector)?,
            Stage::Ct1Acknowledged {
                encaps,
                ct1,
                mut vector,
            } if msg.kind == Kind::EkCt1Ack => {
                if let Some(bytes) = vector.add(msg.data()?)? {
                    self.finish_encapsulation(
                        *encaps,
                        &ct1,
                        bytes.as_slice().try_into().map_err(|_| Error::Encoding)?,
                    )?
                } else {
                    Stage::Ct1Acknowledged {
                        encaps,
                        ct1,
                        vector,
                    }
                }
            }
            unchanged => unchanged,
        };
        Ok((receiving_epoch, output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn authenticator_and_output_kdfs_match_openssl() {
        let expected = crate::vectors::ratchet_vector;
        let auth = Auth::new(&Secret32::from_bytes([1; 32])).unwrap();
        assert_eq!(
            [auth.root.as_slice(), auth.mac.as_slice()].concat(),
            expected("braid_init")
        );
        let mut auth = Auth {
            root: Zeroizing::new([3; 32]),
            mac: Zeroizing::new([0; 32]),
        };
        auth.update(7, &Secret32::from_bytes([2; 32])).unwrap();
        assert_eq!(
            [auth.root.as_slice(), auth.mac.as_slice()].concat(),
            expected("braid_auth")
        );
        assert_eq!(
            output_key(Secret32::from_bytes([2; 32]), 7)
                .unwrap()
                .0
                .as_slice(),
            expected("braid_output")
        );
        auth.mac.fill(4);
        assert_eq!(
            auth.tag(b":ekheader", 7, &[&[5; 64]]).unwrap().as_slice(),
            expected("braid_header_mac")
        );
        assert_eq!(
            auth.tag(b":ciphertext", 7, &[&[6; 1408], &[7; 160]])
                .unwrap()
                .as_slice(),
            expected("braid_ciphertext_mac")
        );
    }

    fn pair() -> [Scka; 2] {
        let shared = Secret32::from_bytes([91; 32]);
        [Scka::alice(&shared).unwrap(), Scka::bob(&shared).unwrap()]
    }

    fn record(keys: &mut BTreeMap<u64, [u8; 32]>, output: Option<Output>) {
        if let Some(output) = output {
            assert_eq!(output.epoch, keys.len() as u64 + 1);
            assert!(keys.insert(output.epoch, *output.key.0).is_none());
        }
    }

    fn stage(scka: &Scka) -> usize {
        match scka.stage {
            Stage::KeysUnsampled => 0,
            Stage::KeysSampled { .. } => 1,
            Stage::HeaderSent { .. } => 2,
            Stage::Ct1Received { .. } => 3,
            Stage::EkSentCt1Received { .. } => 4,
            Stage::NoHeaderReceived { .. } => 5,
            Stage::HeaderReceived { .. } => 6,
            Stage::Ct1Sampled { .. } => 7,
            Stage::EkReceivedCt1Sampled { .. } => 8,
            Stage::Ct1Acknowledged { .. } => 9,
            Stage::Ct2Sampled { .. } => 10,
        }
    }

    fn restore_once(scka: &mut Scka, restored: &mut [bool; 11]) {
        let id = stage(scka);
        if restored[id] {
            return;
        }
        let mut out = crate::checkpoint::Writer::new();
        scka.write(&mut out).unwrap();
        let bytes = out.finish();
        let mut input = crate::checkpoint::Reader::new(&bytes).unwrap();
        *scka = Scka::read(&mut input).unwrap();
        input.finish().unwrap();
        let mut out = crate::checkpoint::Writer::new();
        scka.write(&mut out).unwrap();
        assert_eq!(*out.finish(), *bytes);
        restored[id] = true;
    }

    #[test]
    fn repeated_rotations_with_loss_reordering_and_asymmetric_sends() {
        let mut seen = [false; 11];
        let mut restored = [false; 11];
        for seed in 1..=4u64 {
            let mut peers = pair();
            let mut keys = [BTreeMap::new(), BTreeMap::new()];
            let mut queue = Vec::new();
            let mut random = seed;
            // Deterministic traffic scheduling only; never used for cryptographic randomness.
            let mut next = || {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                random
            };
            for _ in 0..2500 {
                let sender = (next() % 2) as usize;
                seen[stage(&peers[sender])] = true;
                restore_once(&mut peers[sender], &mut restored);
                let (message, epoch, output) = peers[sender].send().unwrap();
                seen[stage(&peers[sender])] = true;
                restore_once(&mut peers[sender], &mut restored);
                record(&mut keys[sender], output);
                assert_eq!(Message::from_bytes(&message.to_bytes()).unwrap(), message);
                if next() % 5 != 0 {
                    queue.push((1 - sender, message, epoch));
                }
                if !queue.is_empty() && (next() % 3 != 0 || queue.len() > 16) {
                    let index = next() as usize % queue.len();
                    let (receiver, message, sent_epoch) = queue.remove(index);
                    let (received_epoch, output) = peers[receiver].receive(&message).unwrap();
                    assert_eq!(received_epoch, sent_epoch);
                    record(&mut keys[receiver], output);
                    seen[stage(&peers[receiver])] = true;
                    restore_once(&mut peers[receiver], &mut restored);
                }
            }
            assert!(keys[0].len() >= 5 && keys[1].len() >= 5);
            for (epoch, key) in &keys[0] {
                if let Some(other) = keys[1].get(epoch) {
                    assert_eq!(key, other);
                }
            }
        }
        assert!(
            seen.into_iter().all(|visited| visited),
            "every Braid state exercised"
        );
        assert!(restored.into_iter().all(|visited| visited));
    }

    #[test]
    fn header_authentication_failure_does_not_commit_candidate() {
        let [mut alice, mut bob] = pair();
        let first = alice.send().unwrap().0;
        bob.receive(&first).unwrap();
        let last = alice.send().unwrap().0;
        let mut corrupted = last.clone();
        corrupted.chunk.as_mut().unwrap()[4] ^= 1;
        assert!(matches!(
            bob.receive(&corrupted),
            Err(Error::Authentication)
        ));
        assert!(matches!(bob.stage, Stage::NoHeaderReceived { .. }));
        bob.receive(&last).unwrap();
        assert!(matches!(bob.stage, Stage::HeaderReceived { .. }));
        assert!(bob.send().unwrap().2.is_some());
    }

    #[test]
    fn ciphertext_authentication_failure_does_not_advance_epoch() {
        let [mut alice, mut bob] = pair();
        let mut reached = false;
        for _ in 0..100 {
            let from_alice = alice.send().unwrap().0;
            bob.receive(&from_alice).unwrap();
            let from_bob = bob.send().unwrap().0;
            if from_bob.kind == Kind::Ct2 && from_bob.chunk.as_ref().unwrap()[3] == 2 {
                let mut corrupted = from_bob.clone();
                corrupted.chunk.as_mut().unwrap()[4] ^= 1;
                assert!(matches!(
                    alice.receive(&corrupted),
                    Err(Error::Authentication)
                ));
                assert_eq!(alice.epoch, 1);
                assert!(matches!(alice.stage, Stage::EkSentCt1Received { .. }));
                let (epoch, output) = alice.receive(&from_bob).unwrap();
                assert_eq!(epoch, 0);
                assert_eq!(output.unwrap().epoch, 1);
                assert_eq!(alice.epoch, 2);
                reached = true;
                break;
            }
            alice.receive(&from_bob).unwrap();
        }
        assert!(reached);
    }

    #[test]
    fn wire_framing_and_future_epochs_are_strict() {
        let [mut alice, mut bob] = pair();
        let message = alice.send().unwrap().0;
        let bytes = message.to_bytes();
        for len in 0..bytes.len() {
            assert!(Message::from_bytes(&bytes[..len]).is_err());
        }
        let mut invalid = bytes.clone();
        invalid[8] = 7;
        assert!(Message::from_bytes(&invalid).is_err());
        invalid = bytes[..9].to_vec();
        invalid[8] = 4;
        assert!(Message::from_bytes(&invalid).is_err());
        invalid = bytes.clone();
        invalid[..8].fill(0);
        assert!(Message::from_bytes(&invalid).is_err());
        invalid = bytes.clone();
        invalid.push(0);
        assert!(Message::from_bytes(&invalid).is_err());
        let mut future = message;
        future.epoch = u64::MAX;
        assert!(matches!(bob.receive(&future), Err(Error::State)));
        assert_eq!(bob.epoch, 1);
        bob.epoch = u64::MAX;
        bob.stage = Stage::Ct2Sampled {
            encoder: Encoder::new(&[0; 192]).unwrap(),
        };
        assert_eq!(bob.send().unwrap().1, u64::MAX - 1);
    }
}
