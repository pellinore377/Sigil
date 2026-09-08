use crate::{
    enrollment::{bearer, native_only, now},
    prekeys::{active, authorize},
    store::{Store, StoreError},
    store_error, with_store, AppState,
};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, put},
    Json, Router,
};
use rusqlite::{Connection, TransactionBehavior};
use sigil_protocol::accounts::valid_credential;

pub(crate) const MIGRATION: &str = "
CREATE TABLE allowed_senders (
 recipient TEXT NOT NULL REFERENCES devices(id), sender TEXT NOT NULL REFERENCES devices(id),
 PRIMARY KEY(recipient,sender)
);
UPDATE mailbox SET payload=NULL WHERE sender IN (SELECT id FROM devices WHERE account_id != (SELECT account_id FROM devices WHERE id=mailbox.recipient));
";

pub(crate) fn check(db: &Connection, sender: &str, recipient: &str) -> Result<(), StoreError> {
    let allowed:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM allowed_senders WHERE recipient=?1 AND sender=?2) OR EXISTS(SELECT 1 FROM devices a JOIN devices b ON a.account_id=b.account_id WHERE a.id=?1 AND b.id=?2)",(recipient,sender),|r|r.get(0))?;
    if allowed {
        Ok(())
    } else {
        Err(StoreError::Forbidden)
    }
}

pub(crate) fn grant(db: &Connection, sender: &str, recipient: &str) -> Result<(), StoreError> {
    match check(db, sender, recipient) {
        Ok(()) => return Ok(()),
        Err(StoreError::Forbidden) => {}
        Err(e) => return Err(e),
    }
    let count: u32 = db.query_row(
        "SELECT count(*) FROM allowed_senders WHERE recipient=?1",
        [recipient],
        |r| r.get(0),
    )?;
    if count >= 4096 {
        return Err(StoreError::Busy);
    }
    db.execute(
        "INSERT INTO allowed_senders VALUES(?1,?2)",
        (recipient, sender),
    )?;
    Ok(())
}

impl Store {
    pub fn allow_sender(
        &mut self,
        credential: &str,
        sender: &str,
        now: u64,
    ) -> Result<(), StoreError> {
        if !valid_credential(sender) {
            return Err(StoreError::Invalid("invalid sender device"));
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let recipient = authorize(&tx, credential, now)?;
        if !active(&tx, sender)? {
            return Err(StoreError::NotFound);
        }
        grant(&tx, sender, &recipient)?;
        tx.commit()?;
        Ok(())
    }

    /// Removes permission and discards pending payloads from this sender atomically.
    /// Same-account devices use implicit authorization and must instead be revoked.
    pub fn remove_sender(
        &mut self,
        credential: &str,
        sender: &str,
        now: u64,
    ) -> Result<(), StoreError> {
        if !valid_credential(sender) {
            return Err(StoreError::Invalid("invalid sender device"));
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let recipient = authorize(&tx, credential, now)?;
        let same:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM devices WHERE id=?1 AND account_id=(SELECT account_id FROM devices WHERE id=?2))",(sender,&recipient),|r|r.get(0))?;
        if same {
            return Err(StoreError::Invalid(
                "revoke same-account devices through device authorization",
            ));
        }
        tx.execute(
            "DELETE FROM allowed_senders WHERE recipient=?1 AND sender=?2",
            (&recipient, sender),
        )?;
        tx.execute("UPDATE mailbox SET payload=NULL WHERE recipient=?1 AND sender=?2 AND payload IS NOT NULL",(&recipient,sender))?;
        tx.commit()?;
        Ok(())
    }

    pub fn allowed_senders(
        &mut self,
        credential: &str,
        now: u64,
    ) -> Result<Vec<String>, StoreError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let recipient = authorize(&tx, credential, now)?;
        let values = {
            let mut stmt = tx.prepare(
                "SELECT sender FROM allowed_senders WHERE recipient=?1 ORDER BY sender LIMIT 4096",
            )?;
            let rows = stmt
                .query_map([recipient], |r| r.get(0))?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        tx.commit()?;
        Ok(values)
    }
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/client/v0/mailbox/senders", get(list))
        .route("/client/v0/mailbox/senders/{id}", put(allow).delete(remove))
        .route_layer(middleware::from_fn(native_only))
        .layer(tower_http::limit::RequestBodyLimitLayer::new(
            sigil_protocol::MAX_ADMIN_BODY,
        ))
}
async fn list(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let token = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    match with_store(state, move |store| store.allowed_senders(&token, now()?)).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn allow(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let token = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    match with_store(state, move |store| store.allow_sender(&token, &id, now()?)).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => store_error(e),
    }
}
async fn remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let token = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    match with_store(state, move |store| store.remove_sender(&token, &id, now()?)).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => store_error(e),
    }
}
