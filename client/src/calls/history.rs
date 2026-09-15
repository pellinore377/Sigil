use super::*;

pub(crate) const MIGRATION: &str =
    "CREATE TABLE call_history(id BLOB PRIMARY KEY,content BLOB NOT NULL); PRAGMA user_version=70;";

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HistoryPerson {
    pub peer: Id,
    pub name: String,
    pub address: String,
    pub own: bool,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct History {
    pub id: Id,
    pub direct: bool,
    pub created: u64,
    pub expires: u64,
    pub phase: Phase,
    pub outgoing: bool,
    pub people: Vec<HistoryPerson>,
    #[serde(default)]
    pub missed: bool,
    #[serde(default)]
    pub duration: Option<u64>,
    #[serde(default)]
    pub video: Option<bool>,
}
fn aad(row: &Id) -> Vec<u8> {
    [b"Sigil/call-history/v1".as_slice(), row].concat()
}
fn read(db: &Connection, key: &StorageKey, row: &Id) -> Result<History, Error> {
    let sealed: Vec<u8> = db.query_row("SELECT CASE WHEN length(content)<=65572 THEN content END FROM call_history WHERE id=?1", [row.as_slice()], |r| r.get(0)).optional()?.ok_or(Error::NotFound)?;
    let value: History =
        serde_json::from_slice(&key.open(&sealed, &aad(row))?).map_err(|_| Error::InvalidStore)?;
    if index(key, &value.id)? != *row || value.people.len() > 256 {
        return Err(Error::InvalidStore);
    }
    Ok(value)
}
pub(super) fn retain(db: &Connection, key: &StorageKey, record: &Record) -> Result<(), Error> {
    let row = index(key, &record.id())?;
    let previous = match read(db, key, &row) {
        Ok(value) => Some(value),
        Err(Error::NotFound) => None,
        Err(error) => return Err(error),
    };
    let mut value = previous.clone().unwrap_or(History {
        id: record.id(),
        direct: record.direct,
        created: record.state.roster.roster.created,
        expires: record.state.roster.roster.expires,
        phase: record.phase,
        outgoing: record
            .own
            .as_ref()
            .is_some_and(|own| own.member.key == record.state.roster.roster.owner),
        people: Vec::new(),
        missed: false,
        duration: None,
        video: None,
    });
    value.phase = record.phase;
    value.missed |= record.missed;
    let own = record
        .own
        .as_ref()
        .map(|own| own.fingerprint().map_err(failure))
        .transpose()?;
    let mut bindings = record
        .state
        .participants
        .iter()
        .map(|p| {
            Ok((
                crate::peers::parse(&p.device)?.binding,
                p.fingerprint().map_err(failure)?,
            ))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    for invite in &record.invites {
        let known = crate::peers::known(db, key, &invite.peer)?;
        if known.fingerprint == invite.fingerprint {
            bindings.push((known.binding, known.fingerprint));
        }
    }
    for (binding, fingerprint) in bindings {
        let reference = crate::peers::reference(&binding.server, &binding.device);
        if !value.people.iter().any(|person| person.peer == reference) {
            if value.people.len() == 256 {
                return Err(Error::Limit);
            }
            value.people.push(HistoryPerson {
                peer: reference,
                name: binding.username.clone(),
                address: format!("@{}:{}", binding.username, binding.server),
                own: Some(fingerprint) == own,
            });
        }
    }
    if previous.as_ref() == Some(&value) {
        return Ok(());
    }
    let raw = Zeroizing::new(serde_json::to_vec(&value).map_err(|_| Error::InvalidStore)?);
    if raw.len() > 65536 {
        return Err(Error::Limit);
    }
    db.execute("INSERT INTO call_history VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET content=excluded.content", (row.as_slice(), key.seal(&raw, &aad(&row))?))?;
    db.execute("DELETE FROM call_history WHERE rowid IN(SELECT rowid FROM call_history ORDER BY rowid DESC LIMIT -1 OFFSET 1000)", [])?;
    Ok(())
}
impl ClientStore {
    pub fn record_call_media(&mut self, id: Id, duration: u64, video: bool) -> Result<(), Error> {
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = index(&self.key, &id)?;
        let mut value = read(&tx, &self.key, &row)?;
        if duration > value.expires.saturating_sub(value.created) || value.missed {
            return Err(Error::InvalidEvent);
        }
        value.duration = Some(value.duration.unwrap_or(0).max(duration));
        value.video = Some(value.video.unwrap_or(false) || video);
        let raw = Zeroizing::new(serde_json::to_vec(&value).map_err(|_| Error::InvalidStore)?);
        tx.execute("UPDATE call_history SET content=?1 WHERE id=?2", (self.key.seal(&raw, &aad(&row))?, row.as_slice()))?;
        tx.commit()?;
        Ok(())
    }
    pub fn call_history(&self) -> Result<Vec<History>, Error> {
        let rows: Vec<Vec<u8>> = self
            .db
            .prepare("SELECT id FROM call_history ORDER BY rowid DESC LIMIT 1000")?
            .query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        rows.into_iter()
            .map(|row| {
                read(
                    &self.db,
                    &self.key,
                    &row.try_into().map_err(|_| Error::InvalidStore)?,
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_history_has_unknown_media_without_inventing_a_missed_call() {
        let legacy = serde_json::json!({"id":vec![0;32],"direct":true,"created":10,"expires":70,"phase":"ended","outgoing":false,"people":[]});
        let value: History = serde_json::from_value(legacy).unwrap();
        assert!(!value.missed);
        assert_eq!(value.duration, None);
        assert_eq!(value.video, None);
        let restored: History = serde_json::from_slice(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(restored == value);
    }
}
