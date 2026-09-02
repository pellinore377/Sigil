//! Events and envelopes. An event is what a client sends (a message, a
//! reaction, a receipt, a membership change: all the same shape). An
//! envelope is the sealed, padded form the server stores. Every envelope is
//! exactly 1024, 4096 or 16384 bytes.

use crate::encoding::{Reader, Writer};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

pub const NONCE_LEN: usize = 24;
pub const TAG_LEN: usize = 16;
/// Total envelope sizes on the wire.
pub const BUCKETS: [usize; 3] = [1024, 4096, 16384];
const OVERHEAD: usize = NONCE_LEN + TAG_LEN;
const AD_CTX: &[u8] = b"sigil v1 envelope";

/// Event kinds. The inner payload of `body` is defined per kind by the
/// application layer; the protocol only fixes the framing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum Kind {
    Text = 1,
    Reaction = 2,
    Edit = 3,
    Redact = 4,
    Receipt = 5,
    Typing = 6,
    Membership = 7,
    Policy = 8,
    Media = 9,
    Call = 10,
    Welcome = 11,
    Commit = 12,
    Proposal = 13,
    Link = 14,
    /// A poll: `{question, options[{id,text}], closed, max}`.
    Poll = 15,
    /// A vote on a poll (reference = the poll's event id): `{ids[]}`.
    Vote = 16,
    /// The poll's author ends it (reference = the poll's event id).
    PollEnd = 17,
    /// A place: `{lat, lon, description, self, until?, end?}`.
    Location = 18,
}

impl TryFrom<u16> for Kind {
    type Error = ();
    fn try_from(k: u16) -> Result<Kind, ()> {
        Ok(match k {
            1 => Kind::Text,
            2 => Kind::Reaction,
            3 => Kind::Edit,
            4 => Kind::Redact,
            5 => Kind::Receipt,
            6 => Kind::Typing,
            7 => Kind::Membership,
            8 => Kind::Policy,
            9 => Kind::Media,
            10 => Kind::Call,
            11 => Kind::Welcome,
            12 => Kind::Commit,
            13 => Kind::Proposal,
            14 => Kind::Link,
            15 => Kind::Poll,
            16 => Kind::Vote,
            17 => Kind::PollEnd,
            18 => Kind::Location,
            _ => return Err(()),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub kind: u16,
    /// Sender's clock, milliseconds since the Unix epoch. Display only;
    /// ordering is by the slot's sequence number.
    pub ts_ms: u64,
    /// What this refers to (an event id for a reaction, edit, receipt), or empty.
    pub reference: Vec<u8>,
    pub body: Vec<u8>,
}

impl Event {
    pub fn encode(&self) -> Vec<u8> {
        Writer::new()
            .u8(crate::VERSION)
            .u16(self.kind)
            .u64(self.ts_ms)
            .bytes(&self.reference)
            .bytes(&self.body)
            .finish()
    }
    pub fn decode(b: &[u8]) -> crate::Result<Event> {
        let mut r = Reader::new(b);
        if r.u8()? != crate::VERSION {
            return Err(crate::Error::Malformed);
        }
        let kind = r.u16()?;
        let ts_ms = r.u64()?;
        let reference = r.bytes()?.to_vec();
        let body = r.bytes()?.to_vec();
        r.done()?;
        Ok(Event {
            kind,
            ts_ms,
            reference,
            body,
        })
    }
}

/// Pad to the smallest bucket: append `0x80`, then zeros. Returns the
/// padded plaintext whose length is `bucket - OVERHEAD`.
pub fn pad(plain: &[u8]) -> crate::Result<Vec<u8>> {
    let need = plain.len() + 1;
    let bucket = BUCKETS
        .iter()
        .copied()
        .find(|b| need <= b - OVERHEAD)
        .ok_or(crate::Error::TooLarge)?;
    let mut out = Vec::with_capacity(bucket - OVERHEAD);
    out.extend_from_slice(plain);
    out.push(0x80);
    out.resize(bucket - OVERHEAD, 0);
    Ok(out)
}

pub fn unpad(padded: &[u8]) -> crate::Result<&[u8]> {
    let mut i = padded.len();
    while i > 0 && padded[i - 1] == 0 {
        i -= 1;
    }
    if i == 0 || padded[i - 1] != 0x80 {
        return Err(crate::Error::Padding);
    }
    Ok(&padded[..i - 1])
}

/// `envelope = nonce ‖ XChaCha20-Poly1305(envelope_key, nonce, ad = "sigil v1 envelope" ‖ address, pad(plain))`.
/// `plain` is the group layer's message (in v1, an MLS message) and is
/// opaque here. The nonce is 24 random bytes; a fixed one appears only in vectors.
pub fn seal(
    envelope_key: &[u8; 32],
    address: &[u8; 32],
    nonce: &[u8; NONCE_LEN],
    plain: &[u8],
) -> crate::Result<Vec<u8>> {
    let padded = pad(plain)?;
    let mut ad = AD_CTX.to_vec();
    ad.extend_from_slice(address);
    let ct = XChaCha20Poly1305::new(envelope_key.into())
        .encrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: &padded,
                aad: &ad,
            },
        )
        .map_err(|_| crate::Error::Auth)?;
    let mut out = nonce.to_vec();
    out.extend_from_slice(&ct);
    debug_assert!(BUCKETS.contains(&out.len()));
    Ok(out)
}

pub fn open(
    envelope_key: &[u8; 32],
    address: &[u8; 32],
    envelope: &[u8],
) -> crate::Result<Vec<u8>> {
    if !BUCKETS.contains(&envelope.len()) {
        return Err(crate::Error::Length);
    }
    let (nonce, ct) = envelope.split_at(NONCE_LEN);
    let mut ad = AD_CTX.to_vec();
    ad.extend_from_slice(address);
    let padded = XChaCha20Poly1305::new(envelope_key.into())
        .decrypt(XNonce::from_slice(nonce), Payload { msg: ct, aad: &ad })
        .map_err(|_| crate::Error::Auth)?;
    Ok(unpad(&padded)?.to_vec())
}
