#![forbid(unsafe_code)]
//! Experimental native persistence. Only the recovery module exports history;
//! this live database is never a backup/rollback recovery format.
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use sha2::{Digest, Sha256};
use sigil_crypto::{
    storage::StorageKey,
    triple::{Packet, Session, MAX_SEALED_CHECKPOINT_LEN},
    MAX_PLAINTEXT,
};
use std::path::Path;
pub mod attachments;
pub mod calls;
mod erasure;
#[cfg(test)]
mod load_tests;
mod private_db;
pub use erasure::JournalErasure;
use zeroize::Zeroizing;
mod claims;
mod connection;
pub use connection::SendProgress;
pub mod conversations;
mod event;
pub mod groups;
pub mod mobile;
mod notes;
mod rich_text;
pub mod services;
pub use notes::{NoteCard, NoteEntry, NoteKind, NotesPage};
pub use rich_text::SigilTextDraft;
mod structured;
pub use event::SendIntentAttempt;
pub use event::{event_history_id, text_history_id};
pub use structured::alarms::{AlarmBatch, AlarmJob, AlarmNotification};
pub use structured::{
    CardDefinition, CardState, CheckState, LocationBatch, LocationJob, LocationKind, LocationState,
    PollState, RecurringState, TaskCompletion, TaskPage, TaskState,
};
mod federation;
mod handshake;
mod incoming;
pub use incoming::{Incoming, IncomingAttempt, MailboxEvent};
pub mod link;
pub mod network;
mod peers;
mod prekeys;
pub mod push;
pub use prekeys::{PrekeyAttempt, PrekeySupply};
mod retirement;
pub use retirement::{SessionMaintenance, INACTIVE_GRACE_SECONDS};
mod retry;
pub use retry::{ControlCleanup, JournalCleanup, RetryAttempt, RetryRequest};
pub use retry::{RecoveryAction, RecoveryAdvice, RecoveryBlock};
mod selection;
pub use peers::{device_fingerprint, DeviceReview, DeviceReviewCursor, DeviceReviewPage, Peer};
pub mod recovery;
#[cfg(test)]
#[path = "../tests/common/mod.rs"]
mod test_schema;
mod transport;
mod worker;
pub use worker::{BackendWork, SyncFailure, SyncStep};
mod schedule;
pub use schedule::ScheduledSync;
mod lifetime;
mod outbound;
pub use outbound::OutboundAttempt;

pub type Id = [u8; 32];
#[derive(Debug)]
pub enum Error {
    Storage(rusqlite::Error),
    Io(std::io::Error),
    Crypto(sigil_crypto::Error),
    InvalidStore,
    InvalidEvent,
    Conflict,
    NotFound,
    Limit,
    AlreadyDelivered,
    Expired,
    Cancelled,
    /// Authenticated retained content was deleted or superseded.
    Obsolete,
    Unprepared,
    UnsupportedSession,
    RetiredSession,
    Network(network::Error),
    Preview(sigil_media::Error),
}
impl From<sigil_media::Error> for Error {
    fn from(value: sigil_media::Error) -> Self {
        Self::Preview(value)
    }
}
impl From<rusqlite::Error> for Error {
    fn from(value: rusqlite::Error) -> Self {
        Self::Storage(value)
    }
}
impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
impl From<sigil_crypto::Error> for Error {
    fn from(value: sigil_crypto::Error) -> Self {
        Self::Crypto(value)
    }
}
impl From<network::Error> for Error {
    fn from(value: network::Error) -> Self {
        Self::Network(value)
    }
}

pub struct ClientStore {
    db: Connection,
    key: StorageKey,
}
fn binding(kind: u8, session: &Id, record: &[u8]) -> Vec<u8> {
    let mut bytes = b"Sigil/client/v0".to_vec();
    bytes.push(kind);
    bytes.extend_from_slice(session);
    bytes.extend_from_slice(record);
    bytes
}
impl ClientStore {
    /// Caller supplies a private directory and an independent platform-wrapped key.
    /// Never restore this live-state database from a history backup.
    pub fn open(path: &Path, key: StorageKey) -> Result<Self, Error> {
        Self::open_with_storage_limit(path, key, 1024 * 1024 * 1024)
    }

    /// Limit SQLite database pages, including retained history and replay evidence.
    /// Callers persist their chosen budget and supply it on every open. Raising
    /// the budget never requires replacing identities or resetting the database.
    /// Rollback journals and migration vacuum require additional filesystem space.
    pub fn open_with_storage_limit(
        path: &Path,
        key: StorageKey,
        bytes: u64,
    ) -> Result<Self, Error> {
        let mut db = private_db::open(path, bytes)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type IN ('trigger','view'))",
            [],
            |row| row.get::<_, bool>(0),
        )? {
            return Err(Error::InvalidStore);
        }
        let version: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        let app: i64 = tx.query_row("PRAGMA application_id", [], |r| r.get(0))?;
        if version == 0 && app == 0 {
            let count: i64 =
                tx.query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get(0))?;
            if count != 0 {
                return Err(Error::InvalidStore);
            }
            tx.execute_batch("CREATE TABLE vault(id INTEGER PRIMARY KEY CHECK(id=1), verifier BLOB NOT NULL);
                CREATE TABLE sessions(id BLOB PRIMARY KEY, revision INTEGER NOT NULL, state BLOB NOT NULL);
                CREATE TABLE outbox(session BLOB REFERENCES sessions(id), id BLOB, tag BLOB NOT NULL, packet BLOB, PRIMARY KEY(session,id));
                CREATE TABLE inbox(session BLOB REFERENCES sessions(id), id BLOB, tag BLOB NOT NULL, content BLOB NOT NULL, PRIMARY KEY(session,id));
                PRAGMA application_id=1397179212; PRAGMA user_version=1;")?;
            tx.execute(
                "INSERT INTO vault VALUES(1,?1)",
                [key.seal(b"Sigil client storage", b"vault/v0")?],
            )?;
        } else if !(1..=68).contains(&version) || app != 1397179212 {
            return Err(Error::InvalidStore);
        }
        let verifier: Vec<u8> = tx.query_row(
            "SELECT verifier FROM vault WHERE id=1 AND length(verifier)<128",
            [],
            |r| r.get(0),
        )?;
        if key.open(&verifier, b"vault/v0")?.as_slice() != b"Sigil client storage" {
            return Err(Error::InvalidStore);
        }
        if version < 2 {
            tx.execute_batch(
                "CREATE TABLE identity(id INTEGER PRIMARY KEY CHECK(id=1), state BLOB NOT NULL);
                CREATE TABLE prekeys(id BLOB PRIMARY KEY, state BLOB);
                PRAGMA user_version=2;",
            )?;
        }
        if version < 3 {
            tx.execute_batch("CREATE TABLE initiations(prekey BLOB PRIMARY KEY, session BLOB NOT NULL UNIQUE REFERENCES sessions(id)); PRAGMA user_version=3;")?;
        }
        if version < 4 {
            tx.execute_batch("CREATE TABLE deliveries(id BLOB PRIMARY KEY, session BLOB NOT NULL, metadata BLOB NOT NULL, FOREIGN KEY(session,id) REFERENCES outbox(session,id)); PRAGMA user_version=4;")?;
        }
        if version < 5 {
            tx.execute_batch(
                "ALTER TABLE deliveries ADD COLUMN receipt BLOB; PRAGMA user_version=5;",
            )?;
        }
        if version < 6 {
            tx.execute_batch("ALTER TABLE sessions ADD COLUMN suite INTEGER NOT NULL DEFAULT 1 CHECK(suite IN (1,2)); PRAGMA user_version=6;")?;
        }
        if version < 7 {
            tx.execute_batch("ALTER TABLE outbox ADD COLUMN content BLOB; PRAGMA user_version=7;")?;
        }
        if version < 8 {
            tx.execute_batch(recovery::MIGRATION)?;
        }
        if version < 9 {
            tx.execute_batch(connection::MIGRATION)?;
        }
        if version < 10 {
            tx.execute_batch(prekeys::MIGRATION)?;
        }
        if version < 11 {
            tx.execute_batch(claims::MIGRATION)?;
        }
        if version < 12 {
            tx.execute_batch(prekeys::RETENTION_MIGRATION)?;
        }
        if version < 13 {
            tx.execute_batch(peers::MIGRATION)?;
        }
        if version < 14 {
            tx.execute_batch("ALTER TABLE sessions ADD COLUMN peer BLOB CHECK(peer IS NULL OR length(peer)=32); PRAGMA user_version=14;")?;
        }
        if version < 15 {
            tx.execute_batch(incoming::MIGRATION)?;
        }
        if version < 16 {
            tx.execute_batch("CREATE TABLE incoming_cursor(id INTEGER PRIMARY KEY CHECK(id=1), state BLOB NOT NULL); PRAGMA user_version=16;")?;
        }
        if version < 17 {
            tx.execute_batch("CREATE TABLE text_events(id BLOB PRIMARY KEY, state BLOB NOT NULL); PRAGMA user_version=17;")?;
        }
        if version < 18 {
            tx.execute_batch("CREATE TABLE archive_checkpoints(generation INTEGER PRIMARY KEY,manifest BLOB NOT NULL,data BLOB NOT NULL); PRAGMA user_version=18;")?;
        }
        if version < 19 {
            tx.execute_batch("CREATE TABLE archive_competition(id BLOB PRIMARY KEY,data BLOB NOT NULL); PRAGMA user_version=19;")?;
        }
        if version < 20 {
            tx.execute_batch("ALTER TABLE sessions ADD COLUMN retired INTEGER NOT NULL DEFAULT 0 CHECK(retired IN (0,1)); PRAGMA user_version=20;")?;
        }
        if version < 21 {
            tx.execute_batch(
                "ALTER TABLE deliveries ADD COLUMN expired BLOB; PRAGMA user_version=21;",
            )?;
        }
        if version < 22 {
            tx.execute_batch("CREATE TABLE active_sessions(peer BLOB PRIMARY KEY,state BLOB NOT NULL); PRAGMA user_version=22;")?;
        }
        if version < 23 {
            tx.execute_batch("CREATE TABLE initial_headers(session BLOB PRIMARY KEY REFERENCES sessions(id),data BLOB NOT NULL); PRAGMA user_version=23;")?;
        }
        if version < 24 {
            tx.execute_batch("PRAGMA user_version=24;")?;
        }
        if version < 25 {
            tx.execute_batch("CREATE TABLE retry_outbox(id BLOB PRIMARY KEY,state BLOB NOT NULL); CREATE TABLE retry_requests(id BLOB PRIMARY KEY,state BLOB NOT NULL); PRAGMA user_version=25;")?;
        }
        if version < 26 {
            // Earlier clients cannot validate cross-session resend history.
            tx.execute_batch("PRAGMA user_version=26;")?;
        }
        if version < 27 {
            tx.execute_batch("CREATE TABLE retry_incoming(sequence INTEGER PRIMARY KEY CHECK(sequence>0), acknowledged INTEGER NOT NULL CHECK(acknowledged IN (0,1)), state BLOB NOT NULL); PRAGMA user_version=27;")?;
        }
        if version < 28 {
            tx.execute_batch("CREATE TABLE retry_cursor(id INTEGER PRIMARY KEY CHECK(id=1),state BLOB NOT NULL); PRAGMA user_version=28;")?;
        }
        if version < 29 {
            tx.execute_batch("ALTER TABLE retry_requests ADD COLUMN finished INTEGER NOT NULL DEFAULT 0 CHECK(finished IN (0,1,2)); CREATE INDEX retry_pending ON retry_requests(finished,id); PRAGMA user_version=29;")?;
        }
        if version < 30 {
            tx.execute_batch("CREATE TABLE retry_gc_cursor(id INTEGER PRIMARY KEY CHECK(id=1),state BLOB NOT NULL); PRAGMA user_version=30;")?;
        }
        if version < 31 {
            tx.execute_batch("CREATE TABLE retired_retry_outbox(id BLOB PRIMARY KEY,state BLOB NOT NULL); CREATE INDEX inbox_message ON inbox(id); PRAGMA user_version=31;")?;
        }
        if version < 32 {
            tx.execute_batch("CREATE TABLE retired_retry_requests(id BLOB PRIMARY KEY,state BLOB NOT NULL); PRAGMA user_version=32;")?;
        }
        if version < 33 {
            tx.execute_batch("CREATE TABLE recovered_deliveries(sequence INTEGER PRIMARY KEY CHECK(sequence>0),acknowledged INTEGER NOT NULL CHECK(acknowledged IN (0,1)),state BLOB NOT NULL); PRAGMA user_version=33;")?;
        }
        if version < 34 {
            tx.execute_batch("ALTER TABLE retry_gc_cursor RENAME TO retry_gc_cursor_old; CREATE TABLE retry_gc_cursor(id INTEGER PRIMARY KEY CHECK(id IN (1,2)),state BLOB NOT NULL); INSERT INTO retry_gc_cursor SELECT id,state FROM retry_gc_cursor_old; DROP TABLE retry_gc_cursor_old; PRAGMA user_version=34;")?;
        }
        if version < 35 {
            tx.execute_batch("CREATE TABLE device_link_records(id BLOB PRIMARY KEY,state BLOB NOT NULL); PRAGMA user_version=35;")?;
        }
        if version < 36 {
            tx.execute_batch("CREATE TABLE session_activity(id BLOB PRIMARY KEY,state BLOB NOT NULL); CREATE TABLE session_maintenance(id INTEGER PRIMARY KEY CHECK(id=1),state BLOB NOT NULL); PRAGMA user_version=36;")?;
        }
        if version < 37 {
            tx.execute_batch("CREATE TABLE outbound_cursor(id INTEGER PRIMARY KEY CHECK(id=1),state BLOB NOT NULL); CREATE INDEX outbox_pending_sessions ON outbox(session) WHERE packet IS NOT NULL; PRAGMA user_version=37;")?;
        }
        if version < 38 {
            tx.execute_batch("CREATE TABLE prekey_cursor(id INTEGER PRIMARY KEY CHECK(id=1),state BLOB NOT NULL); CREATE INDEX prekey_pending_publications ON prekey_publications(slot) WHERE retire_at IS NULL; PRAGMA user_version=38;")?;
        }
        if version < 39 {
            tx.execute_batch("CREATE TABLE sync_schedule(id INTEGER PRIMARY KEY CHECK(id=1),state BLOB NOT NULL); PRAGMA user_version=39;")?;
        }
        if version < 40 {
            tx.execute_batch("CREATE TABLE send_intents(id BLOB PRIMARY KEY,state BLOB NOT NULL); CREATE TABLE send_intent_cursor(id INTEGER PRIMARY KEY CHECK(id=1),state BLOB NOT NULL); PRAGMA user_version=40;")?;
        }
        if version < 41 {
            tx.execute_batch("ALTER TABLE retry_outbox ADD COLUMN complete INTEGER NOT NULL DEFAULT 0 CHECK(complete IN (0,1)); CREATE INDEX retry_outbox_pending ON retry_outbox(id) WHERE complete=0; CREATE TABLE retry_send_cursor(id INTEGER PRIMARY KEY CHECK(id=1),state BLOB NOT NULL); PRAGMA user_version=41;")?;
        }
        if version < 42 {
            lifetime::migrate(&tx, &key)?;
            tx.pragma_update(None, "user_version", 42)?;
        }
        if version < 43 {
            tx.execute_batch(groups::MIGRATION)?;
        }
        if version < 44 {
            tx.execute_batch(groups::KEY_MIGRATION)?;
        }
        if version < 45 {
            tx.execute_batch(groups::MESSAGE_MIGRATION)?;
        }
        if version < 46 {
            // Older clients cannot interpret typed file events/recovery records.
            tx.pragma_update(None, "user_version", 46)?;
        }
        if version < 47 {
            tx.execute_batch("ALTER TABLE sync_schedule RENAME TO sync_schedule_old; CREATE TABLE sync_schedule(id INTEGER PRIMARY KEY CHECK(id IN (1,2)),state BLOB NOT NULL); INSERT INTO sync_schedule SELECT id,state FROM sync_schedule_old; DROP TABLE sync_schedule_old; CREATE TABLE archive_work(id INTEGER PRIMARY KEY CHECK(id=1),state BLOB NOT NULL); PRAGMA user_version=47;")?;
        }
        if version < 48 {
            tx.execute_batch("CREATE TABLE archive_media(record BLOB PRIMARY KEY REFERENCES archive_records(id) ON DELETE CASCADE,content BLOB NOT NULL); CREATE INDEX archive_media_content ON archive_media(content,record);")?;
            recovery::migrate_media(&tx, &key)?;
            tx.pragma_update(None, "user_version", 48)?;
        }
        if version < 49 {
            tx.execute_batch("ALTER TABLE sync_schedule RENAME TO sync_schedule_old; CREATE TABLE sync_schedule(id INTEGER PRIMARY KEY CHECK(id IN (1,2,3)),state BLOB NOT NULL); INSERT INTO sync_schedule SELECT id,state FROM sync_schedule_old; DROP TABLE sync_schedule_old; CREATE TABLE push_state(id INTEGER PRIMARY KEY CHECK(id=1),state BLOB NOT NULL); PRAGMA user_version=49;")?;
        }
        if version < 50 {
            // Older clients cannot interpret canonical SigilText events/recovery.
            tx.pragma_update(None, "user_version", 50)?;
        }
        if version < 51 {
            // Structured cards add authenticated account origins to rich content.
            tx.pragma_update(None, "user_version", 51)?;
        }
        if version < 52 {
            tx.execute_batch(structured::MIGRATION)?;
        }
        if version < 53 {
            tx.execute_batch(structured::tasks::MIGRATION)?;
        }
        if version < 54 {
            // Earlier readers must not reopen journals containing per-period actions.
            tx.pragma_update(None, "user_version", 54)?;
        }
        if version < 55 {
            tx.execute_batch(groups::SERVICE_MIGRATION)?;
        }
        if version < 56 {
            tx.execute_batch(groups::ENVELOPE_MIGRATION)?;
            groups::migrate_envelopes(&tx, &key)?;
        }
        if version < 57 {
            tx.execute_batch(groups::WORK_MIGRATION)?;
        }
        if version < 58 {
            tx.execute_batch(groups::INVITATION_MIGRATION)?;
        }
        if version < 59 {
            tx.execute_batch(groups::BOOTSTRAP_MIGRATION)?;
        }
        if version < 60 {
            tx.execute_batch(groups::KEY_RECOVERY_MIGRATION)?;
        }
        if version < 61 {
            tx.execute_batch(groups::HISTORY_MIGRATION)?;
        }
        if version < 62 {
            tx.execute_batch(conversations::MIGRATION)?;
        }
        if version < 63 {
            tx.execute_batch(recovery::LIFECYCLE_MIGRATION)?;
        }
        if version < 64 {
            tx.execute_batch(structured::COMPOSITION_MIGRATION)?;
            tx.execute_batch(structured::polls::MIGRATION)?;
            tx.execute_batch(structured::alarms::MIGRATION)?;
        }
        if version < 65 {
            tx.execute_batch(structured::locations::MIGRATION)?;
        }
        if version < 66 {
            tx.execute_batch(services::MIGRATION)?;
        }
        if version < 67 {
            tx.execute_batch(calls::MIGRATION)?;
        }
        if version < 68 {
            tx.execute_batch(private_db::MIGRATION)?;
            tx.execute_batch("CREATE TABLE IF NOT EXISTS erasure_cursor(kind INTEGER PRIMARY KEY,state BLOB NOT NULL);")?;
            tx.execute_batch("CREATE TABLE IF NOT EXISTS conversation_transfer_origins(id BLOB PRIMARY KEY,state BLOB NOT NULL); CREATE TABLE IF NOT EXISTS conversation_transfer_parts(id BLOB NOT NULL,part INTEGER NOT NULL,state BLOB NOT NULL,PRIMARY KEY(id,part));")?;
            tx.pragma_update(None, "user_version", 68)?;
        }
        if version < 52 {
            if version >= 51 {
                event::migrate_structured(&tx, &key)?;
                groups::migrate_structured(&tx, &key)?;
            }
            recovery::migrate_structured(&tx, &key)?;
        }
        if version < 63 {
            conversations::migrate(&tx, &key)?;
        }
        tx.commit()?;
        private_db::finish(&db)?;
        Ok(Self { db, key })
    }

    /// Import an already authenticated session. For new incoming handshakes use accept_initial.
    pub fn insert_session(&mut self, id: Id, session: Session) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        insert(&tx, &self.key, &id, &session)?;
        tx.commit()?;
        Ok(())
    }

    /// Returns bytes only after both advanced state and the outbox entry commit.
    /// Reuse the same id for retries; different plaintext under that id conflicts.
    pub fn send(&mut self, session: Id, id: Id, plaintext: &[u8]) -> Result<Vec<u8>, Error> {
        if plaintext.len() > MAX_PLAINTEXT {
            return Err(Error::Limit);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let packet = send_in(&tx, &self.key, session, id, plaintext)?;
        tx.commit()?;
        Ok(packet)
    }

    /// Authenticate and durably commit plaintext plus candidate state before returning.
    /// Only after success may the caller acknowledge the server delivery.
    pub fn receive(
        &mut self,
        session: Id,
        id: Id,
        packet: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let plaintext = if sigil_protocol::initial::decode(packet).is_ok() {
            handshake::accept_repeated(&tx, &self.key, session, id, packet)?
        } else {
            let parsed = Packet::from_bytes(packet)?;
            receive_in(&tx, &self.key, session, id, packet, &parsed)?
        };
        if groups::is_wire_control(&plaintext) {
            return Err(Error::Unprepared);
        }
        tx.commit()?;
        Ok(plaintext)
    }

    /// Read an already committed message without contacting the server.
    pub fn message(&self, session: Id, id: Id) -> Result<Zeroizing<Vec<u8>>, Error> {
        let content: Vec<u8> = self
            .db
            .query_row(
                "SELECT content FROM inbox WHERE session=?1 AND id=?2 AND length(content)<=65572",
                (session.as_slice(), id.as_slice()),
                |r| r.get(0),
            )
            .optional()?
            .ok_or(Error::NotFound)?;
        let raw = self.key.open(&content, &binding(2, &session, &id))?;
        conversations::require_payload(&self.db, &self.key, &raw, conversations::now(), false)?;
        Ok(raw)
    }

    /// Locally retained outgoing text survives server acknowledgement. Earlier
    /// schemas have no recoverable outgoing text; missing content is not inferred.
    pub fn outgoing_message(&self, session: Id, id: Id) -> Result<Zeroizing<Vec<u8>>, Error> {
        let content: Vec<u8> = self.db.query_row(
            "SELECT content FROM outbox WHERE session=?1 AND id=?2 AND content IS NOT NULL AND length(content)<=65572",
            (session.as_slice(), id.as_slice()), |r| r.get(0)).optional()?.ok_or(Error::NotFound)?;
        let raw = self.key.open(&content, &binding(9, &session, &id))?;
        conversations::require_payload(&self.db, &self.key, &raw, conversations::now(), false)?;
        Ok(raw)
    }

    /// Bounded restart queue. IDs determine retry identity, not UI ordering.
    pub fn pending(&self, session: Id) -> Result<Vec<(Id, Vec<u8>)>, Error> {
        load(&self.db, &self.key, &session)?;
        let mut statement = self.db.prepare("SELECT id,packet FROM outbox WHERE session=?1 AND packet IS NOT NULL AND length(id)=32 AND length(packet)<=67266 ORDER BY rowid LIMIT 16")?;
        let mut rows = statement.query([session.as_slice()])?;
        let mut result = Vec::new();
        while let Some(row) = rows.next()? {
            let id: Vec<u8> = row.get(0)?;
            let id: Id = id.try_into().map_err(|_| Error::InvalidStore)?;
            let sealed: Vec<u8> = row.get(1)?;
            let packet = open_packet(&self.key, &sealed, &session, &id)?;
            result.push((id, packet));
        }
        Ok(result)
    }
}

fn retained_payload<'a>(
    key: &StorageKey,
    plaintext: &'a [u8],
) -> Result<std::borrow::Cow<'a, [u8]>, Error> {
    match calls::retained(key, plaintext)? {
        std::borrow::Cow::Owned(v) => Ok(std::borrow::Cow::Owned(v)),
        std::borrow::Cow::Borrowed(v) => groups::retained_payload(key, v),
    }
}
fn send_in(
    tx: &Transaction<'_>,
    key: &StorageKey,
    session: Id,
    id: Id,
    plaintext: &[u8],
) -> Result<Vec<u8>, Error> {
    let tag = key.commitment(plaintext, &binding(1, &session, &id))?;
    if let Some(packet) = retry_outgoing(tx, key, &session, &id, &tag)? {
        return Ok(packet);
    }
    let pending: i64 = tx.query_row(
        "SELECT count(*) FROM outbox WHERE session=?1 AND packet IS NOT NULL",
        [session.as_slice()],
        |r| r.get(0),
    )?;
    if pending >= 256 {
        return Err(Error::Limit);
    }
    let (revision, mut state) = load(tx, key, &session)?;
    let packet = state.send(plaintext)?.to_bytes();
    let packet = handshake::wrap(tx, key, &session, &state, packet, plaintext.len())?;
    save(tx, key, &session, revision, &state)?;
    queue(tx, key, &session, &id, &tag, &packet, plaintext)?;
    Ok(packet)
}
fn receive_in(
    tx: &Transaction<'_>,
    key: &StorageKey,
    session: Id,
    id: Id,
    packet: &[u8],
    parsed: &Packet,
) -> Result<Zeroizing<Vec<u8>>, Error> {
    require_triple(tx, &session)?;
    let aad = binding(2, &session, &id);
    let tag = key.commitment(&Sha256::digest(packet), &aad)?;
    let prior: Option<(Vec<u8>,Vec<u8>)> = tx.query_row("SELECT tag,content FROM inbox WHERE session=?1 AND id=?2 AND length(tag)=32 AND length(content)<=65572", (session.as_slice(), id.as_slice()), |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
    if let Some((old, content)) = prior {
        if old != tag {
            return Err(Error::Conflict);
        }
        let raw = key.open(&content, &aad)?;
        erasure::require_retained(&raw)?;
        return Ok(raw);
    }
    let mut state = load(tx, key, &session)?;
    let plaintext = Zeroizing::new(state.1.receive(parsed)?);
    commit_received(tx, key, session, id, packet, &state, &plaintext)?;
    Ok(plaintext)
}
fn commit_received(
    tx: &Transaction<'_>,
    key: &StorageKey,
    session: Id,
    id: Id,
    packet: &[u8],
    state: &(i64, Session),
    plaintext: &[u8],
) -> Result<(), Error> {
    let aad = binding(2, &session, &id);
    let tag = key.commitment(&Sha256::digest(packet), &aad)?;
    let content = key.seal(&retained_payload(key, plaintext)?, &aad)?;
    save(tx, key, &session, state.0, &state.1)?;
    tx.execute(
        "INSERT INTO inbox VALUES(?1,?2,?3,?4)",
        (session.as_slice(), id.as_slice(), tag.as_slice(), content),
    )?;
    Ok(())
}
fn require_triple(db: &Connection, id: &Id) -> Result<(), Error> {
    let (suite, retired): (i64, bool) = db
        .query_row(
            "SELECT suite,retired FROM sessions WHERE id=?1",
            [id.as_slice()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    if retired {
        Err(Error::RetiredSession)
    } else if suite == 2 {
        Ok(())
    } else {
        Err(Error::UnsupportedSession)
    }
}

fn session_peer(db: &Connection, id: &Id) -> Result<Option<Id>, Error> {
    let peer: Option<Vec<u8>> = db
        .query_row(
            "SELECT peer FROM sessions WHERE id=?1",
            [id.as_slice()],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    peer.map(|v| v.try_into().map_err(|_| Error::InvalidStore))
        .transpose()
}
fn state_binding(id: &Id, revision: i64, peer: Option<Id>) -> Vec<u8> {
    let mut aad = binding(0, id, &revision.to_be_bytes());
    if let Some(peer) = peer {
        aad.extend_from_slice(b"Sigil/peer-session/v0");
        aad.extend_from_slice(&peer);
    }
    aad
}
fn load(tx: &Connection, key: &StorageKey, id: &Id) -> Result<(i64, Session), Error> {
    require_triple(tx, id)?;
    let (revision, state): (i64, Vec<u8>) = tx
        .query_row(
            "SELECT revision,state FROM sessions WHERE id=?1 AND length(state)<=?2",
            (id.as_slice(), MAX_SEALED_CHECKPOINT_LEN as i64),
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    if revision < 0 {
        return Err(Error::InvalidStore);
    }
    let peer = session_peer(tx, id)?;
    let state = Session::open_checkpoint(key, &state, &state_binding(id, revision, peer))?;
    if let Some(peer) = peer {
        peers::destination(tx, key, &peer)?;
    }
    Ok((revision, state))
}
fn save(
    tx: &Transaction<'_>,
    key: &StorageKey,
    id: &Id,
    revision: i64,
    state: &Session,
) -> Result<(), Error> {
    let next = revision.checked_add(1).ok_or(Error::Limit)?;
    let sealed = state.seal_checkpoint(key, &state_binding(id, next, session_peer(tx, id)?))?;
    if tx.execute(
        "UPDATE sessions SET revision=?1,state=?2 WHERE id=?3 AND revision=?4",
        (next, sealed, id.as_slice(), revision),
    )? != 1
    {
        return Err(Error::Conflict);
    }
    Ok(())
}

fn insert(tx: &Transaction<'_>, key: &StorageKey, id: &Id, session: &Session) -> Result<(), Error> {
    if tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",
        [id.as_slice()],
        |r| r.get::<_, bool>(0),
    )? {
        return Err(Error::Conflict);
    }
    let sealed = session.seal_checkpoint(key, &binding(0, id, &0_i64.to_be_bytes()))?;
    tx.execute(
        "INSERT INTO sessions(id,revision,state,suite) VALUES(?1,0,?2,2)",
        (id.as_slice(), sealed),
    )?;
    Ok(())
}

fn retry_outgoing(
    tx: &Transaction<'_>,
    key: &StorageKey,
    session: &Id,
    id: &Id,
    tag: &Id,
) -> Result<Option<Vec<u8>>, Error> {
    type Row = (Vec<u8>, Option<Vec<u8>>);
    let prior:Option<Row>=tx.query_row("SELECT tag,packet FROM outbox WHERE session=?1 AND id=?2 AND length(tag)=32 AND (packet IS NULL OR length(packet)<=67266)",(session.as_slice(),id.as_slice()),|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    let Some((old, packet)) = prior else {
        return Ok(None);
    };
    load(tx, key, session)?;
    if old != tag {
        return Err(Error::Conflict);
    }
    let packet = match packet {
        Some(packet) => packet,
        None if conversations::cancelled(tx, key, session, id)? => return Err(Error::Cancelled),
        None if transport::expired(tx, key, session, id)? => return Err(Error::Expired),
        None => return Err(Error::AlreadyDelivered),
    };
    Ok(Some(open_packet(key, &packet, session, id)?))
}

fn open_packet(key: &StorageKey, sealed: &[u8], session: &Id, id: &Id) -> Result<Vec<u8>, Error> {
    let packet = key.open(sealed, &binding(3, session, id))?;
    // Raw PQXDH initial packets from earlier experiments lack the ratchet
    // bootstrap. Never retransmit them under the current native protocol.
    if packet.starts_with(b"SGHI") {
        sigil_protocol::initial::decode(&packet).map_err(|_| Error::UnsupportedSession)?;
    } else {
        Packet::from_bytes(&packet).map_err(|_| Error::UnsupportedSession)?;
    }
    Ok(packet.to_vec())
}
fn queue(
    tx: &Transaction<'_>,
    key: &StorageKey,
    session: &Id,
    id: &Id,
    tag: &Id,
    packet: &[u8],
    plaintext: &[u8],
) -> Result<(), Error> {
    let sealed = key.seal(packet, &binding(3, session, id))?;
    let content = key.seal(&retained_payload(key, plaintext)?, &binding(9, session, id))?;
    tx.execute(
        "INSERT INTO outbox(session,id,tag,packet,content) VALUES(?1,?2,?3,?4,?5)",
        (
            session.as_slice(),
            id.as_slice(),
            tag.as_slice(),
            sealed,
            content,
        ),
    )?;
    Ok(())
}
