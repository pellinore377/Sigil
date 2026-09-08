//! Admin-authorized retained history; imported records retain source provenance.
use super::*;
use crate::{recovery, ClientStore};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sigil_crypto::{sender_keys as sk, storage::StorageKey};
use sigil_protocol::event::{Content, Group};
use zeroize::Zeroizing;
const GRANT: &[u8; 8] = b"SGHG\0\x01\0\0";
const WIRE: &[u8; 8] = b"SGHT\0\x01\0\0";
const CHUNK: usize = 48 * 1024;
const MAX_ACTIVE: u32 = 16;
pub(crate) const MIGRATION:&str="CREATE TABLE group_history_work(id BLOB PRIMARY KEY,group_id BLOB NOT NULL REFERENCES groups(id),state BLOB NOT NULL,active INTEGER NOT NULL CHECK(active IN (0,1))); CREATE INDEX group_history_work_group ON group_history_work(group_id,id) WHERE active=1; CREATE TABLE group_shared_history(id BLOB PRIMARY KEY,group_id BLOB NOT NULL REFERENCES groups(id),state BLOB NOT NULL); CREATE INDEX group_shared_history_group ON group_shared_history(group_id,id); PRAGMA user_version=61;";
#[derive(Clone, Copy)]
pub struct HistoryRange {
    pub source_device: Id,
    pub recipient_device: Id,
    pub from_timestamp: u64,
    pub until_timestamp: u64,
    pub expires_at: u64,
}
struct Grant {
    id: Id,
    group: Id,
    head: Id,
    epoch: u64,
    issuer: Id,
    source: Id,
    target: Id,
    member: Id,
    from: u64,
    until: u64,
    expires: u64,
    signature: [u8; 64],
}
impl Grant {
    fn unsigned(&self) -> Result<Vec<u8>, Error> {
        if self.id == [0; 32]
            || self.source == self.target
            || self.from >= self.until
            || self.until > self.expires
            || self.expires > i64::MAX as u64
        {
            return Err(Error::InvalidEvent);
        }
        let mut out = GRANT.to_vec();
        for id in [
            &self.id,
            &self.group,
            &self.head,
            &self.issuer,
            &self.source,
            &self.target,
            &self.member,
        ] {
            out.extend_from_slice(id);
        }
        for time in [self.from, self.until, self.expires, self.epoch] {
            out.extend_from_slice(&time.to_be_bytes());
        }
        Ok(out)
    }
    fn statement(&self) -> Result<Id, Error> {
        Ok(digest(
            b"Sigil/group-history-grant/v0",
            &[&self.unsigned()?],
        ))
    }
    fn bytes(&self) -> Result<Vec<u8>, Error> {
        let mut out = self.unsigned()?;
        out.extend_from_slice(&self.signature);
        Ok(out)
    }
    fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() != 328 {
            return Err(Error::InvalidEvent);
        }
        let mut r = codec::Reader(bytes);
        if r.take(8)? != GRANT {
            return Err(Error::InvalidEvent);
        }
        let g = Self {
            id: r.array()?,
            group: r.array()?,
            head: r.array()?,
            issuer: r.array()?,
            source: r.array()?,
            target: r.array()?,
            member: r.array()?,
            from: u64::from_be_bytes(r.array()?),
            until: u64::from_be_bytes(r.array()?),
            expires: u64::from_be_bytes(r.array()?),
            epoch: u64::from_be_bytes(r.array()?),
            signature: r.array()?,
        };
        g.unsigned()?;
        Ok(g)
    }
    fn verify(&self, state: &State) -> Result<(), Error> {
        if state.closed
            || !state.earlier_history
            || state.group != self.group
            || state.head != self.head
        {
            return Err(Error::Obsolete);
        }
        let (issuer, key) = state.device(self.issuer)?;
        if issuer.role != Role::Admin || state.device(self.target)?.0.id != self.member {
            return Err(Error::Unprepared);
        }
        state.device(self.source)?;
        verify_signature(&key, &self.statement()?, &self.signature)?;
        Ok(())
    }
}
struct Frame {
    grant: Vec<u8>,
    sender: Id,
    target: Id,
    sequence: u64,
    kind: u8,
    offset: u32,
    total: u32,
    root: Id,
    payload: Zeroizing<Vec<u8>>,
}
impl Frame {
    fn bytes(&self) -> Result<Zeroizing<Vec<u8>>, Error> {
        if self.kind > 4
            || self.payload.len() > CHUNK
            || self.total > 65536
            || self.offset > self.total
            || self.offset as usize + self.payload.len() > self.total as usize
        {
            return Err(Error::InvalidEvent);
        }
        match self.kind {
            0 | 3 | 4 if !self.payload.is_empty() || self.total != 0 || self.offset != 0 => {
                return Err(Error::InvalidEvent)
            }
            1 if self.payload.is_empty() || self.sequence == 0 => return Err(Error::InvalidEvent),
            2 if self.payload.len() != 8 || self.total != 8 || self.offset != 0 => {
                return Err(Error::InvalidEvent)
            }
            _ => {}
        }
        let grant = Grant::parse(&self.grant)?;
        let valid = match self.kind {
            0 => {
                self.sender == grant.issuer
                    && (self.target == grant.source || self.target == grant.target)
            }
            1 | 2 | 4 => self.sender == grant.source && self.target == grant.target,
            3 => self.sender == grant.target && self.target == grant.source,
            _ => false,
        };
        if !valid {
            return Err(Error::Conflict);
        }
        let mut out = Zeroizing::new(WIRE.to_vec());
        out.extend_from_slice(&self.grant);
        out.extend_from_slice(&self.sender);
        out.extend_from_slice(&self.target);
        out.extend_from_slice(&self.sequence.to_be_bytes());
        out.push(self.kind);
        out.extend_from_slice(&self.offset.to_be_bytes());
        out.extend_from_slice(&self.total.to_be_bytes());
        out.extend_from_slice(&self.root);
        out.extend_from_slice(&self.payload);
        Ok(out)
    }
    fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > CHUNK + 449 {
            return Err(Error::Limit);
        }
        let mut r = codec::Reader(bytes);
        if r.take(8)? != WIRE {
            return Err(Error::InvalidEvent);
        }
        let grant = r.take(328)?.to_vec();
        let frame = Self {
            grant,
            sender: r.array()?,
            target: r.array()?,
            sequence: u64::from_be_bytes(r.array()?),
            kind: r.array::<1>()?[0],
            offset: u32::from_be_bytes(r.array()?),
            total: u32::from_be_bytes(r.array()?),
            root: r.array()?,
            payload: Zeroizing::new(r.0.to_vec()),
        };
        frame.bytes()?;
        Ok(frame)
    }
    fn message(&self) -> Result<Id, Error> {
        Ok(digest(b"Sigil/group-history-frame/v0", &[&self.bytes()?]))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistoryShareStatus {
    Pending,
    Transferring,
    Authorized,
    Complete,
    Unavailable,
    Revoked,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Work {
    grant: Vec<u8>,
    status: HistoryShareStatus,
    sequence: u64,
    cursor: Option<Id>,
    item: Option<Id>,
    offset: u32,
    root: Id,
    total: u32,
    count: u64,
    offered: u8,
    active: Option<Id>,
    offers: Vec<Id>,
    packet: Option<Zeroizing<Vec<u8>>>,
    incoming: Option<Zeroizing<Vec<u8>>>,
    buffer: Zeroizing<Vec<u8>>,
}
impl Work {
    fn active(&self) -> bool {
        matches!(
            self.status,
            HistoryShareStatus::Pending | HistoryShareStatus::Transferring
        ) || self.packet.is_some()
            || self.incoming.is_some()
            || self.active.is_some()
            || !self.offers.is_empty()
    }
    fn new(grant: Vec<u8>) -> Self {
        Self {
            grant,
            status: HistoryShareStatus::Pending,
            sequence: 1,
            cursor: None,
            item: None,
            offset: 0,
            root: [0; 32],
            total: 0,
            count: 0,
            offered: 0,
            active: None,
            offers: Vec::new(),
            packet: None,
            incoming: None,
            buffer: Zeroizing::new(Vec::new()),
        }
    }
}
fn aad(own: &Id, id: &Id) -> Vec<u8> {
    [b"Sigil/group-history-work/v0".as_slice(), own, id].concat()
}
fn load(db: &Connection, key: &StorageKey, own: &Id, id: &Id) -> Result<Work, Error> {
    let (group,raw,active):(Vec<u8>,Vec<u8>,bool)=db.query_row("SELECT group_id,CASE WHEN length(state)<=574000 THEN state END,active FROM group_history_work WHERE id=?1",[id.as_slice()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?.ok_or(Error::NotFound)?;
    let w: Work = serde_json::from_slice(&storage_record::open_record(key, &raw, &aad(own, id))?)
        .map_err(|_| Error::InvalidStore)?;
    let g = Grant::parse(&w.grant)?;
    if g.id != *id
        || g.group.as_slice() != group
        || ![g.issuer, g.source, g.target].contains(own)
        || w.buffer.len() > 65536
        || w.total > 65536
        || w.offers.len() > 2
        || w.sequence == 0
        || w.active() != active
    {
        return Err(Error::InvalidStore);
    }
    Ok(w)
}
fn save(tx: &Transaction<'_>, key: &StorageKey, own: &Id, w: &Work) -> Result<(), Error> {
    let g = Grant::parse(&w.grant)?;
    let raw = Zeroizing::new(serde_json::to_vec(w).map_err(|_| Error::InvalidStore)?);
    let raw = storage_record::seal_record(key, &raw, &aad(own, &g.id))?;
    tx.execute("INSERT INTO group_history_work VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET state=excluded.state,active=excluded.active",(g.id.as_slice(),g.group.as_slice(),raw,w.active()))?;
    Ok(())
}
fn ensure(tx: &Transaction<'_>, key: &StorageKey, own: &Id, bytes: &[u8]) -> Result<Work, Error> {
    let g = Grant::parse(bytes)?;
    match load(tx, key, own, &g.id) {
        Ok(w) => {
            if w.grant != bytes {
                return Err(Error::Conflict);
            }
            Ok(w)
        }
        Err(Error::NotFound) => {
            let ids = tx
                .prepare(
                    "SELECT id FROM group_history_work WHERE group_id=?1 AND active=1 LIMIT 17",
                )?
                .query_map([g.group.as_slice()], |r| r.get::<_, Vec<u8>>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            let mut active = 0;
            for id in ids {
                if load(
                    tx,
                    key,
                    own,
                    &id.try_into().map_err(|_| Error::InvalidStore)?,
                )?
                .active()
                {
                    active += 1;
                }
            }
            if active >= MAX_ACTIVE {
                return Err(Error::Limit);
            }
            let w = Work::new(bytes.to_vec());
            save(tx, key, own, &w)?;
            Ok(w)
        }
        Err(e) => Err(e),
    }
}
pub(super) fn is_wire(bytes: &[u8]) -> bool {
    bytes.starts_with(&WIRE[..4])
}
pub(super) fn marker(key: &StorageKey, bytes: &[u8]) -> Result<DistributionReceipt, Error> {
    let f = Frame::parse(bytes)?;
    let g = Grant::parse(&f.grant)?;
    Ok(DistributionReceipt {
        kind: 3,
        message: f.message()?,
        recipient: f.target,
        tag: key.commitment(bytes, b"Sigil/private-control-receipt/v0")?,
        context: sk::Context {
            group: g.group,
            state: g.head,
            epoch: g.epoch,
            sender: f.sender,
            chain: g.id,
        },
    })
}
pub(super) fn group(bytes: &[u8]) -> Result<Id, Error> {
    Ok(Grant::parse(&Frame::parse(bytes)?.grant)?.group)
}
pub(super) fn install(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    state: &State,
    bytes: &[u8],
    expires: u64,
) -> Result<(), Error> {
    let f = Frame::parse(bytes)?;
    let g = Grant::parse(&f.grant)?;
    g.verify(state)?;
    if expires > g.expires || f.target != *own {
        return Err(Error::Conflict);
    }
    let mut w = ensure(tx, key, own, &f.grant)?;
    if matches!(
        w.status,
        HistoryShareStatus::Complete
            | HistoryShareStatus::Unavailable
            | HistoryShareStatus::Revoked
    ) {
        return Ok(());
    }
    match f.kind {
        0 => {}
        3 => {
            if *own != g.source {
                return Err(Error::Conflict);
            }
            if f.sequence < w.sequence {
                return Ok(());
            }
            let sent = Frame::parse(w.packet.as_deref().ok_or(Error::Unprepared)?)?;
            if f.sequence != w.sequence || f.root != sent.message()? {
                return Err(Error::Conflict);
            }
            if sent.kind == 1 {
                w.offset = w
                    .offset
                    .checked_add(sent.payload.len() as u32)
                    .ok_or(Error::Limit)?;
                if w.offset == w.buffer.len() as u32 {
                    w.cursor = w.item.take();
                    w.buffer.clear();
                    w.offset = 0;
                    w.count = w.count.checked_add(1).ok_or(Error::Limit)?;
                }
            } else if sent.kind == 2 {
                work::terminal(tx, key, own, &mut w, HistoryShareStatus::Complete)?;
            }
            w.sequence = w.sequence.checked_add(1).ok_or(Error::Limit)?;
            w.packet = None;
        }
        _ => {
            if *own != g.target {
                return Err(Error::Conflict);
            }
            if f.kind != 4 && f.sequence < w.sequence {
                return Ok(());
            }
            if f.kind != 4 && f.sequence != w.sequence {
                return Err(Error::Conflict);
            }
            if w.incoming.as_deref().is_some_and(|prior| prior != bytes) {
                return Err(Error::Conflict);
            }
            w.incoming = Some(Zeroizing::new(bytes.to_vec()));
        }
    }
    save(tx, key, own, &w)
}
pub struct SharedGroupHistory {
    pub id: Id,
    pub source_device: Id,
    pub grant: Id,
    pub plaintext: Zeroizing<Vec<u8>>,
}
fn shared_aad(own: &Id, id: &Id) -> Vec<u8> {
    [b"Sigil/group-shared-history/v0".as_slice(), own, id].concat()
}
fn retain(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    g: &Grant,
    bytes: &[u8],
) -> Result<(), Error> {
    let event = Group::from_bytes(bytes).map_err(|_| Error::InvalidEvent)?;
    eligible(&event, g, 0)?;
    let binding = peers::parse(&peers::own(tx, key)?)?.binding;
    if recovery::record_deleted(
        tx,
        key,
        recovery::account_scope(&binding.server, binding.account)?,
        messages::group_event_history_id(&event),
    )? {
        return Ok(());
    }
    let id = key.commitment(
        &[
            g.group.as_slice(),
            &messages::group_event_history_id(&event),
            &g.source,
        ]
        .concat(),
        b"Sigil/shared-history-index/v0",
    )?;
    let raw = Zeroizing::new([g.group.as_slice(), &g.source, &g.id, bytes].concat());
    let old: Option<Vec<u8>> = tx
        .query_row(
            "SELECT state FROM group_shared_history WHERE id=?1",
            [id.as_slice()],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(old) = old {
        let old = storage_record::open_record(key, &old, &shared_aad(own, &id))?;
        if old.len() < 96 || old[96..] != *bytes {
            return Err(Error::Conflict);
        }
        return Ok(());
    }
    tx.execute(
        "INSERT INTO group_shared_history VALUES(?1,?2,?3)",
        (
            id.as_slice(),
            g.group.as_slice(),
            storage_record::seal_record(key, &raw, &shared_aad(own, &id))?,
        ),
    )?;
    Ok(())
}
fn eligible(event: &Group<'_>, g: &Grant, now: u64) -> Result<(), Error> {
    if let Content::Conversation(raw) = event.content {
        use sigil_protocol::conversation::{Action, Body, Operation};
        let operation = Operation::from_bytes(raw).map_err(|_| Error::InvalidEvent)?;
        if operation.ephemeral() {
            return Err(Error::Obsolete);
        }
        if let Action::Post {
            body: Body::File(raw),
            ..
        }
        | Action::Edit {
            body: Body::File(raw),
            ..
        } = operation.action
        {
            if sigil_protocol::file::File::from_bytes(&raw)
                .map_err(|_| Error::InvalidEvent)?
                .expires_at
                .is_some_and(|v| v <= now)
            {
                return Err(Error::Obsolete);
            }
        }
    }
    if event.group != g.group || event.timestamp < g.from || event.timestamp >= g.until {
        return Err(Error::Obsolete);
    }
    if let Content::File(raw) = event.content {
        let file = sigil_protocol::file::File::from_bytes(raw).map_err(|_| Error::InvalidEvent)?;
        if file.expires_at.is_some_and(|v| v <= now) {
            return Err(Error::Obsolete);
        }
    }
    Ok(())
}
impl ClientStore {
    pub fn prepare_group_history_share(
        &mut self,
        group: Id,
        id: Id,
        range: HistoryRange,
        now: u64,
    ) -> Result<(), Error> {
        if now == 0
            || range.until_timestamp > now
            || range.expires_at <= now
            || range.expires_at > now.saturating_add(604800)
        {
            return Err(Error::Expired);
        }
        self.refresh_group_authority_for_send(group, now)?;
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = keys::current(&tx, &self.key, &own, &group)?;
        let mut grant = Grant {
            id,
            group,
            head: state.head,
            epoch: state.epoch,
            issuer: own,
            source: range.source_device,
            target: range.recipient_device,
            member: state.device(range.recipient_device)?.0.id,
            from: range.from_timestamp,
            until: range.until_timestamp,
            expires: range.expires_at,
            signature: [0; 64],
        };
        if let Ok(w) = load(&tx, &self.key, &own, &id) {
            if Grant::parse(&w.grant)?.unsigned()? != grant.unsigned()? {
                return Err(Error::Conflict);
            }
            return Ok(());
        }
        grant.signature = crate::handshake::identity(&tx, &self.key)?.sign(&grant.statement()?)?;
        grant.verify(&state)?;
        ensure(&tx, &self.key, &own, &grant.bytes()?)?;
        tx.commit()?;
        Ok(())
    }
    pub fn group_history_share_status(
        &mut self,
        id: Id,
    ) -> Result<(HistoryShareStatus, u64), Error> {
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let w = load(&self.db, &self.key, &own, &id)?;
        Ok((w.status, w.count))
    }
    pub fn shared_group_history(
        &mut self,
        group: Id,
        after: Option<Id>,
    ) -> Result<Vec<SharedGroupHistory>, Error> {
        let rows=self.db.prepare("SELECT id FROM group_shared_history WHERE group_id=?1 AND id>?2 ORDER BY id LIMIT 32")?.query_map((group.as_slice(),after.map(|v|v.to_vec()).unwrap_or_default()),|r|r.get::<_,Vec<u8>>(0))?.collect::<Result<Vec<_>,_>>()?;
        rows.into_iter()
            .map(|id| {
                let id: Id = id.try_into().map_err(|_| Error::InvalidStore)?;
                self.shared_group_history_record(group, id)
            })
            .collect()
    }
    pub fn shared_group_history_record(
        &mut self,
        group: Id,
        id: Id,
    ) -> Result<SharedGroupHistory, Error> {
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let raw: Vec<u8> = self.db.query_row("SELECT CASE WHEN length(state)<=66000 THEN state END FROM group_shared_history WHERE group_id=?1 AND id=?2", (group.as_slice(), id.as_slice()), |r| r.get(0)).optional()?.ok_or(Error::NotFound)?;
        let raw = storage_record::open_record(&self.key, &raw, &shared_aad(&own, &id))?;
        if raw.len() < 96 || raw[..32] != group {
            return Err(Error::InvalidStore);
        }
        let event = Group::from_bytes(&raw[96..]).map_err(|_| Error::InvalidStore)?;
        if event.group != group {
            return Err(Error::InvalidStore);
        }
        Ok(SharedGroupHistory {
            id,
            source_device: raw[32..64].try_into().map_err(|_| Error::InvalidStore)?,
            grant: raw[64..96].try_into().map_err(|_| Error::InvalidStore)?,
            plaintext: Zeroizing::new(raw[96..].to_vec()),
        })
    }
}
#[path = "group_history_work.rs"]
pub(super) mod work;
pub(crate) use work::cancelled;
pub(crate) fn sent(
    tx: &Transaction<'_>,
    key: &StorageKey,
    session: &Id,
    id: &Id,
) -> Result<(), Error> {
    let group: Option<Vec<u8>> = tx
        .query_row(
            "SELECT group_id FROM group_key_outbox WHERE id=?1",
            [id.as_slice()],
            |r| r.get(0),
        )
        .optional()?;
    let Some(group) = group else {
        return Ok(());
    };
    let group: Id = group.try_into().map_err(|_| Error::InvalidStore)?;
    let own = device_fingerprint(&peers::own(tx, key)?)?;
    let (expected, receipt) = keys::job(tx, key, &own, &group, id)?.ok_or(Error::InvalidStore)?;
    if expected != *session {
        return Err(Error::Conflict);
    }
    if receipt.kind != 3 {
        return Ok(());
    }
    let mut w = load(tx, key, &own, &receipt.context.chain)?;
    let offer = w.offers.contains(id);
    let terminal = matches!(
        w.status,
        HistoryShareStatus::Complete
            | HistoryShareStatus::Unavailable
            | HistoryShareStatus::Authorized
    );
    if offer || (terminal && w.active == Some(*id) && w.packet.is_none()) {
        w.offers.retain(|v| v != id);
        if w.active == Some(*id) {
            w.active = None;
        }
        key_recovery::forget_job(tx, key, &own, &group, Some(*id))?;
        save(tx, key, &own, &w)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "group_history_tests.rs"]
mod tests;
