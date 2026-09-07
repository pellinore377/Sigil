use super::*;
use crate::storage::StorageKey;

impl Receiver {
    /// Only live slots can be sealed; consumed slots must remain durable tombstones.
    pub fn seal_checkpoint(&self, key: &StorageKey, binding: &[u8]) -> Result<Vec<u8>, Error> {
        let (kem, ec) = self.one_time.as_ref().ok_or(Error::State)?;
        let mut bytes = Zeroizing::new(Vec::with_capacity(2048));
        bytes.extend_from_slice(b"SGPK\0\x01\0\0");
        bytes.extend_from_slice(&Zeroizing::new(self.signed_ec.0.to_bytes())[..]);
        bytes.extend_from_slice(kem.seed()?.as_ref());
        bytes.push(u8::from(ec.is_some()));
        if let Some(ec) = ec {
            bytes.extend_from_slice(&Zeroizing::new(ec.0.to_bytes())[..]);
        }
        bytes.extend_from_slice(&self.bundle.to_bytes());
        key.seal(&bytes, binding)
    }

    pub fn open_checkpoint(
        key: &StorageKey,
        sealed: &[u8],
        binding: &[u8],
        expected_identity: &[u8; 32],
    ) -> Result<Self, Error> {
        let bytes = key.open(sealed, binding)?;
        if bytes.len() < 105 || &bytes[..8] != b"SGPK\0\x01\0\0" {
            return Err(Error::Encoding);
        }
        fn dh(bytes: &[u8]) -> Result<DhKey, Error> {
            let secret = Zeroizing::new(<[u8; 32]>::try_from(bytes).map_err(|_| Error::Encoding)?);
            Ok(DhKey(x25519_dalek::StaticSecret::from(*secret)))
        }
        let signed_ec = dh(&bytes[8..40])?;
        let seed =
            Zeroizing::new(<[u8; 64]>::try_from(&bytes[40..104]).map_err(|_| Error::Encoding)?);
        let kem = KemKey::from_seed(&seed);
        let (ec, offset) = match bytes[104] {
            0 => (None, 105),
            1 if bytes.len() >= 137 => (Some(dh(&bytes[105..137])?), 137),
            _ => return Err(Error::Encoding),
        };
        let bundle = Bundle::from_bytes(&bytes[offset..], expected_identity)?;
        if signed_ec.public_key() != bundle.signed_ec
            || kem.public_key() != bundle.kem
            || ec.as_ref().map(DhKey::public_key) != bundle.one_time_ec
        {
            return Err(Error::InvalidKey);
        }
        Ok(Self {
            signed_ec,
            one_time: Some((kem, ec)),
            bundle,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_prekey_checkpoints_validate_keys_identity_and_framing() {
        let identity = IdentityKey::generate().unwrap();
        let key = StorageKey::new(Secret32::from_bytes([9; 32])).unwrap();
        for ec in [false, true] {
            let receiver = Receiver::generate(&identity, ec).unwrap();
            let sealed = receiver.seal_checkpoint(&key, b"slot").unwrap();
            let restored =
                Receiver::open_checkpoint(&key, &sealed, b"slot", &identity.public_key()).unwrap();
            assert_eq!(
                restored.bundle().unwrap().to_bytes(),
                receiver.bundle().unwrap().to_bytes()
            );
            assert!(
                Receiver::open_checkpoint(&key, &sealed, b"other", &identity.public_key()).is_err()
            );
            assert!(Receiver::open_checkpoint(
                &key,
                &sealed,
                b"slot",
                &IdentityKey::generate().unwrap().public_key()
            )
            .is_err());
            let bytes = key.open(&sealed, b"slot").unwrap();
            for offset in [0, 4, 20, 50, 104] {
                let mut bad = Zeroizing::new(bytes.to_vec());
                bad[offset] ^= 0xff;
                assert!(Receiver::open_checkpoint(
                    &key,
                    &key.seal(&bad, b"slot").unwrap(),
                    b"slot",
                    &identity.public_key()
                )
                .is_err());
            }
            assert!(Receiver::open_checkpoint(
                &key,
                &key.seal(&bytes[..104], b"slot").unwrap(),
                b"slot",
                &identity.public_key()
            )
            .is_err());
            let mut bad = Zeroizing::new(bytes.to_vec());
            bad.push(0);
            assert!(Receiver::open_checkpoint(
                &key,
                &key.seal(&bad, b"slot").unwrap(),
                b"slot",
                &identity.public_key()
            )
            .is_err());
            let sender = IdentityKey::generate().unwrap();
            let (_, initial) = initiate_session(
                &sender,
                &identity.public_key(),
                receiver.bundle().unwrap(),
                b"synthetic",
            )
            .unwrap();
            let mut restored = restored;
            restored
                .accept_session(&identity, &sender.public_key(), &initial)
                .unwrap();
            assert!(restored.seal_checkpoint(&key, b"slot").is_err());
        }
    }
}
