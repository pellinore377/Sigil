use crate::{
    call_config::{self, Stored},
    prekeys::authorize,
    push_config::{sql, unsigned},
    store::{Store, StoreError},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use sigil_calls::{Id, SignedConnect, SignedRoster};
#[cfg(test)]
#[path = "call_tests.rs"]
mod tests;

pub(crate) fn failure(error: sigil_calls::Error) -> StoreError {
    use sigil_calls::Error::*;
    match error {
        Authentication => StoreError::Forbidden,
        Conflict | Replay => StoreError::Conflict,
        Expired => StoreError::NotFound,
        Limit => StoreError::Busy,
        Invalid => StoreError::Invalid("invalid call proof"),
        Entropy => StoreError::InvalidData,
    }
}
pub(crate) struct Snapshot {
    pub configuration: Stored,
    pub rosters: Vec<SignedRoster>,
    pub now: u64,
}
pub(crate) struct Admission {
    pub snapshot: Snapshot,
    pub fresh: bool,
}
fn clock(db: &Connection, now: u64) -> Result<u64, StoreError> {
    db.execute(
        "UPDATE call_configuration SET clock=?1 WHERE id=1 AND clock<?1",
        [sql(now)?],
    )?;
    Ok(
        db.query_row("SELECT clock FROM call_configuration WHERE id=1", [], |r| {
            unsigned(r, 0)
        })?,
    )
}
fn cleanup(db: &Connection, now: u64) -> Result<(), StoreError> {
    db.execute("DELETE FROM calls WHERE id IN (SELECT id FROM calls WHERE expires<=?1 ORDER BY expires LIMIT 64)",[sql(now)?])?;
    db.execute("UPDATE calls SET closed=1 WHERE closed=0 AND NOT EXISTS(SELECT 1 FROM devices d JOIN accounts a ON a.id=d.account_id WHERE d.id=calls.device AND d.revoked=0 AND d.expires_at>?1 AND a.disabled=0)",[sql(now)?])?;
    db.execute(
        "DELETE FROM call_connections WHERE call IN (SELECT id FROM calls WHERE closed=1)",
        [],
    )?;
    Ok(())
}
fn roster(db: &Connection, id: Id) -> Result<(String, bool, SignedRoster), StoreError> {
    let (device, closed, bytes): (String, bool, Vec<u8>) = db
        .query_row(
            "SELECT device,closed,roster FROM calls WHERE id=?1 AND length(roster)<=16384",
            [id.as_slice()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .ok_or(StoreError::NotFound)?;
    let value = SignedRoster::from_bytes(&bytes).map_err(|_| StoreError::InvalidData)?;
    if value.roster.call != id {
        return Err(StoreError::InvalidData);
    }
    Ok((device, closed, value))
}
fn snapshot(db: &Connection, now: u64) -> Result<Snapshot, StoreError> {
    let configuration = call_config::read(db)?;
    let mut rosters = Vec::new();
    if configuration.settings.is_some() {
        let values:Vec<Vec<u8>>=db.prepare("SELECT roster FROM calls WHERE closed=0 AND expires>?1 AND length(roster)<=16384 ORDER BY id LIMIT 9")?.query_map([sql(now)?],|r|r.get(0))?.collect::<Result<_,_>>()?;
        if values.len() > 8 {
            return Err(StoreError::InvalidData);
        }
        for bytes in values {
            let value = SignedRoster::from_bytes(&bytes).map_err(|_| StoreError::InvalidData)?;
            value
                .roster
                .active(now)
                .map_err(|_| StoreError::InvalidData)?;
            rosters.push(value);
        }
    }
    Ok(Snapshot {
        configuration,
        rosters,
        now,
    })
}
impl Store {
    pub(crate) fn advance_call(
        &mut self,
        value: SignedRoster,
        now: u64,
    ) -> Result<SignedRoster, StoreError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = clock(&tx, now)?;
        cleanup(&tx, now)?;
        call_config::read(&tx)?
            .settings
            .ok_or(StoreError::NotFound)?;
        value.verify().map_err(failure)?;
        if value.roster.version != 2 || value.roster.revision == 0 {
            return Err(StoreError::Forbidden);
        }
        let (_, closed, old) = roster(&tx, value.roster.call)?;
        if old == value && closed == value.roster.closed {
            tx.commit()?;
            return Ok(value);
        }
        if closed {
            return Err(StoreError::Conflict);
        }
        old.successor(&value, true).map_err(failure)?;
        if now >= value.roster.expires {
            return Err(StoreError::NotFound);
        }
        tx.execute(
            "UPDATE calls SET roster=?1,closed=?2 WHERE id=?3",
            (
                value.to_bytes().map_err(failure)?,
                value.roster.closed,
                value.roster.call.as_slice(),
            ),
        )?;
        tx.execute(
            "DELETE FROM call_connections WHERE call=?1",
            [value.roster.call.as_slice()],
        )?;
        tx.commit()?;
        Ok(value)
    }
    pub(crate) fn call_relay(
        &mut self,
        proof: &sigil_calls::RelayRequest,
        now: u64,
    ) -> Result<(Stored, u64, u64), StoreError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = clock(&tx, now)?;
        cleanup(&tx, now)?;
        let config = call_config::read(&tx)?;
        if config.settings.is_none() {
            return Err(StoreError::NotFound);
        }
        let (_, closed, roster) = roster(&tx, proof.call)?;
        if closed {
            return Err(StoreError::NotFound);
        }
        proof.verify(&roster.roster, now).map_err(failure)?;
        tx.commit()?;
        Ok((config, roster.roster.expires, now))
    }
    pub fn publish_call(
        &mut self,
        credential: &str,
        value: SignedRoster,
        now: u64,
    ) -> Result<SignedRoster, StoreError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = clock(&tx, now)?;
        let device = authorize(&tx, credential, now)?;
        value.verify().map_err(failure)?;
        cleanup(&tx, now)?;
        let config = call_config::read(&tx)?;
        let settings = config.settings.ok_or(StoreError::NotFound)?;
        let host = crate::store::read_configuration(&tx)?
            .settings
            .ok_or(StoreError::NotFound)?
            .server_name;
        if value.roster.server != host {
            return Err(StoreError::Invalid("call is hosted elsewhere"));
        }
        let id = value.roster.call;
        match roster(&tx, id) {
            Ok((old_device, closed, old)) => {
                if old_device != device {
                    return Err(StoreError::Forbidden);
                }
                if old == value && closed == value.roster.closed {
                    tx.commit()?;
                    return Ok(value);
                }
                if closed {
                    return Err(StoreError::Conflict);
                }
                old.successor(&value, true).map_err(failure)?;
                if now >= value.roster.expires {
                    return Err(StoreError::NotFound);
                }
                tx.execute(
                    "UPDATE calls SET roster=?1,closed=?2 WHERE id=?3",
                    (
                        value.to_bytes().map_err(failure)?,
                        value.roster.closed,
                        id.as_slice(),
                    ),
                )?;
                tx.execute(
                    "DELETE FROM call_connections WHERE call=?1",
                    [id.as_slice()],
                )?;
            }
            Err(StoreError::NotFound) => {
                value.roster.active(now).map_err(failure)?;
                if value.roster.revision != 0 || now.saturating_sub(value.roster.created) > 60 {
                    return Err(StoreError::Conflict);
                }
                let (live, total): (u64, u64) = tx.query_row(
                    "SELECT count(*) FILTER(WHERE closed=0 AND expires>?1),count(*) FROM calls",
                    [sql(now)?],
                    |r| Ok((unsigned(r, 0)?, unsigned(r, 1)?)),
                )?;
                let (own_live,own_total):(u64,u64)=tx.query_row("SELECT count(*) FILTER(WHERE c.closed=0 AND c.expires>?1),count(*) FROM calls c JOIN devices d ON d.id=c.device WHERE d.account_id=(SELECT account_id FROM devices WHERE id=?2)",(sql(now)?,&device),|r|Ok((unsigned(r,0)?,unsigned(r,1)?)))?;
                if live >= u64::from(settings.max_calls)
                    || total >= 4096
                    || own_live >= 2
                    || own_total >= 256
                {
                    return Err(StoreError::Busy);
                }
                tx.execute(
                    "INSERT INTO calls VALUES(?1,?2,?3,0,?4)",
                    (
                        id.as_slice(),
                        device,
                        sql(value.roster.expires)?,
                        value.to_bytes().map_err(failure)?,
                    ),
                )?;
            }
            Err(error) => return Err(error),
        }
        tx.commit()?;
        Ok(value)
    }
    pub(crate) fn call_snapshot(&mut self, now: u64) -> Result<Snapshot, StoreError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = clock(&tx, now)?;
        cleanup(&tx, now)?;
        let value = snapshot(&tx, now)?;
        tx.commit()?;
        Ok(value)
    }
    pub(crate) fn admit_call(
        &mut self,
        proof: &SignedConnect,
        now: u64,
    ) -> Result<Admission, StoreError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = clock(&tx, now)?;
        cleanup(&tx, now)?;
        let configuration = call_config::read(&tx)?;
        configuration.settings.ok_or(StoreError::NotFound)?;
        let (_, closed, roster) = roster(&tx, proof.request.call)?;
        if closed {
            return Err(StoreError::NotFound);
        }
        proof.verify(&roster.roster, now).map_err(failure)?;
        let digest = proof.request.digest().map_err(failure)?;
        let old:Option<(u64,Vec<u8>,u64)>=tx.query_row("SELECT sequence,digest,updated FROM call_connections WHERE call=?1 AND participant=?2",(proof.request.call.as_slice(),proof.request.participant.as_slice()),|r|Ok((unsigned(r,0)?,r.get(1)?,unsigned(r,2)?))).optional()?;
        let fresh = if let Some((sequence, hash, updated)) = old {
            if sequence == proof.request.sequence && hash == digest {
                false
            } else {
                if sequence >= proof.request.sequence {
                    return Err(StoreError::Conflict);
                }
                if now <= updated {
                    return Err(StoreError::Busy);
                }
                true
            }
        } else {
            true
        };
        if fresh {
            tx.execute("INSERT INTO call_connections VALUES(?1,?2,?3,?4,?5) ON CONFLICT(call,participant) DO UPDATE SET sequence=excluded.sequence,digest=excluded.digest,updated=excluded.updated",(proof.request.call.as_slice(),proof.request.participant.as_slice(),sql(proof.request.sequence)?,digest.as_slice(),sql(now)?))?;
        }
        let snapshot = snapshot(&tx, now)?;
        tx.commit()?;
        Ok(Admission { snapshot, fresh })
    }
}
