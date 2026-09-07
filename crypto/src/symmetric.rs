use crate::{Error, Secret32};
use aes_gcm_siv::{
    aead::{Aead, KeyInit, Payload},
    Nonce,
};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use zeroize::{Zeroize, Zeroizing};

pub const MAX_PLAINTEXT: usize = 65536;
pub const MAX_AAD: usize = 4096;

type Aes256GcmSiv = aes_gcm_siv::AesGcmSiv<aes::Aes256>;

pub struct MessageKey(pub(crate) Secret32);

pub fn derive_root(root: &Secret32, dh: &Secret32) -> Result<(Secret32, Secret32), Error> {
    let mut output = Zeroizing::new([0; 64]);
    Hkdf::<Sha256>::new(Some(root.0.as_ref()), dh.0.as_ref())
        .expand(b"Sigil/experimental/root/v0", output.as_mut())
        .map_err(|_| Error::Limit)?;
    let mut next = Zeroizing::new([0; 32]);
    let mut chain = Zeroizing::new([0; 32]);
    next.copy_from_slice(&output[..32]);
    chain.copy_from_slice(&output[32..]);
    Ok((Secret32(next), Secret32(chain)))
}

pub fn derive_chain(chain: &Secret32) -> Result<(Secret32, MessageKey), Error> {
    fn hmac(key: &Secret32, constant: u8) -> Result<Secret32, Error> {
        let mut mac = <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(key.0.as_ref())
            .map_err(|_| Error::InvalidKey)?;
        mac.update(&[constant]);
        let mut bytes = mac.finalize().into_bytes();
        let secret = Secret32::from_bytes(bytes.into());
        bytes.zeroize();
        Ok(secret)
    }
    Ok((hmac(chain, 2)?, MessageKey(hmac(chain, 1)?)))
}

impl MessageKey {
    /// Consumes this key handle. The future ratchet must also prevent re-derivation and rollback.
    pub fn seal(self, plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>, Error> {
        self.seal_with_nonce(plaintext, aad, &[0; 12])
    }
    pub fn open(&self, ciphertext: &[u8], aad: &[u8]) -> Result<Vec<u8>, Error> {
        if ciphertext.len() < 16 || ciphertext.len() > MAX_PLAINTEXT + 16 || aad.len() > MAX_AAD {
            return Err(Error::Limit);
        }
        Aes256GcmSiv::new(self.0 .0.as_ref().into())
            .decrypt(
                Nonce::from_slice(&[0; 12]),
                Payload {
                    msg: ciphertext,
                    aad,
                },
            )
            .map_err(|_| Error::Authentication)
    }
    pub(crate) fn seal_with_nonce(
        &self,
        plaintext: &[u8],
        aad: &[u8],
        nonce: &[u8; 12],
    ) -> Result<Vec<u8>, Error> {
        if plaintext.len() > MAX_PLAINTEXT || aad.len() > MAX_AAD {
            return Err(Error::Limit);
        }
        Aes256GcmSiv::new(self.0 .0.as_ref().into())
            .encrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .map_err(|_| Error::Authentication)
    }
    #[cfg(test)]
    pub(crate) fn from_test_bytes(bytes: [u8; 32]) -> Self {
        Self(Secret32::from_bytes(bytes))
    }
}
