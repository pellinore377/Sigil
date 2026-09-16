use crate::{
    AppState,
    enrollment::{bearer, native_only, now},
    error,
    prekeys::{active, authorize},
    store::{Store, StoreError},
    store_error, with_store,
};
use axum::{
    Json, Router,
    extract::{Path, RawQuery, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use rusqlite::{OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};
use sigil_protocol::{
    accounts::valid_credential,
    mailbox::{Delivery, MAX_BODY, MAX_PAYLOAD_HEX, Receipt, Submit},
};
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tower_http::limit::RequestBodyLimitLayer;

pub(crate) const WAIT_PATH: &str = "/client/v0/mailbox/wait";
/// Server-side deadline for a wait request; must exceed MAX_WAIT.
pub(crate) const WAIT_DEADLINE: u64 = 35;
pub(crate) const MAX_WAITERS: usize = 512;
const MAX_WAIT: u64 = 25;

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
        self.submit_message_inner(credential, request, None, false, now)
    }
    pub fn submit_recovery_message(
        &mut self,
        credential: &str,
        request: Submit,
        proof: &str,
        now: u64,
    ) -> Result<Receipt, StoreError> {
        self.submit_message_inner(credential, request, Some(proof), false, now)
    }
    /// Silent traffic (receipts, controls, key shares) is stored without a push job.
    pub fn submit_message_silent(
        &mut self,
        credential: &str,
        request: Submit,
        proof: Option<&str>,
        silent: bool,
        now: u64,
    ) -> Result<Receipt, StoreError> {
        self.submit_message_inner(credential, request, proof, silent, now)
    }
    fn submit_message_inner(
        &mut self,
        credential: &str,
        request: Submit,
        proof: Option<&str>,
        silent: bool,
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
        if !active(&tx, &request.recipient_device)? {
            return Err(StoreError::NotFound);
        }
        crate::admission::check(&tx, &sender, &request.recipient_device)?;
        let recovery = proof
            .map(|proof| recovery_reserve(&tx, &sender, &request, proof, now))
            .transpose()?
            .unwrap_or(false);
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
        // A revoked device never collects again, so refuse rather than hold a slot for it.
        let (account, revoked): (String, bool) = tx.query_row(
            "SELECT account_id,revoked FROM devices WHERE id=?1",
            [&request.recipient_device],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if revoked {
            return Err(StoreError::NotFound);
        }
        let used = crate::recovery::used(&tx, &account, now)?;
        let quota = crate::admin::quota(&tx, &account)?;
        let peer_pending: u32 = tx.query_row(
            "SELECT count(*) FROM mailbox WHERE recipient=?1 AND sender=?2 AND payload IS NOT NULL AND expires_at>?3",
            (&request.recipient_device, &sender, now as i64),
            |r| r.get(0),
        )?;
        if pending >= 256 + 4 * u32::from(recovery)
            || used.saturating_add(request.payload.len() as u64) > quota
        {
            return Err(StoreError::MailboxFull);
        }
        // A recipient that stops collecting must not silence the people writing to it,
        // whatever the reason: an old build, a broken session, or a device left off.
        // Retire their oldest undelivered message instead, which was expiring unread
        // anyway, so a live conversation outlives a backlog nobody is collecting.
        let allowance = 64 + u32::from(recovery);
        if peer_pending >= allowance {
            tx.execute(
                "UPDATE mailbox SET payload=NULL,expires_at=0 WHERE sequence IN (SELECT sequence FROM mailbox WHERE recipient=?1 AND sender=?2 AND payload IS NOT NULL AND expires_at>?3 ORDER BY sequence LIMIT ?4)",
                (
                    &request.recipient_device,
                    &sender,
                    now as i64,
                    i64::from(peer_pending + 1 - allowance),
                ),
            )?;
        }
        tx.execute("INSERT INTO mailbox(sender,message_id,recipient,payload,payload_hash,expires_at) VALUES(?1,?2,?3,?4,?5,?6)", (&sender,&request.message_id,&request.recipient_device,&request.payload,hash.as_slice(),request.expires_at as i64))?;
        let sequence = tx.last_insert_rowid();
        if !silent {
            crate::push::enqueue(&tx, &request.recipient_device, now)?;
        }
        tx.commit()?;
        Ok(Receipt {
            sequence,
            expires_at: request.expires_at,
        })
    }

    pub fn mailbox_device(&mut self, credential: &str, now: u64) -> Result<String, StoreError> {
        let tx = self.0.transaction()?;
        authorize(&tx, credential, now)
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
            let mut statement = tx.prepare("SELECT sequence,coalesce(sender,remote_device),message_id,payload,expires_at,remote_server,remote_account FROM mailbox WHERE recipient=?1 AND payload IS NOT NULL AND expires_at>?2 AND sequence>?3 AND (sender IS NOT NULL OR (EXISTS(SELECT 1 FROM federation_senders s WHERE s.grant_id=mailbox.remote_grant) AND NOT EXISTS(SELECT 1 FROM federation_peers p WHERE p.server=mailbox.remote_server AND p.error='retired'))) ORDER BY sequence LIMIT 16")?;
            let rows = statement
                .query_map((&device, now as i64, after), |r| {
                    Ok(Delivery {
                        origin: r
                            .get::<_, Option<String>>(5)?
                            .map(|server| {
                                Ok::<_, rusqlite::Error>(sigil_protocol::federation::RemoteSender {
                                    server,
                                    device: r.get(1)?,
                                    account: r.get(6)?,
                                })
                            })
                            .transpose()?,
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

// One extra packet per pair lets signed recovery proceed without discarding the
// failed original. Account storage quotas and normal sender admission still apply.
fn recovery_reserve(
    tx: &rusqlite::Transaction<'_>,
    sender: &str,
    message: &Submit,
    proof: &str,
    now: u64,
) -> Result<bool, StoreError> {
    use sigil_protocol::{device::SignedBinding, retry::Request};
    let invalid = || StoreError::Invalid("invalid recovery authorization");
    if proof.len() != sigil_protocol::retry::BYTES * 2
        || !proof
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid());
    }
    let bytes = proof
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| {
            u8::from_str_radix(std::str::from_utf8(p).map_err(|_| invalid())?, 16)
                .map_err(|_| invalid())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let request = Request::from_bytes(&bytes).map_err(|_| invalid())?;
    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let id = Sha256::digest(
        [
            b"Sigil/retry-request-id/v0".as_slice(),
            &request.requester,
            &request.target,
            &request.message,
        ]
        .concat(),
    );
    if message.message_id != hex(&id) || message.expires_at != request.expires_at {
        return Err(invalid());
    }
    let (requester, target) = if message.payload == proof {
        (sender, message.recipient_device.as_str())
    } else {
        (message.recipient_device.as_str(), sender)
    };
    let mut identity = None;
    for (device, expected) in [(requester, request.requester), (target, request.target)] {
        let statement: Vec<u8> = tx
            .query_row(
                "SELECT statement FROM device_bindings WHERE device=?1 AND length(statement)<=512",
                [device],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(invalid)?;
        let binding = SignedBinding::from_bytes(&statement)
            .map_err(|_| invalid())?
            .binding;
        if hex(&binding.device) != device
            || Sha256::digest(binding.signing_bytes().map_err(|_| invalid())?).as_slice()
                != expected
        {
            return Err(invalid());
        }
        if device == requester {
            identity = Some(binding.identity);
        }
    }
    sigil_crypto::verify_signature(
        &identity.ok_or_else(invalid)?,
        &request.signing_bytes().map_err(|_| invalid())?,
        &request.signature,
    )
    .map_err(|_| invalid())?;
    Ok(tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM mailbox WHERE sender=?1 AND recipient=?2 AND message_id=?3 AND payload IS NOT NULL AND expires_at>?4)",
        (target, requester, hex(&request.message), now as i64), |r| r.get(0))?)
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/client/v0/messages", post(submit))
        .route("/client/v0/mailbox", get(poll))
        .route(WAIT_PATH, get(wait))
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
    if headers.get_all(sigil_protocol::mailbox::RECOVERY_HEADER).iter().count() > 1 {
        return error(StatusCode::BAD_REQUEST, "invalid_request", "Invalid recovery authorization");
    }
    let proof = match headers.get(sigil_protocol::mailbox::RECOVERY_HEADER) {
        Some(value) => match value.to_str() {
            Ok(value) => Some(value.to_owned()),
            Err(_) => {
                return error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "Invalid recovery authorization",
                );
            }
        },
        None => None,
    };
    let log = state.ciphertext_log.clone();
    let record = log.capture(&request);
    let recipient = request.recipient_device.clone();
    let silent = headers
        .get(sigil_protocol::mailbox::SILENT_HEADER)
        .is_some_and(|value| value == "1");
    match with_store(state.clone(), move |store| {
        store.submit_message_silent(&token, request, proof.as_deref(), silent, now()?)
    })
    .await
    {
        Ok(v) => {
            log.accepted(record, v.sequence);
            let _ = state.mailbox_wake.send(recipient);
            (StatusCode::ACCEPTED, Json(v)).into_response()
        }
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
            );
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
/// Returns like poll, but holds the request until mail beyond `after` exists
/// or `timeout` seconds pass. Waiters use their own slots so ordinary requests
/// stay unaffected; when those run out the empty reply arrives immediately.
async fn wait(
    State(state): State<AppState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Response {
    let token = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    let mut after = 0i64;
    let mut timeout = MAX_WAIT;
    for pair in query.as_deref().unwrap_or("").split('&').filter(|v| !v.is_empty()) {
        let number = |v: &str| {
            (!v.is_empty() && v.len() <= 19 && v.bytes().all(|b| b.is_ascii_digit()))
                .then(|| v.parse::<i64>().ok())
                .flatten()
        };
        match pair.split_once('=') {
            Some(("after", v)) if number(v).is_some() => after = number(v).unwrap_or(0),
            Some(("timeout", v)) if number(v).is_some_and(|t| (1..=MAX_WAIT as i64).contains(&t)) => {
                timeout = number(v).map_or(MAX_WAIT, |t| t as u64);
            }
            _ => return error(StatusCode::BAD_REQUEST, "invalid_request", "Invalid mailbox wait"),
        }
    }
    let empty = || Json(Vec::<Delivery>::new()).into_response();
    let Ok(_slot) = state.wait_slots.clone().try_acquire_owned() else {
        return empty();
    };
    let mut wake = state.mailbox_wake.subscribe();
    let credential = token.clone();
    let device = match with_store(state.clone(), move |store| store.mailbox_device(&credential, now()?)).await {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout);
    loop {
        let credential = token.clone();
        let list = match with_store(state.clone(), move |store| {
            store.mailbox_after(&credential, after, now()?)
        })
        .await
        {
            Ok(v) => v,
            Err(e) => return store_error(e),
        };
        if !list.is_empty() {
            return Json(list).into_response();
        }
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return empty();
            }
            match tokio::time::timeout(remaining, wake.recv()).await {
                Ok(Ok(target)) if target == "*" || target == device => break,
                Ok(Ok(_)) => {}
                Ok(Err(RecvError::Lagged(_))) => break,
                Ok(Err(RecvError::Closed)) | Err(_) => return empty(),
            }
        }
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
