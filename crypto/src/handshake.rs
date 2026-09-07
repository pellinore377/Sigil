//! Restricted PQXDH evaluation: mandatory one-time ML-KEM prekey, optional EC one-time key.
//! Experimental encoding with encrypted private-slot checkpoints; last-resort key lifecycle remains open.
mod checkpoint;
#[cfg(test)]
mod reference;
mod wire;
use crate::{
    encapsulate, identity::validate_public, verify_signature, DhKey, Error, IdentityKey, KemKey,
    MessageKey, Secret32, MAX_PLAINTEXT,
};
use hkdf::Hkdf;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

const CONTEXT: &[u8] = b"Sigil/experimental/pqxdh/v0_CURVE25519_SHA-256_ML-KEM-1024";
const TR_INITIAL: &[u8] = b"Sigil/experimental/pqxdh/initial/v1_TripleRatchet";

pub struct Bundle {
    identity: [u8; 32],
    signed_ec: [u8; 32],
    ec_signature: [u8; 64],
    kem: Vec<u8>,
    kem_signature: [u8; 64],
    one_time_ec: Option<[u8; 32]>,
}

pub struct InitialMessage {
    profile: u8,
    identity: [u8; 32],
    ephemeral: [u8; 32],
    bundle_id: [u8; 32],
    kem_ciphertext: Vec<u8>,
    ciphertext: Vec<u8>,
}

/// One private prekey slot. Successful authentication consumes it in memory only.
pub struct Receiver {
    signed_ec: DhKey,
    one_time: Option<(KemKey, Option<DhKey>)>,
    bundle: Bundle,
}

fn encode_ec(key: &[u8; 32]) -> Vec<u8> {
    let mut encoded = vec![1];
    encoded.extend_from_slice(key);
    encoded
}

fn encode_kem(key: &[u8]) -> Vec<u8> {
    let mut encoded = vec![2];
    encoded.extend_from_slice(key);
    encoded
}

impl Bundle {
    /// Stable one-time KEM identifier, matching the server's publication ID.
    pub fn prekey_id(&self) -> [u8; 32] {
        Sha256::digest(&self.kem).into()
    }
    /// Hash canonical public keys, not randomized signatures. Identifies the
    /// recipient slot in an initial packet; it does not establish peer trust.
    pub fn id(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(CONTEXT);
        hash.update(encode_ec(&self.identity));
        hash.update(encode_ec(&self.signed_ec));
        hash.update(encode_kem(&self.kem));
        hash.update([u8::from(self.one_time_ec.is_some())]);
        if let Some(key) = self.one_time_ec {
            hash.update(encode_ec(&key));
        }
        hash.finalize().into()
    }

    fn verify(&self, expected_identity: &[u8; 32]) -> Result<(), Error> {
        if self.identity != *expected_identity {
            return Err(Error::Authentication);
        }
        crate::kem::parse_public(&self.kem)?;
        validate_public(&self.signed_ec)?;
        if let Some(key) = self.one_time_ec {
            validate_public(&key)?;
        }
        verify_signature(
            &self.identity,
            &encode_ec(&self.signed_ec),
            &self.ec_signature,
        )?;
        verify_signature(&self.identity, &encode_kem(&self.kem), &self.kem_signature)
    }
}

impl Receiver {
    /// Experimental handoff into the Triple Ratchet. The responder must receive
    /// the initiator's first ratchet packet before sending a reply. In-memory only.
    pub fn accept_session(
        &mut self,
        identity: &IdentityKey,
        expected_sender: &[u8; 32],
        initial: &InitialMessage,
    ) -> Result<(crate::triple::Session, Vec<u8>), Error> {
        if initial.profile != 2 {
            return Err(Error::Encoding);
        }
        let (secret, plaintext) = self.accept_inner(identity, expected_sender, initial)?;
        let local = DhKey(x25519_dalek::StaticSecret::from(
            self.signed_ec.0.to_bytes(),
        ));
        let context = Sha256::digest(initial.to_bytes()).into();
        let session = crate::triple::Session::responder(secret, local, context)?;
        self.one_time.take();
        Ok((session, plaintext))
    }

    pub fn generate(identity: &IdentityKey, include_ec_one_time: bool) -> Result<Self, Error> {
        let signed_ec = DhKey::generate()?;
        let kem = KemKey::generate()?;
        let one_time_ec = include_ec_one_time.then(DhKey::generate).transpose()?;
        let ec_public = signed_ec.public_key();
        let kem_public = kem.public_key();
        let bundle = Bundle {
            identity: identity.public_key(),
            signed_ec: ec_public,
            ec_signature: identity.sign(&encode_ec(&ec_public))?,
            kem_signature: identity.sign(&encode_kem(&kem_public))?,
            kem: kem_public,
            one_time_ec: one_time_ec.as_ref().map(DhKey::public_key),
        };
        Ok(Self {
            signed_ec,
            one_time: Some((kem, one_time_ec)),
            bundle,
        })
    }

    pub fn bundle(&self) -> Result<&Bundle, Error> {
        self.one_time.as_ref().ok_or(Error::InvalidKey)?;
        Ok(&self.bundle)
    }

    /// The caller must independently establish the expected peer identity.
    /// No plaintext, secret, or consumed-key mutation is returned on authentication failure.
    pub fn accept(
        &mut self,
        identity: &IdentityKey,
        expected_sender: &[u8; 32],
        initial: &InitialMessage,
    ) -> Result<(Secret32, Vec<u8>), Error> {
        let result = self.accept_inner(identity, expected_sender, initial)?;
        self.one_time.take();
        Ok(result)
    }

    fn accept_inner(
        &self,
        identity: &IdentityKey,
        expected_sender: &[u8; 32],
        initial: &InitialMessage,
    ) -> Result<(Secret32, Vec<u8>), Error> {
        if initial.ciphertext.len() < 16 || initial.ciphertext.len() > MAX_PLAINTEXT + 16 {
            return Err(Error::Limit);
        }
        if initial.identity != *expected_sender
            || identity.public_key() != self.bundle.identity
            || initial.bundle_id != self.bundle.id()
        {
            return Err(Error::Authentication);
        }
        validate_public(&initial.identity)?;
        validate_public(&initial.ephemeral)?;
        let (kem, one_time_ec) = self.one_time.as_ref().ok_or(Error::InvalidKey)?;
        let ss = kem.decapsulate(&initial.kem_ciphertext)?;
        let secret = combine(
            self.signed_ec.exchange(&initial.identity)?,
            identity.exchange(&initial.ephemeral)?,
            self.signed_ec.exchange(&initial.ephemeral)?,
            one_time_ec
                .as_ref()
                .map(|key| key.exchange(&initial.ephemeral))
                .transpose()?,
            ss,
        )?;
        let plaintext = initial_key(&secret, initial.profile)?
            .open(&initial.ciphertext, &associated_data(initial, &self.bundle))?;
        Ok((secret, plaintext))
    }
}

/// Starts one experimental handshake using an independently selected peer identity.
pub fn initiate(
    identity: &IdentityKey,
    expected_recipient: &[u8; 32],
    bundle: &Bundle,
    plaintext: &[u8],
) -> Result<(Secret32, InitialMessage), Error> {
    initiate_profile(identity, expected_recipient, bundle, plaintext, 1)
}

fn initiate_profile(
    identity: &IdentityKey,
    expected_recipient: &[u8; 32],
    bundle: &Bundle,
    plaintext: &[u8],
    profile: u8,
) -> Result<(Secret32, InitialMessage), Error> {
    if plaintext.len() > MAX_PLAINTEXT {
        return Err(Error::Limit);
    }
    bundle.verify(expected_recipient)?;
    let ephemeral = DhKey::generate()?;
    let (kem_ciphertext, ss) = encapsulate(&bundle.kem)?;
    let secret = combine(
        identity.exchange(&bundle.signed_ec)?,
        ephemeral.exchange(&bundle.identity)?,
        ephemeral.exchange(&bundle.signed_ec)?,
        bundle
            .one_time_ec
            .map(|key| ephemeral.exchange(&key))
            .transpose()?,
        ss,
    )?;
    let mut initial = InitialMessage {
        profile,
        identity: identity.public_key(),
        ephemeral: ephemeral.public_key(),
        bundle_id: bundle.id(),
        kem_ciphertext,
        ciphertext: Vec::new(),
    };
    initial.ciphertext =
        initial_key(&secret, profile)?.seal(plaintext, &associated_data(&initial, bundle))?;
    Ok((secret, initial))
}

/// Initiates PQXDH profile 2 and binds the Triple Ratchet to the authenticated
/// initial transcript. No classical fallback. Persistence is the caller's job.
pub fn initiate_session(
    identity: &IdentityKey,
    expected_recipient: &[u8; 32],
    bundle: &Bundle,
    plaintext: &[u8],
) -> Result<(crate::triple::Session, InitialMessage), Error> {
    let (secret, initial) = initiate_profile(identity, expected_recipient, bundle, plaintext, 2)?;
    let context = Sha256::digest(initial.to_bytes()).into();
    let session = crate::triple::Session::initiator(secret, bundle.signed_ec, context)?;
    Ok((session, initial))
}

fn combine(
    dh1: Secret32,
    dh2: Secret32,
    dh3: Secret32,
    dh4: Option<Secret32>,
    ss: Secret32,
) -> Result<Secret32, Error> {
    let mut input = Zeroizing::new(Vec::with_capacity(192));
    input.extend_from_slice(&[0xff; 32]);
    for secret in [&dh1, &dh2, &dh3]
        .into_iter()
        .chain(dh4.as_ref())
        .chain([&ss])
    {
        input.extend_from_slice(secret.0.as_ref());
    }
    let mut output = Zeroizing::new([0; 32]);
    Hkdf::<Sha256>::new(Some(&[0; 32]), &input)
        .expand(CONTEXT, output.as_mut())
        .map_err(|_| Error::Limit)?;
    Ok(Secret32(output))
}

fn initial_key(secret: &Secret32, profile: u8) -> Result<MessageKey, Error> {
    let info = match profile {
        1 => b"Sigil/experimental/pqxdh/initial/v0".as_slice(),
        2 => TR_INITIAL,
        _ => return Err(Error::Encoding),
    };
    let mut output = Zeroizing::new([0; 32]);
    Hkdf::<Sha256>::new(None, secret.0.as_ref())
        .expand(info, output.as_mut())
        .map_err(|_| Error::Limit)?;
    Ok(MessageKey(Secret32(output)))
}

fn associated_data(initial: &InitialMessage, bundle: &Bundle) -> Vec<u8> {
    let mut aad = encode_ec(&initial.identity);
    aad.extend_from_slice(&encode_ec(&bundle.identity));
    aad.extend_from_slice(&encode_kem(&bundle.kem));
    aad.extend_from_slice(CONTEXT);
    aad.extend_from_slice(&initial.bundle_id);
    aad.extend_from_slice(&encode_ec(&initial.ephemeral));
    aad.extend_from_slice(&initial.kem_ciphertext);
    if initial.profile == 2 {
        aad.extend_from_slice(TR_INITIAL);
    }
    aad
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratchet_profile_cannot_be_downgraded_or_upgraded_by_reframing() {
        let alice = IdentityKey::generate().unwrap();
        let bob = IdentityKey::generate().unwrap();
        let mut receiver = Receiver::generate(&bob, true).unwrap();
        let (_, initial) = initiate_session(
            &alice,
            &bob.public_key(),
            receiver.bundle().unwrap(),
            b"triple",
        )
        .unwrap();
        let mut downgraded = initial.to_bytes();
        assert_eq!(downgraded[5], 2);
        downgraded[5] = 1;
        let downgraded = InitialMessage::from_bytes(&downgraded).unwrap();
        assert!(receiver
            .accept_session(&bob, &alice.public_key(), &downgraded)
            .is_err());
        assert!(matches!(
            receiver.accept(&bob, &alice.public_key(), &downgraded),
            Err(Error::Authentication)
        ));
        assert!(receiver.bundle().is_ok());
        assert_eq!(
            receiver
                .accept_session(&bob, &alice.public_key(), &initial)
                .unwrap()
                .1,
            b"triple"
        );

        let mut receiver = Receiver::generate(&bob, false).unwrap();
        let (_, initial) = initiate(
            &alice,
            &bob.public_key(),
            receiver.bundle().unwrap(),
            b"legacy evaluation",
        )
        .unwrap();
        assert!(receiver
            .accept_session(&bob, &alice.public_key(), &initial)
            .is_err());
        let mut upgraded = initial.to_bytes();
        upgraded[5] = 2;
        let upgraded = InitialMessage::from_bytes(&upgraded).unwrap();
        assert!(matches!(
            receiver.accept_session(&bob, &alice.public_key(), &upgraded),
            Err(Error::Authentication)
        ));
        assert!(receiver.bundle().is_ok());
        assert_eq!(
            receiver
                .accept(&bob, &alice.public_key(), &initial)
                .unwrap()
                .1,
            b"legacy evaluation"
        );
    }

    #[test]
    fn handshake_handoff_binds_and_starts_ratchet() {
        let alice = IdentityKey::generate().unwrap();
        let bob = IdentityKey::generate().unwrap();
        let mut receiver = Receiver::generate(&bob, true).unwrap();
        let (mut alice_session, initial) = initiate_session(
            &alice,
            &bob.public_key(),
            receiver.bundle().unwrap(),
            b"hello",
        )
        .unwrap();
        let initial = InitialMessage::from_bytes(&initial.to_bytes()).unwrap();
        let (mut bob_session, hello) = receiver
            .accept_session(&bob, &alice.public_key(), &initial)
            .unwrap();
        assert_eq!(hello, b"hello");
        assert!(matches!(bob_session.send(b"too early"), Err(Error::State)));
        let first = alice_session.send(b"first ratchet packet").unwrap();
        assert_eq!(
            bob_session.receive(&first).unwrap(),
            b"first ratchet packet"
        );
        let reply = bob_session.send(b"reply").unwrap();
        assert_eq!(alice_session.receive(&reply).unwrap(), b"reply");
        assert!(receiver
            .accept_session(&bob, &alice.public_key(), &initial)
            .is_err());
    }

    #[test]
    fn pqxdh_kdf_matches_openssl() {
        let output = combine(
            Secret32::from_bytes([1; 32]),
            Secret32::from_bytes([2; 32]),
            Secret32::from_bytes([3; 32]),
            Some(Secret32::from_bytes([4; 32])),
            Secret32::from_bytes([5; 32]),
        )
        .unwrap();
        assert_eq!(
            *output.0,
            [
                0x57, 0x44, 0x24, 0xd9, 0xcd, 0x39, 0xc4, 0x17, 0xd8, 0x35, 0x2e, 0xfa, 0xc3, 0x45,
                0xdf, 0x3b, 0x2a, 0x85, 0xfc, 0x28, 0xa9, 0x03, 0x26, 0x44, 0x22, 0xb7, 0xa0, 0x94,
                0xa6, 0xfa, 0xf8, 0xa1
            ]
        );
    }

    #[test]
    fn handshake_agrees_and_consumes_one_time_keys() {
        for ec in [false, true] {
            let alice = IdentityKey::generate().unwrap();
            let bob = IdentityKey::generate().unwrap();
            let mut receiver = Receiver::generate(&bob, ec).unwrap();
            let (sent, initial) = initiate(
                &alice,
                &bob.public_key(),
                receiver.bundle().unwrap(),
                b"synthetic",
            )
            .unwrap();
            let (received, plaintext) = receiver
                .accept(&bob, &alice.public_key(), &initial)
                .unwrap();
            assert_eq!(sent.0, received.0);
            assert_eq!(plaintext, b"synthetic");
            assert!(receiver.bundle().is_err());
            assert!(receiver
                .accept(&bob, &alice.public_key(), &initial)
                .is_err());
        }
    }

    #[test]
    fn tampering_does_not_consume_prekeys() {
        let alice = IdentityKey::generate().unwrap();
        let bob = IdentityKey::generate().unwrap();
        let mut receiver = Receiver::generate(&bob, true).unwrap();
        let (_, mut initial) = initiate(
            &alice,
            &bob.public_key(),
            receiver.bundle().unwrap(),
            b"synthetic",
        )
        .unwrap();
        initial.kem_ciphertext[0] ^= 1;
        assert!(receiver
            .accept(&bob, &alice.public_key(), &initial)
            .is_err());
        initial.kem_ciphertext[0] ^= 1;
        initial.ciphertext[0] ^= 1;
        assert!(receiver
            .accept(&bob, &alice.public_key(), &initial)
            .is_err());
        initial.ciphertext[0] ^= 1;
        initial.bundle_id[0] ^= 1;
        assert!(receiver
            .accept(&bob, &alice.public_key(), &initial)
            .is_err());
        initial.bundle_id[0] ^= 1;
        assert!(receiver.accept(&bob, &bob.public_key(), &initial).is_err());
        assert!(receiver
            .accept(&alice, &alice.public_key(), &initial)
            .is_err());
        let original_ephemeral = initial.ephemeral;
        initial.ephemeral = DhKey::generate().unwrap().public_key();
        assert!(receiver
            .accept(&bob, &alice.public_key(), &initial)
            .is_err());
        initial.ephemeral = original_ephemeral;
        let byte = initial.kem_ciphertext.pop().unwrap();
        assert!(receiver
            .accept(&bob, &alice.public_key(), &initial)
            .is_err());
        initial.kem_ciphertext.push(byte);
        receiver
            .accept(&bob, &alice.public_key(), &initial)
            .unwrap();
    }

    #[test]
    fn reject_substituted_bundle_before_handshake() {
        let alice = IdentityKey::generate().unwrap();
        let bob = IdentityKey::generate().unwrap();
        let mut receiver = Receiver::generate(&bob, true).unwrap();
        assert!(initiate(&alice, &alice.public_key(), &receiver.bundle, b"").is_err());
        receiver.bundle.ec_signature[0] ^= 1;
        assert!(initiate(&alice, &bob.public_key(), &receiver.bundle, b"").is_err());
        receiver.bundle.ec_signature[0] ^= 1;
        receiver.bundle.kem_signature[0] ^= 1;
        assert!(initiate(&alice, &bob.public_key(), &receiver.bundle, b"").is_err());
        receiver.bundle.kem_signature[0] ^= 1;
        receiver.bundle.one_time_ec = Some([0; 32]);
        assert!(initiate(&alice, &bob.public_key(), &receiver.bundle, b"").is_err());
    }
}
