//! Usernames and the name-bound addresses: the key-package shelf and the
//! requests slot. These are the only server objects tied to a person.

use crate::kdf::kdf;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

/// `@localpart:server`. localpart: 1..=32 of `[a-z0-9._-]`, not starting
/// or ending with `.`. server: 1..=253 of `[a-z0-9.-]`, at least one `.`.
/// Both are lowercase; callers lowercase before parsing.
pub fn parse_username(s: &str) -> crate::Result<(&str, &str)> {
    let rest = s.strip_prefix('@').ok_or(crate::Error::Username)?;
    let (local, server) = rest.split_once(':').ok_or(crate::Error::Username)?;
    let ok_local = !local.is_empty()
        && local.len() <= 32
        && local.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'_' || b == b'-'
        })
        && !local.starts_with('.')
        && !local.ends_with('.');
    let ok_server = !server.is_empty()
        && server.len() <= 253
        && server.contains('.')
        && server
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
        && !server.starts_with('.')
        && !server.ends_with('.');
    if ok_local && ok_server {
        Ok((local, server))
    } else {
        Err(crate::Error::Username)
    }
}

/// Where a user's key packages sit. Anyone with the card can compute it.
pub fn shelf_address(identity_pub: &[u8; 32]) -> [u8; 32] {
    kdf("sigil v1 shelf address", identity_pub)
}

/// Key under which the shelf's contents are sealed, so the server holding
/// the shelf cannot read the packages it hands out.
pub fn shelf_key(identity_pub: &[u8; 32]) -> [u8; 32] {
    kdf("sigil v1 shelf key", identity_pub)
}

/// The requests slot for a given period. `period` is the number of whole
/// 30-day periods since the Unix epoch (`unix_seconds / 2_592_000`).
pub fn requests_address(identity_pub: &[u8; 32], period: u32) -> [u8; 32] {
    let mut ikm = identity_pub.to_vec();
    ikm.extend_from_slice(&period.to_le_bytes());
    kdf("sigil v1 requests address", &ikm)
}

const REQ_READ_CTX: &[u8] = b"sigil v1 requests read";

/// Reading or subscribing to a requests slot proves ownership: an Ed25519
/// signature by the identity key over `"sigil v1 requests read" ‖ address ‖ nonce`,
/// where `nonce` is 32 bytes chosen by the server for this request.
pub fn requests_read_proof(signing: &SigningKey, address: &[u8; 32], nonce: &[u8; 32]) -> [u8; 64] {
    let mut msg = REQ_READ_CTX.to_vec();
    msg.extend_from_slice(address);
    msg.extend_from_slice(nonce);
    signing.sign(&msg).to_bytes()
}

pub fn verify_requests_read_proof(
    identity_pub: &[u8; 32],
    address: &[u8; 32],
    nonce: &[u8; 32],
    proof: &[u8; 64],
) -> crate::Result<()> {
    let vk = VerifyingKey::from_bytes(identity_pub).map_err(|_| crate::Error::Malformed)?;
    let mut msg = REQ_READ_CTX.to_vec();
    msg.extend_from_slice(address);
    msg.extend_from_slice(nonce);
    vk.verify(&msg, &Signature::from_bytes(proof))
        .map_err(|_| crate::Error::Auth)
}
