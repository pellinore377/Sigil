use super::*;

const PREFIX: &[u8] = b"Sigil/erased-content/v0\0";

pub(crate) fn marker(message: Id) -> Vec<u8> {
    [PREFIX, &message].concat()
}
pub(crate) fn erased(raw: &[u8]) -> Option<Id> {
    raw.strip_prefix(PREFIX)?.try_into().ok()
}
pub(crate) fn require_retained(raw: &[u8]) -> Result<(), Error> {
    if erased(raw).is_some() {
        Err(Error::Obsolete)
    } else {
        Ok(())
    }
}

#[derive(Default, Debug)]
pub struct JournalErasure {
    pub checked: usize,
    pub erased: usize,
}

pub(crate) fn cursor(db: &Connection, key: &StorageKey, kind: u8) -> Result<i64, Error> {
    let raw: Option<Vec<u8>> = db
        .query_row(
            "SELECT CASE WHEN length(state)=44 THEN state END FROM erasure_cursor WHERE kind=?1",
            [kind],
            |r| r.get(0),
        )
        .optional()?;
    let after = raw
        .map(|raw| {
            key.open(&raw, &binding(104, &[0; 32], &[kind]))?
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
    Ok(after)
}
pub(crate) fn advance(
    tx: &Transaction<'_>,
    key: &StorageKey,
    kind: u8,
    after: i64,
) -> Result<(), Error> {
    tx.execute("INSERT INTO erasure_cursor VALUES(?1,?2) ON CONFLICT(kind) DO UPDATE SET state=excluded.state", (kind, key.seal(&after.to_be_bytes(), &binding(104, &[0;32], &[kind]))?))?;
    Ok(())
}

impl ClientStore {
    /// Bounded removal of obsolete bodies; retain packet commitments and replay IDs.
    pub fn erase_obsolete_journals(&mut self, now: u64) -> Result<JournalErasure, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = conversations::time_floor(&tx, &self.key, now)?;
        let mut result = JournalErasure::default();
        for (kind, table, domain) in [(0, "inbox", 2), (1, "outbox", 9)] {
            let after = cursor(&tx, &self.key, kind)?;
            let rows = tx.prepare(&format!("SELECT rowid,session,id,CASE WHEN content IS NULL THEN NULL WHEN length(content)<=65572 THEN content ELSE X'' END FROM {table} WHERE rowid>?1 ORDER BY rowid LIMIT 16"))?.query_map([after], |r| Ok((r.get::<_,i64>(0)?, r.get::<_,Vec<u8>>(1)?, r.get::<_,Vec<u8>>(2)?, r.get::<_,Option<Vec<u8>>>(3)?)))?.collect::<Result<Vec<_>,_>>()?;
            let next = rows.last().map_or(0, |row| row.0);
            for (_, session, id, sealed) in rows {
                result.checked += 1;
                let Some(sealed) = sealed else { continue };
                let session: Id = session.try_into().map_err(|_| Error::InvalidStore)?;
                let id: Id = id.try_into().map_err(|_| Error::InvalidStore)?;
                let aad = binding(domain, &session, &id);
                let raw = self.key.open(&sealed, &aad)?;
                if erased(&raw).is_some() {
                    continue;
                }
                let Ok(event) = sigil_protocol::event::Direct::from_bytes(&raw) else {
                    continue;
                };
                conversations::sync::journal(&tx, &self.key, kind, session, id, &event)?;
                match conversations::require_payload(&tx, &self.key, &raw, now, true) {
                    Err(Error::Obsolete) => (),
                    Ok(()) => continue,
                    Err(e) => return Err(e),
                }
                // An unacknowledged incoming result must remain routable after a crash.
                if kind == 0
                    && tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM incoming WHERE acknowledged=0)",
                        [],
                        |r| r.get::<_, bool>(0),
                    )?
                {
                    continue;
                }
                if kind == 1 {
                    conversations::mark_cancelled(&tx, &self.key, &session, &id)?;
                    tx.execute(
                        "UPDATE outbox SET packet=NULL WHERE session=?1 AND id=?2",
                        (session.as_slice(), id.as_slice()),
                    )?;
                }
                tx.execute(
                    &format!("UPDATE {table} SET content=?3 WHERE session=?1 AND id=?2"),
                    (
                        session.as_slice(),
                        id.as_slice(),
                        self.key.seal(&marker(event.message), &aad)?,
                    ),
                )?;
                result.erased += 1;
            }
            advance(&tx, &self.key, kind, next)?;
        }
        if tx.query_row("SELECT EXISTS(SELECT 1 FROM own_device_binding)", [], |r| {
            r.get::<_, bool>(0)
        })? {
            groups::erase_journals(&tx, &self.key, now, &mut result)?;
            structured::erase(&tx, &self.key, &mut result)?;
        }
        tx.commit()?;
        Ok(result)
    }
}
