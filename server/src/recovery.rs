use crate::{
    enrollment::{bearer, native_only, now},
    error,
    prekeys::authorize,
    store::{Store, StoreError},
    store_error, with_store, AppState,
};
use axum::{
    extract::{rejection::JsonRejection, Path, State},
    http::{HeaderMap, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};
use sigil_protocol::{
    accounts::valid_credential,
    recovery::{
        DeleteObjects, Head, PublishHead, PutObject, MAX_BODY, MAX_DELETE_OBJECTS, MAX_OBJECT_BYTES,
    },
};
use std::collections::BTreeSet;
use tower_http::limit::RequestBodyLimitLayer;

const MAX_OBJECTS: i64 = 262144;
pub(crate) const MIGRATION: &str = "
CREATE TABLE recovery_objects(account_id TEXT NOT NULL REFERENCES accounts(id), id TEXT NOT NULL,
 data BLOB, PRIMARY KEY(account_id,id));
CREATE TABLE recovery_heads(account_id TEXT PRIMARY KEY REFERENCES accounts(id), generation INTEGER NOT NULL CHECK(generation>0),
 manifest TEXT NOT NULL, previous_manifest TEXT, restored_checkpoint INTEGER NOT NULL DEFAULT 0 CHECK(restored_checkpoint IN (0,1)),
 FOREIGN KEY(account_id,manifest) REFERENCES recovery_objects(account_id,id));
";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn decode(value: &str) -> Result<Vec<u8>, StoreError> {
    if !(72..=MAX_OBJECT_BYTES * 2).contains(&value.len())
        || !value.len().is_multiple_of(2)
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(StoreError::Invalid("invalid encrypted recovery object"));
    }
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            u8::from_str_radix(
                std::str::from_utf8(pair).map_err(|_| StoreError::InvalidData)?,
                16,
            )
            .map_err(|_| StoreError::InvalidData)
        })
        .collect()
}
fn account(db: &Connection, credential: &str, now: u64) -> Result<String, StoreError> {
    let device = authorize(db, credential, now)?;
    Ok(db.query_row(
        "SELECT account_id FROM devices WHERE id=?1",
        [device],
        |r| r.get(0),
    )?)
}
fn read_head(db: &Connection, account: &str) -> Result<Head, StoreError> {
    Ok(db.query_row("SELECT generation,manifest,restored_checkpoint FROM recovery_heads WHERE account_id=?1", [account], |r|
        Ok(Head { generation: r.get::<_, i64>(0)? as u64, manifest: Some(r.get(1)?), restored_checkpoint: r.get(2)? })).optional()?.unwrap_or_default())
}

pub(crate) fn used(db: &Connection, account: &str, now: u64) -> Result<u64, StoreError> {
    let value: i64 = db.query_row("SELECT
        coalesce((SELECT bytes FROM retained_storage WHERE account_id=?1),0) +
        (SELECT coalesce(sum(length(data)),0) FROM recovery_objects WHERE account_id=?1) +
        (SELECT coalesce(sum(reserved_bytes+stored_bytes),0) FROM attachments WHERE account_id=?1) +
        (SELECT coalesce(sum(length(m.payload)),0) FROM mailbox m JOIN devices d ON d.id=m.recipient WHERE d.account_id=?1 AND m.payload IS NOT NULL AND m.expires_at>?2)",
        (account, now as i64), |r| r.get(0))?;
    value.try_into().map_err(|_| StoreError::InvalidData)
}

impl Store {
    pub fn account_storage(
        &mut self,
        credential: &str,
        now: u64,
    ) -> Result<sigil_protocol::recovery::StorageStatus, StoreError> {
        let tx = self.0.transaction()?;
        let account = account(&tx, credential, now)?;
        let quota = crate::admin::quota(&tx, &account)?;
        let status = sigil_protocol::recovery::StorageStatus {
            used_bytes: used(&tx, &account, now)?,
            quota_bytes: quota,
            recovery_objects: tx.query_row(
                "SELECT count(*) FROM recovery_objects WHERE account_id=?1",
                [&account],
                |r| r.get::<_, i64>(0),
            )? as u64,
            recovery_object_limit: MAX_OBJECTS as u64,
        };
        tx.commit()?;
        Ok(status)
    }
    pub fn put_recovery_object(
        &mut self,
        credential: &str,
        id: &str,
        request: PutObject,
        now: u64,
    ) -> Result<(), StoreError> {
        if !valid_credential(id) {
            return Err(StoreError::Invalid("invalid recovery object identifier"));
        }
        let bytes = decode(&request.ciphertext)?;
        if hex(&Sha256::digest(&bytes)) != id {
            return Err(StoreError::Invalid(
                "recovery identifier must hash the ciphertext",
            ));
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let account = account(&tx, credential, now)?;
        let previous: Option<Option<Vec<u8>>> = tx
            .query_row(
                "SELECT data FROM recovery_objects WHERE account_id=?1 AND id=?2",
                (&account, id),
                |r| r.get(0),
            )
            .optional()?;
        if let Some(previous) = previous {
            if previous.as_deref() == Some(bytes.as_slice()) {
                return Ok(());
            }
            return Err(StoreError::AlreadyExists);
        }
        let quota = crate::admin::quota(&tx, &account)?;
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM recovery_objects WHERE account_id=?1",
            [&account],
            |r| r.get(0),
        )?;
        if count >= MAX_OBJECTS
            || used(&tx, &account, now)?.saturating_add(bytes.len() as u64) > quota
        {
            return Err(StoreError::Busy);
        }
        tx.execute(
            "INSERT INTO recovery_objects VALUES(?1,?2,?3)",
            (&account, id, bytes),
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn recovery_object(
        &mut self,
        credential: &str,
        id: &str,
        now: u64,
    ) -> Result<PutObject, StoreError> {
        if !valid_credential(id) {
            return Err(StoreError::Invalid("invalid recovery object identifier"));
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let account = account(&tx, credential, now)?;
        let bytes: Vec<u8> = tx.query_row("SELECT data FROM recovery_objects WHERE account_id=?1 AND id=?2 AND data IS NOT NULL AND length(data)<=?3", (&account,id,MAX_OBJECT_BYTES as i64), |r| r.get(0)).optional()?.ok_or(StoreError::NotFound)?;
        tx.commit()?;
        Ok(PutObject {
            ciphertext: hex(&bytes),
        })
    }

    pub fn recovery_head(&mut self, credential: &str, now: u64) -> Result<Head, StoreError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let account = account(&tx, credential, now)?;
        let head = read_head(&tx, &account)?;
        tx.commit()?;
        Ok(head)
    }

    pub fn publish_recovery_head(
        &mut self,
        credential: &str,
        request: PublishHead,
        now: u64,
    ) -> Result<Head, StoreError> {
        let generation = request
            .generation()
            .ok_or(StoreError::Invalid("invalid recovery generation"))?;
        if !valid_credential(&request.manifest)
            || request.expected_generation >= i64::MAX as u64
            || request.expected_manifest.as_deref() == Some(&request.manifest)
            || request
                .expected_manifest
                .as_ref()
                .is_some_and(|id| !valid_credential(id))
            || (request.expected_generation == 0) != request.expected_manifest.is_none()
        {
            return Err(StoreError::Invalid("invalid recovery head"));
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let account = account(&tx, credential, now)?;
        let current = read_head(&tx, &account)?;
        if current.restored_checkpoint && !request.acknowledge_restored_checkpoint {
            return Err(StoreError::Conflict);
        }
        if current.generation == generation
            && current.manifest.as_deref() == Some(&request.manifest)
        {
            let previous: Option<String> = tx.query_row(
                "SELECT previous_manifest FROM recovery_heads WHERE account_id=?1",
                [&account],
                |r| r.get(0),
            )?;
            if previous == request.expected_manifest && !current.restored_checkpoint {
                return Ok(current);
            }
        }
        if (request.restore_generation.is_some() && !current.restored_checkpoint)
            || current.generation != request.expected_generation
            || current.manifest != request.expected_manifest
        {
            return Err(StoreError::Conflict);
        }
        if !tx.query_row("SELECT EXISTS(SELECT 1 FROM recovery_objects WHERE account_id=?1 AND id=?2 AND data IS NOT NULL)", (&account,&request.manifest), |r| r.get::<_, bool>(0))? {
            return Err(StoreError::NotFound);
        }
        tx.execute("INSERT INTO recovery_heads VALUES(?1,?2,?3,?4,0) ON CONFLICT(account_id) DO UPDATE SET generation=excluded.generation,manifest=excluded.manifest,previous_manifest=excluded.previous_manifest,restored_checkpoint=0",
            (&account,generation as i64,&request.manifest,&request.expected_manifest))?;
        tx.commit()?;
        Ok(Head {
            generation,
            manifest: Some(request.manifest),
            restored_checkpoint: false,
        })
    }

    /// Clients must establish that these objects are unreferenced. Encrypted
    /// child references are intentionally unavailable to the server.
    pub fn delete_recovery_objects(
        &mut self,
        credential: &str,
        request: DeleteObjects,
        now: u64,
    ) -> Result<(), StoreError> {
        if request.expected_generation == 0
            || request.expected_generation > i64::MAX as u64
            || !valid_credential(&request.expected_manifest)
            || request.objects.is_empty()
            || request.objects.len() > MAX_DELETE_OBJECTS
            || request
                .objects
                .iter()
                .any(|id| !valid_credential(id) || *id == request.expected_manifest)
            || request.objects.iter().collect::<BTreeSet<_>>().len() != request.objects.len()
        {
            return Err(StoreError::Invalid("invalid recovery object deletion"));
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let account = account(&tx, credential, now)?;
        let current = read_head(&tx, &account)?;
        if current.generation != request.expected_generation
            || current.manifest.as_deref() != Some(&request.expected_manifest)
            || current.restored_checkpoint
        {
            return Err(StoreError::Conflict);
        }
        for id in &request.objects {
            if tx.execute(
                "UPDATE recovery_objects SET data=NULL WHERE account_id=?1 AND id=?2",
                (&account, id),
            )? == 0
            {
                return Err(StoreError::NotFound);
            }
        }
        tx.commit()?;
        Ok(())
    }
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/client/v0/recovery/objects/{id}",
            get(get_object).put(put_object),
        )
        .route("/client/v0/recovery/head", get(get_head).put(publish_head))
        .route("/client/v0/storage", get(get_storage))
        .route("/client/v0/recovery/objects/delete", post(delete_objects))
        .layer(RequestBodyLimitLayer::new(MAX_BODY))
        .route_layer(middleware::from_fn(native_only))
}
async fn get_storage(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let token = match bearer(&headers) {
        Ok(token) => token,
        Err(error) => return store_error(error),
    };
    match with_store(state, move |store| store.account_storage(&token, now()?)).await {
        Ok(status) => Json(status).into_response(),
        Err(error) => store_error(error),
    }
}
async fn get_object(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let token = match bearer(&headers) {
        Ok(value) => value,
        Err(e) => return store_error(e),
    };
    match with_store(state, move |store| {
        store.recovery_object(&token, &id, now()?)
    })
    .await
    {
        Ok(value) => Json(value).into_response(),
        Err(e) => store_error(e),
    }
}
async fn put_object(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Result<Json<PutObject>, JsonRejection>,
) -> Response {
    let token = match bearer(&headers) {
        Ok(value) => value,
        Err(e) => return store_error(e),
    };
    let request = match body {
        Ok(Json(value)) => value,
        Err(e) => return error(e.status(), "invalid_request", "Invalid recovery object"),
    };
    match with_store(state, move |store| {
        store.put_recovery_object(&token, &id, request, now()?)
    })
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => store_error(e),
    }
}
async fn get_head(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let token = match bearer(&headers) {
        Ok(value) => value,
        Err(e) => return store_error(e),
    };
    match with_store(state, move |store| store.recovery_head(&token, now()?)).await {
        Ok(value) => Json(value).into_response(),
        Err(e) => store_error(e),
    }
}
async fn publish_head(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<PublishHead>, JsonRejection>,
) -> Response {
    let token = match bearer(&headers) {
        Ok(value) => value,
        Err(e) => return store_error(e),
    };
    let request = match body {
        Ok(Json(value)) => value,
        Err(e) => return error(e.status(), "invalid_request", "Invalid recovery head"),
    };
    match with_store(state, move |store| {
        store.publish_recovery_head(&token, request, now()?)
    })
    .await
    {
        Ok(value) => Json(value).into_response(),
        Err(e) => store_error(e),
    }
}
async fn delete_objects(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<DeleteObjects>, JsonRejection>,
) -> Response {
    let token = match bearer(&headers) {
        Ok(value) => value,
        Err(e) => return store_error(e),
    };
    let request = match body {
        Ok(Json(value)) => value,
        Err(e) => return error(e.status(), "invalid_request", "Invalid recovery deletion"),
    };
    match with_store(state, move |store| {
        store.delete_recovery_objects(&token, request, now()?)
    })
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => store_error(e),
    }
}
