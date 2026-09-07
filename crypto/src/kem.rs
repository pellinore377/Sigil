use crate::{random_bytes, Error, Secret32};
use ml_kem::{Decapsulate, DecapsulationKey, EncapsulationKey, KeyExport, MlKem1024};
use zeroize::Zeroize;

pub(crate) const PUBLIC_KEY_LEN: usize = 1568;
pub(crate) const CIPHERTEXT_LEN: usize = 1568;

pub(crate) fn parse_public(public: &[u8]) -> Result<EncapsulationKey<MlKem1024>, Error> {
    let encoded = public.try_into().map_err(|_| Error::InvalidKey)?;
    EncapsulationKey::<MlKem1024>::new(&encoded).map_err(|_| Error::InvalidKey)
}

pub struct KemKey(DecapsulationKey<MlKem1024>);
impl KemKey {
    pub(crate) fn seed(&self) -> Result<zeroize::Zeroizing<[u8; 64]>, Error> {
        let mut exported = self.0.to_seed().ok_or(Error::State)?;
        let seed = zeroize::Zeroizing::new(
            exported
                .as_slice()
                .try_into()
                .map_err(|_| Error::Encoding)?,
        );
        exported.zeroize();
        Ok(seed)
    }
    pub fn generate() -> Result<Self, Error> {
        Ok(Self::from_seed(&*random_bytes::<64>()?))
    }
    pub fn public_key(&self) -> Vec<u8> {
        self.0.encapsulation_key().to_bytes().to_vec()
    }
    pub fn decapsulate(&self, ciphertext: &[u8]) -> Result<Secret32, Error> {
        let encoded = ciphertext.try_into().map_err(|_| Error::InvalidKey)?;
        let mut shared = self.0.decapsulate(&encoded);
        let result = Secret32::from_bytes(
            shared
                .as_slice()
                .try_into()
                .map_err(|_| Error::InvalidKey)?,
        );
        shared.zeroize();
        Ok(result)
    }
    pub(crate) fn from_seed(seed: &[u8; 64]) -> Self {
        Self(DecapsulationKey::from_seed((*seed).into()))
    }
}

pub fn encapsulate(public: &[u8]) -> Result<(Vec<u8>, Secret32), Error> {
    encapsulate_with_randomness(public, &*random_bytes::<32>()?)
}

pub(crate) fn encapsulate_with_randomness(
    public: &[u8],
    random: &[u8; 32],
) -> Result<(Vec<u8>, Secret32), Error> {
    let key = parse_public(public)?;
    let mut randomness = (*random).into();
    let (ciphertext, mut shared) = key.encapsulate_deterministic(&randomness);
    randomness.zeroize();
    let result = Secret32::from_bytes(
        shared
            .as_slice()
            .try_into()
            .map_err(|_| Error::InvalidKey)?,
    );
    shared.zeroize();
    Ok((ciphertext.to_vec(), result))
}
