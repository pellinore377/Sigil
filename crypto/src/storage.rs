//! Versioned local encryption. The wrapping key must be kept outside the database.
use crate::{random_bytes, Error, Secret32, MAX_AAD, MAX_PLAINTEXT};
use aes_gcm_siv::{
    aead::{Aead, KeyInit, Payload},
    Nonce,
};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use zeroize::Zeroizing;

type Cipher = aes_gcm_siv::AesGcmSiv<aes::Aes256>;
const PREFIX: &[u8; 8] = b"SGST\0\x01\0\0";
/// Largest experimental PQXDH initial packet, including its framing and AEAD tag.
pub const MAX_RECORD: usize = MAX_PLAINTEXT + 1694;

pub struct StorageKey {
    encryption: Secret32,
    commitment: Secret32,
}
impl StorageKey {
    pub fn new(master: Secret32) -> Result<Self, Error> {
        let mut keys = Zeroizing::new([0; 64]);
        Hkdf::<Sha256>::new(None, master.0.as_ref())
            .expand(b"Sigil/experimental/local-storage/v0", keys.as_mut())
            .map_err(|_| Error::Limit)?;
        Ok(Self {
            encryption: Secret32::from_bytes(keys[..32].try_into().map_err(|_| Error::Encoding)?),
            commitment: Secret32::from_bytes(keys[32..].try_into().map_err(|_| Error::Encoding)?),
        })
    }
    pub fn seal(&self, plaintext: &[u8], binding: &[u8]) -> Result<Vec<u8>, Error> {
        if plaintext.len() > MAX_RECORD || binding.len() > MAX_AAD {
            return Err(Error::Limit);
        }
        let nonce = random_bytes::<12>()?;
        let mut output = PREFIX.to_vec();
        output.extend_from_slice(nonce.as_ref());
        let mut aad = output.clone();
        aad.extend_from_slice(binding);
        let ciphertext = Cipher::new(self.encryption.0.as_ref().into())
            .encrypt(
                Nonce::from_slice(nonce.as_ref()),
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(|_| Error::Authentication)?;
        output.extend_from_slice(&ciphertext);
        Ok(output)
    }
    pub fn open(&self, ciphertext: &[u8], binding: &[u8]) -> Result<Zeroizing<Vec<u8>>, Error> {
        if !(36..=MAX_RECORD + 36).contains(&ciphertext.len()) || binding.len() > MAX_AAD {
            return Err(Error::Limit);
        }
        if &ciphertext[..8] != PREFIX {
            return Err(Error::Encoding);
        }
        let mut aad = ciphertext[..20].to_vec();
        aad.extend_from_slice(binding);
        Cipher::new(self.encryption.0.as_ref().into())
            .decrypt(
                Nonce::from_slice(&ciphertext[8..20]),
                Payload {
                    msg: &ciphertext[20..],
                    aad: &aad,
                },
            )
            .map(Zeroizing::new)
            .map_err(|_| Error::Authentication)
    }
    /// Keyed equality tag; does not expose an offline plaintext dictionary oracle.
    pub fn commitment(&self, bytes: &[u8], binding: &[u8]) -> Result<[u8; 32], Error> {
        if bytes.len() > MAX_PLAINTEXT + 64 || binding.len() > MAX_AAD {
            return Err(Error::Limit);
        }
        let mut mac = <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(self.commitment.0.as_ref())
            .map_err(|_| Error::InvalidKey)?;
        mac.update(&(binding.len() as u64).to_be_bytes());
        mac.update(binding);
        mac.update(bytes);
        Ok(mac.finalize().into_bytes().into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn records_bind_version_nonce_key_and_context() {
        let key = StorageKey::new(Secret32::from_bytes([1; 32])).unwrap();
        let first = key.seal(b"synthetic secret", b"row-a").unwrap();
        let second = key.seal(b"synthetic secret", b"row-a").unwrap();
        assert_ne!(first, second);
        assert_eq!(
            key.open(&first, b"row-a").unwrap().as_slice(),
            b"synthetic secret"
        );
        assert!(key.open(&first, b"row-b").is_err());
        let wrong = StorageKey::new(Secret32::from_bytes([2; 32])).unwrap();
        assert!(wrong.open(&first, b"row-a").is_err());
        for index in [0, 4, 5, 6, 7, 8, 19, 20, first.len() - 1] {
            let mut bad = first.clone();
            bad[index] ^= 1;
            assert!(key.open(&bad, b"row-a").is_err());
        }
        assert!(key.open(&first[..35], b"row-a").is_err());
        assert!(key.seal(&vec![0; MAX_RECORD + 1], b"row-a").is_err());
    }
    #[test]
    fn equality_tags_are_keyed_and_unambiguous() {
        let key = StorageKey::new(Secret32::from_bytes([1; 32])).unwrap();
        let other = StorageKey::new(Secret32::from_bytes([2; 32])).unwrap();
        assert_ne!(
            key.commitment(b"bc", b"a").unwrap(),
            key.commitment(b"c", b"ab").unwrap()
        );
        assert_ne!(
            key.commitment(b"bc", b"a").unwrap(),
            other.commitment(b"bc", b"a").unwrap()
        );
        assert_eq!(
            key.commitment(b"bc", b"a").unwrap(),
            key.commitment(b"bc", b"a").unwrap()
        );
    }
}
