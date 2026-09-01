//! Bags: how a client talks to a server through an Envoy. A bag is sealed
//! to the server's SigilKEM public key, so the Envoy forwards ciphertext.
//! Requests are padded to 2048, 8192 or 32768 bytes; responses to 1024,
//! 4096, 16384 or 65536.

use crate::kdf::kdf;
use crate::kem;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

pub const NONCE_LEN: usize = 24;
pub const TAG_LEN: usize = 16;
pub const REQUEST_BUCKETS: [usize; 3] = [2048, 8192, 32768];
pub const RESPONSE_BUCKETS: [usize; 4] = [1024, 4096, 16384, 65536];
/// version(1) + ciphertext + nonce + tag
const REQ_OVERHEAD: usize = 1 + kem::CIPHERTEXT_LEN + NONCE_LEN + TAG_LEN;
const RESP_OVERHEAD: usize = NONCE_LEN + TAG_LEN;

fn pad_to(plain: &[u8], buckets: &[usize], overhead: usize) -> crate::Result<Vec<u8>> {
    let need = plain.len() + 1;
    let bucket = buckets
        .iter()
        .copied()
        .find(|b| need <= b - overhead)
        .ok_or(crate::Error::TooLarge)?;
    let mut out = plain.to_vec();
    out.push(0x80);
    out.resize(bucket - overhead, 0);
    Ok(out)
}

/// Keys for one bag exchange, derived from the KEM shared secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BagKeys {
    pub request: [u8; 32],
    pub response: [u8; 32],
}

pub fn keys(shared: &[u8; 32]) -> BagKeys {
    BagKeys {
        request: kdf("sigil v1 bag request", shared),
        response: kdf("sigil v1 bag response", shared),
    }
}

/// Client side. `eseed` is the 32-byte ephemeral KEM seed (random in
/// production). Returns the bag and the keys for reading the response.
pub fn seal_request(
    server_pub: &[u8],
    eseed: &[u8; 32],
    nonce: &[u8; NONCE_LEN],
    request: &[u8],
) -> crate::Result<(Vec<u8>, BagKeys)> {
    let (ct, ss) = kem::encapsulate(server_pub, eseed)?;
    let k = keys(&ss);
    let padded = pad_to(request, &REQUEST_BUCKETS, REQ_OVERHEAD)?;
    let mut ad = b"sigil v1 bag".to_vec();
    ad.extend_from_slice(&ct);
    let sealed = XChaCha20Poly1305::new((&k.request).into())
        .encrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: &padded,
                aad: &ad,
            },
        )
        .map_err(|_| crate::Error::Auth)?;
    let mut out = vec![crate::VERSION];
    out.extend_from_slice(&ct);
    out.extend_from_slice(nonce);
    out.extend_from_slice(&sealed);
    debug_assert!(REQUEST_BUCKETS.contains(&out.len()));
    Ok((out, k))
}

/// Server side. Returns the request plaintext and the keys.
pub fn open_request(server_key: &kem::SecretKey, bag: &[u8]) -> crate::Result<(Vec<u8>, BagKeys)> {
    if !REQUEST_BUCKETS.contains(&bag.len()) || bag[0] != crate::VERSION {
        return Err(crate::Error::Malformed);
    }
    let ct = &bag[1..1 + kem::CIPHERTEXT_LEN];
    let nonce = &bag[1 + kem::CIPHERTEXT_LEN..1 + kem::CIPHERTEXT_LEN + NONCE_LEN];
    let sealed = &bag[1 + kem::CIPHERTEXT_LEN + NONCE_LEN..];
    let ss = server_key.decapsulate(ct)?;
    let k = keys(&ss);
    let mut ad = b"sigil v1 bag".to_vec();
    ad.extend_from_slice(ct);
    let padded = XChaCha20Poly1305::new((&k.request).into())
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: sealed,
                aad: &ad,
            },
        )
        .map_err(|_| crate::Error::Auth)?;
    Ok((crate::envelope::unpad(&padded)?.to_vec(), k))
}

/// `response = nonce ‖ XChaCha20-Poly1305(response_key, nonce, ad = "sigil v1 bag response", pad(plain))`.
pub fn seal_response(
    k: &BagKeys,
    nonce: &[u8; NONCE_LEN],
    response: &[u8],
) -> crate::Result<Vec<u8>> {
    let padded = pad_to(response, &RESPONSE_BUCKETS, RESP_OVERHEAD)?;
    let sealed = XChaCha20Poly1305::new((&k.response).into())
        .encrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: &padded,
                aad: b"sigil v1 bag response",
            },
        )
        .map_err(|_| crate::Error::Auth)?;
    let mut out = nonce.to_vec();
    out.extend_from_slice(&sealed);
    Ok(out)
}

pub fn open_response(k: &BagKeys, response: &[u8]) -> crate::Result<Vec<u8>> {
    if !RESPONSE_BUCKETS.contains(&response.len()) {
        return Err(crate::Error::Length);
    }
    let (nonce, sealed) = response.split_at(NONCE_LEN);
    let padded = XChaCha20Poly1305::new((&k.response).into())
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: sealed,
                aad: b"sigil v1 bag response",
            },
        )
        .map_err(|_| crate::Error::Auth)?;
    Ok(crate::envelope::unpad(&padded)?.to_vec())
}
