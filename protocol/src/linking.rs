//! Device linking: the new device shows a QR code, an existing device scans
//! it, both derive a link secret, both show the same seven emoji, the user
//! confirms on the existing device, and the existing device sends the
//! identity across.
//!
//! Two slots carry the exchange, and both are ordinary slots (protocol
//! spec section 6) whose epoch secret is derived here: the **offer slot**,
//! from the offer alone, where the existing device leaves the KEM
//! ciphertext; and the **link slot**, from the link secret, where the
//! transfer itself happens. The new device has no tokens yet, so it only
//! ever reads; the existing device pays for every write.

use crate::encoding::{Reader, Writer};
use crate::epoch::{self, EpochMaterial};
use crate::kdf::{kdf, kdf_n};
use crate::kem;

/// What the QR code carries: version, the new device's SigilKEM public key,
/// and a 16-byte nonce.
pub struct LinkOffer {
    pub kem_pub: Vec<u8>,
    pub nonce: [u8; 16],
}

impl LinkOffer {
    pub fn encode(&self) -> Vec<u8> {
        Writer::new()
            .u8(crate::VERSION)
            .fixed(&self.kem_pub)
            .fixed(&self.nonce)
            .finish()
    }
    pub fn decode(b: &[u8]) -> crate::Result<LinkOffer> {
        let mut r = Reader::new(b);
        if r.u8()? != crate::VERSION {
            return Err(crate::Error::Malformed);
        }
        let kem_pub = r.fixed::<{ kem::PUBLIC_KEY_LEN }>()?.to_vec();
        let nonce = r.fixed()?;
        r.done()?;
        Ok(LinkOffer { kem_pub, nonce })
    }
    /// The offer slot: epoch material from `KDF("sigil v1 link offer", offer)`.
    pub fn slot(&self) -> EpochMaterial {
        epoch::derive(&kdf("sigil v1 link offer", &self.encode()))
    }
}

pub struct LinkMaterial {
    pub link_secret: [u8; 32],
    /// The link slot's epoch material (address, keys, envelope key).
    pub slot: EpochMaterial,
    /// Seven indices into the emoji table.
    pub sas: [u8; 7],
}

/// `link_secret = KDF("sigil v1 link secret", shared ‖ nonce)`.
pub fn derive(shared: &[u8; 32], offer: &LinkOffer) -> LinkMaterial {
    let mut ikm = shared.to_vec();
    ikm.extend_from_slice(&offer.nonce);
    let link_secret = kdf("sigil v1 link secret", &ikm);
    let sas_bytes = kdf_n("sigil v1 link sas", &link_secret, 7);
    let mut sas = [0u8; 7];
    for (i, b) in sas_bytes.iter().enumerate() {
        sas[i] = b & 0x3f;
    }
    LinkMaterial {
        link_secret,
        slot: epoch::derive(&kdf("sigil v1 link rendezvous", &link_secret)),
        sas,
    }
}

pub fn sas_string(sas: &[u8; 7]) -> String {
    sas.iter()
        .map(|&i| crate::emoji::TABLE[i as usize])
        .collect::<Vec<_>>()
        .join(" ")
}
