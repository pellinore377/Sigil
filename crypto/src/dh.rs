use crate::{random_bytes, Error, Secret32};
use x25519_dalek::{PublicKey, StaticSecret};

pub struct DhKey(pub(crate) StaticSecret);
impl DhKey {
    pub fn generate() -> Result<Self, Error> {
        Ok(Self(StaticSecret::from(*random_bytes::<32>()?)))
    }
    pub fn public_key(&self) -> [u8; 32] {
        PublicKey::from(&self.0).to_bytes()
    }
    pub fn exchange(&self, public: &[u8]) -> Result<Secret32, Error> {
        let encoded: [u8; 32] = public.try_into().map_err(|_| Error::InvalidKey)?;
        let shared = self.0.diffie_hellman(&PublicKey::from(encoded));
        if !shared.was_contributory() {
            return Err(Error::InvalidKey);
        }
        Ok(Secret32::from_bytes(shared.to_bytes()))
    }
    #[cfg(test)]
    pub(crate) fn from_test_bytes(bytes: [u8; 32]) -> Self {
        Self(StaticSecret::from(bytes))
    }
}
