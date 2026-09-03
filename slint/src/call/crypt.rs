//! Frame encryption for calls: every Opus frame is sealed under a key
//! derived from the conversation's MLS epoch before it leaves the device,
//! so the forwarding unit relays bytes it cannot decode. This is the last
//! piece of the server-blind design (plan 5.5 step 6), done simply: a key
//! id, a counter, and XChaCha20-Poly1305 with the sender's peer id in the
//! nonce so two senders never share one.
//!
//! Frame: `kid u8 ‖ counter u32be ‖ ciphertext‖tag`. Nonce (24 bytes):
//! `kid ‖ sender peer id (16) ‖ counter (4) ‖ 0×3`. The five-byte header is
//! authenticated as associated data.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

pub struct FrameCrypt {
    keys: Vec<(u8, XChaCha20Poly1305)>,
    me: [u8; 16],
    counter: u32,
    current: Option<u8>,
}

impl FrameCrypt {
    pub fn new(my_peer: [u8; 16]) -> FrameCrypt {
        FrameCrypt {
            keys: Vec::new(),
            me: my_peer,
            counter: 0,
            current: None,
        }
    }

    /// A key for an epoch; the newest becomes the one frames are sealed under.
    pub fn add_key(&mut self, kid: u8, key: &[u8; 32]) {
        self.keys.retain(|(k, _)| *k != kid);
        self.keys.push((kid, XChaCha20Poly1305::new(key.into())));
        if self.keys.len() > 4 {
            self.keys.remove(0);
        }
        self.current = Some(kid);
    }

    pub fn has_key(&self, kid: u8) -> bool {
        self.keys.iter().any(|(k, _)| *k == kid)
    }

    pub fn ready(&self) -> bool {
        self.current.is_some()
    }

    fn nonce(kid: u8, sender: &[u8; 16], counter: u32) -> XNonce {
        let mut n = [0u8; 24];
        n[0] = kid;
        n[1..17].copy_from_slice(sender);
        n[17..21].copy_from_slice(&counter.to_be_bytes());
        XNonce::from(n)
    }

    /// Seal one frame from us. None until a key has arrived.
    pub fn seal(&mut self, payload: &[u8]) -> Option<Vec<u8>> {
        let kid = self.current?;
        let (_, cipher) = self.keys.iter().find(|(k, _)| *k == kid)?;
        let counter = self.counter;
        self.counter = self.counter.wrapping_add(1);
        let mut out = Vec::with_capacity(payload.len() + 21);
        out.push(kid);
        out.extend_from_slice(&counter.to_be_bytes());
        let ct = cipher
            .encrypt(
                &Self::nonce(kid, &self.me, counter),
                Payload {
                    msg: payload,
                    aad: &out[..5],
                },
            )
            .ok()?;
        out.extend_from_slice(&ct);
        Some(out)
    }

    /// Open one frame from `sender`. Err carries the key id we lack, so the
    /// caller can ask for it; None for a frame that is simply bad.
    pub fn open(&self, sender: &[u8; 16], frame: &[u8]) -> Result<Option<Vec<u8>>, u8> {
        if frame.len() < 5 + 16 {
            return Ok(None);
        }
        let kid = frame[0];
        let counter = u32::from_be_bytes([frame[1], frame[2], frame[3], frame[4]]);
        let Some((_, cipher)) = self.keys.iter().find(|(k, _)| *k == kid) else {
            return Err(kid);
        };
        Ok(cipher
            .decrypt(
                &Self::nonce(kid, sender, counter),
                Payload {
                    msg: &frame[5..],
                    aad: &frame[..5],
                },
            )
            .ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_and_the_wrong_sender_or_key_fails() {
        let key = [7u8; 32];
        let mut a = FrameCrypt::new([1u8; 16]);
        a.add_key(3, &key);
        let mut b = FrameCrypt::new([2u8; 16]);
        b.add_key(3, &key);
        let sealed = a.seal(b"opus bytes").unwrap();
        assert_eq!(b.open(&[1u8; 16], &sealed).unwrap().unwrap(), b"opus bytes");
        assert!(
            b.open(&[9u8; 16], &sealed).unwrap().is_none(),
            "another sender's id"
        );
        let mut c = FrameCrypt::new([3u8; 16]);
        c.add_key(4, &key);
        assert_eq!(
            c.open(&[1u8; 16], &sealed).unwrap_err(),
            3,
            "asks for the key it lacks"
        );
        let s2 = a.seal(b"opus bytes").unwrap();
        assert_ne!(sealed, s2, "the counter moves");
    }
}
