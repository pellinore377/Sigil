use crate::{
    push_config::{sql, unsigned},
    store::{Store, StoreError},
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use sha2::{Digest, Sha256};
use sigil_protocol::profile::{ContactProfile, Photo, SetPhoto, ShareProfile, MAX_PHOTO};

pub(crate) const RESERVATION: u64 = 256;
#[cfg(test)]
#[path = "profile_photo_tests.rs"]
mod tests;
pub(crate) const MIGRATION:&str="CREATE TABLE profile_photos(account TEXT PRIMARY KEY REFERENCES accounts(id),revision INTEGER NOT NULL,image BLOB NOT NULL); CREATE TABLE profile_shares(owner TEXT NOT NULL REFERENCES accounts(id),origin TEXT NOT NULL,account TEXT NOT NULL,PRIMARY KEY(owner,origin,account)); CREATE INDEX profile_shares_contact ON profile_shares(origin,account);";
fn account(db: &Connection, token: &str, now: u64) -> Result<String, StoreError> {
    let device = crate::prekeys::authorize(db, token, now)?;
    Ok(db.query_row(
        "SELECT account_id FROM devices WHERE id=?1",
        [device],
        |r| r.get(0),
    )?)
}
fn raw(db: &Connection, account: &str) -> Result<(u64, Vec<u8>), StoreError> {
    let value: (u64, Vec<u8>) = db
        .query_row(
            "SELECT revision,image FROM profile_photos WHERE account=?1",
            [account],
            |r| Ok((unsigned(r, 0)?, r.get(1)?)),
        )
        .optional()?
        .unwrap_or_default();
    if value.1.len() > MAX_PHOTO {
        return Err(StoreError::InvalidData);
    }
    Ok(value)
}
fn describe(revision: u64, image: &[u8]) -> Photo {
    Photo {
        revision,
        hash: (!image.is_empty()).then(|| crate::federation_auth::hex(&Sha256::digest(image))),
        bytes: image.len() as u32,
    }
}
fn refund(tx: &Transaction<'_>, account: &str, bytes: u64) -> Result<(), StoreError> {
    if tx.execute(
        "UPDATE retained_storage SET bytes=bytes-?2 WHERE account_id=?1 AND bytes>=?2",
        (account, sql(bytes)?),
    )? != 1
    {
        return Err(StoreError::InvalidData);
    }
    Ok(())
}
fn authorize(db: &Connection, owner: &str, origin: &str, account: &str) -> Result<(), StoreError> {
    if !sigil_protocol::accounts::valid_credential(owner)
        || !sigil_protocol::accounts::valid_credential(account)
        || !sigil_protocol::valid_server_name(origin)
    {
        return Err(StoreError::Invalid("invalid profile address"));
    }
    let home = crate::store::read_configuration(db)?
        .settings
        .ok_or(StoreError::Unauthorized)?
        .server_name;
    if !db.query_row(
        "SELECT EXISTS(SELECT 1 FROM accounts WHERE id=?1 AND disabled=0)",
        [owner],
        |r| r.get::<_, bool>(0),
    )? {
        return Err(StoreError::NotFound);
    }
    if !(owner==account && origin==home) && !db.query_row("SELECT EXISTS(SELECT 1 FROM profile_shares WHERE owner=?1 AND origin=?2 AND account=?3)",(owner,origin,account),|r|r.get::<_,bool>(0))?{return Err(StoreError::Forbidden);}
    Ok(())
}
pub(crate) fn contact(
    db: &Connection,
    owner: &str,
    origin: &str,
    account: &str,
) -> Result<ContactProfile, StoreError> {
    authorize(db, owner, origin, account)?;
    let (revision, image) = raw(db, owner)?;
    Ok(ContactProfile {
        profile: crate::profile::read(db, owner)?,
        photo: describe(revision, &image),
    })
}
pub(crate) fn image(
    db: &Connection,
    owner: &str,
    origin: &str,
    account: &str,
    hash: &str,
) -> Result<Vec<u8>, StoreError> {
    if !sigil_protocol::accounts::valid_credential(hash) {
        return Err(StoreError::Invalid("invalid photo hash"));
    }
    authorize(db, owner, origin, account)?;
    let (revision, image) = raw(db, owner)?;
    if describe(revision, &image).hash.as_deref() != Some(hash) {
        return Err(StoreError::NotFound);
    }
    Ok(image)
}
pub(crate) fn unshare(
    tx: &Transaction<'_>,
    owner: &str,
    origin: &str,
    account: &str,
) -> Result<(), StoreError> {
    if tx.execute(
        "DELETE FROM profile_shares WHERE owner=?1 AND origin=?2 AND account=?3",
        (owner, origin, account),
    )? != 0
    {
        refund(tx, owner, RESERVATION)?;
    }
    Ok(())
}
pub(crate) fn delete_account(tx: &Transaction<'_>, account: &str) -> Result<(), StoreError> {
    let home = crate::store::read_configuration(tx)?
        .settings
        .ok_or(StoreError::Unauthorized)?
        .server_name;
    let rows=tx.prepare("SELECT owner,origin,account FROM profile_shares WHERE owner=?1 OR (origin=?2 AND account=?1)")?.query_map((account,&home),|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?.collect::<Result<Vec<_>,_>>()?;
    for (owner, origin, peer) in rows {
        unshare(tx, &owner, &origin, &peer)?;
    }
    let (revision, image) = raw(tx, account)?;
    if revision > 0 {
        tx.execute("DELETE FROM profile_photos WHERE account=?1", [account])?;
        refund(tx, account, RESERVATION + image.len() as u64)?;
    }
    Ok(())
}
impl Store {
    pub fn profile_photo(&self, token: &str, now: u64) -> Result<Photo, StoreError> {
        let account = self.session(token, now)?.account_id;
        let (revision, image) = raw(&self.0, &account)?;
        Ok(describe(revision, &image))
    }
    pub fn set_profile_photo(
        &mut self,
        token: &str,
        request: SetPhoto,
        now: u64,
    ) -> Result<Photo, StoreError> {
        let image =
            sigil_protocol::profile::photo_bytes(&request.photo).map_err(StoreError::Invalid)?;
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let account = account(&tx, token, now)?;
        let (revision, old) = raw(&tx, &account)?;
        if revision != request.revision {
            if request.revision.checked_add(1) == Some(revision) && old == image {
                return Ok(describe(revision, &old));
            }
            return Err(StoreError::Conflict);
        }
        let next = crate::push_config::next(revision)?;
        let previous = if revision == 0 {
            0
        } else {
            RESERVATION + old.len() as u64
        };
        let current = RESERVATION + image.len() as u64;
        if current > previous {
            crate::storage_budget::reserve(&tx, &account, current - previous, now)?;
        } else if previous > current {
            refund(&tx, &account, previous - current)?;
        }
        tx.execute("INSERT INTO profile_photos VALUES(?1,?2,?3) ON CONFLICT(account) DO UPDATE SET revision=excluded.revision,image=excluded.image",(&account,sql(next)?,&image))?;
        tx.commit()?;
        Ok(describe(next, &image))
    }
    pub fn share_profile(
        &mut self,
        token: &str,
        request: ShareProfile,
        now: u64,
    ) -> Result<(), StoreError> {
        if !sigil_protocol::valid_server_name(&request.server)
            || !sigil_protocol::accounts::valid_credential(&request.account)
        {
            return Err(StoreError::Invalid("invalid contact"));
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let owner = account(&tx, token, now)?;
        let home = crate::store::read_configuration(&tx)?
            .settings
            .ok_or(StoreError::Unauthorized)?
            .server_name;
        if request.server == home && request.account == owner {
            return Err(StoreError::Invalid(
                "your profile is already available to you",
            ));
        }
        if request.allowed {
            if tx.query_row("SELECT EXISTS(SELECT 1 FROM contact_requests WHERE recipient=?1 AND origin=?2 AND account=?3 AND state=3)",(&owner,&request.server,&request.account),|r|r.get::<_,bool>(0))? { return Err(StoreError::Forbidden); }
            if tx.query_row("SELECT EXISTS(SELECT 1 FROM profile_shares WHERE owner=?1 AND origin=?2 AND account=?3)",(&owner,&request.server,&request.account),|r|r.get::<_,bool>(0))?{return Ok(());}
            if tx.query_row("SELECT (SELECT count(*) FROM profile_shares WHERE owner=?1)>=4096 OR (SELECT count(*) FROM profile_shares)>=65536",[&owner],|r|r.get::<_,bool>(0))?{return Err(StoreError::Busy);}
            crate::storage_budget::reserve(&tx, &owner, RESERVATION, now)?;
            tx.execute(
                "INSERT INTO profile_shares VALUES(?1,?2,?3)",
                (&owner, &request.server, &request.account),
            )?;
        } else {
            unshare(&tx, &owner, &request.server, &request.account)?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn contact_profile(
        &self,
        token: &str,
        owner: &str,
        now: u64,
    ) -> Result<ContactProfile, StoreError> {
        let own = self.session(token, now)?;
        let origin = own
            .address
            .split_once(':')
            .ok_or(StoreError::InvalidData)?
            .1;
        contact(&self.0, owner, origin, &own.account_id)
    }
    pub fn contact_photo(
        &self,
        token: &str,
        owner: &str,
        hash: &str,
        now: u64,
    ) -> Result<Vec<u8>, StoreError> {
        let own = self.session(token, now)?;
        let origin = own
            .address
            .split_once(':')
            .ok_or(StoreError::InvalidData)?
            .1;
        image(&self.0, owner, origin, &own.account_id, hash)
    }
}
