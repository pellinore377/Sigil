use super::*;
use crate::claims;
#[derive(Debug)]
pub struct HistoryAttempt {
    pub id: Id,
    pub result: Result<HistoryShareStatus, Error>,
}
fn check_content(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    g: &Grant,
    w: &Work,
    now: u64,
) -> Result<(), Error> {
    if let Some(item) = w.item {
        let bytes = messages::history_content(tx, key, own, &g.group, &item)?;
        let event = Group::from_bytes(&bytes).map_err(|_| Error::InvalidStore)?;
        eligible(&event, g, now)?;
        crate::conversations::require_history(tx, key, &event, now)?;
        let fields = peers::parse(&peers::own(tx, key)?)?.binding;
        recovery::require_resend(
            tx,
            key,
            recovery::account_scope(&fields.server, fields.account)?,
            messages::group_event_history_id(&event),
            event.content,
        )?;
        if bytes != w.buffer {
            return Err(Error::Obsolete);
        }
    }
    Ok(())
}
pub(crate) fn cancelled(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    receipt: &DistributionReceipt,
) -> Result<bool, Error> {
    let w = load(tx, key, own, &receipt.context.chain)?;
    Ok(w.status == HistoryShareStatus::Revoked
        || (w.active != Some(receipt.message) && !w.offers.contains(&receipt.message)))
}
pub(super) fn terminal(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    w: &mut Work,
    status: HistoryShareStatus,
) -> Result<(), Error> {
    let g = Grant::parse(&w.grant)?;
    for id in w.offers.drain(..).chain(w.active.take()) {
        key_recovery::forget_job(tx, key, own, &g.group, Some(id))?;
    }
    w.status = status;
    w.packet = None;
    w.incoming = None;
    w.buffer.clear();
    w.item = None;
    save(tx, key, own, w)
}
impl ClientStore {
    fn queue_history_online(&mut self, id: Id, bytes: &[u8], now: u64) -> Result<(), Error> {
        let frame = Frame::parse(bytes)?;
        let grant = Grant::parse(&frame.grant)?;
        let message = frame.message()?;
        if grant.id != id || grant.expires <= now {
            return Err(Error::Expired);
        }
        self.refresh_group_authority_for_send(grant.group, now)?;
        let binding = self.own_device_binding()?;
        let own = device_fingerprint(&binding)?;
        if own != frame.sender {
            return Err(Error::Conflict);
        }
        let state = self.group_status(grant.group)?.state;
        grant.verify(&state)?;
        let target = state
            .members
            .iter()
            .flat_map(|m| &m.devices)
            .find(|d| peers::fingerprint(&d.binding).ok() == Some(frame.target))
            .ok_or(Error::Unprepared)?
            .to_bytes()
            .map_err(|_| Error::InvalidStore)?;
        let peer = self.observe_peer_binding(&target)?.id;
        let identity = self.identity()?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = keys::current(&tx, &self.key, &own, &grant.group)?;
        grant.verify(&state)?;
        let known = keys::authorized_peer(&tx, &self.key, &state, &peer)?;
        let w = load(&tx, &self.key, &own, &id)?;
        if frame.kind == 1 {
            check_content(&tx, &self.key, &own, &grant, &w, now)?;
        }
        if keys::job(&tx, &self.key, &own, &grant.group, &message)?.is_some() {
            return Ok(());
        }
        let claim = digest(b"Sigil/group-history-claim/v0", &[&own, &message]);
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
        self.refresh_group_authority_for_send(grant.group, now)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = keys::current(&tx, &self.key, &own, &grant.group)?;
        grant.verify(&state)?;
        keys::authorized_peer(&tx, &self.key, &state, &peer)?;
        let mut w = load(&tx, &self.key, &own, &id)?;
        if frame.kind == 1 {
            check_content(&tx, &self.key, &own, &grant, &w, now)?;
        }
        if frame.kind != 0 && w.packet.as_ref().map(|v| v.as_slice()) != Some(bytes) {
            return Err(Error::Obsolete);
        }
        if keys::job(&tx, &self.key, &own, &grant.group, &message)?.is_some() {
            return Ok(());
        }
        let session = digest(b"Sigil/group-history-session/v0", &[&own, &message]);
        claims::start_in(
            &tx,
            &self.key,
            claim,
            session,
            message,
            (bytes, None, Some(grant.expires)),
            now,
        )?;
        keys::mark_channel(&tx, &self.key, &own, &session, &grant.group, &frame.target)?;
        let mut raw = session.to_vec();
        raw.extend_from_slice(&retained_payload(&self.key, bytes)?);
        let sealed = self.key.seal(
            &raw,
            &crate::binding(48, &grant.group, &[own.as_slice(), &message].concat()),
        )?;
        if frame.kind == 0 {
            if !w.offers.contains(&message) {
                w.offers.push(message);
            }
            w.offered |= if frame.target == grant.source { 1 } else { 2 };
        } else {
            let previous = w.active.replace(message);
            key_recovery::forget_job(&tx, &self.key, &own, &grant.group, previous)?;
        }
        tx.execute(
            "INSERT INTO group_key_outbox VALUES(?1,?2,?3)",
            (message.as_slice(), grant.group.as_slice(), sealed),
        )?;
        if frame.kind == 3 || frame.kind == 4 {
            w.packet = None;
        }
        save(&tx, &self.key, &own, &w)?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn check_history_send(
        &mut self,
        id: Id,
        message: Id,
        now: u64,
    ) -> Result<(), Error> {
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let w = load(&self.db, &self.key, &own, &id)?;
        let g = Grant::parse(&w.grant)?;
        if g.expires <= now {
            return Err(Error::Expired);
        }
        self.refresh_group_authority_for_send(g.group, now)?;
        let tx = self.db.transaction()?;
        let state = keys::current(&tx, &self.key, &own, &g.group)?;
        g.verify(&state)?;
        if w.active == Some(message) && own == g.source {
            check_content(&tx, &self.key, &own, &g, &w, now)?;
        }
        Ok(())
    }
    pub fn advance_group_history_online(
        &mut self,
        id: Id,
        now: u64,
    ) -> Result<HistoryShareStatus, Error> {
        if now == 0 || now > i64::MAX as u64 {
            return Err(Error::Expired);
        }
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let mut w = load(&self.db, &self.key, &own, &id)?;
        let g = Grant::parse(&w.grant)?;
        if g.expires <= now {
            let status = match w.status {
                HistoryShareStatus::Pending | HistoryShareStatus::Transferring => {
                    HistoryShareStatus::Unavailable
                }
                status => status,
            };
            let tx = self.db.transaction()?;
            terminal(&tx, &self.key, &own, &mut w, status)?;
            tx.commit()?;
            return Ok(status);
        }
        if matches!(
            w.status,
            HistoryShareStatus::Revoked
                | HistoryShareStatus::Unavailable
                | HistoryShareStatus::Authorized
        ) && w.packet.is_none()
        {
            return Ok(w.status);
        }
        if w.status == HistoryShareStatus::Complete && w.packet.is_none() {
            return Ok(w.status);
        }
        if !self.sync_group_service_online(g.group, now)? {
            return Ok(w.status);
        }
        let status = self.group_status(g.group)?;
        if status.frozen || g.verify(&status.state).is_err() {
            let tx = self.db.transaction()?;
            terminal(&tx, &self.key, &own, &mut w, HistoryShareStatus::Revoked)?;
            tx.commit()?;
            return Ok(w.status);
        }
        if own == g.issuer {
            for (bit, target) in [(1, g.source), (2, g.target)] {
                if target == own || w.offered & bit != 0 {
                    continue;
                }
                let frame = Frame {
                    grant: w.grant.clone(),
                    sender: own,
                    target,
                    sequence: 0,
                    kind: 0,
                    offset: 0,
                    total: 0,
                    root: [0; 32],
                    payload: Zeroizing::new(Vec::new()),
                };
                self.queue_history_online(id, &frame.bytes()?, now)?;
                return Ok(w.status);
            }
            if own != g.source && own != g.target {
                w.status = HistoryShareStatus::Authorized;
                let tx = self.db.transaction()?;
                save(&tx, &self.key, &own, &w)?;
                tx.commit()?;
                return Ok(w.status);
            }
        }
        if own == g.source {
            let tx = self
                .db
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            w = load(&tx, &self.key, &own, &id)?;
            if let Err(error) = check_content(&tx, &self.key, &own, &g, &w, now) {
                if !matches!(error, Error::Obsolete | Error::NotFound) {
                    return Err(error);
                }
                key_recovery::forget_job(&tx, &self.key, &own, &g.group, w.active.take())?;
                w.packet = None;
                w.buffer.clear();
                w.item = None;
                w.status = HistoryShareStatus::Unavailable;
            }
            if w.packet.is_none() {
                if w.buffer.is_empty() && w.status != HistoryShareStatus::Unavailable {
                    let next:Option<Vec<u8>>=tx.query_row("SELECT id FROM group_messages WHERE group_id=?1 AND id>?2 ORDER BY id LIMIT 1",(g.group.as_slice(),w.cursor.map(|v|v.to_vec()).unwrap_or_default()),|r|r.get(0)).optional()?;
                    if let Some(next) = next {
                        let next: Id = next.try_into().map_err(|_| Error::InvalidStore)?;
                        let bytes =
                            messages::history_content(&tx, &self.key, &own, &g.group, &next)?;
                        let event = Group::from_bytes(&bytes).map_err(|_| Error::InvalidStore)?;
                        let fields = peers::parse(&peers::own(&tx, &self.key)?)?.binding;
                        let eligible = eligible(&event, &g, now).and_then(|_| {
                            crate::conversations::require_history(&tx, &self.key, &event, now)?;
                            recovery::require_resend(
                                &tx,
                                &self.key,
                                recovery::account_scope(&fields.server, fields.account)?,
                                messages::group_event_history_id(&event),
                                event.content,
                            )
                        });
                        match eligible {
                            Ok(()) => {
                                w.item = Some(next);
                                w.buffer = bytes;
                                w.offset = 0;
                            }
                            Err(Error::Obsolete) => {
                                w.cursor = Some(next);
                                save(&tx, &self.key, &own, &w)?;
                                tx.commit()?;
                                return Ok(w.status);
                            }
                            Err(e) => return Err(e),
                        }
                    }
                }
                let (kind, offset, total, root, payload) = if !w.buffer.is_empty() {
                    let offset = w.offset as usize;
                    (
                        1,
                        w.offset,
                        w.buffer.len() as u32,
                        digest(b"Sigil/group-history-item/v0", &[&w.buffer]),
                        Zeroizing::new(
                            w.buffer[offset..w.buffer.len().min(offset + CHUNK)].to_vec(),
                        ),
                    )
                } else if w.count == 0 || w.status == HistoryShareStatus::Unavailable {
                    (4, 0, 0, [0; 32], Zeroizing::new(Vec::new()))
                } else {
                    (
                        2,
                        0,
                        8,
                        [0; 32],
                        Zeroizing::new(w.count.to_be_bytes().to_vec()),
                    )
                };
                let frame = Frame {
                    grant: w.grant.clone(),
                    sender: own,
                    target: g.target,
                    sequence: w.sequence,
                    kind,
                    offset,
                    total,
                    root,
                    payload,
                };
                w.packet = Some(frame.bytes()?);
                w.status = if kind == 4 {
                    HistoryShareStatus::Unavailable
                } else {
                    HistoryShareStatus::Transferring
                };
                save(&tx, &self.key, &own, &w)?;
            }
            tx.commit()?;
        } else if own == g.target {
            w = self.import_group_history(id, own, now)?;
        }
        if let Some(bytes) = w.packet {
            self.queue_history_online(id, &bytes, now)?;
        }
        Ok(load(&self.db, &self.key, &own, &id)?.status)
    }
    pub(super) fn import_group_history(
        &mut self,
        id: Id,
        own: Id,
        now: u64,
    ) -> Result<Work, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut w = load(&tx, &self.key, &own, &id)?;
        let g = Grant::parse(&w.grant)?;
        if own != g.target {
            return Err(Error::Unprepared);
        }
        let status = store::load(&tx, &self.key, &own, &g.group)?;
        if status.frozen || g.verify(&status.state).is_err() {
            terminal(&tx, &self.key, &own, &mut w, HistoryShareStatus::Revoked)?;
            tx.commit()?;
            return Ok(w);
        }
        if let Some(bytes) = w.incoming.take() {
            let frame = Frame::parse(&bytes)?;
            if frame.kind == 4 {
                terminal(
                    &tx,
                    &self.key,
                    &own,
                    &mut w,
                    HistoryShareStatus::Unavailable,
                )?;
                tx.commit()?;
                return Ok(w);
            }
            if frame.kind == 1 {
                if frame.offset == 0 && w.buffer.is_empty() {
                    w.root = frame.root;
                    w.total = frame.total;
                }
                if frame.offset as usize != w.buffer.len()
                    || w.root != frame.root
                    || w.total != frame.total
                {
                    return Err(Error::Conflict);
                }
                w.buffer.extend_from_slice(&frame.payload);
                if w.buffer.len() == w.total as usize {
                    if digest(b"Sigil/group-history-item/v0", &[&w.buffer]) != w.root {
                        return Err(Error::Conflict);
                    }
                    eligible(
                        &Group::from_bytes(&w.buffer).map_err(|_| Error::InvalidEvent)?,
                        &g,
                        now,
                    )?;
                    retain(&tx, &self.key, &own, &g, &w.buffer)?;
                    w.count = w.count.checked_add(1).ok_or(Error::Limit)?;
                    w.buffer.clear();
                    w.total = 0;
                }
                w.status = HistoryShareStatus::Transferring;
            } else if frame.kind == 2 {
                if !w.buffer.is_empty()
                    || u64::from_be_bytes(
                        frame
                            .payload
                            .as_slice()
                            .try_into()
                            .map_err(|_| Error::InvalidEvent)?,
                    ) != w.count
                {
                    return Err(Error::Conflict);
                }
                w.status = HistoryShareStatus::Complete;
            } else {
                return Err(Error::Conflict);
            }
            w.sequence = w.sequence.checked_add(1).ok_or(Error::Limit)?;
            w.packet = Some(
                Frame {
                    grant: w.grant.clone(),
                    sender: own,
                    target: g.source,
                    sequence: frame.sequence,
                    kind: 3,
                    offset: 0,
                    total: 0,
                    root: frame.message()?,
                    payload: Zeroizing::new(Vec::new()),
                }
                .bytes()?,
            );
            save(&tx, &self.key, &own, &w)?;
        }
        tx.commit()?;
        Ok(w)
    }
    pub fn resume_group_history_online(&mut self, now: u64) -> Result<Vec<HistoryAttempt>, Error> {
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let tx = self.db.transaction()?;
        let cursor = super::super::work::load(&tx, &self.key, &own, b"history")?;
        let next: Option<Vec<u8>> = tx
            .query_row(
                "SELECT id FROM group_history_work WHERE active=1 AND id>?1 ORDER BY id LIMIT 1",
                [cursor.after.map(|v| v.to_vec()).unwrap_or_default()],
                |r| r.get(0),
            )
            .optional()?;
        let id = next
            .map(|v| v.try_into().map_err(|_| Error::InvalidStore))
            .transpose()?;
        super::super::work::save(
            &tx,
            &self.key,
            &own,
            b"history",
            &super::super::work::Cursor {
                after: id,
                relay: None,
            },
        )?;
        tx.commit()?;
        Ok(id
            .map(|id| HistoryAttempt {
                id,
                result: self.advance_group_history_online(id, now),
            })
            .into_iter()
            .collect())
    }
}
