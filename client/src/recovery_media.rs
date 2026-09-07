//! Keyed lookup of live archive references. This index carries no file keys;
//! every positive lookup reauthenticates the committed recovery record.
use super::*;
fn content_index(key: &StorageKey, bytes: &[u8]) -> Result<Id, Error> {
    Ok(key.commitment(bytes, b"Sigil/archive-media-reference/v0")?)
}
pub(super) fn index_media(db: &Connection, key: &StorageKey, record: &Record) -> Result<(), Error> {
    if let Content::File(bytes) = &record.content {
        db.execute("INSERT INTO archive_media VALUES(?1,?2) ON CONFLICT(record) DO UPDATE SET content=excluded.content",
            (record.id.as_slice(), content_index(key, bytes)?.as_slice()))?;
    } else {
        db.execute(
            "DELETE FROM archive_media WHERE record=?1",
            [record.id.as_slice()],
        )?;
    }
    Ok(())
}
pub(crate) fn migrate_media(db: &Connection, key: &StorageKey) -> Result<(), Error> {
    if !configured(db)? {
        return Ok(());
    }
    let state = load(db, key)?;
    let mut after = Vec::new();
    let mut count = 0;
    loop {
        let records = record_page(db, &after)?;
        if records.is_empty() {
            break;
        }
        for (reference, bytes) in records {
            count += 1;
            if count > MAX_RECORDS {
                return Err(Error::Limit);
            }
            let record = state.key.open_record(&reference, &bytes)?;
            index_media(db, key, &record)?;
            after = reference.id.to_vec();
        }
    }
    Ok(())
}
fn authorized(db: &Connection, key: &StorageKey, scope: Id) -> Result<State, Error> {
    let state = load(db, key)?;
    if state.scope != scope || matches!(state.status.pending, Some((Operation::Import, _))) {
        return Err(Error::Conflict);
    }
    Ok(state)
}
/// None means recovery is disabled or this earlier message was never archived.
/// It does not infer archive authority for older local history.
pub(crate) fn retained_file(
    db: &Connection,
    key: &StorageKey,
    scope: Id,
    id: Id,
) -> Result<Option<Zeroizing<Vec<u8>>>, Error> {
    if !configured(db)? {
        return Ok(None);
    }
    let state = authorized(db, key, scope)?;
    let Some((reference, bytes)) = existing(db, id)? else {
        return Ok(None);
    };
    match state.key.open_record(&reference, &bytes)?.content {
        Content::File(bytes) => Ok(Some(bytes)),
        Content::Deleted => Err(Error::Obsolete),
        _ => Err(Error::InvalidEvent),
    }
}
pub(crate) fn references_file(
    db: &Connection,
    key: &StorageKey,
    scope: Id,
    bytes: &[u8],
) -> Result<bool, Error> {
    let state = authorized(db, key, scope)?;
    let id: Option<Vec<u8>> = db
        .query_row(
            "SELECT record FROM archive_media WHERE content=?1 ORDER BY record LIMIT 1",
            [content_index(key, bytes)?.as_slice()],
            |r| r.get(0),
        )
        .optional()?;
    let Some(id) = id else {
        return Ok(false);
    };
    let id = id.try_into().map_err(|_| Error::InvalidStore)?;
    let (reference, sealed) = existing(db, id)?.ok_or(Error::InvalidStore)?;
    match state.key.open_record(&reference, &sealed)?.content {
        Content::File(current) if current.as_slice() == bytes => Ok(true),
        _ => Err(Error::InvalidStore),
    }
}
