//! Pages on this device: the sealed local cache and unsent work of each open page.
use crate::page_sync::{Doc, Inflight, PageSession, Signer};
use crate::{storage_blob, ClientStore, Error, Id};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sigil_pages::{Op, Page};
use sigil_protocol::pages::Grant;

pub(crate) const MIGRATION: &str = "
CREATE TABLE IF NOT EXISTS page_cache(id BLOB NOT NULL CHECK(length(id)=32), part INTEGER NOT NULL, data BLOB NOT NULL, PRIMARY KEY(id,part));
CREATE TABLE IF NOT EXISTS page_work(id BLOB PRIMARY KEY CHECK(length(id)=32), state BLOB NOT NULL);
PRAGMA user_version=87;";
/// Plaintext per cache row.
const PART: usize = 1024 * 1024;
const MAX_PARTS: usize = 64;
/// Tail entries before the cache is rewritten.
const MAX_TAIL: usize = 64;
const MAX_TAIL_BYTES: usize = 256 * 1024;

/// Server-ordered state at `head` and the window a later op may need.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cache {
    revision: [u8; 16],
    head: u64,
    snapshot_seq: u64,
    base: Page,
    window: Vec<(u64, Vec<Op>)>,
}
/// Unsent work and the batches received since the cache.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Work {
    revision: [u8; 16],
    tail: Vec<(u64, Vec<Op>)>,
    inflight: Option<Inflight>,
    pending: Vec<Op>,
    cursor: u32,
}

fn binding(kind: &[u8], id: &Id, part: u32) -> Vec<u8> {
    [
        b"Sigil/client/page-".as_slice(),
        kind,
        b"/v0",
        id,
        &part.to_be_bytes(),
    ]
    .concat()
}
fn encode(value: &impl Serialize) -> Result<Vec<u8>, Error> {
    serde_json::to_vec(value).map_err(|_| Error::InvalidStore)
}
fn write_cache(
    tx: &Transaction<'_>,
    key: &sigil_crypto::storage::StorageKey,
    id: &Id,
    cache: &Cache,
) -> Result<(), Error> {
    let bytes = zeroize::Zeroizing::new(encode(cache)?);
    let parts: Vec<&[u8]> = bytes.chunks(PART).collect();
    if parts.len() > MAX_PARTS {
        return Err(Error::Limit);
    }
    tx.execute("DELETE FROM page_cache WHERE id=?1", [id.as_slice()])?;
    for (index, part) in parts.iter().enumerate() {
        let mut framed = (parts.len() as u32).to_be_bytes().to_vec();
        framed.extend_from_slice(part);
        let sealed = storage_blob::seal(key, &binding(b"cache", id, index as u32), &framed)?;
        tx.execute(
            "INSERT INTO page_cache VALUES(?1,?2,?3)",
            (id.as_slice(), index as i64, sealed),
        )?;
    }
    Ok(())
}
fn read_cache(
    db: &rusqlite::Connection,
    key: &sigil_crypto::storage::StorageKey,
    id: &Id,
) -> Result<Option<Cache>, Error> {
    let rows: Vec<(i64, Vec<u8>)> = db
        .prepare("SELECT part,data FROM page_cache WHERE id=?1 ORDER BY part")?
        .query_map([id.as_slice()], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    if rows.is_empty() {
        return Ok(None);
    }
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    for (index, (part, data)) in rows.iter().enumerate() {
        let framed = storage_blob::open(key, &binding(b"cache", id, index as u32), data, PART + 4)?;
        let count = u32::from_be_bytes(
            framed
                .get(..4)
                .ok_or(Error::InvalidStore)?
                .try_into()
                .map_err(|_| Error::InvalidStore)?,
        );
        if *part != index as i64 || count as usize != rows.len() {
            return Err(Error::InvalidStore);
        }
        bytes.extend_from_slice(&framed[4..]);
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| Error::InvalidStore)
}

impl ClientStore {
    /// This device's batch signer: its identity key and server device id.
    pub fn page_signer(&mut self) -> Result<Signer, Error> {
        let device = self
            .connection_session()?
            .ok_or(Error::Unprepared)?
            .device_id;
        let device = crate::connection::decode_id(&device)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let identity = crate::handshake::identity(&tx, &self.key)?;
        tx.commit()?;
        Ok(Signer::new(identity, device))
    }
    /// Persists a session's unsent work; the cache is rewritten only when the tail grows. Callers debounce.
    pub fn page_save(&mut self, session: &mut PageSession) -> Result<(), Error> {
        let id = session.id.0;
        let doc = &session.doc;
        let mut tail: Vec<(u64, Vec<Op>)> = match session.cache {
            Some((_, head)) => doc
                .window
                .iter()
                .filter(|(s, _)| *s > head)
                .cloned()
                .collect(),
            None => Vec::new(),
        };
        let rewrite = session.cache.is_none_or(|(_, head)| head > doc.head)
            || tail.len() > MAX_TAIL
            || encode(&tail)?.len() > MAX_TAIL_BYTES;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut revision = session.cache.map(|(r, _)| r).unwrap_or_default();
        if rewrite {
            getrandom::fill(&mut revision).map_err(|_| sigil_crypto::Error::Entropy)?;
            let cache = Cache {
                revision,
                head: doc.head,
                snapshot_seq: doc.snapshot_seq,
                base: doc.base.clone(),
                window: doc.window.clone(),
            };
            write_cache(&tx, &self.key, &id, &cache)?;
            tail.clear();
        }
        let work = Work {
            revision,
            tail,
            inflight: doc.inflight.clone(),
            pending: doc.pending.clone(),
            cursor: doc.cursor,
        };
        let bytes = zeroize::Zeroizing::new(encode(&work)?);
        let sealed = storage_blob::seal(&self.key, &binding(b"work", &id, 0), &bytes)?;
        tx.execute("INSERT INTO page_work VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET state=excluded.state", (id.as_slice(), sealed))?;
        tx.commit()?;
        if rewrite {
            session.cache = Some((revision, doc.head));
        }
        session.unsaved = false;
        Ok(())
    }
    /// A session from what this device kept, without the network; `None` when nothing is kept.
    pub fn page_resume(&self, grant: &Grant, signer: Signer) -> Result<Option<PageSession>, Error> {
        let id = grant.id;
        let Some(cache) = read_cache(&self.db, &self.key, &id)? else {
            return Ok(None);
        };
        let sealed: Option<Vec<u8>> = self
            .db
            .query_row(
                "SELECT state FROM page_work WHERE id=?1",
                [id.as_slice()],
                |r| r.get(0),
            )
            .optional()?;
        let work: Work = match sealed {
            Some(sealed) => {
                let bytes = storage_blob::open(
                    &self.key,
                    &binding(b"work", &id, 0),
                    &sealed,
                    2 * 1024 * 1024,
                )?;
                serde_json::from_slice(&bytes).map_err(|_| Error::InvalidStore)?
            }
            None => return Err(Error::InvalidStore),
        };
        if work.revision != cache.revision {
            return Err(Error::InvalidStore);
        }
        let mut doc = Doc::new(cache.base, cache.head);
        doc.snapshot_seq = cache.snapshot_seq;
        doc.window = cache.window;
        for (seq, ops) in work.tail {
            if seq != doc.head + 1 || sigil_pages::apply_batch(&mut doc.base, &ops).is_err() {
                return Err(Error::InvalidStore);
            }
            doc.window.push((seq, ops));
            doc.head = seq;
        }
        doc.inflight = work.inflight;
        doc.pending = work.pending;
        doc.cursor = work.cursor;
        doc.rebuild().map_err(|_| Error::InvalidStore)?;
        let mut session =
            PageSession::from_doc(grant, signer, doc).map_err(|_| Error::InvalidStore)?;
        session.cache = Some((cache.revision, cache.head));
        Ok(Some(session))
    }
    /// Drops what this device kept of a page.
    pub fn page_forget(&mut self, id: Id) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM page_cache WHERE id=?1", [id.as_slice()])?;
        tx.execute("DELETE FROM page_work WHERE id=?1", [id.as_slice()])?;
        tx.commit()?;
        Ok(())
    }
}
