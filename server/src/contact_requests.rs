use crate::{
    federation_auth::hex,
    prekeys::authorize,
    push_config::{sql, unsigned},
    store::{Store, StoreError},
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use sha2::{Digest, Sha256};
use sigil_protocol::{
    accounts::valid_credential,
    contacts::*,
    device::{SignedBinding, Statement},
};
#[cfg(test)]
#[path = "contact_request_tests.rs"]
pub(crate) mod tests;

pub(crate) const BYTES: u64 = 2048;
pub(crate) const MIGRATION: &str = "
CREATE TABLE contact_requests(id TEXT PRIMARY KEY,recipient TEXT NOT NULL REFERENCES accounts(id),origin TEXT NOT NULL,account TEXT NOT NULL,device TEXT NOT NULL,binding BLOB NOT NULL,signature TEXT NOT NULL,created_at INTEGER NOT NULL,expires_at INTEGER NOT NULL,state INTEGER NOT NULL CHECK(state BETWEEN 0 AND 3));
CREATE INDEX contact_requests_recipient ON contact_requests(recipient,state,id);
CREATE INDEX contact_requests_sender ON contact_requests(origin,account,state);
CREATE INDEX contact_requests_expiry ON contact_requests(state,expires_at);
CREATE TABLE contact_request_policy(account TEXT PRIMARY KEY REFERENCES accounts(id),enabled INTEGER NOT NULL CHECK(enabled IN(0,1)));
";
fn identifier(origin: &str, account: &str, recipient: &str) -> String {
    hex(&Sha256::digest(
        [
            b"Sigil/contact-request-id/v1\0".as_slice(),
            &(origin.len() as u16).to_be_bytes(),
            origin.as_bytes(),
            account.as_bytes(),
            recipient.as_bytes(),
        ]
        .concat(),
    ))
}
fn own(db: &Connection, credential: &str, now: u64) -> Result<(String, String), StoreError> {
    let device = authorize(db, credential, now)?;
    let account = db.query_row(
        "SELECT account_id FROM devices WHERE id=?1",
        [&device],
        |r| r.get(0),
    )?;
    Ok((account, device))
}
fn enabled(db: &Connection, account: &str) -> Result<bool, StoreError> {
    Ok(db
        .query_row(
            "SELECT enabled FROM contact_request_policy WHERE account=?1",
            [account],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(true))
}
fn state(value: i64) -> Result<RequestState, StoreError> {
    match value {
        0 => Ok(RequestState::Pending),
        1 => Ok(RequestState::Accepted),
        2 | 3 => Ok(RequestState::Declined),
        _ => Err(StoreError::InvalidData),
    }
}
fn receipt(db: &Connection, id: &str) -> Result<RequestReceipt, StoreError> {
    let (status, expires_at) = db
        .query_row(
            "SELECT state,expires_at FROM contact_requests WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, unsigned(r, 1)?)),
        )
        .optional()?
        .ok_or(StoreError::NotFound)?;
    Ok(RequestReceipt {
        id: id.into(),
        state: state(status)?,
        expires_at,
    })
}
fn remove(tx: &Transaction<'_>, id: &str, recipient: &str) -> Result<(), StoreError> {
    if tx.execute(
        "DELETE FROM contact_requests WHERE id=?1 AND recipient=?2",
        (id, recipient),
    )? != 1
    {
        return Err(StoreError::Conflict);
    }
    if tx.execute(
        "UPDATE retained_storage SET bytes=bytes-?2 WHERE account_id=?1 AND bytes>=?2",
        (recipient, BYTES as i64),
    )? != 1
    {
        return Err(StoreError::InvalidData);
    }
    Ok(())
}
pub(crate) fn cleanup(tx: &Transaction<'_>, now: u64) -> Result<usize, StoreError> {
    let expired = tx.prepare("SELECT id,recipient FROM contact_requests WHERE state<3 AND expires_at<=?1 ORDER BY expires_at,id LIMIT 64")?.query_map([sql(now)?],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<Result<Vec<_>,_>>()?;
    let changed = expired.len();
    for (id, recipient) in expired {
        remove(tx, &id, &recipient)?;
    }
    Ok(changed)
}
pub(crate) fn request_in(
    tx: &Transaction<'_>,
    origin: &str,
    account: &str,
    device: &str,
    request: &RequestContact,
    now: u64,
) -> Result<RequestReceipt, StoreError> {
    if !request.valid()
        || request.expires_at <= now
        || request.expires_at > now.saturating_add(604800)
    {
        return Err(StoreError::Invalid("invalid contact request"));
    }
    let home = crate::store::read_configuration(tx)?
        .settings
        .ok_or(StoreError::Unauthorized)?
        .server_name;
    if request.server != home || (origin == home && account == request.recipient) {
        return Err(StoreError::Forbidden);
    }
    let raw = Statement {
        statement: request.binding.clone(),
    }
    .bytes()
    .map_err(StoreError::Invalid)?;
    let signed = SignedBinding::from_bytes(&raw).map_err(StoreError::Invalid)?;
    if signed.binding.server != origin
        || hex(&signed.binding.account) != account
        || hex(&signed.binding.device) != device
    {
        return Err(StoreError::Forbidden);
    }
    sigil_crypto::verify_signature(
        &signed.binding.identity,
        &signed
            .binding
            .signing_bytes()
            .map_err(StoreError::Invalid)?,
        &signed.signature,
    )
    .map_err(|_| StoreError::Forbidden)?;
    sigil_crypto::verify_signature(
        &signed.binding.identity,
        &request.signing_bytes().map_err(StoreError::Invalid)?,
        &request.signature_bytes().map_err(StoreError::Invalid)?,
    )
    .map_err(|_| StoreError::Forbidden)?;
    if !tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM accounts WHERE id=?1 AND disabled=0)",
        [&request.recipient],
        |r| r.get::<_, bool>(0),
    )? || !enabled(tx, &request.recipient)?
    {
        return Err(StoreError::Forbidden);
    }
    cleanup(tx, now)?;
    let id = identifier(origin, account, &request.recipient);
    if tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM contact_requests WHERE id=?1 AND state<3 AND expires_at<=?2)",
        (&id, sql(now)?),
        |r| r.get::<_, bool>(0),
    )? {
        remove(tx, &id, &request.recipient)?;
    }
    match receipt(tx, &id) {
        Ok(value) => return Ok(value),
        Err(StoreError::NotFound) => (),
        Err(e) => return Err(e),
    }
    let (total,incoming,pending,outgoing,remote): (u64,u64,u64,u64,u64) = tx.query_row("SELECT (SELECT count(*) FROM contact_requests),(SELECT count(*) FROM contact_requests WHERE recipient=?1),(SELECT count(*) FROM contact_requests WHERE recipient=?1 AND state=0),(SELECT count(*) FROM contact_requests WHERE origin=?2 AND account=?3 AND state=0),(SELECT count(*) FROM contact_requests WHERE origin=?2 AND state=0)",(&request.recipient,origin,account),|r|Ok((unsigned(r,0)?,unsigned(r,1)?,unsigned(r,2)?,unsigned(r,3)?,unsigned(r,4)?)))?;
    if total >= 65536
        || incoming >= 4096
        || pending >= 64
        || outgoing >= 32
        || (origin != home && remote >= 128)
    {
        return Err(StoreError::Busy);
    }
    crate::storage_budget::reserve(tx, &request.recipient, BYTES, now)?;
    tx.execute(
        "INSERT INTO contact_requests VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,0)",
        (
            &id,
            &request.recipient,
            origin,
            account,
            device,
            raw,
            &request.signature,
            sql(now)?,
            sql(request.expires_at)?,
        ),
    )?;
    receipt(tx, &id)
}
pub(crate) fn status_in(
    tx: &Transaction<'_>,
    origin: &str,
    account: &str,
    recipient: &str,
    now: u64,
) -> Result<RequestReceipt, StoreError> {
    if !valid_credential(recipient) {
        return Err(StoreError::Invalid("invalid recipient"));
    }
    let id = identifier(origin, account, recipient);
    if !tx.query_row("SELECT EXISTS(SELECT 1 FROM contact_requests WHERE id=?1 AND origin=?2 AND account=?3 AND (expires_at>?4 OR state=3))",(&id,origin,account,sql(now)?),|r|r.get::<_,bool>(0))? { return Err(StoreError::NotFound); }
    receipt(tx, &id)
}
impl Store {
    pub fn request_contact(
        &mut self,
        credential: &str,
        request: RequestContact,
        now: u64,
    ) -> Result<RequestReceipt, StoreError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (account, device) = own(&tx, credential, now)?;
        let home = crate::store::read_configuration(&tx)?
            .settings
            .ok_or(StoreError::Unauthorized)?
            .server_name;
        let published: Vec<u8> = tx
            .query_row(
                "SELECT statement FROM device_bindings WHERE device=?1",
                [&device],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(StoreError::NotFound)?;
        if hex(&published) != request.binding {
            return Err(StoreError::Forbidden);
        }
        let value = request_in(&tx, &home, &account, &device, &request, now)?;
        tx.commit()?;
        Ok(value)
    }
    pub fn contact_request_status(
        &mut self,
        credential: &str,
        recipient: &str,
        now: u64,
    ) -> Result<RequestReceipt, StoreError> {
        let tx = self.0.transaction()?;
        let (account, _) = own(&tx, credential, now)?;
        let home = crate::store::read_configuration(&tx)?
            .settings
            .ok_or(StoreError::Unauthorized)?
            .server_name;
        let value = status_in(&tx, &home, &account, recipient, now)?;
        tx.commit()?;
        Ok(value)
    }
    pub fn contact_requests(
        &mut self,
        credential: &str,
        after: Option<&str>,
        now: u64,
    ) -> Result<RequestPage, StoreError> {
        if after.is_some_and(|v| !valid_credential(v)) {
            return Err(StoreError::Invalid("invalid request cursor"));
        }
        let tx = self.0.transaction()?;
        let (account, _) = own(&tx, credential, now)?;
        let rows=tx.prepare("SELECT id,origin,account,device,binding,signature,created_at,expires_at FROM contact_requests WHERE recipient=?1 AND state=0 AND expires_at>?2 AND id>?3 ORDER BY id LIMIT 33")?.query_map((&account,sql(now)?,after.unwrap_or("")),|r|Ok(IncomingRequest{receipt:RequestReceipt{id:r.get(0)?,state:RequestState::Pending,expires_at:unsigned(r,7)?},origin:r.get(1)?,account:r.get(2)?,device:r.get(3)?,binding:hex(&r.get::<_,Vec<u8>>(4)?),signature:r.get(5)?,created_at:unsigned(r,6)?}))?.collect::<Result<Vec<_>,_>>()?;
        let next = if rows.len() > 32 {
            Some(rows[31].receipt.id.clone())
        } else {
            None
        };
        tx.commit()?;
        Ok(RequestPage {
            requests: rows.into_iter().take(32).collect(),
            next,
        })
    }
    pub fn incoming_contact_status(
        &self,
        credential: &str,
        id: &str,
        signature: &str,
        now: u64,
    ) -> Result<RequestReceipt, StoreError> {
        if !valid_credential(id)
            || signature.len() != 128
            || !signature
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(StoreError::Invalid("invalid request"));
        }
        let (account, _) = own(&self.0, credential, now)?;
        if !self.0.query_row("SELECT EXISTS(SELECT 1 FROM contact_requests WHERE id=?1 AND recipient=?2 AND signature=?3 AND (expires_at>?4 OR state=3))",(id,&account,signature,sql(now)?),|r|r.get::<_,bool>(0))? { return Err(StoreError::NotFound); }
        receipt(&self.0, id)
    }
    pub fn resolve_contact_request(
        &mut self,
        credential: &str,
        id: &str,
        status: RequestState,
        signature: &str,
        now: u64,
    ) -> Result<RequestReceipt, StoreError> {
        if !valid_credential(id)
            || status == RequestState::Pending
            || signature.len() != 128
            || !signature
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(StoreError::Invalid("invalid request decision"));
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (account, _) = own(&tx, credential, now)?;
        let old: i64 = tx.query_row("SELECT state FROM contact_requests WHERE id=?1 AND recipient=?2 AND (expires_at>?3 OR state=3) AND signature=?4",(id,&account,sql(now)?,signature),|r|r.get(0)).optional()?.ok_or(StoreError::NotFound)?;
        let value = match status {
            RequestState::Accepted => 1,
            RequestState::Declined => 2,
            RequestState::Blocked => 3,
            _ => unreachable!(),
        };
        if old != 0 && old != value && value != 3 {
            return Err(StoreError::Conflict);
        }
        tx.execute(
            "UPDATE contact_requests SET state=?2 WHERE id=?1",
            (id, value),
        )?;
        let value = receipt(&tx, id)?;
        tx.commit()?;
        Ok(value)
    }
    pub fn contact_request_policy(
        &self,
        credential: &str,
        now: u64,
    ) -> Result<RequestPolicy, StoreError> {
        let (account, _) = own(&self.0, credential, now)?;
        Ok(RequestPolicy {
            enabled: enabled(&self.0, &account)?,
        })
    }
    pub fn set_contact_request_policy(
        &mut self,
        credential: &str,
        value: RequestPolicy,
        now: u64,
    ) -> Result<RequestPolicy, StoreError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (account, _) = own(&tx, credential, now)?;
        tx.execute("INSERT INTO contact_request_policy VALUES(?1,?2) ON CONFLICT(account) DO UPDATE SET enabled=excluded.enabled",(&account,value.enabled))?;
        tx.commit()?;
        Ok(value)
    }
}

pub(crate) fn delete_account(tx: &Transaction<'_>, account: &str) -> Result<(), StoreError> {
    let home = crate::store::read_configuration(tx)?
        .settings
        .ok_or(StoreError::Unauthorized)?
        .server_name;
    let rows=tx.prepare("SELECT id,recipient FROM contact_requests WHERE recipient=?1 OR (origin=?2 AND account=?1)")?.query_map((account,&home),|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<Result<Vec<_>,_>>()?;
    for (id, recipient) in rows {
        remove(tx, &id, &recipient)?;
    }
    tx.execute(
        "DELETE FROM contact_request_policy WHERE account=?1",
        [account],
    )?;
    Ok(())
}
impl Store {
    pub fn block_contact_requests(
        &mut self,
        credential: &str,
        request: BlockContact,
        now: u64,
    ) -> Result<(), StoreError> {
        if !sigil_protocol::valid_server_name(&request.server)
            || !valid_credential(&request.account)
        {
            return Err(StoreError::Invalid("invalid contact"));
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (account, _) = own(&tx, credential, now)?;
        let home = crate::store::read_configuration(&tx)?
            .settings
            .ok_or(StoreError::Unauthorized)?
            .server_name;
        if request.server == home && request.account == account {
            return Err(StoreError::Invalid("cannot block yourself"));
        }
        cleanup(&tx, now)?;
        let id = identifier(&request.server, &request.account, &account);
        let old: Option<i64> = tx
            .query_row(
                "SELECT state FROM contact_requests WHERE id=?1",
                [&id],
                |r| r.get(0),
            )
            .optional()?;
        if request.blocked {
            if old.is_none() {
                let (total,own):(u64,u64)=tx.query_row("SELECT (SELECT count(*) FROM contact_requests),(SELECT count(*) FROM contact_requests WHERE recipient=?1)",[&account],|r|Ok((unsigned(r,0)?,unsigned(r,1)?)))?;
                if total >= 65536 || own >= 4096 {
                    return Err(StoreError::Busy);
                }
                crate::storage_budget::reserve(&tx, &account, BYTES, now)?;
                tx.execute(
                    "INSERT INTO contact_requests VALUES(?1,?2,?3,?4,'',X'','',?5,?5,3)",
                    (&id, &account, &request.server, &request.account, sql(now)?),
                )?;
            } else {
                tx.execute("UPDATE contact_requests SET state=3 WHERE id=?1", [&id])?;
            }
        } else if old == Some(3) {
            remove(&tx, &id, &account)?;
        }
        tx.commit()?;
        Ok(())
    }
}
