//! Storage reservations for retained protocol evidence, independent of live queues.
//! Fixed reservations include row/index overhead; variable proof/bundle bytes are
//! charged separately. No replay identifiers are discarded to reclaim a quota.
use crate::store::{read_configuration, StoreError};
use rusqlite::{Connection, Transaction};

pub(crate) const MAILBOX: u64 = 512;
pub(crate) const PREKEY: u64 = 512;
pub(crate) const DEVICE: u64 = 2048;
pub(crate) const LINK: u64 = 512;
pub(crate) const CANCELLATION: u64 = 256;
pub(crate) const MIGRATION: &str = "
CREATE TABLE retained_storage(account_id TEXT PRIMARY KEY REFERENCES accounts(id),bytes INTEGER NOT NULL CHECK(bytes>=0));
CREATE INDEX mailbox_live_recipient ON mailbox(recipient,sender,sequence) WHERE payload IS NOT NULL;
CREATE INDEX prekeys_available ON prekeys(device_id,expires_at) WHERE bundle IS NOT NULL AND claimant IS NULL;
CREATE INDEX devices_active_account ON devices(account_id,expires_at) WHERE revoked=0;
";

pub(crate) fn rebuild(db: &Connection) -> Result<(), StoreError> {
    db.execute_batch("DELETE FROM retained_storage;
        INSERT INTO retained_storage SELECT a.id,
        (SELECT count(*)*512 FROM mailbox m JOIN devices d ON d.id=m.sender WHERE d.account_id=a.id) +
        (SELECT coalesce(sum(512+coalesce(length(p.bundle),0)),0) FROM prekeys p JOIN devices d ON d.id=p.device_id WHERE d.account_id=a.id) +
        (SELECT count(*)*2048 FROM devices d WHERE d.account_id=a.id) +
        (SELECT coalesce(sum(512+length(l.proof)),0) FROM device_links l JOIN devices d ON d.id=l.sponsor WHERE d.account_id=a.id) +
        (SELECT count(*)*256 FROM cancelled_device_links l JOIN devices d ON d.id=l.sponsor WHERE d.account_id=a.id)
        FROM accounts a;")?;
    // Schema 12 migration also rebuilds historical stores before attachments exist.
    if db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='attachments')",
        [],
        |r| r.get::<_, bool>(0),
    )? {
        db.execute("UPDATE retained_storage SET bytes=bytes+(SELECT count(*) FROM attachments WHERE account_id=retained_storage.account_id)*?1", [crate::attachments::FILE_METADATA as i64])?;
    }
    if db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='push_channels')",
        [],
        |r| r.get::<_, bool>(0),
    )? {
        db.execute("UPDATE retained_storage SET bytes=bytes+(SELECT count(*) FROM push_channels c JOIN devices d ON d.id=c.device WHERE d.account_id=retained_storage.account_id)*?1", [crate::push::SLOT_BYTES as i64])?;
    }
    if db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='federation_senders')",[],|r|r.get::<_,bool>(0))?{
        db.execute("UPDATE retained_storage SET bytes=bytes+(SELECT count(*) FROM federation_revocations r JOIN devices d ON d.id=r.recipient WHERE d.account_id=retained_storage.account_id)*512+(SELECT count(*) FROM federation_senders s JOIN devices d ON d.id=s.recipient WHERE s.grant_id IS NOT NULL AND d.account_id=retained_storage.account_id)*512+(SELECT count(*) FROM federation_senders s JOIN devices d ON d.id=s.recipient WHERE d.account_id=retained_storage.account_id)*?1+(SELECT count(*) FROM mailbox m JOIN devices d ON d.id=m.recipient WHERE m.remote_server IS NOT NULL AND d.account_id=retained_storage.account_id)*?1",[crate::federation_mailbox::METADATA as i64])?;
    }
    if db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='federation_outbox')",[],|r|r.get::<_,bool>(0))?{
        db.execute("UPDATE retained_storage SET bytes=bytes+(SELECT coalesce(sum(?1+coalesce(length(o.body),0)),0) FROM federation_outbox o JOIN devices d ON d.id=o.sender WHERE d.account_id=retained_storage.account_id)",[crate::federation_outbox::METADATA as i64])?;
    }
    if db.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('prekeys') WHERE name='remote_server')",
        [],
        |r| r.get::<_, bool>(0),
    )? {
        db.execute("UPDATE retained_storage SET bytes=bytes+(SELECT count(*) FROM prekeys p JOIN devices d ON d.id=p.device_id WHERE p.remote_server IS NOT NULL AND d.account_id=retained_storage.account_id)*?1",[crate::federation_mailbox::METADATA as i64])?;
    }
    Ok(())
}

pub(crate) fn reserve(
    tx: &Transaction<'_>,
    account: &str,
    bytes: u64,
    now: u64,
) -> Result<(), StoreError> {
    let quota = read_configuration(tx)?
        .settings
        .ok_or(StoreError::Unauthorized)?
        .default_quota_bytes;
    if crate::recovery::used(tx, account, now)?
        .checked_add(bytes)
        .is_none_or(|used| used > quota)
    {
        return Err(StoreError::Busy);
    }
    let bytes = i64::try_from(bytes).map_err(|_| StoreError::InvalidData)?;
    tx.execute("INSERT INTO retained_storage VALUES(?1,?2) ON CONFLICT(account_id) DO UPDATE SET bytes=bytes+excluded.bytes", (account, bytes))?;
    Ok(())
}

pub(crate) fn for_device(
    tx: &Transaction<'_>,
    device: &str,
    bytes: u64,
    now: u64,
) -> Result<(), StoreError> {
    let account: String = tx.query_row(
        "SELECT account_id FROM devices WHERE id=?1",
        [device],
        |r| r.get(0),
    )?;
    reserve(tx, &account, bytes, now)
}
