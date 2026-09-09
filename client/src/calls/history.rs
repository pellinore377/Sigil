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
    });
    value.phase = record.phase;
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
