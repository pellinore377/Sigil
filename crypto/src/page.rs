//! Per-page keys: sealed op batches, snapshots, presence and version labels.
use crate::{random_bytes, Error, IdentityKey, Secret32};
use aes_gcm_siv::{
    aead::{Aead, KeyInit, Payload},
    Nonce,
};
use hkdf::Hkdf;
use sha2::{Digest, Sha256};
use sigil_protocol::pages::{
    Grant, Request, Slot, MAX_LABEL_BYTES, MAX_OP_BYTES, MAX_PRESENCE_BYTES, MAX_SNAPSHOT_BYTES,
};
use zeroize::Zeroizing;
type Cipher = aes_gcm_siv::AesGcmSiv<aes::Aes256>;
type Id = [u8; 32];
const HEADER: &[u8; 7] = b"SGPE\0\x01\0";
const BATCH: &[u8; 8] = b"SGPB\0\x01\0\0";
const SIGNED: &[u8] = b"Sigil/page-op/v0";
pub const OVERHEAD: usize = 8 + 12 + 16;
pub const BATCH_HEADER: usize = 8 + 32 + 8 + 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    Op,
    Snapshot,
    Presence,
    Label,
}
impl Purpose {
    fn name(self) -> &'static str {
        match self {
            Self::Op => "op",
            Self::Snapshot => "snapshot",
            Self::Presence => "presence",
            Self::Label => "label",
        }
    }
    fn tag(self) -> u8 {
        match self {
            Self::Op => 1,
            Self::Snapshot => 2,
            Self::Presence => 3,
            Self::Label => 4,
        }
    }
    fn limit(self) -> usize {
        match self {
            Self::Op => MAX_OP_BYTES,
            Self::Snapshot => MAX_SNAPSHOT_BYTES,
            Self::Presence => MAX_PRESENCE_BYTES,
            Self::Label => MAX_LABEL_BYTES,
        }
    }
}

/// Where a sealed value sits; every field enters the associated data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context<'a> {
    Op { base: u64, request: &'a Request },
    Snapshot { seq: u64 },
    Presence { slot: &'a Slot },
    Label { version: u64 },
}
impl Context<'_> {
    fn purpose(self) -> Purpose {
        match self {
            Self::Op { .. } => Purpose::Op,
            Self::Snapshot { .. } => Purpose::Snapshot,
            Self::Presence { .. } => Purpose::Presence,
            Self::Label { .. } => Purpose::Label,
        }
    }
}

pub struct PageKey {
    id: Id,
    epoch: u64,
    key: Secret32,
}
impl PageKey {
    pub fn generate(id: Id, epoch: u64) -> Result<Self, Error> {
        Self::new(id, epoch, *random_bytes::<32>()?)
    }
    pub fn new(id: Id, epoch: u64, key: [u8; 32]) -> Result<Self, Error> {
        if epoch == 0 || epoch > i64::MAX as u64 || id == [0; 32] {
            return Err(Error::InvalidKey);
        }
        Ok(Self {
            id,
            epoch,
            key: Secret32::from_bytes(key),
        })
    }
    pub fn from_grant(grant: &Grant) -> Result<Self, Error> {
        grant.validate().map_err(|_| Error::Encoding)?;
        Self::new(grant.id, grant.epoch, grant.key)
    }
    pub fn id(&self) -> Id {
        self.id
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    /// Raw key for a grant; it travels only inside end-to-end events.
    pub fn key_bytes(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(*self.key.0)
    }
    fn subkey(&self, purpose: Purpose) -> Result<Secret32, Error> {
        let mut salt = [0; 40];
        salt[..32].copy_from_slice(&self.id);
        salt[32..].copy_from_slice(&self.epoch.to_be_bytes());
        let info = format!("Sigil/page/{}/v0", purpose.name());
        let mut key = Zeroizing::new([0; 32]);
        Hkdf::<Sha256>::new(Some(&salt), self.key.0.as_ref())
            .expand(info.as_bytes(), key.as_mut())
            .map_err(|_| Error::Limit)?;
        Ok(Secret32(key))
    }
    fn aad(&self, header: &[u8], context: Context) -> Vec<u8> {
        let mut aad = header.to_vec();
        aad.extend_from_slice(&self.id);
        aad.extend_from_slice(&self.epoch.to_be_bytes());
        match context {
            Context::Op { base, request } => {
                aad.extend_from_slice(&base.to_be_bytes());
                aad.extend_from_slice(&request.0);
            }
            Context::Snapshot { seq } => aad.extend_from_slice(&seq.to_be_bytes()),
            Context::Presence { slot } => aad.extend_from_slice(&slot.0),
            Context::Label { version } => aad.extend_from_slice(&version.to_be_bytes()),
        }
        aad
    }
    pub fn seal(&self, context: Context, plaintext: &[u8]) -> Result<Vec<u8>, Error> {
        self.seal_with_nonce(context, plaintext, &*random_bytes::<12>()?)
    }
    fn seal_with_nonce(
        &self,
        context: Context,
        plaintext: &[u8],
        nonce: &[u8; 12],
    ) -> Result<Vec<u8>, Error> {
        let purpose = context.purpose();
        if plaintext.len() + OVERHEAD > purpose.limit() {
            return Err(Error::Limit);
        }
        let mut bytes = Vec::with_capacity(plaintext.len() + OVERHEAD);
        bytes.extend_from_slice(HEADER);
        bytes.push(purpose.tag());
        bytes.extend_from_slice(nonce);
        let key = self.subkey(purpose)?;
        let ciphertext = Cipher::new(key.0.as_ref().into())
            .encrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: plaintext,
                    aad: &self.aad(&bytes, context),
                },
            )
            .map_err(|_| Error::Authentication)?;
        bytes.extend_from_slice(&ciphertext);
        Ok(bytes)
    }
    pub fn open(&self, context: Context, bytes: &[u8]) -> Result<Zeroizing<Vec<u8>>, Error> {
        let purpose = context.purpose();
        if bytes.len() < OVERHEAD || bytes.len() > purpose.limit() {
            return Err(Error::Encoding);
        }
        if &bytes[..7] != HEADER || bytes[7] != purpose.tag() {
            return Err(Error::Encoding);
        }
        let key = self.subkey(purpose)?;
        Cipher::new(key.0.as_ref().into())
            .decrypt(
                Nonce::from_slice(&bytes[8..20]),
                Payload {
                    msg: &bytes[20..],
                    aad: &self.aad(&bytes[..20], context),
                },
            )
            .map(Zeroizing::new)
            .map_err(|_| Error::Authentication)
    }
    pub fn seal_batch(
        &self,
        base: u64,
        request: &Request,
        batch: &Batch,
    ) -> Result<Vec<u8>, Error> {
        self.seal(Context::Op { base, request }, &batch.to_bytes()?)
    }
    /// Applies on AEAD success alone; attribution is `Batch::verify`.
    pub fn open_batch(&self, base: u64, request: &Request, bytes: &[u8]) -> Result<Batch, Error> {
        Batch::from_bytes(&self.open(Context::Op { base, request }, bytes)?)
    }
}

/// Decrypted op batch: author device, time, XEdDSA signature and opaque ops.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Batch {
    pub device: Id,
    pub created: u64,
    pub signature: [u8; 64],
    pub ops: Vec<u8>,
}
impl Batch {
    pub const MAX_OPS: usize = MAX_OP_BYTES - OVERHEAD - BATCH_HEADER;
    #[allow(clippy::too_many_arguments)]
    pub fn sign(
        identity: &IdentityKey,
        key: &PageKey,
        base: u64,
        request: &Request,
        device: Id,
        created: u64,
        ops: Vec<u8>,
    ) -> Result<Self, Error> {
        let mut batch = Self {
            device,
            created,
            signature: [0; 64],
            ops,
        };
        batch.check()?;
        batch.signature = identity.sign(&batch.statement(key.id, key.epoch, base, request))?;
        Ok(batch)
    }
    pub fn statement(&self, id: Id, epoch: u64, base: u64, request: &Request) -> Vec<u8> {
        let mut bytes = SIGNED.to_vec();
        bytes.extend_from_slice(&id);
        bytes.extend_from_slice(&epoch.to_be_bytes());
        bytes.extend_from_slice(&base.to_be_bytes());
        bytes.extend_from_slice(&request.0);
        bytes.extend_from_slice(&self.device);
        bytes.extend_from_slice(&self.created.to_be_bytes());
        bytes.extend_from_slice(&Sha256::digest(&self.ops));
        bytes
    }
    /// `public` is the identity key the caller binds to `device`.
    pub fn verify(
        &self,
        public: &[u8; 32],
        key: &PageKey,
        base: u64,
        request: &Request,
    ) -> Result<(), Error> {
        crate::verify_signature(
            public,
            &self.statement(key.id, key.epoch, base, request),
            &self.signature,
        )
    }
    fn check(&self) -> Result<(), Error> {
        if self.ops.is_empty() || self.ops.len() > Self::MAX_OPS {
            return Err(Error::Limit);
        }
        if self.created == 0 || self.created > i64::MAX as u64 || self.device == [0; 32] {
            return Err(Error::Encoding);
        }
        Ok(())
    }
    pub fn to_bytes(&self) -> Result<Zeroizing<Vec<u8>>, Error> {
        self.check()?;
        let mut bytes = Zeroizing::new(Vec::with_capacity(BATCH_HEADER + self.ops.len()));
        bytes.extend_from_slice(BATCH);
        bytes.extend_from_slice(&self.device);
        bytes.extend_from_slice(&self.created.to_be_bytes());
        bytes.extend_from_slice(&self.signature);
        bytes.extend_from_slice(&self.ops);
        Ok(bytes)
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() <= BATCH_HEADER || &bytes[..8] != BATCH {
            return Err(Error::Encoding);
        }
        let batch = Self {
            device: bytes[8..40].try_into().map_err(|_| Error::Encoding)?,
            created: u64::from_be_bytes(bytes[40..48].try_into().map_err(|_| Error::Encoding)?),
            signature: bytes[48..112].try_into().map_err(|_| Error::Encoding)?,
            ops: bytes[112..].to_vec(),
        };
        batch.check()?;
        Ok(batch)
    }
}

/// Fresh access or manage token.
pub fn token() -> Result<Zeroizing<[u8; 32]>, Error> {
    random_bytes()
}
/// What the server stores for a token.
pub fn token_hash(token: &[u8]) -> [u8; 32] {
    Sha256::digest(token).into()
}

#[cfg(test)]
#[path = "page_tests.rs"]
mod tests;
