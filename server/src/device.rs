//! Stores client-signed public statements; peers perform signature/trust checks.
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
use rusqlite::{OptionalExtension, TransactionBehavior};
use sigil_protocol::{
    accounts::valid_credential,
    device::{SignedBinding, Statement},
};
use tower_http::limit::RequestBodyLimitLayer;

pub(crate) const MIGRATION: &str = "CREATE TABLE device_bindings(device TEXT PRIMARY KEY REFERENCES devices(id), statement BLOB NOT NULL);";
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
impl Store {
    pub fn publish_device_binding(
        &mut self,
        credential: &str,
        request: Statement,
        now: u64,
    ) -> Result<(), StoreError> {
        let bytes = request.bytes().map_err(StoreError::Invalid)?;
        let signed = SignedBinding::from_bytes(&bytes).map_err(StoreError::Invalid)?;
        let binding = signed.binding;
        let server = self
            .configuration()?
            .settings
            .ok_or(StoreError::Unauthorized)?
            .server_name;
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let device = authorize(&tx, credential, now)?;
        let (account, username): (String, String) = tx.query_row(
            "SELECT a.id,a.username FROM accounts a JOIN devices d ON d.account_id=a.id WHERE d.id=?1", [&device],
            |r| Ok((r.get(0)?, r.get(1)?)))?;
        if binding.server != server
            || binding.username != username
            || hex(&binding.account) != account
            || hex(&binding.device) != device
        {
            return Err(StoreError::Invalid(
                "device statement does not match the authorized account",
            ));
        }
        let identity: Option<Vec<u8>> = tx
            .query_row(
                "SELECT public_key FROM encryption_identities WHERE device_id=?1",
                [&device],
                |r| r.get(0),
            )
            .optional()?;
        if identity.is_some_and(|v| v != binding.identity) {
            return Err(StoreError::Invalid(
                "encryption identity changes require a new authorized device",
            ));
        }
        let previous: Option<Vec<u8>> = tx
            .query_row(
                "SELECT statement FROM device_bindings WHERE device=?1",
                [&device],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(previous) = previous {
            return if previous == bytes {
                Ok(())
            } else {
                Err(StoreError::AlreadyExists)
            };
        }
        tx.execute(
            "INSERT INTO encryption_identities VALUES(?1,?2) ON CONFLICT(device_id) DO NOTHING",
            (&device, binding.identity.as_slice()),
        )?;
        tx.execute(
            "INSERT INTO device_bindings VALUES(?1,?2)",
            (&device, bytes),
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn device_binding(
        &mut self,
        credential: &str,
        target: &str,
        now: u64,
    ) -> Result<Statement, StoreError> {
        if !valid_credential(target) {
            return Err(StoreError::Invalid("invalid device identifier"));
        }
        let tx = self.0.transaction()?;
        let requester = authorize(&tx, credential, now)?;
        if !active(&tx, target)? {
            return Err(StoreError::NotFound);
        }
        match crate::admission::check(&tx, &requester, target) {
            Ok(()) => {}
            Err(StoreError::Forbidden) => crate::admission::check(&tx, target, &requester)?,
            Err(error) => return Err(error),
        }
        let bytes: Vec<u8> = tx
            .query_row(
                "SELECT statement FROM device_bindings WHERE device=?1 AND length(statement)<=512",
                [target],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(StoreError::NotFound)?;
        Ok(Statement {
            statement: hex(&bytes),
        })
    }
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/client/v0/device-binding", put(publish))
        .route("/client/v0/devices/{id}/binding", get(fetch))
        .route_layer(middleware::from_fn(native_only))
        .layer(RequestBodyLimitLayer::new(2048))
}
async fn publish(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<Statement>,
) -> Response {
    let credential = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    match with_store(state, move |store| {
        store.publish_device_binding(&credential, request, now()?)
    })
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => store_error(e),
    }
}
async fn fetch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let credential = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    match with_store(state, move |store| {
        store.device_binding(&credential, &id, now()?)
    })
    .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
