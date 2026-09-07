//! Private ciphertext cache with an independent, caller-persisted page budget.
//! This database is disposable transfer state, never a messaging-state backup.
use crate::{ClientStore, Error, Id};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sigil_crypto::{
    attachment::{CiphertextList, FileKey, Shape, CHUNK_OVERHEAD},
    storage::StorageKey,
    Secret32,
};
use std::path::Path;
use zeroize::Zeroizing;

const MAX_STATE: usize = 4096;
const APP: i64 = 1397178691;
const VAULT: &[u8] = b"Sigil/attachment-cache/v0";
#[path = "attachment_transfer.rs"]
mod transfer;
pub use transfer::UploadStep;
#[path = "attachment_download.rs"]
mod download;
pub use download::DownloadStep;
#[path = "attachment_schedule.rs"]
mod scheduling;
pub use scheduling::{ScheduledTransfers, TransferAttempt, TransferProgress};
#[path = "attachment_events.rs"]
mod events;
#[cfg(test)]
#[path = "attachment_cache_tests.rs"]
mod tests;

pub use sigil_protocol::file::Metadata;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Staging,
    Ready,
    Starting,
    Uploading,
    Checking,
    Restored,
    Published,
    Downloading,
    Complete,
    Evicting,
    Cancelling,
    Cancelled,
    Expired,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Direction {
    Upload,
    Download,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    file: Id,
    length: u64,
    phase: Phase,
    direction: Direction,
    descriptor: Option<Zeroizing<Vec<u8>>>,
    access: Option<Zeroizing<String>>,
    metadata: Option<Metadata>,
    expires_at: Option<u64>,
    upload_deadline: Option<u64>,
    #[serde(default)]
    managed: bool,
}
impl State {
    fn discard(&mut self, phase: Phase) {
        self.phase = phase;
        self.descriptor = None;
        self.access = None;
        self.metadata = None;
    }
    fn shape(&self) -> Shape {
        Shape {
            file: self.file,
            length: self.length,
        }
    }
    fn key(&self) -> Result<(FileKey, Id), Error> {
        Ok(FileKey::from_descriptor(
            self.descriptor.as_ref().ok_or(Error::Cancelled)?,
        )?)
    }
    fn validate(&self) -> Result<(), Error> {
        self.shape().chunks()?;
        if (self.direction == Direction::Download
            && matches!(
                self.phase,
                Phase::Staging
                    | Phase::Ready
                    | Phase::Starting
                    | Phase::Uploading
                    | Phase::Checking
                    | Phase::Restored
                    | Phase::Published
                    | Phase::Cancelling
            ))
            || (self.direction == Direction::Upload
                && matches!(self.phase, Phase::Downloading | Phase::Complete))
        {
            return Err(Error::InvalidStore);
        }
        if self
            .expires_at
            .is_some_and(|v| v == 0 || v > i64::MAX as u64)
            || self
                .upload_deadline
                .is_some_and(|v| v == 0 || v > i64::MAX as u64)
        {
            return Err(Error::InvalidStore);
        }
        if matches!(
            self.phase,
            Phase::Cancelling | Phase::Cancelled | Phase::Expired | Phase::Evicting
        ) {
            if self.descriptor.is_some() || self.access.is_some() || self.metadata.is_some() {
                return Err(Error::InvalidStore);
            }
        } else {
            let (key, root) = self.key()?;
            if key.shape() != self.shape()
                || !self
                    .access
                    .as_ref()
                    .is_some_and(|v| sigil_protocol::accounts::valid_credential(v))
                || (self.phase == Phase::Staging && root != [0; 32])
                || (matches!(
                    self.phase,
                    Phase::Uploading | Phase::Published | Phase::Restored
                ) && self.upload_deadline.is_none())
            {
                return Err(Error::InvalidStore);
            }
            self.metadata
                .as_ref()
                .ok_or(Error::InvalidStore)?
                .validate()
                .map_err(|_| Error::InvalidStore)?;
        }
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Part {
    hash: Id,
    uploaded: bool,
}

/// No Debug implementation: this value carries a file encryption key and capability.
pub struct Descriptor {
    pub bytes: Zeroizing<Vec<u8>>,
    pub access: Zeroizing<String>,
    pub metadata: Metadata,
    pub expires_at: Option<u64>,
}

pub struct Cache {
    db: Connection,
    key: StorageKey,
    scope: Id,
    budget: u64,
}
fn aad(file: &Id, part: Option<u32>) -> Vec<u8> {
    let mut bytes = b"Sigil/attachment-cache-record/v0".to_vec();
    bytes.extend_from_slice(file);
    if let Some(part) = part {
        bytes.extend_from_slice(&part.to_be_bytes());
    }
    bytes
}
fn live(state: &State, now: u64) -> Result<(), Error> {
    if now == 0 || now > i64::MAX as u64 {
        return Err(Error::InvalidEvent);
    }
    if state.expires_at.is_some_and(|v| v <= now) {
        return Err(Error::Expired);
    }
    if matches!(
        state.phase,
        Phase::Cancelling | Phase::Cancelled | Phase::Evicting
    ) {
        return Err(Error::Cancelled);
    }
    if state.phase == Phase::Expired {
        return Err(Error::Expired);
    }
    Ok(())
}
fn load(db: &Connection, key: &StorageKey, file: Id) -> Result<State, Error> {
    let bytes: Vec<u8> = db
        .query_row(
            "SELECT state FROM files WHERE id=?1 AND length(state)<=?2",
            (file.as_slice(), (MAX_STATE + 36) as i64),
            |r| r.get(0),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    let state: State = serde_json::from_slice(&key.open(&bytes, &aad(&file, None))?)
        .map_err(|_| Error::InvalidStore)?;
    state.validate()?;
    if state.file != file {
        return Err(Error::InvalidStore);
    }
    Ok(state)
}
fn save(db: &Connection, key: &StorageKey, state: &State, insert: bool) -> Result<(), Error> {
    state.validate()?;
    let bytes = Zeroizing::new(serde_json::to_vec(state).map_err(|_| Error::InvalidStore)?);
    if bytes.len() > MAX_STATE {
        return Err(Error::Limit);
    }
    let sealed = key.seal(&bytes, &aad(&state.file, None))?;
    let count = if insert {
        db.execute(
            "INSERT INTO files(id,state) VALUES(?1,?2)",
            (state.file.as_slice(), sealed),
        )?
    } else {
        db.execute(
            "UPDATE files SET state=?2 WHERE id=?1",
            (state.file.as_slice(), sealed),
        )?
    };
    if count != 1 {
        return Err(Error::Conflict);
    }
    Ok(())
}
fn part(
    db: &Connection,
    key: &StorageKey,
    shape: Shape,
    index: u32,
) -> Result<Option<(Part, Vec<u8>)>, Error> {
    let size = shape.chunk_length(index)? + CHUNK_OVERHEAD;
    let row: Option<(Vec<u8>, Vec<u8>)> = db.query_row(
        "SELECT state,data FROM chunks WHERE file=?1 AND part=?2 AND length(state)<=256 AND length(data)=?3",
        (shape.file.as_slice(), index, size as i64), |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
    let Some((sealed, data)) = row else {
        if db.query_row(
            "SELECT EXISTS(SELECT 1 FROM chunks WHERE file=?1 AND part=?2)",
            (shape.file.as_slice(), index),
            |r| r.get::<_, bool>(0),
        )? {
            return Err(Error::InvalidStore);
        }
        return Ok(None);
    };
    let record: Part = serde_json::from_slice(&key.open(&sealed, &aad(&shape.file, Some(index)))?)
        .map_err(|_| Error::InvalidStore)?;
    if record.hash != <Id>::from(Sha256::digest(&data)) {
        return Err(Error::InvalidStore);
    }
    Ok(Some((record, data)))
}
fn committed_root(db: &Connection, key: &StorageKey, state: &State) -> Result<Id, Error> {
    let mut list = CiphertextList::new(state.shape())?;
    let mut query =
        db.prepare("SELECT part,state,length(data) FROM chunks WHERE file=?1 ORDER BY part")?;
    let mut rows = query.query([state.file.as_slice()])?;
    let mut expected = 0;
    while let Some(row) = rows.next()? {
        let index: u32 = row.get(0)?;
        let size: i64 = row.get(2)?;
        if index != expected || size != (state.shape().chunk_length(index)? + CHUNK_OVERHEAD) as i64
        {
            return Err(Error::InvalidStore);
        }
        let sealed = row.get_ref(1)?.as_blob().map_err(|_| Error::InvalidStore)?;
        if sealed.len() > 256 {
            return Err(Error::InvalidStore);
        }
        let record: Part =
            serde_json::from_slice(&key.open(sealed, &aad(&state.file, Some(index)))?)
                .map_err(|_| Error::InvalidStore)?;
        list.push_hash(index, record.hash)?;
        expected += 1;
    }
    if expected != state.shape().chunks()? {
        return Err(Error::Unprepared);
    }
    Ok(list.finish()?)
}

impl ClientStore {
    /// Supply a separate private path and persist this budget across launches.
    /// Two GiB accommodates the default one-GiB file plus SQLite overhead;
    /// WAL/checkpoint files need additional filesystem space.
    pub fn open_attachment_cache(&self, path: &Path, bytes: u64) -> Result<Cache, Error> {
        let scope = self.connected_account_scope()?;
        let master = Zeroizing::new(self.key.commitment(&scope, VAULT)?);
        Cache::open(
            path,
            StorageKey::new(Secret32::from_bytes(*master))?,
            scope,
            bytes,
        )
    }
}
impl Cache {
    fn open(path: &Path, key: StorageKey, scope: Id, budget: u64) -> Result<Self, Error> {
        let mut db = crate::private_db::open(path, budget)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type IN ('trigger','view'))",
            [],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(Error::InvalidStore);
        }
        let app: i64 = tx.query_row("PRAGMA application_id", [], |r| r.get(0))?;
        let version: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if app == 0
            && version == 0
            && tx.query_row("SELECT count(*) FROM sqlite_schema", [], |r| {
                r.get::<_, i64>(0)
            })? == 0
        {
            tx.execute_batch("CREATE TABLE vault(id INTEGER PRIMARY KEY CHECK(id=1),state BLOB NOT NULL);
                CREATE TABLE files(id BLOB PRIMARY KEY,state BLOB NOT NULL);
                CREATE TABLE chunks(file BLOB NOT NULL REFERENCES files(id),part INTEGER NOT NULL CHECK(part>=0),state BLOB NOT NULL,data BLOB NOT NULL,PRIMARY KEY(file,part));
                PRAGMA user_version=1;")?;
            tx.pragma_update(None, "application_id", APP)?;
            tx.execute("INSERT INTO vault VALUES(1,?1)", [key.seal(&scope, VAULT)?])?;
        } else if app != APP || !(1..=3).contains(&version) {
            return Err(Error::InvalidStore);
        }
        let verifier: Vec<u8> = tx.query_row(
            "SELECT state FROM vault WHERE id=1 AND length(state)<128",
            [],
            |r| r.get(0),
        )?;
        if key.open(&verifier, VAULT)?.as_slice() != scope {
            return Err(Error::InvalidStore);
        }
        if version < 2 {
            tx.execute_batch("CREATE TABLE sync_schedule(id INTEGER PRIMARY KEY CHECK(id=1),state BLOB NOT NULL);
                CREATE TABLE transfer_cursor(id INTEGER PRIMARY KEY CHECK(id=1),state BLOB NOT NULL);
                PRAGMA user_version=2;")?;
        }
        if version < 3 {
            // Older adapters must not ignore archive-managed cache retention.
            tx.pragma_update(None, "user_version", 3)?;
        }
        tx.commit()?;
        let mode: String = db.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
        if mode != "wal" {
            return Err(Error::InvalidStore);
        }
        Ok(Self {
            db,
            key,
            scope,
            budget,
        })
    }
    pub fn prepare_upload(
        &mut self,
        length: u64,
        metadata: Metadata,
        expires_at: Option<u64>,
    ) -> Result<Id, Error> {
        metadata.validate().map_err(|_| Error::InvalidEvent)?;
        if expires_at.is_some_and(|v| v == 0 || v > i64::MAX as u64) {
            return Err(Error::InvalidEvent);
        }
        let file_key = FileKey::generate(length)?;
        if file_key.shape().ciphertext_length()? > self.budget {
            return Err(Error::Limit);
        }
        let mut access = Zeroizing::new([0; 32]);
        getrandom::fill(access.as_mut()).map_err(|_| sigil_crypto::Error::Entropy)?;
        let state = State {
            file: file_key.shape().file,
            length,
            phase: Phase::Staging,
            direction: Direction::Upload,
            descriptor: Some(file_key.descriptor([0; 32])),
            access: Some(Zeroizing::new(crate::transport::hex(access.as_ref()))),
            metadata: Some(metadata),
            expires_at,
            upload_deadline: None,
            managed: false,
        };
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        save(&tx, &self.key, &state, true)?;
        tx.commit()?;
        Ok(state.file)
    }
    pub fn phase(&self, file: Id) -> Result<Phase, Error> {
        Ok(load(&self.db, &self.key, file)?.phase)
    }
    /// Freeze one exact chunk. Changed bytes under an existing index conflict.
    pub fn stage_chunk(&mut self, file: Id, index: u32, plaintext: &[u8]) -> Result<Id, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = load(&tx, &self.key, file)?;
        if state.direction != Direction::Upload {
            return Err(Error::Conflict);
        }
        let (file_key, _) = state.key()?;
        if plaintext.len() != state.shape().chunk_length(index)? {
            return Err(Error::InvalidEvent);
        }
        if let Some((record, data)) = part(&tx, &self.key, state.shape(), index)? {
            if file_key.open_chunk(index, &data)?.as_slice() != plaintext {
                return Err(Error::Conflict);
            }
            return Ok(record.hash);
        }
        if state.phase != Phase::Staging {
            return Err(Error::Conflict);
        }
        let data = file_key.seal_chunk(index, plaintext)?;
        let hash = Sha256::digest(&data).into();
        let record = self.key.seal(
            &serde_json::to_vec(&Part {
                hash,
                uploaded: false,
            })
            .map_err(|_| Error::InvalidStore)?,
            &aad(&file, Some(index)),
        )?;
        tx.execute(
            "INSERT INTO chunks(file,part,state,data) VALUES(?1,?2,?3,?4)",
            (file.as_slice(), index, record, data),
        )?;
        tx.commit()?;
        Ok(hash)
    }
    /// Requires complete ordered coverage; never publishes a partial descriptor.
    pub fn finish_staging(&mut self, file: Id) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut state = load(&tx, &self.key, file)?;
        if state.phase == Phase::Ready {
            return Ok(());
        }
        if state.phase != Phase::Staging {
            return Err(Error::Conflict);
        }
        let (file_key, _) = state.key()?;
        state.descriptor = Some(file_key.descriptor(committed_root(&tx, &self.key, &state)?));
        state.phase = Phase::Ready;
        save(&tx, &self.key, &state, false)?;
        tx.commit()?;
        Ok(())
    }
    /// Only a confirmed publication can be handed to encrypted message creation.
    pub fn published_descriptor(&self, file: Id, now: u64) -> Result<Descriptor, Error> {
        let state = load(&self.db, &self.key, file)?;
        live(&state, now)?;
        if state.phase != Phase::Published {
            return Err(Error::Unprepared);
        }
        Ok(Descriptor {
            bytes: state.descriptor.ok_or(Error::InvalidStore)?,
            access: state.access.ok_or(Error::InvalidStore)?,
            metadata: state.metadata.ok_or(Error::InvalidStore)?,
            expires_at: state.expires_at,
        })
    }
    /// Stops local work and logically removes the local file key. Remote cleanup
    /// is still required if network submission has started.
    pub fn cancel(&mut self, file: Id) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut state = load(&tx, &self.key, file)?;
        let phase = match state.phase {
            Phase::Staging
            | Phase::Ready
            | Phase::Cancelled
            | Phase::Downloading
            | Phase::Complete => Phase::Cancelled,
            Phase::Expired => Phase::Expired,
            Phase::Evicting => Phase::Evicting,
            _ => Phase::Cancelling,
        };
        state.discard(phase);
        save(&tx, &self.key, &state, false)?;
        tx.commit()?;
        Ok(())
    }
    /// Evict a finished local cache entry without deleting the server's file.
    /// The caller must first retain any needed descriptor in authenticated history.
    /// Repeated cleanup steps release chunks and finally the cache row.
    pub fn evict(&mut self, file: Id) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut state = load(&tx, &self.key, file)?;
        if !matches!(
            state.phase,
            Phase::Published
                | Phase::Restored
                | Phase::Complete
                | Phase::Cancelled
                | Phase::Expired
                | Phase::Evicting
        ) {
            return Err(Error::Conflict);
        }
        state.discard(Phase::Evicting);
        save(&tx, &self.key, &state, false)?;
        tx.commit()?;
        Ok(())
    }
    /// Bounded ciphertext cleanup. This is logical deletion, not forensic erasure.
    pub fn cleanup(&mut self, file: Id) -> Result<usize, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = load(&tx, &self.key, file)?;
        if !matches!(
            state.phase,
            Phase::Cancelling | Phase::Cancelled | Phase::Expired | Phase::Evicting
        ) {
            return Err(Error::Conflict);
        }
        let count = tx.execute("DELETE FROM chunks WHERE file=?1 AND part IN (SELECT part FROM chunks WHERE file=?1 ORDER BY part LIMIT 16)",[file.as_slice()])?;
        if state.phase == Phase::Evicting && count < 16 {
            tx.execute("DELETE FROM files WHERE id=?1", [file.as_slice()])?;
        }
        tx.commit()?;
        Ok(count)
    }
}
