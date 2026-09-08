use super::*;
use sigil_protocol::text::Text;

pub(crate) const MIGRATION: &str = "CREATE TABLE structured_alarms(id BLOB PRIMARY KEY,pending INTEGER NOT NULL CHECK(pending IN(0,1)),content BLOB NOT NULL); CREATE INDEX structured_alarms_pending ON structured_alarms(pending,id); CREATE TABLE structured_alarm_cursor(id INTEGER PRIMARY KEY CHECK(id=1),content BLOB NOT NULL);";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    conversation: Id,
    message: Id,
    reference: Reference,
    revision: Option<Id>,
    at: u64,
    generation: u64,
    pending: bool,
    enabled: bool,
    eligible: bool,
    cancelled: bool,
    fired: bool,
}
pub struct AlarmJob {
    pub id: Id,
    pub version: Id,
    /// None cancels the platform alarm with the same ID.
    pub at: Option<u64>,
}
pub struct AlarmBatch {
    pub jobs: Vec<AlarmJob>,
    pub more: bool,
}
pub struct AlarmNotification {
    pub conversation: Id,
    pub message: crate::conversations::Reference,
    pub text: Text,
}
fn aad(id: &Id) -> Vec<u8> {
    [b"Sigil/local-alarm/v1".as_slice(), id].concat()
}
fn load(db: &Connection, key: &StorageKey, id: &Id) -> Result<Option<State>, Error> {
    let row: Option<(bool, Vec<u8>)> = db.query_row("SELECT pending,CASE WHEN length(content)<=4096 THEN content END FROM structured_alarms WHERE id=?1", [id.as_slice()], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
    let Some((pending, bytes)) = row else {
        return Ok(None);
    };
    let state: State =
        serde_json::from_slice(&key.open(&bytes, &aad(id))?).map_err(|_| Error::InvalidStore)?;
    let (scope, _) = account_context(db, key)?;
    if state.pending != pending
        || state.at == 0
        || state.at > i64::MAX as u64
        || state.generation == 0
        || state.generation > i64::MAX as u64
        || card_index(key, &scope, &state.conversation, &state.reference)? != *id
    {
        return Err(Error::InvalidStore);
    }
    Ok(Some(state))
}
fn save(db: &Connection, key: &StorageKey, id: &Id, state: &State) -> Result<(), Error> {
    let bytes = Zeroizing::new(serde_json::to_vec(state).map_err(|_| Error::InvalidStore)?);
    db.execute("INSERT INTO structured_alarms VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET pending=excluded.pending,content=excluded.content", (id.as_slice(), state.pending, key.seal(&bytes, &aad(id))?))?;
    Ok(())
}
fn changed(state: &mut State) -> Result<(), Error> {
    state.generation = state
        .generation
        .checked_add(1)
        .filter(|v| *v <= i64::MAX as u64)
        .ok_or(Error::Limit)?;
    state.pending = true;
    Ok(())
}
fn version(key: &StorageKey, id: &Id, state: &State) -> Result<Id, Error> {
    Ok(key.commitment(
        &[id.as_slice(), &state.generation.to_be_bytes()].concat(),
        b"Sigil/local-alarm-version/v1",
    )?)
}
fn due(content: &Construct) -> Option<u64> {
    match content {
        Construct::Reminder(value) => Some(value.at),
        Construct::Timer(value) => Some(value.ends_at),
        _ => None,
    }
}
pub(super) fn insert(
    tx: &Transaction<'_>,
    key: &StorageKey,
    index: &Id,
    conversation: Id,
    message: Id,
    card: &Card,
) -> Result<(), Error> {
    let Some(at) = due(&card.content) else {
        return Ok(());
    };
    if load(tx, key, index)?.is_some() {
        return Ok(());
    }
    save(
        tx,
        key,
        index,
        &State {
            conversation,
            message,
            reference: invalid(Reference::of(card))?,
            revision: None,
            at,
            generation: 1,
            pending: true,
            enabled: true,
            eligible: at > crate::conversations::now(),
            cancelled: false,
            fired: false,
        },
    )
}
pub(super) fn refresh(
    tx: &Transaction<'_>,
    key: &StorageKey,
    index: &Id,
    card: &Card,
) -> Result<(), Error> {
    if due(&card.content).is_none() {
        return Ok(());
    }
    let Some(mut state) = load(tx, key, index)? else {
        return Ok(());
    };
    let definition = definition(tx, key, index, card)?;
    let at = due(&definition.content).ok_or(Error::InvalidStore)?;
    if state.at == at && state.revision == definition.revision {
        return Ok(());
    }
    if state.at != at {
        state.fired = false;
        state.eligible = at > crate::conversations::now();
    }
    state.at = at;
    state.revision = definition.revision;
    changed(&mut state)?;
    save(tx, key, index, &state)
}
pub(super) fn cancel(tx: &Connection, key: &StorageKey, index: &Id) -> Result<(), Error> {
    let Some(mut state) = load(tx, key, index)? else {
        return Ok(());
    };
    if !state.cancelled {
        state.cancelled = true;
        changed(&mut state)?;
        save(tx, key, index, &state)?;
    }
    Ok(())
}
fn available(
    db: &Connection,
    key: &StorageKey,
    state: &State,
    now: u64,
) -> Result<Option<bool>, Error> {
    let (_, own) = account_context(db, key)?;
    match crate::conversations::message(
        db,
        key,
        state.conversation,
        crate::conversations::Reference {
            author: state.reference.creator,
            message: state.message,
        },
        now,
        own,
    ) {
        Ok(message) => Ok(Some(!message.deleted && !message.view_once)),
        Err(Error::NotFound) => Ok(None),
        Err(Error::Obsolete) => Ok(Some(false)),
        Err(error) => Err(error),
    }
}
impl ClientStore {
    /// Durable replacement/cancellation jobs. A platform adapter acknowledges each exact version.
    pub fn alarm_jobs(&mut self, now: u64) -> Result<AlarmBatch, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let aad = b"Sigil/local-alarm-cursor/v1";
        let prior: Option<Vec<u8>> = tx.query_row("SELECT CASE WHEN length(content)<=68 THEN content END FROM structured_alarm_cursor WHERE id=1", [], |r| r.get(0)).optional()?;
        let cursor = prior.map(|bytes| self.key.open(&bytes, aad)).transpose()?;
        if cursor
            .as_ref()
            .is_some_and(|raw| !matches!(raw.len(), 0 | 32))
        {
            return Err(Error::InvalidStore);
        }
        let ids = tx
            .prepare(
                "SELECT id FROM structured_alarms WHERE pending=1 AND id>?1 ORDER BY id LIMIT 64",
            )?
            .query_map(
                [cursor.as_ref().map_or(&[][..], |raw| raw.as_slice())],
                |r| r.get::<_, Vec<u8>>(0),
            )?
            .collect::<Result<Vec<_>, _>>()?;
        let more = ids.len() == 64;
        let next = if more {
            ids.last().map(Vec::as_slice).unwrap_or_default()
        } else {
            &[]
        };
        tx.execute("INSERT INTO structured_alarm_cursor VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET content=excluded.content", [self.key.seal(next, aad)?])?;
        let mut jobs = Vec::new();
        for bytes in ids {
            let id = bytes
                .as_slice()
                .try_into()
                .map_err(|_| Error::InvalidStore)?;
            let mut state = load(&tx, &self.key, &id)?.ok_or(Error::InvalidStore)?;
            if !state.cancelled {
                match available(&tx, &self.key, &state, now)? {
                    None => continue,
                    Some(false) => {
                        state.cancelled = true;
                        changed(&mut state)?;
                        save(&tx, &self.key, &id, &state)?;
                    }
                    Some(true) => (),
                }
            }
            jobs.push(AlarmJob {
                id,
                version: version(&self.key, &id, &state)?,
                at: (state.enabled && state.eligible && !state.cancelled && !state.fired)
                    .then_some(state.at),
            });
        }
        tx.commit()?;
        Ok(AlarmBatch { jobs, more })
    }
    pub fn acknowledge_alarm_job(&mut self, id: Id, token: Id) -> Result<bool, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(mut state) = load(&tx, &self.key, &id)? else {
            return Ok(false);
        };
        if version(&self.key, &id, &state)? != token {
            return Ok(false);
        }
        state.pending = false;
        save(&tx, &self.key, &id, &state)?;
        tx.commit()?;
        Ok(true)
    }
    pub fn set_alarm_enabled(
        &mut self,
        conversation: Id,
        reference: Reference,
        enabled: bool,
    ) -> Result<(), Error> {
        let (scope, _) = account_context(&self.db, &self.key)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (index, _) = visible_card(&tx, &self.key, &scope, &conversation, &reference)?;
        let mut state = load(&tx, &self.key, &index)?.ok_or(Error::NotFound)?;
        if state.enabled != enabled {
            state.enabled = enabled;
            changed(&mut state)?;
            save(&tx, &self.key, &index, &state)?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn alarm_notification(
        &self,
        id: Id,
        token: Id,
        now: u64,
    ) -> Result<Option<AlarmNotification>, Error> {
        let Some(state) = load(&self.db, &self.key, &id)? else {
            return Ok(None);
        };
        if version(&self.key, &id, &state)? != token
            || !state.enabled
            || !state.eligible
            || state.cancelled
            || state.fired
            || now < state.at
            || available(&self.db, &self.key, &state, now)? != Some(true)
        {
            return Ok(None);
        }
        let card = load_card(&self.db, &self.key, &id)?.ok_or(Error::InvalidStore)?;
        let definition = definition(&self.db, &self.key, &id, &card)?;
        let text = match definition.content {
            Construct::Reminder(value) => value.text,
            Construct::Timer(_) => invalid(Text::plain("Timer ended", Default::default()))?,
            _ => return Err(Error::InvalidStore),
        };
        Ok(Some(AlarmNotification {
            conversation: state.conversation,
            message: crate::conversations::Reference {
                author: state.reference.creator,
                message: state.message,
            },
            text,
        }))
    }
    /// Call after presenting/replacing the notification under its stable ID.
    pub fn acknowledge_alarm_fired(&mut self, id: Id, token: Id, now: u64) -> Result<bool, Error> {
        if self.alarm_notification(id, token, now)?.is_none() {
            return Ok(false);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut state = load(&tx, &self.key, &id)?.ok_or(Error::InvalidStore)?;
        if version(&self.key, &id, &state)? != token {
            return Ok(false);
        }
        state.fired = true;
        changed(&mut state)?;
        save(&tx, &self.key, &id, &state)?;
        tx.commit()?;
        Ok(true)
    }
}
