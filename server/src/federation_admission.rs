use crate::{
    federation_auth as auth, federation_config,
    push_config::{sql, unsigned},
    store::{Store, StoreError},
};
use axum::http::HeaderMap;
use rusqlite::{Connection, Transaction, TransactionBehavior};
use sha2::{Digest, Sha256};
pub(crate) const GLOBAL_BUDGET: u64 = 64 * 1024 * 1024;

/// Verification and nonce consumption share the operation's transaction. No
/// caller-provided Verified token can accidentally authorize a different body.
pub(crate) fn admit(
    db: &Transaction<'_>,
    path: &str,
    body: &[u8],
    headers: &HeaderMap,
    now: u64,
) -> Result<String, StoreError> {
    let (origin, destination, metadata) =
        auth::inspect(headers).map_err(|_| StoreError::Unauthorized)?;
    let config = federation_config::read(db)?;
    if !config.enabled || config.server.as_deref() != Some(destination) {
        return Err(StoreError::Forbidden);
    }
    let peer = federation_config::peer(db, origin)?.ok_or(StoreError::Forbidden)?;
    if !peer.allowed
        || peer.error.is_some()
        || peer.checked_at == 0
        || peer.checked_at > now
        || now - peer.checked_at > 3600
    {
        return Err(StoreError::Forbidden);
    }
    let discovery = peer.pinned.ok_or(StoreError::Forbidden)?;
    let key = if discovery.current.id == metadata.key_id {
        &discovery.current
    } else {
        let old = discovery
            .rotation
            .as_ref()
            .ok_or(StoreError::Unauthorized)?;
        if old.previous.id != metadata.key_id || metadata.created >= discovery.current.not_before {
            return Err(StoreError::Unauthorized);
        }
        &old.previous
    };
    let request = auth::Request {
        origin,
        destination,
        path,
        body,
    };
    auth::verify(key, &request, headers, now).map_err(|_| StoreError::Unauthorized)?;
    // A duplicate remains a replay even if the peer has exhausted its rate budget.
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM federation_nonces WHERE server=?1 AND nonce=?2)",
        (origin, &metadata.nonce),
        |r| r.get(0),
    )?;
    if exists {
        return Err(StoreError::Conflict);
    }
    let (bytes, credit, updated): (u64, u64, u64) = db.query_row(
        "SELECT nonce_bytes+ingress_bytes+egress_bytes,credit,updated_at FROM federation_admission WHERE server=?1",
        [origin],
        |r| Ok((unsigned(r, 0)?, unsigned(r, 1)?, unsigned(r, 2)?)),
    )?;
    let total: u64 = db.query_row(
        "SELECT nonce_bytes FROM federation_usage WHERE id=1",
        [],
        |r| unsigned(r, 0),
    )?;
    let credit = credit.saturating_add(now.saturating_sub(updated)).min(20);
    if credit == 0
        || bytes.saturating_add(1024) > config.peer_quota_bytes
        || total.saturating_add(1024) > GLOBAL_BUDGET
    {
        return Err(StoreError::Busy);
    }
    db.execute(
        "UPDATE federation_admission SET credit=?2,updated_at=?3,nonce_bytes=nonce_bytes+1024 WHERE server=?1",
        (origin, sql(credit - 1)?, sql(now.max(updated))?),
    )?;
    db.execute(
        "INSERT INTO federation_nonces VALUES(?1,?2,?3)",
        (origin, &metadata.nonce, sql(metadata.expires)?),
    )?;
    db.execute(
        "UPDATE federation_usage SET nonce_bytes=nonce_bytes+1024 WHERE id=1",
        [],
    )?;
    Ok(auth::hex(&Sha256::digest(body)))
}
impl Store {
    pub(crate) fn federation_ping(
        &mut self,
        body: &[u8],
        headers: &HeaderMap,
        now: u64,
    ) -> Result<String, StoreError> {
        if body != b"{}" {
            return Err(StoreError::Invalid("invalid federation ping"));
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let hash = admit(
            &tx,
            sigil_protocol::federation::PING_PATH,
            body,
            headers,
            now,
        )?;
        tx.commit()?;
        Ok(hash)
    }
}
pub(crate) fn cleanup(db: &Connection, now: u64) -> Result<usize, StoreError> {
    let rows:Vec<(i64,String)>=db.prepare("SELECT rowid,server FROM federation_nonces WHERE expires_at<=?1 ORDER BY expires_at LIMIT 64")?.query_map([sql(now)?],|r|Ok((r.get(0)?,r.get(1)?)))?.collect::<Result<_,_>>()?;
    for (row, server) in &rows {
        if db.execute("UPDATE federation_admission SET nonce_bytes=nonce_bytes-1024 WHERE server=?1 AND nonce_bytes>=1024",[server])?!=1{return Err(StoreError::InvalidData)}
        db.execute("DELETE FROM federation_nonces WHERE rowid=?1", [row])?;
    }
    if db.execute(
        "UPDATE federation_usage SET nonce_bytes=nonce_bytes-?1 WHERE id=1 AND nonce_bytes>=?1",
        [rows.len() as i64 * 1024],
    )? != 1
    {
        return Err(StoreError::InvalidData);
    }
    Ok(rows.len())
}
