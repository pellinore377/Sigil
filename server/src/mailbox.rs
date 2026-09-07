use crate::{
    enrollment::{bearer, native_only, now},
    error,
    prekeys::{active, authorize},
    store::{Store, StoreError},
    store_error, with_store, AppState,
};
use axum::{
    extract::{rejection::JsonRejection, Path, RawQuery, State},
    http::{HeaderMap, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use rusqlite::{OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};
use sigil_protocol::{
    accounts::valid_credential,
    mailbox::{Delivery, Receipt, Submit, MAX_BODY, MAX_PAYLOAD_HEX},
};
use tower_http::limit::RequestBodyLimitLayer;

pub(crate) const MIGRATION: &str = "
CREATE TABLE mailbox (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
 sender TEXT NOT NULL REFERENCES devices(id), message_id TEXT NOT NULL,
 recipient TEXT NOT NULL REFERENCES devices(id), payload TEXT, payload_hash BLOB NOT NULL,
 expires_at INTEGER NOT NULL, UNIQUE(sender,message_id)
);
CREATE INDEX mailbox_recipient ON mailbox(recipient,sequence);
CREATE INDEX mailbox_expiry ON mailbox(expires_at) WHERE payload IS NOT NULL;
";

impl Store {
    pub fn submit_message(
        &mut self,
        credential: &str,
        request: Submit,
        now: u64,
    ) -> Result<Receipt, StoreError> {
        if !valid_credential(&request.recipient_device)
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
                "invalid encrypted payload, identifier, or expiry (maximum 7 days)",
            ));
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let sender = authorize(&tx, credential, now)?;
        if tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM federation_outbox WHERE sender=?1 AND message_id=?2)",
            (&sender, &request.message_id),
            |r| r.get::<_, bool>(0),
        )? {
            return Err(StoreError::AlreadyExists);
        }
        let quota = crate::store::read_configuration(&tx)?
            .settings
            .ok_or(StoreError::Unauthorized)?
            .default_quota_bytes;
        if !active(&tx, &request.recipient_device)? {
            return Err(StoreError::NotFound);
        }
        crate::admission::check(&tx, &sender, &request.recipient_device)?;
        let hash = Sha256::digest(request.payload.as_bytes());
        let previous: Option<(i64,String,Vec<u8>,i64)> = tx.query_row("SELECT sequence,recipient,payload_hash,expires_at FROM mailbox WHERE sender=?1 AND message_id=?2", (&sender,&request.message_id), |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        if let Some((sequence, recipient, old_hash, expiry)) = previous {
            if recipient != request.recipient_device
                || old_hash != hash.as_slice()
                || expiry != request.expires_at as i64
            {
                return Err(StoreError::AlreadyExists);
            }
            return Ok(Receipt {
                sequence,
                expires_at: expiry as u64,
            });
        }
        crate::storage_budget::for_device(&tx, &sender, crate::storage_budget::MAILBOX, now)?;
        let pending: u32 = tx.query_row(
            "SELECT count(*) FROM mailbox WHERE recipient=?1 AND payload IS NOT NULL AND expires_at>?2",
            (&request.recipient_device,now as i64),
            |r| r.get(0),
        )?;
        let account: String = tx.query_row(
            "SELECT account_id FROM devices WHERE id=?1",
            [&request.recipient_device],
            |r| r.get(0),
        )?;
        let used = crate::recovery::used(&tx, &account, now)?;
        let peer_pending: u32 = tx.query_row(
            "SELECT count(*) FROM mailbox WHERE recipient=?1 AND sender=?2 AND payload IS NOT NULL AND expires_at>?3",
            (&request.recipient_device, &sender, now as i64),
            |r| r.get(0),
        )?;
        if peer_pending >= 64
            || pending >= 256
            || used.saturating_add(request.payload.len() as u64) > quota
        {
            return Err(StoreError::Busy);
        }
        tx.execute("INSERT INTO mailbox(sender,message_id,recipient,payload,payload_hash,expires_at) VALUES(?1,?2,?3,?4,?5,?6)", (&sender,&request.message_id,&request.recipient_device,&request.payload,hash.as_slice(),request.expires_at as i64))?;
        let sequence = tx.last_insert_rowid();
        crate::push::enqueue(&tx, &request.recipient_device, now)?;
        tx.commit()?;
        Ok(Receipt {
            sequence,
            expires_at: request.expires_at,
        })
    }

    /// Reads at most 16 unacknowledged messages. Polling does not consume them.
    pub fn mailbox(&mut self, credential: &str, now: u64) -> Result<Vec<Delivery>, StoreError> {
        self.mailbox_after(credential, 0, now)
    }
    /// Cursor polling never acknowledges skipped messages. Restart at zero to
    /// revisit earlier unacknowledged deliveries after reaching the end.
    pub fn mailbox_after(
        &mut self,
        credential: &str,
        after: i64,
        now: u64,
    ) -> Result<Vec<Delivery>, StoreError> {
        if after < 0 {
            return Err(StoreError::Invalid("Invalid mailbox cursor"));
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let device = authorize(&tx, credential, now)?;
        let values = {
            let mut statement = tx.prepare("SELECT sequence,sender,message_id,payload,expires_at FROM mailbox WHERE recipient=?1 AND sender IS NOT NULL AND payload IS NOT NULL AND expires_at>?2 AND sequence>?3 ORDER BY sequence LIMIT 16")?;
            let rows = statement
                .query_map((&device, now as i64, after), |r| {
                    Ok(Delivery {
                        sequence: r.get(0)?,
                        sender_device: r.get(1)?,
                        message_id: r.get(2)?,
                        payload: r.get(3)?,
                        expires_at: r.get::<_, i64>(4)? as u64,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        tx.commit()?;
        Ok(values)
    }

    /// Client must durably commit authenticated state/message before acknowledging.
    pub fn acknowledge_message(
        &mut self,
        credential: &str,
        sequence: i64,
        now: u64,
    ) -> Result<(), StoreError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let device = authorize(&tx, credential, now)?;
        if !tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM mailbox WHERE sequence=?1 AND recipient=?2)",
            (sequence, device),
            |r| r.get::<_, bool>(0),
        )? {
            return Err(StoreError::NotFound);
        }
        crate::federation_mailbox::release_payload(&tx, sequence)?;
        tx.commit()?;
        Ok(())
    }
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/client/v0/messages", post(submit))
        .route("/client/v0/mailbox", get(poll))
        .route("/client/v0/mailbox/{sequence}", delete(ack))
        .layer(RequestBodyLimitLayer::new(MAX_BODY))
        .route_layer(middleware::from_fn(native_only))
}
async fn submit(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<Submit>, JsonRejection>,
) -> Response {
    let token = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    let request = match body {
        Ok(Json(v)) => v,
        Err(e) => return error(e.status(), "invalid_request", "Invalid message request"),
    };
    match with_store(state, move |store| {
        store.submit_message(&token, request, now()?)
    })
    .await
    {
        Ok(v) => (StatusCode::ACCEPTED, Json(v)).into_response(),
        Err(e) => store_error(e),
    }
}
async fn poll(
    State(state): State<AppState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Response {
    let token = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    let after = match query.as_deref() {
        None | Some("") => Some(0),
        Some(query) => query
            .strip_prefix("after=")
            .filter(|v| !v.is_empty() && v.len() <= 19 && v.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|v| v.parse::<i64>().ok()),
    };
    let after = match after {
        Some(after) => after,
        _ => {
            return error(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "Invalid mailbox cursor",
            )
        }
    };
    match with_store(state, move |store| {
        store.mailbox_after(&token, after, now()?)
    })
    .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn ack(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(sequence): Path<i64>,
) -> Response {
    let token = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    match with_store(state, move |store| {
        store.acknowledge_message(&token, sequence, now()?)
    })
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => store_error(e),
    }
}
