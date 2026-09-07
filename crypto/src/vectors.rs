use crate::*;
use hkdf::Hkdf;
use sha2::Sha256;

fn hex(value: &str) -> Vec<u8> {
    assert!(value.len().is_multiple_of(2));
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
        .collect()
}
fn bytes<const N: usize>(value: &str) -> [u8; N] {
    hex(value).try_into().unwrap()
}

pub(crate) fn ratchet_vector(name: &str) -> Vec<u8> {
    let vectors: serde_json::Value =
        serde_json::from_str(include_str!("../tests/vectors/ratchet.json")).unwrap();
    hex(vectors[name].as_str().unwrap())
}

#[test]
fn xeddsa_verification_matches_rfc8032_signature() {
    // RFC 8032 section 7.1, test 1. Its Edwards public key has sign bit zero,
    // so the published signature also verifies after Montgomery conversion.
    // This tests verification, not XEdDSA's distinct randomized signing recipe.
    let edwards = bytes("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a");
    let public = curve25519_dalek::edwards::CompressedEdwardsY(edwards)
        .decompress()
        .unwrap()
        .to_montgomery()
        .to_bytes();
    let signature = hex(concat!(
        "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155",
        "5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"
    ));
    verify_signature(&public, b"", &signature).unwrap();
    assert!(verify_signature(&public, b"changed", &signature).is_err());
}

#[test]
fn x25519_rfc7748_shared_secret() {
    let alice = DhKey::from_test_bytes(bytes(
        "77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a",
    ));
    let bob = DhKey::from_test_bytes(bytes(
        "5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb",
    ));
    assert_eq!(
        alice.public_key(),
        bytes("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a")
    );
    assert_eq!(
        bob.public_key(),
        bytes("de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f")
    );
    let expected = bytes::<32>("4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742");
    assert_eq!(*alice.exchange(&bob.public_key()).unwrap().0, expected);
    assert_eq!(*bob.exchange(&alice.public_key()).unwrap().0, expected);
}

#[test]
fn x25519_rejects_wrong_lengths_and_noncontributory_keys() {
    let key = DhKey::generate().unwrap();
    for public in [&[0u8; 31][..], &[0u8; 33][..], &[0u8; 32][..]] {
        assert!(key.exchange(public).is_err());
    }
    let mut one = [0; 32];
    one[0] = 1;
    assert!(key.exchange(&one).is_err());
}

#[test]
fn hkdf_sha256_rfc5869_case_one() {
    let ikm = [0x0b; 22];
    let salt = hex("000102030405060708090a0b0c");
    let info = hex("f0f1f2f3f4f5f6f7f8f9");
    let mut output = [0; 42];
    Hkdf::<Sha256>::new(Some(&salt), &ikm)
        .expand(&info, &mut output)
        .unwrap();
    assert_eq!(
        output.as_slice(),
        hex("3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865")
    );
}

#[test]
fn aes256_gcm_siv_rfc8452_empty_and_eight_bytes() {
    let mut key = [0; 32];
    key[0] = 1;
    let message = MessageKey::from_test_bytes(key);
    let mut nonce = [0; 12];
    nonce[0] = 3;
    assert_eq!(
        message.seal_with_nonce(&[], &[], &nonce).unwrap(),
        hex("07f5f4169bbf55a8400cd47ea6fd400f")
    );
    assert_eq!(
        message
            .seal_with_nonce(&hex("0100000000000000"), &[], &nonce)
            .unwrap(),
        hex("c2ef328e5c71c83b843122130f7364b761e0b97427e3df28")
    );
}

#[test]
fn chain_derivation_authentication_and_limits() {
    let chain = Secret32::from_bytes([7; 32]);
    let (next, send) = derive_chain(&chain).unwrap();
    let (_, receive) = derive_chain(&chain).unwrap();
    assert!(*next.0 != *chain.0);
    let encrypted = send.seal(b"synthetic message", b"bound header").unwrap();
    assert_eq!(
        receive.open(&encrypted, b"bound header").unwrap(),
        b"synthetic message"
    );
    assert_eq!(
        receive.open(&encrypted, b"changed header"),
        Err(Error::Authentication)
    );
    for index in 0..encrypted.len() {
        let mut corrupted = encrypted.clone();
        corrupted[index] ^= 1;
        assert_eq!(
            receive.open(&corrupted, b"bound header"),
            Err(Error::Authentication)
        );
    }
    assert_eq!(
        receive.open(&encrypted, b"bound header").unwrap(),
        b"synthetic message"
    );
    assert!(receive.open(&[0; 15], &[]).is_err());
    let (_, send) = derive_chain(&next).unwrap();
    assert_eq!(
        send.seal(&vec![0; MAX_PLAINTEXT + 1], &[]),
        Err(Error::Limit)
    );
    assert_eq!(
        receive.open(&encrypted, &vec![0; MAX_AAD + 1]),
        Err(Error::Limit)
    );
}

#[test]
fn root_kdf_is_deterministic_and_binds_both_inputs() {
    let root = Secret32::from_bytes([1; 32]);
    let dh = Secret32::from_bytes([2; 32]);
    let (a, b) = derive_root(&root, &dh).unwrap();
    let (c, d) = derive_root(&root, &dh).unwrap();
    assert_eq!(*a.0, *c.0);
    assert_eq!(*b.0, *d.0);
    assert!(*a.0 != *b.0);
    let (other, _) = derive_root(&dh, &root).unwrap();
    assert!(*a.0 != *other.0);
}

#[test]
fn mlkem1024_matches_nist_keygen_and_encapsulation_vectors() {
    let value: serde_json::Value =
        serde_json::from_str(include_str!("../tests/vectors/mlkem1024.json")).unwrap();
    let keygen = &value["keygen"];
    let seed = format!(
        "{}{}",
        keygen["d"].as_str().unwrap(),
        keygen["z"].as_str().unwrap()
    );
    let key = KemKey::from_seed(&bytes(&seed));
    assert_eq!(key.public_key(), hex(keygen["ek"].as_str().unwrap()));
    let encap = &value["encapsulation"];
    let (ciphertext, secret) = crate::kem::encapsulate_with_randomness(
        &hex(encap["ek"].as_str().unwrap()),
        &bytes(encap["m"].as_str().unwrap()),
    )
    .unwrap();
    assert_eq!(ciphertext, hex(encap["c"].as_str().unwrap()));
    assert_eq!(*secret.0, bytes(encap["k"].as_str().unwrap()));
}

#[test]
fn mlkem_rejects_encoding_errors_and_implicitly_rejects_corruption() {
    let key = KemKey::generate().unwrap();
    let (mut ct, sent) = encapsulate(&key.public_key()).unwrap();
    assert_eq!(*key.decapsulate(&ct).unwrap().0, *sent.0);
    ct[0] ^= 1;
    assert!(*key.decapsulate(&ct).unwrap().0 != *sent.0);
    assert!(key.decapsulate(&ct[..ct.len() - 1]).is_err());
    assert!(encapsulate(&[0; 32]).is_err());
    let mut malformed = key.public_key();
    malformed[0] = 255;
    malformed[1] = 255;
    assert!(encapsulate(&malformed).is_err());
}

#[test]
fn root_and_chain_outputs_match_openssl_reference() {
    let (root, chain) = derive_root(
        &Secret32::from_bytes([1; 32]),
        &Secret32::from_bytes([2; 32]),
    )
    .unwrap();
    assert_eq!(
        *root.0,
        bytes("5bd838aa92f6bbf2fbad5bafd8f43e03de2ef77cdb2af857945c1431f370fff1")
    );
    assert_eq!(
        *chain.0,
        bytes("d510fdf3857a269d398f96aae7c2956ff1e5b5fdc626bb40f103c897e8d2b1bb")
    );
    let (next, message) = derive_chain(&Secret32::from_bytes([7; 32])).unwrap();
    assert_eq!(
        *next.0,
        bytes("469e03123f1c9c6d5c3ffaa260c7fef9f863c0b6af93f7dcae5c13cbfdadfca5")
    );
    let reference = MessageKey::from_test_bytes(bytes(
        "3d8b2fa2a2537fdcdda46e704966a105722d268d2e9ed43bbe5b1dd9d6802935",
    ));
    assert_eq!(
        message.seal(b"synthetic", b"context").unwrap(),
        reference.seal(b"synthetic", b"context").unwrap()
    );
}
