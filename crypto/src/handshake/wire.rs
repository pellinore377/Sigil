use super::*;
use crate::kem::{CIPHERTEXT_LEN, PUBLIC_KEY_LEN};

// Magic, experimental version, suite, object kind, reserved byte. No fallback.
const BUNDLE_HEADER: &[u8; 8] = b"SGPQ\x00\x01\x01\x00";
const INITIAL_HEADER: &[u8; 8] = b"SGPQ\x00\x01\x02\x00";
const BUNDLE_BASE: usize = 8 + 33 + 33 + 64 + 1 + PUBLIC_KEY_LEN + 64 + 1;
const INITIAL_BASE: usize = 8 + 33 + 33 + 32 + CIPHERTEXT_LEN + 4;

struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8], Error> {
        let (field, rest) = self.0.split_at_checked(length).ok_or(Error::Encoding)?;
        self.0 = rest;
        Ok(field)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        self.take(N)?.try_into().map_err(|_| Error::Encoding)
    }

    fn expect(&mut self, expected: &[u8]) -> Result<(), Error> {
        if self.take(expected.len())? != expected {
            return Err(Error::Encoding);
        }
        Ok(())
    }

    fn ec(&mut self) -> Result<[u8; 32], Error> {
        self.expect(&[1])?;
        let key = self.array()?;
        validate_public(&key)?;
        Ok(key)
    }

    fn finish(self) -> Result<(), Error> {
        if !self.0.is_empty() {
            return Err(Error::Encoding);
        }
        Ok(())
    }
}

impl Bundle {
    /// Experimental, canonical public encoding. Contains no private keys.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(BUNDLE_BASE + 33);
        bytes.extend_from_slice(BUNDLE_HEADER);
        bytes.extend_from_slice(&encode_ec(&self.identity));
        bytes.extend_from_slice(&encode_ec(&self.signed_ec));
        bytes.extend_from_slice(&self.ec_signature);
        bytes.extend_from_slice(&encode_kem(&self.kem));
        bytes.extend_from_slice(&self.kem_signature);
        bytes.push(u8::from(self.one_time_ec.is_some()));
        if let Some(key) = self.one_time_ec {
            bytes.extend_from_slice(&encode_ec(&key));
        }
        bytes
    }

    /// Checks framing, key encodings and signatures against an independently
    /// selected identity. Downloading the expected identity alongside this bundle
    /// does not establish who owns that identity.
    pub fn from_bytes(bytes: &[u8], expected_identity: &[u8; 32]) -> Result<Self, Error> {
        if bytes.len() != BUNDLE_BASE && bytes.len() != BUNDLE_BASE + 33 {
            return Err(Error::Encoding);
        }
        let mut reader = Reader(bytes);
        reader.expect(BUNDLE_HEADER)?;
        let identity = reader.ec()?;
        let signed_ec = reader.ec()?;
        let ec_signature = reader.array()?;
        reader.expect(&[2])?;
        let kem = reader.take(PUBLIC_KEY_LEN)?.to_vec();
        let kem_signature = reader.array()?;
        let one_time_ec = match reader.array::<1>()? {
            [0] => None,
            [1] => Some(reader.ec()?),
            _ => return Err(Error::Encoding),
        };
        reader.finish()?;
        let bundle = Self {
            identity,
            signed_ec,
            ec_signature,
            kem,
            kem_signature,
            one_time_ec,
        };
        bundle.verify(expected_identity)?;
        Ok(bundle)
    }
}

impl InitialMessage {
    /// Unauthenticated routing hint until Receiver acceptance succeeds.
    pub fn recipient_bundle_id(&self) -> [u8; 32] {
        self.bundle_id
    }
    pub fn is_triple_ratchet(&self) -> bool {
        self.profile == 2
    }
    /// Experimental ciphertext encoding; this is not a delivery envelope.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(INITIAL_BASE + self.ciphertext.len());
        let mut header = *INITIAL_HEADER;
        header[5] = self.profile;
        bytes.extend_from_slice(&header);
        bytes.extend_from_slice(&encode_ec(&self.identity));
        bytes.extend_from_slice(&encode_ec(&self.ephemeral));
        bytes.extend_from_slice(&self.bundle_id);
        bytes.extend_from_slice(&self.kem_ciphertext);
        bytes.extend_from_slice(&(self.ciphertext.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&self.ciphertext);
        bytes
    }

    /// Structural validation only. Authentication and one-time key consumption
    /// happen in Receiver::accept, never while parsing untrusted bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        if !(INITIAL_BASE + 16..=INITIAL_BASE + MAX_PLAINTEXT + 16).contains(&bytes.len()) {
            return Err(Error::Limit);
        }
        // Check the only variable length before public-key work or allocation.
        let claimed = u32::from_be_bytes(
            bytes[INITIAL_BASE - 4..INITIAL_BASE]
                .try_into()
                .map_err(|_| Error::Encoding)?,
        ) as usize;
        if claimed != bytes.len() - INITIAL_BASE {
            return Err(Error::Encoding);
        }
        let mut reader = Reader(bytes);
        let mut header = *INITIAL_HEADER;
        header[5] = bytes[5];
        if header[5] != 1 && header[5] != 2 {
            return Err(Error::Encoding);
        }
        reader.expect(&header)?;
        let identity = reader.ec()?;
        let ephemeral = reader.ec()?;
        let bundle_id = reader.array()?;
        let kem_ciphertext = reader.take(CIPHERTEXT_LEN)?.to_vec();
        reader.take(4)?;
        let ciphertext = reader.take(claimed)?.to_vec();
        reader.finish()?;
        Ok(Self {
            profile: header[5],
            identity,
            ephemeral,
            bundle_id,
            kem_ciphertext,
            ciphertext,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peers_exchange_encoded_objects() {
        for ec in [false, true] {
            let alice = IdentityKey::generate().unwrap();
            let bob = IdentityKey::generate().unwrap();
            let mut receiver = Receiver::generate(&bob, ec).unwrap();
            let bytes = receiver.bundle().unwrap().to_bytes();
            assert_eq!(bytes.len(), BUNDLE_BASE + if ec { 33 } else { 0 });
            let bundle = Bundle::from_bytes(&bytes, &bob.public_key()).unwrap();
            assert_eq!(bytes, bundle.to_bytes());
            let (sent, initial) =
                initiate(&alice, &bob.public_key(), &bundle, b"synthetic").unwrap();
            let encoded = initial.to_bytes();
            let received = InitialMessage::from_bytes(&encoded).unwrap();
            assert_eq!(encoded, received.to_bytes());
            let (secret, text) = receiver
                .accept(&bob, &alice.public_key(), &received)
                .unwrap();
            assert_eq!(sent.0, secret.0);
            assert_eq!(text, b"synthetic");
            assert!(receiver
                .accept(&bob, &alice.public_key(), &received)
                .is_err());
        }
    }

    #[test]
    fn bundles_reject_truncation_extensions_and_unknown_headers() {
        let bob = IdentityKey::generate().unwrap();
        let receiver = Receiver::generate(&bob, true).unwrap();
        let bytes = receiver.bundle().unwrap().to_bytes();
        for end in 0..bytes.len() {
            assert!(Bundle::from_bytes(&bytes[..end], &bob.public_key()).is_err());
        }
        let mut extended = bytes.clone();
        extended.push(0);
        assert!(Bundle::from_bytes(&extended, &bob.public_key()).is_err());
        for index in 0..8 {
            let mut changed = bytes.clone();
            changed[index] ^= 1;
            assert!(Bundle::from_bytes(&changed, &bob.public_key()).is_err());
        }
        for index in [8, 41, 138, BUNDLE_BASE - 1, BUNDLE_BASE] {
            let mut changed = bytes.clone();
            changed[index] = 255;
            assert!(Bundle::from_bytes(&changed, &bob.public_key()).is_err());
        }
    }

    #[test]
    fn initial_frames_reject_truncation_lengths_and_unknown_headers() {
        let alice = IdentityKey::generate().unwrap();
        let bob = IdentityKey::generate().unwrap();
        let receiver = Receiver::generate(&bob, false).unwrap();
        let (_, initial) = initiate(
            &alice,
            &bob.public_key(),
            receiver.bundle().unwrap(),
            b"synthetic",
        )
        .unwrap();
        let bytes = initial.to_bytes();
        for end in 0..bytes.len() {
            assert!(InitialMessage::from_bytes(&bytes[..end]).is_err());
        }
        let mut changed = bytes.clone();
        changed.push(0);
        assert!(InitialMessage::from_bytes(&changed).is_err());
        for index in 0..8 {
            let mut changed = bytes.clone();
            changed[index] ^= 1;
            assert!(InitialMessage::from_bytes(&changed).is_err());
        }
        for length in [0u32, 15, u32::MAX] {
            let mut changed = bytes.clone();
            changed[INITIAL_BASE - 4..INITIAL_BASE].copy_from_slice(&length.to_be_bytes());
            assert!(InitialMessage::from_bytes(&changed).is_err());
        }
        assert!(Bundle::from_bytes(&bytes, &bob.public_key()).is_err());
        assert!(InitialMessage::from_bytes(&receiver.bundle().unwrap().to_bytes()).is_err());
    }

    #[test]
    fn malformed_and_unsigned_public_keys_are_rejected() {
        let bob = IdentityKey::generate().unwrap();
        let receiver = Receiver::generate(&bob, true).unwrap();
        let bytes = receiver.bundle().unwrap().to_bytes();
        for offset in [9, 42, BUNDLE_BASE + 1] {
            let mut changed = bytes.clone();
            changed[offset..offset + 32].fill(0);
            assert!(Bundle::from_bytes(&changed, &bob.public_key()).is_err());
        }
        let mut changed = bytes.clone();
        changed[139..142].fill(255); // Noncanonical ML-KEM coefficients.
        assert!(Bundle::from_bytes(&changed, &bob.public_key()).is_err());
        for offset in [74, 139 + PUBLIC_KEY_LEN] {
            let mut changed = bytes.clone();
            changed[offset] ^= 1;
            assert!(Bundle::from_bytes(&changed, &bob.public_key()).is_err());
        }
        let other = IdentityKey::generate().unwrap();
        assert!(Bundle::from_bytes(&bytes, &other.public_key()).is_err());
    }

    #[test]
    fn maximum_payload_and_untrusted_ciphertext() {
        let alice = IdentityKey::generate().unwrap();
        let bob = IdentityKey::generate().unwrap();
        let mut receiver = Receiver::generate(&bob, false).unwrap();
        let plaintext = vec![42; MAX_PLAINTEXT];
        let (_, initial) = initiate(
            &alice,
            &bob.public_key(),
            receiver.bundle().unwrap(),
            &plaintext,
        )
        .unwrap();
        let mut bytes = initial.to_bytes();
        assert_eq!(bytes.len(), INITIAL_BASE + MAX_PLAINTEXT + 16);
        bytes[INITIAL_BASE] ^= 1;
        let forged = InitialMessage::from_bytes(&bytes).unwrap();
        assert!(receiver.accept(&bob, &alice.public_key(), &forged).is_err());
        bytes[INITIAL_BASE] ^= 1;
        let decoded = InitialMessage::from_bytes(&bytes).unwrap();
        assert_eq!(
            receiver
                .accept(&bob, &alice.public_key(), &decoded)
                .unwrap()
                .1,
            plaintext
        );
        bytes.push(0);
        assert!(InitialMessage::from_bytes(&bytes).is_err());
    }
}
