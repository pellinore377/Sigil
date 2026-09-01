//! Backup and recovery key material. The backup opens with the password and
//! the recovery key together; the server holds neither.

use crate::kdf::kdf;
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

pub const SALT_LEN: usize = 16;
/// Argon2id parameters: 64 MiB, 3 passes, 1 lane, 32-byte output.
pub const ARGON2_M_KIB: u32 = 65536;
pub const ARGON2_T: u32 = 3;
pub const ARGON2_P: u32 = 1;

/// `pw_key = Argon2id(password, salt)`; password is the UTF-8 of the
/// string after Unicode NFC normalisation (callers normalise).
pub fn password_key(password: &[u8], salt: &[u8; SALT_LEN]) -> [u8; 32] {
    let params = Params::new(ARGON2_M_KIB, ARGON2_T, ARGON2_P, Some(32)).unwrap();
    let mut out = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password, salt, &mut out)
        .expect("argon2 parameters are valid");
    out
}

/// `backup_key = KDF("sigil v1 backup key", pw_key ‖ recovery_key)`.
pub fn backup_key(pw_key: &[u8; 32], recovery_key: &[u8; 32]) -> [u8; 32] {
    let mut ikm = pw_key.to_vec();
    ikm.extend_from_slice(recovery_key);
    kdf("sigil v1 backup key", &ikm)
}

/// Where the backup lives on the server. Derived from the data key, so the
/// server cannot map a label to a name.
pub fn backup_label(data_key: &[u8; 32]) -> [u8; 32] {
    kdf("sigil v1 backup label", data_key)
}

/// `wrap = nonce ‖ XChaCha20-Poly1305(backup_key, nonce, ad = "sigil v1 data key wrap", data_key)`.
pub fn wrap_data_key(backup_key: &[u8; 32], nonce: &[u8; 24], data_key: &[u8; 32]) -> Vec<u8> {
    let ct = XChaCha20Poly1305::new(backup_key.into())
        .encrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: data_key,
                aad: b"sigil v1 data key wrap",
            },
        )
        .unwrap();
    let mut out = nonce.to_vec();
    out.extend_from_slice(&ct);
    out
}

pub fn unwrap_data_key(backup_key: &[u8; 32], wrap: &[u8]) -> crate::Result<[u8; 32]> {
    if wrap.len() != 24 + 32 + 16 {
        return Err(crate::Error::Length);
    }
    let (nonce, ct) = wrap.split_at(24);
    let dk = XChaCha20Poly1305::new(backup_key.into())
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: ct,
                aad: b"sigil v1 data key wrap",
            },
        )
        .map_err(|_| crate::Error::Auth)?;
    Ok(dk.try_into().unwrap())
}

/// TPM authorisation value for recovery Path 2.
pub fn tpm_auth(pw_key: &[u8; 32]) -> [u8; 32] {
    kdf("sigil v1 tpm auth", pw_key)
}

const CODE_ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// The printed recovery code: base32 (RFC 4648 alphabet, lowercase, no
/// padding) of `recovery_key ‖ check`, where `check` is the first two bytes
/// of `KDF("sigil v1 recovery code", recovery_key)`. 34 bytes become 55
/// characters, shown in groups of five separated by `-`.
pub fn recovery_code(recovery_key: &[u8; 32]) -> String {
    let check = kdf("sigil v1 recovery code", recovery_key);
    let mut data = recovery_key.to_vec();
    data.extend_from_slice(&check[..2]);
    let mut bits = 0u32;
    let mut nbits = 0;
    let mut chars = Vec::new();
    for b in data {
        bits = (bits << 8) | b as u32;
        nbits += 8;
        while nbits >= 5 {
            nbits -= 5;
            chars.push(CODE_ALPHABET[((bits >> nbits) & 31) as usize]);
        }
    }
    if nbits > 0 {
        chars.push(CODE_ALPHABET[((bits << (5 - nbits)) & 31) as usize]);
    }
    chars
        .chunks(5)
        .map(|c| core::str::from_utf8(c).unwrap())
        .collect::<Vec<_>>()
        .join("-")
}

/// Parse a code as typed: case-insensitive, separators and whitespace ignored.
pub fn parse_recovery_code(code: &str) -> crate::Result<[u8; 32]> {
    let mut bits = 0u32;
    let mut nbits = 0;
    let mut data = Vec::with_capacity(34);
    for c in code.chars() {
        if c == '-' || c.is_whitespace() {
            continue;
        }
        let c = c.to_ascii_lowercase() as u8;
        let v = CODE_ALPHABET
            .iter()
            .position(|&a| a == c)
            .ok_or(crate::Error::Malformed)? as u32;
        bits = (bits << 5) | v;
        nbits += 5;
        if nbits >= 8 {
            nbits -= 8;
            data.push(((bits >> nbits) & 0xff) as u8);
        }
    }
    if data.len() != 34 {
        return Err(crate::Error::Length);
    }
    let key: [u8; 32] = data[..32].try_into().unwrap();
    let check = kdf("sigil v1 recovery code", &key);
    if check[..2] != data[32..] {
        return Err(crate::Error::Auth);
    }
    Ok(key)
}
