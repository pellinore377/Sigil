//! Experimental incremental ML-KEM-1024 adapter for ML-KEM Braid.
//! The header alone is not a validated public key. `finish` validates both
//! public parts before using the vector. Authenticate Braid's output separately.

use crate::{random_bytes, Error, Secret32};
use libcrux_ml_kem::mlkem1024::incremental as kem;
use zeroize::Zeroizing;

pub const HEADER_LEN: usize = kem::pk1_len();
pub const VECTOR_LEN: usize = kem::pk2_len();
pub const CT1_LEN: usize = kem::Ciphertext1::len();
pub const CT2_LEN: usize = kem::Ciphertext2::len();

#[derive(Clone)]
pub(super) struct Key {
    pub(super) seed: Zeroizing<[u8; 64]>,
    bytes: Zeroizing<[u8; kem::COMPRESSED_KEYPAIR_LEN]>,
}

impl Key {
    pub fn generate() -> Result<Self, Error> {
        Ok(Self::from_seed(&*random_bytes::<64>()?))
    }

    pub(super) fn from_seed(seed: &[u8; 64]) -> Self {
        let mut bytes = Zeroizing::new([0; kem::COMPRESSED_KEYPAIR_LEN]);
        kem::generate_key_pair_compressed(*seed, &mut bytes);
        Self {
            seed: Zeroizing::new(*seed),
            bytes,
        }
    }

    pub fn header(&self) -> [u8; HEADER_LEN] {
        // FIPS 203 expanded dk = dkPKE || t || rho || H(ek) || z.
        self.bytes[2 * VECTOR_LEN..2 * VECTOR_LEN + HEADER_LEN]
            .try_into()
            .expect("fixed ML-KEM-1024 layout")
    }

    pub fn vector(&self) -> [u8; VECTOR_LEN] {
        self.bytes[VECTOR_LEN..2 * VECTOR_LEN]
            .try_into()
            .expect("fixed ML-KEM-1024 layout")
    }

    pub fn decapsulate(&self, ct1: &[u8; CT1_LEN], ct2: &[u8; CT2_LEN]) -> Secret32 {
        Secret32::from_bytes(kem::decapsulate_compressed_key(
            &self.bytes,
            &kem::Ciphertext1 { value: *ct1 },
            &kem::Ciphertext2 { value: *ct2 },
        ))
    }
}

#[derive(Clone)]
pub(super) struct Encapsulation {
    pub(super) header: [u8; HEADER_LEN],
    pub(super) randomness: Zeroizing<[u8; 32]>,
    state: Zeroizing<[u8; kem::encaps_state_len()]>,
}

impl Encapsulation {
    pub fn start(header: &[u8; HEADER_LEN]) -> Result<(Self, [u8; CT1_LEN], Secret32), Error> {
        Self::with_randomness(header, &*random_bytes::<32>()?)
    }

    pub(super) fn with_randomness(
        header: &[u8; HEADER_LEN],
        randomness: &[u8; 32],
    ) -> Result<(Self, [u8; CT1_LEN], Secret32), Error> {
        let mut state = Zeroizing::new([0; kem::encaps_state_len()]);
        let mut shared = Zeroizing::new([0; 32]);
        let ct1 = kem::encapsulate1(header, *randomness, state.as_mut(), shared.as_mut())
            .map_err(|_| Error::InvalidKey)?;
        Ok((
            Self {
                header: *header,
                randomness: Zeroizing::new(*randomness),
                state,
            },
            ct1.value,
            Secret32(shared),
        ))
    }

    pub fn finish(self, vector: &[u8; VECTOR_LEN]) -> Result<[u8; CT2_LEN], Error> {
        self.validate(vector)?;
        Ok(kem::encapsulate2(&self.state, vector).value)
    }

    pub fn validate(&self, vector: &[u8; VECTOR_LEN]) -> Result<(), Error> {
        kem::validate_pk_bytes(&self.header, vector).map_err(|_| Error::InvalidKey)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{kem::encapsulate_with_randomness, KemKey};
    use sha3::{Digest, Sha3_256};

    fn hex(s: &str) -> Vec<u8> {
        s.as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }

    fn public_header(public: &[u8]) -> [u8; HEADER_LEN] {
        let mut header = [0; HEADER_LEN];
        header[..32].copy_from_slice(&public[VECTOR_LEN..]);
        header[32..].copy_from_slice(&Sha3_256::digest(public));
        header
    }

    #[test]
    fn incremental_matches_nist_vectors() {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/vectors/mlkem1024.json")).unwrap();
        let kg = &vectors["keygen"];
        let seed = hex(&format!(
            "{}{}",
            kg["d"].as_str().unwrap(),
            kg["z"].as_str().unwrap()
        ));
        let key = Key::from_seed(seed.as_slice().try_into().unwrap());
        let public = hex(kg["ek"].as_str().unwrap());
        assert_eq!(key.header(), public_header(&public));
        assert_eq!(key.vector().as_slice(), &public[..VECTOR_LEN]);
        let enc = &vectors["encapsulation"];
        let public = hex(enc["ek"].as_str().unwrap());
        let randomness = hex(enc["m"].as_str().unwrap());
        let (state, ct1, shared) = Encapsulation::with_randomness(
            &public_header(&public),
            randomness.as_slice().try_into().unwrap(),
        )
        .unwrap();
        let ct2 = state
            .finish(public[..VECTOR_LEN].try_into().unwrap())
            .unwrap();
        let expected = hex(enc["c"].as_str().unwrap());
        assert_eq!(ct1.as_slice(), &expected[..CT1_LEN]);
        assert_eq!(ct2.as_slice(), &expected[CT1_LEN..]);
        assert_eq!(shared.0.as_slice(), hex(enc["k"].as_str().unwrap()));
    }

    #[test]
    fn noncanonical_vector_rejected_even_with_matching_hash() {
        let key = Key::generate().unwrap();
        let mut public = key.vector().to_vec();
        public.extend_from_slice(&key.header()[..32]);
        public[0] = 255;
        public[1] = 255;
        let (state, _, _) = Encapsulation::start(&public_header(&public)).unwrap();
        assert_eq!(
            state.finish(public[..VECTOR_LEN].try_into().unwrap()),
            Err(Error::InvalidKey)
        );
    }

    #[test]
    fn incremental_matches_independent_complete_kem() {
        for n in 0..16u8 {
            let seed = [n; 64];
            let random = [n + 17; 32];
            let key = Key::from_seed(&seed);
            let reference = KemKey::from_seed(&seed);
            let mut public = key.vector().to_vec();
            public.extend_from_slice(&key.header()[..32]);
            assert_eq!(public, reference.public_key());
            let (state, ct1, shared) =
                Encapsulation::with_randomness(&key.header(), &random).unwrap();
            let ct2 = state.finish(&key.vector()).unwrap();
            let mut ciphertext = ct1.to_vec();
            ciphertext.extend_from_slice(&ct2);
            let (expected_ct, expected_key) =
                encapsulate_with_randomness(&public, &random).unwrap();
            assert_eq!(ciphertext, expected_ct);
            assert_eq!(*shared.0, *expected_key.0);
            assert_eq!(*key.decapsulate(&ct1, &ct2).0, *shared.0);
            assert_eq!(*reference.decapsulate(&ciphertext).unwrap().0, *shared.0);
            let mut bad_ct2 = ct2;
            bad_ct2[0] ^= 1;
            ciphertext[CT1_LEN] ^= 1;
            let rejected = key.decapsulate(&ct1, &bad_ct2);
            assert_ne!(*rejected.0, *shared.0);
            assert_eq!(*rejected.0, *reference.decapsulate(&ciphertext).unwrap().0);
        }
    }

    #[test]
    fn mismatched_public_parts_are_rejected() {
        let key = Key::generate().unwrap();
        let other = Key::generate().unwrap();
        let (state, _, _) = Encapsulation::start(&key.header()).unwrap();
        assert_eq!(state.finish(&other.vector()), Err(Error::InvalidKey));
        let mut header = key.header();
        header[32] ^= 1;
        let (state, _, _) = Encapsulation::start(&header).unwrap();
        assert_eq!(state.finish(&key.vector()), Err(Error::InvalidKey));
    }
}
