//! Durable history archives. No operation in this module imports live sessions.
#[path = "recovery_cleanup.rs"]
mod cleanup;
#[path = "recovery_media_cleanup.rs"]
mod media_cleanup;
pub(crate) use media_cleanup::removed as media_removed;
#[path = "recovery_competition.rs"]
mod competition;
#[path = "recovery_lifecycle.rs"]
mod lifecycle;
#[path = "recovery_media.rs"]
mod media;
#[path = "recovery_work.rs"]
mod work;
use super::*;
pub(crate) use lifecycle::MIGRATION as LIFECYCLE_MIGRATION;
pub use lifecycle::{HistoryProgress, MediaCheckpoint, MediaCheckpointPage, RecoveryPolicy};
pub(crate) use media::{copy_id, recovery_file};
pub(super) use media::{migrate_media, references_file, retained_file};
use sigil_crypto::{
    recovery::{
        Content, Head, Manifest, Object, Record, RecoveryKey, Reference, MAX_MANIFEST_PAGES,
        MAX_OBJECT_LEN, MAX_PAGE_RECORDS,
    },
    Secret32,
};
pub use work::{RecoveryMaintenance, RecoveryProgress, ScheduledRecovery};

pub(crate) const MIGRATION: &str = "
CREATE TABLE archive(id INTEGER PRIMARY KEY CHECK(id=1),scope BLOB NOT NULL,state BLOB NOT NULL);
CREATE TABLE archive_records(id BLOB PRIMARY KEY,revision INTEGER NOT NULL,object BLOB NOT NULL,data BLOB NOT NULL);
CREATE TABLE archive_objects(id BLOB PRIMARY KEY,data BLOB NOT NULL,uploaded INTEGER NOT NULL DEFAULT 0 CHECK(uploaded IN (0,1)));
CREATE TABLE archive_import(id BLOB PRIMARY KEY,revision INTEGER NOT NULL,object BLOB NOT NULL,data BLOB NOT NULL);
CREATE TABLE archive_pages(position INTEGER PRIMARY KEY,object BLOB NOT NULL,data BLOB NOT NULL);
PRAGMA user_version=8;";
const MAX_RECORDS: usize = MAX_PAGE_RECORDS * MAX_MANIFEST_PAGES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Upload,
    Import,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub anchor: Option<Head>,
    pub pending: Option<(Operation, Head)>,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Download {
    Manifest {
        head: Head,
    },
    Page {
        index: usize,
        object: Id,
    },
    Records {
        index: usize,
        records: Vec<Reference>,
    },
    Complete,
}
struct State {
    scope: Id,
    key: RecoveryKey,
    status: Status,
    reconcile_restored: bool,
    restore_base: Option<Head>,
}

/// Canonical server DNS name and stable account ID, never a transport device ID.
pub fn account_scope(server: &str, account: Id) -> Result<Id, Error> {
    if !sigil_protocol::valid_server_name(server) {
        return Err(Error::InvalidStore);
    }
    Ok(Sha256::digest(
        [
            b"Sigil/recovery/account/v0".as_slice(),
            &(server.len() as u16).to_be_bytes(),
            server.as_bytes(),
            &account,
        ]
        .concat(),
    )
    .into())
}
fn take_head(bytes: &[u8]) -> Result<Head, Error> {
    if bytes.len() != 40 {
        return Err(Error::InvalidStore);
    }
    let generation = u64::from_be_bytes(bytes[..8].try_into().map_err(|_| Error::InvalidStore)?);
    if generation == 0 || generation > i64::MAX as u64 {
        return Err(Error::InvalidStore);
    }
    Ok(Head {
        generation,
        manifest: bytes[8..].try_into().map_err(|_| Error::InvalidStore)?,
    })
}
fn server_head(response: &sigil_protocol::recovery::Head) -> Result<Head, Error> {
    let id = response.manifest.as_ref().ok_or(Error::NotFound)?;
    if response.restored_checkpoint || !sigil_protocol::accounts::valid_credential(id) {
        return Err(Error::Conflict);
    }
    let mut bytes = [0; 40];
    bytes[..8].copy_from_slice(&response.generation.to_be_bytes());
    for (target, pair) in bytes[8..].iter_mut().zip(id.as_bytes().as_chunks::<2>().0) {
        *target = u8::from_str_radix(std::str::from_utf8(pair).map_err(|_| Error::Conflict)?, 16)
            .map_err(|_| Error::Conflict)?;
    }
    take_head(&bytes)
}
fn put_head(bytes: &mut Vec<u8>, head: Head) {
    bytes.extend_from_slice(&head.generation.to_be_bytes());
    bytes.extend_from_slice(&head.manifest);
}
fn load(db: &Connection, wrapping: &StorageKey) -> Result<State, Error> {
    let (scope, sealed): (Vec<u8>, Vec<u8>) = db.query_row("SELECT scope,state FROM archive WHERE id=1 AND length(scope)=32 AND length(state)<=224", [], |r| Ok((r.get(0)?,r.get(1)?))).optional()?.ok_or(Error::NotFound)?;
    let scope: Id = scope.try_into().map_err(|_| Error::InvalidStore)?;
    let bytes = wrapping.open(&sealed, &binding(10, &scope, b"archive/v0"))?;
    if bytes.len() < 34 {
        return Err(Error::InvalidStore);
    }
    let key = RecoveryKey::from_secret(
        Secret32::from_bytes(bytes[..32].try_into().map_err(|_| Error::InvalidStore)?),
        scope,
    )?;
    let mut offset = 33;
    let anchor = match bytes[32] {
        0 => None,
        1 => {
            let head = take_head(bytes.get(offset..offset + 40).ok_or(Error::InvalidStore)?)?;
            offset += 40;
            Some(head)
        }
        _ => return Err(Error::InvalidStore),
    };
    let mode = *bytes.get(offset).ok_or(Error::InvalidStore)?;
    offset += 1;
    let pending = match mode {
        0 => None,
        1 | 2 => {
            let head = take_head(bytes.get(offset..offset + 40).ok_or(Error::InvalidStore)?)?;
            offset += 40;
            Some((
                if mode == 1 {
                    Operation::Upload
                } else {
                    Operation::Import
                },
                head,
            ))
        }
        _ => return Err(Error::InvalidStore),
    };
    // Older states omit this optional authenticated marker. A repair is tied
    // to the unchanged trusted anchor until its successor is acknowledged.
    let (reconcile_restored, restore_base) = match bytes.get(offset..) {
        Some([]) => (false, None),
        Some([1]) if anchor.is_some() && !matches!(pending, Some((Operation::Import, _))) => {
            (true, None)
        }
        Some([2, rest @ ..])
            if anchor.is_some() && !matches!(pending, Some((Operation::Import, _))) =>
        {
            let base = take_head(rest)?;
            if !anchor.is_some_and(|anchor| base.generation < anchor.generation) {
                return Err(Error::InvalidStore);
            }
            (true, Some(base))
        }
        _ => return Err(Error::InvalidStore),
    };
    Ok(State {
        scope,
        key,
        status: Status { anchor, pending },
        reconcile_restored,
        restore_base,
    })
}
fn save(db: &Connection, wrapping: &StorageKey, state: &State) -> Result<(), Error> {
    let mut bytes = Zeroizing::new(state.key.export_secret().to_vec());
    bytes.push(u8::from(state.status.anchor.is_some()));
    if let Some(head) = state.status.anchor {
        put_head(&mut bytes, head);
    }
    bytes.push(match state.status.pending {
        None => 0,
        Some((Operation::Upload, _)) => 1,
        Some((Operation::Import, _)) => 2,
    });
    if let Some((_, head)) = state.status.pending {
        put_head(&mut bytes, head);
    }
    if let Some(base) = state.restore_base {
        bytes.push(2);
        put_head(&mut bytes, base);
    } else if state.reconcile_restored {
        bytes.push(1);
    }
    let sealed = wrapping.seal(&bytes, &binding(10, &state.scope, b"archive/v0"))?;
    db.execute(
        "INSERT INTO archive VALUES(1,?1,?2) ON CONFLICT(id) DO UPDATE SET state=excluded.state",
        (state.scope.as_slice(), sealed),
    )?;
    Ok(())
}
fn idle(state: &State) -> Result<(), Error> {
    if state.status.pending.is_some() {
        Err(Error::Conflict)
    } else {
        Ok(())
    }
}
fn pending(state: &State, operation: Operation) -> Result<Head, Error> {
    match state.status.pending {
        Some((mode, head)) if mode == operation => Ok(head),
        _ => Err(Error::Unprepared),
    }
}
fn row_record(row: &rusqlite::Row<'_>) -> Result<(Reference, Vec<u8>), Error> {
    let id: Id = row
        .get_ref(0)?
        .as_blob()
        .map_err(|_| Error::InvalidStore)?
        .try_into()
        .map_err(|_| Error::InvalidStore)?;
    let revision: i64 = row.get(1)?;
    let object: Id = row
        .get_ref(2)?
        .as_blob()
        .map_err(|_| Error::InvalidStore)?
        .try_into()
        .map_err(|_| Error::InvalidStore)?;
    let data = row.get_ref(3)?.as_blob().map_err(|_| Error::InvalidStore)?;
    if revision <= 0 || data.len() > MAX_OBJECT_LEN {
        return Err(Error::InvalidStore);
    }
    Ok((
        Reference {
            id,
            revision: revision as u64,
            object,
        },
        data.to_vec(),
    ))
}
fn existing(db: &Connection, id: Id) -> Result<Option<(Reference, Vec<u8>)>, Error> {
    let mut statement =
        db.prepare("SELECT id,revision,object,data FROM archive_records WHERE id=?1")?;
    let mut rows = statement.query([id.as_slice()])?;
    rows.next()?.map(row_record).transpose()
}
fn record_page(db: &Connection, after: &[u8]) -> Result<Vec<(Reference, Vec<u8>)>, Error> {
    let mut statement = db.prepare(
        "SELECT id,revision,object,data FROM archive_records WHERE id>?1 ORDER BY id LIMIT 16",
    )?;
    let mut rows = statement.query([after])?;
    let mut records = Vec::with_capacity(16);
    while let Some(row) = rows.next()? {
        records.push(row_record(row)?);
    }
    Ok(records)
}
fn same_content(a: &Content, b: &Content) -> bool {
    match (a, b) {
        (Content::Deleted, Content::Deleted) => true,
        (Content::Omitted, Content::Omitted) => true,
        (
            Content::Redacted {
                snapshot: a,
                original: x,
            },
            Content::Redacted {
                snapshot: b,
                original: y,
            },
        ) => a == b && x == y,
        (
            Content::HistoryLink {
                record: a,
                author: b,
                message: c,
            },
            Content::HistoryLink {
                record: x,
                author: y,
                message: z,
            },
        ) => a == x && b == y && c == z,
        (Content::Retained(a), Content::Retained(b)) => a == b,
        (Content::File(a), Content::File(b)) => a == b,
        (Content::Rich(a), Content::Rich(b)) => a == b,
        (Content::Conversation(a), Content::Conversation(b)) => a == b,
        (
            Content::Media {
                record: a,
                original: i,
                file: x,
            },
            Content::Media {
                record: b,
                original: j,
                file: y,
            },
        ) => a == b && i == j && x == y,
        _ => false,
    }
}
fn same_metadata(a: &Record, b: &Record) -> bool {
    a.id == b.id
        && a.conversation == b.conversation
        && a.author == b.author
        && a.created_at == b.created_at
        && a.direction == b.direction
}
fn merge(old: &Record, incoming: &Record) -> Result<bool, Error> {
    if !same_metadata(old, incoming) {
        return Err(Error::Conflict);
    }
    if incoming.revision < old.revision {
        return Ok(false);
    }
    if incoming.revision == old.revision {
        if matches!(
            (&old.content, &incoming.content),
            (
                Content::Omitted,
                Content::Deleted | Content::Redacted { .. }
            ) | (Content::Redacted { .. }, Content::Deleted)
        ) {
            return Ok(true);
        }
        if matches!(
            (&old.content, &incoming.content),
            (
                Content::Deleted,
                Content::Omitted | Content::Redacted { .. }
            ) | (Content::Redacted { .. }, Content::Omitted)
        ) {
            return Ok(false);
        }
        return if same_content(&old.content, &incoming.content) {
            Ok(false)
        } else {
            Err(Error::Conflict)
        };
    }
    if matches!(old.content, Content::Deleted) && !matches!(incoming.content, Content::Deleted) {
        return Err(Error::Conflict);
    }
    if matches!(old.content, Content::Omitted)
        && !matches!(
            incoming.content,
            Content::Deleted | Content::Omitted | Content::Redacted { .. }
        )
    {
        return Err(Error::Conflict);
    }
    if matches!(old.content, Content::Redacted { .. })
        && !matches!(
            incoming.content,
            Content::Deleted | Content::Redacted { .. }
        )
    {
        return Err(Error::Conflict);
    }
    if !matches!(
        incoming.content,
        Content::Deleted | Content::Omitted | Content::Redacted { .. }
    ) && std::mem::discriminant(&old.content) != std::mem::discriminant(&incoming.content)
    {
        return Err(Error::Conflict);
    }
    Ok(true)
}
fn queue(db: &Connection, object: &Object) -> Result<(), Error> {
    db.execute(
        "INSERT INTO archive_objects(id,data) VALUES(?1,?2) ON CONFLICT(id) DO NOTHING",
        (object.id().as_slice(), object.bytes()),
    )?;
    Ok(())
}
fn clear_pending(db: &Connection) -> Result<(), Error> {
    db.execute_batch(
        "DELETE FROM archive_objects; DELETE FROM archive_import; DELETE FROM archive_pages;",
    )?;
    Ok(())
}
fn manifest(db: &Connection, state: &State, operation: Operation) -> Result<Manifest, Error> {
    if operation == Operation::Import && missing_ancestor(db, state)?.is_some() {
        return Err(Error::Unprepared);
    }
    let head = pending(state, operation)?;
    let bytes: Vec<u8> = db.query_row(
        "SELECT data FROM archive_objects WHERE id=?1 AND length(data)<=?2",
        (head.manifest.as_slice(), MAX_OBJECT_LEN as i64),
        |r| r.get(0),
    )?;
    Ok(state.key.open_manifest(&head, &bytes)?)
}

fn missing_ancestor(db: &Connection, state: &State) -> Result<Option<Head>, Error> {
    let mut cursor = pending(state, Operation::Import)?;
    let Some(anchor) = state.status.anchor else {
        return Ok(None);
    };
    if cursor.generation < anchor.generation || cursor.generation - anchor.generation > 64 {
        return Err(Error::Conflict);
    }
    while cursor.generation > anchor.generation {
        let bytes: Option<Vec<u8>> = db
            .query_row(
                "SELECT data FROM archive_objects WHERE id=?1 AND length(data)<=?2",
                (cursor.manifest.as_slice(), MAX_OBJECT_LEN as i64),
                |r| r.get(0),
            )
            .optional()?;
        let Some(bytes) = bytes else {
            return Ok(Some(cursor));
        };
        let checkpoint = state.key.open_manifest(&cursor, &bytes)?;
        cursor = Head {
            generation: cursor.generation - 1,
            manifest: checkpoint.previous.ok_or(Error::InvalidStore)?,
        };
    }
    if cursor != anchor {
        return Err(Error::Conflict);
    }
    Ok(None)
}

// Restore may have lost the manifests needed by another anchored client.
fn queue_checkpoint_proofs(db: &Connection, state: &State) -> Result<(), Error> {
    if !state.reconcile_restored {
        return Ok(());
    }
    let mut cursor = state.status.anchor.ok_or(Error::Conflict)?;
    for _ in 0..64 {
        let bytes: Option<Vec<u8>> = db.query_row("SELECT data FROM archive_checkpoints WHERE generation=?1 AND manifest=?2 AND length(data)<=?3", (cursor.generation as i64, cursor.manifest.as_slice(), MAX_OBJECT_LEN as i64), |r| r.get(0)).optional()?;
        let Some(bytes) = bytes else {
            break;
        };
        let checkpoint = state.key.open_manifest(&cursor, &bytes)?;
        queue(db, &Object::from_bytes(bytes)?)?;
        let Some(previous) = checkpoint.previous else {
            break;
        };
        cursor = Head {
            generation: cursor.generation - 1,
            manifest: previous,
        };
    }
    Ok(())
}

// Follow authenticated references rather than copying arbitrary staging rows.
// Every transferred object starts unacknowledged on the replacement transport.
fn copy_upload(source: &Connection, target: &Connection, state: &State) -> Result<(), Error> {
    let head = pending(state, Operation::Upload)?;
    let snapshot = manifest(source, state, Operation::Upload)?;
    if !state
        .status
        .anchor
        .is_some_and(|anchor| snapshot.continues(&anchor))
    {
        return Err(Error::Conflict);
    }
    let object = |id: Id| -> Result<Object, Error> {
        let bytes: Vec<u8> = source.query_row(
            "SELECT data FROM archive_objects WHERE id=?1 AND length(data)<=?2",
            (id.as_slice(), MAX_OBJECT_LEN as i64),
            |r| r.get(0),
        )?;
        let object = Object::from_bytes(bytes)?;
        if object.id() != id {
            return Err(Error::InvalidStore);
        }
        Ok(object)
    };
    queue(target, &object(head.manifest)?)?;
    let mut count = 1;
    for page in &snapshot.pages {
        let bytes = object(page.object)?;
        let references = state.key.open_page(page, bytes.bytes())?;
        queue(target, &bytes)?;
        count += 1;
        for reference in references {
            let bytes = object(reference.object)?;
            state.key.open_record(&reference, bytes.bytes())?;
            queue(target, &bytes)?;
            count += 1;
        }
    }
    let mut cursor = state.status.anchor.ok_or(Error::Conflict)?;
    for _ in 0..64 {
        let present: bool = source.query_row(
            "SELECT EXISTS(SELECT 1 FROM archive_objects WHERE id=?1)",
            [cursor.manifest.as_slice()],
            |r| r.get(0),
        )?;
        if !present {
            break;
        }
        let bytes = object(cursor.manifest)?;
        let checkpoint = state.key.open_manifest(&cursor, bytes.bytes())?;
        queue(target, &bytes)?;
        count += 1;
        let Some(previous) = checkpoint.previous else {
            break;
        };
        cursor = Head {
            generation: cursor.generation - 1,
            manifest: previous,
        };
    }
    for db in [source, target] {
        if db.query_row("SELECT count(*) FROM archive_objects", [], |r| {
            r.get::<_, i64>(0)
        })? != count
        {
            return Err(Error::InvalidStore);
        }
    }
    Ok(())
}

fn remember_checkpoint(db: &Connection, state: &State, operation: Operation) -> Result<(), Error> {
    let head = pending(state, operation)?;
    manifest(db, state, operation)?;
    db.execute("INSERT INTO archive_checkpoints SELECT ?1,id,data FROM archive_objects WHERE id=?2 ON CONFLICT(generation) DO UPDATE SET manifest=excluded.manifest,data=excluded.data", (head.generation as i64, head.manifest.as_slice()))?;
    if operation == Operation::Import {
        let mut cursor = head;
        for _ in 0..64 {
            let bytes: Option<Vec<u8>> = db
                .query_row(
                    "SELECT data FROM archive_objects WHERE id=?1 AND length(data)<=?2",
                    (cursor.manifest.as_slice(), MAX_OBJECT_LEN as i64),
                    |r| r.get(0),
                )
                .optional()?;
            let Some(bytes) = bytes else {
                break;
            };
            let checkpoint = state.key.open_manifest(&cursor, &bytes)?;
            db.execute("INSERT INTO archive_checkpoints VALUES(?1,?2,?3) ON CONFLICT(generation) DO UPDATE SET manifest=excluded.manifest,data=excluded.data", (cursor.generation as i64, cursor.manifest.as_slice(), bytes))?;
            let Some(previous) = checkpoint.previous else {
                break;
            };
            cursor = Head {
                generation: cursor.generation - 1,
                manifest: previous,
            };
        }
    }
    db.execute(
        "DELETE FROM archive_checkpoints WHERE generation < ?1 OR generation > ?2",
        (
            head.generation.saturating_sub(63) as i64,
            head.generation as i64,
        ),
    )?;
    Ok(())
}

fn ancestor(db: &Connection, state: &State, target: Head) -> Result<bool, Error> {
    let mut cursor = state.status.anchor.ok_or(Error::Conflict)?;
    if target.generation >= cursor.generation || cursor.generation - target.generation > 64 {
        return Ok(false);
    }
    while cursor.generation > target.generation {
        let bytes: Option<Vec<u8>> = db.query_row("SELECT data FROM archive_checkpoints WHERE generation=?1 AND manifest=?2 AND length(data)<=?3", (cursor.generation as i64, cursor.manifest.as_slice(), MAX_OBJECT_LEN as i64), |r| r.get(0)).optional()?;
        let Some(bytes) = bytes else {
            return Ok(false);
        };
        let checkpoint = state.key.open_manifest(&cursor, &bytes)?;
        cursor = Head {
            generation: cursor.generation - 1,
            manifest: checkpoint.previous.ok_or(Error::InvalidStore)?,
        };
    }
    Ok(cursor == target)
}

fn retain(
    tx: &Transaction<'_>,
    wrapping: &StorageKey,
    state: &State,
    record: &Record,
) -> Result<Reference, Error> {
    retain_inner(tx, wrapping, state, record, true)
}
fn retain_inner(
    tx: &Transaction<'_>,
    wrapping: &StorageKey,
    state: &State,
    record: &Record,
    propagate: bool,
) -> Result<Reference, Error> {
    if matches!(state.status.pending, Some((Operation::Import, _))) {
        return Err(Error::Conflict);
    }
    if let Some((reference, bytes)) = existing(tx, record.id)? {
        let old = state.key.open_record(&reference, &bytes)?;
        if record.revision < old.revision {
            return Err(Error::Conflict);
        }
        if !merge(&old, record)? {
            return Ok(reference);
        }
        if old.revision.checked_add(1) != Some(record.revision) {
            return Err(Error::Conflict);
        }
    } else {
        if record.revision != 1 {
            return Err(Error::Conflict);
        }
        if tx.query_row("SELECT count(*) FROM archive_records", [], |r| {
            r.get::<_, i64>(0)
        })? >= MAX_RECORDS as i64
        {
            return Err(Error::Limit);
        }
    }
    let (reference, object) = state.key.seal_record(record)?;
    preserve_local(tx, wrapping, state, record, reference.object)?;
    work::dirty(tx, wrapping, &state.scope)?;
    tx.execute("INSERT INTO archive_records VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,object=excluded.object,data=excluded.data", (reference.id.as_slice(), reference.revision as i64, reference.object.as_slice(), object.bytes()))?;
    media::index_media(tx, wrapping, record)?;
    crate::structured::archive(tx, wrapping, state.scope, record)?;
    crate::conversations::restore(tx, wrapping, record)?;
    if propagate {
        propagate_media(tx, wrapping, state, record)?;
    }
    Ok(reference)
}
fn propagate_media(
    tx: &Transaction<'_>,
    key: &StorageKey,
    state: &State,
    parent: &Record,
) -> Result<bool, Error> {
    if !matches!(
        parent.content,
        Content::Deleted | Content::Omitted | Content::Redacted { .. }
    ) {
        return Ok(false);
    }
    let Some((reference, raw)) = existing(tx, copy_id(parent.id))? else {
        return Ok(false);
    };
    let mut copy = state.key.open_record(&reference, &raw)?;
    if matches!(copy.content, Content::Deleted)
        || (matches!(copy.content, Content::Omitted) && matches!(parent.content, Content::Omitted))
    {
        return Ok(false);
    }
    if copy.conversation != parent.conversation
        || copy.author != parent.author
        || copy.created_at != parent.created_at
        || copy.direction != parent.direction
    {
        return Err(Error::InvalidStore);
    }
    if !matches!(copy.content, Content::Omitted)
        && !matches!(copy.content,Content::Media { record, .. } if record==parent.id)
    {
        return Err(Error::InvalidStore);
    }
    copy.revision = copy.revision.checked_add(1).ok_or(Error::Limit)?;
    copy.content = if matches!(parent.content, Content::Deleted | Content::Redacted { .. }) {
        Content::Deleted
    } else {
        Content::Omitted
    };
    retain_inner(tx, key, state, &copy, false)?;
    Ok(true)
}
impl ClientStore {
    pub(crate) fn recovery_scope(&self) -> Result<Id, Error> {
        Ok(load(&self.db, &self.key)?.scope)
    }
    /// Initialize once with an independently generated recovery secret. The caller
    /// must keep an offline copy; account reauthorization cannot recreate it.
    pub fn configure_recovery(
        &mut self,
        server: &str,
        account: Id,
        secret: Secret32,
    ) -> Result<(), Error> {
        let scope = account_scope(server, account)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.query_row("SELECT EXISTS(SELECT 1 FROM archive)", [], |r| {
            r.get::<_, bool>(0)
        })? {
            return Err(Error::Conflict);
        }
        let state = State {
            scope,
            key: RecoveryKey::from_secret(secret, scope)?,
            status: Status {
                anchor: None,
                pending: None,
            },
            reconcile_restored: false,
            restore_base: None,
        };
        save(&tx, &self.key, &state)?;
        tx.commit()?;
        Ok(())
    }
    pub fn recovery_status(&self) -> Result<Status, Error> {
        Ok(load(&self.db, &self.key)?.status)
    }
    /// Copy only authenticated committed history, recovery key and trusted head
    /// into a freshly enrolled client for the same account. Source is unchanged;
    /// destination publishes the entire handoff atomically under its storage key.
    /// A pending upload transfers its exact authenticated snapshot with all
    /// acknowledgements reset. Pending imports are refused. No live identity,
    /// session, prekey or verification state is copied.
    pub fn copy_recovery_history_to(&mut self, destination: &mut ClientStore) -> Result<(), Error> {
        let scope = destination.connected_account_scope()?;
        let source = self
            .db
            .transaction_with_behavior(TransactionBehavior::Deferred)?;
        let mut state = load(&source, &self.key)?;
        if source.query_row(
            "SELECT EXISTS(SELECT 1 FROM archive_competition)",
            [],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(Error::Conflict);
        }
        if matches!(state.status.pending, Some((Operation::Import, _))) {
            return Err(Error::Conflict);
        }
        if state.scope != scope || state.status.anchor.is_none() {
            return Err(Error::Conflict);
        }
        let target = destination
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if target.query_row(
            "SELECT EXISTS(SELECT 1 FROM archive_competition)",
            [],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(Error::Conflict);
        }
        if target.query_row("SELECT EXISTS(SELECT 1 FROM archive) OR EXISTS(SELECT 1 FROM archive_records) OR EXISTS(SELECT 1 FROM archive_objects) OR EXISTS(SELECT 1 FROM archive_import) OR EXISTS(SELECT 1 FROM archive_pages) OR EXISTS(SELECT 1 FROM identity) OR EXISTS(SELECT 1 FROM sessions) OR EXISTS(SELECT 1 FROM prekeys) OR EXISTS(SELECT 1 FROM inbox) OR EXISTS(SELECT 1 FROM outbox) OR EXISTS(SELECT 1 FROM peers) OR EXISTS(SELECT 1 FROM own_device_binding)", [], |r| r.get::<_, bool>(0))? {
            return Err(Error::Conflict);
        }
        // A new transport identity must authorize repair from its own response.
        state.reconcile_restored = false;
        state.restore_base = None;
        save(&target, &destination.key, &state)?;
        if target.query_row("SELECT count(*) FROM archive_checkpoints", [], |r| {
            r.get::<_, i64>(0)
        })? != 0
        {
            return Err(Error::Conflict);
        }
        let mut checkpoints = source.prepare(
            "SELECT generation,manifest,data FROM archive_checkpoints ORDER BY generation LIMIT 65",
        )?;
        let mut rows = checkpoints.query([])?;
        let mut count = 0;
        while let Some(row) = rows.next()? {
            count += 1;
            if count > 64 {
                return Err(Error::Limit);
            }
            let generation =
                u64::try_from(row.get::<_, i64>(0)?).map_err(|_| Error::InvalidStore)?;
            let id: Vec<u8> = row.get(1)?;
            let bytes = row.get_ref(2)?.as_blob().map_err(|_| Error::InvalidStore)?;
            if bytes.len() > MAX_OBJECT_LEN {
                return Err(Error::Limit);
            }
            let head = Head {
                generation,
                manifest: id.try_into().map_err(|_| Error::InvalidStore)?,
            };
            state.key.open_manifest(&head, bytes)?;
            target.execute(
                "INSERT INTO archive_checkpoints VALUES(?1,?2,?3)",
                (generation as i64, head.manifest.as_slice(), bytes),
            )?;
        }
        drop(rows);
        drop(checkpoints);
        if matches!(state.status.pending, Some((Operation::Upload, _))) {
            copy_upload(&source, &target, &state)?;
        }
        let mut statement =
            source.prepare("SELECT id,revision,object,data FROM archive_records ORDER BY id")?;
        let mut rows = statement.query([])?;
        let mut count = 0;
        while let Some(row) = rows.next()? {
            count += 1;
            if count > MAX_RECORDS {
                return Err(Error::Limit);
            }
            let (reference, bytes) = row_record(row)?;
            let record = state.key.open_record(&reference, &bytes)?;
            target.execute(
                "INSERT INTO archive_records VALUES(?1,?2,?3,?4)",
                (
                    reference.id.as_slice(),
                    reference.revision as i64,
                    reference.object.as_slice(),
                    bytes,
                ),
            )?;
            media::index_media(&target, &destination.key, &record)?;
            crate::structured::archive(&target, &destination.key, state.scope, &record)?;
            crate::conversations::restore(&target, &destination.key, &record)?;
        }
        drop(rows);
        drop(statement);
        source.commit()?;
        target.commit()?;
        Ok(())
    }
    /// Authorize repair only when an authenticated server response identifies
    /// the trusted checkpoint, its exact pending successor, or a proven ancestor
    /// within the bounded manifest ledger. This never imports a rollback or
    /// discards an ambiguous upload. Serialize this with all recovery requests.
    /// A pending direct successor retains its exact ciphertext but loses all
    /// object acknowledgements, since restore may have lost those objects.
    /// If the restored head is that exact successor, its publication is resolved
    /// atomically; a new repair snapshot then includes all current local edits.
    pub fn reconcile_restored_recovery_head(
        &mut self,
        response: &sigil_protocol::recovery::Head,
    ) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut state = load(&tx, &self.key)?;
        let anchor = state.status.anchor.ok_or(Error::Conflict)?;
        if !response.restored_checkpoint {
            return Err(Error::Conflict);
        }
        let mut unflagged = response.clone();
        unflagged.restored_checkpoint = false;
        let restored = server_head(&unflagged)?;
        let older = ancestor(&tx, &state, restored)?;
        let matches = |head: Head| {
            response.generation == head.generation
                && response.manifest.as_deref() == Some(&super::transport::hex(&head.manifest))
        };
        match state.status.pending {
            Some((Operation::Import, _)) => return Err(Error::Conflict),
            Some((Operation::Upload, intended)) => {
                if !manifest(&tx, &state, Operation::Upload)?.continues(&anchor) {
                    return Err(Error::Conflict);
                }
                if matches(intended) {
                    // Exact locally authenticated publication, not a competing
                    // server-selected successor. Retained edits are untouched.
                    remember_checkpoint(&tx, &state, Operation::Upload)?;
                    state.status.anchor = Some(intended);
                    state.status.pending = None;
                    clear_pending(&tx)?;
                } else if matches(anchor) || older {
                    tx.execute("UPDATE archive_objects SET uploaded=0", [])?;
                } else {
                    return Err(Error::Conflict);
                }
            }
            None if matches(anchor) || older => {}
            None => return Err(Error::Conflict),
        }
        state.reconcile_restored = true;
        work::dirty(&tx, &self.key, &state.scope)?;
        state.restore_base = older.then_some(restored);
        if matches!(state.status.pending, Some((Operation::Upload, _))) {
            queue_checkpoint_proofs(&tx, &state)?;
        }
        save(&tx, &self.key, &state)?;
        tx.commit()?;
        Ok(())
    }
    pub fn recovery_record(&self, id: Id) -> Result<Record, Error> {
        let state = load(&self.db, &self.key)?;
        let (reference, bytes) = existing(&self.db, id)?.ok_or(Error::NotFound)?;
        Ok(state.key.open_record(&reference, &bytes)?)
    }
    /// Includes locally retained content omitted from remote recovery by policy.
    pub fn retained_history_record(&self, id: Id) -> Result<Record, Error> {
        local_record(&self.db, &self.key, id)
    }
    /// Read at most 16 authenticated committed records, including tombstones,
    /// in stable ID order. Staged imports are never exposed. Continue with the
    /// last returned ID; restart from None after archive changes. This is a
    /// bounded scan, not timestamp ordering or an incremental change feed.
    pub fn recovery_records(&self, after: Option<Id>) -> Result<Vec<Record>, Error> {
        let state = load(&self.db, &self.key)?;
        let mut statement = self.db.prepare(
            "SELECT id,revision,object,data FROM archive_records WHERE id>?1 ORDER BY id LIMIT 16",
        )?;
        // An empty BLOB precedes every valid 32-byte ID, including all zeroes,
        // and preserves the primary-key range scan on every page.
        let mut rows = statement.query([after.as_ref().map_or(&[][..], |id| id.as_slice())])?;
        let mut records = Vec::with_capacity(16);
        while let Some(row) = rows.next()? {
            let (reference, bytes) = row_record(row)?;
            records.push(state.key.open_record(&reference, &bytes)?);
        }
        Ok(records)
    }
    /// Explicit retained-history classification is required by the caller. IDs
    /// remain stable through revisions; deletion is permanent for that entry ID.
    pub fn retain_recovery_record(&mut self, record: &Record) -> Result<Reference, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = load(&tx, &self.key)?;
        let reference = retain(&tx, &self.key, &state, record)?;
        tx.commit()?;
        Ok(reference)
    }

    /// Freeze a complete snapshot and its exact randomized ciphertext in one
    /// transaction. Repeating this call while uploading resumes the same bytes.
    pub fn prepare_recovery_upload(&mut self, created_at: u64) -> Result<Head, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut state = load(&tx, &self.key)?;
        if let Some((Operation::Upload, head)) = state.status.pending {
            return Ok(head);
        }
        idle(&state)?;
        clear_pending(&tx)?;
        queue_checkpoint_proofs(&tx, &state)?;
        let policy = crate::conversations::recovery_policy(&tx, &self.key)?;
        let mut pages = Vec::new();
        let mut references = Vec::with_capacity(MAX_PAGE_RECORDS);
        let mut after = Vec::new();
        let mut count = 0;
        loop {
            let records = record_page(&tx, &after)?;
            if records.is_empty() {
                break;
            }
            for (reference, _) in records {
                let (mut reference, mut bytes) =
                    existing(&tx, reference.id)?.ok_or(Error::InvalidStore)?;
                count += 1;
                if count > MAX_RECORDS {
                    return Err(Error::Limit);
                }
                after = reference.id.to_vec();
                let mut record = state.key.open_record(&reference, &bytes)?;
                if apply_tombstone(&tx, &self.key, &mut record, created_at)?
                    || work::expire(&tx, &self.key, &mut record, created_at)?
                    || work::expire_history(&mut record, policy, created_at)?
                {
                    reference = retain(&tx, &self.key, &state, &record)?;
                    bytes = existing(&tx, record.id)?.ok_or(Error::InvalidStore)?.1;
                }
                queue(&tx, &Object::from_bytes(bytes)?)?;
                references.push(reference);
                if references.len() == MAX_PAGE_RECORDS {
                    let (page, object) = state.key.seal_page(&references)?;
                    pages.push(page);
                    queue(&tx, &object)?;
                    references.clear();
                }
            }
        }
        if !references.is_empty() {
            let (page, object) = state.key.seal_page(&references)?;
            pages.push(page);
            queue(&tx, &object)?;
        }
        let (head, object) =
            state
                .key
                .seal_manifest(state.status.anchor.as_ref(), created_at, &pages)?;
        queue(&tx, &object)?;
        state.status.pending = Some((Operation::Upload, head));
        work::snapshot(&tx, &self.key, &state.scope)?;
        save(&tx, &self.key, &state)?;
        tx.commit()?;
        Ok(head)
    }
    /// Bounded batches; object acknowledgements require authenticated server responses.
    pub fn pending_recovery_objects(&self) -> Result<Vec<Object>, Error> {
        let state = load(&self.db, &self.key)?;
        pending(&state, Operation::Upload)?;
        let mut statement = self.db.prepare(
            "SELECT id,data FROM archive_objects WHERE uploaded=0 ORDER BY rowid LIMIT 16",
        )?;
        let mut rows = statement.query([])?;
        let mut objects = Vec::new();
        while let Some(row) = rows.next()? {
            let bytes = row.get_ref(1)?.as_blob().map_err(|_| Error::InvalidStore)?;
            if bytes.len() > MAX_OBJECT_LEN {
                return Err(Error::InvalidStore);
            }
            let object = Object::from_bytes(bytes.to_vec())?;
            if object.id().as_slice()
                != row.get_ref(0)?.as_blob().map_err(|_| Error::InvalidStore)?
            {
                return Err(Error::InvalidStore);
            }
            objects.push(object);
        }
        Ok(objects)
    }
    pub fn acknowledge_recovery_object(&mut self, head: Head, id: Id) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = load(&tx, &self.key)?;
        if pending(&state, Operation::Upload)? != head {
            return Err(Error::Conflict);
        }
        if tx.execute(
            "UPDATE archive_objects SET uploaded=1 WHERE id=?1",
            [id.as_slice()],
        )? != 1
        {
            return Err(Error::NotFound);
        }
        tx.commit()?;
        Ok(())
    }
    pub fn recovery_publication(&self) -> Result<sigil_protocol::recovery::PublishHead, Error> {
        let state = load(&self.db, &self.key)?;
        let head = pending(&state, Operation::Upload)?;
        if self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM archive_objects WHERE uploaded=0)",
            [],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(Error::Unprepared);
        }
        manifest(&self.db, &state, Operation::Upload)?;
        Ok(sigil_protocol::recovery::PublishHead {
            expected_generation: state
                .restore_base
                .or(state.status.anchor)
                .map_or(0, |h| h.generation),
            expected_manifest: state
                .restore_base
                .or(state.status.anchor)
                .map(|h| super::transport::hex(&h.manifest)),
            manifest: super::transport::hex(&head.manifest),
            acknowledge_restored_checkpoint: state.reconcile_restored,
            restore_generation: state.restore_base.map(|_| head.generation),
        })
    }
    /// Commit only a head received over authenticated transport after publication.
    pub fn acknowledge_recovery_head(
        &mut self,
        response: &sigil_protocol::recovery::Head,
    ) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut state = load(&tx, &self.key)?;
        let head = match state.status.pending {
            Some((Operation::Upload, head)) => head,
            None => state.status.anchor.ok_or(Error::Unprepared)?,
            _ => return Err(Error::Conflict),
        };
        if response.generation != head.generation
            || response.manifest.as_deref() != Some(&super::transport::hex(&head.manifest))
            || response.restored_checkpoint
        {
            return Err(Error::Conflict);
        }
        if state.status.pending.is_some() {
            if tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM archive_objects WHERE uploaded=0)",
                [],
                |r| r.get::<_, bool>(0),
            )? {
                return Err(Error::Unprepared);
            }
            manifest(&tx, &state, Operation::Upload)?;
            lifecycle::protect(&tx, &state, Operation::Upload)?;
            remember_checkpoint(&tx, &state, Operation::Upload)?;
            cleanup::checkpoint(&tx, &self.key, &state, Operation::Upload)?;
            state.status.anchor = Some(head);
            state.status.pending = None;
            state.reconcile_restored = false;
            state.restore_base = None;
            save(&tx, &self.key, &state)?;
            clear_pending(&tx)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// A fresh device must explicitly accept the lack of an independent rollback
    /// anchor. An existing anchor permits itself, its direct successor, or
    /// staging a chain of at most 64 links; history stays hidden until verified.
    /// A verified competing successor replaces upload staging, preserving local
    /// edits for the next snapshot. Requests must be serialized by the caller.
    pub fn begin_recovery_import(
        &mut self,
        response: &sigil_protocol::recovery::Head,
        object: &Object,
        accept_unanchored: bool,
    ) -> Result<(), Error> {
        let head = server_head(response)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut state = load(&tx, &self.key)?;
        let manifest = state.key.open_manifest(&head, object.bytes())?;
        // A concurrent repair may have cleared the server restore flag by
        // publishing a different direct successor. Its authenticated predecessor
        // resolves that race; the local archive still merges monotonically.
        if state.reconcile_restored
            && !state
                .status
                .anchor
                .is_some_and(|anchor| manifest.continues(&anchor))
        {
            return Err(Error::Conflict);
        }
        if state.status.pending == Some((Operation::Import, head)) {
            return Ok(());
        }
        let competing = match state.status.pending {
            Some((Operation::Upload, intended))
                if intended.generation == head.generation && intended.manifest != head.manifest =>
            {
                true
            }
            None => false,
            _ => return Err(Error::Conflict),
        };
        match state.status.anchor {
            Some(anchor) if head == anchor || manifest.continues(&anchor) => {}
            Some(anchor)
                if !competing
                    && head.generation > anchor.generation + 1
                    && head.generation - anchor.generation <= 64 => {}
            None if accept_unanchored || (competing && head.generation == 1) => {}
            _ => return Err(Error::Conflict),
        }
        clear_pending(&tx)?;
        queue(&tx, object)?;
        state.reconcile_restored = false;
        state.restore_base = None;
        state.status.pending = Some((Operation::Import, head));
        work::dirty(&tx, &self.key, &state.scope)?;
        save(&tx, &self.key, &state)?;
        tx.commit()?;
        Ok(())
    }
    pub fn missing_recovery_pages(&self) -> Result<Vec<usize>, Error> {
        let state = load(&self.db, &self.key)?;
        let manifest = manifest(&self.db, &state, Operation::Import)?;
        let mut missing = Vec::new();
        for (index, page) in manifest.pages.iter().enumerate() {
            let present: Option<Vec<u8>> = self
                .db
                .query_row(
                    "SELECT object FROM archive_pages WHERE position=?1",
                    [index as i64],
                    |r| r.get(0),
                )
                .optional()?;
            match present {
                None => missing.push(index),
                Some(id) if id == page.object => {}
                _ => return Err(Error::InvalidStore),
            }
        }
        Ok(missing)
    }
    /// Stage only the next required ancestry manifest. Records and the anchor
    /// remain untouched until the complete chain and snapshot authenticate.
    pub fn stage_recovery_manifest(&mut self, object: &Object) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = load(&tx, &self.key)?;
        let expected = missing_ancestor(&tx, &state)?.ok_or(Error::Unprepared)?;
        state.key.open_manifest(&expected, object.bytes())?;
        queue(&tx, object)?;
        // Reject a completed chain that terminates at a different anchor.
        missing_ancestor(&tx, &state)?;
        tx.commit()?;
        Ok(())
    }
    /// Discard only uncommitted import staging. Published anchors and retained
    /// records survive cancellation; upload ambiguity cannot be cancelled here.
    pub fn cancel_recovery_import(&mut self, head: Head) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut state = load(&tx, &self.key)?;
        if pending(&state, Operation::Import)? != head {
            return Err(Error::Conflict);
        }
        state.status.pending = None;
        save(&tx, &self.key, &state)?;
        clear_pending(&tx)?;
        tx.commit()?;
        Ok(())
    }
    /// Verify one complete bounded page before staging it. No recovered records
    /// become visible until every page validates and finish_recovery_import commits.
    pub fn import_recovery_page(
        &mut self,
        index: usize,
        page: &Object,
        records: &[Object],
    ) -> Result<(), Error> {
        if records.len() > MAX_PAGE_RECORDS {
            return Err(Error::Limit);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = load(&tx, &self.key)?;
        let manifest = manifest(&tx, &state, Operation::Import)?;
        let expected = manifest.pages.get(index).ok_or(Error::Conflict)?;
        let references = state.key.open_page(expected, page.bytes())?;
        if references.len() != records.len() {
            return Err(Error::Conflict);
        }
        for (reference, object) in references.iter().zip(records) {
            state.key.open_record(reference, object.bytes())?;
            tx.execute("INSERT INTO archive_import VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,object=excluded.object,data=excluded.data", (reference.id.as_slice(), reference.revision as i64, reference.object.as_slice(), object.bytes()))?;
        }
        tx.execute("INSERT INTO archive_pages VALUES(?1,?2,?3,1) ON CONFLICT(position) DO UPDATE SET object=excluded.object,data=excluded.data,complete=1", (index as i64, expected.object.as_slice(), page.bytes()))?;
        tx.commit()?;
        Ok(())
    }
    /// Select at most 16 missing records, retaining completed page progress.
    pub fn next_recovery_download(&mut self) -> Result<Download, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = load(&tx, &self.key)?;
        if let Some(head) = missing_ancestor(&tx, &state)? {
            tx.commit()?;
            return Ok(Download::Manifest { head });
        }
        let manifest = manifest(&tx, &state, Operation::Import)?;
        for (index, page) in manifest.pages.iter().enumerate() {
            let stored: Option<(Vec<u8>, bool)> = tx
                .query_row(
                    "SELECT object,complete FROM archive_pages WHERE position=?1",
                    [index as i64],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let Some((id, complete)) = stored else {
                tx.commit()?;
                return Ok(Download::Page {
                    index,
                    object: page.object,
                });
            };
            if id != page.object {
                return Err(Error::InvalidStore);
            }
            if complete {
                continue;
            }
            let bytes: Vec<u8> = tx.query_row(
                "SELECT data FROM archive_pages WHERE position=?1 AND length(data)<=?2",
                (index as i64, MAX_OBJECT_LEN as i64),
                |r| r.get(0),
            )?;
            let references = state.key.open_page(page, &bytes)?;
            let mut missing = Vec::new();
            for reference in references {
                let present: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM archive_import WHERE id=?1 AND object=?2 AND revision=?3)", (reference.id.as_slice(), reference.object.as_slice(), reference.revision as i64), |r| r.get(0))?;
                if !present {
                    missing.push(reference);
                }
                if missing.len() == 16 {
                    break;
                }
            }
            if !missing.is_empty() {
                tx.commit()?;
                return Ok(Download::Records {
                    index,
                    records: missing,
                });
            }
            tx.execute(
                "UPDATE archive_pages SET complete=1 WHERE position=?1",
                [index as i64],
            )?;
        }
        tx.commit()?;
        Ok(Download::Complete)
    }
    pub fn stage_recovery_page(&mut self, index: usize, page: &Object) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = load(&tx, &self.key)?;
        let manifest = manifest(&tx, &state, Operation::Import)?;
        let expected = manifest.pages.get(index).ok_or(Error::Conflict)?;
        state.key.open_page(expected, page.bytes())?;
        tx.execute("INSERT INTO archive_pages VALUES(?1,?2,?3,0) ON CONFLICT(position) DO UPDATE SET object=excluded.object,data=excluded.data,complete=0", (index as i64, expected.object.as_slice(), page.bytes()))?;
        tx.commit()?;
        Ok(())
    }
    pub fn stage_recovery_record(
        &mut self,
        index: usize,
        id: Id,
        object: &Object,
    ) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = load(&tx, &self.key)?;
        let manifest = manifest(&tx, &state, Operation::Import)?;
        let page = manifest.pages.get(index).ok_or(Error::Conflict)?;
        let bytes: Vec<u8> = tx
            .query_row(
                "SELECT data FROM archive_pages WHERE position=?1 AND length(data)<=?2",
                (index as i64, MAX_OBJECT_LEN as i64),
                |r| r.get(0),
            )
            .optional()?
            .ok_or(Error::Unprepared)?;
        let references = state.key.open_page(page, &bytes)?;
        let reference = references
            .iter()
            .find(|r| r.id == id)
            .ok_or(Error::Conflict)?;
        state.key.open_record(reference, object.bytes())?;
        tx.execute("INSERT INTO archive_import VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,object=excluded.object,data=excluded.data", (reference.id.as_slice(), reference.revision as i64, reference.object.as_slice(), object.bytes()))?;
        tx.execute(
            "UPDATE archive_pages SET complete=0 WHERE position=?1",
            [index as i64],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn finish_recovery_import(&mut self) -> Result<Head, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut state = load(&tx, &self.key)?;
        let head = pending(&state, Operation::Import)?;
        let manifest = manifest(&tx, &state, Operation::Import)?;
        let mut count = 0;
        let mut needs_upload = false;
        for (index, page) in manifest.pages.iter().enumerate() {
            let bytes: Vec<u8> = tx.query_row("SELECT data FROM archive_pages WHERE position=?1 AND object=?2 AND length(data)<=?3", (index as i64, page.object.as_slice(), MAX_OBJECT_LEN as i64), |r| r.get(0)).optional()?.ok_or(Error::Unprepared)?;
            let references = state.key.open_page(page, &bytes)?;
            count += references.len();
            for reference in references {
                let bytes: Vec<u8> = tx
                    .query_row(
                        "SELECT data FROM archive_import WHERE id=?1 AND length(data)<=?2",
                        (reference.id.as_slice(), MAX_OBJECT_LEN as i64),
                        |r| r.get(0),
                    )
                    .optional()?
                    .ok_or(Error::Unprepared)?;
                let incoming = state.key.open_record(&reference, &bytes)?;
                if let Some((old_reference, old_bytes)) = existing(&tx, reference.id)? {
                    let old = state.key.open_record(&old_reference, &old_bytes)?;
                    if !merge(&old, &incoming)? {
                        needs_upload |= old.revision > incoming.revision
                            || !same_content(&old.content, &incoming.content);
                        continue;
                    }
                }
                preserve_local(&tx, &self.key, &state, &incoming, reference.object)?;
                tx.execute("INSERT INTO archive_records VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,object=excluded.object,data=excluded.data", (reference.id.as_slice(), reference.revision as i64, reference.object.as_slice(), bytes))?;
                media::index_media(&tx, &self.key, &incoming)?;
                crate::structured::archive(&tx, &self.key, state.scope, &incoming)?;
                crate::conversations::restore(&tx, &self.key, &incoming)?;
            }
        }
        if tx.query_row("SELECT count(*) FROM archive_import", [], |r| {
            r.get::<_, i64>(0)
        })? != count as i64
        {
            return Err(Error::InvalidStore);
        }
        let retained_count = tx.query_row("SELECT count(*) FROM archive_records", [], |r| {
            r.get::<_, i64>(0)
        })?;
        if retained_count > MAX_RECORDS as i64 {
            return Err(Error::Limit);
        }
        needs_upload |= retained_count != count as i64;
        state.status.pending = None;
        let mut after = Vec::new();
        loop {
            let records = record_page(&tx, &after)?;
            if records.is_empty() {
                break;
            }
            for (reference, _) in records {
                after = reference.id.to_vec();
                let (reference, raw) = existing(&tx, reference.id)?.ok_or(Error::InvalidStore)?;
                let record = state.key.open_record(&reference, &raw)?;
                needs_upload |= propagate_media(&tx, &self.key, &state, &record)?;
                let mut normalized = record;
                if apply_tombstone(&tx, &self.key, &mut normalized, crate::conversations::now())? {
                    retain(&tx, &self.key, &state, &normalized)?;
                    needs_upload = true;
                }
                let record = normalized;
                if let Content::HistoryLink {
                    record: legacy,
                    author,
                    message,
                } = record.content
                {
                    let target: Id = Sha256::digest(
                        [
                            b"Sigil/conversation-history/v0".as_slice(),
                            &record.conversation,
                            &author,
                            &message,
                        ]
                        .concat(),
                    )
                    .into();
                    let target = read_record(&tx, &self.key, target)?;
                    if target.conversation != record.conversation
                        || target.author != record.author
                        || target.created_at != record.created_at
                    {
                        return Err(Error::InvalidStore);
                    }
                    if matches!(target.content, Content::Deleted | Content::Redacted { .. }) {
                        if let Some((reference, raw)) = existing(&tx, legacy)? {
                            let mut old = state.key.open_record(&reference, &raw)?;
                            if !matches!(old.content, Content::Deleted) {
                                old.revision = old.revision.checked_add(1).ok_or(Error::Limit)?;
                                old.content = Content::Deleted;
                                retain(&tx, &self.key, &state, &old)?;
                                needs_upload = true;
                            }
                        }
                    }
                }
            }
        }
        state.status.pending = Some((Operation::Import, head));
        lifecycle::protect(&tx, &state, Operation::Import)?;
        remember_checkpoint(&tx, &state, Operation::Import)?;
        cleanup::checkpoint(&tx, &self.key, &state, Operation::Import)?;
        state.status.anchor = Some(head);
        state.status.pending = None;
        work::reconciled(&tx, &self.key, &state.scope, needs_upload)?;
        save(&tx, &self.key, &state)?;
        clear_pending(&tx)?;
        tx.commit()?;
        Ok(head)
    }
}

pub(super) fn configured(db: &Connection) -> Result<bool, Error> {
    Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM archive)", [], |r| r.get(0))?)
}
pub(crate) fn read_record(db: &Connection, key: &StorageKey, id: Id) -> Result<Record, Error> {
    let state = load(db, key)?;
    let (reference, raw) = existing(db, id)?.ok_or(Error::NotFound)?;
    Ok(state.key.open_record(&reference, &raw)?)
}
fn apply_tombstone(
    db: &Connection,
    key: &StorageKey,
    record: &mut Record,
    now: u64,
) -> Result<bool, Error> {
    if let Some(mut content) = crate::conversations::archive_tombstone(db, key, record, now)? {
        if matches!(content, Content::Deleted) {
            content = deletion_content(db, key, record.id)?;
        }
        if std::mem::discriminant(&content) != std::mem::discriminant(&record.content) {
            record.revision = record.revision.checked_add(1).ok_or(Error::Limit)?;
            record.content = content;
            return Ok(true);
        }
    }
    Ok(false)
}
fn deletion_content(db: &Connection, key: &StorageKey, id: Id) -> Result<Content, Error> {
    let local = local_record(db, key, id)?;
    let Content::Conversation(raw) = local.content else {
        return Ok(Content::Deleted);
    };
    use sigil_protocol::conversation::{Action, Body, Snapshot};
    let mut snapshot = Snapshot::from_bytes(&raw).map_err(|_| Error::InvalidStore)?;
    let original = Sha256::digest(
        snapshot
            .operation
            .to_bytes()
            .map_err(|_| Error::InvalidStore)?,
    )
    .into();
    match &mut snapshot.operation.action {
        Action::Post { body, .. } | Action::Edit { body, .. } => *body = Body::Text(" ".into()),
        _ => return Ok(Content::Deleted),
    }
    Ok(Content::Redacted {
        snapshot: Zeroizing::new(snapshot.to_bytes().map_err(|_| Error::InvalidStore)?),
        original,
    })
}
fn preserve_local(
    db: &Connection,
    key: &StorageKey,
    state: &State,
    incoming: &Record,
    object: Id,
) -> Result<(), Error> {
    media_cleanup::queue(db, key, incoming, object)?;
    if matches!(
        incoming.content,
        Content::Deleted | Content::Redacted { .. }
    ) {
        db.execute(
            "DELETE FROM archive_local WHERE id=?1",
            [incoming.id.as_slice()],
        )?;
    } else if matches!(incoming.content, Content::Omitted) {
        if let Some((reference, raw)) = existing(db, incoming.id)? {
            let old = state.key.open_record(&reference, &raw)?;
            if !matches!(
                old.content,
                Content::Deleted | Content::Omitted | Content::Redacted { .. }
            ) {
                let bytes = Zeroizing::new(
                    [
                        reference.revision.to_be_bytes().as_slice(),
                        &reference.object,
                        &raw,
                    ]
                    .concat(),
                );
                db.execute("INSERT INTO archive_local VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET state=excluded.state",
                    (incoming.id.as_slice(), groups::storage_record::seal_record(key, &bytes, &binding(101,&incoming.id,b"retained local history"))?))?;
            }
        }
    }
    Ok(())
}
pub(crate) fn local_record(db: &Connection, key: &StorageKey, id: Id) -> Result<Record, Error> {
    let record = read_record(db, key, id)?;
    if !matches!(record.content, Content::Omitted) {
        return Ok(record);
    }
    let raw: Option<Vec<u8>> = db
        .query_row(
            "SELECT CASE WHEN length(state)<=70000 THEN state END FROM archive_local WHERE id=?1",
            [id.as_slice()],
            |r| r.get(0),
        )
        .optional()?;
    let Some(raw) = raw else {
        return Ok(record);
    };
    let bytes = groups::storage_record::open_record(
        key,
        &raw,
        &binding(101, &id, b"retained local history"),
    )?;
    if bytes.len() < 40 {
        return Err(Error::InvalidStore);
    }
    let reference = Reference {
        id,
        revision: u64::from_be_bytes(bytes[..8].try_into().map_err(|_| Error::InvalidStore)?),
        object: bytes[8..40].try_into().map_err(|_| Error::InvalidStore)?,
    };
    let old = load(db, key)?.key.open_record(&reference, &bytes[40..])?;
    if !same_metadata(&record, &old) || old.revision >= record.revision {
        return Err(Error::InvalidStore);
    }
    Ok(old)
}
pub(crate) fn forget_record(tx: &Transaction<'_>, key: &StorageKey, id: Id) -> Result<(), Error> {
    if !configured(tx)? {
        return Ok(());
    }
    let state = load(tx, key)?;
    let Some((reference, raw)) = existing(tx, id)? else {
        return Ok(());
    };
    let mut record = state.key.open_record(&reference, &raw)?;
    if matches!(record.content, Content::Deleted | Content::Redacted { .. }) {
        return Ok(());
    }
    record.revision = record.revision.checked_add(1).ok_or(Error::Limit)?;
    record.content = deletion_content(tx, key, id)?;
    retain(tx, key, &state, &record)?;
    Ok(())
}
/// Initial capture only. Later archival edits/deletions are never overwritten by
/// a delivery retry, and the archive must belong to the connected account.
pub(super) fn retain_new(
    tx: &Transaction<'_>,
    key: &StorageKey,
    scope: Id,
    record: &Record,
) -> Result<(), Error> {
    let state = load(tx, key)?;
    if state.scope != scope || record.revision != 1 {
        return Err(Error::Conflict);
    }
    if matches!(state.status.pending, Some((Operation::Import, _))) {
        return Err(Error::Conflict);
    }
    if let Some((reference, bytes)) = existing(tx, record.id)? {
        let prior = state.key.open_record(&reference, &bytes)?;
        if !same_metadata(&prior, record) {
            return Err(Error::Conflict);
        }
        if prior.revision > 1 || matches!(prior.content, Content::Deleted) {
            return Ok(());
        }
    }
    retain(tx, key, &state, record)?;
    Ok(())
}

pub(super) fn require_resend(
    db: &Connection,
    key: &StorageKey,
    scope: Id,
    id: Id,
    body: sigil_protocol::event::Content<'_>,
) -> Result<(), Error> {
    if !configured(db)? {
        return Ok(());
    }
    let state = load(db, key)?;
    if state.scope != scope || matches!(state.status.pending, Some((Operation::Import, _))) {
        return Err(Error::Conflict);
    }
    if let Some((reference, bytes)) = existing(db, id)? {
        state.key.open_record(&reference, &bytes)?;
        let record = local_record(db, key, id)?;
        match (record.content, body) {
            (Content::Retained(current), sigil_protocol::event::Content::Text(text))
                if current.as_slice() == text.as_bytes() => {}
            (Content::File(current), sigil_protocol::event::Content::File(bytes))
                if current.as_slice() == bytes => {}
            (Content::Rich(current), sigil_protocol::event::Content::Rich(bytes))
                if current.as_slice() == bytes => {}
            _ => return Err(Error::Obsolete),
        }
    }
    Ok(())
}

pub(crate) fn migrate_structured(tx: &Transaction<'_>, key: &StorageKey) -> Result<(), Error> {
    if !configured(tx)? {
        return Ok(());
    }
    let state = load(tx, key)?;
    let mut statement =
        tx.prepare("SELECT id,revision,object,data FROM archive_records ORDER BY id")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let (reference, bytes) = row_record(row)?;
        let record = state.key.open_record(&reference, &bytes)?;
        crate::structured::archive(tx, key, state.scope, &record)?;
    }
    Ok(())
}

pub(crate) fn record_deleted(
    db: &Connection,
    key: &StorageKey,
    scope: Id,
    id: Id,
) -> Result<bool, Error> {
    if !configured(db)? {
        return Ok(false);
    }
    let state = load(db, key)?;
    if state.scope != scope {
        return Err(Error::Conflict);
    }
    let Some((reference, bytes)) = existing(db, id)? else {
        return Ok(false);
    };
    Ok(matches!(
        state.key.open_record(&reference, &bytes)?.content,
        Content::Deleted | Content::Redacted { .. }
    ))
}
