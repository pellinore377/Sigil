use super::*;
use crate::{
    claims::tests::pair,
    incoming::tests::{next, start, trust},
};

#[test]
fn missing_retired_session_recovers_through_explicit_action_and_normal_worker() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let original = next(&bob);
    let first = bob.accept_delivery(&original).unwrap();
    bob.acknowledge_incoming_online().unwrap();
    bob.send_peer_text(a, [91; 32], "confirmation", now, now)
        .unwrap();
    bob.send_pending_online(first.session, now).unwrap();
    alice.receive_mailbox_online(now).unwrap();
    alice.acknowledge_incoming_online().unwrap();
    bob.retire_session(first.session).unwrap();
    alice
        .queue_peer_text(b, [92; 32], "recover missing session", now, now)
        .unwrap();
    assert!(alice.sync_step_online(now).failure.is_none());
    let step = bob.sync_step_online(now);
    let RecoveryAdvice::Offer(action) = &step.incoming[0].recovery else {
        panic!("lost session must offer recovery");
    };
    bob.approve_recovery(action, now).unwrap();
    let mut recovered = false;
    for _ in 0..5 {
        assert!(bob.sync_step_online(now).failure.is_none());
        assert!(alice.sync_step_online(now).failure.is_none());
        let step = bob.sync_step_online(now);
        assert!(step.failure.is_none());
        recovered|=step.incoming.iter().any(|v|matches!(&v.result,Ok(MailboxEvent::Text(text)) if text.text().unwrap().body=="recover missing session"));
    }
    assert!(recovered);
    assert_eq!(
        bob.message(first.session, [4; 32]).unwrap(),
        first.plaintext
    );
}

#[test]
fn recovery_offers_require_missing_state_current_trust_and_live_delivery() {
    let (_dir, _fixture, mut alice, mut bob, now) = pair();
    let (a, b) = trust(&mut alice, &mut bob);
    start(&mut alice, b, now);
    let mut delivery = next(&bob);
    assert!(matches!(
        bob.recovery_advice(&delivery, &Err(Error::Limit), now),
        RecoveryAdvice::Refused(RecoveryBlock::Capacity)
    ));
    // A damaged local sealed key is not evidence authorizing protocol recovery.
    let state: Vec<u8> = bob
        .db
        .query_row(
            "SELECT state FROM prekeys WHERE id=?1",
            [[2u8; 32].as_slice()],
            |r| r.get(0),
        )
        .unwrap();
    bob.db
        .execute("UPDATE prekeys SET state=zeroblob(length(state))", [])
        .unwrap();
    let result = bob.accept_delivery(&delivery).map(MailboxEvent::Text);
    assert!(result.is_err());
    assert!(matches!(
        bob.recovery_advice(&delivery, &result, now),
        RecoveryAdvice::Refused(_)
    ));
    bob.db
        .execute("UPDATE prekeys SET state=?1", [&state])
        .unwrap();
    bob.db.execute("UPDATE prekeys SET state=NULL", []).unwrap();
    let result = bob.accept_delivery(&delivery).map(MailboxEvent::Text);
    let RecoveryAdvice::Offer(action) = bob.recovery_advice(&delivery, &result, now) else {
        panic!("missing state");
    };
    assert_eq!(action.sequence(), delivery.sequence);
    bob.block_peer(a, true).unwrap();
    assert!(matches!(
        bob.approve_recovery(&action, now),
        Err(Error::Unprepared)
    ));
    assert!(matches!(
        bob.recovery_advice(&delivery, &result, now),
        RecoveryAdvice::Refused(RecoveryBlock::Trust)
    ));
    bob.block_peer(a, false).unwrap();
    // Availability can change after the action was offered. Expired approval
    // must reject before attempting to commit newly decryptable traffic.
    bob.db
        .execute("UPDATE prekeys SET state=?1", [&state])
        .unwrap();
    assert!(matches!(
        bob.approve_recovery(&action, delivery.expires_at),
        Err(Error::Expired)
    ));
    assert_eq!(
        bob.db
            .query_row("SELECT count(*) FROM incoming", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    delivery.payload = "00".repeat(32);
    let result = bob.accept_delivery(&delivery).map(MailboxEvent::Text);
    assert!(matches!(
        bob.recovery_advice(&delivery, &result, now),
        RecoveryAdvice::Refused(RecoveryBlock::InvalidTraffic)
    ));
    assert_eq!(
        bob.db
            .query_row("SELECT count(*) FROM retry_outbox", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
