//! A dedicated platform worker calls this scheduler; no hidden timer or UI work.
use super::*;
use crate::schedule;

#[derive(Debug, PartialEq, Eq)]
pub enum TransferProgress {
    Upload(UploadStep),
    Download(DownloadStep),
    Cleanup(usize),
    Reconciled(Phase),
}
pub struct TransferAttempt {
    pub file: Id,
    pub result: Result<TransferProgress, Error>,
}
pub struct ScheduledTransfers {
    /// None when deferred or after a bounded scan found no eligible work.
    pub attempt: Option<TransferAttempt>,
    pub next_at: u64,
    pub scheduling_error: Option<Error>,
}
fn select(store: &ClientStore, cache: &mut Cache, now: u64) -> Result<Option<(Id, Phase)>, Error> {
    if now == 0 || now > i64::MAX as u64 {
        return Err(Error::Expired);
    }
    let server = events::source(store, cache)?;
    let tx = cache
        .db
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let archive = rusqlite::Transaction::new_unchecked(&store.db, TransactionBehavior::Immediate)?;
    let sealed:Option<Vec<u8>>=tx.query_row("SELECT CASE WHEN length(state) IN(36,68) THEN state END FROM transfer_cursor WHERE id=1",[],|r|r.get(0)).optional()?;
    let cursor = match sealed {
        None => Vec::new(),
        Some(bytes) => cache
            .key
            .open(&bytes, b"Sigil/attachment-work-cursor/v0")?
            .to_vec(),
    };
    if !cursor.is_empty() && cursor.len() != 32 {
        return Err(Error::InvalidStore);
    }
    let mut ids = tx
        .prepare("SELECT id FROM files WHERE id>?1 ORDER BY id LIMIT 16")?
        .query_map([&cursor], |r| r.get::<_, Vec<u8>>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if ids.is_empty() && !cursor.is_empty() {
        ids = tx
            .prepare("SELECT id FROM files ORDER BY id LIMIT 16")?
            .query_map([], |r| r.get::<_, Vec<u8>>(0))?
            .collect::<Result<Vec<_>, _>>()?;
    }
    let mut next = Vec::new();
    let mut selected = None;
    for bytes in ids {
        let file: Id = bytes
            .as_slice()
            .try_into()
            .map_err(|_| Error::InvalidStore)?;
        let mut state = load(&tx, &cache.key, file)?;
        if !matches!(
            state.phase,
            Phase::Cancelling | Phase::Cancelled | Phase::Expired | Phase::Evicting
        ) && (state.expires_at.is_some_and(|v| v <= now)
            || (state.phase == Phase::Uploading && state.upload_deadline.is_some_and(|v| v <= now)))
        {
            state.discard(Phase::Expired);
            save(&tx, &cache.key, &state, false)?;
        }
        if state.managed
            && matches!(
                state.phase,
                Phase::Published | Phase::Restored | Phase::Downloading | Phase::Complete
            )
            && !crate::recovery::references_file(
                &archive,
                &store.key,
                cache.scope,
                &events::content(&state, &server)?,
            )?
        {
            // This is local cache eviction, never deletion of a server file
            // another device or recipient may still retain.
            state.discard(Phase::Evicting);
            save(&tx, &cache.key, &state, false)?;
        }
        next = bytes;
        let eligible = match state.phase {
            Phase::Ready
            | Phase::Starting
            | Phase::Uploading
            | Phase::Checking
            | Phase::Downloading
            | Phase::Cancelling
            | Phase::Evicting => true,
            Phase::Cancelled | Phase::Expired => tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM chunks WHERE file=?1)",
                [file.as_slice()],
                |r| r.get(0),
            )?,
            _ => false,
        };
        if eligible {
            selected = Some((file, state.phase));
            break;
        }
    }
    tx.execute("INSERT INTO transfer_cursor VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET state=excluded.state",
        [cache.key.seal(&next,b"Sigil/attachment-work-cursor/v0")?])?;
    tx.commit()?;
    Ok(selected)
}
impl ClientStore {
    /// Offline retention/cleanup pass using the same bounded fair file cursor.
    /// Examine at most 16 files and release at most 16 chunks. A zero result
    /// can mean the selected file needs network work; no network runs here.
    pub fn maintain_attachment_cache(&self, cache: &mut Cache, now: u64) -> Result<usize, Error> {
        match select(self, cache, now)? {
            Some((file, Phase::Cancelled | Phase::Expired | Phase::Evicting)) => {
                cache.cleanup(file)
            }
            _ => Ok(0),
        }
    }
    /// One selected file per pass, with durable reservation/backoff and a bounded
    /// scan cursor. Run on a dedicated transfer worker so media cannot block chat
    /// polling. Platform wakeups honor next_at and any scheduling_error.
    pub fn sync_attachments_due_online(
        &self,
        cache: &mut Cache,
    ) -> Result<ScheduledTransfers, Error> {
        self.attachments_with_clock(cache, schedule::clock)
    }
    fn attachments_with_clock(
        &self,
        cache: &mut Cache,
        mut clock: impl FnMut() -> Result<u64, Error>,
    ) -> Result<ScheduledTransfers, Error> {
        if self.connected_account_scope()? != cache.scope {
            return Err(Error::Conflict);
        }
        let now = clock()?;
        if now == 0 || now > i64::MAX as u64 - schedule::RESERVATION_SECONDS {
            return Err(Error::Expired);
        }
        let tx = cache
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (mut state, _) = schedule::read(&tx, &cache.key, &cache.scope)?;
        if now < state.last {
            return Err(Error::Expired);
        }
        if now < state.next {
            return Ok(ScheduledTransfers {
                attempt: None,
                next_at: state.next,
                scheduling_error: None,
            });
        }
        state.last = now;
        state.next = now + schedule::RESERVATION_SECONDS;
        let reserved = schedule::write(&tx, &cache.key, &cache.scope, &state)?;
        tx.commit()?;
        let attempt = select(self, cache, now)?.map(|(file, phase)| TransferAttempt {
            file,
            result: match phase {
                Phase::Downloading => self
                    .download_attachment_step(cache, file, now)
                    .map(TransferProgress::Download),
                Phase::Checking => self
                    .reconcile_attachment_upload(cache, file)
                    .map(TransferProgress::Reconciled),
                Phase::Cancelled | Phase::Expired | Phase::Evicting => {
                    cache.cleanup(file).map(TransferProgress::Cleanup)
                }
                _ => self
                    .upload_attachment_step(cache, file)
                    .map(TransferProgress::Upload),
            },
        });
        if let Some(TransferAttempt {
            file,
            result: Err(Error::Network(crate::network::Error::Status { code: 409, .. })),
        }) = &attempt
        {
            let tx = cache
                .db
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let mut current = load(&tx, &cache.key, *file)?;
            if matches!(current.phase, Phase::Starting | Phase::Uploading) {
                current.phase = Phase::Checking;
                save(&tx, &cache.key, &current, false)?;
            }
            tx.commit()?;
        }
        let failed = attempt.as_ref().is_some_and(|v| v.result.is_err());
        let network = match attempt.as_ref().and_then(|v| v.result.as_ref().err()) {
            Some(Error::Network(error)) => Some(error),
            _ => None,
        };
        let completion = clock().and_then(|finished| {
            schedule::complete(
                &mut cache.db,
                &cache.key,
                &cache.scope,
                (&state, &reserved),
                finished,
                (failed, network),
                if attempt.is_some() { 1 } else { 5 },
            )
        });
        let (next_at, scheduling_error) = match completion {
            Ok(next) => (next, None),
            Err(error) => (state.next, Some(error)),
        };
        Ok(ScheduledTransfers {
            attempt,
            next_at,
            scheduling_error,
        })
    }
}

#[cfg(test)]
#[path = "attachment_schedule_tests.rs"]
mod tests;
