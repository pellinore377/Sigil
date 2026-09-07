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
                let mut pending = None;
                // Only small authenticated progress records are scanned; load at
                // most one ciphertext chunk for the network operation.
                let mut query =
                    tx.prepare("SELECT part,state FROM chunks WHERE file=?1 ORDER BY part")?;
                let mut rows = query.query([file.as_slice()])?;
                let mut expected = 0;
                while let Some(row) = rows.next()? {
                    let index: u32 = row.get(0)?;
                    if index != expected || index >= state.shape().chunks()? {
                        return Err(Error::InvalidStore);
                    }
                    let sealed = row.get_ref(1)?.as_blob().map_err(|_| Error::InvalidStore)?;
                    if sealed.len() > 256 {
                        return Err(Error::InvalidStore);
                    }
                    let record: Part =
                        serde_json::from_slice(&cache.key.open(sealed, &aad(&file, Some(index)))?)
                            .map_err(|_| Error::InvalidStore)?;
                    if !record.uploaded {
                        pending = Some(index);
                        break;
                    }
                    expected += 1;
                }
                drop(rows);
                drop(query);
                if let Some(index) = pending {
                    let (mut record, data) =
                        part(&tx, &cache.key, state.shape(), index)?.ok_or(Error::InvalidStore)?;
                    client.put_attachment_chunk(state.shape(), index, &data)?;
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
                    UploadStep::Chunk(index)
                } else {
                    if expected != state.shape().chunks()? {
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
