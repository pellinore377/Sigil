//! Experimental classical Double Ratchet, pinned specification revision 4 section 3.
//! The engine is in memory; sigil-client supplies durable transactions.
use crate::skipped::Skipped;
use crate::{
    derive_chain, derive_root, identity::validate_public, DhKey, Error, MessageKey, Secret32,
    MAX_PLAINTEXT,
};
mod checkpoint;

pub const MAX_SKIPPED_KEYS: usize = crate::skipped::MAX;
const HEADER_LEN: usize = 48;
const PREFIX: &[u8; 8] = b"SGDR\0\x01\0\0";

pub struct Packet {
    pub(crate) dh: [u8; 32],
    pub(crate) previous: u32,
    pub(crate) number: u32,
    pub(crate) ciphertext: Vec<u8>,
}

impl Packet {
    fn header(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(HEADER_LEN);
        bytes.extend_from_slice(PREFIX);
        bytes.extend_from_slice(&self.dh);
        bytes.extend_from_slice(&self.previous.to_be_bytes());
        bytes.extend_from_slice(&self.number.to_be_bytes());
        bytes
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = self.header();
        bytes.extend_from_slice(&self.ciphertext);
        bytes
    }

    /// Framing/key checks only; Session::receive authenticates the packet.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        if !(HEADER_LEN + 16..=HEADER_LEN + MAX_PLAINTEXT + 16).contains(&bytes.len()) {
            return Err(Error::Limit);
        }
        if &bytes[..8] != PREFIX {
            return Err(Error::Encoding);
        }
        let dh = bytes[8..40].try_into().map_err(|_| Error::Encoding)?;
        validate_public(&dh)?;
        Ok(Self {
            dh,
            previous: u32::from_be_bytes(bytes[40..44].try_into().map_err(|_| Error::Encoding)?),
            number: u32::from_be_bytes(bytes[44..48].try_into().map_err(|_| Error::Encoding)?),
            ciphertext: bytes[48..].to_vec(),
        })
    }
}

struct Step {
    local: DhKey,
    remote: Option<[u8; 32]>,
    root: Secret32,
    send: Option<Secret32>,
    receive: Option<Secret32>,
    sent: u32,
    received: u32,
    previous: u32,
}

impl Step {
    // Private transactional candidate. No public Clone/serialization of secrets.
    fn candidate(&self) -> Self {
        fn copy(key: &Secret32) -> Secret32 {
            Secret32::from_bytes(*key.0)
        }
        Self {
            local: DhKey(x25519_dalek::StaticSecret::from(self.local.0.to_bytes())),
            remote: self.remote,
            root: copy(&self.root),
            send: self.send.as_ref().map(copy),
            receive: self.receive.as_ref().map(copy),
            sent: self.sent,
            received: self.received,
            previous: self.previous,
        }
    }

    fn skip(
        &mut self,
        until: u32,
        added: &mut Skipped<([u8; 32], u32)>,
        work: &mut usize,
    ) -> Result<(), Error> {
        let gap = until.checked_sub(self.received).ok_or(Error::Replay)? as usize;
        if gap > MAX_SKIPPED_KEYS || *work + gap > MAX_SKIPPED_KEYS {
            return Err(Error::Limit);
        }
        *work += gap;
        if gap == 0 {
            return Ok(());
        }
        let remote = self.remote.ok_or(Error::State)?;
        for _ in 0..gap {
            let (chain, key) = derive_chain(self.receive.as_ref().ok_or(Error::State)?)?;
            added.insert((remote, self.received), key);
            self.receive = Some(chain);
            self.received += 1;
        }
        Ok(())
    }

    fn turn(&mut self, remote: [u8; 32]) -> Result<(), Error> {
        let (root, receive) = derive_root(&self.root, &self.local.exchange(&remote)?)?;
        let local = DhKey::generate()?;
        let (root, send) = derive_root(&root, &local.exchange(&remote)?)?;
        self.previous = self.sent;
        self.sent = 0;
        self.received = 0;
        self.remote = Some(remote);
        self.local = local;
        self.root = root;
        self.receive = Some(receive);
        self.send = Some(send);
        Ok(())
    }
}

pub struct Session {
    step: Step,
    skipped: Skipped<([u8; 32], u32)>,
    context: [u8; 32],
}

impl Session {
    pub(crate) fn has_received(&self) -> bool {
        self.step.receive.is_some()
    }
    pub(crate) fn candidate(&self) -> Self {
        Self {
            step: self.step.candidate(),
            skipped: self.skipped.candidate(),
            context: self.context,
        }
    }
    /// Shared secret, responder DH key and context must come from an authenticated
    /// handshake. Context must bind both identities and the handshake instance.
    pub fn initiator(
        secret: Secret32,
        responder: [u8; 32],
        context: [u8; 32],
    ) -> Result<Self, Error> {
        validate_public(&responder)?;
        let local = DhKey::generate()?;
        let (root, send) = derive_root(&secret, &local.exchange(&responder)?)?;
        Ok(Self {
            step: Step {
                local,
                remote: Some(responder),
                root,
                send: Some(send),
                receive: None,
                sent: 0,
                received: 0,
                previous: 0,
            },
            skipped: Skipped::new(),
            context,
        })
    }

    /// The responder cannot send until it authenticates the initiator's first packet.
    pub fn responder(secret: Secret32, local: DhKey, context: [u8; 32]) -> Self {
        Self {
            step: Step {
                local,
                remote: None,
                root: secret,
                send: None,
                receive: None,
                sent: 0,
                received: 0,
                previous: 0,
            },
            skipped: Skipped::new(),
            context,
        }
    }

    fn aad(&self, packet: &Packet) -> Vec<u8> {
        let mut aad = b"Sigil/experimental/double-ratchet/v0".to_vec();
        aad.extend_from_slice(&self.context);
        aad.extend_from_slice(&packet.header());
        aad
    }

    /// Commits only in memory. Persistence adapters must atomically save advanced
    /// state and an outbox packet before allowing network transmission.
    pub fn send(&mut self, plaintext: &[u8]) -> Result<Packet, Error> {
        if plaintext.len() > MAX_PLAINTEXT {
            return Err(Error::Limit);
        }
        let mut candidate = self.candidate();
        let (mut packet, key) = candidate.send_key()?;
        packet.ciphertext = key.seal(plaintext, &self.aad(&packet))?;
        *self = candidate;
        Ok(packet)
    }

    // These key-only operations run on a private candidate. The caller must
    // authenticate the complete classical/hybrid header before committing it.
    pub(crate) fn send_key(&mut self) -> Result<(Packet, MessageKey), Error> {
        let next = self.step.sent.checked_add(1).ok_or(Error::Limit)?;
        let (chain, key) = derive_chain(self.step.send.as_ref().ok_or(Error::State)?)?;
        let packet = Packet {
            dh: self.step.local.public_key(),
            previous: self.step.previous,
            number: self.step.sent,
            ciphertext: Vec::new(),
        };
        self.step.send = Some(chain);
        self.step.sent = next;
        Ok((packet, key))
    }

    /// Authentication failure discards the candidate, preserving chains, counters,
    /// DH keys and skipped keys. Success commits in memory before returning plaintext.
    pub fn receive(&mut self, packet: &Packet) -> Result<Vec<u8>, Error> {
        if !(16..=MAX_PLAINTEXT + 16).contains(&packet.ciphertext.len()) {
            return Err(Error::Limit);
        }
        let aad = self.aad(packet);
        let mut candidate = self.candidate();
        let key = candidate.receive_key(packet)?;
        let plaintext = key.open(&packet.ciphertext, &aad)?;
        *self = candidate;
        Ok(plaintext)
    }

    pub(crate) fn receive_key(&mut self, packet: &Packet) -> Result<MessageKey, Error> {
        let id = (packet.dh, packet.number);
        if let Some(key) = self.skipped.remove(&id) {
            return Ok(key);
        }
        let next = packet.number.checked_add(1).ok_or(Error::Limit)?;
        validate_public(&packet.dh)?;
        let mut candidate = self.step.candidate();
        let mut work = 0;
        if candidate.remote != Some(packet.dh) {
            // There is no old receiving chain at initialization.
            if candidate.receive.is_some() {
                candidate.skip(packet.previous, &mut self.skipped, &mut work)?;
            } else if packet.previous != 0 {
                return Err(Error::State);
            }
            candidate.turn(packet.dh)?;
        }
        candidate.skip(packet.number, &mut self.skipped, &mut work)?;
        let (chain, key) = derive_chain(candidate.receive.as_ref().ok_or(Error::State)?)?;
        candidate.receive = Some(chain);
        candidate.received = next;
        self.step = candidate;
        Ok(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn pair() -> (Session, Session) {
        let key = DhKey::generate().unwrap();
        let alice =
            Session::initiator(Secret32::from_bytes([7; 32]), key.public_key(), [8; 32]).unwrap();
        (
            alice,
            Session::responder(Secret32::from_bytes([7; 32]), key, [8; 32]),
        )
    }

    #[test]
    fn alternating_turns_and_encoded_packets() {
        let (mut alice, mut bob) = pair();
        assert!(matches!(bob.send(b"early"), Err(Error::State)));
        for _ in 0..8 {
            let packet = Packet::from_bytes(&alice.send(b"alice").unwrap().to_bytes()).unwrap();
            assert_eq!(bob.receive(&packet).unwrap(), b"alice");
            assert!(bob.receive(&packet).is_err());
            let reply = bob.send(b"bob").unwrap();
            assert_eq!(alice.receive(&reply).unwrap(), b"bob");
        }
    }

    #[test]
    fn delayed_messages_survive_dh_turns() {
        let (mut alice, mut bob) = pair();
        let first = alice.send(b"first").unwrap();
        let delayed = alice.send(b"delayed old chain").unwrap();
        bob.receive(&first).unwrap();
        alice.receive(&bob.send(b"reply").unwrap()).unwrap();
        let new = alice.send(b"new chain").unwrap();
        assert_eq!(bob.receive(&new).unwrap(), b"new chain");
        assert_eq!(bob.skipped.len(), 1);
        assert_eq!(bob.receive(&delayed).unwrap(), b"delayed old chain");
        assert!(bob.skipped.is_empty());
        assert!(bob.receive(&delayed).is_err());
    }

    #[test]
    fn failed_dh_turn_preserves_existing_send_chain() {
        let (mut alice, mut bob) = pair();
        bob.receive(&alice.send(b"first").unwrap()).unwrap();
        let mut reply = bob.send(b"reply").unwrap();
        reply.ciphertext[0] ^= 1;
        assert!(alice.receive(&reply).is_err());
        let old = alice.send(b"old chain still usable").unwrap();
        assert_eq!(bob.receive(&old).unwrap(), b"old chain still usable");
        reply.ciphertext[0] ^= 1;
        alice.receive(&reply).unwrap();
        let new = alice.send(b"new chain usable").unwrap();
        assert_ne!(new.dh, old.dh);
        assert_eq!(bob.receive(&new).unwrap(), b"new chain usable");
    }

    #[test]
    fn bounded_skipped_retention_evicts_only_after_authenticated_receive() {
        let (mut alice, mut bob) = pair();
        let oldest = alice.send(b"oldest").unwrap();
        for _ in 1..MAX_SKIPPED_KEYS {
            alice.send(b"delayed").unwrap();
        }
        bob.receive(&alice.send(b"end of old chain").unwrap())
            .unwrap();
        assert_eq!(bob.skipped.len(), MAX_SKIPPED_KEYS);
        alice.receive(&bob.send(b"reply").unwrap()).unwrap();
        alice.send(b"new delayed").unwrap();
        let newest = alice.send(b"newest").unwrap();
        let mut corrupted = Packet::from_bytes(&newest.to_bytes()).unwrap();
        corrupted.ciphertext[0] ^= 1;
        assert!(bob.receive(&corrupted).is_err());
        assert_eq!(bob.candidate().receive(&oldest).unwrap(), b"oldest");
        assert_eq!(bob.receive(&newest).unwrap(), b"newest");
        assert_eq!(bob.skipped.len(), MAX_SKIPPED_KEYS);
        assert!(bob.receive(&oldest).is_err());
    }

    #[test]
    fn eviction_does_not_expand_the_combined_per_packet_kdf_work_budget() {
        let (mut alice, mut bob) = pair();
        bob.receive(&alice.send(b"start").unwrap()).unwrap();
        let mut old = Vec::new();
        for _ in 0..100 {
            old.push(alice.send(b"old delayed").unwrap());
        }
        alice.receive(&bob.send(b"reply").unwrap()).unwrap();
        let mut new = Vec::new();
        for _ in 0..100 {
            new.push(alice.send(b"new delayed").unwrap());
        }
        let target = alice.send(b"target").unwrap();
        assert_eq!(bob.receive(&target), Err(Error::Limit));
        assert!(bob.skipped.is_empty());
        bob.receive(old.last().unwrap()).unwrap();
        assert_eq!(bob.receive(&target).unwrap(), b"target");
        assert_eq!(bob.skipped.len(), MAX_SKIPPED_KEYS);
        for packet in new.iter().rev() {
            assert_eq!(bob.receive(packet).unwrap(), b"new delayed");
        }
    }

    #[test]
    fn failed_authentication_preserves_every_receive_path() {
        let (mut alice, mut bob) = pair();
        let mut first = alice.send(b"first").unwrap();
        first.ciphertext[0] ^= 1;
        assert!(bob.receive(&first).is_err());
        assert!(matches!(bob.send(b"still early"), Err(Error::State)));
        first.ciphertext[0] ^= 1;
        bob.receive(&first).unwrap();
        let mut delayed = alice.send(b"delayed").unwrap();
        let mut latest = alice.send(b"latest").unwrap();
        latest.ciphertext[0] ^= 1;
        assert!(bob.receive(&latest).is_err());
        assert!(bob.skipped.is_empty());
        latest.ciphertext[0] ^= 1;
        bob.receive(&latest).unwrap();
        delayed.ciphertext[0] ^= 1;
        assert!(bob.receive(&delayed).is_err());
        assert_eq!(bob.skipped.len(), 1);
        delayed.ciphertext[0] ^= 1;
        bob.receive(&delayed).unwrap();
        assert!(bob.skipped.is_empty());
    }

    #[test]
    fn skipped_key_budget_and_counter_overflow_fail_without_advancing() {
        let (mut alice, mut bob) = pair();
        let mut packets = Vec::new();
        for _ in 0..MAX_SKIPPED_KEYS + 2 {
            packets.push(alice.send(b"synthetic").unwrap());
        }
        assert_eq!(
            bob.receive(&packets[MAX_SKIPPED_KEYS + 1]),
            Err(Error::Limit)
        );
        bob.receive(&packets[MAX_SKIPPED_KEYS]).unwrap();
        assert_eq!(bob.skipped.len(), MAX_SKIPPED_KEYS);
        bob.receive(&packets[0]).unwrap();
        bob.receive(&packets[MAX_SKIPPED_KEYS + 1]).unwrap();
        alice.step.sent = u32::MAX;
        assert!(matches!(alice.send(b"overflow"), Err(Error::Limit)));
        assert_eq!(alice.step.sent, u32::MAX);
    }

    #[test]
    fn context_headers_and_limits_are_authenticated() {
        let (mut alice, mut bob) = pair();
        assert!(matches!(
            alice.send(&vec![0; MAX_PLAINTEXT + 1]),
            Err(Error::Limit)
        ));
        let packet = alice.send(b"synthetic").unwrap();
        assert_eq!(packet.number, 0);
        bob.context[0] ^= 1;
        assert!(bob.receive(&packet).is_err());
        bob.context[0] ^= 1;
        let encoded = packet.to_bytes();
        for index in [0, 4, 5, 6, 7, 40, 44] {
            let mut changed = encoded.clone();
            changed[index] ^= 1;
            if let Ok(changed) = Packet::from_bytes(&changed) {
                assert!(bob.receive(&changed).is_err());
            }
        }
        assert!(Packet::from_bytes(&encoded[..HEADER_LEN]).is_err());
        bob.receive(&packet).unwrap();
    }
}
