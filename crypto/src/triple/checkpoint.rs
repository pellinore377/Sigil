use super::*;
use crate::{
    checkpoint::{Reader, Writer},
    storage::StorageKey,
};

const PREFIX: &[u8; 8] = b"SGTS\0\x02\0\0";

impl Session {
    /// Saves both ratchets in one authenticated encrypted record. The caller
    /// must atomically commit this with its inbox/outbox. Snapshot rollback is
    /// not a supported live-session recovery mechanism.
    pub fn seal_checkpoint(&self, key: &StorageKey, binding: &[u8]) -> Result<Vec<u8>, Error> {
        let mut out = Writer::new();
        out.put(PREFIX)?;
        out.put(&self.context)?;
        out.blob(&self.ec.checkpoint_bytes())?;
        self.pq.write(&mut out)?;
        key.seal(&out.finish(), binding)
    }

    pub fn open_checkpoint(key: &StorageKey, sealed: &[u8], binding: &[u8]) -> Result<Self, Error> {
        if sealed.len() > MAX_SEALED_CHECKPOINT_LEN {
            return Err(Error::Limit);
        }
        let bytes = key.open(sealed, binding)?;
        let mut input = Reader::new(&bytes)?;
        let prefix = input.take::<8>()?;
        if &prefix != PREFIX && &prefix != b"SGTS\0\x01\0\0" {
            return Err(Error::Encoding);
        }
        let context = input.take()?;
        let ec_bytes = input.blob(9000)?;
        if ec_bytes.get(5) != Some(&prefix[5]) {
            return Err(Error::Encoding);
        }
        let ec = ratchet::Session::from_checkpoint_bytes(ec_bytes)?;
        // The embedded classical record independently stores its context.
        if ec_bytes.get(8..40) != Some(context.as_slice()) {
            return Err(Error::State);
        }
        let pq = spqr::Ratchet::read(&mut input)?;
        input.finish()?;
        Ok(Self { ec, pq, context })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reopen(session: Session, key: &StorageKey, binding: &[u8]) -> Session {
        let sealed = session.seal_checkpoint(key, binding).unwrap();
        Session::open_checkpoint(key, &sealed, binding).unwrap()
    }

    #[test]
    fn peer_confirmation_requires_authenticated_receive_and_survives_checkpoint() {
        let (mut alice, mut bob) = super::super::tests::pair();
        let key = StorageKey::new(Secret32::from_bytes([31; 32])).unwrap();
        assert!(!alice.peer_confirmed());
        assert!(!bob.peer_confirmed());
        let packet = alice.send(b"initial text").unwrap();
        alice = reopen(alice, &key, b"alice");
        assert!(!alice.peer_confirmed());
        bob.receive(&packet).unwrap();
        let reply = bob.send(b"reply").unwrap();
        let mut damaged = Packet::from_bytes(&reply.to_bytes()).unwrap();
        damaged.ciphertext[0] ^= 1;
        assert!(alice.receive(&damaged).is_err());
        assert!(!alice.peer_confirmed());
        alice = reopen(alice, &key, b"alice");
        assert!(!alice.peer_confirmed());
        alice.receive(&reply).unwrap();
        assert!(reopen(alice, &key, b"alice").peer_confirmed());
        assert!(reopen(bob, &key, b"bob").peer_confirmed());
    }

    #[test]
    fn encrypted_restart_during_every_message_preserves_both_ratchets() {
        let (mut alice, mut bob) = super::super::tests::pair();
        let key = StorageKey::new(Secret32::from_bytes([31; 32])).unwrap();
        for _ in 0..130 {
            alice = reopen(alice, &key, b"alice");
            let packet = alice.send(b"synthetic alice").unwrap();
            alice = reopen(alice, &key, b"alice");
            bob = reopen(bob, &key, b"bob");
            assert_eq!(bob.receive(&packet).unwrap(), b"synthetic alice");
            bob = reopen(bob, &key, b"bob");
            let packet = bob.send(b"synthetic bob").unwrap();
            bob = reopen(bob, &key, b"bob");
            assert_eq!(alice.receive(&packet).unwrap(), b"synthetic bob");
            alice = reopen(alice, &key, b"alice");
        }
        assert!(alice.pq.status().0 >= 4 && bob.pq.status().0 >= 4);
    }

    #[test]
    fn maximum_skipped_keys_survive_encrypted_restart() {
        let (mut alice, mut bob) = super::super::tests::pair();
        let key = StorageKey::new(Secret32::from_bytes([31; 32])).unwrap();
        let mut packets = Vec::new();
        for _ in 0..129 {
            packets.push(alice.send(b"synthetic").unwrap());
        }
        bob.receive(packets.last().unwrap()).unwrap();
        bob = reopen(bob, &key, b"bob");
        assert_eq!(bob.pq.status().4, 128);
        for packet in &packets[..128] {
            assert_eq!(bob.receive(packet).unwrap(), b"synthetic");
        }
        bob = reopen(bob, &key, b"bob");
        assert_eq!(bob.pq.status().4, 0);
        alice.receive(&bob.send(b"reply").unwrap()).unwrap();
    }

    #[test]
    fn permanent_loss_does_not_exhaust_both_caches_and_eviction_order_survives_restart() {
        let (mut alice, mut bob) = super::super::tests::pair();
        let key = StorageKey::new(Secret32::from_bytes([31; 32])).unwrap();
        let mut lost = Vec::new();
        for n in 0..1000u32 {
            lost.push(alice.send(&n.to_be_bytes()).unwrap());
            let delivered = alice.send(b"later traffic").unwrap();
            if n % 17 == 0 || n == 128 {
                let mut bad = Packet::from_bytes(&delivered.to_bytes()).unwrap();
                bad.ciphertext[0] ^= 1;
                assert!(bob.receive(&bad).is_err());
            }
            if n == 128 {
                let before = bob.seal_checkpoint(&key, b"bob").unwrap();
                let mut probe = Session::open_checkpoint(&key, &before, b"bob").unwrap();
                assert_eq!(probe.receive(&lost[0]).unwrap(), 0u32.to_be_bytes());
            }
            assert_eq!(bob.receive(&delivered).unwrap(), b"later traffic");
            if n == 128 {
                assert!(bob.receive(&lost[0]).is_err());
            }
            if n % 31 == 0 {
                bob = reopen(bob, &key, b"bob");
            }
        }
        assert_eq!(bob.pq.status().4, 128);
        bob = reopen(bob, &key, b"bob");
        assert!(bob.receive(&lost[0]).is_err());
        for n in (872..1000usize).rev() {
            assert_eq!(bob.receive(&lost[n]).unwrap(), (n as u32).to_be_bytes());
        }
        assert_eq!(bob.pq.status().4, 0);
        alice
            .receive(&bob.send(b"reply after sustained loss").unwrap())
            .unwrap();
    }

    #[test]
    fn version_one_checkpoints_keep_all_old_keys_and_upgrade_explicitly() {
        let (mut alice, mut bob) = super::super::tests::pair();
        let key = StorageKey::new(Secret32::from_bytes([31; 32])).unwrap();
        let mut packets = Vec::new();
        for _ in 0..33 {
            packets.push(alice.send(b"legacy delayed").unwrap());
        }
        bob.receive(&packets[32]).unwrap();
        let sealed = bob.seal_checkpoint(&key, b"bob").unwrap();
        let mut legacy = key.open(&sealed, b"bob").unwrap();
        assert_eq!(&legacy[..8], b"SGTS\0\x02\0\0");
        legacy[5] = 1;
        legacy[44 + 5] = 1; // embedded classical record after length prefix
                            // One chain: FIFO order is the original v1 sorted (DH, counter) order.
        let legacy = key.seal(&legacy, b"bob").unwrap();
        let mut restored = Session::open_checkpoint(&key, &legacy, b"bob").unwrap();
        for packet in &packets[..32] {
            assert_eq!(restored.receive(packet).unwrap(), b"legacy delayed");
        }
        let upgraded = restored.seal_checkpoint(&key, b"bob").unwrap();
        assert_eq!(
            &key.open(&upgraded, b"bob").unwrap()[..8],
            b"SGTS\0\x02\0\0"
        );
    }

    #[test]
    fn checkpoints_reject_wrong_bindings_tampering_and_malformed_contents() {
        let (mut alice, _) = super::super::tests::pair();
        alice.send(b"start").unwrap();
        let key = StorageKey::new(Secret32::from_bytes([31; 32])).unwrap();
        let other = StorageKey::new(Secret32::from_bytes([32; 32])).unwrap();
        let sealed = alice.seal_checkpoint(&key, b"alice").unwrap();
        assert!(Session::open_checkpoint(&other, &sealed, b"alice").is_err());
        assert!(Session::open_checkpoint(&key, &sealed, b"bob").is_err());
        let mut tampered = sealed.clone();
        tampered[24] ^= 1;
        assert!(Session::open_checkpoint(&key, &tampered, b"alice").is_err());
        let bytes = key.open(&sealed, b"alice").unwrap();
        for len in [0, 7, 39, 43, bytes.len() - 1] {
            let bad = key.seal(&bytes[..len], b"alice").unwrap();
            assert!(Session::open_checkpoint(&key, &bad, b"alice").is_err());
        }
        for offset in [0, 8, 40, bytes.len() - 4] {
            let mut bad = Zeroizing::new(bytes.to_vec());
            bad[offset] ^= 128;
            let sealed = key.seal(&bad, b"alice").unwrap();
            assert!(
                Session::open_checkpoint(&key, &sealed, b"alice").is_err(),
                "offset {offset}"
            );
        }
        let mut bad = Zeroizing::new(bytes.to_vec());
        bad.push(0);
        assert!(
            Session::open_checkpoint(&key, &key.seal(&bad, b"alice").unwrap(), b"alice").is_err()
        );
    }
}
