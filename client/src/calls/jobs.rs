use super::*;
use control::{Body, Wire};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Job {
    wire: Wire,
    peer: Id,
    claim: Id,
    session: Id,
    queued: bool,
}
pub struct Attempt {
    pub id: Id,
    pub result: Result<(), Error>,
}
fn aad(id: &Id) -> Vec<u8> {
    [b"Sigil/call-job/v1".as_slice(), id].concat()
}
fn read(db: &Connection, key: &StorageKey, id: &Id) -> Result<Job, Error> {
    let raw: Vec<u8> = db
        .query_row(
            "SELECT CASE WHEN length(content)<=131108 THEN content END FROM call_jobs WHERE id=?1",
            [id.as_slice()],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    let job: Job =
        serde_json::from_slice(&key.open(&raw, &aad(id))?).map_err(|_| Error::InvalidStore)?;
    if job.wire.message != *id {
        return Err(Error::InvalidStore);
    }
    job.wire.bytes()?;
    Ok(job)
}
/// Every pending job's body and session, so tests can reorder deliveries.
#[cfg(test)]
pub(super) fn pending(db: &Connection, key: &StorageKey) -> Result<Vec<(Body, Id)>, Error> {
    let ids: Vec<Vec<u8>> = db
        .prepare("SELECT id FROM call_jobs ORDER BY queued")?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    ids.into_iter()
        .map(|id| {
            let id: Id = id.try_into().map_err(|_| Error::InvalidStore)?;
            let job = read(db, key, &id)?;
            Ok((job.wire.body, job.session))
        })
        .collect()
}
fn save(db: &Connection, key: &StorageKey, job: &Job) -> Result<(), Error> {
    let raw = Zeroizing::new(serde_json::to_vec(job).map_err(|_| Error::InvalidStore)?);
    if raw.len() > 131072 {
        return Err(Error::Limit);
    }
    db.execute("INSERT INTO call_jobs VALUES(?1,?2,(SELECT IFNULL(MAX(queued),0)+1 FROM call_jobs)) ON CONFLICT(id) DO UPDATE SET content=excluded.content",(job.wire.message.as_slice(),key.seal(&raw,&aad(&job.wire.message))?))?;
    Ok(())
}
pub(super) fn queue(
    tx: &Transaction<'_>,
    key: &StorageKey,
    record: &Record,
    peer: Id,
    body: Body,
    now: u64,
) -> Result<(), Error> {
    let known = channels::authorized(tx, key, record, &peer)?;
    let own = crate::device_fingerprint(&crate::peers::own(tx, key)?)?;
    let expires = if matches!(body, Body::Invite(_) | Body::DirectInvite(_)) {
        record
            .invites
            .iter()
            .find(|v| v.peer == peer)
            .ok_or(Error::Unprepared)?
            .until
    } else {
        record.state.roster.roster.expires
    };
    let raw = Zeroizing::new(
        if known.trusted {
            serde_json::to_vec(&(record.id(), peer, expires, &body))
        } else {
            serde_json::to_vec(&(record.id(), peer, expires, &body, true))
        }
        .map_err(|_| Error::InvalidStore)?,
    );
    let id = key.commitment(&raw, b"Sigil/call-job-id/v1")?;
    match read(tx, key, &id) {
        Ok(prior) => {
            if prior.peer != peer {
                return Err(Error::Conflict);
            }
            return Ok(());
        }
        Err(Error::NotFound) => (),
        Err(e) => return Err(e),
    }
    if tx.query_row("SELECT count(*) FROM call_jobs", [], |r| r.get::<_, u32>(0))? >= 1024 {
        return Err(Error::Limit);
    }
    if now >= expires {
        return Ok(());
    }
    save(
        tx,
        key,
        &Job {
            wire: Wire {
                message: id,
                call: record.id(),
                sender: own,
                recipient: known.fingerprint,
                expires,
                scoped: !known.trusted,
                body,
            },
            peer,
            claim: sigil_calls::random_id().map_err(failure)?,
            session: sigil_calls::random_id().map_err(failure)?,
            queued: false,
        },
    )
}
fn current(db: &Connection, key: &StorageKey, job: &Job, now: u64) -> Result<(), Error> {
    if now >= job.wire.expires {
        return Err(Error::Obsolete);
    }
    let record = match load(db, key, &job.wire.call) {
        Ok(v) => v,
        Err(Error::NotFound) => return Err(Error::Obsolete),
        Err(e) => return Err(e),
    };
    let known = match channels::authorized(db, key, &record, &job.peer) {
        Ok(value) => value,
        Err(Error::Unprepared | Error::NotFound) => return Err(Error::Obsolete),
        Err(e) => return Err(e),
    };
    if known.fingerprint != job.wire.recipient {
        return Err(Error::Obsolete);
    }
    let permitted = match &job.wire.body {
        Body::Invite(_) | Body::DirectInvite(_) => {
            record.phase == Phase::Active
                && record.invites.iter().any(|v| {
                    v.peer == job.peer
                        && v.fingerprint == known.fingerprint
                        && v.until == job.wire.expires
                })
        }
        Body::Join(proof) => {
            record.phase == Phase::Joining
                && record
                    .own
                    .as_ref()
                    .is_some_and(|v| v.member == proof.member)
        }
        Body::State(state) => {
            state.digest().map_err(failure)? == record.state.digest().map_err(failure)?
                && record.commits.is_empty()
        }
        Body::Ready(ready) => {
            record.phase == Phase::Active
                && ready.sequence == record.ready_sequence
                && ready.challenge == record.lease
        }
        Body::Shares(shares) => {
            record.phase == Phase::Active
                && !shares.is_empty()
                && shares.iter().all(|share| {
                    // Either the state we hold, or the one we have declared readiness
                    // for and expect the owner to commit next.
                    (Some(share.state) == record.state.digest().ok()
                        || Some(share.state) == record.anticipated)
                        && record.shares.iter().any(|s| {
                            s.key.context.sender == share.key.context.sender
                                && s.generation == share.generation
                                && s.signature == share.signature
                        })
                })
                && record
                    .state
                    .participants
                    .iter()
                    .any(|p| p.fingerprint().ok() == Some(known.fingerprint))
                && (record.owner_peer.is_some()
                    || share_digest(shares)? == share_digest(&record.shares)?)
        }
        Body::Leave => matches!(record.phase, Phase::Declined | Phase::Left),
        Body::Handoff { state, proof } => {
            record.phase == Phase::Left
                && record.transfer.as_ref() == Some(proof)
                && state.digest().map_err(failure)? == record.state.digest().map_err(failure)?
        }
        Body::CancelInvite => record.phase == Phase::Left && record.transfer.is_some(),
    };
    if permitted {
        if matches!(job.wire.body, Body::Shares(_)) {
            match record.authorize(db, key, now) {
                Err(Error::Conflict | Error::Expired | Error::Unprepared) => {
                    return Err(Error::Obsolete)
                }
                result => result?,
            }
        }
        Ok(())
    } else {
        Err(Error::Obsolete)
    }
}
pub(crate) fn delivery_peer(
    tx: &Transaction<'_>,
    key: &StorageKey,
    session: &Id,
    message: &Id,
) -> Result<Option<crate::Peer>, Error> {
    let job = match read(tx, key, message) {
        Ok(job) => job,
        Err(Error::NotFound) => return Ok(None),
        Err(error) => return Err(error),
    };
    if !job.wire.scoped {
        return Ok(None);
    }
    if !job.queued || job.session != *session {
        return Err(Error::Conflict);
    }
    current(tx, key, &job, crate::conversations::now())?;
    let record = load(tx, key, &job.wire.call)?;
    Ok(Some(channels::authorized(tx, key, &record, &job.peer)?))
}
pub(super) fn leave(
    tx: &Transaction<'_>,
    key: &StorageKey,
    record: &Record,
    owner: Id,
    now: u64,
) -> Result<(), Error> {
    match queue(tx, key, record, owner, Body::Leave, now) {
        Err(Error::Unprepared | Error::NotFound) => Ok(()),
        result => result,
    }
}
pub(crate) fn check_retained(
    db: &Connection,
    key: &StorageKey,
    raw: &[u8],
    now: u64,
) -> Result<(), Error> {
    if !control::authenticate_marker(key, raw)? {
        return Ok(());
    }
    let message = control::receipt_message(raw)?.ok_or(Error::InvalidStore)?;
    let job = match read(db, key, &message) {
        Ok(v) => v,
        Err(Error::NotFound) => return Err(Error::Obsolete),
        Err(e) => return Err(e),
    };
    if control::retained(key, &job.wire.bytes()?)?.as_ref() != raw {
        return Err(Error::InvalidStore);
    }
    current(db, key, &job, now)
}
/// Wait before claiming this recipient again, longer each time up to a minute; the
/// first failure retries next pass because a peer's own next pass usually clears it.
fn defer(db: &Connection, id: &Id, now: u64) -> Result<(), Error> {
    let attempts: i64 = db
        .query_row(
            "SELECT attempts FROM call_job_backoff WHERE id=?1",
            [id.as_slice()],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0i64)
        .saturating_add(1);
    let wait = match attempts {
        ..=1 => 0,
        n => 1u64 << (n - 2).min(6),
    }
    .min(60);
    db.execute(
        "INSERT INTO call_job_backoff VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET until=excluded.until,attempts=excluded.attempts",
        (id.as_slice(), now.saturating_add(wait) as i64, attempts),
    )?;
    Ok(())
}
fn settle(db: &Connection, id: &Id) -> Result<(), Error> {
    db.execute("DELETE FROM call_job_backoff WHERE id=?1", [id.as_slice()])?;
    Ok(())
}
fn discard(tx: &Transaction<'_>, key: &StorageKey, id: Id) -> Result<(), Error> {
    let session: Option<Vec<u8>> = tx
        .query_row(
            "SELECT session FROM deliveries WHERE id=?1",
            [id.as_slice()],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(session) = session {
        let session: Id = session.try_into().map_err(|_| Error::InvalidStore)?;
        if crate::transport::receipt(tx, key, session, id)?.is_none() {
            crate::conversations::mark_cancelled(tx, key, &session, &id)?;
            tx.execute(
                "UPDATE outbox SET packet=NULL WHERE session=?1 AND id=?2",
                (session.as_slice(), id.as_slice()),
            )?;
        }
    }
    tx.execute("DELETE FROM call_jobs WHERE id=?1", [id.as_slice()])?;
    settle(tx, &id)
}
fn share_digest(shares: &[Share]) -> Result<Id, Error> {
    use sha2::{Digest, Sha256};
    let mut values: Vec<_> = shares
        .iter()
        .map(|s| {
            (
                &s.state,
                s.generation,
                &s.key.context.sender,
                &s.key.context.incarnation,
                &s.signature,
            )
        })
        .collect();
    values.sort_by_key(|v| *v.2);
    Ok(Sha256::digest(serde_json::to_vec(&values).map_err(|_| Error::InvalidStore)?).into())
}
impl ClientStore {
    fn allow_call_controls(&mut self, record: &Record) -> Result<(), Error> {
        let id = record.id();
        if record.phase != Phase::Active
            || record.transfer.is_none() && record.state.roster.delegations.is_empty()
        {
            return Ok(());
        }
        let recipients: Vec<_> = record
            .state
            .participants
            .iter()
            .filter(|p| Some(p.member.id) != record.own.as_ref().map(|own| own.member.id))
            .map(peer)
            .collect::<Result<_, _>>()?;
        for recipient in &recipients {
            if record.owner_peer.is_some_and(|owner| owner != *recipient)
                || record.control_grants.contains(recipient)
            {
                continue;
            }
            let known = channels::authorized(&self.db, &self.key, record, recipient)?;
            self.allow_known_sender_online(&known)?;
            let tx = self
                .db
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let mut current = load(&tx, &self.key, &id)?;
            if current.state.digest().map_err(failure)? != record.state.digest().map_err(failure)?
                || current.phase != Phase::Active
            {
                return Err(Error::Conflict);
            }
            current.control_grants.retain(|p| recipients.contains(p));
            if !current.control_grants.contains(recipient) {
                current.control_grants.push(*recipient);
            }
            super::save(&tx, &self.key, &current)?;
            tx.commit()?;
        }
        Ok(())
    }
    fn prepare_call_job_online(&mut self, id: Id, now: u64) -> Result<(), Error> {
        let mut job = read(&self.db, &self.key, &id)?;
        if let Err(error) = current(&self.db, &self.key, &job, now) {
            if matches!(error, Error::Obsolete) {
                let tx = self
                    .db
                    .transaction_with_behavior(TransactionBehavior::Immediate)?;
                discard(&tx, &self.key, id)?;
                tx.commit()?;
                return Ok(());
            }
            return Err(error);
        }
        if job.queued {
            if self.delivery_receipt(job.session, id)?.is_some() {
                self.db
                    .execute("DELETE FROM call_jobs WHERE id=?1", [id.as_slice()])?;
                settle(&self.db, &id)?;
            }
            return Ok(());
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let selection = if job.wire.scoped {
            Err(Error::Unprepared)
        } else {
            crate::selection::for_send(&tx, &self.key, &job.peer, now)
        };
        match selection {
            Ok(session) => {
                let raw = job.wire.bytes()?;
                crate::send_in(&tx, &self.key, session, id, &raw)?;
                let destination = crate::peers::destination(&tx, &self.key, &job.peer)?;
                let prepared = crate::transport::prepare(
                    &tx,
                    &self.key,
                    session,
                    id,
                    destination,
                    Some(job.wire.expires),
                    now,
                );
                match prepared {
                    Ok(_) => {
                        job.session = session;
                        job.queued = true;
                        save(&tx, &self.key, &job)?;
                        settle(&tx, &id)?;
                        tx.commit()?;
                        return Ok(());
                    }
                    Err(Error::Expired) => tx.rollback()?,
                    Err(error) => return Err(error),
                }
            }
            Err(Error::Unprepared | Error::Expired) => tx.commit()?,
            Err(e) => return Err(e),
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let record = load(&tx, &self.key, &job.wire.call)?;
        let known = channels::authorized(&tx, &self.key, &record, &job.peer)?;
        crate::claims::prepare(
            &tx,
            &self.key,
            &crate::handshake::identity(&tx, &self.key)?.public_key(),
            job.claim,
            known.binding.device,
            known.binding.identity,
            (
                (!job.wire.scoped).then_some(job.peer),
                Some(&known.binding.server),
            ),
        )?;
        tx.commit()?;
        if let Err(error) = self.claim_prekey_online(job.claim, now) {
            // 404 also covers exhausted bundles and 403 a grant still in flight, so the
            // job waits rather than ends.
            if matches!(
                error,
                Error::Network(crate::network::Error::Status {
                    code: 403 | 404 | 507,
                    ..
                })
            ) {
                defer(&self.db, &id, now)?;
            }
            if job.wire.scoped
                && matches!(
                    error,
                    Error::Network(crate::network::Error::Status {
                        code: 403 | 404,
                        ..
                    })
                )
            {
                return Ok(());
            }
            if matches!(error, Error::Expired) {
                let tx = self
                    .db
                    .transaction_with_behavior(TransactionBehavior::Immediate)?;
                let previous = read(&tx, &self.key, &id)?;
                if previous.claim != job.claim || previous.queued {
                    return Err(Error::Conflict);
                }
                let identity = crate::handshake::identity(&tx, &self.key)?.public_key();
                crate::claims::abandon(&tx, &self.key, &job.claim, &identity)?;
                job.claim = sigil_calls::random_id().map_err(failure)?;
                job.session = sigil_calls::random_id().map_err(failure)?;
                save(&tx, &self.key, &job)?;
                tx.commit()?;
            }
            return Err(error);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous = read(&tx, &self.key, &id)?;
        if previous.claim != job.claim || previous.queued {
            return Err(Error::Conflict);
        }
        current(&tx, &self.key, &job, now)?;
        crate::claims::start_in(
            &tx,
            &self.key,
            job.claim,
            job.session,
            id,
            (&job.wire.bytes()?, None, Some(job.wire.expires)),
            now,
        )?;
        job.queued = true;
        save(&tx, &self.key, &job)?;
        settle(&tx, &id)?;
        tx.commit()?;
        Ok(())
    }
    pub fn resume_calls_online(&mut self, now: u64) -> Result<Vec<Attempt>, Error> {
        let now = crate::conversations::time_floor(&self.db, &self.key, now)?;
        self.own_device_binding()?;
        let rows: Vec<Vec<u8>> = self
            .db
            .prepare("SELECT id FROM calls ORDER BY id LIMIT 256")?
            .query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let mut attempts = Vec::new();
        for row in rows {
            let index: Id = row.try_into().map_err(|_| Error::InvalidStore)?;
            let mut record = load_index(&self.db, &self.key, &index)?;
            if now >= record.state.roster.roster.expires {
                record.finish(Phase::Ended);
                super::save(&self.db, &self.key, &record)?;
                self.db
                    .execute("DELETE FROM calls WHERE id=?1", [index.as_slice()])?;
                continue;
            }
            if record.expire(now) {
                super::save(&self.db, &self.key, &record)?;
            }
            // A finished call with nothing left to publish or announce needs no more reads.
            if matches!(record.phase, Phase::Declined | Phase::Left | Phase::Ended)
                && record.commits.is_empty()
                && record.notify.is_empty()
                && record.shares.is_empty()
                && record.joining.is_empty()
            {
                continue;
            }
            if let Err(error) = self.allow_call_controls(&record) {
                attempts.push(Attempt {
                    id: record.id(),
                    result: Err(error),
                });
                continue;
            }
            if record.owner_peer.is_some() {
                continue;
            }
            let id = record.id();
            let result = (|| {
                let tx = self
                    .db
                    .transaction_with_behavior(TransactionBehavior::Immediate)?;
                let mut current = load(&tx, &self.key, &id)?;
                if current.direct
                    && current.phase == Phase::Active
                    && current.state.participants.len() == 1
                    && current.joining.is_empty()
                    && !current.invites.is_empty()
                    && current.invites.iter().all(|v| now >= v.until)
                {
                    end(&self.key, &mut current)?;
                    super::save(&tx, &self.key, &current)?;
                }
                if !current.joining.is_empty() {
                    let mut state = current.state.clone();
                    state.roster.roster.previous =
                        Some(state.roster.roster.digest().map_err(failure)?);
                    state.roster.roster.revision += 1;
                    for proof in &current.joining {
                        let known = crate::peers::trusted(&tx, &self.key, &peer(proof)?)?;
                        if known.fingerprint != proof.fingerprint().map_err(failure)? {
                            return Err(Error::Conflict);
                        }
                        state.roster.roster.members.push(proof.member.clone());
                        state.participants.push(proof.clone());
                    }
                    state.roster.roster.members.sort_by_key(|m| m.id);
                    state.participants.sort_by_key(|p| p.member.id);
                    state.roster = current
                        .state
                        .roster
                        .update(state.roster.roster, &current.key(&self.key)?)
                        .map_err(failure)?;
                    current.change(&self.key, state)?;
                    current.joining.clear();
                    super::save(&tx, &self.key, &current)?;
                }
                tx.commit()?;
                for _ in 0..4 {
                    let record = load(&self.db, &self.key, &id)?;
                    let Some(roster) = record.commits.first() else {
                        break;
                    };
                    let published = self.connected_client()?.publish_call(roster);
                    let rejected_initial = matches!(
                        published,
                        Err(crate::network::Error::Status { code: 409, .. })
                    ) && roster.roster.revision == 0
                        && now.saturating_sub(roster.roster.created) > 60;
                    if !rejected_initial {
                        published?;
                    }
                    let tx = self
                        .db
                        .transaction_with_behavior(TransactionBehavior::Immediate)?;
                    let mut current = load(&tx, &self.key, &id)?;
                    if current.commits.first() != Some(roster) {
                        return Err(Error::Conflict);
                    }
                    if rejected_initial {
                        // A rejected initial roster past its admission window cannot be retried.
                        current.finish(Phase::Ended);
                        current.commits.clear();
                        current.notify.clear();
                        super::save(&tx, &self.key, &current)?;
                        tx.commit()?;
                        return Ok(());
                    }
                    current.commits.remove(0);
                    super::save(&tx, &self.key, &current)?;
                    tx.commit()?;
                }
                let tx = self
                    .db
                    .transaction_with_behavior(TransactionBehavior::Immediate)?;
                let mut current = load(&tx, &self.key, &id)?;
                let digest = current.state.digest().map_err(failure)?;
                if current.commits.is_empty() && current.announced != Some(digest) {
                    let own = crate::device_fingerprint(&crate::peers::own(&tx, &self.key)?)?;
                    for recipient in &current.notify {
                        let known = match channels::authorized(&tx, &self.key, &current, recipient)
                        {
                            Ok(value) => value,
                            Err(Error::Unprepared | Error::NotFound)
                                if current.state.roster.roster.closed
                                    || !current
                                        .state
                                        .participants
                                        .iter()
                                        .any(|p| peer(p).ok() == Some(*recipient)) =>
                            {
                                continue
                            }
                            Err(e) => return Err(e),
                        };
                        if known.fingerprint != own {
                            queue(
                                &tx,
                                &self.key,
                                &current,
                                *recipient,
                                Body::State(current.state.clone()),
                                now,
                            )?;
                        }
                    }
                    current.announced = Some(digest);
                    current.notify.clear();
                    super::save(&tx, &self.key, &current)?;
                }
                if current.commits.is_empty() && !current.shares.is_empty() {
                    current.authorize(&tx, &self.key, now)?;
                    let digest = share_digest(&current.shares)?;
                    if current.announced_media != Some(digest) {
                        for proof in &current.state.participants {
                            if proof.member.id != current.own_id()? {
                                queue(
                                    &tx,
                                    &self.key,
                                    &current,
                                    peer(proof)?,
                                    Body::Shares(current.shares.clone()),
                                    now,
                                )?;
                            }
                        }
                        current.announced_media = Some(digest);
                        super::save(&tx, &self.key, &current)?;
                    }
                }
                tx.commit()?;
                Ok(())
            })();
            let stop = matches!(result, Err(Error::Network(_)));
            attempts.push(Attempt { id, result });
            if stop {
                return Ok(attempts);
            }
        }
        let aad = b"Sigil/call-job-cursor/v2";
        // A cursor left by an older shape reads as absent, which just restarts the
        // sweep from the front. Selecting it as NULL must not fail the whole stage.
        let raw:Option<Vec<u8>>=self.db.query_row("SELECT CASE WHEN length(content)=44 THEN content END FROM call_cursor WHERE id=1",[],|r|r.get::<_,Option<Vec<u8>>>(0)).optional()?.flatten();
        let after: i64 = match raw.map(|r| self.key.open(&r, aad)).transpose()? {
            Some(value) => i64::from_be_bytes(
                value.as_slice().try_into().map_err(|_| Error::InvalidStore)?,
            ),
            None => 0,
        };
        if after < 0 {
            return Err(Error::InvalidStore);
        }
        // Job ids do not ascend with age, so a job queued during this pass can sort
        // below the cursor. Sweep once more from the start rather than leaving it for
        // the next poll: an announcement queued here then leaves in this same pass.
        // Oldest first, and once the tail is reached sweep the front again so a job
        // queued by this very pass still leaves in it. Successful jobs are removed,
        // so the second sweep only sees work that is genuinely still pending.
        let mut next = after;
        let mut wrapped = after == 0;
        loop {
            let rows: Vec<(Vec<u8>, i64)> = self
                .db
                .prepare("SELECT id,queued FROM call_jobs WHERE queued>?1 AND NOT EXISTS(SELECT 1 FROM call_job_backoff b WHERE b.id=call_jobs.id AND b.until>?2) ORDER BY queued LIMIT 16")?
                .query_map((next, now as i64), |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<_, _>>()?;
            if rows.is_empty() {
                next = 0;
                if wrapped {
                    break;
                }
                wrapped = true;
                continue;
            }
            let mut stopped = false;
            for (id, queued) in rows {
                let id: Id = id.try_into().map_err(|_| Error::InvalidStore)?;
                let result = self.prepare_call_job_online(id, now);
                let stop = matches!(result, Err(Error::Network(_)));
                attempts.push(Attempt { id, result });
                next = queued;
                if stop {
                    stopped = true;
                    break;
                }
            }
            if stopped {
                break;
            }
        }
        self.db.execute("INSERT INTO call_cursor VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET content=excluded.content",[self.key.seal(&next.to_be_bytes(),aad)?])?;
        Ok(attempts)
    }
}
