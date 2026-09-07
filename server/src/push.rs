//! One revisioned provider capability per device. Push never carries an event.
use crate::{
    auth::{digest, random_secret},
    prekeys::{active, authorize},
    push_config::{self, Stored},
    push_provider::{validate_target, Vapid},
    store::{Store, StoreError},
};
use push_config::{optional_unsigned, sql, unsigned};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use sigil_protocol::{
    accounts::valid_credential,
    push::{Confirm, Disable, Providers, Register, State, Status, Target},
};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

pub(crate) const SLOT_BYTES: u64 = 8192;
pub(crate) const CHALLENGE_LIFETIME: u64 = 600;
pub(crate) const CHANNEL_LIFETIME: u64 = 30 * 24 * 60 * 60;
pub(crate) const MIGRATION: &str = "
CREATE TABLE push_channels(device TEXT PRIMARY KEY REFERENCES devices(id),revision INTEGER NOT NULL CHECK(revision>0),state INTEGER NOT NULL CHECK(state BETWEEN 0 AND 4),provider INTEGER NOT NULL CHECK(provider IN (0,1)),generation INTEGER NOT NULL CHECK(generation>=0),channel TEXT,expires_at INTEGER,last_change INTEGER NOT NULL CHECK(last_change>=0),request_hash BLOB NOT NULL CHECK(length(request_hash)=32),target TEXT CHECK(length(target)<=8192),proof TEXT,proof_hash BLOB,not_before INTEGER NOT NULL CHECK(not_before>=0));
CREATE INDEX push_channels_expiry ON push_channels(expires_at,device) WHERE target IS NOT NULL;
CREATE TABLE push_jobs(device TEXT PRIMARY KEY REFERENCES push_channels(device),revision INTEGER NOT NULL CHECK(revision>0),through_sequence INTEGER NOT NULL CHECK(through_sequence>=0),expires_at INTEGER NOT NULL,due_at INTEGER NOT NULL,attempts INTEGER NOT NULL CHECK(attempts>=0),lease TEXT,lease_until INTEGER NOT NULL);
CREATE INDEX push_jobs_due ON push_jobs(due_at,device);
";
pub(crate) struct Channel {
    pub revision: u64,
    pub state: State,
    pub provider: u8,
    pub generation: u64,
    pub channel: Option<String>,
    pub expires: Option<u64>,
    pub changed: u64,
    pub hash: Vec<u8>,
    pub target: Option<Zeroizing<String>>,
    pub proof: Option<Zeroizing<String>>,
    pub proof_hash: Option<Vec<u8>>,
    pub not_before: u64,
}
impl Channel {
    pub(crate) fn status(&self) -> Status {
        Status {
            revision: self.revision,
            state: self.state.clone(),
            channel: self.channel.clone(),
            expires_at: self.expires,
        }
    }
}
pub(crate) fn read(db: &Connection, device: &str) -> Result<Option<Channel>, StoreError> {
    let row = db.query_row("SELECT revision,state,provider,generation,channel,expires_at,last_change,request_hash,target,proof,proof_hash,not_before FROM push_channels WHERE device=?1 AND (target IS NULL OR length(target)<=8192)", [device], |r| {
        let state = match r.get::<_,u8>(1)? { 0=>State::Disabled, 1=>State::Pending, 2=>State::Active, 3=>State::Invalid, 4=>State::Expired, _=>return Err(rusqlite::Error::InvalidQuery) };
        Ok(Channel { revision:unsigned(r,0)?, state, provider:r.get(2)?, generation:unsigned(r,3)?, channel:r.get(4)?, expires:optional_unsigned(r,5)?, changed:unsigned(r,6)?, hash:r.get(7)?, target:r.get::<_,Option<String>>(8)?.map(Zeroizing::new), proof:r.get::<_,Option<String>>(9)?.map(Zeroizing::new), proof_hash:r.get(10)?, not_before:unsigned(r,11)? })
    }).optional()?;
    Ok(row)
}
pub(crate) fn clear(db: &Connection, device: &str, state: State) -> Result<(), StoreError> {
    let state = match state {
        State::Disabled => 0,
        State::Invalid => 3,
        State::Expired => 4,
        _ => return Err(StoreError::InvalidData),
    };
    db.execute("UPDATE push_channels SET state=?2,target=NULL,proof=NULL,proof_hash=NULL,expires_at=NULL WHERE device=?1", (device,state))?;
    db.execute("DELETE FROM push_jobs WHERE device=?1", [device])?;
    Ok(())
}
pub(crate) fn current(
    db: &Connection,
    config: &Stored,
    device: &str,
    now: u64,
) -> Result<Option<Channel>, StoreError> {
    if now > i64::MAX as u64 {
        return Err(StoreError::InvalidData);
    }
    let Some(mut row) = read(db, device)? else {
        return Ok(None);
    };
    if now < row.changed {
        return Err(StoreError::InvalidData);
    }
    if matches!(row.state, State::Pending | State::Active) {
        let terminal = if config.generation(row.provider).ok() != Some(row.generation)
            || !active(db, device)?
        {
            Some(State::Invalid)
        } else if row.expires.is_none_or(|expires| expires <= now) {
            Some(State::Expired)
        } else {
            None
        };
        if let Some(state) = terminal {
            clear(db, device, state.clone())?;
            row.state = state;
            row.target = None;
            row.proof = None;
            row.proof_hash = None;
            row.expires = None;
        }
    }
    Ok(Some(row))
}
fn empty() -> Status {
    Status {
        revision: 0,
        state: State::Disabled,
        channel: None,
        expires_at: None,
    }
}
pub(crate) fn bytes(value: &str) -> Result<[u8; 32], StoreError> {
    if !valid_credential(value) {
        return Err(StoreError::Invalid("invalid push identifier"));
    }
    let mut bytes = [0; 32];
    for (i, chunk) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let digit = |b: u8| {
            if b.is_ascii_digit() {
                b - b'0'
            } else {
                b - b'a' + 10
            }
        };
        bytes[i] = (digit(chunk[0]) << 4) | digit(chunk[1]);
    }
    Ok(bytes)
}
pub(crate) fn deadline(now: u64, seconds: u64) -> Result<u64, StoreError> {
    now.checked_add(seconds)
        .filter(|n| *n <= i64::MAX as u64)
        .ok_or(StoreError::InvalidData)
}

impl Store {
    pub fn push_providers(&mut self, credential: &str, now: u64) -> Result<Providers, StoreError> {
        let tx = self.0.transaction()?;
        authorize(&tx, credential, now)?;
        let config = push_config::read(&tx)?;
        let vapid_public_key = if config.settings.unified {
            Some(
                Vapid::from_pkcs8(
                    config
                        .settings
                        .vapid
                        .as_ref()
                        .ok_or(StoreError::InvalidData)?,
                )
                .map_err(|_| StoreError::InvalidData)?
                .public_key(),
            )
        } else {
            None
        };
        Ok(Providers {
            unified_push: config.settings.unified,
            fcm: config.settings.fcm.is_some(),
            vapid_public_key,
        })
    }
    pub fn push_status(&mut self, credential: &str, now: u64) -> Result<Status, StoreError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let device = authorize(&tx, credential, now)?;
        let config = push_config::read(&tx)?;
        let status = current(&tx, &config, &device, now)?
            .map(|c| c.status())
            .unwrap_or_else(empty);
        tx.commit()?;
        Ok(status)
    }
    pub fn register_push(
        &mut self,
        credential: &str,
        request: Register,
        now: u64,
    ) -> Result<Status, StoreError> {
        let hash = push_config::hash(b"Sigil/register-push/v0", &request)?;
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let device = authorize(&tx, credential, now)?;
        let config = push_config::read(&tx)?;
        let previous = current(&tx, &config, &device, now)?;
        let revision = previous.as_ref().map_or(0, |p| p.revision);
        if request.expected_revision != revision {
            if previous.as_ref().is_some_and(|p| {
                request.expected_revision.checked_add(1) == Some(p.revision) && p.hash == hash
            }) {
                let status = previous.unwrap().status();
                tx.commit()?;
                return Ok(status);
            }
            return Err(StoreError::Conflict);
        }
        if previous
            .as_ref()
            .is_some_and(|p| now < p.changed.saturating_add(60))
        {
            return Err(StoreError::Busy);
        }
        let provider = match request.target {
            Target::Fcm { .. } => 0,
            Target::UnifiedPush { .. } => 1,
        };
        let generation = config.generation(provider)?;
        let vapid = if provider == 1 {
            Vapid::from_pkcs8(
                config
                    .settings
                    .vapid
                    .as_ref()
                    .ok_or(StoreError::InvalidData)?,
            )
            .map_err(|_| StoreError::InvalidData)?
            .public_key()
        } else {
            String::new()
        };
        validate_target(&request.target, &vapid)
            .map_err(|_| StoreError::Invalid("invalid push target"))?;
        let target = Zeroizing::new(
            serde_json::to_string(&request.target).map_err(|_| StoreError::InvalidData)?,
        );
        if target.len() > sigil_protocol::push::MAX_BODY {
            return Err(StoreError::Invalid("push target is too large"));
        }
        if previous.is_none() {
            crate::storage_budget::for_device(&tx, &device, SLOT_BYTES, now)?;
        }
        let channel = random_secret().map_err(|_| StoreError::InvalidData)?;
        let proof = Zeroizing::new(random_secret().map_err(|_| StoreError::InvalidData)?);
        let expires = deadline(now, CHALLENGE_LIFETIME)?;
        let revision = push_config::next(revision)?;
        let not_before = previous
            .as_ref()
            .filter(|p| {
                p.target
                    .as_ref()
                    .is_some_and(|t| t.as_str() == target.as_str())
            })
            .map_or(0, |p| p.not_before);
        tx.execute("INSERT INTO push_channels VALUES(?1,?2,1,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12) ON CONFLICT(device) DO UPDATE SET revision=excluded.revision,state=1,provider=excluded.provider,generation=excluded.generation,channel=excluded.channel,expires_at=excluded.expires_at,last_change=excluded.last_change,request_hash=excluded.request_hash,target=excluded.target,proof=excluded.proof,proof_hash=excluded.proof_hash,not_before=excluded.not_before",
            (&device,sql(revision)?,provider,sql(generation)?,&channel,sql(expires)?,sql(now)?,hash.as_slice(),target.as_str(),proof.as_str(),digest(&proof).as_slice(),sql(not_before)?))?;
        tx.execute("INSERT INTO push_jobs VALUES(?1,?2,0,?3,?4,0,NULL,0) ON CONFLICT(device) DO UPDATE SET revision=excluded.revision,through_sequence=0,expires_at=excluded.expires_at,due_at=excluded.due_at,attempts=0,lease=NULL,lease_until=0",(&device,sql(revision)?,sql(expires)?,sql(now)?))?;
        tx.commit()?;
        Ok(Status {
            revision,
            state: State::Pending,
            channel: Some(channel),
            expires_at: Some(expires),
        })
    }
    pub fn confirm_push(
        &mut self,
        credential: &str,
        request: Confirm,
        now: u64,
    ) -> Result<Status, StoreError> {
        bytes(&request.channel)?;
        bytes(&request.proof)?;
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let device = authorize(&tx, credential, now)?;
        let config = push_config::read(&tx)?;
        let row = current(&tx, &config, &device, now)?.ok_or(StoreError::NotFound)?;
        if request.revision != row.revision || row.channel.as_ref() != Some(&request.channel) {
            return Err(StoreError::Conflict);
        }
        if !matches!(row.state, State::Pending | State::Active)
            || row
                .proof_hash
                .as_ref()
                .is_none_or(|p| !bool::from(p.as_slice().ct_eq(digest(&request.proof).as_slice())))
        {
            return Err(StoreError::Forbidden);
        }
        if row.state == State::Active {
            let status = row.status();
            tx.commit()?;
            return Ok(status);
        }
        let expires = deadline(now, CHANNEL_LIFETIME)?;
        tx.execute(
            "UPDATE push_channels SET state=2,proof=NULL,expires_at=?2 WHERE device=?1",
            (&device, sql(expires)?),
        )?;
        tx.execute("DELETE FROM push_jobs WHERE device=?1", [&device])?;
        enqueue(&tx, &device, now)?;
        let status = read(&tx, &device)?.ok_or(StoreError::InvalidData)?.status();
        tx.commit()?;
        Ok(status)
    }
    pub fn disable_push(
        &mut self,
        credential: &str,
        request: Disable,
        now: u64,
    ) -> Result<Status, StoreError> {
        let hash = push_config::hash(b"Sigil/disable-push/v0", &request)?;
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let device = authorize(&tx, credential, now)?;
        let config = push_config::read(&tx)?;
        let previous = current(&tx, &config, &device, now)?;
        let revision = previous.as_ref().map_or(0, |p| p.revision);
        if request.expected_revision != revision {
            if previous.as_ref().is_some_and(|p| {
                request.expected_revision.checked_add(1) == Some(p.revision) && p.hash == hash
            }) {
                let status = previous.unwrap().status();
                tx.commit()?;
                return Ok(status);
            }
            return Err(StoreError::Conflict);
        }
        if previous.is_none() {
            crate::storage_budget::for_device(&tx, &device, SLOT_BYTES, now)?;
        }
        let revision = push_config::next(revision)?;
        tx.execute("INSERT INTO push_channels VALUES(?1,?2,0,0,0,NULL,NULL,?3,?4,NULL,NULL,NULL,0) ON CONFLICT(device) DO UPDATE SET revision=excluded.revision,state=0,channel=NULL,expires_at=NULL,last_change=excluded.last_change,request_hash=excluded.request_hash,target=NULL,proof=NULL,proof_hash=NULL,not_before=0",(&device,sql(revision)?,sql(now)?,hash.as_slice()))?;
        tx.execute("DELETE FROM push_jobs WHERE device=?1", [&device])?;
        tx.commit()?;
        Ok(Status {
            revision,
            state: State::Disabled,
            channel: None,
            expires_at: None,
        })
    }
}
/// Called inside mailbox insertion/confirmation. One job represents all queued mail.
pub(crate) fn enqueue(tx: &Transaction<'_>, device: &str, now: u64) -> Result<(), StoreError> {
    if !tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM push_channels WHERE device=?1 AND state=2)",
        [device],
        |r| r.get::<_, bool>(0),
    )? {
        return Ok(());
    }
    let config = push_config::read(tx)?;
    let Some(row) = current(tx, &config, device, now)? else {
        return Ok(());
    };
    if row.state != State::Active {
        return Ok(());
    }
    let (through,expires):(Option<i64>,Option<u64>)=tx.query_row("SELECT max(sequence),max(expires_at) FROM mailbox WHERE recipient=?1 AND payload IS NOT NULL AND expires_at>?2",(device,sql(now)?),|r|Ok((r.get(0)?,optional_unsigned(r,1)?)))?;
    let (Some(through), Some(expires)) = (through, expires) else {
        return Ok(());
    };
    tx.execute("INSERT INTO push_jobs VALUES(?1,?2,?3,?4,?5,0,NULL,0) ON CONFLICT(device) DO UPDATE SET through_sequence=max(through_sequence,excluded.through_sequence),expires_at=max(expires_at,excluded.expires_at)",(device,sql(row.revision)?,through,sql(expires.min(row.expires.ok_or(StoreError::InvalidData)?))?,sql(now)?))?;
    Ok(())
}
