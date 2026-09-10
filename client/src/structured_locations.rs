use super::*;
use sigil_protocol::text::{
    location::{Duration, Mode, Point, Share},
    Text,
};
pub(crate) const MIGRATION:&str="CREATE TABLE location_jobs(id BLOB PRIMARY KEY,content BLOB NOT NULL); PRAGMA user_version=65;";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Job {
    conversation: Id,
    reference: Reference,
    stop: Option<Action>,
}
pub enum LocationKind {
    Once,
    Pin,
    Live(Duration),
}
pub struct LocationState {
    pub share: Share,
    pub stopped: bool,
    pub until: Option<u64>,
}
impl LocationState {
    pub fn active(&self, now: u64) -> bool {
        !self.stopped && self.until.is_some_and(|until| now < until)
    }
}
pub struct LocationJob {
    pub conversation: Id,
    pub reference: Reference,
    pub next_sample_at: u64,
    pub until: u64,
}
pub struct LocationBatch {
    pub jobs: Vec<LocationJob>,
    pub stops: Vec<(Id, Action)>,
    pub next: Option<Id>,
}
fn aad(id: &Id) -> Vec<u8> {
    [b"Sigil/local-location-job/v1".as_slice(), id].concat()
}
fn device(db: &Connection, key: &StorageKey) -> Result<Id, Error> {
    connection::decode_id(
        &connection::session_in(db, key)?
            .ok_or(Error::Unprepared)?
            .device_id,
    )
}
fn save(db: &Connection, key: &StorageKey, id: &Id, job: &Job) -> Result<(), Error> {
    let bytes = Zeroizing::new(serde_json::to_vec(job).map_err(|_| Error::InvalidStore)?);
    db.execute("INSERT INTO location_jobs VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET content=excluded.content",(id.as_slice(),key.seal(&bytes,&aad(id))?))?;
    Ok(())
}
fn load(db: &Connection, key: &StorageKey, id: &Id) -> Result<Option<Job>, Error> {
    let bytes:Option<Vec<u8>>=db.query_row("SELECT CASE WHEN length(content)<=8192 THEN content END FROM location_jobs WHERE id=?1",[id.as_slice()],|r|r.get(0)).optional()?;
    bytes
        .map(|bytes| {
            let job: Job = serde_json::from_slice(&key.open(&bytes, &aad(id))?)
                .map_err(|_| Error::InvalidStore)?;
            let (scope, _) = account_context(db, key)?;
            if card_index(key, &scope, &job.conversation, &job.reference)? != *id {
                return Err(Error::InvalidStore);
            }
            if let Some(stop) = &job.stop {
                if stop.card != job.reference || stop.change != Change::StopLocation {
                    return Err(Error::InvalidStore);
                }
            }
            Ok(job)
        })
        .transpose()
}
pub(super) fn insert(
    tx: &Transaction<'_>,
    key: &StorageKey,
    index: &Id,
    conversation: Id,
    card: &Card,
) -> Result<(), Error> {
    let Construct::Location(Share {
        mode: Mode::Live { device: owner, .. },
        ..
    }) = &card.content
    else {
        return Ok(());
    };
    let (_, actor) = account_context(tx, key)?;
    if card.creator != actor || *owner != device(tx, key)? || load(tx, key, index)?.is_some() {
        return Ok(());
    }
    save(
        tx,
        key,
        index,
        &Job {
            conversation,
            reference: invalid(Reference::of(card))?,
            stop: None,
        },
    )
}
pub(super) fn stopped(db: &Connection, key: &StorageKey, index: &Id) -> Result<bool, Error> {
    Ok(head(
        db,
        key,
        index,
        &register_index(key, index, Register::LocationStop)?,
    )?
    .is_some())
}
pub(super) fn locally_stopped(
    db: &Connection,
    key: &StorageKey,
    index: &Id,
) -> Result<bool, Error> {
    Ok(load(db, key, index)?.is_some_and(|job| job.stop.is_some()))
}
pub(super) fn state(
    db: &Transaction<'_>,
    key: &StorageKey,
    conversation: Id,
    index: &Id,
    card: &Card,
    definition: &CardDefinition,
) -> Result<Option<LocationState>, Error> {
    let Construct::Location(share) = &definition.content else {
        return Ok(None);
    };
    let allowed = if let Mode::Live { device, .. } = share.mode {
        crate::groups::location_allowed(db, key, conversation, card.creator, device)?
    } else {
        true
    };
    Ok(Some(LocationState {
        share: share.clone(),
        stopped: !allowed || stopped(db, key, index)? || locally_stopped(db, key, index)?,
        until: invalid(share.until(card.created_at))?,
    }))
}
impl ClientStore {
    pub fn location_card(
        &self,
        id: Id,
        kind: LocationKind,
        point: Point,
        label: Text,
        now: u64,
    ) -> Result<Card, Error> {
        if point.sampled_at > now || now.saturating_sub(point.sampled_at) > 300 {
            return Err(Error::InvalidEvent);
        }
        let mode = match kind {
            LocationKind::Once => Mode::Once,
            LocationKind::Pin => Mode::Pin,
            LocationKind::Live(duration) => Mode::Live {
                duration,
                device: device(&self.db, &self.key)?,
            },
        };
        let card = Card {
            id,
            creator: self.account_reference()?,
            created_at: now,
            content: Construct::Location(Share { mode, point, label }),
        };
        invalid(card.validate(Default::default()))?;
        Ok(card)
    }
    pub fn update_location(
        &self,
        conversation: Id,
        reference: Reference,
        point: Point,
        now: u64,
    ) -> Result<Action, Error> {
        observe_card_expiry(&self.db, &self.key, conversation, &reference, now)?;
        let (scope, actor) = account_context(&self.db, &self.key)?;
        let (index, card) = visible_card(&self.db, &self.key, &scope, &conversation, &reference)?;
        let definition = definition(&self.db, &self.key, &index, &card)?;
        let tx = self.db.unchecked_transaction()?;
        let mut state = state(&tx, &self.key, conversation, &index, &card, &definition)?
            .ok_or(Error::InvalidEvent)?;
        drop(tx);
        let Mode::Live { device: owner, .. } = state.share.mode else {
            return Err(Error::InvalidEvent);
        };
        if card.creator != actor
            || owner != device(&self.db, &self.key)?
            || !state.active(now)
            || load(&self.db, &self.key, &index)?.is_some_and(|job| job.stop.is_some())
        {
            return Err(Error::Obsolete);
        }
        if point.sampled_at > now
            || point.sampled_at < state.share.point.sampled_at.saturating_add(10)
        {
            return Err(Error::Unprepared);
        }
        state.share.point = point;
        self.edit_card(
            conversation,
            reference,
            Construct::Location(state.share),
            now,
        )
    }
    /// Stop local sampling durably before the caller queues the encrypted stop action.
    pub fn stop_location(
        &mut self,
        conversation: Id,
        reference: Reference,
        now: u64,
    ) -> Result<Action, Error> {
        let (scope, actor) = account_context(&self.db, &self.key)?;
        let (index, card) = visible_card(&self.db, &self.key, &scope, &conversation, &reference)?;
        if let Some(job) = load(&self.db, &self.key, &index)? {
            if let Some(stop) = job.stop {
                return Ok(stop);
            }
        }
        let action = Action {
            card: reference,
            actor,
            created_at: now,
            previous: None,
            revision: None,
            change: Change::StopLocation,
        };
        invalid(action.validate_for(&card))?;
        self.require_action(conversation, &action)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        save(
            &tx,
            &self.key,
            &index,
            &Job {
                conversation,
                reference: action.card,
                stop: Some(action.clone()),
            },
        )?;
        tx.commit()?;
        Ok(action)
    }
    /// Page through durable shares; the platform owns permission and GPS sampling.
    pub fn location_jobs(&mut self, after: Option<Id>, now: u64) -> Result<LocationBatch, Error> {
        let now = crate::conversations::time_floor(&self.db, &self.key, now)?;
        let rows: Vec<Id> = self
            .db
            .prepare("SELECT id FROM location_jobs WHERE id>?1 ORDER BY id LIMIT 65")?
            .query_map([after.unwrap_or([0; 32]).as_slice()], |r| {
                r.get::<_, Vec<u8>>(0)
            })?
            .map(|r| {
                r.map_err(Error::from)?
                    .try_into()
                    .map_err(|_| Error::InvalidStore)
            })
            .collect::<Result<_, _>>()?;
        let next = (rows.len() > 64).then(|| rows[63]);
        let mut jobs = Vec::new();
        let mut stops = Vec::new();
        for index in rows.into_iter().take(64) {
            let job = load(&self.db, &self.key, &index)?.ok_or(Error::InvalidStore)?;
            observe_card_expiry(&self.db, &self.key, job.conversation, &job.reference, now)?;
            let (scope, actor) = account_context(&self.db, &self.key)?;
            let card = match visible_card(
                &self.db,
                &self.key,
                &scope,
                &job.conversation,
                &job.reference,
            ) {
                Ok((_, card)) => card,
                Err(Error::Unprepared) => continue,
                Err(Error::Obsolete | Error::NotFound) => {
                    self.db
                        .execute("DELETE FROM location_jobs WHERE id=?1", [index.as_slice()])?;
                    continue;
                }
                Err(e) => return Err(e),
            };
            let definition = definition(&self.db, &self.key, &index, &card)?;
            let tx = self.db.unchecked_transaction()?;
            let state = state(&tx, &self.key, job.conversation, &index, &card, &definition)?
                .ok_or(Error::InvalidStore)?;
            drop(tx);
            if stopped(&self.db, &self.key, &index)?
                || state.until.is_none_or(|until| now >= until)
                || card.creator != actor
            {
                self.db
                    .execute("DELETE FROM location_jobs WHERE id=?1", [index.as_slice()])?;
                continue;
            }
            if let Some(stop) = job.stop {
                stops.push((job.conversation, stop));
            } else if !matches!(state.share.mode,Mode::Live{device:owner,..} if owner==device(&self.db,&self.key)?) {
                self.db.execute("DELETE FROM location_jobs WHERE id=?1", [index.as_slice()])?;
            } else if state.stopped {
                let stop = self.stop_location(job.conversation, job.reference, now)?;
                stops.push((job.conversation, stop));
            } else {
                jobs.push(LocationJob {
                    conversation: job.conversation,
                    reference: job.reference,
                    next_sample_at: state.share.point.sampled_at.saturating_add(10),
                    until: state.until.ok_or(Error::InvalidStore)?,
                });
            }
        }
        Ok(LocationBatch { jobs, stops, next })
    }
}
