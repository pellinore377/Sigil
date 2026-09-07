//! Public signed recovery control; parsing alone does not authenticate it.
const PREFIX: &[u8; 8] = b"SGRR\0\x01\0\0";
pub const SIGNED_BYTES: usize = 112;
pub const BYTES: usize = SIGNED_BYTES + 64;
#[derive(Clone, PartialEq, Eq)]
pub struct Request {
    pub message: [u8; 32],
    pub requester: [u8; 32],
    pub target: [u8; 32],
    pub expires_at: u64,
    pub signature: [u8; 64],
}
impl Request {
    pub fn signing_bytes(&self) -> Result<[u8; SIGNED_BYTES], &'static str> {
        if self.expires_at == 0 || self.expires_at > i64::MAX as u64 {
            return Err("invalid retry expiry");
        }
        let mut bytes = [0; SIGNED_BYTES];
        bytes[..8].copy_from_slice(PREFIX);
        bytes[8..40].copy_from_slice(&self.message);
        bytes[40..72].copy_from_slice(&self.requester);
        bytes[72..104].copy_from_slice(&self.target);
        bytes[104..].copy_from_slice(&self.expires_at.to_be_bytes());
        Ok(bytes)
    }
    pub fn to_bytes(&self) -> Result<Vec<u8>, &'static str> {
        Ok([self.signing_bytes()?.as_slice(), &self.signature].concat())
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.len() != BYTES || &bytes[..8] != PREFIX {
            return Err("invalid retry request");
        }
        let value = Self {
            message: bytes[8..40].try_into().map_err(|_| "invalid message")?,
            requester: bytes[40..72].try_into().map_err(|_| "invalid requester")?,
            target: bytes[72..104].try_into().map_err(|_| "invalid target")?,
            expires_at: u64::from_be_bytes(
                bytes[104..112].try_into().map_err(|_| "invalid expiry")?,
            ),
            signature: bytes[112..].try_into().map_err(|_| "invalid signature")?,
        };
        value.signing_bytes()?;
        Ok(value)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retry_framing_is_canonical_and_bounded() {
        let request = Request {
            message: [1; 32],
            requester: [2; 32],
            target: [3; 32],
            expires_at: 1000,
            signature: [4; 64],
        };
        let bytes = request.to_bytes().unwrap();
        assert!(Request::from_bytes(&bytes).unwrap() == request);
        for end in 0..bytes.len() {
            assert!(Request::from_bytes(&bytes[..end]).is_err());
        }
        assert!(Request::from_bytes(&[bytes.as_slice(), &[0]].concat()).is_err());
        for n in 0..8 {
            let mut bad = bytes.clone();
            bad[n] ^= 1;
            assert!(Request::from_bytes(&bad).is_err());
        }
        for expires_at in [0, u64::MAX] {
            assert!(Request {
                expires_at,
                ..request.clone()
            }
            .to_bytes()
            .is_err());
        }
    }
}
