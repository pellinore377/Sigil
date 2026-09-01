//! Everything a conversation derives from one epoch secret. The epoch
//! secret comes from the group key schedule (in v1, MLS's exporter with
//! label `"sigil v1 epoch"`); nothing below depends on where it came from.

use crate::kdf::kdf;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

pub struct EpochMaterial {
    pub slot_seed: [u8; 32],
    pub read_cap: [u8; 32],
    pub write_key: SigningKey,
    pub write_pub: [u8; 32],
    pub address: [u8; 32],
    pub envelope_key: [u8; 32],
    pub call_room: [u8; 32],
}

pub fn derive(epoch_secret: &[u8; 32]) -> EpochMaterial {
    let slot_seed = kdf("sigil v1 slot seed", epoch_secret);
    let read_cap = kdf("sigil v1 slot read", &slot_seed);
    let write_key = SigningKey::from_bytes(&kdf("sigil v1 slot write", &slot_seed));
    let write_pub = write_key.verifying_key().to_bytes();
    let address = slot_address(&read_cap, &write_pub);
    EpochMaterial {
        slot_seed,
        read_cap,
        write_key,
        write_pub,
        address,
        envelope_key: kdf("sigil v1 envelope key", epoch_secret),
        call_room: kdf("sigil v1 call room", epoch_secret),
    }
}

/// `address = KDF("sigil v1 slot address", read_cap ‖ write_pub)`. The
/// server recomputes this on a read to check the capability.
pub fn slot_address(read_cap: &[u8; 32], write_pub: &[u8; 32]) -> [u8; 32] {
    let mut ikm = read_cap.to_vec();
    ikm.extend_from_slice(write_pub);
    kdf("sigil v1 slot address", &ikm)
}

const PUT_CTX: &[u8] = b"sigil v1 slot put";

/// Signature a writer attaches to a `slot.put`: over
/// `"sigil v1 slot put" ‖ address ‖ envelope`.
pub fn sign_put(write_key: &SigningKey, address: &[u8; 32], envelope: &[u8]) -> [u8; 64] {
    let mut msg = PUT_CTX.to_vec();
    msg.extend_from_slice(address);
    msg.extend_from_slice(envelope);
    write_key.sign(&msg).to_bytes()
}

pub fn verify_put(
    write_pub: &[u8; 32],
    address: &[u8; 32],
    envelope: &[u8],
    sig: &[u8; 64],
) -> crate::Result<()> {
    let vk = VerifyingKey::from_bytes(write_pub).map_err(|_| crate::Error::Malformed)?;
    let mut msg = PUT_CTX.to_vec();
    msg.extend_from_slice(address);
    msg.extend_from_slice(envelope);
    vk.verify(&msg, &Signature::from_bytes(sig))
        .map_err(|_| crate::Error::Auth)
}
