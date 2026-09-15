use super::*;
use sigil_protocol::attachments::{Begin, Publish, State as RemoteState, Status};

#[derive(Debug, PartialEq, Eq)]
pub enum UploadStep {
    Idle,
    Begun,
    Chunk(u32),
    Published,
    Restored,
    Removed,
}

fn apply_status(state: &mut State, status: &Status) -> Result<(), Error> {
    if state.expires_at != status.expires_at
        || state
            .upload_deadline
            .is_some_and(|v| v != status.upload_deadline)
    {
        return Err(Error::Conflict);
    }
    state.upload_deadline = Some(status.upload_deadline);
    state.phase = match status.state {
        RemoteState::Uploading => Phase::Uploading,
        RemoteState::Published => {
            let (_, root) = state.key()?;
            if status.root.as_deref() != Some(crate::transport::hex(&root).as_str()) {
                return Err(Error::Conflict);
            }
            if status.restored_checkpoint {
                Phase::Restored
            } else {
                Phase::Published
            }
        }
        RemoteState::Removed => Phase::Cancelled,
        RemoteState::Expired => Phase::Expired,
    };
    if matches!(state.phase, Phase::Cancelled | Phase::Expired) {
        state.descriptor = None;
        state.access = None;
        state.metadata = None;
    }
    Ok(())
}
impl ClientStore {
    /// One bounded network operation on a worker thread. Exact work survives
    /// transport errors and failed local receipt writes. Caller schedules backoff.
    pub fn upload_attachment_step(&self, cache: &mut Cache, file: Id) -> Result<UploadStep, Error> {
        if self.connected_account_scope()? != cache.scope {
            return Err(Error::Conflict);
        }
        let client = self.connected_client()?;
        // Persist that creation can be in flight before sending any request.
        // Cancellation then requires a server tombstone even after a timeout.
        {
            let tx = cache
                .db
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let mut state = load(&tx, &cache.key, file)?;
            if state.phase == Phase::Ready {
                state.phase = Phase::Starting;
                save(&tx, &cache.key, &state, false)?;
            }
            tx.commit()?;
        }
        let tx = cache
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut state = load(&tx, &cache.key, file)?;
        let step = match state.phase {
            Phase::Starting => {
                let status = client.begin_attachment(
                    file,
                    &Begin {
                        plaintext_bytes: state.length,
                        access_token: state
                            .access
                            .as_ref()
                            .ok_or(Error::InvalidStore)?
                            .to_string(),
                        expires_at: state.expires_at,
                    },
                )?;
                apply_status(&mut state, &status)?;
                save(&tx, &cache.key, &state, false)?;
                match state.phase {
                    Phase::Published => UploadStep::Published,
                    Phase::Restored => UploadStep::Restored,
                    Phase::Cancelled | Phase::Expired => UploadStep::Removed,
                    _ => UploadStep::Begun,
                }
            }
            Phase::Uploading => {
                // Load pending chunks, release the store, transfer concurrently, then record them.
                let total = state.shape().chunks()?;
                let mut pending = Vec::new();
                let mut uploaded = 0;
                {
                    let mut query =
                        tx.prepare("SELECT part,state FROM chunks WHERE file=?1 ORDER BY part")?;
                    let mut rows = query.query([file.as_slice()])?;
                    let mut expected = 0;
                    while let Some(row) = rows.next()? {
                        let index: u32 = row.get(0)?;
                        if index != expected || index >= total {
                            return Err(Error::InvalidStore);
                        }
                        let sealed = row.get_ref(1)?.as_blob().map_err(|_| Error::InvalidStore)?;
                        if sealed.len() > 256 {
                            return Err(Error::InvalidStore);
                        }
                        let record: Part =
                            serde_json::from_slice(&cache.key.open(sealed, &aad(&file, Some(index)))?)
                                .map_err(|_| Error::InvalidStore)?;
                        if record.uploaded {
                            uploaded += 1;
                        } else if pending.len() < TRANSFER_LANES {
                            pending.push(index);
                        }
                        expected += 1;
                    }
                }
                if pending.is_empty() {
                    if uploaded != total {
                        return Err(Error::InvalidStore);
                    }
                    let (_, root) = state.key()?;
                    let status = client.publish_attachment(
                        state.shape(),
                        &Publish {
                            root: crate::transport::hex(&root),
                            acknowledge_restored_checkpoint: false,
                        },
                    )?;
                    apply_status(&mut state, &status)?;
                    save(&tx, &cache.key, &state, false)?;
                    UploadStep::Published
                } else {
                    let mut records = Vec::new();
                    let mut chunks = Vec::new();
                    for index in pending {
                        let (record, data) = part(&tx, &cache.key, state.shape(), index)?
                            .ok_or(Error::InvalidStore)?;
                        records.push(record);
                        chunks.push((index, data));
                    }
                    let shape = state.shape();
                    tx.commit()?;
                    let results = client
                        .put_attachment_chunks(shape, &chunks)
                        .into_iter()
                        .map(|result| result.map_err(Error::from))
                        .collect();
                    return self.record_uploaded_chunks(cache, file, chunks, records, results);
                }
            }
            Phase::Cancelling => {
                client.remove_attachment(file)?;
                state.phase = Phase::Cancelled;
                save(&tx, &cache.key, &state, false)?;
                UploadStep::Removed
            }
            _ => UploadStep::Idle,
        };
        tx.commit()?;
        Ok(step)
    }
    /// Marks transferred chunks; any success continues immediately, failures alone back off.
    fn record_uploaded_chunks(
        &self,
        cache: &mut Cache,
        file: Id,
        chunks: Vec<(u32, Vec<u8>)>,
        records: Vec<Part>,
        results: Vec<Result<(), Error>>,
    ) -> Result<UploadStep, Error> {
        let tx = cache
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if load(&tx, &cache.key, file)?.phase != Phase::Uploading {
            tx.commit()?;
            return Ok(UploadStep::Idle);
        }
        let mut last = None;
        let mut failure = None;
        for (((index, _), mut record), result) in chunks.into_iter().zip(records).zip(results) {
            match result {
                Ok(()) => {
                    record.uploaded = true;
                    let sealed = cache.key.seal(
                        &serde_json::to_vec(&record).map_err(|_| Error::InvalidStore)?,
                        &aad(&file, Some(index)),
                    )?;
                    if tx.execute(
                        "UPDATE chunks SET state=?3 WHERE file=?1 AND part=?2",
                        (file.as_slice(), index, sealed),
                    )? != 1
                    {
                        return Err(Error::Conflict);
                    }
                    last = Some(index);
                }
                Err(error) => failure = failure.or(Some(error)),
            }
        }
        tx.commit()?;
        match (last, failure) {
            (Some(index), _) => Ok(UploadStep::Chunk(index)),
            (None, Some(error)) => Err(error),
            (None, None) => Ok(UploadStep::Idle),
        }
    }
    /// Reconcile expiry/removal after a failed transfer. A restored checkpoint
    /// remains blocked; this operation never approves restoring file access.
    pub fn reconcile_attachment_upload(&self, cache: &mut Cache, file: Id) -> Result<Phase, Error> {
        if self.connected_account_scope()? != cache.scope {
            return Err(Error::Conflict);
        }
        let client = self.connected_client()?;
        let tx = cache
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut state = load(&tx, &cache.key, file)?;
        if !matches!(
            state.phase,
            Phase::Starting
                | Phase::Uploading
                | Phase::Published
                | Phase::Checking
                | Phase::Restored
        ) {
            return Err(Error::Conflict);
        }
        let status = client.attachment_status(state.shape())?;
        apply_status(&mut state, &status)?;
        save(&tx, &cache.key, &state, false)?;
        tx.commit()?;
        Ok(state.phase)
    }
}
