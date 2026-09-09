//! Authenticated per-device sending selection; live state is never restored from history.
use super::*;

fn aad(peer: &Id) -> Vec<u8> {
    binding(19, peer, b"Sigil/active-session/v0")
}
pub(super) fn record(
    db: &Connection,
    key: &StorageKey,
    peer: &Id,
) -> Result<Option<(Id, Id)>, Error> {
    let sealed: Option<Vec<u8>> = db
        .query_row(
            "SELECT CASE WHEN length(state)=100 THEN state END FROM active_sessions WHERE peer=?1",
            [peer.as_slice()],
            |r| r.get(0),
        )
        .optional()?;
    let Some(sealed) = sealed else {
        return Ok(None);
    };
    let bytes = key.open(&sealed, &aad(peer))?;
    if bytes.len() != 64 {
        return Err(Error::InvalidStore);
    }
    Ok(Some((
        bytes[..32].try_into().map_err(|_| Error::InvalidStore)?,
        bytes[32..].try_into().map_err(|_| Error::InvalidStore)?,
    )))
}
pub(super) fn selected(db: &Connection, key: &StorageKey, peer: &Id) -> Result<Option<Id>, Error> {
    let known = peers::known(db, key, peer)?;
    if !known.trusted {
        return Err(Error::Unprepared);
    }
    let Some((session, fingerprint)) = record(db, key, peer)? else {
        return Ok(None);
    };
    if fingerprint != known.fingerprint || session_peer(db, &session)? != Some(*peer) {
        return Err(Error::Conflict);
    }
    load(db, key, &session)?;
    Ok(Some(session))
}
pub(super) fn activate(
    tx: &Transaction<'_>,
    key: &StorageKey,
    peer: &Id,
    session: &Id,
) -> Result<(), Error> {
    let known = peers::known(tx, key, peer)?;
    if !known.trusted || session_peer(tx, session)? != Some(*peer) {
        return Err(Error::Unprepared);
    }
    load(tx, key, session)?;
    if let Some((current, fingerprint)) = record(tx, key, peer)? {
        if fingerprint != known.fingerprint || session_peer(tx, &current)? != Some(*peer) {
            return Err(Error::Conflict);
        }
        if current == *session {
            return Ok(());
        }
        load(tx, key, &current)?;
    }
    let sealed = key.seal(
        &[session.as_slice(), &known.fingerprint].concat(),
        &aad(peer),
    )?;
    tx.execute("INSERT INTO active_sessions(peer,state) VALUES(?1,?2) ON CONFLICT(peer) DO UPDATE SET state=excluded.state", (peer.as_slice(), sealed))?;
    Ok(())
}
/// Authenticated replies outrank unconfirmed initials. Confirmed sessions use
/// shared transcript ordering; fresh initials still activate directly for recovery.
pub(super) fn converge(
    tx: &Transaction<'_>,
    key: &StorageKey,
    peer: &Id,
    session: &Id,
) -> Result<(), Error> {
    if let Some(current) = selected(tx, key, peer)? {
        if current == *session {
            return Ok(());
        }
        let current_state = load(tx, key, &current)?.1;
        let incoming_state = load(tx, key, session)?.1;
        if current_state.peer_confirmed()
            && current_state.convergence_id() < incoming_state.convergence_id()
        {
            return Ok(());
        }
    }
    activate(tx, key, peer, session)
}
/// New sends may leave an expired initial, but retries must retain their session.
pub(super) fn for_send(
    tx: &Transaction<'_>,
    key: &StorageKey,
    peer: &Id,
    now: u64,
) -> Result<Id, Error> {
    let current = selected(tx, key, peer)?.ok_or(Error::Unprepared)?;
    if load(tx, key, &current)?.1.peer_confirmed() {
        return Ok(current);
    }
    match handshake::prepare_expiry(tx, key, &current, None, now) {
        Ok(_) => return Ok(current),
        Err(Error::Expired) => {}
        Err(error) => return Err(error),
    }
    let ids: Vec<Vec<u8>> = tx
        .prepare("SELECT id FROM sessions WHERE peer=?1 AND retired=0 ORDER BY id LIMIT ?2")?
        .query_map((peer.as_slice(), peers::MAX_SESSIONS as i64 + 1), |r| {
            r.get(0)
        })?
        .collect::<Result<_, _>>()?;
    if ids.len() > peers::MAX_SESSIONS {
        return Err(Error::Limit);
    }
    let mut best: Option<(Id, Id)> = None;
    for raw in ids {
        let id: Id = raw.try_into().map_err(|_| Error::InvalidStore)?;
        let state = load(tx, key, &id)?.1;
        if state.peer_confirmed() {
            let candidate = (state.convergence_id(), id);
            if best.is_none_or(|prior| candidate < prior) {
                best = Some(candidate);
            }
        }
    }
    let (_, session) = best.ok_or(Error::Expired)?;
    activate(tx, key, peer, &session)?;
    Ok(session)
}
pub(super) fn retire(
    tx: &Transaction<'_>,
    key: &StorageKey,
    peer: &Id,
    session: &Id,
) -> Result<(), Error> {
    if record(tx, key, peer)?.is_some_and(|(current, _)| current == *session) {
        tx.execute(
            "DELETE FROM active_sessions WHERE peer=?1",
            [peer.as_slice()],
        )?;
    }
    Ok(())
}
impl ClientStore {
    /// The verified device's current sending session. Older databases start with
    /// no inferred selection; new sessions or authenticated incoming traffic select it.
    pub fn active_session(&self, peer: Id) -> Result<Option<Id>, Error> {
        selected(&self.db, &self.key, &peer)
    }
}

#[cfg(test)]
#[path = "selection_tests.rs"]
mod tests;
