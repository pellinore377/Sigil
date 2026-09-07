use crate::{random_bytes, DhKey, Error, Secret32, MAX_AAD};
use curve25519_dalek::{
    edwards::EdwardsPoint,
    montgomery::MontgomeryPoint,
    scalar::{clamp_integer, Scalar},
};
use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha512};
use subtle::{Choice, ConditionallySelectable};
use zeroize::Zeroizing;

/// Experimental X25519 identity with XEdDSA prekey signatures.
pub struct IdentityKey(DhKey);

impl IdentityKey {
    pub fn seal_checkpoint(
        &self,
        key: &crate::storage::StorageKey,
        binding: &[u8],
    ) -> Result<Vec<u8>, Error> {
        let mut bytes = zeroize::Zeroizing::new([0; 40]);
        bytes[..8].copy_from_slice(b"SGIK\0\x01\0\0");
        bytes[8..].copy_from_slice(&zeroize::Zeroizing::new(self.0 .0.to_bytes())[..]);
        key.seal(bytes.as_ref(), binding)
    }

    pub fn open_checkpoint(
        key: &crate::storage::StorageKey,
        sealed: &[u8],
        binding: &[u8],
    ) -> Result<Self, Error> {
        let bytes = key.open(sealed, binding)?;
        if bytes.len() != 40 || &bytes[..8] != b"SGIK\0\x01\0\0" {
            return Err(Error::Encoding);
        }
        let secret = zeroize::Zeroizing::new(
            <[u8; 32]>::try_from(&bytes[8..]).map_err(|_| Error::Encoding)?,
        );
        Ok(Self(DhKey(x25519_dalek::StaticSecret::from(*secret))))
    }
    #[cfg(test)]
    pub(crate) fn from_test_bytes(bytes: [u8; 32]) -> Self {
        Self(DhKey::from_test_bytes(bytes))
    }

    pub fn generate() -> Result<Self, Error> {
        Ok(Self(DhKey::generate()?))
    }

    pub fn public_key(&self) -> [u8; 32] {
        self.0.public_key()
    }

    pub fn exchange(&self, public: &[u8; 32]) -> Result<Secret32, Error> {
        validate_public(public)?;
        self.0.exchange(public)
    }

    pub fn sign(&self, message: &[u8]) -> Result<[u8; 64], Error> {
        if message.len() > MAX_AAD {
            return Err(Error::Limit);
        }
        Ok(self.sign_with_randomness(message, &*random_bytes::<64>()?))
    }

    // XEdDSA rev. 1 §§2.3/3. Every scalar encoding is reduced modulo q,
    // including the positive orientation used in the randomized nonce hash.
    fn sign_with_randomness(&self, message: &[u8], randomness: &[u8; 64]) -> [u8; 64] {
        let secret = Zeroizing::new(clamp_integer(self.0 .0.to_bytes()));
        let scalar = Zeroizing::new(Scalar::from_bytes_mod_order(*secret));
        let mut public = EdwardsPoint::mul_base(&scalar).compress().to_bytes();
        let opposite = Zeroizing::new(-*scalar);
        let a = Zeroizing::new(Scalar::conditional_select(
            &scalar,
            &opposite,
            Choice::from(public[31] >> 7),
        ));
        public[31] &= 127;
        let encoded = Zeroizing::new(a.to_bytes());
        let mut prefix = [255; 32];
        prefix[0] = 254;
        let mut nonce_hash = Sha512::new();
        nonce_hash.update(prefix);
        nonce_hash.update(encoded.as_slice());
        nonce_hash.update(message);
        nonce_hash.update(randomness);
        let wide = Zeroizing::new(<[u8; 64]>::from(nonce_hash.finalize()));
        let r = Zeroizing::new(Scalar::from_bytes_mod_order_wide(&wide));
        let point = EdwardsPoint::mul_base(&r).compress().to_bytes();
        let mut challenge = Sha512::new();
        challenge.update(point);
        challenge.update(public);
        challenge.update(message);
        let hash = Zeroizing::new(<[u8; 64]>::from(challenge.finalize()));
        let h = Zeroizing::new(Scalar::from_bytes_mod_order_wide(&hash));
        let product = Zeroizing::new(*h * *a);
        let s = Zeroizing::new(*r + *product);
        let mut signature = [0; 64];
        signature[..32].copy_from_slice(&point);
        signature[32..].copy_from_slice(&s.to_bytes());
        signature
    }
}

/// Reject alternate field encodings and weak/twist keys at the protocol boundary.
/// Raw RFC 7748 operations intentionally retain their standard decoding rules.
pub(crate) fn validate_public(public: &[u8; 32]) -> Result<VerifyingKey, Error> {
    let mut modulus = [0xff; 32];
    modulus[0] = 0xed;
    modulus[31] = 0x7f;
    if public.iter().rev().cmp(modulus.iter().rev()).is_ge() {
        return Err(Error::InvalidKey);
    }
    let edwards = MontgomeryPoint(*public)
        .to_edwards(0)
        .ok_or(Error::InvalidKey)?
        .compress()
        .to_bytes();
    let key = VerifyingKey::from_bytes(&edwards).map_err(|_| Error::InvalidKey)?;
    if key.is_weak() {
        return Err(Error::InvalidKey);
    }
    Ok(key)
}

pub fn verify_signature(public: &[u8; 32], message: &[u8], signature: &[u8]) -> Result<(), Error> {
    if message.len() > MAX_AAD {
        return Err(Error::Limit);
    }
    let key = validate_public(public)?;
    let signature = Signature::from_slice(signature).map_err(|_| Error::Authentication)?;
    key.verify_strict(message, &signature)
        .map_err(|_| Error::Authentication)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signing_matches_independent_c_vectors_in_both_orientations() {
        fn field(case: &serde_json::Value, name: &str) -> Vec<u8> {
            case[name]
                .as_str()
                .unwrap()
                .as_bytes()
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                .collect()
        }
        let cases: serde_json::Value =
            serde_json::from_str(include_str!("../tests/vectors/xeddsa.json")).unwrap();
        let mut orientations = 0;
        for case in cases.as_array().unwrap() {
            let key = IdentityKey::from_test_bytes(field(case, "secret").try_into().unwrap());
            assert_eq!(key.public_key().as_slice(), field(case, "public"));
            let message = field(case, "message");
            let randomness: [u8; 64] = field(case, "random").try_into().unwrap();
            let signature = key.sign_with_randomness(&message, &randomness);
            assert_eq!(signature.as_slice(), field(case, "signature"));
            verify_signature(&key.public_key(), &message, &signature).unwrap();
            orientations |= 1 << case["orientation"].as_u64().unwrap();
        }
        assert_eq!(orientations, 3);
    }

    #[test]
    fn signing_hash_state_and_owned_scalars_have_erasure_contracts() {
        fn on_drop<T: zeroize::ZeroizeOnDrop>() {}
        on_drop::<Sha512>();
        on_drop::<sha2::Sha256>();
        on_drop::<Zeroizing<Scalar>>();
        let key = IdentityKey::generate().unwrap();
        let message = vec![42; MAX_AAD];
        verify_signature(&key.public_key(), &message, &key.sign(&message).unwrap()).unwrap();
        assert_eq!(key.sign(&vec![42; MAX_AAD + 1]), Err(Error::Limit));
        let a = key.sign_with_randomness(b"first", &[0; 64]);
        let b = key.sign_with_randomness(b"second", &[0; 64]);
        assert_ne!(&a[..32], &b[..32]);
    }

    #[test]
    fn signatures_bind_message_and_identity() {
        let key = IdentityKey::generate().unwrap();
        let message = b"synthetic prekey";
        let signature = key.sign(message).unwrap();
        verify_signature(&key.public_key(), message, &signature).unwrap();
        assert!(verify_signature(&key.public_key(), b"changed", &signature).is_err());
        let other = IdentityKey::generate().unwrap();
        assert!(verify_signature(&other.public_key(), message, &signature).is_err());
        for index in 0..64 {
            let mut changed = signature;
            changed[index] ^= 1;
            assert!(verify_signature(&key.public_key(), message, &changed).is_err());
        }
        assert!(verify_signature(&key.public_key(), message, &signature[..63]).is_err());
        let mut noncanonical = signature;
        noncanonical[63] |= 0x80;
        assert!(verify_signature(&key.public_key(), message, &noncanonical).is_err());
    }

    #[test]
    fn protocol_keys_reject_weak_and_noncanonical_encodings() {
        for public in [[0; 32], [0xff; 32]] {
            assert!(validate_public(&public).is_err());
        }
        let mut one = [0; 32];
        one[0] = 1;
        assert!(validate_public(&one).is_err());
        let mut modulus = [0xff; 32];
        modulus[0] = 0xed;
        modulus[31] = 0x7f;
        assert!(validate_public(&modulus).is_err());
        let mut public = IdentityKey::generate().unwrap().public_key();
        validate_public(&public).unwrap();
        public[31] |= 0x80;
        assert!(validate_public(&public).is_err());
    }

    #[test]
    fn identity_dh_and_signature_limits() {
        let alice = IdentityKey::generate().unwrap();
        let bob = IdentityKey::generate().unwrap();
        assert_eq!(
            alice.exchange(&bob.public_key()).unwrap().0,
            bob.exchange(&alice.public_key()).unwrap().0
        );
        assert_eq!(alice.sign(&vec![0; MAX_AAD + 1]), Err(Error::Limit));
        assert_eq!(
            verify_signature(&alice.public_key(), &vec![0; MAX_AAD + 1], &[0; 64]),
            Err(Error::Limit)
        );
    }
}

#[cfg(test)]
mod checkpoint_tests {
    use super::*;
    use crate::storage::StorageKey;
    #[test]
    fn encrypted_identity_checkpoint_is_bound_and_strictly_framed() {
        let identity = IdentityKey::generate().unwrap();
        let key = StorageKey::new(Secret32::from_bytes([9; 32])).unwrap();
        let sealed = identity.seal_checkpoint(&key, b"identity").unwrap();
        let restored = IdentityKey::open_checkpoint(&key, &sealed, b"identity").unwrap();
        assert_eq!(restored.public_key(), identity.public_key());
        verify_signature(
            &identity.public_key(),
            b"synthetic",
            &restored.sign(b"synthetic").unwrap(),
        )
        .unwrap();
        assert!(IdentityKey::open_checkpoint(&key, &sealed, b"other").is_err());
        let mut bytes = key.open(&sealed, b"identity").unwrap();
        bytes[4] = 1;
        assert!(IdentityKey::open_checkpoint(
            &key,
            &key.seal(&bytes, b"identity").unwrap(),
            b"identity"
        )
        .is_err());
        assert!(IdentityKey::open_checkpoint(
            &key,
            &key.seal(&bytes[..39], b"identity").unwrap(),
            b"identity"
        )
        .is_err());
    }
}
