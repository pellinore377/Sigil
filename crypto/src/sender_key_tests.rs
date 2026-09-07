use super::*;

fn pair() -> (Sender, Receiver) {
    let (sender, distribution) = Sender::new([1; 32], [2; 32], 3, [4; 32]).unwrap();
    let receiver =
        Receiver::from_authenticated_distribution(distribution, sender.context()).unwrap();
    (sender, receiver)
}
fn snapshot(receiver: &Receiver) -> Zeroizing<Vec<u8>> {
    let key = StorageKey::new(Secret32::from_bytes([8; 32])).unwrap();
    let sealed = receiver.seal_checkpoint(&key, b"synthetic").unwrap();
    key.open(
        &sealed,
        &receiver.context.checkpoint_aad(b"receiver", b"synthetic"),
    )
    .unwrap()
}

#[test]
fn distribution_is_strict_secret_framing_and_never_grants_implicit_context_trust() {
    let (sender, distribution) = Sender::new([1; 32], [2; 32], 3, [4; 32]).unwrap();
    let bytes = distribution.to_bytes().unwrap();
    assert_eq!(bytes.len(), 208);
    assert_eq!(
        Distribution::from_bytes(&bytes)
            .unwrap()
            .to_bytes()
            .unwrap()
            .as_slice(),
        bytes.as_slice()
    );
    for n in 0..bytes.len() {
        assert!(Distribution::from_bytes(&bytes[..n]).is_err());
    }
    let mut trailing = bytes.to_vec();
    trailing.push(0);
    assert!(Distribution::from_bytes(&trailing).is_err());
    let mut invalid = bytes.to_vec();
    invalid[176..208].fill(0);
    assert!(Distribution::from_bytes(&invalid).is_err());
    let mut context = sender.context();
    context.epoch += 1;
    assert!(Receiver::from_authenticated_distribution(distribution, context).is_err());
}

#[test]
fn all_packet_bytes_are_bound_and_failures_leave_receiver_unchanged() {
    let (mut sender, mut receiver) = pair();
    let packet = sender.seal([5; 32], b"synthetic group text").unwrap();
    let bytes = packet.to_bytes();
    let initial = snapshot(&receiver);
    for n in 0..bytes.len() {
        let mut changed = bytes.clone();
        changed[n] ^= 1;
        if let Ok(packet) = Packet::from_bytes(&changed) {
            assert!(receiver.open(&packet).is_err());
        }
        assert_eq!(snapshot(&receiver).as_slice(), initial.as_slice());
    }
    for n in 0..bytes.len() {
        assert!(Packet::from_bytes(&bytes[..n]).is_err());
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(Packet::from_bytes(&trailing).is_err());
    assert_eq!(
        receiver.open(&Packet::from_bytes(&bytes).unwrap()).unwrap(),
        b"synthetic group text"
    );
    assert_eq!(receiver.open(&packet), Err(Error::Replay));
    assert_eq!(receiver.counter(), 1);
    assert_eq!(sender.counter(), 1);
}

#[test]
fn valid_signature_does_not_bypass_aead_and_group_members_cannot_forge_sender() {
    let (mut sender, mut receiver) = pair();
    let mut packet = sender.seal([5; 32], b"original").unwrap();
    let initial = snapshot(&receiver);
    packet.message[0] ^= 1;
    packet.signature = sender.signing.sign(&packet.statement()).unwrap();
    assert_eq!(receiver.open(&packet), Err(Error::Authentication));
    assert_eq!(snapshot(&receiver).as_slice(), initial.as_slice());
    packet.message[0] ^= 1;
    // A receiver knows the symmetric chain but not the sender's signing secret.
    let (_, message_key) = derive_chain(&receiver.chain).unwrap();
    packet.ciphertext = message_key.seal(b"forged!!", &packet.header()).unwrap();
    packet.signature = IdentityKey::generate()
        .unwrap()
        .sign(&packet.statement())
        .unwrap();
    assert_eq!(receiver.open(&packet), Err(Error::Authentication));
    assert_eq!(snapshot(&receiver).as_slice(), initial.as_slice());
}

#[test]
fn bounded_reordering_evicts_oldest_only_after_success_and_continues_after_loss() {
    let (mut sender, mut receiver) = pair();
    let packets: Vec<_> = (0..=258u32)
        .map(|n| {
            let mut id = [0; 32];
            id[..4].copy_from_slice(&n.to_be_bytes());
            sender.seal(id, &n.to_be_bytes()).unwrap()
        })
        .collect();
    assert_eq!(receiver.open(&packets[129]), Err(Error::Limit));
    assert_eq!(receiver.counter(), 0);
    assert_eq!(receiver.open(&packets[128]).unwrap(), 128u32.to_be_bytes());
    assert_eq!(receiver.skipped.len(), MAX_SKIPPED_KEYS);
    receiver.open(&packets[129]).unwrap();
    let mut bad = Packet::from_bytes(&packets[258].to_bytes()).unwrap();
    bad.signature[0] ^= 1;
    assert_eq!(receiver.open(&bad), Err(Error::Authentication));
    assert_eq!(receiver.open(&packets[0]).unwrap(), 0u32.to_be_bytes());
    receiver.open(&packets[258]).unwrap();
    assert_eq!(receiver.skipped.len(), MAX_SKIPPED_KEYS);
    assert_eq!(receiver.open(&packets[1]), Err(Error::Replay));
    assert_eq!(receiver.open(&packets[130]).unwrap(), 130u32.to_be_bytes());
    assert_eq!(receiver.open(&packets[130]), Err(Error::Replay));
    assert_eq!(receiver.counter(), 259);
    let next = sender.seal([9; 32], b"continued").unwrap();
    assert_eq!(receiver.open(&next).unwrap(), b"continued");
}

#[test]
fn encrypted_checkpoints_preserve_counters_signing_and_skipped_keys_with_context_binding() {
    let (mut sender, mut receiver) = pair();
    let first = sender.seal([5; 32], b"first").unwrap();
    let second = sender.seal([6; 32], b"second").unwrap();
    receiver.open(&second).unwrap();
    let key = StorageKey::new(Secret32::from_bytes([8; 32])).unwrap();
    let a = sender.seal_checkpoint(&key, b"synthetic record").unwrap();
    let b = receiver.seal_checkpoint(&key, b"synthetic record").unwrap();
    let mut wrong = sender.context();
    wrong.group[0] ^= 1;
    assert!(Sender::open_checkpoint(&key, &a, wrong, b"synthetic record").is_err());
    assert!(Receiver::open_checkpoint(&key, &b, wrong, b"synthetic record").is_err());
    assert!(Sender::open_checkpoint(&key, &a, sender.context(), b"another record").is_err());
    assert!(Receiver::open_checkpoint(&key, &a, sender.context(), b"synthetic record").is_err());
    let context = sender.context();
    drop(sender);
    drop(receiver);
    let mut sender = Sender::open_checkpoint(&key, &a, context, b"synthetic record").unwrap();
    let mut receiver = Receiver::open_checkpoint(&key, &b, context, b"synthetic record").unwrap();
    assert_eq!(sender.counter(), 2);
    assert_eq!(receiver.counter(), 2);
    assert_eq!(receiver.open(&first).unwrap(), b"first");
    assert_eq!(receiver.open(&second), Err(Error::Replay));
    let next = sender.seal([7; 32], b"third").unwrap();
    assert_eq!(receiver.open(&next).unwrap(), b"third");
}

#[test]
fn fresh_epoch_uses_independent_entropy_and_old_chain_cannot_decrypt_it() {
    let (mut old, mut old_receiver) = pair();
    let old_packet = old.seal([5; 32], b"old epoch").unwrap();
    let (mut fresh, distribution) = Sender::new([1; 32], [9; 32], 4, [4; 32]).unwrap();
    let mut new_receiver =
        Receiver::from_authenticated_distribution(distribution, fresh.context()).unwrap();
    let packet = fresh.seal([6; 32], b"fresh epoch").unwrap();
    assert!(old.context().chain != fresh.context().chain);
    assert_eq!(old_receiver.open(&packet), Err(Error::Authentication));
    assert_eq!(new_receiver.open(&old_packet), Err(Error::Authentication));
    // Isolate the entropy boundary: even with the new public context/signature
    // key, possession of the old chain does not decrypt the fresh ciphertext.
    old_receiver.context = fresh.context();
    old_receiver.public = fresh.signing.public_key();
    assert_eq!(old_receiver.open(&packet), Err(Error::Authentication));
    assert_eq!(new_receiver.open(&packet).unwrap(), b"fresh epoch");
}

#[test]
fn bounds_and_counter_exhaustion_fail_without_key_advance_or_wraparound() {
    let (mut sender, mut receiver) = pair();
    let before = *sender.chain.0;
    assert!(matches!(
        sender.seal([1; 32], &vec![0; MAX_PLAINTEXT + 1]),
        Err(Error::Limit)
    ));
    assert_eq!(*sender.chain.0, before);
    let maximum = sender.seal([2; 32], &vec![7; MAX_PLAINTEXT]).unwrap();
    assert_eq!(maximum.to_bytes().len(), MAX_PACKET);
    assert_eq!(
        receiver
            .open(&Packet::from_bytes(&maximum.to_bytes()).unwrap())
            .unwrap()
            .len(),
        MAX_PLAINTEXT
    );
    sender.counter = u64::MAX - 1;
    receiver.counter = u64::MAX - 1;
    let last = sender.seal([3; 32], b"last").unwrap();
    receiver.open(&last).unwrap();
    let before = *sender.chain.0;
    assert!(matches!(
        sender.seal([4; 32], b"overflow"),
        Err(Error::Limit)
    ));
    assert_eq!(*sender.chain.0, before);
    assert_eq!(sender.counter(), u64::MAX);
    let mut beyond = last;
    beyond.counter = u64::MAX;
    beyond.signature = sender.signing.sign(&beyond.statement()).unwrap();
    let initial = snapshot(&receiver);
    assert_eq!(receiver.open(&beyond), Err(Error::Limit));
    assert_eq!(snapshot(&receiver).as_slice(), initial.as_slice());
}

#[test]
fn packet_and_chain_agree_with_independent_openssl_sodium_fixtures() {
    fn field(object: &serde_json::Value, name: &str) -> Vec<u8> {
        object[name]
            .as_str()
            .unwrap()
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../tests/vectors/sender-keys.json")).unwrap();
    let encoded = field(&fixture, "distribution");
    let distribution = Distribution::from_bytes(&encoded).unwrap();
    let context = distribution.context();
    let mut sender = Sender {
        context,
        chain: Secret32::from_bytes(*distribution.chain.0),
        counter: 0,
        signing: IdentityKey::from_test_bytes(
            field(&fixture, "signing_secret").try_into().unwrap(),
        ),
    };
    assert_eq!(sender.signing.public_key(), distribution.public);
    let mut receiver = Receiver::from_authenticated_distribution(distribution, context).unwrap();
    for item in fixture["packets"].as_array().unwrap() {
        let bytes = field(item, "packet");
        let packet = Packet::from_bytes(&bytes).unwrap();
        let plaintext = field(item, "plaintext");
        assert_eq!(packet.to_bytes(), bytes);
        assert_eq!(receiver.open(&packet).unwrap(), plaintext);
        let produced = sender.seal(packet.message(), &plaintext).unwrap();
        // XEdDSA signing is randomized; compare the complete deterministic
        // header/ciphertext and independently verify the C-produced signature.
        assert_eq!(
            &produced.to_bytes()[..bytes.len() - 64],
            &bytes[..bytes.len() - 64]
        );
        verify_signature(
            &sender.signing.public_key(),
            &produced.statement(),
            &produced.signature,
        )
        .unwrap();
    }
    assert_eq!(&sender.chain.0[..], field(&fixture, "final_chain"));
    assert_eq!(&receiver.chain.0[..], field(&fixture, "final_chain"));
    assert_eq!(sender.counter(), 3);
    assert_eq!(receiver.counter(), 3);
}
