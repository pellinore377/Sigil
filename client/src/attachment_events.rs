//! Handoffs between durable, authenticated events and private file transfer state.
use super::*;
use sigil_protocol::event::Content;
#[path = "conversation_files.rs"]
pub(super) mod conversation_files;

pub(super) fn source(store: &ClientStore, cache: &Cache) -> Result<String, Error> {
    if store.connected_account_scope()? != cache.scope {
        return Err(Error::Conflict);
    }
    let connection = store.connection_session()?.ok_or(Error::Unprepared)?;
    Ok(connection
        .address
        .split_once(':')
        .ok_or(Error::InvalidStore)?
        .1
        .to_owned())
}
pub(super) fn descriptor(bytes: &[u8], server: &str) -> Result<Descriptor, Error> {
    let file = sigil_protocol::file::File::from_bytes(bytes).map_err(|_| Error::InvalidEvent)?;
    Ok(Descriptor {
        source: (file.source != server).then(|| file.source.to_owned()),
        bytes: Zeroizing::new(file.descriptor.to_vec()),
        access: Zeroizing::new(crate::transport::hex(file.access)),
        metadata: Metadata {
            name: file.name.to_owned(),
            media_type: file.media_type.to_owned(),
        },
        expires_at: file.expires_at,
    })
}
pub(super) fn content(state: &State, server: &str) -> Result<Zeroizing<Vec<u8>>, Error> {
    let metadata = state.metadata.as_ref().ok_or(Error::InvalidStore)?;
    let access = Zeroizing::new(crate::connection::decode_id(
        state.access.as_ref().ok_or(Error::InvalidStore)?,
    )?);
    Ok(Zeroizing::new(
        sigil_protocol::file::File {
            caption: "",
            source: state.source.as_deref().unwrap_or(server),
            name: &metadata.name,
            media_type: &metadata.media_type,
            expires_at: state.expires_at,
            access: &access,
            descriptor: state.descriptor.as_ref().ok_or(Error::InvalidStore)?,
        }
        .to_bytes()
        .map_err(|_| Error::InvalidEvent)?,
    ))
}
fn handoff(
    cache: &mut Cache,
    file: Id,
    server: &str,
    now: u64,
    queue: impl FnOnce(Content<'_>) -> Result<(), Error>,
) -> Result<(), Error> {
    let tx = cache
        .db
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let state = load(&tx, &cache.key, file)?;
    live(&state, now)?;
    if state.phase != Phase::Published {
        return Err(Error::Unprepared);
    }
    let bytes = content(&state, server)?;
    let result = queue(Content::File(&bytes));
    // No cache writes participated: only the messaging transaction commits.
    drop(tx);
    result
}
fn prepare(
    store: &ClientStore,
    cache: &mut Cache,
    record: Id,
    expected: Option<&[u8]>,
    now: u64,
) -> Result<Id, Error> {
    let server = source(store, cache)?;
    // Every two-store operation takes the cache lock before the main store.
    // The main write reservation serializes archive edits with this read/handoff;
    // only the cache is changed, so there is no cross-database commit claim.
    let tx = cache
        .db
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let archive = rusqlite::Transaction::new_unchecked(&store.db, TransactionBehavior::Immediate)?;
    let retained = if expected.is_none() {
        Some(crate::recovery::recovery_file(
            &archive,
            &store.key,
            cache.scope,
            record,
        )?)
    } else {
        crate::recovery::retained_file(&archive, &store.key, cache.scope, record)?
    };
    let bytes = match expected {
        Some(expected) => {
            if retained
                .as_ref()
                .is_some_and(|bytes| bytes.as_slice() != expected)
            {
                return Err(Error::Obsolete);
            }
            expected
        }
        None => retained.as_ref().ok_or(Error::NotFound)?.as_slice(),
    };
    let file = download::prepare(
        &tx,
        &cache.key,
        cache.budget,
        descriptor(bytes, &server)?,
        now,
        retained.is_some(),
    )?;
    tx.commit()?;
    drop(archive);
    Ok(file)
}
impl ClientStore {
    pub(crate) fn with_published_file(
        &mut self,
        cache: &mut Cache,
        file: Id,
        now: u64,
        queue: impl FnOnce(&mut Self, &[u8]) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let server = source(self, cache)?;
        handoff(cache, file, &server, now, |body| queue(self, body.bytes()))
    }
    /// Freeze a published file into the ordinary verified-peer send queue. The
    /// cache lock prevents local cancellation during this descriptor handoff;
    /// the messaging transaction owns the durable event before that lock releases.
    pub fn queue_peer_file(
        &mut self,
        peer: Id,
        message: Id,
        (cache, file): (&mut Cache, Id),
        timestamp: u64,
        now: u64,
    ) -> Result<(), Error> {
        let server = source(self, cache)?;
        handoff(cache, file, &server, now, |content| {
            self.queue_peer_content(peer, message, content, timestamp, now)
        })
    }
    /// Same publication handoff, with Sender Keys and current group authorization.
    pub fn queue_group_file(
        &mut self,
        group: Id,
        message: Id,
        (cache, file): (&mut Cache, Id),
        timestamp: u64,
        now: u64,
    ) -> Result<(), Error> {
        let server = source(self, cache)?;
        handoff(cache, file, &server, now, |content| {
            self.queue_group_content(group, message, content, timestamp, now)
        })
    }
    /// Read authenticated retained group history; this does not grant access to
    /// any earlier epoch or revive a retired group sender/receiver chain.
    pub fn prepare_received_group_file(
        &mut self,
        cache: &mut Cache,
        group: Id,
        author: Id,
        message: Id,
        now: u64,
    ) -> Result<Id, Error> {
        let retained = self.group_message(group, author, message)?;
        let event = retained.event()?;
        let Content::File(bytes) = event.content else {
            return Err(Error::InvalidEvent);
        };
        prepare(
            self,
            cache,
            crate::groups::group_event_history_id(&event),
            Some(bytes),
            now,
        )
    }
    pub fn prepare_shared_group_file(
        &mut self,
        cache: &mut Cache,
        group: Id,
        record: Id,
        now: u64,
    ) -> Result<Id, Error> {
        let retained = self.shared_group_history_record(group, record)?;
        let event = sigil_protocol::event::Group::from_bytes(&retained.plaintext)
            .map_err(|_| Error::InvalidStore)?;
        let Content::File(bytes) = event.content else {
            return Err(Error::InvalidEvent);
        };
        prepare(
            self,
            cache,
            crate::groups::group_event_history_id(&event),
            Some(bytes),
            now,
        )
    }
    /// Resolve a durable inbox journal entry, not a caller-constructed Incoming.
    /// No new peer verification is inferred from the file descriptor.
    pub fn prepare_received_file(
        &mut self,
        cache: &mut Cache,
        sequence: i64,
        now: u64,
    ) -> Result<Id, Error> {
        let incoming = self.retained_incoming_event(sequence)?;
        crate::conversations::require_payload(
            &self.db,
            &self.key,
            &incoming.plaintext,
            now,
            false,
        )?;
        let event = incoming.event()?;
        let Content::File(bytes) = event.content else {
            return Err(Error::InvalidEvent);
        };
        let author = self.peer(incoming.peer)?.binding.identity;
        prepare(
            self,
            cache,
            crate::event_history_id(&event, &author),
            Some(bytes),
            now,
        )
    }
    /// Media recovery restores a retained file key, never a live session or peer
    /// verification. Existing archive import/checkpoint authority still applies.
    pub fn prepare_recovered_file(
        &self,
        cache: &mut Cache,
        record: Id,
        now: u64,
    ) -> Result<Id, Error> {
        prepare(self, cache, record, None, now)
    }
    /// Reauthenticate a chunk against this current committed history record.
    /// A tombstone blocks this read immediately, before background cache cleanup.
    /// Returned plaintext already held by a caller cannot later be recalled.
    pub fn recovered_file_chunk(
        &self,
        cache: &Cache,
        record: Id,
        index: u32,
        now: u64,
    ) -> Result<Zeroizing<Vec<u8>>, Error> {
        self.recovered_file_chunk_for(cache, record, (index, None), now)
    }
    pub(super) fn recovered_file_chunk_for(
        &self,
        cache: &Cache,
        record: Id,
        (index, expected): (u32, Option<&[u8]>),
        now: u64,
    ) -> Result<Zeroizing<Vec<u8>>, Error> {
        let server = source(self, cache)?;
        let tx = rusqlite::Transaction::new_unchecked(&cache.db, TransactionBehavior::Immediate)?;
        let archive =
            rusqlite::Transaction::new_unchecked(&self.db, TransactionBehavior::Immediate)?;
        let bytes = crate::recovery::recovery_file(&archive, &self.key, cache.scope, record)?;
        if expected.is_some_and(|expected| expected != bytes.as_slice()) {
            return Err(Error::Obsolete);
        }
        let file =
            sigil_protocol::file::File::from_bytes(&bytes).map_err(|_| Error::InvalidStore)?;
        let id = sigil_protocol::file::KeyDescriptor::from_bytes(file.descriptor)
            .map_err(|_| Error::InvalidStore)?
            .file;
        let state = load(&tx, &cache.key, id)?;
        live(&state, now)?;
        let cached = content(&state, &server)?;
        let cached =
            sigil_protocol::file::File::from_bytes(&cached).map_err(|_| Error::InvalidStore)?;
        if !cached.same_attachment(&file) {
            return Err(Error::Conflict);
        }
        download::completed(&tx, &cache.key, id, index, now)
    }
}

#[cfg(test)]
#[path = "attachment_event_tests.rs"]
mod tests;
