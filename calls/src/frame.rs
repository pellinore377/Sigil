use crate::{hash, random_id, Error, Id};
use serde::{Deserialize, Serialize};
use sframe::{
    frame::{EncryptedFrameView, MediaFrameView, MonotonicCounter},
    key::{DecryptionKey, EncryptionKey},
    CipherSuite,
};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;
const MAX_FRAME: usize = 1024 * 1024;
const MAX_COUNTER: u64 = (1 << 20) - 1;
const MAX_BYTES: u64 = 1 << 30;
const HEADER: usize = 18;
const PREFIX: &[u8; 8] = b"SGCF\0\x01\0\0";
const SUITE: CipherSuite = CipherSuite::AesGcm256Sha512;
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Context {
    pub call: Id,
    pub roster: Id,
    pub sender: Id,
    pub incarnation: Id,
}
impl Context {
    fn bytes(&self) -> Result<Vec<u8>, Error> {
        if [self.call, self.roster, self.sender, self.incarnation].contains(&[0; 32]) {
            return Err(Error::Invalid);
        }
        Ok([
            b"Sigil/call-media/v1".as_slice(),
            &self.call,
            &self.roster,
            &self.sender,
            &self.incarnation,
        ]
        .concat())
    }
    fn kid(&self) -> Result<u64, Error> {
        Ok(u64::from_be_bytes(
            hash(b"Sigil/call-media-kid/v1", &self.bytes()?)[..8]
                .try_into()
                .unwrap(),
        ))
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyShare {
    pub context: Context,
    seed: Zeroizing<Id>,
    signing: Id,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum MediaKind {
    Audio = 0,
    Camera = 1,
    Screen = 2,
}
pub struct Frame {
    pub kind: MediaKind,
    pub timestamp: u64,
    pub keyframe: bool,
    pub data: Zeroizing<Vec<u8>>,
}
pub struct Sender {
    context: Context,
    key: EncryptionKey,
    identity: sigil_crypto::IdentityKey,
    counter: MonotonicCounter,
    bytes: u64,
    started: Instant,
}
pub struct Receiver {
    context: Context,
    key: DecryptionKey,
    signing: Id,
    highest: Option<u64>,
    seen: u128,
}
impl Sender {
    /// A fresh key for every handle; callers cannot restart a sender from an old key.
    pub fn generate(context: Context) -> Result<(Self, KeyShare), Error> {
        context.bytes()?;
        let seed = Zeroizing::new(random_id()?);
        let identity = sigil_crypto::IdentityKey::generate().map_err(|_| Error::Entropy)?;
        let signing = identity.public_key();
        let key = EncryptionKey::derive_from(SUITE, context.kid()?, seed.as_slice())
            .map_err(|_| Error::Authentication)?;
        Ok((
            Self {
                context,
                key,
                identity,
                counter: MonotonicCounter::new(MAX_COUNTER),
                bytes: 0,
                started: Instant::now(),
            },
            KeyShare {
                context,
                seed,
                signing,
            },
        ))
    }
    pub fn seal(
        &mut self,
        kind: MediaKind,
        timestamp: u64,
        keyframe: bool,
        data: &[u8],
    ) -> Result<Vec<u8>, Error> {
        if data.is_empty() || data.len() > MAX_FRAME || kind == MediaKind::Audio && keyframe {
            return Err(Error::Invalid);
        }
        if self.counter.is_exhausted()
            || self.started.elapsed() >= Duration::from_secs(600)
            || self.bytes + data.len() as u64 > MAX_BYTES
        {
            return Err(Error::Expired);
        }
        let mut header = Vec::with_capacity(HEADER);
        header.extend_from_slice(PREFIX);
        header.push(kind as u8);
        header.push(u8::from(keyframe));
        header.extend_from_slice(&timestamp.to_be_bytes());
        let aad = [self.context.bytes()?, header.clone()].concat();
        self.bytes += data.len() as u64;
        let encrypted = MediaFrameView::with_meta_data(&mut self.counter, data, &aad)
            .encrypt(&self.key)
            .map_err(|_| Error::Authentication)?;
        header.extend_from_slice(&encrypted.as_ref()[aad.len()..]);
        let digest = hash(
            b"Sigil/call-signed-frame/v1",
            &[self.context.bytes()?, header.clone()].concat(),
        );
        header.extend_from_slice(
            &self
                .identity
                .sign(&digest)
                .map_err(|_| Error::Authentication)?,
        );
        Ok(header)
    }
}
impl Receiver {
    pub fn new(share: &KeyShare) -> Result<Self, Error> {
        Ok(Self {
            context: share.context,
            key: DecryptionKey::derive_from(SUITE, share.context.kid()?, share.seed.as_slice())
                .map_err(|_| Error::Authentication)?,
            signing: share.signing,
            highest: None,
            seen: 0,
        })
    }
    pub fn open(&mut self, bytes: &[u8]) -> Result<Frame, Error> {
        if bytes.len() < HEADER + 18 + 64
            || bytes.len() > HEADER + MAX_FRAME + 34 + 64
            || &bytes[..8] != PREFIX
            || bytes[9] > 1
        {
            return Err(Error::Invalid);
        }
        let (bytes, signature) = bytes.split_at(bytes.len() - 64);
        let digest = hash(
            b"Sigil/call-signed-frame/v1",
            &[self.context.bytes()?, bytes.to_vec()].concat(),
        );
        sigil_crypto::verify_signature(&self.signing, &digest, signature)
            .map_err(|_| Error::Authentication)?;
        let kind = match bytes[8] {
            0 => MediaKind::Audio,
            1 => MediaKind::Camera,
            2 => MediaKind::Screen,
            _ => return Err(Error::Invalid),
        };
        let keyframe = bytes[9] == 1;
        if kind == MediaKind::Audio && keyframe {
            return Err(Error::Invalid);
        }
        let aad = [self.context.bytes()?, bytes[..HEADER].to_vec()].concat();
        let encrypted = EncryptedFrameView::try_with_meta_data(&bytes[HEADER..], &aad)
            .map_err(|_| Error::Invalid)?;
        let counter = encrypted.header().counter();
        if counter > MAX_COUNTER {
            return Err(Error::Invalid);
        }
        if let Some(highest) = self.highest {
            if counter <= highest
                && (highest - counter >= 128 || self.seen & (1u128 << (highest - counter)) != 0)
            {
                return Err(Error::Replay);
            }
        }
        let plain = encrypted
            .decrypt(&self.key)
            .map_err(|_| Error::Authentication)?;
        if plain.payload().is_empty() || plain.payload().len() > MAX_FRAME {
            return Err(Error::Invalid);
        }
        match self.highest {
            None => {
                self.highest = Some(counter);
                self.seen = 1;
            }
            Some(highest) if counter > highest => {
                self.seen = if counter - highest >= 128 {
                    1
                } else {
                    (self.seen << (counter - highest)) | 1
                };
                self.highest = Some(counter);
            }
            Some(highest) => self.seen |= 1u128 << (highest - counter),
        }
        Ok(Frame {
            kind,
            timestamp: u64::from_be_bytes(bytes[10..18].try_into().unwrap()),
            keyframe,
            data: Zeroizing::new(plain.payload().to_vec()),
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_recipient_with_the_shared_encryption_key_cannot_forge_the_senders_frames() {
        let context = Context {
            call: [1; 32],
            roster: [2; 32],
            sender: [3; 32],
            incarnation: [4; 32],
        };
        let (mut sender, share) = Sender::generate(context).unwrap();
        let (mut attacker, _) = Sender::generate(context).unwrap();
        attacker.key =
            EncryptionKey::derive_from(SUITE, context.kid().unwrap(), share.seed.as_slice())
                .unwrap();
        let forged = attacker
            .seal(MediaKind::Audio, 1, false, b"forged voice")
            .unwrap();
        let mut receiver = Receiver::new(&share).unwrap();
        assert!(matches!(receiver.open(&forged), Err(Error::Authentication)));
        let original = sender
            .seal(MediaKind::Audio, 1, false, b"synthetic voice")
            .unwrap();
        assert_eq!(&*receiver.open(&original).unwrap().data, b"synthetic voice");
    }
}
