//! Reclaim delivery journals without deleting logical retry/chain evidence.
use super::*;

pub struct JournalCleanup {
    pub scanned: usize,
    pub reclaimed: usize,
}
impl ClientStore {
    /// Inspect at most 16 journals with a trusted clock. Delete only acknowledged,
    /// terminal controls past their signed deadline. A sealed cyclic cursor and
    /// deletions commit together; retained request proofs still prevent reopening.
    pub fn reclaim_retry_journals(&mut self, now: u64) -> Result<JournalCleanup, Error> {
        self.reclaim_journals(now, false)
    }
    /// Reclaim at most 16 expired, authenticated server acknowledgements of
    /// recovered originals. Retained replacement messages/proofs are untouched.
    pub fn reclaim_recovered_journals(&mut self, now: u64) -> Result<JournalCleanup, Error> {
        self.reclaim_journals(now, true)
    }
    fn reclaim_journals(&mut self, now: u64, recovered: bool) -> Result<JournalCleanup, Error> {
        if now == 0 || now > i64::MAX as u64 {
            return Err(Error::Expired);
        }
        let own = self.own_device_binding()?;
        let fingerprint = device_fingerprint(&own)?;
        let (table, cursor_id, label): (&str, i64, &[u8]) = if recovered {
            ("recovered_deliveries", 2, b"recovered journal scan")
        } else {
            ("retry_incoming", 1, b"retry journal scan")
        };
        let aad = binding(24, &fingerprint, label);
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let sealed: Option<Vec<u8>> = tx
            .query_row(
                "SELECT CASE WHEN length(state)=44 THEN state END FROM retry_gc_cursor WHERE id=?1",
                [cursor_id],
                |r| r.get(0),
            )
            .optional()?;
        let after = match sealed {
            Some(bytes) => i64::from_be_bytes(
                self.key
                    .open(&bytes, &aad)?
                    .as_slice()
                    .try_into()
                    .map_err(|_| Error::InvalidStore)?,
            ),
            None => 0,
        };
        if after < 0 {
            return Err(Error::InvalidStore);
        }
        let sequences: Vec<i64> = tx
            .prepare(&format!(
                "SELECT sequence FROM {table} WHERE sequence>?1 ORDER BY sequence LIMIT 16"
            ))?
            .query_map([after], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let mut progress = JournalCleanup {
            scanned: sequences.len(),
            reclaimed: 0,
        };
        for sequence in &sequences {
            if recovered {
                if resolution::reclaimable(&tx, &self.key, &fingerprint, *sequence, now)? {
                    tx.execute(
                        "DELETE FROM recovered_deliveries WHERE sequence=?1",
                        [sequence],
                    )?;
                    progress.reclaimed += 1;
                }
                continue;
            }
            let (id, acknowledged) =
                journal_read(&tx, &self.key, &own, *sequence)?.ok_or(Error::InvalidStore)?;
            let (finished, expires) = match read(&tx, &self.key, "retry_requests", &id)? {
                Some(record) => (
                    record.finished,
                    Request::from_bytes(&record.packet)
                        .map_err(|_| Error::InvalidStore)?
                        .expires_at,
                ),
                None => (
                    2,
                    accepted_retired(&tx, &self.key, &id, &fingerprint)?
                        .ok_or(Error::InvalidStore)?
                        .expires,
                ),
            };
            if !acknowledged || finished == 0 || expires > now {
                continue;
            }
            if finished == 1 {
                let receipt = transport::receipt(&tx, &self.key, work::session(&id), id)?
                    .ok_or(Error::InvalidStore)?;
                if receipt.expires_at != expires {
                    return Err(Error::InvalidStore);
                }
            }
            tx.execute("DELETE FROM retry_incoming WHERE sequence=?1", [sequence])?;
            tx.execute("DELETE FROM control_journals WHERE sequence=?1", [sequence])?;
            progress.reclaimed += 1;
        }
        let next = sequences.last().copied().unwrap_or(0);
        tx.execute("INSERT INTO retry_gc_cursor VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET state=excluded.state", (cursor_id, self.key.seal(&next.to_be_bytes(), &aad)?))?;
        tx.commit()?;
        Ok(progress)
    }
}

pub struct ControlCleanup {
    /// At most 16 full controls inspected per call, with a durable cyclic cursor.
    pub scanned: usize,
    /// At most 16 full records replaced with expiry tombstones per call.
    pub retired: usize,
}
fn scan_controls(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    kind: u8,
) -> Result<Vec<Id>, Error> {
    let aad = binding(39, own, &[kind]);
    let sealed: Option<Vec<u8>> = tx.query_row("SELECT CASE WHEN length(state) IN (36,68) THEN state END FROM control_cleanup_cursor WHERE kind=?1",[kind],|r|r.get(0)).optional()?;
    let after = match sealed {
        Some(value) => key.open(&value, &aad)?.to_vec(),
        None => Vec::new(),
    };
    if !after.is_empty() && after.len() != 32 {
        return Err(Error::InvalidStore);
    }
    let table = if kind == 0 {
        "retry_outbox"
    } else {
        "retry_requests"
    };
    let raw: Vec<Vec<u8>> = tx
        .prepare(&format!(
            "SELECT id FROM {table} WHERE id>?1 ORDER BY id LIMIT 16"
        ))?
        .query_map([after], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let ids: Vec<Id> = raw
        .into_iter()
        .map(|v| v.try_into().map_err(|_| Error::InvalidStore))
        .collect::<Result<_, _>>()?;
    let next = ids.last().map_or(&[][..], |id| id.as_slice());
    tx.execute("INSERT INTO control_cleanup_cursor VALUES(?1,?2) ON CONFLICT(kind) DO UPDATE SET state=excluded.state",(kind,key.seal(next,&aad)?))?;
    Ok(ids)
}
pub(super) fn retired(
    db: &Connection,
    key: &StorageKey,
    id: &Id,
    own: &Id,
) -> Result<Option<u64>, Error> {
    let sealed: Option<Vec<u8>> = db.query_row("SELECT CASE WHEN length(state)=76 THEN state END FROM retired_retry_outbox WHERE id=?1", [id.as_slice()], |r| r.get(0)).optional()?;
    let Some(sealed) = sealed else {
        return Ok(None);
    };
    let bytes = key.open(&sealed, &binding(25, id, b"outgoing retry retired"))?;
    if bytes.len() != 40 || bytes[..32] != *own {
        return Err(Error::InvalidStore);
    }
    let expiry = u64::from_be_bytes(bytes[32..].try_into().map_err(|_| Error::InvalidStore)?);
    if expiry == 0 || expiry > i64::MAX as u64 {
        return Err(Error::InvalidStore);
    }
    Ok(Some(expiry))
}
impl ClientStore {
    /// Retire expired outgoing controls only when no retained response or retry
    /// depends on them. Tombstones prevent deadline renewal under the same ID.
    /// This is local compaction, not physical erasure or unlimited retention.
    pub fn reclaim_outgoing_retry_controls(&mut self, now: u64) -> Result<ControlCleanup, Error> {
        if now == 0 || now > i64::MAX as u64 {
            return Err(Error::Expired);
        }
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ids = scan_controls(&tx, &self.key, &own, 0)?;
        let mut controls = Vec::with_capacity(ids.len());
        for id in ids {
            let record = read(&tx, &self.key, "retry_outbox", &id)?.ok_or(Error::InvalidStore)?;
            let request = Request::from_bytes(&record.packet).map_err(|_| Error::InvalidStore)?;
            if request.requester != own || retired(&tx, &self.key, &id, &own)?.is_some() {
                return Err(Error::InvalidStore);
            }
            controls.push((id, request.expires_at));
        }
        let mut progress = ControlCleanup {
            scanned: controls.len(),
            retired: 0,
        };
        for (id, expires) in controls {
            if expires > now
                || tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM control_dependencies WHERE kind=0 AND parent=?1)",
                    [id.as_slice()],
                    |r| r.get::<_, bool>(0),
                )?
            {
                continue;
            }
            // Retained response content keeps its authorization proof even after
            // mailbox acknowledgement, including cached acknowledgement replay.
            if tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM inbox WHERE id=?1)",
                [id.as_slice()],
                |r| r.get::<_, bool>(0),
            )? {
                continue;
            }
            let mut bytes = own.to_vec();
            bytes.extend_from_slice(&expires.to_be_bytes());
            let sealed = self
                .key
                .seal(&bytes, &binding(25, &id, b"outgoing retry retired"))?;
            tx.execute(
                "INSERT INTO retired_retry_outbox VALUES(?1,?2)",
                (id.as_slice(), sealed),
            )?;
            tx.execute("DELETE FROM retry_outbox WHERE id=?1", [id.as_slice()])?;
            tx.execute(
                "DELETE FROM control_dependencies WHERE kind=0 AND id=?1",
                [id.as_slice()],
            )?;
            progress.retired += 1;
        }
        tx.commit()?;
        Ok(progress)
    }
}

pub(super) struct AcceptedTombstone {
    pub peer: Id,
    pub digest: Id,
    pub expires: u64,
}
pub(super) fn accepted_retired(
    db: &Connection,
    key: &StorageKey,
    id: &Id,
    own: &Id,
) -> Result<Option<AcceptedTombstone>, Error> {
    let sealed: Option<Vec<u8>> = db.query_row("SELECT CASE WHEN length(state)=140 THEN state END FROM retired_retry_requests WHERE id=?1", [id.as_slice()], |r| r.get(0)).optional()?;
    let Some(sealed) = sealed else {
        return Ok(None);
    };
    let bytes = key.open(&sealed, &binding(26, id, b"accepted retry retired"))?;
    if bytes.len() != 104 || bytes[..32] != *own {
        return Err(Error::InvalidStore);
    }
    let expires = u64::from_be_bytes(bytes[96..].try_into().map_err(|_| Error::InvalidStore)?);
    if expires == 0 || expires > i64::MAX as u64 {
        return Err(Error::InvalidStore);
    }
    Ok(Some(AcceptedTombstone {
        peer: bytes[32..64].try_into().map_err(|_| Error::InvalidStore)?,
        digest: bytes[64..96].try_into().map_err(|_| Error::InvalidStore)?,
        expires,
    }))
}
impl ClientStore {
    /// Compact expired cancelled/discarded controls with no response, journal or
    /// child request. Full response proofs remain pinned for history/late receipts.
    pub fn reclaim_accepted_retry_controls(&mut self, now: u64) -> Result<ControlCleanup, Error> {
        if now == 0 || now > i64::MAX as u64 {
            return Err(Error::Expired);
        }
        let own = self.own_device_binding()?;
        let fingerprint = device_fingerprint(&own)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ids = scan_controls(&tx, &self.key, &fingerprint, 1)?;
        let mut records = Vec::with_capacity(ids.len());
        for id in ids {
            let record = read(&tx, &self.key, "retry_requests", &id)?.ok_or(Error::InvalidStore)?;
            let request = Request::from_bytes(&record.packet).map_err(|_| Error::InvalidStore)?;
            if request.target != fingerprint
                || record.receipt.is_some()
                || accepted_retired(&tx, &self.key, &id, &fingerprint)?.is_some()
            {
                return Err(Error::InvalidStore);
            }
            records.push((id, record, request.expires_at));
        }
        let mut progress = ControlCleanup {
            scanned: records.len(),
            retired: 0,
        };
        for (id, record, expires) in records {
            if record.finished != 2 || expires > now || tx.query_row("SELECT EXISTS(SELECT 1 FROM control_dependencies WHERE kind=1 AND parent=?1) OR EXISTS(SELECT 1 FROM control_journals WHERE control=?1)",[id.as_slice()],|r|r.get::<_,bool>(0))? {
                continue;
            }
            let session = work::session(&id);
            if tx.query_row("SELECT EXISTS(SELECT 1 FROM outbox WHERE session=?1 AND id=?2) OR EXISTS(SELECT 1 FROM deliveries WHERE id=?2)", (session.as_slice(), id.as_slice()), |r| r.get::<_, bool>(0))? { continue }
            let mut bytes = fingerprint.to_vec();
            bytes.extend_from_slice(&record.peer);
            bytes.extend_from_slice(&Sha256::digest(&record.packet));
            bytes.extend_from_slice(&expires.to_be_bytes());
            let sealed = self
                .key
                .seal(&bytes, &binding(26, &id, b"accepted retry retired"))?;
            tx.execute(
                "INSERT INTO retired_retry_requests VALUES(?1,?2)",
                (id.as_slice(), sealed),
            )?;
            tx.execute("DELETE FROM retry_requests WHERE id=?1", [id.as_slice()])?;
            tx.execute(
                "DELETE FROM control_dependencies WHERE kind=1 AND id=?1",
                [id.as_slice()],
            )?;
            progress.retired += 1;
        }
        tx.commit()?;
        Ok(progress)
    }
}
