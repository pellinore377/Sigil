//! Fair, bounded scheduling over existing durable packet journals.
use super::*;

#[derive(Debug)]
pub struct OutboundAttempt {
    pub session: Id,
    /// Acceptance is server acceptance, not peer delivery. An error can follow
    /// server acceptance or committed expiry; existing journals reconcile retry.
    pub result: Result<SendProgress, Error>,
}

fn cursor(
    db: &Connection,
    key: &StorageKey,
    own: &Id,
) -> Result<(Vec<u8>, Option<Vec<u8>>), Error> {
    let sealed: Option<Vec<u8>> = db.query_row(
        "SELECT CASE WHEN length(state) IN (36,68) THEN state END FROM outbound_cursor WHERE id=1",
        [], |r| r.get(0),
    ).optional()?;
    let after = match &sealed {
        Some(value) => key
            .open(value, &binding(33, own, b"outbound scan"))?
            .to_vec(),
        None => Vec::new(),
    };
    if !after.is_empty() && after.len() != 32 {
        return Err(Error::InvalidStore);
    }
    Ok((after, sealed))
}

impl ClientStore {
    /// Attempt one queued packet from each of at most 16 sessions. Queue order
    /// within each session is preserved. Local errors do not starve other sessions;
    /// network errors stop the pass so callers can honor Retry-After. Cursor wrap
    /// takes an empty pass. Unprepared packets require application intervention.
    pub fn resume_outbound_online(&mut self, now: u64) -> Result<Vec<OutboundAttempt>, Error> {
        if now == 0 || now > i64::MAX as u64 {
            return Err(Error::Expired);
        }
        self.connected_client()?;
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let (after, expected) = cursor(&self.db, &self.key, &own)?;
        let ids: Vec<Vec<u8>> = self.db.prepare(
            "SELECT DISTINCT o.session FROM outbox o JOIN sessions s ON s.id=o.session WHERE o.packet IS NOT NULL AND o.session>?1 AND (s.peer IS NOT NULL OR EXISTS(SELECT 1 FROM group_key_outbox g WHERE g.id=o.id) OR EXISTS(SELECT 1 FROM call_jobs c WHERE c.id=o.id)) AND s.suite=2 AND s.retired=0 ORDER BY o.session LIMIT 16"
        )?.query_map([after], |r| r.get(0))?.collect::<Result<_, _>>()?;
        let mut attempts = Vec::new();
        let mut next = Vec::new();
        for bytes in ids {
            let session: Id = bytes.try_into().map_err(|_| Error::InvalidStore)?;
            let result = self.send_pending_limit(session, now, 1);
            let stop = matches!(result, Err(Error::Network(_)));
            attempts.push(OutboundAttempt { session, result });
            next = session.to_vec();
            if stop {
                break;
            }
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if cursor(&tx, &self.key, &own)?.1 == expected {
            tx.execute("INSERT INTO outbound_cursor VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET state=excluded.state",
                [self.key.seal(&next, &binding(33, &own, b"outbound scan"))?])?;
        }
        tx.commit()?;
        Ok(attempts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        claims::tests::pair,
        incoming::tests::{start, trust},
    };
    fn reopen(path: &Path) -> ClientStore {
        ClientStore::open(
            path,
            StorageKey::new(sigil_crypto::Secret32::from_bytes([9; 32])).unwrap(),
        )
        .unwrap()
    }
    #[test]
    fn cursor_bounds_errors_includes_zero_and_survives_restart() {
        let (dir, _fixture, mut alice, mut bob, now) = pair();
        let (_, b) = trust(&mut alice, &mut bob);
        alice.prepare_peer_claim([30; 32], b).unwrap();
        alice.claim_prekey_online([30; 32], now).unwrap();
        alice
            .start_claimed_text([30; 32], [30; 32], [30; 32], "healthy queue", now, now)
            .unwrap();
        // Corrupt checkpoints are never sent, but must not monopolize a pass.
        for n in 0..17u8 {
            alice
                .db
                .execute(
                    "INSERT INTO sessions(id,revision,state,suite,peer) VALUES(?1,0,x'00',2,?2)",
                    ([n; 32].as_slice(), b.as_slice()),
                )
                .unwrap();
            alice
                .db
                .execute(
                    "INSERT INTO outbox(session,id,tag,packet) VALUES(?1,?1,x'00',x'00')",
                    [[n; 32].as_slice()],
                )
                .unwrap();
        }
        let first = alice.resume_outbound_online(now).unwrap();
        assert_eq!(first.len(), 16);
        assert_eq!(first[0].session, [0; 32]);
        assert!(first.iter().all(|a| a.result.is_err()));
        drop(alice);
        let mut alice = reopen(&dir.path().join("alice.db"));
        let second = alice.resume_outbound_online(now).unwrap();
        assert_eq!(second.len(), 2);
        assert_eq!(second[0].session, [16; 32]);
        assert_eq!(second[1].session, [30; 32]);
        assert_eq!(second[1].result.as_ref().unwrap().accepted, 1);
        assert!(alice.resume_outbound_online(now).unwrap().is_empty());
        assert_eq!(
            alice.resume_outbound_online(now).unwrap()[0].session,
            [0; 32]
        );
        alice
            .db
            .execute("UPDATE outbound_cursor SET state=zeroblob(68)", [])
            .unwrap();
        assert!(alice.resume_outbound_online(now).is_err());
        let received = bob.receive_mailbox_online(now).unwrap();
        assert_eq!(received.len(), 1);
        assert!(
            matches!(&received[0].result, Ok(MailboxEvent::Text(t)) if t.text().unwrap().body == "healthy queue")
        );
    }
    #[test]
    fn ambiguous_acceptance_retries_exact_packet_then_preserves_queue_order() {
        let (dir, _fixture, mut alice, mut bob, now) = pair();
        let (_, b) = trust(&mut alice, &mut bob);
        start(&mut alice, b, now);
        assert!(bob.sync_step_online(now).failure.is_none());
        alice
            .send_text([3; 32], [5; 32], "first queued", now, now)
            .unwrap();
        alice
            .send_text([3; 32], [6; 32], "second queued", now, now)
            .unwrap();
        let frozen =
            serde_json::to_vec(&alice.pending_deliveries([3; 32], now).unwrap()[0]).unwrap();
        alice.db.execute_batch("CREATE TRIGGER fail_receipt BEFORE UPDATE ON deliveries BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
        let step = alice.sync_step_online(now);
        assert!(step.failure.is_none());
        assert!(matches!(step.outbound[0].result, Err(Error::Storage(_))));
        assert_eq!(bob.connected_client().unwrap().mailbox().unwrap().len(), 1);
        alice
            .db
            .execute_batch("DROP TRIGGER fail_receipt;")
            .unwrap();
        drop(alice);
        let mut alice = reopen(&dir.path().join("alice.db"));
        assert_eq!(
            serde_json::to_vec(&alice.pending_deliveries([3; 32], now).unwrap()[0]).unwrap(),
            frozen
        );
        assert!(alice.resume_outbound_online(now).unwrap().is_empty());
        assert_eq!(
            alice.resume_outbound_online(now).unwrap()[0]
                .result
                .as_ref()
                .unwrap()
                .accepted,
            1
        );
        assert_eq!(bob.connected_client().unwrap().mailbox().unwrap().len(), 1);
        assert!(alice.resume_outbound_online(now).unwrap().is_empty());
        assert_eq!(
            alice.resume_outbound_online(now).unwrap()[0]
                .result
                .as_ref()
                .unwrap()
                .accepted,
            1
        );
        let received = bob.receive_mailbox_online(now).unwrap();
        let bodies: Vec<_> = received
            .iter()
            .map(|a| match &a.result {
                Ok(MailboxEvent::Text(t)) => t.text().unwrap().body,
                _ => panic!("expected received text"),
            })
            .collect();
        assert_eq!(bodies, vec!["first queued", "second queued"]);
    }

    #[test]
    fn network_failure_stops_batch_and_restart_visits_next_session() {
        let (dir, fixture, mut alice, mut bob, now) = pair();
        let (_, b) = trust(&mut alice, &mut bob);
        for n in [0u8, 1] {
            if n == 1 {
                bob.prepare_prekey_publication([81; 32], true, 3600)
                    .unwrap();
                bob.publish_prekey_online([81; 32]).unwrap();
            }
            alice.prepare_peer_claim([n; 32], b).unwrap();
            alice.claim_prekey_online([n; 32], now).unwrap();
            alice
                .start_claimed_text([n; 32], [n; 32], [n; 32], "queued initial", now, now)
                .unwrap();
        }
        // Upgrade the immediately preceding native schema with queued work intact.
        crate::test_schema::rewind(&alice.db, 36);
        alice.db.execute_batch("PRAGMA user_version=36;").unwrap();
        drop(alice);
        let mut alice = reopen(&dir.path().join("alice.db"));
        drop(fixture);
        let first = alice.resume_outbound_online(now).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].session, [0; 32]);
        assert!(matches!(first[0].result, Err(Error::Network(_))));
        drop(alice);
        let mut alice = reopen(&dir.path().join("alice.db"));
        let second = alice.resume_outbound_online(now).unwrap();
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].session, [1; 32]);
        assert!(matches!(second[0].result, Err(Error::Network(_))));
        assert_eq!(alice.pending_deliveries([0; 32], now).unwrap().len(), 1);
        assert_eq!(alice.pending_deliveries([1; 32], now).unwrap().len(), 1);
    }
}
