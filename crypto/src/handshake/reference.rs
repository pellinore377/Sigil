use super::*;

#[test]
fn openssl_and_libsodium_checked_handshakes() {
    fn bytes(case: &serde_json::Value, name: &str) -> Vec<u8> {
        let value = case[name].as_str().unwrap().as_bytes();
        assert!(value.len().is_multiple_of(2));
        value
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }
    for source in [
        include_str!("../../tests/vectors/pqxdh.json"),
        include_str!("../../tests/vectors/pqxdh-triple.json"),
    ] {
        let cases: Vec<serde_json::Value> = serde_json::from_str(source).unwrap();
        assert_eq!(cases.len(), 2);
        let alice = IdentityKey::from_test_bytes([1; 32]);
        let bob = IdentityKey::from_test_bytes([2; 32]);
        for case in cases {
            let mut receiver = receiver(&bob, case["ec"].as_bool().unwrap());
            let bundle = Bundle::from_bytes(&bytes(&case, "bundle"), &bob.public_key()).unwrap();
            assert_eq!(bundle.id(), receiver.bundle.id());
            let mut initial = InitialMessage::from_bytes(&bytes(&case, "initial")).unwrap();
            initial.ciphertext[0] ^= 1;
            assert!(receiver
                .accept(&bob, &alice.public_key(), &initial)
                .is_err());
            initial.ciphertext[0] ^= 1;
            let (secret, plaintext) = receiver
                .accept(&bob, &alice.public_key(), &initial)
                .unwrap();
            assert_eq!(secret.0.as_ref(), bytes(&case, "secret"));
            assert_eq!(plaintext, bytes(&case, "plaintext"));
            assert!(receiver
                .accept(&bob, &alice.public_key(), &initial)
                .is_err());
        }
    }
}

fn receiver(identity: &IdentityKey, ec: bool) -> Receiver {
    let signed_ec = DhKey::from_test_bytes([3; 32]);
    let kem = KemKey::from_seed(&[4; 64]);
    let one_time_ec = ec.then(|| DhKey::from_test_bytes([5; 32]));
    let bundle = Bundle {
        identity: identity.public_key(),
        signed_ec: signed_ec.public_key(),
        ec_signature: identity.sign(&encode_ec(&signed_ec.public_key())).unwrap(),
        kem_signature: identity.sign(&encode_kem(&kem.public_key())).unwrap(),
        kem: kem.public_key(),
        one_time_ec: one_time_ec.as_ref().map(DhKey::public_key),
    };
    Receiver {
        signed_ec,
        one_time: Some((kem, one_time_ec)),
        bundle,
    }
}

/// Explicit maintenance tool: output must pass the independent C checker before
/// replacing the checked-in fixture. Random signatures/ephemerals make new output differ.
#[test]
#[ignore = "generates synthetic reference input in /tmp; run the C checker before adopting"]
fn generate_reference_inputs() {
    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
    let alice = IdentityKey::from_test_bytes([1; 32]);
    let bob = IdentityKey::from_test_bytes([2; 32]);
    for profile in [1, 2] {
        let cases: Vec<_> = [false, true]
            .into_iter()
            .map(|ec| {
                let receiver = receiver(&bob, ec);
                let plaintext = b"Sigil synthetic reference";
                let (secret, initial) = initiate_profile(
                    &alice,
                    &bob.public_key(),
                    &receiver.bundle,
                    plaintext,
                    profile,
                )
                .unwrap();
                serde_json::json!({"ec": ec, "bundle": hex(&receiver.bundle.to_bytes()),
            "initial": hex(&initial.to_bytes()), "secret": hex(secret.0.as_ref()),
            "plaintext": hex(plaintext)})
            })
            .collect();
        std::fs::write(
            format!("/tmp/sigil-pqxdh-profile-{profile}-vectors.json"),
            serde_json::to_vec_pretty(&cases).unwrap(),
        )
        .unwrap();
    }
}
