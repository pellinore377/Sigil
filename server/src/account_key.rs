//! Account keys, pending devices and passkey-wrapped recovery secrets. The server verifies
//! endorsements so an unendorsed sign-in never becomes a messaging device; peers verify again.
use crate::{
    accounts::retire_account_delivery,
    auth::digest,
    enrollment::{bearer, native_only, now},
    prekeys::authorize,
    push_config::sql,
    store::{Store, StoreError},
    store_error, with_store, AppState,
};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use sigil_protocol::{
    accounts::{
        valid_credential, AccountKey, Activate, PendingState, PublishAccountKey, RecoveryWrap,
        RecoveryWraps, ResetIdentity, Session, MAX_RECOVERY_WRAPS,
    },
    device::{SignedBinding, Statement},
};

pub(crate) const MIGRATION: &str = "
CREATE TABLE IF NOT EXISTS account_keys(account_id TEXT PRIMARY KEY REFERENCES accounts(id), public BLOB NOT NULL, bundle BLOB);
CREATE TABLE IF NOT EXISTS device_endorsements(device TEXT PRIMARY KEY REFERENCES devices(id), signature BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS pending_devices(id TEXT PRIMARY KEY, account_id TEXT NOT NULL REFERENCES accounts(id), label TEXT NOT NULL,
 token_hash BLOB NOT NULL UNIQUE, expires_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS recovery_wraps(account_id TEXT NOT NULL REFERENCES accounts(id), id BLOB NOT NULL, salt BLOB NOT NULL,
 wrapped BLOB NOT NULL, label TEXT NOT NULL, created INTEGER NOT NULL, PRIMARY KEY(account_id,id));
";
pub(crate) const LINK_COLUMNS: &str =
    "ALTER TABLE device_links ADD COLUMN endorsement BLOB; ALTER TABLE device_links ADD COLUMN secrets BLOB;";
const PENDING_LIFETIME: u64 = 60 * 60;
pub(crate) const DEVICE_LIFETIME: u64 = 30 * 24 * 60 * 60;
const MAX_BUNDLE: usize = 512;

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub(crate) fn unhex(value: &str, max: usize) -> Result<Vec<u8>, StoreError> {
    if value.is_empty()
        || value.len() > max * 2
        || !value.len().is_multiple_of(2)
        || !value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(StoreError::Invalid("invalid hex field"));
    }
    Ok(value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap_or("zz"), 16).unwrap_or(0))
        .collect())
}
fn key32(value: &str) -> Result<[u8; 32], StoreError> {
    unhex(value, 32)?
        .try_into()
        .map_err(|_| StoreError::Invalid("invalid account key"))
}

pub(crate) fn account_key(db: &Connection, account: &str) -> Result<Option<AccountKey>, StoreError> {
    Ok(db
        .query_row(
            "SELECT public,bundle FROM account_keys WHERE account_id=?1",
            [account],
            |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Option<Vec<u8>>>(1)?)),
        )
        .optional()?
        .map(|(public, bundle)| AccountKey {
            public: hex(&public),
            bundle: bundle.map(|b| hex(&b)),
        }))
}
pub(crate) fn endorsement(db: &Connection, device: &str) -> Result<Option<Vec<u8>>, StoreError> {
    Ok(db
        .query_row(
            "SELECT signature FROM device_endorsements WHERE device=?1",
            [device],
            |r| r.get(0),
        )
        .optional()?)
}

/// Checks `signature` endorses `binding` under the account's stored key.
pub(crate) fn verify(
    db: &Connection,
    account: &str,
    binding: &SignedBinding,
    signature: &str,
) -> Result<Vec<u8>, StoreError> {
    let public: Vec<u8> = db
        .query_row(
            "SELECT public FROM account_keys WHERE account_id=?1",
            [account],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(StoreError::Conflict)?;
    let public: [u8; 32] = public.try_into().map_err(|_| StoreError::InvalidData)?;
    endorse_check(&public, binding, signature)
}
fn endorse_check(
    public: &[u8; 32],
    binding: &SignedBinding,
    signature: &str,
) -> Result<Vec<u8>, StoreError> {
    let signature = unhex(signature, 64)?;
    let fingerprint =
        sigil_crypto::link::fingerprint(&binding.binding).map_err(|_| StoreError::InvalidData)?;
    sigil_crypto::account::verify_endorsement(public, &fingerprint, &signature)
        .map_err(|_| StoreError::Unauthorized)?;
    Ok(signature)
}

/// A sign-in to an account that already has devices or a key waits for an endorsement.
pub(crate) fn admit(
    tx: &Transaction<'_>,
    account: &str,
    label: &str,
    credential: &str,
    now: u64,
) -> Result<(String, u64, bool), StoreError> {
    let device = crate::auth::random_secret().map_err(|_| StoreError::InvalidData)?;
    let established: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM account_keys WHERE account_id=?1) OR EXISTS(SELECT 1 FROM devices WHERE account_id=?1 AND revoked=0 AND expires_at>?2)",
        (account, sql(now)?),
        |r| r.get(0),
    )?;
    let duplicate: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM devices WHERE token_hash=?1) OR EXISTS(SELECT 1 FROM pending_devices WHERE token_hash=?1)",
        [digest(credential).as_slice()],
        |r| r.get(0),
    )?;
    if duplicate {
        return Err(StoreError::AlreadyExists);
    }
    if established {
        let expires = now + PENDING_LIFETIME;
        tx.execute("DELETE FROM pending_devices WHERE expires_at<=?1", [sql(now)?])?;
        if tx.query_row(
            "SELECT count(*) FROM pending_devices WHERE account_id=?1",
            [account],
            |r| r.get::<_, i64>(0),
        )? >= 8
        {
            return Err(StoreError::Busy);
        }
        tx.execute(
            "INSERT INTO pending_devices VALUES(?1,?2,?3,?4,?5)",
            (&device, account, label, digest(credential).as_slice(), sql(expires)?),
        )?;
        return Ok((device, expires, true));
    }
    let expires = now + DEVICE_LIFETIME;
    crate::storage_budget::reserve(tx, account, crate::storage_budget::DEVICE, now)?;
    tx.execute(
        "INSERT INTO devices(id,account_id,label,token_hash,expires_at) VALUES(?1,?2,?3,?4,?5)",
        (&device, account, label, digest(credential).as_slice(), sql(expires)?),
    )?;
    Ok((device, expires, false))
}

fn pending(db: &Connection, credential: &str, now: u64) -> Result<(String, String, String), StoreError> {
    if !valid_credential(credential) {
        return Err(StoreError::Unauthorized);
    }
    db.query_row(
        "SELECT p.id,p.account_id,p.label FROM pending_devices p JOIN accounts a ON a.id=p.account_id WHERE p.token_hash=?1 AND p.expires_at>?2 AND a.disabled=0",
        (digest(credential).as_slice(), sql(now)?),
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .optional()?
    .ok_or(StoreError::Unauthorized)
}
/// Rate limiting keys pending devices by id like active ones.
pub(crate) fn authorize_pending(db: &Connection, credential: &str, now: u64) -> Result<String, StoreError> {
    pending(db, credential, now).map(|v| v.0)
}
fn account_of(db: &Connection, device: &str) -> Result<(String, String), StoreError> {
    Ok(db.query_row(
        "SELECT a.id,a.username FROM accounts a JOIN devices d ON d.account_id=a.id WHERE d.id=?1",
        [device],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?)
}
fn wraps(db: &Connection, account: &str) -> Result<Vec<RecoveryWrap>, StoreError> {
    db.prepare("SELECT id,salt,wrapped,label,created FROM recovery_wraps WHERE account_id=?1 ORDER BY created,id")?
        .query_map([account], |r| {
            Ok(RecoveryWrap {
                id: hex(&r.get::<_, Vec<u8>>(0)?),
                salt: hex(&r.get::<_, Vec<u8>>(1)?),
                wrapped: hex(&r.get::<_, Vec<u8>>(2)?),
                label: r.get(3)?,
                created: r.get::<_, i64>(4)? as u64,
            })
        })?
        .collect::<Result<_, _>>()
        .map_err(Into::into)
}
fn binding_for(
    statement: &str,
    server: &str,
    username: &str,
    account: &str,
    device: &str,
) -> Result<SignedBinding, StoreError> {
    let bytes = Statement {
        statement: statement.into(),
    }
    .bytes()
    .map_err(StoreError::Invalid)?;
    let signed = SignedBinding::from_bytes(&bytes).map_err(StoreError::Invalid)?;
    let b = &signed.binding;
    if b.server != server || b.username != username || hex(&b.account) != account || hex(&b.device) != device {
        return Err(StoreError::Invalid("device statement does not match the pending device"));
    }
    sigil_crypto::verify_signature(
        &b.identity,
        &b.signing_bytes().map_err(StoreError::Invalid)?,
        &signed.signature,
    )
    .map_err(|_| StoreError::Unauthorized)?;
    Ok(signed)
}

impl Store {
    fn server_name(&self) -> Result<String, StoreError> {
        Ok(self
            .configuration()?
            .settings
            .ok_or(StoreError::Unauthorized)?
            .server_name)
    }
    pub fn pending_state(&mut self, credential: &str, now: u64) -> Result<PendingState, StoreError> {
        let server = self.server_name()?;
        let (device, account, label) = pending(&self.0, credential, now)?;
        let (username, expires): (String, i64) = self.0.query_row(
            "SELECT a.username,p.expires_at FROM accounts a JOIN pending_devices p ON p.account_id=a.id WHERE p.id=?1",
            [&device],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(PendingState {
            session: Session {
                account_id: account.clone(),
                address: format!("@{username}:{server}"),
                device_id: device,
                device_label: label,
                expires_at: expires as u64,
                pending: true,
            },
            account_key: account_key(&self.0, &account)?,
            wraps: wraps(&self.0, &account)?,
        })
    }
    /// Moves an endorsed pending device into the account; other devices are untouched.
    pub fn activate_pending(&mut self, credential: &str, request: Activate, now: u64) -> Result<Session, StoreError> {
        self.promote(credential, &request.statement, None, &request.endorsement, now)
    }
    /// Lost recovery secret: a new account key; every other device is signed out.
    pub fn reset_identity(&mut self, credential: &str, request: ResetIdentity, now: u64) -> Result<Session, StoreError> {
        self.promote(
            credential,
            &request.statement,
            Some(&request.key),
            &request.key.endorsement,
            now,
        )
    }
    fn promote(
        &mut self,
        credential: &str,
        statement: &str,
        reset: Option<&PublishAccountKey>,
        signature: &str,
        now: u64,
    ) -> Result<Session, StoreError> {
        let server = self.server_name()?;
        let tx = self.0.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (device, account, label) = pending(&tx, credential, now)?;
        let username: String =
            tx.query_row("SELECT username FROM accounts WHERE id=?1", [&account], |r| r.get(0))?;
        let signed = binding_for(statement, &server, &username, &account, &device)?;
        let signature = if let Some(key) = reset {
            let public = key32(&key.public)?;
            let signature = endorse_check(&public, &signed, signature)?;
            let bundle = key.bundle.as_deref().map(|b| unhex(b, MAX_BUNDLE)).transpose()?;
            tx.execute(
                "UPDATE devices SET revoked=1,token_hash=NULL WHERE account_id=?1",
                [&account],
            )?;
            retire_account_delivery(&tx, &account)?;
            tx.execute("DELETE FROM recovery_wraps WHERE account_id=?1", [&account])?;
            tx.execute("DELETE FROM recovery_heads WHERE account_id=?1", [&account])?;
            tx.execute(
                "UPDATE recovery_objects SET data=NULL WHERE account_id=?1",
                [&account],
            )?;
            tx.execute(
                "INSERT INTO account_keys VALUES(?1,?2,?3) ON CONFLICT(account_id) DO UPDATE SET public=excluded.public,bundle=excluded.bundle",
                (&account, public.as_slice(), bundle),
            )?;
            signature
        } else {
            verify(&tx, &account, &signed, signature)?
        };
        if tx.query_row(
            "SELECT count(*) FROM devices WHERE account_id=?1 AND revoked=0 AND expires_at>?2",
            (&account, sql(now)?),
            |r| r.get::<_, i64>(0),
        )? >= 256
        {
            return Err(StoreError::Busy);
        }
        let expires = now + DEVICE_LIFETIME;
        crate::storage_budget::reserve(&tx, &account, crate::storage_budget::DEVICE, now)?;
        tx.execute("DELETE FROM pending_devices WHERE id=?1", [&device])?;
        tx.execute(
            "INSERT INTO devices(id,account_id,label,token_hash,expires_at) VALUES(?1,?2,?3,?4,?5)",
            (&device, &account, &label, digest(credential).as_slice(), sql(expires)?),
        )?;
        let bytes = signed.to_bytes().map_err(StoreError::Invalid)?;
        tx.execute(
            "INSERT INTO encryption_identities VALUES(?1,?2)",
            (&device, signed.binding.identity.as_slice()),
        )?;
        tx.execute("INSERT INTO device_bindings VALUES(?1,?2)", (&device, bytes))?;
        tx.execute(
            "INSERT INTO device_endorsements VALUES(?1,?2)",
            (&device, signature),
        )?;
        tx.commit()?;
        Ok(Session {
            account_id: account,
            address: format!("@{username}:{server}"),
            device_id: device,
            device_label: label,
            expires_at: expires,
            pending: false,
        })
    }
    pub fn cancel_pending(&mut self, credential: &str, now: u64) -> Result<(), StoreError> {
        let (device, _, _) = pending(&self.0, credential, now)?;
        self.0.execute("DELETE FROM pending_devices WHERE id=?1", [device])?;
        Ok(())
    }
    pub fn account_key(&mut self, credential: &str, now: u64) -> Result<AccountKey, StoreError> {
        let tx = self.0.transaction()?;
        let device = authorize(&tx, credential, now)?;
        account_key(&tx, &account_of(&tx, &device)?.0)?.ok_or(StoreError::NotFound)
    }
    /// First publication pins the key; later calls may only replace the bundle.
    pub fn publish_account_key(&mut self, credential: &str, request: PublishAccountKey, now: u64) -> Result<(), StoreError> {
        let public = key32(&request.public)?;
        let bundle = request.bundle.as_deref().map(|b| unhex(b, MAX_BUNDLE)).transpose()?;
        let tx = self.0.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let device = authorize(&tx, credential, now)?;
        let (account, _) = account_of(&tx, &device)?;
        let statement: Vec<u8> = tx
            .query_row("SELECT statement FROM device_bindings WHERE device=?1", [&device], |r| r.get(0))
            .optional()?
            .ok_or(StoreError::Conflict)?;
        let signed = SignedBinding::from_bytes(&statement).map_err(|_| StoreError::InvalidData)?;
        let signature = endorse_check(&public, &signed, &request.endorsement)?;
        let stored: Option<Vec<u8>> = tx
            .query_row("SELECT public FROM account_keys WHERE account_id=?1", [&account], |r| r.get(0))
            .optional()?;
        match stored {
            Some(stored) if stored != public => return Err(StoreError::Conflict),
            Some(_) => {
                if let Some(bundle) = bundle {
                    tx.execute("UPDATE account_keys SET bundle=?2 WHERE account_id=?1", (&account, bundle))?;
                }
            }
            None => {
                tx.execute("INSERT INTO account_keys VALUES(?1,?2,?3)", (&account, public.as_slice(), bundle))?;
            }
        }
        tx.execute(
            "INSERT INTO device_endorsements VALUES(?1,?2) ON CONFLICT(device) DO UPDATE SET signature=excluded.signature",
            (&device, signature),
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn recovery_wraps(&mut self, credential: &str, now: u64) -> Result<RecoveryWraps, StoreError> {
        let tx = self.0.transaction()?;
        let device = authorize(&tx, credential, now)?;
        Ok(RecoveryWraps {
            wraps: wraps(&tx, &account_of(&tx, &device)?.0)?,
        })
    }
    pub fn put_recovery_wrap(&mut self, credential: &str, wrap: RecoveryWrap, now: u64) -> Result<(), StoreError> {
        let id = unhex(&wrap.id, 1023)?;
        let salt = unhex(&wrap.salt, 32)?;
        let wrapped = unhex(&wrap.wrapped, 128)?;
        if salt.len() != 32 || wrap.label.is_empty() || wrap.label.len() > 80 || wrap.label.chars().any(char::is_control) {
            return Err(StoreError::Invalid("invalid recovery passkey"));
        }
        let tx = self.0.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let device = authorize(&tx, credential, now)?;
        let (account, _) = account_of(&tx, &device)?;
        let count: i64 = tx.query_row("SELECT count(*) FROM recovery_wraps WHERE account_id=?1 AND id<>?2", (&account, &id), |r| r.get(0))?;
        if count as usize >= MAX_RECOVERY_WRAPS {
            return Err(StoreError::Busy);
        }
        tx.execute(
            "INSERT INTO recovery_wraps VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(account_id,id) DO UPDATE SET salt=excluded.salt,wrapped=excluded.wrapped,label=excluded.label",
            (&account, id, salt, wrapped, &wrap.label, sql(now)?),
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn delete_recovery_wrap(&mut self, credential: &str, id: &str, now: u64) -> Result<(), StoreError> {
        let id = unhex(id, 1023)?;
        let tx = self.0.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let device = authorize(&tx, credential, now)?;
        let (account, _) = account_of(&tx, &device)?;
        if tx.execute("DELETE FROM recovery_wraps WHERE account_id=?1 AND id=?2", (&account, id))? == 0 {
            return Err(StoreError::NotFound);
        }
        tx.commit()?;
        Ok(())
    }
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/client/v0/pending", get(pending_get).delete(pending_cancel))
        .route("/client/v0/pending/activate", post(pending_activate))
        .route("/client/v0/pending/reset", post(pending_reset))
        .route("/client/v0/account-key", get(key_get).put(key_put))
        .route("/client/v0/recovery-wraps", get(wraps_get))
        .route("/client/v0/recovery-wraps/{id}", put(wrap_put).delete(wrap_delete))
        .route_layer(middleware::from_fn(native_only))
        .layer(tower_http::limit::RequestBodyLimitLayer::new(8192))
}
fn reply<T: serde::Serialize>(result: Result<T, StoreError>) -> Response {
    match result {
        Ok(value) => Json(value).into_response(),
        Err(error) => store_error(error),
    }
}
fn done(result: Result<(), StoreError>) -> Response {
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => store_error(error),
    }
}
macro_rules! with_bearer {
    ($state:expr, $headers:expr, |$s:ident, $c:ident| $body:expr) => {{
        let $c = match bearer(&$headers) {
            Ok(value) => value,
            Err(error) => return store_error(error),
        };
        with_store($state, move |$s| $body).await
    }};
}
async fn pending_get(State(state): State<AppState>, headers: HeaderMap) -> Response {
    reply(with_bearer!(state, headers, |s, c| s.pending_state(&c, now()?)))
}
async fn pending_cancel(State(state): State<AppState>, headers: HeaderMap) -> Response {
    done(with_bearer!(state, headers, |s, c| s.cancel_pending(&c, now()?)))
}
async fn pending_activate(State(state): State<AppState>, headers: HeaderMap, Json(v): Json<Activate>) -> Response {
    reply(with_bearer!(state, headers, |s, c| s.activate_pending(&c, v, now()?)))
}
async fn pending_reset(State(state): State<AppState>, headers: HeaderMap, Json(v): Json<ResetIdentity>) -> Response {
    reply(with_bearer!(state, headers, |s, c| s.reset_identity(&c, v, now()?)))
}
async fn key_get(State(state): State<AppState>, headers: HeaderMap) -> Response {
    reply(with_bearer!(state, headers, |s, c| s.account_key(&c, now()?)))
}
async fn key_put(State(state): State<AppState>, headers: HeaderMap, Json(v): Json<PublishAccountKey>) -> Response {
    done(with_bearer!(state, headers, |s, c| s.publish_account_key(&c, v, now()?)))
}
async fn wraps_get(State(state): State<AppState>, headers: HeaderMap) -> Response {
    reply(with_bearer!(state, headers, |s, c| s.recovery_wraps(&c, now()?)))
}
async fn wrap_put(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Json(v): Json<RecoveryWrap>) -> Response {
    if v.id != id {
        return store_error(StoreError::Invalid("passkey id mismatch"));
    }
    done(with_bearer!(state, headers, |s, c| s.put_recovery_wrap(&c, v, now()?)))
}
async fn wrap_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    done(with_bearer!(state, headers, |s, c| s.delete_recovery_wrap(&c, &id, now()?)))
}

#[cfg(test)]
#[path = "account_key_tests.rs"]
mod tests;
