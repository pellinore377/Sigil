use super::*;
use crate::conversations::{Action, Body, Destination, FilePost, Reference};

pub struct ViewOnceFile {
    file: Id,
    content: Zeroizing<Vec<u8>>,
    expires: Option<u64>,
    time: u64,
}
fn file_id(bytes: &[u8]) -> Result<Id, Error> {
    let file = sigil_protocol::file::File::from_bytes(bytes).map_err(|_| Error::InvalidEvent)?;
    Ok(
        sigil_protocol::file::KeyDescriptor::from_bytes(file.descriptor)
            .map_err(|_| Error::InvalidEvent)?
            .file,
    )
}
fn verify(db: &Connection, key: &StorageKey, bytes: &[u8], now: u64) -> Result<Id, Error> {
    let file = sigil_protocol::file::File::from_bytes(bytes).map_err(|_| Error::InvalidEvent)?;
    let id = file_id(bytes)?;
    let state = load(db, key, id)?;
    live(&state, now)?;
    if !matches!(
        state.phase,
        Phase::Complete | Phase::Published | Phase::Restored
    ) {
        return Err(Error::Unprepared);
    }
    if content(&state, file.source)?.as_slice() != bytes {
        return Err(Error::Conflict);
    }
    Ok(id)
}
impl ViewOnceFile {
    #[cfg(target_os = "linux")]
    pub fn preview(
        &mut self,
        cache: &Cache,
        job: &sigil_media::sandbox::Job<'_>,
        mut clock: impl FnMut() -> u64,
    ) -> Result<sigil_media::Preview, Error> {
        let (format, count) = super::super::preview::request(&self.content)?;
        let input = super::super::preview::collect(count, job.cancel, |index| {
            self.chunk(cache, index, clock())
        })?;
        let result = job.render(&input, format)?;
        self.chunk(cache, 0, clock())?;
        Ok(result)
    }
    pub fn chunk(
        &mut self,
        cache: &Cache,
        index: u32,
        now: u64,
    ) -> Result<Zeroizing<Vec<u8>>, Error> {
        self.time = self.time.max(now);
        if self.expires.is_some_and(|v| self.time >= v) {
            return Err(Error::Obsolete);
        }
        let tx = cache.db.unchecked_transaction()?;
        if verify(&tx, &cache.key, &self.content, self.time)? != self.file {
            return Err(Error::Conflict);
        }
        download::completed(&tx, &cache.key, self.file, index, self.time)
    }
}
impl ClientStore {
    pub fn queue_conversation_file(
        &mut self,
        destination: Destination,
        cache: &mut Cache,
        file: Id,
        post: FilePost,
        now: u64,
    ) -> Result<(), Error> {
        let server = source(self, cache)?;
        handoff(cache, file, &server, now, |body| {
            let operation = self.conversation_operation(
                post.id,
                Action::Post {
                    body: Body::File(body.bytes().to_vec()),
                    reply: post.reply,
                    thread: post.thread,
                    expires_at: post.expires_at,
                    view_once: post.view_once,
                },
            )?;
            match destination {
                Destination::Peer(peer) => {
                    self.queue_peer_operation(peer, &operation, post.timestamp, now)
                }
                Destination::Group(group) => {
                    self.queue_group_operation(group, &operation, post.timestamp, now)
                }
            }
        })
    }
    pub fn prepare_conversation_file(
        &mut self,
        cache: &mut Cache,
        conversation: Id,
        reference: Reference,
        now: u64,
    ) -> Result<Id, Error> {
        let server = source(self, cache)?;
        let tx = cache
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = crate::conversations::time_floor(&self.db, &self.key, now)?;
        let main = rusqlite::Transaction::new_unchecked(&self.db, TransactionBehavior::Immediate)?;
        let view =
            crate::conversations::file_message(&main, &self.key, conversation, reference, now)?;
        let Some(Body::File(bytes)) = view.body else {
            return Err(Error::InvalidEvent);
        };
        let id = download::prepare(
            &tx,
            &cache.key,
            cache.budget,
            descriptor(&bytes, &server)?,
            now,
            false,
        )?;
        tx.commit()?;
        main.commit()?;
        Ok(id)
    }
    pub fn conversation_file_chunk(
        &mut self,
        cache: &Cache,
        conversation: Id,
        reference: Reference,
        index: u32,
        now: u64,
    ) -> Result<Zeroizing<Vec<u8>>, Error> {
        self.conversation_file_chunk_for(cache, conversation, reference, (index, None), now)
    }
    pub(in crate::attachments) fn conversation_file_chunk_for(
        &mut self,
        cache: &Cache,
        conversation: Id,
        reference: Reference,
        (index, expected): (u32, Option<&[u8]>),
        now: u64,
    ) -> Result<Zeroizing<Vec<u8>>, Error> {
        source(self, cache)?;
        let tx = rusqlite::Transaction::new_unchecked(&cache.db, TransactionBehavior::Immediate)?;
        let now = crate::conversations::time_floor(&self.db, &self.key, now)?;
        let main = rusqlite::Transaction::new_unchecked(&self.db, TransactionBehavior::Immediate)?;
        let view =
            crate::conversations::file_message(&main, &self.key, conversation, reference, now)?;
        if view.view_once {
            return Err(Error::Unprepared);
        }
        let Some(Body::File(bytes)) = view.body else {
            return Err(Error::InvalidEvent);
        };
        if expected.is_some_and(|expected| expected != bytes.as_slice()) {
            return Err(Error::Obsolete);
        }
        let id = verify(&tx, &cache.key, &bytes, now)?;
        let bytes = download::completed(&tx, &cache.key, id, index, now)?;
        main.commit()?;
        Ok(bytes)
    }
    pub fn open_view_once_file(
        &mut self,
        cache: &Cache,
        conversation: Id,
        reference: Reference,
        id: Id,
        now: u64,
    ) -> Result<ViewOnceFile, Error> {
        source(self, cache)?;
        let tx = rusqlite::Transaction::new_unchecked(&cache.db, TransactionBehavior::Immediate)?;
        let mut expires = None;
        let body = self.consume_view_once_checked(conversation, reference, id, now, |view| {
            let Some(Body::File(bytes)) = &view.body else {
                return Err(Error::InvalidEvent);
            };
            verify(&tx, &cache.key, bytes, now)?;
            expires = view.expires_at;
            Ok(())
        })?;
        let Body::File(bytes) = body else {
            return Err(Error::InvalidEvent);
        };
        Ok(ViewOnceFile {
            file: file_id(&bytes)?,
            content: Zeroizing::new(bytes),
            expires,
            time: now,
        })
    }
}
