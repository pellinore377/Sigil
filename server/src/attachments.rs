//! Immutable opaque files with reserved quota and separately scoped read access.
use crate::{
    prekeys::authorize,
    storage_budget,
    store::{read_configuration, Store, StoreError},
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use sha2::{Digest, Sha256};
use sigil_crypto::attachment::{CiphertextList, Shape, CHUNK_OVERHEAD};
use sigil_protocol::{
    accounts::valid_credential,
    attachments::{Begin, Part, Parts, Publish, State, Status, MAX_PARTS_PAGE},
};
use subtle::ConstantTimeEq;
type Id = [u8; 32];
pub(crate) const FILE_METADATA: u64 = 512;
const PART_METADATA: u64 = 256;
const UPLOAD_SECONDS: u64 = 86400;
pub(crate) const MIGRATION:&str="
CREATE TABLE attachments(id TEXT PRIMARY KEY,account_id TEXT NOT NULL REFERENCES accounts(id),plaintext_bytes INTEGER NOT NULL CHECK(plaintext_bytes>=0),access_hash BLOB NOT NULL,upload_deadline INTEGER NOT NULL CHECK(upload_deadline>0),expires_at INTEGER,state INTEGER NOT NULL CHECK(state BETWEEN 0 AND 3),root BLOB,restored_checkpoint INTEGER NOT NULL DEFAULT 0 CHECK(restored_checkpoint IN(0,1)),reserved_bytes INTEGER NOT NULL CHECK(reserved_bytes>=0),stored_bytes INTEGER NOT NULL CHECK(stored_bytes>=0),received_chunks INTEGER NOT NULL CHECK(received_chunks>=0));
CREATE TABLE attachment_chunks(file TEXT NOT NULL REFERENCES attachments(id),part INTEGER NOT NULL CHECK(part>=0),hash BLOB NOT NULL,data BLOB NOT NULL,PRIMARY KEY(file,part));
CREATE INDEX attachment_upload_expiry ON attachments(upload_deadline,id) WHERE state=0;
CREATE INDEX attachment_expiry ON attachments(expires_at,id) WHERE state IN(0,1) AND expires_at IS NOT NULL;
CREATE INDEX attachment_account ON attachments(account_id,state,id);
CREATE INDEX attachment_cleanup ON attachments(id) WHERE state IN(2,3) AND stored_bytes>0;
";
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn id(value: &str) -> Result<Id, StoreError> {
    if !valid_credential(value) {
        return Err(StoreError::Invalid("invalid attachment reference"));
    }
    let mut out = [0; 32];
    for (byte, pair) in out.iter_mut().zip(value.as_bytes().as_chunks::<2>().0) {
        *byte = u8::from_str_radix(
            std::str::from_utf8(pair).map_err(|_| StoreError::InvalidData)?,
            16,
        )
        .map_err(|_| StoreError::InvalidData)?;
    }
    Ok(out)
}
fn access_hash(file: &Id, token: &str) -> Result<Id, StoreError> {
    let token = id(token)?;
    Ok(Sha256::digest([b"Sigil/attachment-access/v0".as_slice(), file, &token].concat()).into())
}
fn clock(now: u64) -> Result<(), StoreError> {
    if now == 0 || now > i64::MAX as u64 {
        Err(StoreError::InvalidData)
    } else {
        Ok(())
    }
}
fn account(db: &Connection, credential: &str, now: u64) -> Result<String, StoreError> {
    clock(now)?;
    let device = authorize(db, credential, now)?;
    Ok(db.query_row(
        "SELECT account_id FROM devices WHERE id=?1",
        [device],
        |r| r.get(0),
    )?)
}
fn capacity(shape: Shape) -> Result<u64, StoreError> {
    Ok(shape
        .ciphertext_length()
        .map_err(|_| StoreError::Invalid("attachment exceeds format limit"))?
        + shape.chunks().map_err(|_| StoreError::InvalidData)? as u64 * PART_METADATA)
}
struct File {
    owner: String,
    shape: Shape,
    access: Id,
    deadline: u64,
    expires: Option<u64>,
    state: State,
    root: Option<Id>,
    restored: bool,
    reserved: u64,
    stored: u64,
    received: u32,
    disabled: bool,
}
impl File {
    fn effective(&self, now: u64) -> State {
        if matches!(self.state, State::Removed | State::Expired) {
            return self.state;
        }
        if self.disabled {
            return State::Removed;
        }
        if self.expires.is_some_and(|expiry| expiry <= now)
            || (self.state == State::Uploading && self.deadline <= now)
        {
            return State::Expired;
        }
        self.state
    }
    fn status(&self, now: u64) -> Result<Status, StoreError> {
        Ok(Status {
            state: self.effective(now),
            plaintext_bytes: self.shape.length,
            chunks: self.shape.chunks().map_err(|_| StoreError::InvalidData)?,
            received_chunks: self.received,
            upload_deadline: self.deadline,
            expires_at: self.expires,
            root: self.root.map(|v| hex(&v)),
            restored_checkpoint: self.restored,
        })
    }
}
fn read(db: &Connection, file: &str) -> Result<File, StoreError> {
    let file_id = id(file)?;
    type Row = (
        String,
        i64,
        Vec<u8>,
        i64,
        Option<i64>,
        u8,
        Option<Vec<u8>>,
        bool,
        i64,
        i64,
        u32,
        bool,
    );
    let row:Row=db.query_row("SELECT f.account_id,f.plaintext_bytes,CASE WHEN length(f.access_hash)=32 THEN f.access_hash END,f.upload_deadline,f.expires_at,f.state,CASE WHEN f.root IS NULL THEN NULL WHEN length(f.root)=32 THEN f.root ELSE x'' END,f.restored_checkpoint,f.reserved_bytes,f.stored_bytes,f.received_chunks,a.disabled FROM attachments f JOIN accounts a ON a.id=f.account_id WHERE f.id=?1",[file],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?,r.get(11)?))).optional()?.ok_or(StoreError::NotFound)?;
    let value = File {
        owner: row.0,
        shape: Shape {
            file: file_id,
            length: row.1.try_into().map_err(|_| StoreError::InvalidData)?,
        },
        access: row.2.try_into().map_err(|_| StoreError::InvalidData)?,
        deadline: row.3.try_into().map_err(|_| StoreError::InvalidData)?,
        expires: row
            .4
            .map(|v| v.try_into().map_err(|_| StoreError::InvalidData))
            .transpose()?,
        state: match row.5 {
            0 => State::Uploading,
            1 => State::Published,
            2 => State::Removed,
            3 => State::Expired,
            _ => return Err(StoreError::InvalidData),
        },
        root: row
            .6
            .map(|v| v.try_into().map_err(|_| StoreError::InvalidData))
            .transpose()?,
        restored: row.7,
        reserved: row.8.try_into().map_err(|_| StoreError::InvalidData)?,
        stored: row.9.try_into().map_err(|_| StoreError::InvalidData)?,
        received: row.10,
        disabled: row.11,
    };
    let capacity = capacity(value.shape)?;
    let chunks = value.shape.chunks().map_err(|_| StoreError::InvalidData)?;
    if value.deadline == 0
        || value.deadline > i64::MAX as u64
        || value.expires.is_some_and(|e| e == 0 || e > i64::MAX as u64)
        || value.received > chunks
        || value
            .reserved
            .checked_add(value.stored)
            .is_none_or(|v| v > capacity)
        || (value.state == State::Uploading
            && (value.root.is_some()
                || value.restored
                || value.reserved + value.stored != capacity))
        || (value.state == State::Published
            && (value.root.is_none()
                || value.reserved != 0
                || value.stored != capacity
                || value.received != chunks))
        || (matches!(value.state, State::Removed | State::Expired) && value.reserved != 0)
    {
        return Err(StoreError::InvalidData);
    }
    Ok(value)
}
fn owned(db: &Connection, credential: &str, file: &str, now: u64) -> Result<File, StoreError> {
    let owner = account(db, credential, now)?;
    let record = read(db, file)?;
    if record.owner != owner {
        return Err(StoreError::NotFound);
    }
    Ok(record)
}
impl Store {
    pub fn begin_attachment(
        &mut self,
        credential: &str,
        file: &str,
        request: Begin,
        now: u64,
    ) -> Result<Status, StoreError> {
        clock(now)?;
        let file_id = id(file)?;
        let access = access_hash(&file_id, &request.access_token)?;
        let shape = Shape {
            file: file_id,
            length: request.plaintext_bytes,
        };
        let charge = capacity(shape)?;
        if request
            .expires_at
            .is_some_and(|e| e == 0 || e > i64::MAX as u64)
        {
            return Err(StoreError::Invalid("invalid attachment expiry"));
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let owner = account(&tx, credential, now)?;
        match read(&tx, file) {
            Ok(prior) => {
                if prior.owner != owner {
                    return Err(StoreError::NotFound);
                }
                if prior.shape != shape
                    || prior.expires != request.expires_at
                    || !bool::from(prior.access.ct_eq(&access))
                {
                    return Err(StoreError::Conflict);
                }
                return prior.status(now);
            }
            Err(StoreError::NotFound) => {}
            Err(error) => return Err(error),
        }
        if now > i64::MAX as u64 - UPLOAD_SECONDS || request.expires_at.is_some_and(|e| e <= now) {
            return Err(StoreError::Invalid("invalid attachment expiry"));
        }
        let settings = read_configuration(&tx)?
            .settings
            .ok_or(StoreError::Unauthorized)?;
        if shape.length > settings.max_attachment_bytes {
            return Err(StoreError::Invalid("attachment exceeds configured limit"));
        }
        storage_budget::reserve(&tx, &owner, FILE_METADATA, now)?;
        let quota = crate::admin::quota(&tx, &owner)?;
        if crate::recovery::used(&tx, &owner, now)?
            .checked_add(charge)
            .is_none_or(|v| v > quota)
        {
            return Err(StoreError::Busy);
        }
        tx.execute(
            "INSERT INTO attachments VALUES(?1,?2,?3,?4,?5,?6,0,NULL,0,?7,0,0)",
            (
                file,
                &owner,
                shape.length as i64,
                access.as_slice(),
                (now + UPLOAD_SECONDS) as i64,
                request.expires_at.map(|v| v as i64),
                charge as i64,
            ),
        )?;
        let status = read(&tx, file)?.status(now)?;
        tx.commit()?;
        Ok(status)
    }
    pub fn attachment_status(
        &mut self,
        credential: &str,
        file: &str,
        now: u64,
    ) -> Result<Status, StoreError> {
        let tx = self.0.transaction()?;
        owned(&tx, credential, file, now)?.status(now)
    }
    pub fn put_attachment_chunk(
        &mut self,
        credential: &str,
        file: &str,
        index: u32,
        bytes: &[u8],
        now: u64,
    ) -> Result<(), StoreError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let record = owned(&tx, credential, file, now)?;
        if !matches!(record.effective(now), State::Uploading | State::Published) || record.restored
        {
            return Err(StoreError::Conflict);
        }
        let expected = record
            .shape
            .chunk_length(index)
            .map_err(|_| StoreError::Invalid("invalid attachment chunk index"))?
            + CHUNK_OVERHEAD;
        if bytes.len() != expected {
            return Err(StoreError::Invalid("invalid attachment chunk length"));
        }
        let hash: Id = Sha256::digest(bytes).into();
        let prior: Option<(Vec<u8>, u32)> = tx
            .query_row(
                "SELECT hash,length(data) FROM attachment_chunks WHERE file=?1 AND part=?2",
                (file, index),
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((prior, length)) = prior {
            return if prior == hash && length as usize == bytes.len() {
                Ok(())
            } else {
                Err(StoreError::Conflict)
            };
        }
        if record.state != State::Uploading {
            return Err(StoreError::InvalidData);
        }
        let charge = bytes.len() as u64 + PART_METADATA;
        if record.reserved < charge {
            return Err(StoreError::InvalidData);
        }
        tx.execute(
            "INSERT INTO attachment_chunks VALUES(?1,?2,?3,?4)",
            (file, index, hash.as_slice(), bytes),
        )?;
        tx.execute("UPDATE attachments SET reserved_bytes=reserved_bytes-?2,stored_bytes=stored_bytes+?2,received_chunks=received_chunks+1 WHERE id=?1",(file,charge as i64))?;
        read(&tx, file)?;
        tx.commit()?;
        Ok(())
    }
    pub fn attachment_parts(
        &mut self,
        credential: &str,
        file: &str,
        after: Option<u32>,
        now: u64,
    ) -> Result<Parts, StoreError> {
        let tx = self.0.transaction()?;
        let record = owned(&tx, credential, file, now)?;
        if !matches!(record.effective(now), State::Uploading | State::Published) {
            return Err(StoreError::Conflict);
        }
        let mut query=tx.prepare("SELECT part,CASE WHEN length(hash)=32 THEN hash END FROM attachment_chunks WHERE file=?1 AND part>?2 ORDER BY part LIMIT ?3")?;
        let rows = query.query_map(
            (file, after.map_or(-1, i64::from), MAX_PARTS_PAGE as u32),
            |r| Ok((r.get::<_, u32>(0)?, r.get::<_, Vec<u8>>(1)?)),
        )?;
        let mut chunks = Vec::new();
        for row in rows {
            let (index, hash) = row?;
            if index >= record.shape.chunks().map_err(|_| StoreError::InvalidData)? {
                return Err(StoreError::InvalidData);
            }
            chunks.push(Part {
                index,
                hash: hex(&hash),
            });
        }
        let next_after = if chunks.len() == MAX_PARTS_PAGE {
            chunks.last().map(|v| v.index)
        } else {
            None
        };
        Ok(Parts { chunks, next_after })
    }
    pub fn publish_attachment(
        &mut self,
        credential: &str,
        file: &str,
        request: Publish,
        now: u64,
    ) -> Result<Status, StoreError> {
        let root = id(&request.root)?;
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let record = owned(&tx, credential, file, now)?;
        if !matches!(record.effective(now), State::Uploading | State::Published) {
            return Err(StoreError::Conflict);
        }
        if record.state == State::Published {
            if record.root != Some(root)
                || (record.restored && !request.acknowledge_restored_checkpoint)
            {
                return Err(StoreError::Conflict);
            }
        } else if request.acknowledge_restored_checkpoint {
            return Err(StoreError::Conflict);
        }
        if record.received != record.shape.chunks().map_err(|_| StoreError::InvalidData)?
            || record.reserved != 0
        {
            return Err(StoreError::Conflict);
        }
        let mut list = CiphertextList::new(record.shape).map_err(|_| StoreError::InvalidData)?;
        let mut total = 0u64;
        {
            let mut statement=tx.prepare("SELECT part,CASE WHEN length(hash)=32 THEN hash END,length(data) FROM attachment_chunks WHERE file=?1 ORDER BY part")?;
            let mut rows = statement.query([file])?;
            while let Some(row) = rows.next()? {
                let index: u32 = row.get(0)?;
                let hash: Vec<u8> = row.get(1)?;
                let length = row.get::<_, u32>(2)? as u64;
                if length
                    != record
                        .shape
                        .chunk_length(index)
                        .map_err(|_| StoreError::InvalidData)? as u64
                        + CHUNK_OVERHEAD as u64
                {
                    return Err(StoreError::InvalidData);
                }
                total = total
                    .checked_add(length + PART_METADATA)
                    .ok_or(StoreError::InvalidData)?;
                list.push_hash(index, hash.try_into().map_err(|_| StoreError::InvalidData)?)
                    .map_err(|_| StoreError::InvalidData)?;
            }
        }
        if total != record.stored || list.finish().map_err(|_| StoreError::InvalidData)? != root {
            return Err(StoreError::Conflict);
        }
        tx.execute(
            "UPDATE attachments SET state=1,root=?2,restored_checkpoint=0 WHERE id=?1",
            (file, root.as_slice()),
        )?;
        let status = read(&tx, file)?.status(now)?;
        tx.commit()?;
        Ok(status)
    }
    pub fn attachment_chunk(
        &mut self,
        credential: &str,
        file: &str,
        index: u32,
        access: &str,
        now: u64,
    ) -> Result<Vec<u8>, StoreError> {
        let tx = self.0.transaction()?;
        account(&tx, credential, now)?;
        chunk_in(&tx, file, index, access, now)
    }
    pub fn remove_attachment(
        &mut self,
        credential: &str,
        file: &str,
        now: u64,
    ) -> Result<(), StoreError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        id(file)?;
        let owner = account(&tx, credential, now)?;
        match read(&tx, file) {
            Ok(prior) if prior.owner == owner => {}
            Ok(_) => return Err(StoreError::NotFound),
            Err(StoreError::NotFound) => {
                // Cancellation may beat a delayed creation request. Retain a
                // charged tombstone even when no upload has arrived yet.
                storage_budget::reserve(&tx, &owner, FILE_METADATA, now)?;
                tx.execute(
                    "INSERT INTO attachments VALUES(?1,?2,0,?3,?4,NULL,2,NULL,0,0,0,0)",
                    (file, &owner, [0u8; 32].as_slice(), now as i64),
                )?;
                tx.commit()?;
                return Ok(());
            }
            Err(error) => return Err(error),
        }
        tx.execute(
            "UPDATE attachments SET state=2,reserved_bytes=0,restored_checkpoint=0 WHERE id=?1",
            [file],
        )?;
        tx.commit()?;
        Ok(())
    }
}

pub(crate) fn cleanup(tx: &Transaction<'_>, now: u64) -> Result<usize, StoreError> {
    clock(now)?;
    let ids=tx.prepare("SELECT id FROM (SELECT id FROM attachments WHERE state=0 AND upload_deadline<=?1 UNION SELECT id FROM attachments WHERE state IN(0,1) AND expires_at IS NOT NULL AND expires_at<=?1 UNION SELECT f.id FROM attachments f JOIN accounts a ON a.id=f.account_id WHERE f.state IN(0,1) AND a.disabled=1) ORDER BY id LIMIT 64")?.query_map([now as i64],|r|r.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?;
    let mut changed = 0;
    for file in ids {
        let record = read(tx, &file)?;
        let state = match record.effective(now) {
            State::Removed => 2,
            State::Expired => 3,
            _ => return Err(StoreError::InvalidData),
        };
        changed += tx.execute(
            "UPDATE attachments SET state=?2,reserved_bytes=0,restored_checkpoint=0 WHERE id=?1",
            (&file, state),
        )?;
    }
    let file: Option<String> = tx
        .query_row(
            "SELECT id FROM attachments WHERE state IN(2,3) AND stored_bytes>0 ORDER BY id LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(file) = file {
        let record = read(tx, &file)?;
        let parts=tx.prepare("SELECT part,length(data) FROM attachment_chunks WHERE file=?1 ORDER BY part LIMIT 64")?.query_map([&file],|r|Ok((r.get::<_,u32>(0)?,r.get::<_,u32>(1)?)))?.collect::<Result<Vec<_>,_>>()?;
        if parts.is_empty() {
            return Err(StoreError::InvalidData);
        }
        let mut charge = 0u64;
        for (part, length) in &parts {
            if *length as u64
                != record
                    .shape
                    .chunk_length(*part)
                    .map_err(|_| StoreError::InvalidData)? as u64
                    + CHUNK_OVERHEAD as u64
            {
                return Err(StoreError::InvalidData);
            }
            charge = charge
                .checked_add(*length as u64 + PART_METADATA)
                .ok_or(StoreError::InvalidData)?;
        }
        if record.stored < charge || record.received < parts.len() as u32 {
            return Err(StoreError::InvalidData);
        }
        for (part, _) in &parts {
            changed += tx.execute(
                "DELETE FROM attachment_chunks WHERE file=?1 AND part=?2",
                (&file, part),
            )?;
        }
        tx.execute("UPDATE attachments SET stored_bytes=stored_bytes-?2,received_chunks=received_chunks-?3 WHERE id=?1",(&file,charge as i64,parts.len() as u32))?;
        read(tx, &file)?;
    }
    Ok(changed)
}

#[path = "attachment_http.rs"]
mod http;
pub(crate) use http::routes;

pub(crate) fn chunk_in(
    tx: &Transaction<'_>,
    file: &str,
    index: u32,
    access: &str,
    now: u64,
) -> Result<Vec<u8>, StoreError> {
    let file_id = id(file)?;
    let hash = access_hash(&file_id, access)?;
    let record = read(tx, file)?;
    if record.effective(now) != State::Published
        || record.restored
        || !bool::from(record.access.ct_eq(&hash))
    {
        return Err(StoreError::NotFound);
    }
    let expected = record
        .shape
        .chunk_length(index)
        .map_err(|_| StoreError::NotFound)?
        + CHUNK_OVERHEAD;
    let (bytes,hash):(Vec<u8>,Vec<u8>)=tx.query_row("SELECT CASE WHEN length(data)=?3 THEN data END,CASE WHEN length(hash)=32 THEN hash END FROM attachment_chunks WHERE file=?1 AND part=?2",(file,index,expected as u32),|r|Ok((r.get(0)?,r.get(1)?))).optional()?.ok_or(StoreError::InvalidData)?;
    if Sha256::digest(&bytes).as_slice() != hash {
        return Err(StoreError::InvalidData);
    }
    Ok(bytes)
}
