#![no_main]
use libfuzzer_sys::fuzz_target;
use sigil_crypto as c;
use sigil_protocol as p;
fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > 147456 {
        return;
    }
    if let Ok(value) = p::conversation::Operation::from_bytes(bytes) {
        assert_eq!(value.to_bytes().unwrap(), bytes);
    }
    if let Ok(value) = p::conversation::Snapshot::from_bytes(bytes) {
        assert_eq!(value.to_bytes().unwrap(), bytes);
    }
    if let Ok((initial, bootstrap)) = p::initial::decode(bytes) {
        assert_eq!(p::initial::encode(initial, bootstrap).unwrap(), bytes);
    }
    if let Ok(value) = p::event::Direct::from_bytes(bytes) {
        assert_eq!(value.to_bytes().unwrap(), bytes);
    }
    if let Ok(value) = p::event::Group::from_bytes(bytes) {
        assert_eq!(value.to_bytes().unwrap(), bytes);
    }
    if let Ok(value) = p::device::SignedBinding::from_bytes(bytes) {
        assert_eq!(value.to_bytes().unwrap(), bytes);
    }
    if let Ok(value) = p::file::File::from_bytes(bytes) {
        assert_eq!(value.to_bytes().unwrap(), bytes);
    }
    let _ = p::file::KeyDescriptor::from_bytes(bytes);
    if let Ok(value) = p::link::Transcript::from_bytes(bytes) {
        assert_eq!(value.to_bytes().unwrap().as_slice(), bytes);
    }
    if let Ok(value) = p::link::Offer::from_bytes(bytes) {
        assert_eq!(value.to_bytes().unwrap().as_slice(), bytes);
    }
    if let Ok(value) = p::link::Proof::from_bytes(bytes) {
        assert_eq!(value.to_bytes().unwrap(), bytes);
    }
    if let Ok(value) = p::retry::Request::from_bytes(bytes) {
        assert_eq!(value.to_bytes().unwrap(), bytes);
    }
    if let Ok(value) = c::triple::Packet::from_bytes(bytes) {
        assert_eq!(value.to_bytes(), bytes);
    }
    if let Ok(value) = c::ratchet::Packet::from_bytes(bytes) {
        assert_eq!(value.to_bytes(), bytes);
    }
    if let Ok(value) = c::handshake::InitialMessage::from_bytes(bytes) {
        assert_eq!(value.to_bytes(), bytes);
    }
    if let Ok(value) = c::sender_keys::Packet::from_bytes(bytes) {
        assert_eq!(value.to_bytes(), bytes);
    }
    if let Ok(value) = c::sender_keys::Distribution::from_bytes(bytes) {
        assert_eq!(value.to_bytes().unwrap().as_slice(), bytes);
    }
    // Decoder fuzzing only: taking this expected identity from the input is NOT
    // peer verification and is never used by application handshake acceptance.
    if let Some(identity) = bytes.get(9..41).and_then(|b| <&[u8; 32]>::try_from(b).ok()) {
        if let Ok(value) = c::handshake::Bundle::from_bytes(bytes, identity) {
            assert_eq!(value.to_bytes(), bytes);
        }
    }
});
