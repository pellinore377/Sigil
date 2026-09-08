//! Keyed lookup of live archive references. This index carries no file keys;
//! every positive lookup reauthenticates the committed recovery record.
use super::*;
pub(super) fn file_bytes(content: &Content) -> Result<Option<Zeroizing<Vec<u8>>>, Error> {
    match content {
        Content::Media { file, .. } => Ok(Some(file.clone())),
        Content::File(v) => Ok(Some(v.clone())),
        Content::Conversation(raw) => {
            use sigil_protocol::conversation::{Action, Body, Snapshot};
            let snapshot = Snapshot::from_bytes(raw).map_err(|_| Error::InvalidStore)?;
            Ok(match snapshot.operation.action {
                Action::Post {
                    body: Body::File(v),
                    ..
                }
                | Action::Edit {
                    body: Body::File(v),
                    ..
                } => Some(Zeroizing::new(v)),
                _ => None,
            })
        }
        _ => Ok(None),
    }
}
fn content_index(key: &StorageKey, bytes: &[u8]) -> Result<Id, Error> {
    Ok(key.commitment(bytes, b"Sigil/archive-media-reference/v0")?)
}
pub(super) fn index_media(db: &Connection, key: &StorageKey, record: &Record) -> Result<(), Error> {
    if matches!(record.content, Content::Omitted) {
        let local = local_record(db, key, record.id)?;
        if !matches!(local.content, Content::Omitted) {
            return index_media(db, key, &local);
        }
    }
    if let Content::Media { record: parent, .. } = &record.content {
        if record.id != copy_id(*parent) {
            return Err(Error::InvalidStore);
        }
    }
    if let Some(bytes) = file_bytes(&record.content)? {
        db.execute("INSERT INTO archive_media VALUES(?1,?2) ON CONFLICT(record) DO UPDATE SET content=excluded.content",
            (record.id.as_slice(), content_index(key, &bytes)?.as_slice()))?;
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
    state.key.open_record(&reference, &bytes)?;
    let record = local_record(db, key, id)?;
    if matches!(
        record.content,
        Content::Deleted | Content::Omitted | Content::Redacted { .. }
    ) || !crate::conversations::archived_file_live(db, key, &record)?
    {
        return Err(Error::Obsolete);
    }
    file_bytes(&record.content)?
        .map(Some)
        .ok_or(Error::InvalidEvent)
}
pub(crate) fn references_file(
    db: &Connection,
    key: &StorageKey,
    scope: Id,
    bytes: &[u8],
) -> Result<bool, Error> {
    let state = authorized(db, key, scope)?;
    let mut stmt =
        db.prepare("SELECT record FROM archive_media WHERE content=?1 ORDER BY record")?;
    let mut rows = stmt.query([content_index(key, bytes)?.as_slice()])?;
    while let Some(row) = rows.next()? {
        let id: Vec<u8> = row.get(0)?;
        let (reference, sealed) = existing(db, id.try_into().map_err(|_| Error::InvalidStore)?)?
            .ok_or(Error::InvalidStore)?;
        state.key.open_record(&reference, &sealed)?;
        let record = local_record(db, key, reference.id)?;
        if file_bytes(&record.content)?.is_none_or(|v| v.as_slice() != bytes) {
            return Err(Error::InvalidStore);
        }
        let live = if let Content::Media { record: parent, .. } = record.content {
            match retained_file(db, key, scope, parent) {
                Ok(value) => value.is_some(),
                Err(Error::Obsolete) => false,
                Err(error) => return Err(error),
            }
        } else {
            crate::conversations::archived_file_live(db, key, &record)?
        };
        if live {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn copy_id(record: Id) -> Id {
    Sha256::digest([b"Sigil/recovery-media/v0".as_slice(), &record].concat()).into()
}
pub(crate) fn recovery_file(
    db: &Connection,
    key: &StorageKey,
    scope: Id,
    parent: Id,
) -> Result<Zeroizing<Vec<u8>>, Error> {
    let original = retained_file(db, key, scope, parent)?.ok_or(Error::NotFound)?;
    let state = authorized(db, key, scope)?;
    let Some((reference, raw)) = existing(db, copy_id(parent))? else {
        return Ok(original);
    };
    state.key.open_record(&reference, &raw)?;
    let copy = local_record(db, key, reference.id)?;
    match copy.content {
        Content::Media {
            record,
            original: digest,
            file,
        } if record == parent => {
            if <Id>::from(Sha256::digest(&original)) != digest {
                return Err(Error::Obsolete);
            }
            let base = sigil_protocol::file::File::from_bytes(&original)
                .map_err(|_| Error::InvalidStore)?;
            let media =
                sigil_protocol::file::File::from_bytes(&file).map_err(|_| Error::InvalidStore)?;
            let (a, _) = sigil_crypto::attachment::FileKey::from_descriptor(base.descriptor)?;
            let (b, _) = sigil_crypto::attachment::FileKey::from_descriptor(media.descriptor)?;
            if base.name != media.name
                || base.media_type != media.media_type
                || base.expires_at != media.expires_at
                || a.shape().length != b.shape().length
            {
                return Err(Error::InvalidStore);
            }
            Ok(file)
        }
        Content::Deleted | Content::Omitted => Ok(original),
        _ => Err(Error::InvalidStore),
    }
}
