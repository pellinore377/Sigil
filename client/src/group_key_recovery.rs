//! Bounded same-epoch recovery through fresh membership-authorized PQXDH channels.
use super::*;
use crate::{claims, ClientStore};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sigil_crypto::{sender_keys as sk, storage::StorageKey};
use zeroize::Zeroizing;
pub(crate) const MIGRATION: &str = "CREATE TABLE group_key_recovery(id BLOB PRIMARY KEY,group_id BLOB NOT NULL REFERENCES groups(id),state BLOB NOT NULL); CREATE INDEX group_key_recovery_group ON group_key_recovery(group_id); PRAGMA user_version=60;";
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Work {
    incoming: Option<Id>,
    repaired: Option<Id>,
    reply: Option<Id>,
    request: Option<Id>,
    request_after: u64,
    repair_after: u64,
    forced: bool,
}
fn index(key: &StorageKey, own: &Id, group: &Id, peer: &Id) -> Result<Id, Error> {
    Ok(key.commitment(
        &[own.as_slice(), group, peer].concat(),
        b"Sigil/group-key-recovery-index/v0",
    )?)
}
fn aad(own: &Id, group: &Id, peer: &Id) -> Vec<u8> {
    [b"Sigil/group-key-recovery/v0".as_slice(), own, group, peer].concat()
}
fn load(db: &Connection, key: &StorageKey, own: &Id, group: &Id, peer: &Id) -> Result<Work, Error> {
    let id = index(key, own, group, peer)?;
    let row:Option<(Vec<u8>,Vec<u8>)>=db.query_row("SELECT group_id,CASE WHEN length(state)<=4096 THEN state END FROM group_key_recovery WHERE id=?1",[id.as_slice()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    match row {
        None => Ok(Work::default()),
        Some((g, raw)) => {
            if g != *group {
                return Err(Error::InvalidStore);
            }
            serde_json::from_slice(&key.open(&raw, &aad(own, group, peer))?)
                .map_err(|_| Error::InvalidStore)
        }
    }
}
fn save(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    group: &Id,
    peer: &Id,
    work: &Work,
) -> Result<(), Error> {
    let id = index(key, own, group, peer)?;
    let bytes = serde_json::to_vec(work).map_err(|_| Error::InvalidStore)?;
    tx.execute("INSERT INTO group_key_recovery VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET state=excluded.state",(id.as_slice(),group.as_slice(),key.seal(&bytes,&aad(own,group,peer))?))?;
    Ok(())
}
pub(super) fn requested(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    state: &State,
    peer: &peers::Peer,
    message: Id,
) -> Result<(), Error> {
    let mut work = load(tx, key, own, &state.group, &peer.fingerprint)?;
    work.incoming = Some(message);
    save(tx, key, own, &state.group, &peer.fingerprint, &work)
}
pub(super) fn forget_job(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    group: &Id,
    id: Option<Id>,
) -> Result<(), Error> {
    if let Some(id) = id {
        if let Some((session, receipt)) = keys::job(tx, key, own, group, &id)? {
            if receipt.kind == 0 {
                return Err(Error::InvalidStore);
            }
            tx.execute(
                "UPDATE outbox SET packet=NULL WHERE session=?1 AND id=?2",
                (session.as_slice(), id.as_slice()),
            )?;
            tx.execute("DELETE FROM group_key_outbox WHERE id=?1", [id.as_slice()])?;
        }
    }
    Ok(())
}
pub(crate) fn cancelled_packet(
    tx: &Transaction<'_>,
    key: &StorageKey,
    session: &Id,
    id: &Id,
) -> Result<bool, Error> {
    let raw: Option<Vec<u8>> = tx
        .query_row(
            "SELECT content FROM outbox WHERE session=?1 AND id=?2 AND length(content)=276",
            (session.as_slice(), id.as_slice()),
            |r| r.get(0),
        )
        .optional()?;
    let Some(raw) = raw else {
        return Ok(false);
    };
    let raw = key.open(&raw, &crate::binding(9, session, id))?;
    let Some(receipt) = distribution_receipt(&raw)? else {
        return Ok(false);
    };
    let own = device_fingerprint(&peers::own(tx, key)?)?;
    if receipt.message != *id || receipt.context.sender != own {
        return Err(Error::InvalidStore);
    }
    let status = store::load(tx, key, &own, &receipt.context.group)?;
    if status.frozen || status.state.closed || status.state.head != receipt.context.state {
        return Ok(true);
    }
    let work = load(tx, key, &own, &receipt.context.group, &receipt.recipient)?;
    if receipt.kind == 3 {
        return history::cancelled(tx, key, &own, &receipt);
    }
    Ok(match receipt.kind {
        1 => work.reply.is_some_and(|v| v != *id),
        2 => work.request.is_some_and(|v| v != *id),
        _ => false,
    })
}
pub(super) fn distribution_job(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    context: &sk::Context,
    target: &Id,
) -> Result<(Id, Id, DistributionReceipt), Error> {
    let work = load(tx, key, own, &context.group, target)?;
    let id = work
        .reply
        .unwrap_or_else(|| control::message_id(context, target));
    let (session, receipt) =
        keys::job(tx, key, own, &context.group, &id)?.ok_or(Error::Unprepared)?;
    if receipt.kind > 1 || receipt.context != *context || receipt.recipient != *target {
        return Err(Error::InvalidStore);
    }
    Ok((id, session, receipt))
}
fn context(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    state: &State,
    kind: u8,
) -> Result<sk::Context, Error> {
    if kind == 2 {
        return Ok(sk::Context {
            group: state.group,
            state: state.head,
            epoch: state.epoch,
            sender: *own,
            chain: [0; 32],
        });
    }
    let (sender, ready, _) = keys::load_sender(tx, key, own, state)?.ok_or(Error::Unprepared)?;
    if !ready {
        return Err(Error::Unprepared);
    }
    Ok(sender.context())
}
impl ClientStore {
    fn queue_key_recovery_online(
        &mut self,
        group: Id,
        peer: Id,
        request: Id,
        kind: u8,
        now: u64,
    ) -> Result<Id, Error> {
        self.refresh_group_authority_for_send(group, now)?;
        let binding = self.own_device_binding()?;
        let own = device_fingerprint(&binding)?;
        let identity = self.identity()?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = keys::current(&tx, &self.key, &own, &group)?;
        let known = keys::authorized_peer(&tx, &self.key, &state, &peer)?;
        if known.fingerprint == own {
            return Err(Error::Unprepared);
        }
        let context = context(&tx, &self.key, &own, &state, kind)?;
        let message = if kind == 1 {
            control::repair_id(&context, &known.fingerprint, &request)
        } else {
            control::parse_request(&control::request_wire(
                &context,
                &known.fingerprint,
                &request,
            ))?
            .message
        };
        if keys::job(&tx, &self.key, &own, &group, &message)?.is_some() {
            return Ok(message);
        }
        let claim = digest(b"Sigil/group-key-recovery-claim/v0", &[&own, &message]);
        claims::prepare(
            &tx,
            &self.key,
            &identity,
            claim,
            known.binding.device,
            known.binding.identity,
            (None, Some(&known.binding.server)),
        )?;
        tx.commit()?;
        self.claim_prekey_online(claim, now)?;
        self.refresh_group_authority_for_send(group, now)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = keys::current(&tx, &self.key, &own, &group)?;
        let known = keys::authorized_peer(&tx, &self.key, &state, &peer)?;
        if self::context(&tx, &self.key, &own, &state, kind)? != context {
            return Err(Error::Obsolete);
        }
        if keys::job(&tx, &self.key, &own, &group, &message)?.is_some() {
            return Ok(message);
        }
        let bytes = if kind == 1 {
            let (sender, _, _) =
                keys::load_sender(&tx, &self.key, &own, &state)?.ok_or(Error::Unprepared)?;
            messages::cancel_before(
                &tx,
                &self.key,
                &own,
                &group,
                &known.fingerprint,
                sender.counter(),
            )?;
            control::repair_wire(
                &sender.current_distribution()?,
                &known.fingerprint,
                &request,
            )?
        } else {
            Zeroizing::new(control::request_wire(
                &context,
                &known.fingerprint,
                &request,
            ))
        };
        let session = digest(b"Sigil/group-key-recovery-session/v0", &[&own, &message]);
        claims::start_in(
            &tx,
            &self.key,
            claim,
            session,
            message,
            (&bytes, None, None),
            now,
        )?;
        keys::mark_channel(&tx, &self.key, &own, &session, &group, &known.fingerprint)?;
        let mut record = session.to_vec();
        record.extend_from_slice(&retained_payload(&self.key, &bytes)?);
        let sealed = self.key.seal(
            &record,
            &crate::binding(48, &group, &[own.as_slice(), &message].concat()),
        )?;
        let mut work = load(&tx, &self.key, &own, &group, &known.fingerprint)?;
        let old = if kind == 1 {
            work.reply.replace(message)
        } else {
            work.request.replace(message)
        };
        forget_job(&tx, &self.key, &own, &group, old)?;
        tx.execute(
            "INSERT INTO group_key_outbox VALUES(?1,?2,?3)",
            (message.as_slice(), group.as_slice(), sealed),
        )?;
        if kind == 1 {
            work.repaired = Some(request);
            work.repair_after = now.saturating_add(300);
        } else {
            work.request_after = now.saturating_add(300);
        }
        save(&tx, &self.key, &own, &group, &known.fingerprint, &work)?;
        tx.commit()?;
        Ok(message)
    }
    /// Requests a current chain position; messages before that position require history sharing.
    pub fn request_group_key_recovery(
        &mut self,
        group: Id,
        peer: Id,
        now: u64,
    ) -> Result<(), Error> {
        if now == 0 || now > i64::MAX as u64 {
            return Err(Error::Expired);
        }
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = keys::current(&tx, &self.key, &own, &group)?;
        let known = keys::authorized_peer(&tx, &self.key, &state, &peer)?;
        if known.fingerprint == own {
            return Err(Error::Unprepared);
        }
        let mut work = load(&tx, &self.key, &own, &group, &known.fingerprint)?;
        work.forced = true;
        if work.request.is_none() {
            work.request_after = now;
        }
        save(&tx, &self.key, &own, &group, &known.fingerprint, &work)?;
        tx.commit()?;
        Ok(())
    }
    pub(super) fn group_key_recovery_work_online(
        &mut self,
        group: Id,
        peer: Id,
        now: u64,
    ) -> Result<(), Error> {
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = keys::current(&tx, &self.key, &own, &group)?;
        let known = keys::authorized_peer(&tx, &self.key, &state, &peer)?;
        let mut work = load(&tx, &self.key, &own, &group, &known.fingerprint)?;
        let missing = keys::receiver(&tx, &self.key, &own, &state, &known.fingerprint)?.is_none();
        if (missing || work.forced) && work.request_after == 0 {
            work.request_after = now.saturating_add(60);
        }
        let request = (missing || work.forced) && work.request_after <= now;
        let repair = work
            .incoming
            .filter(|v| Some(*v) != work.repaired && now >= work.repair_after);
        save(&tx, &self.key, &own, &group, &known.fingerprint, &work)?;
        tx.commit()?;
        if let Some(id) = repair {
            self.queue_key_recovery_online(group, peer, id, 1, now)?;
        }
        if request {
            let nonce = digest(
                b"Sigil/group-key-request-attempt/v0",
                &[
                    &own,
                    &state.head,
                    &known.fingerprint,
                    &work.request_after.to_be_bytes(),
                    &work.request.unwrap_or([0; 32]),
                ],
            );
            self.queue_key_recovery_online(group, peer, nonce, 2, now)?;
        }
        Ok(())
    }
}
pub(super) fn received(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    group: &Id,
    peer: &Id,
) -> Result<(), Error> {
    let mut work = load(tx, key, own, group, peer)?;
    work.forced = false;
    save(tx, key, own, group, peer, &work)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn open(path: &std::path::Path) -> ClientStore {
        ClientStore::open(
            path,
            StorageKey::new(sigil_crypto::Secret32::from_bytes([9; 32])).unwrap(),
        )
        .unwrap()
    }
    #[track_caller]
    fn deliver(source: &mut ClientStore, target: &mut ClientStore, now: u64) {
        for _ in 0..2 {
            for attempt in source.resume_outbound_online(now).unwrap() {
                assert!(attempt.result.is_ok(), "{:?}", attempt.result);
            }
        }
        for attempt in target.receive_mailbox_online(now).unwrap() {
            if let Err(error) = attempt.result {
                panic!("{error:?}");
            }
        }
        target.acknowledge_incoming_online().unwrap();
    }
    #[test]
    fn missing_and_expired_distributions_recover_with_fresh_channels_and_atomic_cutover() {
        for expired in [false, true] {
            let (dir, _fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
            alice.publish_device_binding_online().unwrap();
            bob.publish_device_binding_online().unwrap();
            alice
                .prepare_prekey_publication([110; 32], true, 3600)
                .unwrap();
            alice.publish_prekey_online([110; 32]).unwrap();
            let (profile, authority) = service::tests::enable(&dir.path().join("server.db"));
            let group = alice.create_group(authority).unwrap();
            let (a, b) = crate::incoming::tests::trust(&mut alice, &mut bob);
            bob.accept_group_genesis(a, &alice.group_genesis(group).unwrap())
                .unwrap();
            for client in [&mut alice, &mut bob] {
                client
                    .pin_group_service(group, &profile, Zeroizing::new([111; 32]))
                    .unwrap();
            }
            alice.prepare_group_service_request(group, None).unwrap();
            alice.submit_group_service_online(group, now).unwrap();
            let add = alice
                .prepare_group_change(
                    group,
                    Change::Add(
                        Member::new(
                            [112; 32],
                            Role::Member,
                            &[bob.own_device_binding().unwrap()],
                        )
                        .unwrap(),
                    ),
                )
                .unwrap();
            let add = bob.approve_group_proposal(group, &add).unwrap();
            alice
                .prepare_group_service_request(group, Some(&add))
                .unwrap();
            alice.submit_group_service_online(group, now).unwrap();
            bob.sync_group_service_online(group, now).unwrap();
            alice.db.execute("DELETE FROM peers", []).unwrap();
            bob.db.execute("DELETE FROM peers", []).unwrap();
            alice
                .observe_peer_binding(&bob.own_device_binding().unwrap())
                .unwrap();
            bob.observe_peer_binding(&alice.own_device_binding().unwrap())
                .unwrap();
            let original = alice
                .prepare_group_distribution_online(group, b, now)
                .unwrap();
            bob.prepare_group_distribution_online(group, a, now)
                .unwrap();
            for client in [&mut alice, &mut bob] {
                client
                    .prepare_prekey_publication([115; 32], true, 3600)
                    .unwrap();
                client.publish_prekey_online([115; 32]).unwrap();
            }
            deliver(&mut bob, &mut alice, now);
            let afp = device_fingerprint(&alice.own_device_binding().unwrap()).unwrap();
            let bfp = device_fingerprint(&bob.own_device_binding().unwrap()).unwrap();
            assert!(!bob.group_receiver_ready(group, afp).unwrap());
            assert!(alice
                .db
                .query_row("SELECT seed IS NULL FROM group_senders", [], |r| r
                    .get::<_, bool>(0))
                .unwrap());
            alice
                .queue_group_text(group, [113; 32], "pending before key recovery", now, now)
                .unwrap();
            if expired {
                let tx = alice.db.transaction().unwrap();
                let (session, _) = keys::job(&tx, &alice.key, &afp, &group, &original)
                    .unwrap()
                    .unwrap();
                let raw: Vec<u8> = tx
                    .query_row(
                        "SELECT metadata FROM deliveries WHERE id=?1",
                        [original.as_slice()],
                        |r| r.get(0),
                    )
                    .unwrap();
                let mut raw = alice
                    .key
                    .open(&raw, &crate::binding(7, &session, &original))
                    .unwrap();
                raw[32..].copy_from_slice(&(now - 1).to_be_bytes());
                tx.execute(
                    "UPDATE deliveries SET metadata=?1 WHERE id=?2",
                    (
                        alice
                            .key
                            .seal(&raw, &crate::binding(7, &session, &original))
                            .unwrap(),
                        original.as_slice(),
                    ),
                )
                .unwrap();
                tx.commit().unwrap();
            }
            if expired {
                bob.group_key_recovery_work_online(group, a, now - 60)
                    .unwrap();
                assert!(load(&bob.db, &bob.key, &bfp, &group, &afp)
                    .unwrap()
                    .request
                    .is_none());
            } else {
                bob.request_group_key_recovery(group, a, now).unwrap();
            }
            bob.group_key_recovery_work_online(group, a, now).unwrap();
            deliver(&mut bob, &mut alice, now);
            assert!(load(&alice.db, &alice.key, &afp, &group, &bfp)
                .unwrap()
                .incoming
                .is_some());
            alice.db.execute_batch("CREATE TRIGGER fail_repair BEFORE INSERT ON group_key_outbox BEGIN SELECT RAISE(ABORT,'synthetic recovery queue failure'); END;").unwrap();
            assert!(alice.group_key_recovery_work_online(group, b, now).is_err());
            assert!(load(&alice.db, &alice.key, &afp, &group, &bfp)
                .unwrap()
                .reply
                .is_none());
            assert_eq!(
                alice
                    .db
                    .query_row(
                        "SELECT count(*) FROM group_delivery WHERE status=0",
                        [],
                        |r| r.get::<_, u32>(0)
                    )
                    .unwrap(),
                1
            );
            alice.db.execute_batch("DROP TRIGGER fail_repair").unwrap();
            drop(alice);
            let mut alice = open(&dir.path().join("alice.db"));
            alice.group_key_recovery_work_online(group, b, now).unwrap();
            assert_eq!(
                alice
                    .db
                    .query_row(
                        "SELECT count(*) FROM group_delivery WHERE status=3",
                        [],
                        |r| r.get::<_, u32>(0)
                    )
                    .unwrap(),
                1
            );
            let work = load(&alice.db, &alice.key, &afp, &group, &bfp).unwrap();
            let repaired = work.reply.unwrap();
            let tx = alice.db.transaction().unwrap();
            let (session, _) = keys::job(&tx, &alice.key, &afp, &group, &repaired)
                .unwrap()
                .unwrap();
            tx.commit().unwrap();
            let before: Vec<u8> = alice
                .db
                .query_row(
                    "SELECT packet FROM outbox WHERE id=?1",
                    [repaired.as_slice()],
                    |r| r.get(0),
                )
                .unwrap();
            alice.group_key_recovery_work_online(group, b, now).unwrap();
            assert_eq!(
                before,
                alice
                    .db
                    .query_row(
                        "SELECT packet FROM outbox WHERE id=?1",
                        [repaired.as_slice()],
                        |r| r.get::<_, Vec<u8>>(0)
                    )
                    .unwrap()
            );
            alice.send_pending_online(session, now).unwrap();
            bob.db.execute_batch("CREATE TRIGGER fail_receiver BEFORE INSERT ON group_receivers BEGIN SELECT RAISE(ABORT,'synthetic receiver failure'); END;").unwrap();
            let packet = crate::incoming::tests::next(&bob);
            assert!(bob.accept_delivery_online(&packet, now).is_err());
            assert!(!bob.group_receiver_ready(group, afp).unwrap());
            bob.db.execute_batch("DROP TRIGGER fail_receiver").unwrap();
            drop(bob);
            let mut bob = open(&dir.path().join("bob.db"));
            bob.accept_delivery_online(&packet, now).unwrap();
            bob.acknowledge_incoming_online().unwrap();
            assert!(bob.group_receiver_ready(group, afp).unwrap());
            alice
                .queue_group_text(group, [114; 32], "after recovery", now, now)
                .unwrap();
            for attempt in alice.resume_group_outbound_online(now).unwrap() {
                assert!(attempt.result.is_ok(), "{:?}", attempt.result);
            }
            let received = bob.receive_mailbox_online(now).unwrap();
            assert!(received.iter().any(|a|matches!(&a.result,Ok(crate::MailboxEvent::GroupText(m)) if m.text().unwrap().body=="after recovery")));
            assert!(!alice.peer(b).unwrap().trusted && !bob.peer(a).unwrap().trusted);
            // A delayed original seed is harmless after current-position recovery.
            if !expired {
                deliver(&mut alice, &mut bob, now);
            }
            let remove = alice
                .prepare_group_change(group, Change::Remove([112; 32]))
                .unwrap();
            alice
                .prepare_group_service_request(group, Some(&remove))
                .unwrap();
            alice.submit_group_service_online(group, now).unwrap();
            assert_eq!(
                alice
                    .db
                    .query_row("SELECT count(*) FROM group_key_recovery", [], |r| r
                        .get::<_, u32>(0))
                    .unwrap(),
                0
            );
            assert!(alice.group_key_recovery_work_online(group, b, now).is_err());
        }
    }
}
