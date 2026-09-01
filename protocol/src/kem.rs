//! SigilKEM: X25519 and ML-KEM-768 combined. Both shared secrets, both
//! ciphertexts and both public keys go into the combiner, which is the
//! conservative generic construction and needs no property of either
//! component beyond IND-CCA.
//!
//! Sizes: public key 1216, secret seed 32, ciphertext 1120, shared secret 32.

use crate::kdf::{kdf, kdf_n};
use ml_kem::array::Array;
use ml_kem::kem::Decapsulate;
use ml_kem::{EncapsulateDeterministic, EncodedSizeUser, KemCore, MlKem768, MlKem768Params, B32};
use x25519_dalek::{PublicKey as XPublic, StaticSecret as XSecret};

pub const PUBLIC_KEY_LEN: usize = 32 + 1184;
pub const CIPHERTEXT_LEN: usize = 32 + 1088;
pub const SHARED_SECRET_LEN: usize = 32;

type MlEk = <MlKem768 as KemCore>::EncapsulationKey;
type MlDk = <MlKem768 as KemCore>::DecapsulationKey;

pub struct SecretKey {
    x: XSecret,
    m: MlDk,
    public: [u8; PUBLIC_KEY_LEN],
}

/// Derive a key pair from a 32-byte seed. Deterministic, so the spec can
/// carry vectors; production seeds come from the OS RNG.
pub fn keypair(seed: &[u8; 32]) -> SecretKey {
    let x = XSecret::from(kdf("sigil v1 kem x25519 seed", seed));
    let dz = kdf_n("sigil v1 kem mlkem seed", seed, 64);
    let d: B32 = Array::try_from(&dz[..32]).unwrap();
    let z: B32 = Array::try_from(&dz[32..]).unwrap();
    let (m, ek) = MlKem768::generate_deterministic(&d, &z);
    let mut public = [0u8; PUBLIC_KEY_LEN];
    public[..32].copy_from_slice(XPublic::from(&x).as_bytes());
    public[32..].copy_from_slice(&ek.as_bytes());
    SecretKey { x, m, public }
}

impl SecretKey {
    pub fn public(&self) -> &[u8; PUBLIC_KEY_LEN] {
        &self.public
    }

    pub fn decapsulate(&self, ct: &[u8]) -> crate::Result<[u8; 32]> {
        if ct.len() != CIPHERTEXT_LEN {
            return Err(crate::Error::Length);
        }
        let ex: [u8; 32] = ct[..32].try_into().unwrap();
        let ss_x = self.x.diffie_hellman(&XPublic::from(ex)).to_bytes();
        let mct: ml_kem::Ciphertext<MlKem768> = Array::try_from(&ct[32..]).unwrap();
        let ss_m = self.m.decapsulate(&mct).map_err(|_| crate::Error::Auth)?;
        Ok(combine(&ss_x, &ss_m, ct, &self.public))
    }
}

/// Encapsulate to `public` using a 32-byte ephemeral seed. Returns
/// (ciphertext, shared secret).
pub fn encapsulate(public: &[u8], eseed: &[u8; 32]) -> crate::Result<(Vec<u8>, [u8; 32])> {
    if public.len() != PUBLIC_KEY_LEN {
        return Err(crate::Error::Length);
    }
    let px: [u8; 32] = public[..32].try_into().unwrap();
    let ek_bytes: ml_kem::Encoded<MlEk> = Array::try_from(&public[32..]).unwrap();
    let ek = ml_kem::kem::EncapsulationKey::<MlKem768Params>::from_bytes(&ek_bytes);

    let ex = XSecret::from(kdf("sigil v1 kem x25519 eph", eseed));
    let ss_x = ex.diffie_hellman(&XPublic::from(px)).to_bytes();
    let m: B32 = Array::from(kdf("sigil v1 kem mlkem eph", eseed));
    let (mct, ss_m) = ek
        .encapsulate_deterministic(&m)
        .map_err(|_| crate::Error::Malformed)?;

    let mut ct = Vec::with_capacity(CIPHERTEXT_LEN);
    ct.extend_from_slice(XPublic::from(&ex).as_bytes());
    ct.extend_from_slice(&mct);
    let ss = combine(&ss_x, &ss_m, &ct, public);
    Ok((ct, ss))
}

fn combine(ss_x: &[u8; 32], ss_m: &[u8], ct: &[u8], public: &[u8]) -> [u8; 32] {
    let mut ikm = Vec::with_capacity(64 + CIPHERTEXT_LEN + PUBLIC_KEY_LEN);
    ikm.extend_from_slice(ss_x);
    ikm.extend_from_slice(ss_m);
    ikm.extend_from_slice(ct);
    ikm.extend_from_slice(public);
    kdf("sigil v1 kem combine", &ikm)
}
