//! Identity: an Ed25519 signing key plus a SigilKEM key, both derived from
//! one 32-byte identity seed, and the signed contact card.

use crate::encoding::{Reader, Writer};
use crate::kdf::kdf;
use crate::kem;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

pub struct Identity {
    pub signing: SigningKey,
    pub kem: kem::SecretKey,
}

impl Identity {
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        let signing = SigningKey::from_bytes(&kdf("sigil v1 identity signing", seed));
        let kem = kem::keypair(&kdf("sigil v1 identity kem", seed));
        Identity { signing, kem }
    }
    pub fn public(&self) -> [u8; 32] {
        self.signing.verifying_key().to_bytes()
    }
}

/// Fingerprint: 32 bytes bound to the identity public key.
pub fn fingerprint(identity_pub: &[u8; 32]) -> [u8; 32] {
    kdf("sigil v1 fingerprint", identity_pub)
}

/// Display form: the first 20 bytes of the fingerprint as lowercase hex,
/// in ten groups of four characters separated by spaces.
pub fn fingerprint_display(fp: &[u8; 32]) -> String {
    let h = hex::encode(&fp[..20]);
    h.as_bytes()
        .chunks(4)
        .map(|c| core::str::from_utf8(c).unwrap())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A contact card: everything a stranger needs to start a conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactCard {
    pub username: String,
    pub identity_pub: [u8; 32],
    pub kem_pub: Vec<u8>,
    /// Server that hosts this user's conversation slots.
    pub slot_server: String,
    /// Bit 0: server offers TPM recovery. Other bits reserved, must be 0.
    pub flags: u8,
}

const CARD_CTX: &[u8] = b"sigil v1 card";

impl ContactCard {
    fn body(&self) -> Vec<u8> {
        Writer::new()
            .u8(crate::VERSION)
            .str(&self.username)
            .fixed(&self.identity_pub)
            .fixed(&self.kem_pub)
            .str(&self.slot_server)
            .u8(self.flags)
            .finish()
    }

    /// Body followed by a 64-byte Ed25519 signature over `"sigil v1 card" ‖ body`.
    pub fn sign(&self, id: &Identity) -> Vec<u8> {
        let mut body = self.body();
        let mut msg = CARD_CTX.to_vec();
        msg.extend_from_slice(&body);
        body.extend_from_slice(&id.signing.sign(&msg).to_bytes());
        body
    }

    pub fn verify(encoded: &[u8]) -> crate::Result<ContactCard> {
        if encoded.len() < 64 {
            return Err(crate::Error::Length);
        }
        let (body, sig) = encoded.split_at(encoded.len() - 64);
        let mut r = Reader::new(body);
        if r.u8()? != crate::VERSION {
            return Err(crate::Error::Malformed);
        }
        let username = r.str()?.to_string();
        let identity_pub: [u8; 32] = r.fixed()?;
        let kem_pub = r.fixed::<{ kem::PUBLIC_KEY_LEN }>()?.to_vec();
        let slot_server = r.str()?.to_string();
        let flags = r.u8()?;
        r.done()?;
        crate::names::parse_username(&username)?;
        let vk = VerifyingKey::from_bytes(&identity_pub).map_err(|_| crate::Error::Malformed)?;
        let mut msg = CARD_CTX.to_vec();
        msg.extend_from_slice(body);
        let sig = Signature::from_slice(sig).map_err(|_| crate::Error::Malformed)?;
        vk.verify(&msg, &sig).map_err(|_| crate::Error::Auth)?;
        Ok(ContactCard {
            username,
            identity_pub,
            kem_pub,
            slot_server,
            flags,
        })
    }
}
