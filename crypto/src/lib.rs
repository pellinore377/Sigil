#![forbid(unsafe_code)]
//! Experimental primitives, handshake, classical ratchet and encrypted checkpoints. No enabled wire suite.

pub mod attachment;
mod braid;
mod checkpoint;
mod dh;
pub mod group_receipt;
pub mod handshake;
mod identity;
mod kem;
pub mod link;
pub mod private_credentials;
pub mod private_group;
pub mod ratchet;
pub mod recovery;
pub mod sender_keys;
mod skipped;
mod spqr;
pub mod storage;
mod symmetric;
pub mod triple;
pub use dh::DhKey;
pub use identity::{verify_signature, IdentityKey};
pub use kem::{encapsulate, KemKey};
pub use symmetric::{derive_chain, derive_root, MessageKey, MAX_AAD, MAX_PLAINTEXT};
use zeroize::Zeroizing;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Entropy,
    InvalidKey,
    Authentication,
    Limit,
    Encoding,
    State,
    Replay,
}

pub struct Secret32(Zeroizing<[u8; 32]>);
impl Secret32 {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }
    pub fn generate() -> Result<Self, Error> {
        Ok(Self(random_bytes()?))
    }
}

fn random_bytes<const N: usize>() -> Result<Zeroizing<[u8; N]>, Error> {
    let mut bytes = Zeroizing::new([0; N]);
    getrandom::fill(bytes.as_mut()).map_err(|_| Error::Entropy)?;
    Ok(bytes)
}

#[cfg(test)]
mod vectors;
