//! Account authorization requires a live sponsor and a complete two-device proof.
use crate::{
    enrollment::{bearer, native_only, now},
    prekeys::authorize,
    store::{Store, StoreError},
    store_error, with_store, AppState,
};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use rusqlite::{OptionalExtension, TransactionBehavior};
use sigil_protocol::{accounts::Session, device::SignedBinding, link::Authorization};

pub(crate) const MIGRATION: &str = "CREATE TABLE device_links(target TEXT PRIMARY KEY REFERENCES devices(id),sponsor TEXT NOT NULL REFERENCES devices(id),challenge BLOB NOT NULL,joining_challenge BLOB NOT NULL UNIQUE,proof BLOB NOT NULL,UNIQUE(sponsor,challenge)); CREATE TABLE cancelled_device_links(sponsor TEXT NOT NULL REFERENCES devices(id),challenge BLOB NOT NULL,PRIMARY KEY(sponsor,challenge));";
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
impl Store {
    pub fn authorize_device_link(
        &mut self,
        credential: &str,
        authorization: Authorization,
        now: u64,
    ) -> Result<Session, StoreError> {
        let proof = authorization.parse().map_err(StoreError::Invalid)?;
        let bytes = proof.to_bytes().map_err(StoreError::Invalid)?;
        let server = self
            .configuration()?
            .settings
            .ok_or(StoreError::Unauthorized)?
            .server_name;
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let sponsor = authorize(&tx, credential, now)?;
        if tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM cancelled_device_links WHERE sponsor=?1 AND challenge=?2)",
            (&sponsor, proof.transcript.sponsor_challenge.as_slice()),
            |r| r.get::<_, bool>(0),
        )? {
            return Err(StoreError::Unauthorized);
        }
        let statement: Vec<u8> = tx
            .query_row(
                "SELECT statement FROM device_bindings WHERE device=?1 AND length(statement)<=512",
                [&sponsor],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(StoreError::Unauthorized)?;
        let trusted = SignedBinding::from_bytes(&statement).map_err(|_| StoreError::InvalidData)?;
        if trusted.binding != proof.sponsor.binding
            || hex(&trusted.binding.device) != sponsor
            || trusted.binding.server != server
        {
            return Err(StoreError::Unauthorized);
        }
        let expected = sigil_crypto::link::fingerprint(&trusted.binding)
            .map_err(|_| StoreError::InvalidData)?;
        let target = hex(&proof.joining.binding.device);
        let account = hex(&proof.joining.binding.account);
        let (actual_account, username): (String, String) = tx.query_row("SELECT a.id,a.username FROM accounts a JOIN devices d ON d.account_id=a.id WHERE d.id=?1 AND a.disabled=0", [&sponsor], |r| Ok((r.get(0)?,r.get(1)?)))?;
        if actual_account != account || username != proof.joining.binding.username {
            return Err(StoreError::Unauthorized);
        }
        let prior: Option<Vec<u8>> = tx
            .query_row(
                "SELECT proof FROM device_links WHERE target=?1",
                [&target],
                |r| r.get(0),
            )
            .optional()?;
        let verification_time = if prior.as_deref() == Some(bytes.as_slice()) {
            proof.transcript.created_at
        } else {
            now
        };
        sigil_crypto::link::verify(&proof, expected, verification_time)
            .map_err(|_| StoreError::Unauthorized)?;
        let expires_at = if let Some(prior) = prior {
            if prior != bytes {
                return Err(StoreError::Conflict);
            }
            tx.query_row(
                "SELECT expires_at FROM devices WHERE id=?1 AND revoked=0 AND expires_at>?2",
                (&target, now as i64),
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .ok_or(StoreError::Unauthorized)? as u64
        } else {
            let used: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM device_links WHERE (sponsor=?1 AND challenge=?2) OR joining_challenge=?3) OR EXISTS(SELECT 1 FROM devices WHERE id=?4 OR token_hash=?5)", (&sponsor,proof.transcript.sponsor_challenge.as_slice(),proof.transcript.joining_challenge.as_slice(),&target,proof.transcript.credential_commitment.as_slice()), |r|r.get(0))?;
            if used {
                return Err(StoreError::AlreadyExists);
            }
            if tx.query_row(
                "SELECT count(*) FROM devices WHERE account_id=?1 AND revoked=0 AND expires_at>?2",
                (&account, now as i64),
                |r| r.get::<_, i64>(0),
            )? >= 256
            {
                return Err(StoreError::Busy);
            }
            crate::storage_budget::reserve(
                &tx,
                &account,
                crate::storage_budget::DEVICE + crate::storage_budget::LINK + bytes.len() as u64,
                now,
            )?;
            let expiry = now
                .checked_add(30 * 24 * 60 * 60)
                .filter(|time| *time <= i64::MAX as u64)
                .ok_or(StoreError::InvalidData)?;
            tx.execute("INSERT INTO devices(id,account_id,label,token_hash,expires_at) VALUES(?1,?2,'Linked device',?3,?4)", (&target,&account,proof.transcript.credential_commitment.as_slice(),expiry as i64))?;
            tx.execute(
                "INSERT INTO encryption_identities VALUES(?1,?2)",
                (&target, proof.joining.binding.identity.as_slice()),
            )?;
            tx.execute(
                "INSERT INTO device_bindings VALUES(?1,?2)",
                (
                    &target,
                    proof.joining.to_bytes().map_err(StoreError::Invalid)?,
                ),
            )?;
            tx.execute(
                "INSERT INTO device_links VALUES(?1,?2,?3,?4,?5)",
                (
                    &target,
                    &sponsor,
                    proof.transcript.sponsor_challenge.as_slice(),
                    proof.transcript.joining_challenge.as_slice(),
                    bytes,
                ),
            )?;
            expiry
        };
        tx.commit()?;
        Ok(Session {
            account_id: account,
            address: format!("@{username}:{server}"),
            device_id: target,
            device_label: "Linked device".into(),
            expires_at,
        })
    }
}
impl Store {
    pub fn own_device_link(
        &mut self,
        credential: &str,
        now: u64,
    ) -> Result<Authorization, StoreError> {
        let tx = self.0.transaction()?;
        let target = authorize(&tx, credential, now)?;
        let bytes:Vec<u8>=tx.query_row("SELECT CASE WHEN length(proof)<=1380 THEN proof END FROM device_links WHERE target=?1", [&target], |r|r.get(0)).optional()?.ok_or(StoreError::NotFound)?;
        Ok(Authorization { proof: hex(&bytes) })
    }
}
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/client/v0/device-links", post(authorize_link))
        .route("/client/v0/device-links/{challenge}", delete(cancel_link))
        .route("/client/v0/device-link", get(own_link))
        .route_layer(middleware::from_fn(native_only))
        .layer(tower_http::limit::RequestBodyLimitLayer::new(4096))
}
async fn own_link(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let credential = match bearer(&headers) {
        Ok(value) => value,
        Err(error) => return store_error(error),
    };
    match with_store(state, move |store| {
        store.own_device_link(&credential, now()?)
    })
    .await
    {
        Ok(value) => Json(value).into_response(),
        Err(error) => store_error(error),
    }
}
async fn authorize_link(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<Authorization>,
) -> Response {
    let credential = match bearer(&headers) {
        Ok(value) => value,
        Err(error) => return store_error(error),
    };
    match with_store(state, move |store| {
        store.authorize_device_link(&credential, request, now()?)
    })
    .await
    {
        Ok(session) => (StatusCode::CREATED, Json(session)).into_response(),
        Err(error) => store_error(error),
    }
}

impl Store {
    pub fn cancel_device_link(
        &mut self,
        credential: &str,
        challenge: &str,
        now: u64,
    ) -> Result<(), StoreError> {
        if !sigil_protocol::accounts::valid_credential(challenge) {
            return Err(StoreError::Invalid("invalid linking challenge"));
        }
        let raw: Vec<u8> = challenge
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                u8::from_str_radix(std::str::from_utf8(pair).expect("hex"), 16).expect("hex")
            })
            .collect();
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let sponsor = authorize(&tx, credential, now)?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM cancelled_device_links WHERE sponsor=?1 AND challenge=?2)",
            (&sponsor, &raw),
            |r| r.get(0),
        )?;
        if !exists {
            crate::storage_budget::for_device(
                &tx,
                &sponsor,
                crate::storage_budget::CANCELLATION,
                now,
            )?;
            tx.execute(
                "INSERT INTO cancelled_device_links VALUES(?1,?2)",
                (&sponsor, &raw),
            )?;
        }
        tx.execute("UPDATE devices SET revoked=1,token_hash=NULL WHERE id IN (SELECT target FROM device_links WHERE sponsor=?1 AND challenge=?2)",(&sponsor,&raw))?;
        tx.commit()?;
        Ok(())
    }
}
async fn cancel_link(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(challenge): Path<String>,
) -> Response {
    let credential = match bearer(&headers) {
        Ok(value) => value,
        Err(error) => return store_error(error),
    };
    match with_store(state, move |store| {
        store.cancel_device_link(&credential, &challenge, now()?)
    })
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => store_error(error),
    }
}
