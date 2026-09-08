use super::*;
use crate::{claims::tests::pair, incoming::tests::trust};
#[test]
fn call_control_commits_atomically_and_unanswered_ringing_expires() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    super::tests::configure(dir.path());
    let (_, peer) = trust(&mut alice, &mut bob);
    let binding = crate::peers::parse(&bob.own_device_binding().unwrap())
        .unwrap()
        .binding;
    bob.configure_recovery(
        &binding.server,
        binding.account,
        sigil_crypto::Secret32::from_bytes([7; 32]),
    )
    .unwrap();
    let id = [77; 32];
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON calls BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(alice.create_call(id, now, 3600).is_err());
    assert!(matches!(alice.call(id, now), Err(Error::NotFound)));
    alice.db.execute_batch("DROP TRIGGER fail;").unwrap();
    alice.create_call(id, now, 3600).unwrap();
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON call_jobs BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(alice.invite_to_call(id, peer, now).is_err());
    assert!(load(&alice.db, &alice.key, &id).unwrap().invites.is_empty());
    alice.db.execute_batch("DROP TRIGGER fail;").unwrap();
    alice.invite_to_call(id, peer, now).unwrap();
    for attempt in alice.resume_calls_online(now).unwrap() {
        attempt.result.unwrap();
    }
    for attempt in alice.resume_outbound_online(now).unwrap() {
        attempt.result.unwrap();
    }
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON calls BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    let attempts = bob.receive_mailbox_online(now).unwrap();
    assert_eq!(attempts.len(), 1);
    assert!(attempts[0].result.is_err());
    for table in ["calls", "sessions", "inbox"] {
        assert_eq!(
            bob.db
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, u32>(0))
                .unwrap(),
            0
        );
    }
    bob.db.execute_batch("DROP TRIGGER fail;").unwrap();
    assert!(bob.receive_mailbox_online(now).unwrap().is_empty());
    let attempts = bob.receive_mailbox_online(now).unwrap();
    assert_eq!(attempts.len(), 1);
    assert!(matches!(attempts[0].result, Ok(crate::MailboxEvent::Call(value)) if value == id));
    assert!(bob.call(id, now + 59).unwrap().phase == Phase::Ringing);
    assert!(matches!(
        bob.answer_call(id, true, now + 60),
        Err(Error::Expired)
    ));
    assert!(bob.call(id, now + 60).unwrap().phase == Phase::Declined);
    let record = load(&bob.db, &bob.key, &id).unwrap();
    assert!(record.secret.is_none() && record.shares.is_empty() && record.ring_until.is_none());
    assert!(bob.recovery_records(None).unwrap().is_empty());
}
