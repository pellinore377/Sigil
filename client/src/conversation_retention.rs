use super::*;

pub(super) fn link_id(record: Id) -> Id {
    Sha256::digest([b"Sigil/history-link/v0".as_slice(), &record].concat()).into()
}
pub(super) fn link(
    tx: &Transaction<'_>,
    key: &StorageKey,
    record: Id,
    conversation: Id,
    author: Id,
    message: Id,
) -> Result<(), Error> {
    let entry = index(
        key,
        b"Sigil/conversation-event/v0",
        &[&conversation, &author, &message],
    )?;
    let bytes = [conversation.as_slice(), &author, &message].concat();
    let prior:Option<Vec<u8>>=tx.query_row("SELECT CASE WHEN length(state)=132 THEN state END FROM conversation_archive_refs WHERE record=?1",[record.as_slice()],|r|r.get(0)).optional()?;
    if let Some(prior) = prior {
        if key
            .open(&prior, &binding(103, &record, b"history origin"))?
            .as_slice()
            != bytes
        {
            return Err(Error::Conflict);
        }
    } else {
        tx.execute(
            "INSERT INTO conversation_archive_refs VALUES(?1,?2,?3)",
            (
                record.as_slice(),
                entry.as_slice(),
                key.seal(&bytes, &binding(103, &record, b"history origin"))?,
            ),
        )?;
    }
    Ok(())
}
fn legacy_records(db: &Connection, key: &StorageKey, e: &Entry) -> Result<Vec<Id>, Error> {
    let mut query=db.prepare("SELECT record,CASE WHEN length(state)=132 THEN state END FROM conversation_archive_refs WHERE entry=?1")?;
    let mut rows = query.query([entry_id(key, e)?.as_slice()])?;
    let mut result = Vec::new();
    while let Some(row) = rows.next()? {
        let id: Vec<u8> = row.get(0)?;
        let id: Id = id.try_into().map_err(|_| Error::InvalidStore)?;
        let raw: Vec<u8> = row.get(1)?;
        if key
            .open(&raw, &binding(103, &id, b"history origin"))?
            .as_slice()
            != [e.conversation.as_slice(), &e.author, &e.operation.id].concat()
        {
            return Err(Error::InvalidStore);
        }
        result.push(id);
    }
    Ok(result)
}
pub(super) fn backfill_link(
    tx: &Transaction<'_>,
    key: &StorageKey,
    e: &Entry,
) -> Result<(), Error> {
    if !recovery::configured(tx)? || copyable(tx, key, e)? != Some(true) {
        return Ok(());
    }
    let (scope, _) = structured::account_context(tx, key)?;
    for id in legacy_records(tx, key, e)? {
        let old = match recovery::read_record(tx, key, id) {
            Ok(old) => old,
            Err(Error::NotFound) => continue,
            Err(error) => return Err(error),
        };
        if old.conversation != e.conversation
            || old.author != e.identity
            || old.created_at != e.timestamp
        {
            return Err(Error::InvalidStore);
        }
        recovery::retain_new(
            tx,
            key,
            scope,
            &sigil_crypto::recovery::Record {
                id: link_id(id),
                revision: 1,
                conversation: e.conversation,
                author: e.identity,
                created_at: e.timestamp,
                direction: old.direction,
                content: sigil_crypto::recovery::Content::HistoryLink {
                    record: id,
                    author: e.author,
                    message: e.operation.id,
                },
            },
        )?;
    }
    Ok(())
}

pub(crate) fn archive_tombstone(
    db: &Connection,
    key: &StorageKey,
    record: &sigil_crypto::recovery::Record,
    now: u64,
) -> Result<Option<sigil_crypto::recovery::Content>, Error> {
    use sigil_crypto::recovery::Content as Archive;
    if matches!(record.content, Archive::Deleted | Archive::Redacted { .. }) {
        return Ok(None);
    }
    let effective = recovery::local_record(db, key, record.id)?;
    let (conversation, reference) = match &effective.content {
        Archive::Conversation(raw) => {
            let snapshot = sigil_protocol::conversation::Snapshot::from_bytes(raw)
                .map_err(|_| Error::InvalidStore)?;
            if !matches!(
                snapshot.operation.action,
                Action::Post { .. } | Action::Edit { .. }
            ) {
                return Ok(None);
            }
            (
                snapshot.conversation,
                target(
                    &snapshot.operation.action,
                    snapshot.author,
                    snapshot.operation.id,
                ),
            )
        }
        Archive::Retained(_) | Archive::File(_) | Archive::Rich(_) => {
            let raw:Option<Vec<u8>>=db.query_row("SELECT CASE WHEN length(state)=132 THEN state END FROM conversation_archive_refs WHERE record=?1",[record.id.as_slice()],|r|r.get(0)).optional()?;
            let Some(raw) = raw else {
                return Ok(None);
            };
            let raw = key.open(&raw, &binding(103, &record.id, b"history origin"))?;
            if raw.len() != 96 || raw[..32] != record.conversation {
                return Err(Error::InvalidStore);
            }
            (
                record.conversation,
                Reference {
                    author: raw[32..64].try_into().map_err(|_| Error::InvalidStore)?,
                    message: raw[64..].try_into().map_err(|_| Error::InvalidStore)?,
                },
            )
        }
        Archive::Media { record: parent, .. } => {
            let parent = recovery::read_record(db, key, *parent)?;
            return match parent.content {
                Archive::Deleted | Archive::Redacted { .. } => Ok(Some(Archive::Deleted)),
                Archive::Omitted => {
                    if matches!(record.content, Archive::Omitted) {
                        Ok(None)
                    } else {
                        Ok(Some(Archive::Omitted))
                    }
                }
                Archive::Media { .. } | Archive::HistoryLink { .. } => Err(Error::InvalidStore),
                _ => archive_tombstone(db, key, &parent, now),
            };
        }
        _ => return Ok(None),
    };
    let (_, own) = structured::account_context(db, key)?;
    match message(db, key, conversation, reference.clone(), now, own) {
        Ok(value) if value.deleted => return Ok(Some(Archive::Deleted)),
        Ok(_) | Err(Error::NotFound) => (),
        Err(error) => return Err(error),
    }
    let target: Id = Sha256::digest(
        [
            b"Sigil/conversation-history/v0".as_slice(),
            &conversation,
            &reference.author,
            &reference.message,
        ]
        .concat(),
    )
    .into();
    if target != record.id {
        match recovery::read_record(db, key, target) {
            Ok(target) if matches!(target.content, Archive::Deleted | Archive::Redacted { .. }) => {
                return Ok(Some(Archive::Deleted))
            }
            Ok(target)
                if matches!(target.content, Archive::Omitted)
                    && !matches!(record.content, Archive::Omitted) =>
            {
                return Ok(Some(Archive::Omitted))
            }
            Ok(_) | Err(Error::NotFound) => (),
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

fn strip(operation: &mut Operation) -> Result<(), Error> {
    match &mut operation.action {
        Action::Post { body, .. } | Action::Edit { body, .. } => *body = Body::Text(" ".into()),
        _ => return Err(Error::InvalidStore),
    }
    Ok(())
}

pub(super) fn accept_redaction(
    tx: &Transaction<'_>,
    key: &StorageKey,
    mut entry: Entry,
    digest: Id,
) -> Result<(), Error> {
    let id = entry_id(key, &entry)?;
    let reference = Reference {
        author: entry.author,
        message: entry.operation.id,
    };
    if let Some(mut old) = original(tx, key, &entry.conversation, &reference)? {
        if old.identity != entry.identity || old.timestamp != entry.timestamp {
            return Err(Error::Conflict);
        }
        let prior = removed(tx, key, &id)?;
        let original: Id =
            Sha256::digest(old.operation.to_bytes().map_err(|_| Error::InvalidStore)?).into();
        if prior.unwrap_or(original) != digest {
            return Err(Error::Conflict);
        }
        strip(&mut old.operation)?;
        if old.operation != entry.operation {
            return Err(Error::Conflict);
        }
        entry.seen = old.seen;
    } else {
        ingest_only(tx, key, &entry)?;
    }
    tx.execute("INSERT INTO conversation_removed VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET state=excluded.state",
        (id.as_slice(), key.seal(&digest, &binding(99, &id, b"redacted operation"))?))?;
    tx.execute(
        "UPDATE conversation_ops SET state=?2 WHERE id=?1",
        (id.as_slice(), seal(key, &id, &entry)?),
    )?;
    Ok(())
}

pub(super) fn removed(db: &Connection, key: &StorageKey, id: &Id) -> Result<Option<Id>, Error> {
    let raw: Option<Vec<u8>> = db.query_row("SELECT CASE WHEN length(state)=68 THEN state END FROM conversation_removed WHERE id=?1", [id.as_slice()], |r| r.get(0)).optional()?;
    raw.map(|raw| {
        key.open(&raw, &binding(99, id, b"redacted operation"))?
            .as_slice()
            .try_into()
            .map_err(|_| Error::InvalidStore)
    })
    .transpose()
}
#[derive(Default, Debug)]
pub struct HistoryCleanup {
    pub checked: usize,
    pub redacted: usize,
}
impl ClientStore {
    /// Remove obsolete bodies while keeping authenticated operation identities.
    /// At most 16 entries per pass; logical erasure does not promise disk sanitization.
    pub fn maintain_history(&mut self, now: u64) -> Result<HistoryCleanup, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = time_floor(&tx, &self.key, now)?;
        let (_, own) = structured::account_context(&tx, &self.key)?;
        let raw: Option<Vec<u8>> = tx.query_row("SELECT CASE WHEN length(state)=44 THEN state END FROM conversation_cleanup WHERE id=1", [], |r| r.get(0)).optional()?;
        let after = raw
            .map(|v| {
                self.key
                    .open(&v, &binding(100, &[0; 32], b"history cleanup"))?
                    .as_slice()
                    .try_into()
                    .map(i64::from_be_bytes)
                    .map_err(|_| Error::InvalidStore)
            })
            .transpose()?
            .unwrap_or(0);
        if after < 0 {
            return Err(Error::InvalidStore);
        }
        let mut query = tx.prepare("SELECT rowid,id,CASE WHEN length(state)<=574000 THEN state END FROM conversation_ops WHERE rowid>?1 ORDER BY rowid LIMIT 16")?;
        let mut rows = query.query([after])?;
        let mut cursor: i64 = 0;
        let mut result = HistoryCleanup::default();
        while let Some(row) = rows.next()? {
            cursor = row.get(0)?;
            result.checked += 1;
            let id: Vec<u8> = row.get(1)?;
            let id: Id = id.try_into().map_err(|_| Error::InvalidStore)?;
            let mut entry = open(&self.key, &id, &row.get::<_, Vec<u8>>(2)?)?;
            if removed(&tx, &self.key, &id)?.is_some() {
                continue;
            }
            let obsolete = match &entry.operation.action {
                Action::Post { .. } | Action::Edit { .. } => {
                    let reference =
                        target(&entry.operation.action, entry.author, entry.operation.id);
                    match message(&tx, &self.key, entry.conversation, reference, now, own) {
                        Ok(message) => {
                            let obsolete = match &entry.operation.action {
                                Action::Post {
                                    body: Body::Rich(raw),
                                    ..
                                }
                                | Action::Edit {
                                    body: Body::Rich(raw),
                                    ..
                                } => match structured::require_content(
                                    &tx,
                                    &self.key,
                                    entry.conversation,
                                    raw,
                                ) {
                                    Ok(()) => false,
                                    Err(Error::Obsolete) => true,
                                    Err(error) => return Err(error),
                                },
                                _ => false,
                            };
                            message.deleted || obsolete
                        }
                        Err(Error::NotFound) => false,
                        Err(error) => return Err(error),
                    }
                }
                _ => false,
            };
            if !obsolete {
                continue;
            }
            let digest: Id = Sha256::digest(
                entry
                    .operation
                    .to_bytes()
                    .map_err(|_| Error::InvalidStore)?,
            )
            .into();
            recovery::forget_record(&tx, &self.key, history_id(&entry))?;
            structured::forget_sources(&tx, &self.key, history_id(&entry))?;
            for legacy in legacy_records(&tx, &self.key, &entry)? {
                recovery::forget_record(&tx, &self.key, legacy)?;
                structured::forget_sources(&tx, &self.key, legacy)?;
            }
            strip(&mut entry.operation)?;
            accept_redaction(&tx, &self.key, entry, digest)?;
            result.redacted += 1;
        }
        drop(rows);
        drop(query);
        tx.execute("INSERT INTO conversation_cleanup VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET state=excluded.state",
            [self.key.seal(&cursor.to_be_bytes(), &binding(100, &[0;32], b"history cleanup"))?])?;
        tx.commit()?;
        Ok(result)
    }
}
