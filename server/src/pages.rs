//! Shared page op logs: opaque sealed ops and snapshots gated by capability token hashes.
use crate::{
    push_config::{optional_unsigned, sql, unsigned},
    store::{Store, StoreError},
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

#[cfg(test)]
#[path = "page_tests.rs"]
mod tests;

pub(crate) const MAX_OP: usize = 64 * 1024;
pub(crate) const MAX_SNAPSHOT: usize = sigil_protocol::pages::MAX_SNAPSHOT_BYTES;
pub(crate) const MAX_LABEL: usize = 512;
pub(crate) const MAX_PRESENCE: usize = 512;
/// Nonce and tag of the smallest sealed body.
pub(crate) const MIN_SEALED: usize = 28;
pub(crate) const MAX_LIVE: i64 = 4096;
pub(crate) const MAX_PAGE_OPS: usize = 512;
pub(crate) const MAX_SLOTS: i64 = 64;
pub(crate) const PRESENCE_SECONDS: u64 = 30;
pub(crate) const VERSIONS: i64 = 50;
pub(crate) const COMPACT_AFTER: u64 = 30 * 86400;
pub(crate) const PAGE: u64 = 2048;
pub(crate) const ROW: u64 = 256;
const BATCH: i64 = 512;

pub(crate) const MIGRATION: &str = "
CREATE TABLE IF NOT EXISTS pages(id TEXT PRIMARY KEY, owner TEXT NOT NULL REFERENCES accounts(id), access_hash BLOB NOT NULL,
  manage_hash BLOB NOT NULL, epoch INTEGER NOT NULL, head INTEGER NOT NULL, bytes INTEGER NOT NULL,
  created INTEGER NOT NULL, updated INTEGER NOT NULL, expires_at INTEGER, deleted INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS page_ops(page TEXT NOT NULL REFERENCES pages(id), seq INTEGER NOT NULL, base INTEGER NOT NULL,
  epoch INTEGER NOT NULL, created INTEGER NOT NULL, body BLOB NOT NULL, request TEXT NOT NULL, PRIMARY KEY(page, seq));
CREATE UNIQUE INDEX IF NOT EXISTS page_ops_request ON page_ops(page, request);
CREATE TABLE IF NOT EXISTS page_snapshots(page TEXT NOT NULL REFERENCES pages(id), version INTEGER NOT NULL, seq INTEGER NOT NULL,
  epoch INTEGER NOT NULL, created INTEGER NOT NULL, body BLOB NOT NULL, label BLOB NOT NULL DEFAULT x'', PRIMARY KEY(page, version));
CREATE TABLE IF NOT EXISTS page_presence(page TEXT NOT NULL REFERENCES pages(id), slot BLOB NOT NULL, body BLOB NOT NULL,
  updated INTEGER NOT NULL, PRIMARY KEY(page, slot));
CREATE INDEX IF NOT EXISTS pages_owner ON pages(owner, deleted);
CREATE INDEX IF NOT EXISTS pages_expiry ON pages(expires_at) WHERE deleted=0 AND expires_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS pages_purge ON pages(id) WHERE deleted=1 AND bytes>0;
CREATE INDEX IF NOT EXISTS page_presence_updated ON page_presence(updated);
";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CreatePage {
    pub id: String,
    pub access_hash: String,
    pub manage_hash: String,
    #[serde(default)]
    pub expires_at: Option<u64>,
    pub snapshot: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PageInfo {
    pub epoch: u64,
    pub head: u64,
    pub snapshot_seq: u64,
    pub bytes: u64,
    pub expires_at: Option<u64>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppendOp {
    pub epoch: u64,
    pub base: u64,
    pub request: String,
    pub body: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Op {
    pub seq: u64,
    pub base: u64,
    pub epoch: u64,
    pub created: u64,
    pub request: String,
    pub body: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Appended {
    pub seq: u64,
    pub missed: Vec<Op>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Ops {
    pub epoch: u64,
    pub head: u64,
    pub ops: Vec<Op>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PutSnapshot {
    pub epoch: u64,
    pub seq: u64,
    pub body: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub version: u64,
    pub seq: u64,
    pub epoch: u64,
    pub created: u64,
    pub body: String,
    pub label: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Version {
    pub version: u64,
    pub seq: u64,
    pub epoch: u64,
    pub created: u64,
    pub bytes: u64,
    pub label: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Label {
    pub label: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Presence {
    pub slot: String,
    pub body: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PresenceEntry {
    pub slot: String,
    pub body: String,
    pub updated: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Rotate {
    pub epoch: u64,
    pub seq: u64,
    pub access_hash: String,
    pub manage_hash: String,
    pub snapshot: String,
}

#[derive(Debug)]
pub enum PageError {
    Store(StoreError),
    NotFound,
    Removed,
    Denied,
    Exists,
    WrongEpoch,
    BaseAhead,
    BaseBeforeSnapshot,
    Orphans,
    HeadMoved,
    Compacted,
    RequestReused,
    Quota,
    PageLimit,
    Busy,
    RateLimited,
    Invalid(&'static str),
}
impl From<StoreError> for PageError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}
impl From<rusqlite::Error> for PageError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Store(StoreError::Database(error))
    }
}
type Result<T> = std::result::Result<T, PageError>;

/// A capability token as sent in a header; only its SHA-256 is ever stored.
pub struct Token([u8; 32]);
impl Token {
    pub fn parse(value: &str) -> Option<Self> {
        let bytes = decode(value, 32, 32).ok()?;
        Some(Self(bytes.try_into().ok()?))
    }
    fn hash(&self) -> [u8; 32] {
        Sha256::digest(self.0).into()
    }
}
#[derive(Clone, Copy)]
pub enum Kind {
    Access,
    Manage,
}

struct Row {
    owner: String,
    access: Vec<u8>,
    manage: Vec<u8>,
    epoch: u64,
    head: u64,
    bytes: u64,
    expires_at: Option<u64>,
    deleted: bool,
}

fn decode(value: &str, min: usize, max: usize) -> Result<Vec<u8>> {
    if !value.len().is_multiple_of(2)
        || !(min * 2..=max * 2).contains(&value.len())
        || !value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(PageError::Invalid("invalid page field"));
    }
    Ok(value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| (nibble(pair[0]) << 4) | nibble(pair[1]))
        .collect())
}
fn nibble(b: u8) -> u8 {
    if b.is_ascii_digit() { b - b'0' } else { b - b'a' + 10 }
}
fn hex(bytes: &[u8]) -> String {
    crate::federation_auth::hex(bytes)
}
fn valid_id(id: &str) -> Result<()> {
    if sigil_protocol::accounts::valid_credential(id) { Ok(()) } else { Err(PageError::NotFound) }
}
/// 16 bytes of lowercase hex, the only form clients decode.
fn valid_request(name: &str) -> bool {
    decode(name, 16, 16).is_ok()
}
fn row(db: &Connection, id: &str) -> Result<Option<Row>> {
    Ok(db
        .query_row(
            "SELECT owner,access_hash,manage_hash,epoch,head,bytes,expires_at,deleted FROM pages WHERE id=?1",
            [id],
            |r| {
                Ok(Row {
                    owner: r.get(0)?,
                    access: r.get(1)?,
                    manage: r.get(2)?,
                    epoch: unsigned(r, 3)?,
                    head: unsigned(r, 4)?,
                    bytes: unsigned(r, 5)?,
                    expires_at: optional_unsigned(r, 6)?,
                    deleted: r.get(7)?,
                })
            },
        )
        .optional()?)
}
/// Device credential, then a live page, then the token for this route.
fn open(db: &Connection, credential: &str, id: &str, token: &Token, kind: Kind, now: u64) -> Result<(String, Row)> {
    let device = crate::prekeys::authorize(db, credential, now)?;
    valid_id(id)?;
    let page = row(db, id)?.ok_or(PageError::NotFound)?;
    if page.deleted || page.expires_at.is_some_and(|e| e <= now) {
        return Err(PageError::Removed);
    }
    let expected = match kind {
        Kind::Access => &page.access,
        Kind::Manage => &page.manage,
    };
    if !bool::from(expected.as_slice().ct_eq(&token.hash())) {
        return Err(PageError::Denied);
    }
    Ok((device, page))
}
fn newest_snapshot(db: &Connection, id: &str) -> Result<Option<u64>> {
    Ok(db.query_row("SELECT max(seq) FROM page_snapshots WHERE page=?1", [id], |r| optional_unsigned(r, 0))?)
}
fn info(db: &Connection, id: &str, page: &Row) -> Result<PageInfo> {
    Ok(PageInfo {
        epoch: page.epoch,
        head: page.head,
        snapshot_seq: newest_snapshot(db, id)?.unwrap_or(0),
        bytes: page.bytes,
        expires_at: page.expires_at,
    })
}
fn between(db: &Connection, id: &str, after: u64, before: u64) -> Result<Vec<Op>> {
    Ok(db
        .prepare("SELECT seq,base,epoch,created,request,body FROM page_ops WHERE page=?1 AND seq>?2 AND seq<?3 ORDER BY seq LIMIT ?4")?
        .query_map((id, sql(after)?, sql(before.min(i64::MAX as u64))?, MAX_PAGE_OPS as i64), |r| {
            Ok(Op { seq: unsigned(r, 0)?, base: unsigned(r, 1)?, epoch: unsigned(r, 2)?, created: unsigned(r, 3)?, request: r.get(4)?, body: hex(&r.get::<_, Vec<u8>>(5)?) })
        })?
        .collect::<std::result::Result<_, _>>()?)
}
fn reserve(tx: &Transaction<'_>, owner: &str, bytes: u64, now: u64) -> Result<()> {
    crate::storage_budget::reserve(tx, owner, bytes, now).map_err(|e| match e {
        StoreError::Busy => PageError::Quota,
        e => e.into(),
    })
}
fn release(tx: &Transaction<'_>, owner: &str, bytes: u64) -> Result<()> {
    if bytes > 0
        && tx.execute(
            "UPDATE retained_storage SET bytes=bytes-?2 WHERE account_id=?1 AND bytes>=?2",
            (owner, sql(bytes)?),
        )? != 1
    {
        return Err(StoreError::InvalidData.into());
    }
    Ok(())
}
/// Drops unlabelled versions beyond the newest kept ones and refunds them.
fn trim(tx: &Transaction<'_>, id: &str, owner: &str) -> Result<()> {
    let (rows, bytes): (u64, u64) = tx.query_row(
        "SELECT count(*),coalesce(sum(length(body)+length(label)),0) FROM page_snapshots WHERE page=?1 AND length(label)=0 AND version NOT IN (SELECT version FROM page_snapshots WHERE page=?1 ORDER BY version DESC LIMIT ?2)",
        (id, VERSIONS),
        |r| Ok((unsigned(r, 0)?, unsigned(r, 1)?)),
    )?;
    if rows == 0 {
        return Ok(());
    }
    tx.execute("DELETE FROM page_snapshots WHERE page=?1 AND length(label)=0 AND version NOT IN (SELECT version FROM page_snapshots WHERE page=?1 ORDER BY version DESC LIMIT ?2)", (id, VERSIONS))?;
    tx.execute("UPDATE pages SET bytes=bytes-?2 WHERE id=?1", (id, sql(bytes)?))?;
    release(tx, owner, rows * ROW + bytes)
}
fn record_snapshot(tx: &Transaction<'_>, id: &str, seq: u64, epoch: u64, body: &[u8], now: u64) -> Result<u64> {
    let version: u64 = tx.query_row("SELECT coalesce(max(version),0)+1 FROM page_snapshots WHERE page=?1", [id], |r| unsigned(r, 0))?;
    tx.execute(
        "INSERT INTO page_snapshots(page,version,seq,epoch,created,body) VALUES(?1,?2,?3,?4,?5,?6)",
        (id, sql(version)?, sql(seq)?, sql(epoch)?, sql(now)?, body),
    )?;
    tx.execute("UPDATE pages SET bytes=bytes+?2,updated=?3 WHERE id=?1", (id, body.len() as i64, sql(now)?))?;
    Ok(version)
}
fn snapshot_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Snapshot> {
    Ok(Snapshot {
        version: unsigned(r, 0)?,
        seq: unsigned(r, 1)?,
        epoch: unsigned(r, 2)?,
        created: unsigned(r, 3)?,
        body: hex(&r.get::<_, Vec<u8>>(4)?),
        label: hex(&r.get::<_, Vec<u8>>(5)?),
    })
}
/// Removes one bounded batch of a page's stored rows and refunds them.
fn purge(tx: &Transaction<'_>, id: &str, owner: &str, seq: Option<u64>, created: Option<u64>) -> Result<usize> {
    let (seq, created) = (sql(seq.unwrap_or(i64::MAX as u64))?, sql(created.unwrap_or(i64::MAX as u64))?);
    let selected = "SELECT seq FROM page_ops WHERE page=?1 AND seq<=?2 AND created<=?3 ORDER BY seq LIMIT ?4";
    let (rows, bytes): (u64, u64) = tx.query_row(
        &format!("SELECT count(*),coalesce(sum(length(body)),0) FROM page_ops WHERE page=?1 AND seq IN ({selected})"),
        (id, seq, created, BATCH),
        |r| Ok((unsigned(r, 0)?, unsigned(r, 1)?)),
    )?;
    if rows > 0 {
        tx.execute(&format!("DELETE FROM page_ops WHERE page=?1 AND seq IN ({selected})"), (id, seq, created, BATCH))?;
        tx.execute("UPDATE pages SET bytes=bytes-?2 WHERE id=?1", (id, sql(bytes)?))?;
        release(tx, owner, rows * ROW + bytes)?;
    }
    Ok(rows as usize)
}

/// Bounded maintenance: expiry, purge of removed pages, compaction and stale presence.
pub(crate) fn cleanup(tx: &Transaction<'_>, now: u64) -> std::result::Result<usize, StoreError> {
    let flatten = |e: PageError| match e {
        PageError::Store(e) => e,
        _ => StoreError::InvalidData,
    };
    (|| -> Result<usize> {
        let now_sql = sql(now)?;
        let mut changed = tx.execute(
            "UPDATE pages SET deleted=1 WHERE id IN (SELECT id FROM pages WHERE deleted=0 AND expires_at<=?1 LIMIT 64)",
            [now_sql],
        )?;
        changed += tx.execute("DELETE FROM page_presence WHERE rowid IN (SELECT p.rowid FROM page_presence p JOIN pages g ON g.id=p.page WHERE g.deleted=1 LIMIT 256)", [])?;
        let doomed: Vec<(String, String)> = tx
            .prepare("SELECT id,owner FROM pages WHERE deleted=1 AND bytes>0 LIMIT 4")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        for (id, owner) in doomed {
            changed += purge(tx, &id, &owner, None, None)?;
            let (rows, bytes): (u64, u64) = tx.query_row(
                "SELECT count(*),coalesce(sum(length(body)+length(label)),0) FROM page_snapshots WHERE page=?1 AND version IN (SELECT version FROM page_snapshots WHERE page=?1 ORDER BY version LIMIT 64)",
                [&id],
                |r| Ok((unsigned(r, 0)?, unsigned(r, 1)?)),
            )?;
            if rows > 0 {
                tx.execute("DELETE FROM page_snapshots WHERE page=?1 AND version IN (SELECT version FROM page_snapshots WHERE page=?1 ORDER BY version LIMIT 64)", [&id])?;
                tx.execute("UPDATE pages SET bytes=bytes-?2 WHERE id=?1", (&id, sql(bytes)?))?;
                release(tx, &owner, rows * ROW + bytes)?;
                changed += rows as usize;
            }
        }
        // A random starting point visits every page over time without a stored cursor.
        let mut start = [0u8; 32];
        getrandom::fill(&mut start).map_err(|_| StoreError::InvalidData)?;
        let start = hex(&start);
        let mut live: Vec<(String, String)> = tx
            .prepare("SELECT id,owner FROM pages WHERE deleted=0 AND id>=?1 ORDER BY id LIMIT 16")?
            .query_map([&start], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        if live.len() < 16 {
            live.extend(
                tx.prepare("SELECT id,owner FROM pages WHERE deleted=0 AND id<?1 ORDER BY id LIMIT ?2")?
                    .query_map((&start, 16 - live.len() as i64), |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<std::result::Result<Vec<_>, _>>()?,
            );
        }
        for (id, owner) in live {
            if let Some(seq) = newest_snapshot(tx, &id)? {
                changed += purge(tx, &id, &owner, Some(seq), Some(now.saturating_sub(COMPACT_AFTER)))?;
            }
        }
        changed += tx.execute(
            "DELETE FROM page_presence WHERE rowid IN (SELECT rowid FROM page_presence WHERE updated<=?1 ORDER BY updated LIMIT 256)",
            [sql(now.saturating_sub(PRESENCE_SECONDS))?],
        )?;
        Ok(changed)
    })()
    .map_err(flatten)
}

/// Charges of every page, for a storage rebuild.
pub(crate) fn rebuild(db: &Connection) -> std::result::Result<(), StoreError> {
    if db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='pages')", [], |r| r.get::<_, bool>(0))? {
        db.execute("UPDATE retained_storage SET bytes=bytes+(SELECT coalesce(sum(?1+bytes),0) FROM pages WHERE owner=retained_storage.account_id)+?2*((SELECT count(*) FROM page_ops o JOIN pages p ON p.id=o.page WHERE p.owner=retained_storage.account_id)+(SELECT count(*) FROM page_snapshots s JOIN pages p ON p.id=s.page WHERE p.owner=retained_storage.account_id))", (PAGE as i64, ROW as i64))?;
    }
    Ok(())
}

/// Removes an account's pages; tombstones stay so their IDs cannot return.
pub(crate) fn delete_account(tx: &Transaction<'_>, account: &str) -> std::result::Result<(), StoreError> {
    tx.execute("UPDATE pages SET deleted=1 WHERE owner=?1 AND deleted=0", [account])?;
    tx.execute("DELETE FROM page_presence WHERE page IN (SELECT id FROM pages WHERE owner=?1)", [account])?;
    Ok(())
}

impl Store {
    pub fn page_create(&mut self, credential: &str, request: CreatePage, now: u64) -> Result<PageInfo> {
        valid_id(&request.id).map_err(|_| PageError::Invalid("invalid page id"))?;
        let access = decode(&request.access_hash, 32, 32)?;
        let manage = decode(&request.manage_hash, 32, 32)?;
        if access == manage {
            return Err(PageError::Invalid("access and manage tokens must differ"));
        }
        let body = decode(&request.snapshot, MIN_SEALED, MAX_SNAPSHOT)?;
        if request.expires_at.is_some_and(|e| e <= now || e > i64::MAX as u64) {
            return Err(PageError::Invalid("invalid page expiry"));
        }
        let tx = self.0.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let device = crate::prekeys::authorize(&tx, credential, now)?;
        let owner: String = tx.query_row("SELECT account_id FROM devices WHERE id=?1", [&device], |r| r.get(0))?;
        if let Some(page) = row(&tx, &request.id)? {
            // A retried create answers as the first one did.
            if page.owner == owner && !page.deleted && page.access == access && page.manage == manage && page.epoch == 1 {
                return info(&tx, &request.id, &page);
            }
            return Err(PageError::Exists);
        }
        let live: i64 = tx.query_row("SELECT count(*) FROM pages WHERE owner=?1 AND deleted=0", [&owner], |r| r.get(0))?;
        if live >= MAX_LIVE {
            return Err(PageError::PageLimit);
        }
        reserve(&tx, &owner, PAGE + ROW + body.len() as u64, now)?;
        tx.execute(
            "INSERT INTO pages VALUES(?1,?2,?3,?4,1,0,0,?5,?5,?6,0)",
            (&request.id, &owner, &access, &manage, sql(now)?, request.expires_at.map(sql).transpose()?),
        )?;
        record_snapshot(&tx, &request.id, 0, 1, &body, now)?;
        let page = row(&tx, &request.id)?.ok_or(StoreError::InvalidData)?;
        let value = info(&tx, &request.id, &page)?;
        tx.commit()?;
        Ok(value)
    }
    pub fn page_info(&mut self, credential: &str, id: &str, token: &Token, now: u64) -> Result<PageInfo> {
        let tx = self.0.transaction()?;
        let (_, page) = open(&tx, credential, id, token, Kind::Access, now)?;
        let value = info(&tx, id, &page)?;
        tx.commit()?;
        Ok(value)
    }
    /// `admit` rate-limits by device once the caller holds the page's access token.
    pub fn page_append(
        &mut self,
        credential: &str,
        id: &str,
        token: &Token,
        request: AppendOp,
        now: u64,
        admit: impl FnOnce(&str) -> bool,
    ) -> Result<Appended> {
        if !valid_request(&request.request) {
            return Err(PageError::Invalid("invalid request name"));
        }
        let body = decode(&request.body, MIN_SEALED, MAX_OP)?;
        let tx = self.0.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (device, page) = open(&tx, credential, id, token, Kind::Access, now)?;
        if !admit(&device) {
            return Err(PageError::RateLimited);
        }
        // A lost reply must not become a second copy of the batch.
        let found: Option<(u64, u64, Vec<u8>)> = tx
            .query_row(
                "SELECT seq,base,body FROM page_ops WHERE page=?1 AND request=?2",
                (id, &request.request),
                |r| Ok((unsigned(r, 0)?, unsigned(r, 1)?, r.get(2)?)),
            )
            .optional()?;
        if let Some((seq, base, stored)) = found {
            if stored != body {
                return Err(PageError::RequestReused);
            }
            return Ok(Appended { seq, missed: between(&tx, id, base, seq)? });
        }
        if request.epoch != page.epoch {
            return Err(PageError::WrongEpoch);
        }
        if request.base > page.head {
            return Err(PageError::BaseAhead);
        }
        if newest_snapshot(&tx, id)?.is_some_and(|s| request.base < s) {
            return Err(PageError::BaseBeforeSnapshot);
        }
        let seq = crate::push_config::next(page.head)?;
        reserve(&tx, &page.owner, ROW + body.len() as u64, now)?;
        tx.execute(
            "INSERT INTO page_ops VALUES(?1,?2,?3,?4,?5,?6,?7)",
            (id, sql(seq)?, sql(request.base)?, sql(page.epoch)?, sql(now)?, &body, &request.request),
        )?;
        tx.execute("UPDATE pages SET head=?2,bytes=bytes+?3,updated=?4 WHERE id=?1", (id, sql(seq)?, body.len() as i64, sql(now)?))?;
        let missed = between(&tx, id, request.base, seq)?;
        tx.commit()?;
        Ok(Appended { seq, missed })
    }
    pub fn page_ops(&mut self, credential: &str, id: &str, token: &Token, after: u64, now: u64) -> Result<Ops> {
        let tx = self.0.transaction()?;
        let (_, page) = open(&tx, credential, id, token, Kind::Access, now)?;
        if after < page.head
            && !tx.query_row("SELECT EXISTS(SELECT 1 FROM page_ops WHERE page=?1 AND seq=?2)", (id, sql(after + 1)?), |r| r.get::<_, bool>(0))?
        {
            return Err(PageError::Compacted);
        }
        let ops = between(&tx, id, after, u64::MAX)?;
        tx.commit()?;
        Ok(Ops { epoch: page.epoch, head: page.head, ops })
    }
    /// `None` when an equal or newer snapshot is already stored.
    pub fn page_put_snapshot(&mut self, credential: &str, id: &str, token: &Token, request: PutSnapshot, now: u64) -> Result<Option<Version>> {
        let body = decode(&request.body, MIN_SEALED, MAX_SNAPSHOT)?;
        let tx = self.0.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (_, page) = open(&tx, credential, id, token, Kind::Access, now)?;
        if request.epoch != page.epoch {
            return Err(PageError::WrongEpoch);
        }
        if request.seq > page.head {
            return Err(PageError::BaseAhead);
        }
        if newest_snapshot(&tx, id)?.is_some_and(|s| s >= request.seq) {
            return Ok(None);
        }
        // An op after this point on an older base could never be carried past it once compacted.
        if tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM page_ops WHERE page=?1 AND seq>?2 AND base<?2)",
            (id, sql(request.seq)?),
            |r| r.get::<_, bool>(0),
        )? {
            return Err(PageError::Orphans);
        }
        reserve(&tx, &page.owner, ROW + body.len() as u64, now)?;
        let version = record_snapshot(&tx, id, request.seq, page.epoch, &body, now)?;
        trim(&tx, id, &page.owner)?;
        tx.commit()?;
        Ok(Some(Version { version, seq: request.seq, epoch: page.epoch, created: now, bytes: body.len() as u64, label: String::new() }))
    }
    /// The newest snapshot, or one version.
    pub fn page_snapshot(&mut self, credential: &str, id: &str, token: &Token, version: Option<u64>, now: u64) -> Result<Snapshot> {
        let tx = self.0.transaction()?;
        open(&tx, credential, id, token, Kind::Access, now)?;
        let columns = "SELECT version,seq,epoch,created,body,label FROM page_snapshots WHERE page=?1";
        let value = match version {
            Some(v) => tx.query_row(&format!("{columns} AND version=?2"), (id, sql(v)?), snapshot_row),
            None => tx.query_row(&format!("{columns} ORDER BY version DESC LIMIT 1"), [id], snapshot_row),
        }
        .optional()?
        .ok_or(PageError::NotFound)?;
        tx.commit()?;
        Ok(value)
    }
    pub fn page_versions(&mut self, credential: &str, id: &str, token: &Token, now: u64) -> Result<Vec<Version>> {
        let tx = self.0.transaction()?;
        open(&tx, credential, id, token, Kind::Access, now)?;
        let value = tx
            .prepare("SELECT version,seq,epoch,created,length(body),label FROM page_snapshots WHERE page=?1 ORDER BY version DESC")?
            .query_map([id], |r| {
                Ok(Version {
                    version: unsigned(r, 0)?,
                    seq: unsigned(r, 1)?,
                    epoch: unsigned(r, 2)?,
                    created: unsigned(r, 3)?,
                    bytes: unsigned(r, 4)?,
                    label: hex(&r.get::<_, Vec<u8>>(5)?),
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        tx.commit()?;
        Ok(value)
    }
    /// An empty label unnames the version, which may then be trimmed.
    pub fn page_label(&mut self, credential: &str, id: &str, token: &Token, version: u64, request: Label, now: u64) -> Result<()> {
        let label = if request.label.is_empty() { Vec::new() } else { decode(&request.label, MIN_SEALED, MAX_LABEL)? };
        let tx = self.0.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (_, page) = open(&tx, credential, id, token, Kind::Access, now)?;
        let old: u64 = tx
            .query_row("SELECT length(label) FROM page_snapshots WHERE page=?1 AND version=?2", (id, sql(version)?), |r| unsigned(r, 0))
            .optional()?
            .ok_or(PageError::NotFound)?;
        let new = label.len() as u64;
        if new > old {
            reserve(&tx, &page.owner, new - old, now)?;
        } else {
            release(&tx, &page.owner, old - new)?;
        }
        tx.execute("UPDATE page_snapshots SET label=?3 WHERE page=?1 AND version=?2", (id, sql(version)?, &label))?;
        tx.execute("UPDATE pages SET bytes=bytes+?2-?3 WHERE id=?1", (id, sql(new)?, sql(old)?))?;
        trim(&tx, id, &page.owner)?;
        tx.commit()?;
        Ok(())
    }
    pub fn page_put_presence(&mut self, credential: &str, id: &str, token: &Token, request: Presence, now: u64) -> Result<()> {
        let slot = decode(&request.slot, 16, 16)?;
        let body = decode(&request.body, MIN_SEALED, MAX_PRESENCE)?;
        let tx = self.0.transaction_with_behavior(TransactionBehavior::Immediate)?;
        open(&tx, credential, id, token, Kind::Access, now)?;
        tx.execute("DELETE FROM page_presence WHERE page=?1 AND updated<=?2", (id, sql(now.saturating_sub(PRESENCE_SECONDS))?))?;
        let (slots, present): (i64, bool) = tx.query_row(
            "SELECT count(*),coalesce(max(slot=?2),0) FROM page_presence WHERE page=?1",
            (id, &slot),
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if !present && slots >= MAX_SLOTS {
            return Err(PageError::Busy);
        }
        tx.execute(
            "INSERT INTO page_presence VALUES(?1,?2,?3,?4) ON CONFLICT(page,slot) DO UPDATE SET body=excluded.body,updated=excluded.updated",
            (id, &slot, &body, sql(now)?),
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn page_presence(&mut self, credential: &str, id: &str, token: &Token, now: u64) -> Result<Vec<PresenceEntry>> {
        let tx = self.0.transaction()?;
        open(&tx, credential, id, token, Kind::Access, now)?;
        let value = tx
            .prepare("SELECT slot,body,updated FROM page_presence WHERE page=?1 AND updated>?2 ORDER BY updated DESC")?
            .query_map((id, sql(now.saturating_sub(PRESENCE_SECONDS))?), |r| {
                Ok(PresenceEntry { slot: hex(&r.get::<_, Vec<u8>>(0)?), body: hex(&r.get::<_, Vec<u8>>(1)?), updated: unsigned(r, 2)? })
            })?
            .collect::<std::result::Result<_, _>>()?;
        tx.commit()?;
        Ok(value)
    }
    /// Compare-and-swap on the epoch; the snapshot must seal the current head.
    pub fn page_rotate(&mut self, credential: &str, id: &str, token: &Token, request: Rotate, now: u64) -> Result<PageInfo> {
        let access = decode(&request.access_hash, 32, 32)?;
        let manage = decode(&request.manage_hash, 32, 32)?;
        if access == manage {
            return Err(PageError::Invalid("access and manage tokens must differ"));
        }
        let body = decode(&request.snapshot, MIN_SEALED, MAX_SNAPSHOT)?;
        let tx = self.0.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (_, page) = open(&tx, credential, id, token, Kind::Manage, now)?;
        if request.epoch != page.epoch {
            return Err(PageError::WrongEpoch);
        }
        if request.seq != page.head {
            return Err(PageError::HeadMoved);
        }
        let epoch = crate::push_config::next(page.epoch)?;
        reserve(&tx, &page.owner, ROW + body.len() as u64, now)?;
        tx.execute("UPDATE pages SET epoch=?2,access_hash=?3,manage_hash=?4 WHERE id=?1", (id, sql(epoch)?, &access, &manage))?;
        record_snapshot(&tx, id, page.head, epoch, &body, now)?;
        tx.execute("DELETE FROM page_presence WHERE page=?1", [id])?;
        trim(&tx, id, &page.owner)?;
        let page = row(&tx, id)?.ok_or(StoreError::InvalidData)?;
        let value = info(&tx, id, &page)?;
        tx.commit()?;
        Ok(value)
    }
    /// Access ends at once; stored rows are purged by bounded maintenance.
    pub fn page_delete(&mut self, credential: &str, id: &str, token: &Token, now: u64) -> Result<()> {
        let tx = self.0.transaction_with_behavior(TransactionBehavior::Immediate)?;
        match open(&tx, credential, id, token, Kind::Manage, now) {
            Ok(_) => {}
            Err(PageError::Removed) => {
                let page = row(&tx, id)?.ok_or(PageError::NotFound)?;
                if !bool::from(page.manage.as_slice().ct_eq(&token.hash())) {
                    return Err(PageError::Denied);
                }
            }
            Err(e) => return Err(e),
        }
        tx.execute("UPDATE pages SET deleted=1,updated=?2 WHERE id=?1", (id, sql(now)?))?;
        tx.execute("DELETE FROM page_presence WHERE page=?1", [id])?;
        tx.commit()?;
        Ok(())
    }
}
