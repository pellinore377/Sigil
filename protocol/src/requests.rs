//! Envelopes for the requests slot. There is no shared epoch secret yet, so
//! the sender encapsulates to the recipient's identity KEM key. The result
//! is padded to the same sizes as an ordinary envelope, less the ciphertext.

use crate::kdf::kdf;
use crate::kem;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

pub const NONCE_LEN: usize = 24;
const OVERHEAD: usize = kem::CIPHERTEXT_LEN + NONCE_LEN + 16;
/// Whole request-envelope sizes: the two larger ordinary buckets.
pub const BUCKETS: [usize; 2] = [4096, 16384];

fn pad(plain: &[u8]) -> crate::Result<Vec<u8>> {
    let need = plain.len() + 1;
    let bucket = BUCKETS
        .iter()
        .copied()
        .find(|b| need <= b - OVERHEAD)
        .ok_or(crate::Error::TooLarge)?;
    let mut out = plain.to_vec();
    out.push(0x80);
    out.resize(bucket - OVERHEAD, 0);
    Ok(out)
}

/// `ct ‖ nonce ‖ AEAD(KDF("sigil v1 requests envelope", shared), nonce, ad = "sigil v1 requests" ‖ address, pad(plain))`.
pub fn seal(
    recipient_kem_pub: &[u8],
    address: &[u8; 32],
    eseed: &[u8; 32],
    nonce: &[u8; NONCE_LEN],
    plain: &[u8],
) -> crate::Result<Vec<u8>> {
    let (ct, shared) = kem::encapsulate(recipient_kem_pub, eseed)?;
    let key = kdf("sigil v1 requests envelope", &shared);
    let mut ad = b"sigil v1 requests".to_vec();
    ad.extend_from_slice(address);
    let sealed = XChaCha20Poly1305::new((&key).into())
        .encrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: &pad(plain)?,
                aad: &ad,
            },
        )
        .map_err(|_| crate::Error::Auth)?;
    let mut out = ct;
    out.extend_from_slice(nonce);
    out.extend_from_slice(&sealed);
    debug_assert!(BUCKETS.contains(&out.len()));
    Ok(out)
}

pub fn open(
    recipient: &kem::SecretKey,
    address: &[u8; 32],
    envelope: &[u8],
) -> crate::Result<Vec<u8>> {
    if !BUCKETS.contains(&envelope.len()) {
        return Err(crate::Error::Length);
    }
    let (ct, rest) = envelope.split_at(kem::CIPHERTEXT_LEN);
    let (nonce, sealed) = rest.split_at(NONCE_LEN);
    let shared = recipient.decapsulate(ct)?;
    let key = kdf("sigil v1 requests envelope", &shared);
    let mut ad = b"sigil v1 requests".to_vec();
    ad.extend_from_slice(address);
    let padded = XChaCha20Poly1305::new((&key).into())
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: sealed,
                aad: &ad,
            },
        )
        .map_err(|_| crate::Error::Auth)?;
    Ok(crate::envelope::unpad(&padded)?.to_vec())
}
