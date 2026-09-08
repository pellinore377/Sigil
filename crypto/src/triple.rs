//! Experimental Triple Ratchet (Double Ratchet rev. 4 §6).
//! This in-memory engine is not an enabled messaging suite or a persistence API.
use crate::{
    identity::validate_public, ratchet, spqr, DhKey, Error, MessageKey, Secret32, MAX_PLAINTEXT,
};
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroizing;
mod checkpoint;

const PREFIX: &[u8; 8] = b"SGTR\0\x01\0\0";
const INFO: &[u8] =
    b"Sigil/experimental/triple-ratchet/v0_X25519_MLKEM1024_SHA-256_AES256GCMSIV_RaptorQ64";
const MAX_HEADER: usize = 134;
pub const MAX_SEALED_CHECKPOINT_LEN: usize = crate::checkpoint::MAX_CHECKPOINT + 36;

pub struct Packet {
    ec: ratchet::Packet,
    pq: spqr::Header,
    ciphertext: Vec<u8>,
}

impl Packet {
    fn header(&self) -> Vec<u8> {
        let pq = self.pq.to_bytes();
        let mut bytes = Vec::with_capacity(MAX_HEADER);
        bytes.extend_from_slice(PREFIX);
        bytes.extend_from_slice(&self.ec.dh);
        bytes.extend_from_slice(&self.ec.previous.to_be_bytes());
        bytes.extend_from_slice(&self.ec.number.to_be_bytes());
        bytes.push(pq.len() as u8);
        bytes.extend_from_slice(&pq);
        bytes
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = self.header();
        bytes.extend_from_slice(&self.ciphertext);
        bytes
    }

    /// Bounded framing checks; `Session::receive` authenticates every header field.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        if !(66 + 16..=MAX_HEADER + MAX_PLAINTEXT + 16).contains(&bytes.len()) {
            return Err(Error::Limit);
        }
        if &bytes[..8] != PREFIX {
            return Err(Error::Encoding);
        }
        let pq_len = bytes[48] as usize;
        if pq_len != 17 && pq_len != 85 {
            return Err(Error::Encoding);
        }
        let header_len = 49 + pq_len;
        if !(header_len + 16..=header_len + MAX_PLAINTEXT + 16).contains(&bytes.len()) {
            return Err(Error::Limit);
        }
        let dh = bytes[8..40].try_into().map_err(|_| Error::Encoding)?;
        validate_public(&dh)?;
        Ok(Self {
            ec: ratchet::Packet {
                dh,
                previous: u32::from_be_bytes(
                    bytes[40..44].try_into().map_err(|_| Error::Encoding)?,
                ),
                number: u32::from_be_bytes(bytes[44..48].try_into().map_err(|_| Error::Encoding)?),
                ciphertext: Vec::new(),
            },
            pq: spqr::Header::from_bytes(&bytes[49..header_len])?,
            ciphertext: bytes[header_len..].to_vec(),
        })
    }
}

fn split(secret: Secret32) -> Result<(Secret32, Secret32), Error> {
    let mut output = Zeroizing::new([0; 64]);
    Hkdf::<Sha256>::new(Some(&[0; 32]), secret.0.as_ref())
        .expand(&[INFO, b":Initialize"].concat(), output.as_mut())
        .map_err(|_| Error::State)?;
    Ok((
        Secret32::from_bytes(output[..32].try_into().map_err(|_| Error::State)?),
        Secret32::from_bytes(output[32..].try_into().map_err(|_| Error::State)?),
    ))
}

fn hybrid(ec: MessageKey, pq: MessageKey) -> Result<MessageKey, Error> {
    let mut output = Zeroizing::new([0; 32]);
    Hkdf::<Sha256>::new(Some(pq.0 .0.as_ref()), ec.0 .0.as_ref())
        .expand(INFO, output.as_mut())
        .map_err(|_| Error::State)?;
    Ok(MessageKey(Secret32(output)))
}

pub struct Session {
    ec: ratchet::Session,
    pq: spqr::Ratchet,
    context: [u8; 32],
}

impl Session {
    /// Immutable shared hash of the authenticated initial transcript. This is
    /// public handshake context, not a key or a local database session ID.
    pub fn convergence_id(&self) -> [u8; 32] {
        self.context
    }
    /// True only after this session has committed an authenticated peer packet.
    /// Persisting the checkpoint preserves this state; transport receipts cannot set it.
    pub fn peer_confirmed(&self) -> bool {
        self.ec.has_received()
    }
    /// Inputs must come from an authenticated handshake; context binds both
    /// identities, negotiated protocol and handshake instance.
    pub fn initiator(
        secret: Secret32,
        responder: [u8; 32],
        context: [u8; 32],
    ) -> Result<Self, Error> {
        let (ec, pq) = split(secret)?;
        Ok(Self {
            ec: ratchet::Session::initiator(ec, responder, context)?,
            pq: spqr::Ratchet::new(pq, false)?,
            context,
        })
    }

    pub fn responder(secret: Secret32, local: DhKey, context: [u8; 32]) -> Result<Self, Error> {
        let (ec, pq) = split(secret)?;
        Ok(Self {
            ec: ratchet::Session::responder(ec, local, context),
            pq: spqr::Ratchet::new(pq, true)?,
            context,
        })
    }

    fn candidate(&self) -> Self {
        Self {
            ec: self.ec.candidate(),
            pq: self.pq.candidate(),
            context: self.context,
        }
    }

    fn aad(&self, packet: &Packet) -> Vec<u8> {
        [INFO, &self.context, &packet.header()].concat()
    }

    /// In-memory commit only. A durable adapter must save BOTH ratchets and the
    /// exact packet atomically before any network transmission.
    pub fn send(&mut self, plaintext: &[u8]) -> Result<Packet, Error> {
        if plaintext.len() > MAX_PLAINTEXT {
            return Err(Error::Limit);
        }
        let mut candidate = self.candidate();
        let (ec, ec_key) = candidate.ec.send_key()?;
        let (pq, pq_key) = candidate.pq.send_key()?;
        let mut packet = Packet {
            ec,
            pq,
            ciphertext: Vec::new(),
        };
        packet.ciphertext = hybrid(ec_key, pq_key)?.seal(plaintext, &self.aad(&packet))?;
        *self = candidate;
        Ok(packet)
    }

    pub fn receive(&mut self, packet: &Packet) -> Result<Vec<u8>, Error> {
        if !(16..=MAX_PLAINTEXT + 16).contains(&packet.ciphertext.len()) {
            return Err(Error::Limit);
        }
        let mut candidate = self.candidate();
        let ec_key = candidate.ec.receive_key(&packet.ec)?;
        let pq_key = candidate.pq.receive_key(&packet.pq)?;
        let plaintext = hybrid(ec_key, pq_key)?.open(&packet.ciphertext, &self.aad(packet))?;
        *self = candidate;
        Ok(plaintext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_and_hybrid_kdfs_match_openssl() {
        let (ec, pq) = split(Secret32::from_bytes([1; 32])).unwrap();
        assert_eq!(
            [ec.0.as_slice(), pq.0.as_slice()].concat(),
            crate::vectors::ratchet_vector("split")
        );
        let key = hybrid(
            MessageKey(Secret32::from_bytes([1; 32])),
            MessageKey(Secret32::from_bytes([2; 32])),
        )
        .unwrap();
        assert_eq!(
            key.0 .0.as_slice(),
            crate::vectors::ratchet_vector("hybrid")
        );
    }

    pub(super) fn pair() -> (Session, Session) {
        let key = DhKey::generate().unwrap();
        (
            Session::initiator(Secret32::from_bytes([7; 32]), key.public_key(), [8; 32]).unwrap(),
            Session::responder(Secret32::from_bytes([7; 32]), key, [8; 32]).unwrap(),
        )
    }

    #[test]
    fn rotations_and_one_way_bursts() {
        let (mut alice, mut bob) = pair();
        assert!(matches!(bob.send(b"early"), Err(Error::State)));
        for _ in 0..160 {
            for _ in 0..3 {
                let packet = alice.send(b"synthetic alice").unwrap();
                let encoded = packet.to_bytes();
                let packet = Packet::from_bytes(&encoded).unwrap();
                assert_eq!(bob.receive(&packet).unwrap(), b"synthetic alice");
                assert!(bob.receive(&packet).is_err());
            }
            let reply = bob.send(b"synthetic bob").unwrap();
            assert_eq!(alice.receive(&reply).unwrap(), b"synthetic bob");
        }
        assert!(alice.pq.status().0 >= 3 && bob.pq.status().0 >= 3);
    }

    #[test]
    fn every_encoded_byte_is_authenticated_without_state_loss() {
        let (mut alice, mut bob) = pair();
        let key = crate::storage::StorageKey::new(Secret32::from_bytes([31; 32])).unwrap();
        let mut covered = std::collections::BTreeSet::new();
        for round in 0..130 {
            // Alternating asymmetric bursts exercise incomplete fragment
            // decoders and acknowledgement-only pauses in both roles.
            for direction in [false, false, false, true].map(|v| v ^ (round % 2 != 0)) {
                let (sender, receiver) = if direction {
                    (&mut bob, &mut alice)
                } else {
                    (&mut alice, &mut bob)
                };
                let packet = sender.send(b"synthetic payload").unwrap();
                let encoded = packet.to_bytes();
                // Exercise each observed Braid message kind in both roles and
                // epoch parities, rather than only the first handshake header.
                let pq = packet.pq.to_bytes();
                let epoch = u64::from_be_bytes(pq[8..16].try_into().unwrap());
                if covered.insert((direction, epoch % 2, pq[16])) {
                    let before = receiver.seal_checkpoint(&key, b"mutation test").unwrap();
                    let before = key.open(&before, b"mutation test").unwrap();
                    for index in 0..encoded.len() {
                        let mut corrupted = encoded.clone();
                        corrupted[index] ^= 1;
                        if let Ok(packet) = Packet::from_bytes(&corrupted) {
                            assert!(receiver.receive(&packet).is_err(), "byte {index}");
                        }
                    }
                    // Compare the entire plaintext checkpoint, including both
                    // ratchets, skipped keys and partial fragment decoders.
                    let after = receiver.seal_checkpoint(&key, b"mutation test").unwrap();
                    assert_eq!(key.open(&after, b"mutation test").unwrap(), before);
                    *receiver = Session::open_checkpoint(&key, &after, b"mutation test").unwrap();
                }
                assert_eq!(receiver.receive(&packet).unwrap(), b"synthetic payload");
            }
        }
        assert!(alice.pq.status().0 >= 4 && bob.pq.status().0 >= 4);
        for kind in [0, 1, 2, 3, 5, 6] {
            assert!(
                covered.iter().any(|(_, _, seen)| *seen == kind),
                "kind {kind}"
            );
        }
    }

    #[test]
    fn delayed_keys_survive_failed_receive_and_classical_turn() {
        let (mut alice, mut bob) = pair();
        let first = alice.send(b"first").unwrap();
        let delayed = alice.send(b"delayed").unwrap();
        bob.receive(&first).unwrap();
        alice.receive(&bob.send(b"reply").unwrap()).unwrap();
        bob.receive(&alice.send(b"new chain").unwrap()).unwrap();
        let mut corrupted = Packet::from_bytes(&delayed.to_bytes()).unwrap();
        corrupted.ciphertext[0] ^= 1;
        assert!(bob.receive(&corrupted).is_err());
        assert_eq!(bob.receive(&delayed).unwrap(), b"delayed");
        assert!(bob.receive(&delayed).is_err());
    }

    #[test]
    fn wrong_context_and_oversize_do_not_consume_state() {
        let (mut alice, mut bob) = pair();
        assert!(matches!(
            alice.send(&vec![0; MAX_PLAINTEXT + 1]),
            Err(Error::Limit)
        ));
        let packet = alice.send(&vec![6; MAX_PLAINTEXT]).unwrap();
        bob.context[0] ^= 1;
        assert!(matches!(bob.receive(&packet), Err(Error::Authentication)));
        bob.context[0] ^= 1;
        assert_eq!(
            bob.receive(&Packet::from_bytes(&packet.to_bytes()).unwrap())
                .unwrap(),
            vec![6; MAX_PLAINTEXT]
        );
    }

    #[test]
    fn hybrid_key_depends_on_both_ratchets() {
        let key = |n| MessageKey(Secret32::from_bytes([n; 32]));
        let base = hybrid(key(1), key(2)).unwrap();
        assert_ne!(*base.0 .0, *hybrid(key(3), key(2)).unwrap().0 .0);
        assert_ne!(*base.0 .0, *hybrid(key(1), key(3)).unwrap().0 .0);
        assert_ne!(*base.0 .0, *hybrid(key(2), key(1)).unwrap().0 .0);
    }

    #[test]
    fn packet_loss_and_reordering_cross_post_quantum_epochs() {
        let key = crate::storage::StorageKey::new(Secret32::from_bytes([31; 32])).unwrap();
        for seed in [1, 7, 71, 771, 7711, 0x1234abcd, 1 << 63, u64::MAX] {
            let (mut alice, mut bob) = pair();
            bob.receive(&alice.send(b"bootstrap").unwrap()).unwrap();
            alice
                .receive(&bob.send(b"bootstrap reply").unwrap())
                .unwrap();
            let mut peers = [alice, bob];
            let mut queue = Vec::new();
            // Reproducible public scheduling only; production keys continue to
            // use OS entropy. Vary sender bursts, loss and delivery ordering.
            let mut random = seed;
            for index in 0..1000 {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                let sender = random as usize % 2;
                let content = (index as u32).to_be_bytes();
                let packet = peers[sender].send(&content).unwrap();
                if !random.is_multiple_of(31) {
                    queue.push((1 - sender, packet, content));
                }
                if !queue.is_empty()
                    && (index % 3 == 0 || !random.is_multiple_of(3) || queue.len() > 8)
                {
                    // FIFO service periodically prevents intentionally delayed
                    // content from aging beyond the documented retention bound.
                    let selected = if index % 3 == 0 {
                        0
                    } else {
                        random as usize % queue.len()
                    };
                    let (receiver, packet, content) = queue.remove(selected);
                    let mut corrupted = Packet::from_bytes(&packet.to_bytes()).unwrap();
                    corrupted.ciphertext[0] ^= 1;
                    assert!(peers[receiver].receive(&corrupted).is_err());
                    if index % 17 == 0 {
                        let sealed = peers[receiver]
                            .seal_checkpoint(&key, &[receiver as u8])
                            .unwrap();
                        peers[receiver] =
                            Session::open_checkpoint(&key, &sealed, &[receiver as u8]).unwrap();
                    }
                    assert_eq!(
                        peers[receiver].receive(&packet).unwrap(),
                        content,
                        "seed {seed}, step {index}"
                    );
                    assert!(peers[receiver].receive(&packet).is_err());
                }
            }
            for (receiver, packet, content) in queue {
                assert_eq!(
                    peers[receiver].receive(&packet).unwrap(),
                    content,
                    "seed {seed}"
                );
            }
            assert!(
                peers[0].pq.status().0 >= 5 && peers[1].pq.status().0 >= 5,
                "seed {seed}"
            );
            assert!(peers[0].pq.status().3 <= 2 && peers[1].pq.status().3 <= 2);
        }
    }
}
