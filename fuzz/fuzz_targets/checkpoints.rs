#![no_main]
use libfuzzer_sys::fuzz_target;
use sigil_crypto::{storage::StorageKey, Secret32};
// Exercise the decoder after storage authentication using a disposable test key.
// This models malformed authenticated storage, not an AEAD authentication bypass.
fuzz_target!(|bytes: &[u8]| {
    if bytes.len()<33 || bytes.len()>32801 {return;}
    let identity: &[u8;32]=bytes[1..33].try_into().unwrap();
    let key=StorageKey::new(Secret32::from_bytes([7;32])).unwrap();
    let sealed=key.seal(&bytes[33..],b"Sigil/fuzz-only").unwrap();
    match bytes[0] {
        0=>if let Ok(session)=sigil_crypto::triple::Session::open_checkpoint(&key,&sealed,b"Sigil/fuzz-only") {
            let next=session.seal_checkpoint(&key,b"Sigil/fuzz-only").unwrap();
            let restored=sigil_crypto::triple::Session::open_checkpoint(&key,&next,b"Sigil/fuzz-only").unwrap();
            assert_eq!(restored.convergence_id(),session.convergence_id());
            assert_eq!(restored.peer_confirmed(),session.peer_confirmed());
        },
        1=>if let Ok(session)=sigil_crypto::ratchet::Session::open_checkpoint(&key,&sealed,b"Sigil/fuzz-only") {
            let next=session.seal_checkpoint(&key,b"Sigil/fuzz-only").unwrap();
            assert!(sigil_crypto::ratchet::Session::open_checkpoint(&key,&next,b"Sigil/fuzz-only").is_ok());
        },
        2=>if let Ok(slot)=sigil_crypto::handshake::Receiver::open_checkpoint(&key,&sealed,b"Sigil/fuzz-only",identity) {
            let next=slot.seal_checkpoint(&key,b"Sigil/fuzz-only").unwrap();
            assert!(sigil_crypto::handshake::Receiver::open_checkpoint(&key,&next,b"Sigil/fuzz-only",identity).is_ok());
        },
        3=>if let Ok(identity)=sigil_crypto::IdentityKey::open_checkpoint(&key,&sealed,b"Sigil/fuzz-only") {
            let next=identity.seal_checkpoint(&key,b"Sigil/fuzz-only").unwrap();
            assert_eq!(sigil_crypto::IdentityKey::open_checkpoint(&key,&next,b"Sigil/fuzz-only").unwrap().public_key(),identity.public_key());
        },
        _=>{}
    }
});
