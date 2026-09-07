//! Immutable, bounded encrypted chunks. Descriptors are sensitive key material
//! and require authenticated encrypted transport; this is not a ratchet.
use crate::{random_bytes, Error, Secret32};
use aes_gcm_siv::{
    aead::{Aead, KeyInit, Payload},
    Nonce,
};
use hkdf::Hkdf;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;
type Cipher = aes_gcm_siv::AesGcmSiv<aes::Aes256>;
type Id = [u8; 32];
use sigil_protocol::file::{DescriptorError, KeyDescriptor, DESCRIPTOR_PREFIX as DESCRIPTOR};
pub use sigil_protocol::file::{CHUNK_OVERHEAD, CHUNK_SIZE, DESCRIPTOR_SIZE, MAX_FILE_BYTES};
const CHUNK: &[u8; 8] = b"SGAC\0\x01\0\0";
const KEY_DOMAIN: &[u8] = b"Sigil/attachment-chunk-key/v0";
const LIST_DOMAIN: &[u8] = b"Sigil/attachment-ciphertext-list/v0";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    pub file: Id,
    pub length: u64,
}
impl Shape {
    pub fn chunks(self) -> Result<u32, Error> {
        if self.length > MAX_FILE_BYTES {
            return Err(Error::Limit);
        }
        Ok(self.length.div_ceil(CHUNK_SIZE as u64).max(1) as u32)
    }
    pub fn chunk_length(self, index: u32) -> Result<usize, Error> {
        if index >= self.chunks()? {
            return Err(Error::Encoding);
        }
        Ok((self.length - index as u64 * CHUNK_SIZE as u64).min(CHUNK_SIZE as u64) as usize)
    }
    pub fn ciphertext_length(self) -> Result<u64, Error> {
        Ok(self.length + self.chunks()? as u64 * CHUNK_OVERHEAD as u64)
    }
}
pub struct FileKey {
    shape: Shape,
    master: Secret32,
}
impl FileKey {
    pub fn generate(length: u64) -> Result<Self, Error> {
        let shape = Shape {
            file: *random_bytes::<32>()?,
            length,
        };
        shape.chunks()?;
        Ok(Self {
            shape,
            master: Secret32::generate()?,
        })
    }
    pub fn shape(&self) -> Shape {
        self.shape
    }
    fn chunk_key(&self, index: u32) -> Result<Secret32, Error> {
        self.shape.chunk_length(index)?;
        let mut info = KEY_DOMAIN.to_vec();
        info.extend_from_slice(&self.shape.length.to_be_bytes());
        info.extend_from_slice(&index.to_be_bytes());
        let mut key = Zeroizing::new([0; 32]);
        Hkdf::<Sha256>::new(Some(&self.shape.file), self.master.0.as_ref())
            .expand(&info, key.as_mut())
            .map_err(|_| Error::Limit)?;
        Ok(Secret32(key))
    }
    /// Persist this exact ciphertext for retries; do not regenerate uploaded parts.
    pub fn seal_chunk(&self, index: u32, plaintext: &[u8]) -> Result<Vec<u8>, Error> {
        self.seal_with_nonce(index, plaintext, &*random_bytes::<12>()?)
    }
    fn seal_with_nonce(
        &self,
        index: u32,
        plaintext: &[u8],
        nonce: &[u8; 12],
    ) -> Result<Vec<u8>, Error> {
        if plaintext.len() != self.shape.chunk_length(index)? {
            return Err(Error::Encoding);
        }
        let key = self.chunk_key(index)?;
        let mut bytes = Vec::with_capacity(plaintext.len() + CHUNK_OVERHEAD);
        bytes.extend_from_slice(CHUNK);
        bytes.extend_from_slice(&self.shape.file);
        bytes.extend_from_slice(&index.to_be_bytes());
        bytes.extend_from_slice(&self.shape.length.to_be_bytes());
        bytes.extend_from_slice(&(plaintext.len() as u32).to_be_bytes());
        bytes.extend_from_slice(nonce);
        let ciphertext = Cipher::new(key.0.as_ref().into())
            .encrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: plaintext,
                    aad: &bytes,
                },
            )
            .map_err(|_| Error::Authentication)?;
        bytes.extend_from_slice(&ciphertext);
        Ok(bytes)
    }
    pub fn open_chunk(&self, index: u32, bytes: &[u8]) -> Result<Zeroizing<Vec<u8>>, Error> {
        let length = self.shape.chunk_length(index)?;
        if bytes.len() != length + CHUNK_OVERHEAD {
            return Err(Error::Encoding);
        }
        if &bytes[..8] != CHUNK
            || bytes[8..40] != self.shape.file
            || bytes[40..44] != index.to_be_bytes()
            || bytes[44..52] != self.shape.length.to_be_bytes()
            || bytes[52..56] != (length as u32).to_be_bytes()
        {
            return Err(Error::Encoding);
        }
        let key = self.chunk_key(index)?;
        Cipher::new(key.0.as_ref().into())
            .decrypt(
                Nonce::from_slice(&bytes[56..68]),
                Payload {
                    msg: &bytes[68..],
                    aad: &bytes[..68],
                },
            )
            .map(Zeroizing::new)
            .map_err(|_| Error::Authentication)
    }
    /// The caller supplies the completed ordered ciphertext commitment.
    pub fn descriptor(&self, root: Id) -> Zeroizing<Vec<u8>> {
        let mut bytes = Zeroizing::new(Vec::with_capacity(DESCRIPTOR_SIZE));
        bytes.extend_from_slice(DESCRIPTOR);
        bytes.extend_from_slice(&self.shape.file);
        bytes.extend_from_slice(&self.shape.length.to_be_bytes());
        bytes.extend_from_slice(&(CHUNK_SIZE as u32).to_be_bytes());
        bytes.extend_from_slice(self.master.0.as_ref());
        bytes.extend_from_slice(&root);
        bytes
    }
    /// Parsing proves framing only; origin and access authorization are external.
    pub fn from_descriptor(bytes: &[u8]) -> Result<(Self, Id), Error> {
        let descriptor = KeyDescriptor::from_bytes(bytes).map_err(|error| match error {
            DescriptorError::Encoding => Error::Encoding,
            DescriptorError::Limit => Error::Limit,
        })?;
        let shape = Shape {
            file: descriptor.file,
            length: descriptor.length,
        };
        shape.chunks()?;
        Ok((
            Self {
                shape,
                master: Secret32::from_bytes(*descriptor.key),
            },
            *descriptor.root,
        ))
    }
}

/// Streaming commitment to every ciphertext hash in exact chunk order.
/// Persist per-chunk hashes and reconstruct this accumulator after restart.
pub struct CiphertextList {
    shape: Shape,
    next: u32,
    hash: Sha256,
}
impl CiphertextList {
    pub fn new(shape: Shape) -> Result<Self, Error> {
        let chunks = shape.chunks()?;
        let mut hash = Sha256::new();
        hash.update(LIST_DOMAIN);
        hash.update(shape.file);
        hash.update(shape.length.to_be_bytes());
        hash.update(chunks.to_be_bytes());
        Ok(Self {
            shape,
            next: 0,
            hash,
        })
    }
    pub fn push(&mut self, index: u32, ciphertext: &[u8]) -> Result<Id, Error> {
        if ciphertext.len() != self.shape.chunk_length(index)? + CHUNK_OVERHEAD {
            return Err(Error::Encoding);
        }
        let hash: Id = Sha256::digest(ciphertext).into();
        self.push_hash(index, hash)?;
        Ok(hash)
    }
    /// Hashes must come from the exact authenticated/frozen ciphertext records.
    pub fn push_hash(&mut self, index: u32, hash: Id) -> Result<(), Error> {
        if index != self.next || index >= self.shape.chunks()? {
            return Err(Error::State);
        }
        self.hash.update(index.to_be_bytes());
        self.hash.update(hash);
        self.next += 1;
        Ok(())
    }
    pub fn finish(self) -> Result<Id, Error> {
        if self.next != self.shape.chunks()? {
            return Err(Error::State);
        }
        Ok(self.hash.finalize().into())
    }
}

#[cfg(test)]
#[path = "attachment_tests.rs"]
mod tests;
