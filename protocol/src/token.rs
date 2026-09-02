//! Blind tokens: RFC 9474 RSABSSA-SHA384-PSSZERO-Deterministic over
//! RSA-2048. A token proves "this request was paid for" without saying by
//! whom: the issuer signs a blinded value and never sees the token it
//! later accepts.

use crate::encoding::{Reader, Writer};
use crate::kdf::kdf;
use blind_rsa_signatures::{
    BlindSignature, BlindingResult, Deterministic, KeyPair, PSSZero, PublicKey, SecretKey, Sha384,
    Signature,
};
use rand_core10::CryptoRng;

pub const MODULUS_BITS: usize = 2048;
pub const SIG_LEN: usize = MODULUS_BITS / 8;
const MSG_CTX: &[u8] = b"sigil v1 token";

type Pk = PublicKey<Sha384, PSSZero, Deterministic>;
type Sk = SecretKey<Sha384, PSSZero, Deterministic>;
type Kp = KeyPair<Sha384, PSSZero, Deterministic>;

/// `key_id = KDF("sigil v1 token key id", spki_der)`.
pub fn key_id(spki_der: &[u8]) -> [u8; 32] {
    kdf("sigil v1 token key id", spki_der)
}

/// The message that is blind-signed: binds the token to the issuing key.
pub fn message(key_id: &[u8; 32], nonce: &[u8; 32]) -> Vec<u8> {
    let mut m = MSG_CTX.to_vec();
    m.extend_from_slice(key_id);
    m.extend_from_slice(nonce);
    m
}

pub struct Issuer {
    sk: Sk,
    pub spki: Vec<u8>,
    pub key_id: [u8; 32],
}

impl Issuer {
    pub fn generate<R: CryptoRng + ?Sized>(rng: &mut R) -> Issuer {
        let kp = Kp::generate(rng, MODULUS_BITS).expect("rsa keygen");
        Self::from_secret(kp.sk)
    }
    /// PKCS#8 DER.
    pub fn from_der(secret_der: &[u8]) -> crate::Result<Issuer> {
        Ok(Self::from_secret(
            Sk::from_der(secret_der).map_err(|_| crate::Error::Malformed)?,
        ))
    }
    pub fn to_der(&self) -> Vec<u8> {
        self.sk.to_der().expect("der")
    }
    fn from_secret(sk: Sk) -> Issuer {
        let spki = sk.public_key().expect("pk").to_spki().expect("spki");
        let key_id = key_id(&spki);
        Issuer { sk, spki, key_id }
    }
    /// Server side of `token.issue`: sign one blinded message.
    pub fn sign(&self, blinded: &[u8]) -> crate::Result<Vec<u8>> {
        if blinded.len() != SIG_LEN {
            return Err(crate::Error::Length);
        }
        Ok(self
            .sk
            .blind_sign(blinded)
            .map_err(|_| crate::Error::Malformed)?
            .to_vec())
    }
}

/// Client-side state between blinding and finalising one token.
pub struct Pending {
    pub nonce: [u8; 32],
    pub blinded: Vec<u8>,
    /// The blinding inverse; needed to unblind, never sent.
    pub secret: Vec<u8>,
}

pub struct Verifier {
    pk: Pk,
    pub key_id: [u8; 32],
}

impl Verifier {
    pub fn from_spki(spki: &[u8]) -> crate::Result<Verifier> {
        let pk = Pk::from_spki(spki).map_err(|_| crate::Error::Malformed)?;
        Ok(Verifier {
            pk,
            key_id: key_id(spki),
        })
    }

    /// Client: blind a fresh nonce. The RNG supplies the blinding factor;
    /// with a deterministic salt mode nothing else is random.
    pub fn blind<R: CryptoRng + ?Sized>(
        &self,
        rng: &mut R,
        nonce: [u8; 32],
    ) -> crate::Result<Pending> {
        let r = self
            .pk
            .blind(rng, message(&self.key_id, &nonce))
            .map_err(|_| crate::Error::Malformed)?;
        Ok(Pending {
            nonce,
            blinded: r.blind_message.to_vec(),
            secret: r.secret.to_vec(),
        })
    }

    /// Client: unblind the issuer's signature into a spendable token.
    pub fn finalize(&self, p: &Pending, blind_sig: &[u8]) -> crate::Result<Token> {
        let result = BlindingResult {
            blind_message: p.blinded.clone().into(),
            secret: p.secret.clone().into(),
            msg_randomizer: None,
        };
        let sig = self
            .pk
            .finalize(
                &BlindSignature::from(blind_sig.to_vec()),
                &result,
                message(&self.key_id, &p.nonce),
            )
            .map_err(|_| crate::Error::Auth)?;
        Ok(Token {
            key_id: self.key_id,
            nonce: p.nonce,
            signature: sig.to_vec(),
        })
    }

    /// Server: verify a token's signature. Double-spend is the caller's job:
    /// remember `spend_id` per key id and reject repeats.
    pub fn verify(&self, t: &Token) -> crate::Result<()> {
        if t.key_id != self.key_id {
            return Err(crate::Error::Auth);
        }
        self.pk
            .verify(
                &Signature::from(t.signature.clone()),
                None,
                message(&self.key_id, &t.nonce),
            )
            .map_err(|_| crate::Error::Auth)
    }
}

/// Wire form: `version u8 ‖ key_id[32] ‖ nonce[32] ‖ signature[256]` = 321 bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub key_id: [u8; 32],
    pub nonce: [u8; 32],
    pub signature: Vec<u8>,
}

impl Token {
    pub const LEN: usize = 1 + 32 + 32 + SIG_LEN;
    pub fn encode(&self) -> Vec<u8> {
        Writer::new()
            .u8(crate::VERSION)
            .fixed(&self.key_id)
            .fixed(&self.nonce)
            .fixed(&self.signature)
            .finish()
    }
    pub fn decode(b: &[u8]) -> crate::Result<Token> {
        if b.len() != Self::LEN {
            return Err(crate::Error::Length);
        }
        let mut r = Reader::new(b);
        if r.u8()? != crate::VERSION {
            return Err(crate::Error::Malformed);
        }
        let key_id = r.fixed()?;
        let nonce = r.fixed()?;
        let signature = r.fixed::<SIG_LEN>()?.to_vec();
        r.done()?;
        Ok(Token {
            key_id,
            nonce,
            signature,
        })
    }
    /// `H(nonce)`: what the server stores to detect a second spend.
    pub fn spend_id(&self) -> [u8; 32] {
        crate::kdf::hash(&self.nonce)
    }
}
