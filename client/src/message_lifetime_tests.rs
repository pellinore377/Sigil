use super::*;
use crate::{
    claims::tests::pair,
    incoming::tests::{next, trust},
};

fn reopen(path: &Path) -> ClientStore {
    ClientStore::open(
        path,
        StorageKey::new(sigil_crypto::Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap()
}
fn old_sessions(store: &mut ClientStore) {
    let tx = store.db.transaction().unwrap();
    for n in 0u32..1024 {
        let mut id = [230; 32];
        id[..4].copy_from_slice(&n.to_be_bytes());
        let mut aad = state_binding(&id, 1, None);
        aad.extend_from_slice(b"Sigil/retired-session/v0");
        let sealed = store.key.seal(b"retired", &aad).unwrap();
        tx.execute(
            "INSERT INTO sessions(id,revision,state,suite,retired) VALUES(?1,1,?2,2,1)",
            (id.as_slice(), sealed),
        )
        .unwrap();
    }
    tx.commit().unwrap();
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "4096 cryptographic round trips: cargo test --release -p sigil-client incoming::lifetime_tests --lib"
)]
fn retained_sessions_and_messages_cross_old_limits_without_reset_or_replay() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (a, b) = trust(&mut alice, &mut bob);
    old_sessions(&mut alice);
    old_sessions(&mut bob);
    alice
        .queue_peer_text(b, [4; 32], "first lifetime text", now, now)
        .unwrap();
    assert!(alice.sync_step_online(now).failure.is_none());
    let original = next(&bob);
    let first = bob.accept_delivery(&original).unwrap();
    bob.acknowledge_incoming_online().unwrap();
    let (reply, _) = bob.send_peer_text(a, [5; 32], "confirm", now, now).unwrap();
    bob.send_pending_online(reply, now).unwrap();
    alice.receive_mailbox_online(now).unwrap();
    alice.acknowledge_incoming_online().unwrap();
    let session = alice.active_session(b).unwrap().unwrap();
    let own = decode_id(&bob.connection_session().unwrap().unwrap().device_id).unwrap();
    // Exercise server authorization/storage and real packet decryption locally;
    // the first and final sends also use the HTTPS worker.
    let mut server = sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap();
    let sending = crate::connection::tests::credential(&alice);
    let receiving = crate::connection::tests::credential(&bob);
    for n in 0u32..4096 {
        let mut id = [231; 32];
        id[..4].copy_from_slice(&n.to_be_bytes());
        alice
            .send_peer_text(b, id, "retained lifetime text", now, now)
            .unwrap();
        let submit = alice
            .prepare_delivery(session, id, own, now + 604800, now)
            .unwrap();
        let request = sigil_protocol::mailbox::Submit {
            recipient_device: submit.recipient_device.clone(),
            message_id: submit.message_id.clone(),
            payload: submit.payload.clone(),
            expires_at: submit.expires_at,
        };
        let receipt = server.submit_message(&sending, request, now).unwrap();
        let delivery = Delivery {
            sequence: receipt.sequence,
            sender_device: original.sender_device.clone(),
            message_id: submit.message_id,
            payload: submit.payload,
            expires_at: submit.expires_at,
        };
        let received = bob.accept_delivery(&delivery).unwrap();
        assert!(!received.duplicate);
        alice.acknowledge_sent(session, id, &receipt).unwrap();
        server
            .acknowledge_message(&receiving, delivery.sequence, now)
            .unwrap();
        let mut retained = record(&bob.db, &bob.key, &own, delivery.sequence)
            .unwrap()
            .unwrap();
        retained.acknowledged = true;
        bob.db
            .execute(
                "UPDATE incoming SET acknowledged=1,state=?1 WHERE sequence=?2",
                (
                    retained.seal(&bob.key, &own, delivery.sequence).unwrap(),
                    delivery.sequence,
                ),
            )
            .unwrap();
    }
    assert_eq!(
        bob.message(first.session, [4; 32]).unwrap(),
        first.plaintext
    );
    drop(alice);
    drop(bob);
    let mut alice = reopen(&dir.path().join("alice.db"));
    let mut bob = reopen(&dir.path().join("bob.db"));
    assert!(bob.accept_delivery(&original).unwrap().duplicate);
    alice
        .queue_peer_text(b, [6; 32], "after turnover", now, now)
        .unwrap();
    let mut delivered = false;
    for _ in 0..3 {
        let step = alice.sync_step_online(now);
        assert!(step.failure.is_none(), "{:?}", step.failure);
        let step = bob.sync_step_online(now);
        assert!(step.failure.is_none(), "{:?}", step.failure);
        delivered |= step.incoming.iter().any(|item|matches!(&item.result,Ok(MailboxEvent::Text(text)) if text.text().unwrap().body=="after turnover"));
    }
    assert!(delivered);
    assert_eq!(
        bob.message(first.session, [4; 32]).unwrap(),
        first.plaintext
    );
}
