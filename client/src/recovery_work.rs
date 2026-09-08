//! Bounded retained-media expiry and automatic archive publication. Platform
//! lifecycle code owns the worker; import/restore authority is never inferred.
use super::*;
use crate::schedule;

struct Work {
    dirty: bool,
    last: u64,
    cursor: Vec<u8>,
}
fn read(db: &Connection, key: &StorageKey, scope: &Id) -> Result<Work, Error> {
    let sealed: Option<Vec<u8>> = db
        .query_row(
            "SELECT CASE WHEN length(state) IN(45,77) THEN state END FROM archive_work WHERE id=1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let Some(sealed) = sealed else {
        // Existing schemas may contain unpublished edits alongside an older
        // prepared upload. One successor snapshot safely reconciles that case.
        return Ok(Work {
            dirty: true,
            last: 0,
            cursor: Vec::new(),
        });
    };
    let bytes = key.open(&sealed, &binding(54, scope, b"archive work"))?;
    if !matches!(bytes.len(), 9 | 41) || bytes[0] > 1 {
        return Err(Error::InvalidStore);
    }
    let last = u64::from_be_bytes(bytes[1..9].try_into().map_err(|_| Error::InvalidStore)?);
    if last > i64::MAX as u64 {
        return Err(Error::InvalidStore);
    }
    Ok(Work {
        dirty: bytes[0] == 1,
        last,
        cursor: bytes[9..].to_vec(),
    })
}
fn write(db: &Connection, key: &StorageKey, scope: &Id, work: &Work) -> Result<(), Error> {
    let mut bytes = Zeroizing::new(vec![work.dirty as u8]);
    bytes.extend_from_slice(&work.last.to_be_bytes());
    bytes.extend_from_slice(&work.cursor);
    db.execute(
        "INSERT INTO archive_work VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET state=excluded.state",
        [key.seal(&bytes, &binding(54, scope, b"archive work"))?],
    )?;
    Ok(())
}
pub(super) fn dirty(db: &Connection, key: &StorageKey, scope: &Id) -> Result<(), Error> {
    reconciled(db, key, scope, true)
}
pub(super) fn snapshot(db: &Connection, key: &StorageKey, scope: &Id) -> Result<(), Error> {
    reconciled(db, key, scope, false)
}
pub(super) fn reconciled(
    db: &Connection,
    key: &StorageKey,
    scope: &Id,
    dirty: bool,
) -> Result<(), Error> {
    let mut work = read(db, key, scope)?;
    work.dirty = dirty;
    write(db, key, scope, &work)
}
pub(super) fn expire(
    db: &Connection,
    key: &StorageKey,
    record: &mut Record,
    now: u64,
) -> Result<bool, Error> {
    let local;
    let content = if matches!(record.content, Content::Omitted) {
        local = local_record(db, key, record.id)?;
        &local.content
    } else {
        &record.content
    };
    if let Some(bytes) = super::media::file_bytes(content)? {
        let file =
            sigil_protocol::file::File::from_bytes(&bytes).map_err(|_| Error::InvalidStore)?;
        if file.expires_at.is_some_and(|expiry| expiry <= now) {
            record.revision = record.revision.checked_add(1).ok_or(Error::Limit)?;
            record.content = deletion_content(db, key, record.id)?;
            return Ok(true);
        }
    }
    Ok(false)
}
#[derive(Default, Debug, PartialEq, Eq)]
pub struct RecoveryMaintenance {
    pub checked: usize,
    pub expired: usize,
    pub backfilled: usize,
    /// An explicitly prepared import must finish/cancel before local edits.
    pub deferred: bool,
}
#[derive(Debug, PartialEq, Eq)]
pub enum RecoveryProgress {
    Idle,
    Cleanup(usize),
    Upload(Option<Head>),
    Import(Option<Head>),
}
pub struct ScheduledRecovery {
    /// None when the durable deadline has not arrived.
    pub maintenance: Option<RecoveryMaintenance>,
    pub progress: Option<Result<RecoveryProgress, Error>>,
    pub next_at: u64,
    /// The reservation survives failed completion writes; callers must handle
    /// this error because final Retry-After may not have been persisted.
    pub scheduling_error: Option<Error>,
}
impl ClientStore {
    /// Delete a committed recovery entry with optimistic revision checking.
    /// This updates the next archive, not remote recipients or local message
    /// journals. Repeating the same successful deletion is idempotent.
    pub fn delete_recovery_record(
        &mut self,
        id: Id,
        expected_revision: u64,
    ) -> Result<Reference, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = load(&tx, &self.key)?;
        let (reference, bytes) = existing(&tx, id)?.ok_or(Error::NotFound)?;
        let mut record = state.key.open_record(&reference, &bytes)?;
        if matches!(record.content, Content::Deleted)
            && (record.revision == expected_revision
                || expected_revision.checked_add(1) == Some(record.revision))
        {
            return Ok(reference);
        }
        if record.revision != expected_revision {
            return Err(Error::Conflict);
        }
        record.revision = record.revision.checked_add(1).ok_or(Error::Limit)?;
        record.content = Content::Deleted;
        let reference = retain(&tx, &self.key, &state, &record)?;
        tx.commit()?;
        Ok(reference)
    }
    /// Inspect at most 16 committed records and replace expired file keys with
    /// authenticated permanent tombstones. Fair scanning and changes commit
    /// together. Callable offline; no server response establishes deletion.
    pub fn maintain_recovery(&mut self, now: u64) -> Result<RecoveryMaintenance, Error> {
        if now == 0 || now > i64::MAX as u64 {
            return Err(Error::Expired);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !configured(&tx)? {
            return Ok(RecoveryMaintenance::default());
        }
        let state = load(&tx, &self.key)?;
        let mut work = read(&tx, &self.key, &state.scope)?;
        if now < work.last {
            return Err(Error::Expired);
        }
        if matches!(state.status.pending, Some((Operation::Import, _))) {
            return Ok(RecoveryMaintenance {
                deferred: true,
                ..Default::default()
            });
        }
        let mut lifecycle = super::lifecycle::read(&tx, &self.key, &state.scope)?;
        let policy = crate::conversations::recovery_policy(&tx, &self.key)?;
        let backfilled = {
            let (count, after) =
                crate::conversations::backfill_recovery(&tx, &self.key, lifecycle.backfill)?;
            lifecycle.backfill = after;
            super::lifecycle::save(&tx, &self.key, &state.scope, &lifecycle)?;
            work.dirty |= read(&tx, &self.key, &state.scope)?.dirty;
            count
        };
        let mut records = record_page(&tx, &work.cursor)?;
        if records.is_empty() && !work.cursor.is_empty() {
            records = record_page(&tx, &[])?;
        }
        let mut result = RecoveryMaintenance {
            backfilled,
            ..Default::default()
        };
        work.cursor.clear();
        for (reference, _) in records {
            let (reference, bytes) = existing(&tx, reference.id)?.ok_or(Error::InvalidStore)?;
            let mut record = state.key.open_record(&reference, &bytes)?;
            result.checked += 1;
            if apply_tombstone(&tx, &self.key, &mut record, now)?
                || expire(&tx, &self.key, &mut record, now)?
                || expire_history(&mut record, policy, now)?
            {
                retain(&tx, &self.key, &state, &record)?;
                result.expired += 1;
                work.dirty = true;
            }
            work.cursor = record.id.to_vec();
        }
        work.last = now;
        write(&tx, &self.key, &state.scope, &work)?;
        tx.commit()?;
        Ok(result)
    }
    fn recovery_work_step(&mut self, now: u64) -> Result<RecoveryProgress, Error> {
        let state = load(&self.db, &self.key)?;
        if self.recovery_competition()?.is_some()
            || matches!(state.status.pending, Some((Operation::Import, _)))
        {
            return self
                .download_recovery_step(false)
                .map(RecoveryProgress::Import);
        }
        if state.status.pending.is_none() {
            if !read(&self.db, &self.key, &state.scope)?.dirty {
                let removed = self.cleanup_recovery_online()?;
                return Ok(if removed == 0 {
                    RecoveryProgress::Idle
                } else {
                    RecoveryProgress::Cleanup(removed)
                });
            }
            self.prepare_recovery_upload(now)?;
        }
        self.upload_recovery_step().map(RecoveryProgress::Upload)
    }
    /// Dedicated archive worker with independent durable backoff. Calling this
    /// enables automatic publication for the already configured recovery key.
    /// It never accepts an unanchored head or authorizes a restored checkpoint.
    pub fn sync_recovery_due_online(&mut self) -> Result<ScheduledRecovery, Error> {
        self.recovery_with_clock(schedule::clock)
    }
    fn recovery_with_clock(
        &mut self,
        mut clock: impl FnMut() -> Result<u64, Error>,
    ) -> Result<ScheduledRecovery, Error> {
        let scope = self.recovery_scope()?;
        if self.connected_account_scope()? != scope {
            return Err(Error::Conflict);
        }
        let now = clock()?;
        if now == 0 || now > i64::MAX as u64 - schedule::RESERVATION_SECONDS {
            return Err(Error::Expired);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (mut before, _) = schedule::read_recovery(&tx, &self.key, &scope)?;
        if now < before.last {
            return Err(Error::Expired);
        }
        if now < before.next {
            return Ok(ScheduledRecovery {
                maintenance: None,
                progress: None,
                next_at: before.next,
                scheduling_error: None,
            });
        }
        before.last = now;
        before.next = now + schedule::RESERVATION_SECONDS;
        let reserved = schedule::write(&tx, &self.key, &scope, &before)?;
        tx.commit()?;
        let (maintenance, progress) = match self.maintain_recovery(now) {
            Ok(maintenance) => (Some(maintenance), self.recovery_work_step(now)),
            Err(error) => (None, Err(error)),
        };
        let network = match &progress {
            Err(Error::Network(e)) => Some(e),
            _ => None,
        };
        let delay = if matches!(progress, Ok(RecoveryProgress::Idle)) {
            30
        } else {
            1
        };
        let completed = clock().and_then(|finished| {
            schedule::complete(
                &mut self.db,
                &self.key,
                &scope,
                (&before, &reserved),
                finished,
                (progress.is_err(), network),
                delay,
            )
        });
        let (next_at, scheduling_error) = match completed {
            Ok(next) => (next, None),
            Err(error) => (before.next, Some(error)),
        };
        Ok(ScheduledRecovery {
            maintenance,
            progress: Some(progress),
            next_at,
            scheduling_error,
        })
    }
}

#[cfg(test)]
#[path = "recovery_work_tests.rs"]
mod tests;

pub(super) fn expire_history(
    record: &mut Record,
    policy: RecoveryPolicy,
    now: u64,
) -> Result<bool, Error> {
    if matches!(
        record.content,
        Content::Deleted
            | Content::Omitted
            | Content::Redacted { .. }
            | Content::HistoryLink { .. }
    ) {
        return Ok(false);
    }
    let content = match &record.content {
        Content::Conversation(raw) => matches!(
            sigil_protocol::conversation::Snapshot::from_bytes(raw)
                .map_err(|_| Error::InvalidStore)?
                .operation
                .action,
            sigil_protocol::conversation::Action::Post { .. }
                | sigil_protocol::conversation::Action::Edit { .. }
        ),
        _ => true,
    };
    if content
        && policy
            .history_days
            .is_some_and(|days| record.created_at.saturating_add(days as u64 * 86400) <= now)
    {
        record.revision = record.revision.checked_add(1).ok_or(Error::Limit)?;
        record.content = Content::Omitted;
        return Ok(true);
    }
    Ok(false)
}
