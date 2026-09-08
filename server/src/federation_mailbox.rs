use crate::{
    federation_admission, federation_auth as auth, federation_config,
    prekeys::{active, authorize},
    push_config::{sql, unsigned},
    store::{Store, StoreError},
};
use axum::http::HeaderMap;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use sigil_protocol::{
    accounts::valid_credential,
    federation::{Receipt, RemoteSender, Submit},
    mailbox::MAX_PAYLOAD_HEX,
};
pub(crate) const METADATA: u64 = 2048;
pub(crate) const GLOBAL_BUDGET: u64 = 1024 * 1024 * 1024;
pub(crate) const MIGRATION:&str="
ALTER TABLE mailbox RENAME TO mailbox_previous;
CREATE TABLE mailbox (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
 sender TEXT REFERENCES devices(id), message_id TEXT NOT NULL,
 recipient TEXT NOT NULL REFERENCES devices(id), payload TEXT, payload_hash BLOB NOT NULL,
 expires_at INTEGER NOT NULL,
 remote_server TEXT REFERENCES federation_peers(server),remote_account TEXT,remote_device TEXT,remote_grant TEXT,
 UNIQUE(sender,message_id),
 CHECK((sender IS NOT NULL AND remote_server IS NULL AND remote_account IS NULL AND remote_device IS NULL AND remote_grant IS NULL) OR (sender IS NULL AND remote_server IS NOT NULL AND remote_account IS NOT NULL AND remote_device IS NOT NULL AND remote_grant IS NOT NULL))
);
INSERT INTO mailbox(sequence,sender,message_id,recipient,payload,payload_hash,expires_at) SELECT sequence,sender,message_id,recipient,payload,payload_hash,expires_at FROM mailbox_previous;
UPDATE sqlite_sequence SET seq=max(seq,coalesce((SELECT seq FROM sqlite_sequence WHERE name='mailbox_previous'),0)) WHERE name='mailbox';
DROP TABLE mailbox_previous;
CREATE INDEX mailbox_recipient ON mailbox(recipient,sequence);
CREATE INDEX mailbox_expiry ON mailbox(expires_at) WHERE payload IS NOT NULL;
CREATE INDEX mailbox_live_recipient ON mailbox(recipient,sender,sequence) WHERE payload IS NOT NULL;
CREATE UNIQUE INDEX mailbox_remote_id ON mailbox(remote_server,remote_device,message_id) WHERE remote_server IS NOT NULL;
CREATE INDEX mailbox_remote_live ON mailbox(remote_server,recipient,sequence) WHERE remote_server IS NOT NULL AND payload IS NOT NULL;
CREATE INDEX mailbox_remote_grant ON mailbox(remote_grant,sequence) WHERE remote_grant IS NOT NULL AND payload IS NOT NULL;
CREATE TABLE federation_senders(recipient TEXT NOT NULL REFERENCES devices(id),server TEXT NOT NULL REFERENCES federation_peers(server),account TEXT NOT NULL,device TEXT NOT NULL,grant_id TEXT UNIQUE,revision INTEGER NOT NULL,request_hash BLOB NOT NULL,PRIMARY KEY(recipient,server,device));
CREATE TABLE federation_revocations(grant_id TEXT PRIMARY KEY,recipient TEXT NOT NULL REFERENCES devices(id));
ALTER TABLE federation_admission ADD COLUMN ingress_bytes INTEGER NOT NULL DEFAULT 0 CHECK(ingress_bytes>=0);
ALTER TABLE federation_usage ADD COLUMN ingress_bytes INTEGER NOT NULL DEFAULT 0 CHECK(ingress_bytes>=0);
UPDATE federation_admission SET nonce_bytes=(SELECT count(*)*1024 FROM federation_nonces WHERE server=federation_admission.server);
UPDATE federation_usage SET nonce_bytes=(SELECT coalesce(sum(nonce_bytes),0) FROM federation_admission);
";
fn valid(sender: &RemoteSender) -> bool {
    sigil_protocol::valid_server_name(&sender.server)
        && valid_credential(&sender.account)
        && valid_credential(&sender.device)
}
pub(crate) fn validate(request: &Submit, now: u64) -> Result<(), StoreError> {
    if !valid_credential(&request.sender_account)
        || !valid_credential(&request.sender_device)
        || !valid_credential(&request.recipient_device)
        || !valid_credential(&request.message_id)
        || !(32..=MAX_PAYLOAD_HEX).contains(&request.payload.len())
        || !request.payload.len().is_multiple_of(2)
        || !request
            .payload
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || request.expires_at <= now
        || request.expires_at > now.saturating_add(604800)
        || request.expires_at > i64::MAX as u64
    {
        return Err(StoreError::Invalid(
            "invalid federation ciphertext, identifier or expiry",
        ));
    }
    Ok(())
}
pub(crate) fn release_payload(db: &Transaction<'_>, sequence: i64) -> Result<(), StoreError> {
    let row:Option<(String,u64)>=db.query_row("SELECT remote_server,length(payload) FROM mailbox WHERE sequence=?1 AND remote_server IS NOT NULL AND payload IS NOT NULL",[sequence],|r|Ok((r.get(0)?,unsigned(r,1)?))).optional()?;
    if let Some((server, bytes)) = row {
        if db.execute("UPDATE federation_admission SET ingress_bytes=ingress_bytes-?2 WHERE server=?1 AND ingress_bytes>=?2",(&server,sql(bytes)?))?!=1{return Err(StoreError::InvalidData)}
        if db.execute("UPDATE federation_usage SET ingress_bytes=ingress_bytes-?1 WHERE id=1 AND ingress_bytes>=?1",[sql(bytes)?])?!=1{return Err(StoreError::InvalidData)}
    }
    db.execute(
        "UPDATE mailbox SET payload=NULL WHERE sequence=?1",
        [sequence],
    )?;
    Ok(())
}
impl Store {
    pub fn federated_senders(
        &mut self,
        credential: &str,
        now: u64,
    ) -> Result<Vec<RemoteSender>, StoreError> {
        let tx = self.0.transaction()?;
        let recipient = authorize(&tx, credential, now)?;
        let result=tx.prepare("SELECT server,account,device FROM federation_senders WHERE recipient=?1 AND grant_id IS NOT NULL ORDER BY server,device LIMIT 256")?.query_map([recipient],|r|Ok(RemoteSender{server:r.get(0)?,account:r.get(1)?,device:r.get(2)?}))?.collect::<Result<_,_>>()?;
        tx.commit()?;
        Ok(result)
    }
    pub fn receive_federated_message(
        &mut self,
        body: &[u8],
        headers: &HeaderMap,
        now: u64,
    ) -> Result<Receipt, StoreError> {
        if body.len() > sigil_protocol::federation::MAX_BODY {
            return Err(StoreError::Invalid("federation request too large"));
        }
        let request: Submit = serde_json::from_slice(body)
            .map_err(|_| StoreError::Invalid("invalid federation message"))?;
        validate(&request, now)?;
        if serde_json::to_vec(&request).map_err(|_| StoreError::InvalidData)? != body {
            return Err(StoreError::Invalid("noncanonical federation message"));
        }
        let (origin, _, _) = auth::inspect(headers).map_err(|_| StoreError::Unauthorized)?;
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let hash = federation_admission::admit(
            &tx,
            sigil_protocol::federation::DELIVER_PATH,
            body,
            headers,
            now,
        )?;
        if !active(&tx, &request.recipient_device)? {
            return Err(StoreError::NotFound);
        }
        let grant = grant_for(
            &tx,
            origin,
            &request.sender_account,
            &request.sender_device,
            &request.recipient_device,
        )?;
        let old:Option<(i64,Vec<u8>,u64)>=tx.query_row("SELECT sequence,payload_hash,expires_at FROM mailbox WHERE remote_server=?1 AND remote_device=?2 AND message_id=?3",(origin,&request.sender_device,&request.message_id),|r|Ok((r.get(0)?,r.get(1)?,unsigned(r,2)?))).optional()?;
        if let Some((sequence, previous, expires_at)) = old {
            if previous != auth::bytes32(&hash).map_err(|_| StoreError::InvalidData)? {
                return Err(StoreError::AlreadyExists);
            }
            tx.commit()?;
            return Ok(Receipt {
                request_hash: hash,
                sequence,
                expires_at,
            });
        }
        reserve_ingress(&tx, origin, &request, now)?;
        tx.execute("INSERT INTO mailbox(message_id,recipient,payload,payload_hash,expires_at,remote_server,remote_account,remote_device,remote_grant) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",(&request.message_id,&request.recipient_device,&request.payload,auth::bytes32(&hash).map_err(|_|StoreError::InvalidData)?.as_slice(),sql(request.expires_at)?,origin,&request.sender_account,&request.sender_device,&grant))?;
        let sequence = tx.last_insert_rowid();
        crate::push::enqueue(&tx, &request.recipient_device, now)?;
        tx.commit()?;
        Ok(Receipt {
            request_hash: hash,
            sequence,
            expires_at: request.expires_at,
        })
    }
}
fn reserve_ingress(
    tx: &Transaction<'_>,
    origin: &str,
    request: &Submit,
    now: u64,
) -> Result<(), StoreError> {
    let (pending,peer):(u32,u32)=tx.query_row("SELECT count(*),coalesce(sum(remote_server=?3),0) FROM mailbox WHERE recipient=?1 AND payload IS NOT NULL AND expires_at>?2",(&request.recipient_device,sql(now)?,origin),|r|Ok((r.get(0)?,r.get(1)?)))?;
    if pending >= 256 || peer >= 64 {
        return Err(StoreError::Busy);
    }
    reserve_remote(
        tx,
        origin,
        &request.recipient_device,
        METADATA,
        request.payload.len() as u64,
        now,
    )
}
pub(crate) fn grant_for(
    db: &Connection,
    origin: &str,
    account: &str,
    device: &str,
    target: &str,
) -> Result<String, StoreError> {
    db.query_row("SELECT grant_id FROM federation_senders WHERE recipient=?1 AND server=?2 AND account=?3 AND device=?4 AND grant_id IS NOT NULL",(target,origin,account,device),|r|r.get(0)).optional()?.ok_or(StoreError::Forbidden)
}
pub(crate) fn reserve_remote(
    tx: &Transaction<'_>,
    origin: &str,
    target: &str,
    metadata: u64,
    payload: u64,
    now: u64,
) -> Result<(), StoreError> {
    let config = federation_config::read(tx)?;
    let bytes = metadata
        .checked_add(payload)
        .ok_or(StoreError::InvalidData)?;
    let used: u64 = tx.query_row(
        "SELECT ingress_bytes+nonce_bytes+egress_bytes FROM federation_admission WHERE server=?1",
        [origin],
        |r| unsigned(r, 0),
    )?;
    let global: u64 = tx.query_row(
        "SELECT ingress_bytes FROM federation_usage WHERE id=1",
        [],
        |r| unsigned(r, 0),
    )?;
    if used.saturating_add(bytes) > config.peer_quota_bytes
        || global.saturating_add(bytes) > GLOBAL_BUDGET
    {
        return Err(StoreError::Busy);
    }
    // Retained idempotence evidence belongs to both the admitting origin budget
    // and the local recipient account; no invented remote account evades either.
    crate::storage_budget::for_device(tx, target, metadata, now)?;
    let account: String = tx.query_row(
        "SELECT account_id FROM devices WHERE id=?1",
        [target],
        |r| r.get(0),
    )?;
    let quota = crate::admin::quota(tx, &account)?;
    if crate::recovery::used(tx, &account, now)?.saturating_add(payload) > quota {
        return Err(StoreError::Busy);
    }
    tx.execute(
        "UPDATE federation_admission SET ingress_bytes=ingress_bytes+?2 WHERE server=?1",
        (origin, sql(bytes)?),
    )?;
    tx.execute(
        "UPDATE federation_usage SET ingress_bytes=ingress_bytes+?1 WHERE id=1",
        [sql(bytes)?],
    )?;
    Ok(())
}
pub(crate) fn rebuild(db: &Connection) -> Result<(), StoreError> {
    db.execute_batch("UPDATE federation_admission SET ingress_bytes=(SELECT coalesce(sum(2048+coalesce(length(payload),0)),0) FROM mailbox WHERE remote_server=federation_admission.server)+(SELECT count(*)*2048 FROM prekeys WHERE remote_server=federation_admission.server);UPDATE federation_usage SET ingress_bytes=(SELECT coalesce(sum(ingress_bytes),0) FROM federation_admission);")?;
    Ok(())
}

pub(crate) fn cleanup(db: &Transaction<'_>) -> Result<usize, StoreError> {
    let job: Option<(String, String)> = db
        .query_row(
            "SELECT grant_id,recipient FROM federation_revocations ORDER BY rowid LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((grant, recipient)) = job else {
        return Ok(0);
    };
    let rows:Vec<i64>=db.prepare("SELECT sequence FROM mailbox WHERE remote_grant=?1 AND payload IS NOT NULL ORDER BY sequence LIMIT 64")?.query_map([&grant],|r|r.get(0))?.collect::<Result<_,_>>()?;
    for sequence in &rows {
        release_payload(db, *sequence)?;
    }
    if rows.len() < 64 {
        if db.execute("UPDATE retained_storage SET bytes=bytes-?2 WHERE account_id=(SELECT account_id FROM devices WHERE id=?1) AND bytes>=?2",(&recipient,sql(REVOCATION_BYTES)?))?!=1{return Err(StoreError::InvalidData)}
        db.execute(
            "DELETE FROM federation_revocations WHERE grant_id=?1",
            [grant],
        )?;
    }
    Ok(rows.len() + usize::from(rows.len() < 64))
}

const REVOCATION_BYTES: u64 = 512;
impl Store {
    pub fn federation_sender_permission(
        &mut self,
        credential: &str,
        server: &str,
        device: &str,
        now: u64,
    ) -> Result<sigil_protocol::federation::SenderPermission, StoreError> {
        if !sigil_protocol::valid_server_name(server) || !valid_credential(device) {
            return Err(StoreError::Invalid("invalid remote sender"));
        }
        let tx = self.0.transaction()?;
        let recipient = authorize(&tx, credential, now)?;
        let result=tx.query_row("SELECT revision,account,grant_id IS NOT NULL FROM federation_senders WHERE recipient=?1 AND server=?2 AND device=?3",(&recipient,server,device),|r|Ok(sigil_protocol::federation::SenderPermission{revision:unsigned(r,0)?,sender:RemoteSender{server:server.into(),device:device.into(),account:r.get(1)?},allowed:r.get(2)?})).optional()?.ok_or(StoreError::NotFound)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn configure_federation_sender(
        &mut self,
        credential: &str,
        request: sigil_protocol::federation::ConfigureSender,
        now: u64,
    ) -> Result<sigil_protocol::federation::SenderPermission, StoreError> {
        use sha2::{Digest, Sha256};
        if !valid(&request.sender) {
            return Err(StoreError::Invalid("invalid remote sender"));
        }
        let hash =
            Sha256::digest(serde_json::to_vec(&request).map_err(|_| StoreError::InvalidData)?);
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let recipient = authorize(&tx, credential, now)?;
        let sender = &request.sender;
        type Row = (String, Option<String>, u64, Vec<u8>);
        let old:Option<Row>=tx.query_row("SELECT account,grant_id,revision,request_hash FROM federation_senders WHERE recipient=?1 AND server=?2 AND device=?3",(&recipient,&sender.server,&sender.device),|r|Ok((r.get(0)?,r.get(1)?,unsigned(r,2)?,r.get(3)?))).optional()?;
        let revision = old.as_ref().map_or(0, |v| v.2);
        if revision != request.expected_revision {
            if let Some((_, grant, _, previous)) = &old {
                if request.expected_revision.checked_add(1) == Some(revision)
                    && previous == hash.as_slice()
                {
                    return Ok(sigil_protocol::federation::SenderPermission {
                        revision,
                        sender: sender.clone(),
                        allowed: grant.is_some(),
                    });
                }
            }
            return Err(StoreError::Conflict);
        }
        if old.as_ref().is_some_and(|v| v.0 != sender.account) {
            return Err(StoreError::Conflict);
        }
        let peer = federation_config::peer(&tx, &sender.server)?.ok_or(StoreError::Forbidden)?;
        if request.allowed && !peer.allowed {
            return Err(StoreError::Forbidden);
        }
        let prior = old.as_ref().and_then(|v| v.1.clone());
        let mut grant = prior.clone();
        let mut reserve = if old.is_none() { METADATA } else { 0 };
        if request.allowed && grant.is_none() {
            let count:i64=tx.query_row("SELECT count(*) FROM federation_senders WHERE recipient=?1 AND grant_id IS NOT NULL",[&recipient],|r|r.get(0))?;
            if count >= 256 {
                return Err(StoreError::Busy);
            }
            // Prepay cleanup while granting. A full quota must never prevent revocation.
            reserve += REVOCATION_BYTES;
            grant = Some(crate::auth::random_secret().map_err(|_| StoreError::InvalidData)?);
        } else if !request.allowed {
            if let Some(prior) = prior {
                tx.execute(
                    "INSERT INTO federation_revocations VALUES(?1,?2)",
                    (prior, &recipient),
                )?;
            }
            grant = None;
        }
        if reserve != 0 {
            crate::storage_budget::for_device(&tx, &recipient, reserve, now)?;
        }
        let next = revision
            .checked_add(1)
            .filter(|v| *v <= i64::MAX as u64)
            .ok_or(StoreError::InvalidData)?;
        tx.execute("INSERT INTO federation_senders VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(recipient,server,device) DO UPDATE SET grant_id=excluded.grant_id,revision=excluded.revision,request_hash=excluded.request_hash",(&recipient,&sender.server,&sender.account,&sender.device,grant,sql(next)?,hash.as_slice()))?;
        tx.commit()?;
        Ok(sigil_protocol::federation::SenderPermission {
            revision: next,
            sender: request.sender,
            allowed: request.allowed,
        })
    }
}

#[cfg(test)]
#[path = "federation_mailbox_tests.rs"]
mod tests;
