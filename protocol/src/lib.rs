//! Sigil Protocol v1: reference implementation of every derivation and
//! wire format in `docs/spec/sigil-protocol-v1.md`.
//!
//! This crate is deliberately small and dependency-light. It contains no
//! networking, no storage and no MLS: it is the part of the protocol that
//! has to be bit-exact between every client and every server, and the
//! test vectors in `vectors/v1.json` are generated from it.
//!
//! Primitives (standard, audited, pure Rust):
//! - hash and KDF: BLAKE3
//! - AEAD: XChaCha20-Poly1305
//! - signatures: Ed25519
//! - KEM: X25519 + ML-KEM-768, combined as SigilKEM (`kem`)
//! - password hashing: Argon2id

#![forbid(unsafe_code)]

pub mod bag;
pub mod emoji;
pub mod encoding;
pub mod envelope;
pub mod epoch;
pub mod identity;
pub mod kdf;
pub mod kem;
pub mod linking;
pub mod names;
pub mod recovery;
pub mod requests;
pub mod testrng;
pub mod token;
pub mod wire;

/// Protocol version carried in every versioned structure.
pub const VERSION: u8 = 1;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// Input had the wrong length.
    Length,
    /// A signature or authentication tag did not verify.
    Auth,
    /// Padding was malformed after decryption.
    Padding,
    /// The plaintext does not fit the largest bucket.
    TooLarge,
    /// A structure was malformed.
    Malformed,
    /// A username violated the grammar.
    Username,
}

pub type Result<T> = core::result::Result<T, Error>;
