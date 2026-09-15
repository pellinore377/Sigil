use super::*;

#[derive(Debug, PartialEq, Eq)]
pub enum DownloadStep {
    Idle,
    Chunk(u32),
    Complete,
}

fn accept(
    db: &Connection,
    key: &StorageKey,
    state: &State,
    index: u32,
    ciphertext: &[u8],
) -> Result<(), Error> {
    if state.direction != Direction::Download
        || !matches!(state.phase, Phase::Downloading | Phase::Complete)
    {
        return Err(Error::Conflict);
    }
    let (file_key, _) = state.key()?;
    // AEAD succeeds before any durable progress is accepted. No plaintext escapes.
    file_key.open_chunk(index, ciphertext)?;
    if let Some((_, prior)) = part(db, key, state.shape(), index)? {
        if prior != ciphertext {
            return Err(Error::Conflict);
        }
        return Ok(());
    }
    if state.phase != Phase::Downloading {
        return Err(Error::InvalidStore);
    }
    let record = Part {
        hash: Sha256::digest(ciphertext).into(),
        uploaded: false,
    };
    let sealed = key.seal(
        &serde_json::to_vec(&record).map_err(|_| Error::InvalidStore)?,
        &aad(&state.file, Some(index)),
    )?;
    db.execute(
        "INSERT INTO chunks(file,part,state,data) VALUES(?1,?2,?3,?4)",
        (state.file.as_slice(), index, sealed, ciphertext),
    )?;
    Ok(())
}
fn finish(db: &Connection, key: &StorageKey, state: &mut State) -> Result<(), Error> {
    if state.direction != Direction::Download
        || !matches!(state.phase, Phase::Downloading | Phase::Complete)
    {
        return Err(Error::Conflict);
    }
    let (_, root) = state.key()?;
    if committed_root(db, key, state)? != root {
        return Err(sigil_crypto::Error::Authentication.into());
    }
    state.phase = Phase::Complete;
    save(db, key, state, false)
}
pub(super) fn prepare(
    db: &Connection,
    key: &StorageKey,
    budget: u64,
    descriptor: Descriptor,
    now: u64,
    managed: bool,
) -> Result<Id, Error> {
    let (file_key, _) = FileKey::from_descriptor(&descriptor.bytes)?;
    descriptor
        .metadata
        .validate()
        .map_err(|_| Error::InvalidEvent)?;
    if !sigil_protocol::accounts::valid_credential(&descriptor.access) {
        return Err(Error::InvalidEvent);
    }
    if file_key.shape().ciphertext_length()? > budget {
        return Err(Error::Limit);
    }
    let state = State {
        source: descriptor.source,
        file: file_key.shape().file,
        length: file_key.shape().length,
        phase: Phase::Downloading,
        direction: Direction::Download,
        descriptor: Some(descriptor.bytes),
        access: Some(descriptor.access),
        metadata: Some(descriptor.metadata),
        expires_at: descriptor.expires_at,
        upload_deadline: None,
        managed,
        recovery_parent: None,
    };
    state.validate()?;
    live(&state, now)?;
    match load(db, key, state.file) {
        Ok(mut prior) => {
            live(&prior, now)?;
            if prior.source != state.source
                || prior.descriptor != state.descriptor
                || prior.access != state.access
                || prior.metadata != state.metadata
                || prior.expires_at != state.expires_at
                || (prior.direction == Direction::Upload
                    && !matches!(prior.phase, Phase::Published | Phase::Local))
            {
                return Err(Error::Conflict);
            }
            if managed && !prior.managed {
                prior.managed = true;
                save(db, key, &prior, false)?;
            }
        }
        Err(Error::NotFound) => save(db, key, &state, true)?,
        Err(error) => return Err(error),
    }
    Ok(state.file)
}
pub(super) fn completed(
    db: &Connection,
    key: &StorageKey,
    file: Id,
    index: u32,
    now: u64,
) -> Result<Zeroizing<Vec<u8>>, Error> {
    let state = load(db, key, file)?;
    live(&state, now)?;
    if !matches!(
        state.phase,
        Phase::Published | Phase::Complete | Phase::Restored | Phase::Local
    ) {
        return Err(Error::Unprepared);
    }
    let (file_key, _) = state.key()?;
    let (_, data) = part(db, key, state.shape(), index)?.ok_or(Error::InvalidStore)?;
    Ok(file_key.open_chunk(index, &data)?)
}
impl Cache {
    pub(crate) fn staged_chunk(
        &self,
        file: Id,
        index: u32,
        now: u64,
    ) -> Result<Zeroizing<Vec<u8>>, Error> {
        let tx = self.db.unchecked_transaction()?;
        let state = load(&tx, &self.key, file)?;
        live(&state, now)?;
        if state.direction != Direction::Upload
            || !matches!(
                state.phase,
                Phase::Ready
                    | Phase::Starting
                    | Phase::Uploading
                    | Phase::Checking
                    | Phase::Published
            )
        {
            return Err(Error::Unprepared);
        }
        let (file_key, _) = state.key()?;
        let (_, data) = part(&tx, &self.key, state.shape(), index)?.ok_or(Error::InvalidStore)?;
        let plaintext = file_key.open_chunk(index, &data)?;
        tx.commit()?;
        Ok(plaintext)
    }
    /// Caller must obtain this descriptor from authenticated encrypted content
    /// and authorize the conversation/source first. Parsing alone proves neither.
    /// This low-level handoff does not grant archive-managed retention.
    pub fn prepare_authenticated_download(
        &mut self,
        descriptor: Descriptor,
        now: u64,
    ) -> Result<Id, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let file = prepare(&tx, &self.key, self.budget, descriptor, now, false)?;
        tx.commit()?;
        Ok(file)
    }
    pub fn accept_download_chunk(
        &mut self,
        file: Id,
        index: u32,
        ciphertext: &[u8],
        now: u64,
    ) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = load(&tx, &self.key, file)?;
        live(&state, now)?;
        accept(&tx, &self.key, &state, index, ciphertext)?;
        tx.commit()?;
        Ok(())
    }
    pub fn finish_download(&mut self, file: Id, now: u64) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut state = load(&tx, &self.key, file)?;
        live(&state, now)?;
        finish(&tx, &self.key, &mut state)?;
        tx.commit()?;
        Ok(())
    }
    /// Reauthenticate one chunk of a completely verified file. Incomplete cache
    /// data is never handed to viewers, preview processes, or export adapters.
    /// This checks cache integrity/expiry only. Archive-aware viewers use
    /// ClientStore::recovered_file_chunk to check current deletion authority.
    pub fn completed_chunk(
        &self,
        file: Id,
        index: u32,
        now: u64,
    ) -> Result<Zeroizing<Vec<u8>>, Error> {
        let tx = self.db.unchecked_transaction()?;
        let plaintext = completed(&tx, &self.key, file, index, now)?;
        tx.commit()?;
        Ok(plaintext)
    }
}
impl ClientStore {
    /// One chunk request or local completion per worker step; retries preserve
    /// accepted ciphertext and never publish partial plaintext.
    pub fn download_attachment_step(
        &self,
        cache: &mut Cache,
        file: Id,
        now: u64,
    ) -> Result<DownloadStep, Error> {
        if self.connected_account_scope()? != cache.scope {
            return Err(Error::Conflict);
        }
        let client = self.connected_client()?;
        let tx = cache
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut state = load(&tx, &cache.key, file)?;
        live(&state, now)?;
        if matches!(
            state.phase,
            Phase::Complete | Phase::Published | Phase::Local
        ) {
            return Ok(DownloadStep::Complete);
        }
        if state.phase != Phase::Downloading {
            return Ok(DownloadStep::Idle);
        }
        let total = state.shape().chunks()?;
        let mut missing = Vec::new();
        {
            let mut query = tx.prepare("SELECT part FROM chunks WHERE file=?1 ORDER BY part")?;
            let mut rows = query.query([file.as_slice()])?;
            let mut next = 0;
            while let Some(row) = rows.next()? {
                let found: u32 = row.get(0)?;
                if found < next || found >= total {
                    return Err(Error::InvalidStore);
                }
                missing.extend(next..found);
                next = found + 1;
            }
            missing.extend(next..total);
            missing.truncate(TRANSFER_LANES);
        }
        if missing.is_empty() {
            finish(&tx, &cache.key, &mut state)?;
            tx.commit()?;
            return Ok(DownloadStep::Complete);
        }
        // Fetch the next missing chunks concurrently after releasing the store.
        let shape = state.shape();
        let source = state.source.clone();
        let access = state.access.clone().ok_or(Error::InvalidStore)?;
        tx.commit()?;
        let results = client.attachment_chunks_at(source.as_deref(), shape, &missing, &access);
        let tx = cache
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = load(&tx, &cache.key, file)?;
        live(&state, now)?;
        if state.phase != Phase::Downloading {
            tx.commit()?;
            return Ok(DownloadStep::Idle);
        }
        let mut last = None;
        let mut failure = None;
        for (index, result) in missing.into_iter().zip(results) {
            match result {
                Ok(bytes) => {
                    accept(&tx, &cache.key, &state, index, &bytes)?;
                    last = Some(index);
                }
                Err(error) => failure = failure.or(Some(error)),
            }
        }
        let step = match (last, failure) {
            (Some(index), _) => DownloadStep::Chunk(index),
            (None, Some(error)) => return Err(error.into()),
            (None, None) => DownloadStep::Idle,
        };
        tx.commit()?;
        Ok(step)
    }
}
