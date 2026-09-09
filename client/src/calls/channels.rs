use super::*;

pub(super) fn authorized(
    db: &Connection,
    key: &StorageKey,
    record: &Record,
    peer_id: &Id,
) -> Result<crate::Peer, Error> {
    let known = crate::peers::known(db, key, peer_id)?;
    if known.blocked || known.changed_fingerprint.is_some() || known.replaced_by.is_some() {
        return Err(Error::Unprepared);
    }
    if !known.trusted
        && !record.state.participants.iter().any(|proof| {
            peer(proof).ok() == Some(*peer_id)
                && proof.fingerprint().ok() == Some(known.fingerprint)
        })
    {
        return Err(Error::Unprepared);
    }
    Ok(known)
}

pub(crate) fn scoped(db: &Transaction<'_>, key: &StorageKey, session: &Id) -> Result<bool, Error> {
    for (query, kind) in [
        ("SELECT id,content FROM inbox WHERE session=?1 AND length(content)=244 ORDER BY rowid LIMIT 1", 2),
        ("SELECT id,content FROM outbox WHERE session=?1 AND length(content)=244 ORDER BY rowid LIMIT 1", 9),
    ] {
        let row: Option<(Vec<u8>, Vec<u8>)> = db.query_row(query, [session.as_slice()], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        if let Some((message, sealed)) = row {
            let id: Id = message.try_into().map_err(|_| Error::InvalidStore)?;
            let raw = key.open(&sealed, &crate::binding(kind, session, &id))?;
            if control::authenticate_marker(key, &raw)? {
                if control::receipt_message(&raw)? != Some(id) || crate::session_peer(db, session)?.is_some() { return Err(Error::InvalidStore); }
                return Ok(true);
            }
        }
    }
    Ok(false)
}
