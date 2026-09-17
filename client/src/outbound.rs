//! Fair, bounded scheduling over existing durable packet journals.
use super::*;

pub(crate) fn recipient_full(error: &network::Error) -> bool {
    matches!(
        error,
        network::Error::Status {
            code: 507,
            retry_after_seconds: None
        }
    )
}

/// A failure that belongs to one recipient rather than to the server, so the rest of
/// the pass can still proceed and there is nothing here for the reader to act on.
pub(crate) fn recipient_specific(error: &network::Error) -> bool {
    recipient_full(error)
        || matches!(error, network::Error::Status { code: 404 | 507, .. })
}
pub(crate) fn recipient_unavailable(error: &network::Error) -> bool {
    recipient_full(error) || recipient_unknown(error)
}

/// The server has no such recipient device. Unlike a full or unreachable recipient
/// this does not resolve on its own: a device record is not restored once it is gone.
pub(crate) fn recipient_unknown(error: &network::Error) -> bool {
    matches!(
        error,
        network::Error::Status {
            code: 404,
            retry_after_seconds: None
        }
    )
}

/// How long a recipient must keep answering that it does not exist before its queued
/// packets are given up on. A 404 is the server's statement about whether the device
/// record exists, not about whether it is reachable, so this needs to outlast a
/// deployment blip rather than an offline phone.
const UNKNOWN_RECIPIENT_GRACE: u64 = 3600;

/// Record the cancellation of every queued packet in a session, then release them.
pub(crate) fn cancel_queued(tx: &Transaction<'_>, key: &StorageKey, session: &Id) -> Result<(), Error> {
    let ids: Vec<Vec<u8>> = tx
        .prepare("SELECT id FROM outbox WHERE session=?1 AND packet IS NOT NULL")?
        .query_map([session.as_slice()], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    for raw in ids {
        let id: Id = raw.try_into().map_err(|_| Error::InvalidStore)?;
        conversations::mark_cancelled(tx, key, session, &id)?;
    }
    tx.execute(
        "UPDATE outbox SET packet=NULL WHERE session=?1 AND packet IS NOT NULL",
        [session.as_slice()],
    )?;
    Ok(())
}

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
    /// Attempt up to four queued packets from each of at most 16 sessions. Queue order
    /// within each session is preserved. Local errors do not starve other sessions;
    /// unavailable/full recipients leave other sessions running; transport errors
    /// stop the pass so callers can honor Retry-After. Cursor wrap
    /// takes an empty pass. Unprepared packets require application intervention.
    pub fn resume_outbound_online(&mut self, now: u64) -> Result<Vec<OutboundAttempt>, Error> {
        if now == 0 || now > i64::MAX as u64 {
            return Err(Error::Expired);
        }
        self.connected_client()?;
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let (after, expected) = cursor(&self.db, &self.key, &own)?;
        // Sessions whose recipient answered full/unknown wait out their backoff before retrying.
        let query = "SELECT DISTINCT o.session FROM outbox o JOIN sessions s ON s.id=o.session WHERE o.packet IS NOT NULL AND o.session>?1 AND (s.peer IS NOT NULL OR EXISTS(SELECT 1 FROM group_key_outbox g WHERE g.id=o.id) OR EXISTS(SELECT 1 FROM call_jobs c WHERE c.id=o.id)) AND s.suite=2 AND s.retired=0 AND NOT EXISTS(SELECT 1 FROM outbound_backoff b WHERE b.session=o.session AND b.until>?2) ORDER BY o.session LIMIT 16";
        let mut ids: Vec<Vec<u8>> = self.db.prepare(query)?.query_map((&after, now as i64), |r| r.get(0))?.collect::<Result<_, _>>()?;
        if ids.is_empty() && !after.is_empty() {
            // Wrap within the pass so a fresh packet never waits for the next one.
            ids = self.db.prepare(query)?.query_map((Vec::new(), now as i64), |r| r.get(0))?.collect::<Result<_, _>>()?;
        }
        // Up to four rounds; each round submits one packet per session concurrently, so
        // per-session order holds while sessions no longer wait on each other.
        struct Lane {
            session: Id,
            progress: SendProgress,
            error: Option<Error>,
            done: bool,
        }
        let mut lanes: Vec<Lane> = ids
            .into_iter()
            .map(|bytes| {
                Ok(Lane {
                    session: bytes.try_into().map_err(|_| Error::InvalidStore)?,
                    progress: SendProgress { accepted: 0, expired: 0 },
                    error: None,
                    done: false,
                })
            })
            .collect::<Result<_, Error>>()?;
        let network = self.connected_client()?;
        for _ in 0..4 {
            let mut batch = Vec::new();
            for index in 0..lanes.len() {
                if lanes[index].done {
                    continue;
                }
                let session = lanes[index].session;
                match self.prepare_outgoing(session, now) {
                    Ok((prepared, expired)) => {
                        lanes[index].progress.expired += expired;
                        match prepared {
                            crate::connection::Prepared::Request(id, request, silent) => batch.push((index, id, request, silent)),
                            crate::connection::Prepared::Accepted(id, receipt) => match self.acknowledge_sent(session, id, &receipt) {
                                Ok(()) => lanes[index].progress.accepted += 1,
                                Err(error) => { lanes[index].error = Some(error); lanes[index].done = true; }
                            },
                            crate::connection::Prepared::Pending | crate::connection::Prepared::Empty => lanes[index].done = true,
                        }
                    }
                    Err(error) => { lanes[index].error = Some(error); lanes[index].done = true; }
                }
            }
            if batch.is_empty() {
                break;
            }
            let results = network.submit_many(&batch.iter().map(|(_, _, request, silent)| (request, *silent)).collect::<Vec<_>>());
            let mut transport_failure = false;
            for ((index, id, _, _), result) in batch.into_iter().zip(results) {
                let session = lanes[index].session;
                match result.map_err(Error::from).and_then(|receipt| self.acknowledge_sent(session, id, &receipt)) {
                    Ok(()) => lanes[index].progress.accepted += 1,
                    Err(error) => {
                        transport_failure |= matches!(&error, Error::Network(e) if !recipient_unavailable(e));
                        lanes[index].error = Some(error);
                        lanes[index].done = true;
                    }
                }
            }
            if transport_failure {
                break;
            }
        }
        let mut attempts = Vec::new();
        let mut next = Vec::new();
        for lane in lanes {
            let mut result = match lane.error {
                Some(error) => Err(error),
                None => Ok(lane.progress),
            };
            // A replaced peer never regains trust, so its packets can never leave.
            if matches!(result, Err(Error::Unprepared)) && self.session_peer_replaced(lane.session)? {
                self.release_replaced_peer_session(lane.session)?;
                result = Err(Error::Obsolete);
            }
            // Full, unknown and not-yet-trusted recipients wait instead of costing every pass;
            // granting trust clears the wait.
            let unknown = matches!(&result, Err(Error::Network(error)) if recipient_unknown(error));
            let backoff = match &result {
                Err(Error::Network(error)) if recipient_full(error) => 60,
                Err(Error::Network(error)) if recipient_unavailable(error) => 300,
                Err(Error::Unprepared) => 60,
                _ => 0,
            };
            if backoff > 0 {
                // Keep the first refusal's timestamp so a run of them can be measured.
                self.db.execute(
                    "INSERT INTO outbound_backoff VALUES(?1,?2,?3,1) ON CONFLICT(session) DO UPDATE SET until=excluded.until, attempts=outbound_backoff.attempts+1, since=CASE WHEN ?4 AND outbound_backoff.since>0 THEN outbound_backoff.since ELSE excluded.since END",
                    (
                        lane.session.as_slice(),
                        (now + backoff) as i64,
                        if unknown { now as i64 } else { 0 },
                        unknown,
                    ),
                )?;
            } else {
                self.db.execute(
                    "DELETE FROM outbound_backoff WHERE session=?1",
                    [lane.session.as_slice()],
                )?;
            }
            if unknown {
                self.abandon_unknown_recipient(lane.session, now)?;
            }
            next = lane.session.to_vec();
            attempts.push(OutboundAttempt { session: lane.session, result });
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

    /// Whether the session's peer record carries an approved, irreversible replacement.
    fn session_peer_replaced(&self, session: Id) -> Result<bool, Error> {
        let Some(peer) = session_peer(&self.db, &session)? else {
            return Ok(false);
        };
        match peers::known(&self.db, &self.key, &peer) {
            Ok(known) => Ok(known.replaced_by.is_some()),
            Err(Error::NotFound) => Ok(false),
            Err(error) => Err(error),
        }
    }
    /// Packets sealed to a replaced device can never be read by its replacement:
    /// cancel them, drop the wait and retire the session so it frees its peer slot.
    fn release_replaced_peer_session(&mut self, session: Id) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        cancel_queued(&tx, &self.key, &session)?;
        tx.execute(
            "DELETE FROM outbound_backoff WHERE session=?1",
            [session.as_slice()],
        )?;
        match retirement::retire(&tx, &self.key, session) {
            Ok(()) | Err(Error::NotFound | Error::Conflict | Error::UnsupportedSession) => {}
            Err(error) => return Err(error),
        }
        tx.commit()?;
        Ok(())
    }
    /// Give up on packets addressed to a device the server has kept refusing as
    /// unknown. They can never be delivered, and until they are released they hold a
    /// session at its queue limit, which refuses every later message to that peer.
    fn abandon_unknown_recipient(&mut self, session: Id, now: u64) -> Result<(), Error> {
        let since: Option<i64> = self
            .db
            .query_row(
                "SELECT since FROM outbound_backoff WHERE session=?1 AND since>0",
                [session.as_slice()],
                |r| r.get(0),
            )
            .optional()?;
        let Some(since) = since else { return Ok(()) };
        if now.saturating_sub(since.max(0) as u64) < UNKNOWN_RECIPIENT_GRACE {
            return Ok(());
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // The release is already decided; the marker only makes it visible.
        cancel_queued(&tx, &self.key, &session)?;
        tx.execute(
            "DELETE FROM outbound_backoff WHERE session=?1",
            [session.as_slice()],
        )?;
        tx.commit()?;
        // A session whose recipient is gone should not hold one of the peer's slots.
        match self.retire_session(session) {
            Ok(()) | Err(Error::NotFound | Error::Conflict | Error::UnsupportedSession) => Ok(()),
            Err(error) => Err(error),
        }
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
    fn full_recipient_does_not_stall_another_queue_or_discard_packets() {
        recipient_failure(false)
    }
    #[test]
    fn revoked_recipient_does_not_back_off_other_delivery_or_receiving() {
        recipient_failure(true)
    }
    /// A device the server has forgotten never comes back, so its queued packets must
    /// be released rather than holding the session at its limit forever.
    #[test]
    fn a_recipient_the_server_keeps_refusing_stops_holding_the_queue() {
        use crate::connection::tests::credential;
        let (dir, _fixture, mut alice, mut bob, now) = pair();
        let mut server = sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap();
        let (_, b) = trust(&mut alice, &mut bob);
        alice.prepare_peer_claim([0; 32], b).unwrap();
        alice.claim_prekey_online([0; 32], now).unwrap();
        alice
            .start_claimed_text([0; 32], [0; 32], [0; 32], "queued", now, now)
            .unwrap();
        let recipient = bob.connection_session().unwrap().unwrap().device_id;
        server
            .revoke_device(&credential(&bob), &recipient, now)
            .unwrap();
        fn queued(store: &ClientStore) -> i64 {
            store
                .db
                .query_row(
                    "SELECT count(*) FROM outbox WHERE packet IS NOT NULL",
                    [],
                    |r| r.get(0),
                )
                .unwrap()
        }
        assert_eq!(queued(&alice), 1);
        // Inside the grace window the packet is kept: a refusal may be a server blip.
        for attempt in alice.resume_outbound_online(now).unwrap() {
            assert!(attempt.result.is_err());
        }
        assert_eq!(queued(&alice), 1);
        for attempt in alice
            .resume_outbound_online(now + UNKNOWN_RECIPIENT_GRACE - 1)
            .unwrap()
        {
            assert!(attempt.result.is_err());
        }
        assert_eq!(queued(&alice), 1);
        // Past it the packet is released and the dead session frees its peer slot.
        for attempt in alice
            .resume_outbound_online(now + UNKNOWN_RECIPIENT_GRACE + 400)
            .unwrap()
        {
            assert!(attempt.result.is_err());
        }
        assert_eq!(queued(&alice), 0);
        assert_eq!(
            alice
                .db
                .query_row("SELECT count(*) FROM outbound_backoff", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        // The release is recorded, so the message reads as cancelled rather than sending.
        assert!(conversations::cancelled(&alice.db, &alice.key, &[0; 32], &[0; 32]).unwrap());
    }
    /// A replaced device never regains trust, so packets sealed to it are cancelled
    /// and the dead session frees its slot instead of waiting out a backoff forever.
    #[test]
    fn packets_to_a_replaced_peer_are_cancelled_and_the_session_retired() {
        use sigil_crypto::IdentityKey;
        use sigil_protocol::conversation::{Action, Body};
        let (_dir, _fixture, mut alice, mut bob, now) = pair();
        let old_bytes = bob.own_device_binding().unwrap();
        let (_, old) = trust(&mut alice, &mut bob);
        let old_fingerprint = device_fingerprint(&old_bytes).unwrap();
        start(&mut alice, old, now);
        let op = alice
            .conversation_operation(
                [115; 32],
                Action::Post {
                    body: Body::Text("queued".into()),
                    reply: None,
                    thread: None,
                    expires_at: None,
                    view_once: false,
                },
            )
            .unwrap();
        alice.queue_peer_operation(old, &op, now, now).unwrap();
        let sends = alice.resume_send_intents_online(now).unwrap();
        assert_eq!(sends[0].result.as_ref().unwrap(), &[3; 32]);
        let key = IdentityKey::generate().unwrap();
        let mut replacement = crate::peers::parse(&old_bytes).unwrap();
        replacement.binding.device = [90; 32];
        replacement.binding.identity = key.public_key();
        replacement.signature = key
            .sign(&replacement.binding.signing_bytes().unwrap())
            .unwrap();
        let new = alice
            .observe_peer_binding(&replacement.to_bytes().unwrap())
            .unwrap();
        alice
            .approve_peer_replacement(old, new.id, old_fingerprint, new.fingerprint)
            .unwrap();
        // Approval itself released the packet and retired the session; nothing is left to walk.
        assert!(alice.resume_outbound_online(now).unwrap().is_empty());
        let count = |sql: &str| -> i64 { alice.db.query_row(sql, [], |r| r.get(0)).unwrap() };
        assert_eq!(count("SELECT count(*) FROM outbox WHERE packet IS NOT NULL"), 0);
        assert_eq!(count("SELECT count(*) FROM outbound_backoff"), 0);
        assert_eq!(
            count("SELECT retired FROM sessions WHERE id=x'0303030303030303030303030303030303030303030303030303030303030303'"),
            1
        );
        assert_eq!(
            alice.operation_delivery_state(old, op.id).unwrap(),
            conversations::DeliveryState::Cancelled
        );
        // Nothing is left for the next pass to walk.
        assert!(alice.resume_outbound_online(now).unwrap().is_empty());
    }

    fn recipient_failure(revoked: bool) {
        use crate::connection::tests::{credential, prepare};
        use sigil_protocol::{accounts::InviteRequest, mailbox::Submit};
        let (dir, fixture, mut alice, mut bob, now) = pair();
        let mut server = sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap();
        let invite = server
            .invite(
                InviteRequest {
                    username: "charlie".into(),
                    expires_in_seconds: 60,
                },
                now,
            )
            .unwrap();
        let mut charlie = reopen(&dir.path().join("charlie.db"));
        prepare(&mut charlie, &fixture, &invite.secret);
        charlie.enroll_online().unwrap();
        server
            .allow_sender(
                &credential(&charlie),
                &alice.connection_session().unwrap().unwrap().device_id,
                now,
            )
            .unwrap();
        charlie
            .prepare_prekey_publication([80; 32], true, 3600)
            .unwrap();
        charlie.publish_prekey_online([80; 32]).unwrap();
        let (_, b) = trust(&mut alice, &mut bob);
        let (_, c) = trust(&mut alice, &mut charlie);
        for (n, peer) in [(0u8, b), (1, c)] {
            alice.prepare_peer_claim([n; 32], peer).unwrap();
            alice.claim_prekey_online([n; 32], now).unwrap();
            alice
                .start_claimed_text([n; 32], [n; 32], [n; 32], "queued", now, now)
                .unwrap();
        }
        let recipient = bob.connection_session().unwrap().unwrap().device_id;
        let mut receipts = Vec::new();
        if revoked { server.revoke_device(&credential(&bob), &recipient, now).unwrap(); }
        for n in if revoked {64..64u8} else {64..128u8} {
            receipts.push(
                server
                    .submit_message(
                        &credential(&alice),
                        Submit {
                            recipient_device: recipient.clone(),
                            message_id: transport::hex(&[n; 32]),
                            payload: "11".repeat(32),
                            expires_at: now + 3600,
                        },
                        now,
                    )
                    .unwrap(),
            );
        }
        let frozen = serde_json::to_vec(&alice.pending_deliveries([0; 32], now).unwrap()).unwrap();
        let attempts = alice.resume_outbound_online(now).unwrap();
        assert_eq!(attempts.len(), 2);
        assert!(matches!(&attempts[0].result,Err(Error::Network(error)) if recipient_unavailable(error)));
        assert_eq!(attempts[1].result.as_ref().unwrap().accepted, 1);
        assert_eq!(
            charlie.connected_client().unwrap().mailbox().unwrap().len(),
            1
        );
        assert_eq!(
            serde_json::to_vec(&alice.pending_deliveries([0; 32], now).unwrap()).unwrap(),
            frozen
        );
        // The full recipient waits out its backoff; the next pass past it retries.
        let mut step = alice.sync_step_online(now + if revoked { 301 } else { 61 });
        assert!(
            step.failure.is_none(),
            "A recipient limit must not become a server outage"
        );
        assert!(
            step.issue().is_none(),
            "Queued delivery must not flash a global sync warning"
        );
        assert!(step.maintenance.is_some());
        assert!(step
            .outbound
            .iter()
            .any(|a| matches!(&a.result,Err(Error::Network(e)) if recipient_unavailable(e))));
        step.incoming.push(IncomingAttempt { sequence: 1, result: Err(Error::Conflict), recovery: RecoveryAdvice::None, kind: String::new() });
        assert!(matches!(
            step.issue(),
            Some(("receiving messages", Error::Conflict))
        ));
        if revoked {
            assert_eq!(serde_json::to_vec(&alice.pending_deliveries([0; 32], now).unwrap()).unwrap(), frozen);
            return;
        }
        for receipt in receipts {
            server
                .acknowledge_message(&credential(&bob), receipt.sequence, now)
                .unwrap();
        }
        assert_eq!(
            alice.resume_outbound_online(now + 122).unwrap()[0]
                .result
                .as_ref()
                .unwrap()
                .accepted,
            1
        );
        assert!(bob
            .receive_mailbox_online(now)
            .unwrap()
            .iter()
            .all(|a| a.result.is_ok()));
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
        assert_eq!(
            alice.resume_outbound_online(now).unwrap()[0]
                .result
                .as_ref()
                .unwrap()
                .accepted,
            2
        );
        assert_eq!(bob.connected_client().unwrap().mailbox().unwrap().len(), 2);
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
        // Sessions submit concurrently, so a transport failure surfaces on every lane of the round.
        let first = alice.resume_outbound_online(now).unwrap();
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].session, [0; 32]);
        assert_eq!(first[1].session, [1; 32]);
        assert!(first.iter().all(|a| matches!(a.result, Err(Error::Network(_)))));
        drop(alice);
        let mut alice = reopen(&dir.path().join("alice.db"));
        let second = alice.resume_outbound_online(now).unwrap();
        assert_eq!(second.len(), 2);
        assert!(second.iter().all(|a| matches!(a.result, Err(Error::Network(_)))));
        assert_eq!(alice.pending_deliveries([0; 32], now).unwrap().len(), 1);
        assert_eq!(alice.pending_deliveries([1; 32], now).unwrap().len(), 1);
    }
}
