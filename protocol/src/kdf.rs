//! Hashing and key derivation. Everything in the Sigil layer derives from
//! these two functions and a context string.

/// `H(x)`: BLAKE3-256 of `x`.
pub fn hash(x: &[u8]) -> [u8; 32] {
    *blake3::hash(x).as_bytes()
}

/// `KDF(ctx, ikm)`: BLAKE3 in derive-key mode, 32 bytes. `ctx` MUST be one
/// of the context strings listed in the spec; they all begin `sigil v1 `.
pub fn kdf(ctx: &str, ikm: &[u8]) -> [u8; 32] {
    debug_assert!(ctx.starts_with("sigil v1 "));
    *blake3::Hasher::new_derive_key(ctx)
        .update(ikm)
        .finalize()
        .as_bytes()
}

/// `KDF_n(ctx, ikm, n)`: the same, extended to `n` bytes with the XOF.
/// The first 32 bytes equal `kdf(ctx, ikm)`.
pub fn kdf_n(ctx: &str, ikm: &[u8], n: usize) -> Vec<u8> {
    debug_assert!(ctx.starts_with("sigil v1 "));
    let mut out = vec![0u8; n];
    blake3::Hasher::new_derive_key(ctx)
        .update(ikm)
        .finalize_xof()
        .fill(&mut out);
    out
}
