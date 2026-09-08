use crate::{
    admission,
    auth::digest,
    enrollment::{bearer, native_only, now},
    error,
    prekeys::{active, authorize},
    store::{Store, StoreError},
    store_error, with_store, AppState,
};
use axum::{
    extract::{rejection::JsonRejection, Path, State},
    http::{HeaderMap, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{delete, post},
    Json, Router,
};
use rusqlite::{OptionalExtension, TransactionBehavior};
use sigil_protocol::{
    accounts::valid_credential,
    contacts::{ContactInvite, ContactPeer, CreateContactInvite, RedeemContactInvite},
};

pub(crate) const MIGRATION: &str = "
CREATE TABLE contact_invitations (
 id TEXT PRIMARY KEY, owner TEXT NOT NULL REFERENCES devices(id), expires_at INTEGER NOT NULL,
 claimant TEXT REFERENCES devices(id), revoked INTEGER NOT NULL DEFAULT 0 CHECK(revoked IN (0,1))
);
CREATE INDEX contact_invites_owner ON contact_invitations(owner);
";

fn identifier(secret: &str) -> Result<String, StoreError> {
    if !valid_credential(secret) {
        return Err(StoreError::Invalid(
            "contact invitation secret must be random 256-bit lowercase hex",
        ));
    }
    Ok(digest(secret).iter().map(|b| format!("{b:02x}")).collect())
}

impl Store {
    /// The client persists the random secret first. Only its hash is stored.
    pub fn create_contact_invite(
        &mut self,
        credential: &str,
        request: CreateContactInvite,
        now: u64,
    ) -> Result<ContactInvite, StoreError> {
        let id = identifier(&request.secret)?;
        if request.expires_at <= now
            || request.expires_at > now.saturating_add(604800)
            || request.expires_at > i64::MAX as u64
        {
            return Err(StoreError::Invalid(
                "contact invitation expiry must be within seven days",
            ));
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let owner = authorize(&tx, credential, now)?;
        let old: Option<(String, i64)> = tx
            .query_row(
                "SELECT owner,expires_at FROM contact_invitations WHERE id=?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((previous, expiry)) = old {
            if previous != owner || expiry != request.expires_at as i64 {
                return Err(StoreError::AlreadyExists);
            }
            return Ok(ContactInvite {
                id,
                device_id: owner,
                expires_at: expiry as u64,
            });
        }
        let (total,pending):(u32,u32)=tx.query_row("SELECT count(*),coalesce(sum(claimant IS NULL AND revoked=0 AND expires_at>?2),0) FROM contact_invitations WHERE owner=?1",(&owner,now as i64),|r|Ok((r.get(0)?,r.get(1)?)))?;
        if total >= 4096 || pending >= 16 {
            return Err(StoreError::Busy);
        }
        crate::storage_budget::for_device(&tx, &owner, crate::storage_budget::CONTACT, now)?;
        tx.execute(
            "INSERT INTO contact_invitations(id,owner,expires_at) VALUES(?1,?2,?3)",
            (&id, &owner, request.expires_at as i64),
        )?;
        tx.commit()?;
        Ok(ContactInvite {
            id,
            device_id: owner,
            expires_at: request.expires_at,
        })
    }

    pub fn redeem_contact_invite(
        &mut self,
        credential: &str,
        secret: &str,
        now: u64,
    ) -> Result<ContactPeer, StoreError> {
        let id = identifier(secret)?;
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let claimant = authorize(&tx, credential, now)?;
        let invite:Option<(String,Option<String>)>=tx.query_row("SELECT owner,claimant FROM contact_invitations WHERE id=?1 AND revoked=0 AND expires_at>?2",(&id,now as i64),|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let (owner, previous) = invite.ok_or(StoreError::NotFound)?;
        if owner == claimant {
            return Err(StoreError::Invalid(
                "cannot redeem your own contact invitation",
            ));
        }
        if !active(&tx, &owner)? {
            return Err(StoreError::NotFound);
        }
        if let Some(previous) = previous {
            if previous != claimant {
                return Err(StoreError::NotFound);
            }
            // A retry must never silently undo a later permission removal.
            admission::check(&tx, &claimant, &owner)?;
            admission::check(&tx, &owner, &claimant)?;
            return Ok(ContactPeer { device_id: owner });
        }
        admission::grant(&tx, &claimant, &owner, now)?;
        admission::grant(&tx, &owner, &claimant, now)?;
        tx.execute(
            "UPDATE contact_invitations SET claimant=?1 WHERE id=?2",
            (&claimant, &id),
        )?;
        tx.commit()?;
        Ok(ContactPeer { device_id: owner })
    }

    /// Cancels redemption/retry; established sender grants are removed separately.
    pub fn revoke_contact_invite(
        &mut self,
        credential: &str,
        id: &str,
        now: u64,
    ) -> Result<(), StoreError> {
        if !valid_credential(id) {
            return Err(StoreError::Invalid("invalid contact invitation ID"));
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let owner = authorize(&tx, credential, now)?;
        if tx.execute(
            "UPDATE contact_invitations SET revoked=1 WHERE id=?1 AND owner=?2",
            (id, owner),
        )? == 0
        {
            return Err(StoreError::NotFound);
        }
        tx.commit()?;
        Ok(())
    }
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/client/v0/contact-invitations", post(create))
        .route("/client/v0/contact-invitations/redeem", post(redeem))
        .route("/client/v0/contact-invitations/{id}", delete(revoke))
        .route_layer(middleware::from_fn(native_only))
        .layer(tower_http::limit::RequestBodyLimitLayer::new(
            sigil_protocol::MAX_ADMIN_BODY,
        ))
}
async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<CreateContactInvite>, JsonRejection>,
) -> Response {
    let token = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    let request = match body {
        Ok(Json(v)) => v,
        Err(e) => return error(e.status(), "invalid_request", "Invalid contact invitation"),
    };
    match with_store(state, move |store| {
        store.create_contact_invite(&token, request, now()?)
    })
    .await
    {
        Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
        Err(e) => store_error(e),
    }
}
async fn redeem(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<RedeemContactInvite>, JsonRejection>,
) -> Response {
    let token = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    let request = match body {
        Ok(Json(v)) => v,
        Err(e) => return error(e.status(), "invalid_request", "Invalid contact invitation"),
    };
    match with_store(state, move |store| {
        store.redeem_contact_invite(&token, &request.secret, now()?)
    })
    .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
async fn revoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let token = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return store_error(e),
    };
    match with_store(state, move |store| {
        store.revoke_contact_invite(&token, &id, now()?)
    })
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => store_error(e),
    }
}
