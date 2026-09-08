use super::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Removal {
    file: Id,
    server: String,
    object: Id,
}
fn aad(id: &Id) -> Vec<u8> {
    binding(102, id, b"backup media removal")
}
pub(super) fn queue(
    db: &Connection,
    key: &StorageKey,
    incoming: &Record,
    object: Id,
) -> Result<(), Error> {
    if !matches!(incoming.content, Content::Deleted | Content::Omitted) {
        return Ok(());
    }
    let old = match local_record(db, key, incoming.id) {
        Ok(old) => old,
        Err(Error::NotFound) => return Ok(()),
        Err(error) => return Err(error),
    };
    let Content::Media { file, .. } = old.content else {
        return Ok(());
    };
    let file = sigil_protocol::file::File::from_bytes(&file).map_err(|_| Error::InvalidStore)?;
    let (file_key, _) = sigil_crypto::attachment::FileKey::from_descriptor(file.descriptor)?;
    let removal = Removal {
        file: file_key.shape().file,
        server: file.source.into(),
        object,
    };
    db.execute("INSERT INTO archive_media_garbage VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET state=excluded.state",
        (incoming.id.as_slice(),key.seal(&serde_json::to_vec(&removal).map_err(|_|Error::InvalidStore)?,&aad(&incoming.id))?))?;
    Ok(())
}
pub(super) fn cleanup(
    db: &Connection,
    key: &StorageKey,
    client: &network::HttpsClient,
    head: Head,
) -> Result<usize, Error> {
    let row: Option<(Vec<u8>,Vec<u8>)> = db.query_row("SELECT id,CASE WHEN length(state)<=4096 THEN state END FROM archive_media_garbage ORDER BY id LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    let Some((id, raw)) = row else {
        return Ok(0);
    };
    let id: Id = id.try_into().map_err(|_| Error::InvalidStore)?;
    let removal: Removal =
        serde_json::from_slice(&key.open(&raw, &aad(&id))?).map_err(|_| Error::InvalidStore)?;
    let record = read_record(db, key, id)?;
    if !matches!(record.content, Content::Deleted | Content::Omitted) {
        return Err(Error::InvalidStore);
    }
    let protected: Option<Vec<u8>> = db
        .query_row(
            "SELECT state FROM archive_remote WHERE id=?1",
            [removal.object.as_slice()],
            |r| r.get(0),
        )
        .optional()?;
    let Some(protected) = protected else {
        return Ok(0);
    };
    let scope = load(db, key)?.scope;
    if key
        .open(&protected, &binding(97, &scope, &removal.object))?
        .as_slice()
        != removal.object
    {
        return Err(Error::InvalidStore);
    }
    let session = connection::session_in(db, key)?.ok_or(Error::Unprepared)?;
    if session
        .address
        .split_once(':')
        .ok_or(Error::InvalidStore)?
        .1
        != removal.server
    {
        return Err(Error::Conflict);
    }
    let current = client.recovery_head()?;
    if current.restored_checkpoint
        || current.generation != head.generation
        || current.manifest.as_deref() != Some(crate::transport::hex(&head.manifest).as_str())
    {
        return Err(Error::Conflict);
    }
    match client.remove_attachment(removal.file) {
        Ok(()) | Err(network::Error::Status { code: 404, .. }) => (),
        Err(error) => return Err(error.into()),
    }
    db.execute(
        "INSERT INTO archive_media_removed VALUES(?1,?2) ON CONFLICT(id) DO NOTHING",
        (
            removal.file.as_slice(),
            key.seal(&removal.file, &aad(&removal.file))?,
        ),
    )?;
    db.execute(
        "DELETE FROM archive_media_garbage WHERE id=?1",
        [id.as_slice()],
    )?;
    Ok(1)
}
pub(crate) fn removed(db: &Connection, key: &StorageKey, file: Id) -> Result<bool, Error> {
    let raw: Option<Vec<u8>> = db.query_row("SELECT CASE WHEN length(state)=68 THEN state END FROM archive_media_removed WHERE id=?1",[file.as_slice()],|r|r.get(0)).optional()?;
    let Some(raw) = raw else {
        return Ok(false);
    };
    if key.open(&raw, &aad(&file))?.as_slice() != file {
        return Err(Error::InvalidStore);
    }
    Ok(true)
}
