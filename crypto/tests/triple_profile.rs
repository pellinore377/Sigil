//! Independent first-epoch profile oracle using HMAC/HKDF equations.
use aes_gcm_siv::{
    aead::{Aead, KeyInit, Payload},
    Aes256GcmSiv, Nonce,
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use sigil_crypto::{triple::Session, Secret32};
use x25519_dalek::{PublicKey, StaticSecret};

const T: &[u8] =
    b"Sigil/experimental/triple-ratchet/v0_X25519_MLKEM1024_SHA-256_AES256GCMSIV_RaptorQ64";
const P: &[u8] = b"Sigil/experimental/spqr/v0_MLKEM1024_SHA-256_RaptorQ64";
fn mac(key: &[u8], input: &[u8]) -> Vec<u8> {
    let mut h = <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(key).unwrap();
    h.update(input);
    h.finalize().into_bytes().to_vec()
}
fn hkdf(salt: &[u8], ikm: &[u8], info: &[u8], len: usize) -> Vec<u8> {
    let prk = mac(salt, ikm);
    let mut t = Vec::new();
    let mut result = Vec::new();
    for i in 1..=len.div_ceil(32) {
        t = mac(&prk, &[t.as_slice(), info, &[i as u8]].concat());
        result.extend_from_slice(&t);
    }
    result.truncate(len);
    result
}
fn open(key: &[u8], context: &[u8; 32], bytes: &[u8]) -> Result<Vec<u8>, aes_gcm_siv::Error> {
    let offset = 49 + usize::from(bytes[48]);
    Aes256GcmSiv::new_from_slice(key).unwrap().decrypt(
        Nonce::from_slice(&[0; 12]),
        Payload {
            msg: &bytes[offset..],
            aad: &[T, context, &bytes[..offset]].concat(),
        },
    )
}
#[test]
fn independent_initial_profile_keys_and_context_binding() {
    // Profile derivation is independent; target uses a fresh random initiator DH
    // key, whose public value is a legitimate input to our chosen responder key.
    for case in 1..=4u8 {
        let secret = [case; 32];
        let context = [case + 40; 32];
        let responder = StaticSecret::from([case + 10; 32]);
        let mut target = Session::initiator(
            Secret32::from_bytes(secret),
            PublicKey::from(&responder).to_bytes(),
            context,
        )
        .unwrap();
        let split = hkdf(&[0; 32], &secret, &[T, b":Initialize"].concat(), 64);
        let pq_init = hkdf(&[0; 32], &split[32..], &[P, b"Chain Start"].concat(), 96);
        let mut pq_chain = pq_init[32..64].to_vec();
        let mut ec_chain = None;
        for number in 0..9u32 {
            let plain = [case, number as u8, 0, 255];
            let bytes = target.send(&plain).unwrap().to_bytes();
            assert_eq!(
                u32::from_be_bytes(bytes[44..48].try_into().unwrap()),
                number
            );
            assert_eq!(
                u32::from_be_bytes(bytes[53..57].try_into().unwrap()),
                number + 1
            );
            assert_eq!(u64::from_be_bytes(bytes[57..65].try_into().unwrap()), 1);
            if ec_chain.is_none() {
                let remote = PublicKey::from(<[u8; 32]>::try_from(&bytes[8..40]).unwrap());
                let dh = responder.diffie_hellman(&remote);
                ec_chain = Some(
                    hkdf(
                        &split[..32],
                        dh.as_bytes(),
                        b"Sigil/experimental/root/v0",
                        64,
                    )[32..]
                        .to_vec(),
                );
            }
            let current = ec_chain.as_ref().unwrap();
            let ec_mk = mac(current, &[1]);
            ec_chain = Some(mac(current, &[2]));
            let pq_step = hkdf(
                &[0; 32],
                &pq_chain,
                &[P, b"Chain Step", &(number + 1).to_be_bytes()].concat(),
                64,
            );
            pq_chain = pq_step[..32].to_vec();
            let hybrid = hkdf(&pq_step[32..], &ec_mk, T, 32);
            assert_eq!(open(&hybrid, &context, &bytes).unwrap(), plain);
            let wrong_ec = hkdf(&pq_step[32..], &[0; 32], T, 32);
            let wrong_pq = hkdf(&[0; 32], &ec_mk, T, 32);
            assert!(open(&wrong_ec, &context, &bytes).is_err());
            assert!(open(&wrong_pq, &context, &bytes).is_err());
            assert!(open(&hybrid, &[0; 32], &bytes).is_err());
        }
    }
}
#[test]
fn independent_hkdf_matches_rfc5869_case_one() {
    let expected =
        "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865";
    let expected: Vec<u8> = (0..expected.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&expected[i..i + 2], 16).unwrap())
        .collect();
    assert_eq!(
        hkdf(
            &(0u8..=12).collect::<Vec<_>>(),
            &[11; 22],
            &(240u8..=249).collect::<Vec<_>>(),
            42
        ),
        expected
    );
}
