//! Experimental device-link consent transcript. Parsing does not establish trust.
const PREFIX: &[u8; 8] = b"SGLT\0\x01\0\0";
pub const TRANSCRIPT_BYTES: usize = 216;
pub const MAX_LIFETIME: u64 = 600;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transcript {
    /// Hashes of the canonical unsigned device bindings, not their signatures.
    pub sponsor: [u8; 32],
    pub joining: [u8; 32],
    pub sponsor_challenge: [u8; 32],
    pub joining_challenge: [u8; 32],
    pub provisioning_key: [u8; 32],
    /// SHA-256 of the joining device's fresh random transport credential.
    pub credential_commitment: [u8; 32],
    pub created_at: u64,
    pub expires_at: u64,
}
impl Transcript {
    pub fn to_bytes(&self) -> Result<[u8; TRANSCRIPT_BYTES], &'static str> {
        let fields = [
            self.sponsor,
            self.joining,
            self.sponsor_challenge,
            self.joining_challenge,
            self.provisioning_key,
            self.credential_commitment,
        ];
        if fields.contains(&[0; 32])
            || self.sponsor == self.joining
            || self.sponsor_challenge == self.joining_challenge
            || self.created_at == 0
            || self.expires_at > i64::MAX as u64
            || self.expires_at <= self.created_at
            || self.expires_at - self.created_at > MAX_LIFETIME
        {
            return Err("invalid device-link transcript");
        }
        let mut bytes = [0; TRANSCRIPT_BYTES];
        bytes[..8].copy_from_slice(PREFIX);
        for (output, field) in bytes[8..200].as_chunks_mut::<32>().0.iter_mut().zip(fields) {
            output.copy_from_slice(&field);
        }
        bytes[200..208].copy_from_slice(&self.created_at.to_be_bytes());
        bytes[208..].copy_from_slice(&self.expires_at.to_be_bytes());
        Ok(bytes)
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, &'static str> {
        let bytes: &[u8; TRANSCRIPT_BYTES] =
            bytes.try_into().map_err(|_| "invalid device-link length")?;
        if &bytes[..8] != PREFIX {
            return Err("unsupported device-link transcript");
        }
        let field = |start| {
            bytes[start..start + 32]
                .try_into()
                .expect("fixed field bounds")
        };
        let value = Self {
            sponsor: field(8),
            joining: field(40),
            sponsor_challenge: field(72),
            joining_challenge: field(104),
            provisioning_key: field(136),
            credential_commitment: field(168),
            created_at: u64::from_be_bytes(bytes[200..208].try_into().expect("fixed time bounds")),
            expires_at: u64::from_be_bytes(bytes[208..].try_into().expect("fixed time bounds")),
        };
        value.to_bytes()?;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transcript_encoding_is_exact_bounded_and_rejects_invalid_lifetimes() {
        let value = Transcript {
            sponsor: [1; 32],
            joining: [2; 32],
            sponsor_challenge: [3; 32],
            joining_challenge: [4; 32],
            provisioning_key: [5; 32],
            credential_commitment: [6; 32],
            created_at: 1000,
            expires_at: 1600,
        };
        let bytes = value.to_bytes().unwrap();
        assert_eq!(Transcript::from_bytes(&bytes).unwrap(), value);
        for length in 0..bytes.len() {
            assert!(Transcript::from_bytes(&bytes[..length]).is_err());
        }
        assert!(Transcript::from_bytes(&[bytes.as_slice(), &[0]].concat()).is_err());
        for expiry in [0, 1000, 1601, u64::MAX] {
            let mut bad = value.clone();
            bad.expires_at = expiry;
            assert!(bad.to_bytes().is_err());
        }
        let mut bad = value.clone();
        bad.joining_challenge = bad.sponsor_challenge;
        assert!(bad.to_bytes().is_err());
        bad = value;
        bad.provisioning_key = [0; 32];
        assert!(bad.to_bytes().is_err());
    }
}

/// Public joining offer only. Contains no transport credential or private key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub device: [u8; 32],
    pub identity: [u8; 32],
    pub challenge: [u8; 32],
    pub provisioning_key: [u8; 32],
    pub credential_commitment: [u8; 32],
    pub created_at: u64,
    pub expires_at: u64,
}
impl Offer {
    pub const BYTES: usize = 184;
    pub fn to_bytes(&self) -> Result<[u8; Self::BYTES], &'static str> {
        let fields = [
            self.device,
            self.identity,
            self.challenge,
            self.provisioning_key,
            self.credential_commitment,
        ];
        if fields.contains(&[0; 32])
            || self.identity == self.provisioning_key
            || self.created_at == 0
            || self.expires_at <= self.created_at
            || self.expires_at > i64::MAX as u64
            || self.expires_at - self.created_at > MAX_LIFETIME
        {
            return Err("invalid device-link offer");
        }
        let mut bytes = [0; Self::BYTES];
        bytes[..8].copy_from_slice(b"SGLO\0\x01\0\0");
        for (output, field) in bytes[8..168].as_chunks_mut::<32>().0.iter_mut().zip(fields) {
            output.copy_from_slice(&field);
        }
        bytes[168..176].copy_from_slice(&self.created_at.to_be_bytes());
        bytes[176..].copy_from_slice(&self.expires_at.to_be_bytes());
        Ok(bytes)
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, &'static str> {
        let bytes: &[u8; Self::BYTES] = bytes
            .try_into()
            .map_err(|_| "invalid device-link offer length")?;
        if &bytes[..8] != b"SGLO\0\x01\0\0" {
            return Err("unsupported device-link offer");
        }
        let field = |start| {
            bytes[start..start + 32]
                .try_into()
                .expect("fixed offer field")
        };
        let value = Self {
            device: field(8),
            identity: field(40),
            challenge: field(72),
            provisioning_key: field(104),
            credential_commitment: field(136),
            created_at: u64::from_be_bytes(bytes[168..176].try_into().expect("fixed offer time")),
            expires_at: u64::from_be_bytes(bytes[176..].try_into().expect("fixed offer time")),
        };
        value.to_bytes()?;
        Ok(value)
    }
}

/// Complete public authorization proof. No credential or private key is included.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proof {
    pub transcript: Transcript,
    pub sponsor: crate::device::SignedBinding,
    pub joining: crate::device::SignedBinding,
    pub sponsor_signature: [u8; 64],
    pub joining_signature: [u8; 64],
}
impl Proof {
    pub const MAX_BYTES: usize = 1380;
    pub fn to_bytes(&self) -> Result<Vec<u8>, &'static str> {
        let mut bytes = b"SGLP\0\x01\0\0".to_vec();
        bytes.extend_from_slice(&self.transcript.to_bytes()?);
        for binding in [&self.sponsor, &self.joining] {
            let raw = binding.to_bytes()?;
            bytes.extend_from_slice(&(raw.len() as u16).to_be_bytes());
            bytes.extend_from_slice(&raw);
        }
        bytes.extend_from_slice(&self.sponsor_signature);
        bytes.extend_from_slice(&self.joining_signature);
        if bytes.len() > Self::MAX_BYTES {
            return Err("oversized linking proof");
        }
        Ok(bytes)
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.len() > Self::MAX_BYTES {
            return Err("oversized linking proof");
        }
        fn take<'a>(rest: &mut &'a [u8], n: usize) -> Result<&'a [u8], &'static str> {
            let (value, tail) = rest.split_at_checked(n).ok_or("truncated linking proof")?;
            *rest = tail;
            Ok(value)
        }
        let mut rest = bytes;
        if take(&mut rest, 8)? != b"SGLP\0\x01\0\0" {
            return Err("unsupported linking proof");
        }
        let transcript = Transcript::from_bytes(take(&mut rest, TRANSCRIPT_BYTES)?)?;
        let mut binding = || {
            let n = u16::from_be_bytes(
                take(&mut rest, 2)?
                    .try_into()
                    .map_err(|_| "invalid length")?,
            ) as usize;
            crate::device::SignedBinding::from_bytes(take(&mut rest, n)?)
        };
        let sponsor = binding()?;
        let joining = binding()?;
        let sponsor_signature = take(&mut rest, 64)?
            .try_into()
            .map_err(|_| "invalid signature")?;
        let joining_signature = take(&mut rest, 64)?
            .try_into()
            .map_err(|_| "invalid signature")?;
        if !rest.is_empty() {
            return Err("trailing linking proof");
        }
        Ok(Self {
            transcript,
            sponsor,
            joining,
            sponsor_signature,
            joining_signature,
        })
    }
}
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Authorization {
    pub proof: String,
}
impl Authorization {
    pub fn parse(&self) -> Result<Proof, &'static str> {
        if self.proof.len() > Proof::MAX_BYTES * 2 || !self.proof.len().is_multiple_of(2) {
            return Err("invalid linking proof encoding");
        }
        let mut bytes = Vec::with_capacity(self.proof.len() / 2);
        for pair in self.proof.as_bytes().as_chunks::<2>().0 {
            if !pair
                .iter()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
            {
                return Err("invalid linking proof encoding");
            }
            bytes.push(
                u8::from_str_radix(std::str::from_utf8(pair).map_err(|_| "invalid proof")?, 16)
                    .map_err(|_| "invalid proof")?,
            );
        }
        Proof::from_bytes(&bytes)
    }
}
