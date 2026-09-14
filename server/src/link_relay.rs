use crate::{
    AppState,
    enrollment::native_only,
    error,
    store::{Store, StoreError},
    store_error, with_store,
};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::post,
};
use rusqlite::{OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};
use sigil_protocol::{
    accounts::valid_credential,
    link::{RelayCreate, RelayExchange, RelayReply},
};
use subtle::ConstantTimeEq;

pub(crate) const MIGRATION: &str = "CREATE TABLE link_relay(id TEXT PRIMARY KEY,owner BLOB NOT NULL,phone BLOB NOT NULL,expires INTEGER NOT NULL,proposal TEXT,response TEXT); CREATE INDEX link_relay_expiry ON link_relay(expires);";
const CAPACITY: i64 = 256;
type RelayRow = (Vec<u8>, Vec<u8>, Option<String>, Option<String>);
fn hash(value: &str) -> Vec<u8> {
    Sha256::digest(value.as_bytes()).to_vec()
}
impl Store {
    pub(crate) fn link_relay_create(
        &mut self,
        value: RelayCreate,
        now: u64,
    ) -> Result<(), StoreError> {
        if ![&value.id, &value.owner, &value.phone]
            .into_iter()
            .all(|v| valid_credential(v))
            || value.owner == value.phone
            || value.expires <= now
            || value.expires > now.saturating_add(600)
            || value.expires > i64::MAX as u64
        {
            return Err(StoreError::Invalid("Invalid link reservation"));
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM link_relay WHERE expires<=?1", [now as i64])?;
        let previous: Option<(Vec<u8>, Vec<u8>, i64)> = tx
            .query_row(
                "SELECT owner,phone,expires FROM link_relay WHERE id=?1",
                [&value.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let owner = hash(&value.owner);
        let phone = hash(&value.phone);
        if let Some((a, b, expires)) = previous {
            return if bool::from(a.ct_eq(&owner))
                && bool::from(b.ct_eq(&phone))
                && expires == value.expires as i64
            {
                Ok(())
            } else {
                Err(StoreError::Conflict)
            };
        }
        if tx.query_row("SELECT count(*) FROM link_relay", [], |r| {
            r.get::<_, i64>(0)
        })? >= CAPACITY
        {
            return Err(StoreError::Busy);
        }
        tx.execute(
            "INSERT INTO link_relay(id,owner,phone,expires) VALUES(?1,?2,?3,?4)",
            (&value.id, owner, phone, value.expires as i64),
        )?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn link_relay_exchange(
        &mut self,
        value: RelayExchange,
        now: u64,
    ) -> Result<RelayReply, StoreError> {
        if !valid_credential(&value.id)
            || !valid_credential(&value.token)
            || value.packet.as_ref().is_some_and(|p| {
                p.len() < 72
                    || p.len() > 10000
                    || !p.len().is_multiple_of(2)
                    || !p
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
            || (value.cancel && value.packet.is_some())
        {
            return Err(StoreError::Invalid("Invalid link packet"));
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row: Option<RelayRow> = tx
            .query_row(
                "SELECT owner,phone,proposal,response FROM link_relay WHERE id=?1 AND expires>?2",
                (&value.id, now as i64),
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let (owner, phone, proposal, response) = row.ok_or(StoreError::NotFound)?;
        let token = hash(&value.token);
        let is_owner = bool::from(token.ct_eq(&owner));
        let is_phone = bool::from(token.ct_eq(&phone));
        if !is_owner && !is_phone {
            return Err(StoreError::Unauthorized);
        }
        if value.cancel {
            tx.execute("DELETE FROM link_relay WHERE id=?1", [&value.id])?;
            tx.commit()?;
            return Ok(RelayReply { packet: None });
        }
        let (existing, other, column) = if is_owner {
            (&response, proposal, "response")
        } else {
            (&proposal, response, "proposal")
        };
        if let Some(packet) = value.packet {
            if existing.as_ref().is_some_and(|old| old != &packet) {
                return Err(StoreError::Conflict);
            }
            if existing.is_none() {
                tx.execute(
                    &format!("UPDATE link_relay SET {column}=?2 WHERE id=?1"),
                    (&value.id, packet),
                )?;
            }
        }
        tx.commit()?;
        Ok(RelayReply { packet: other })
    }
}
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/client/v0/link-relay/create", post(create))
        .route("/client/v0/link-relay/exchange", post(exchange))
        .layer(tower_http::limit::RequestBodyLimitLayer::new(12000))
        .layer(middleware::from_fn(native_only))
}
async fn create(
    State(state): State<AppState>,
    body: Result<Json<RelayCreate>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let value = match body {
        Ok(Json(v)) => v,
        Err(e) => return error(e.status(), "invalid_request", "Invalid link reservation"),
    };
    match with_store(state, move |s| {
        s.link_relay_create(value, crate::enrollment::now()?)
    })
    .await
    {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({}))).into_response(),
        Err(e) => store_error(e),
    }
}
async fn exchange(
    State(state): State<AppState>,
    body: Result<Json<RelayExchange>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let value = match body {
        Ok(Json(v)) => v,
        Err(e) => return error(e.status(), "invalid_request", "Invalid link packet"),
    };
    match with_store(state, move |s| {
        s.link_relay_exchange(value, crate::enrollment::now()?)
    })
    .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_error(e),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn create() -> RelayCreate {
        RelayCreate {
            id: "11".repeat(32),
            owner: "22".repeat(32),
            phone: "33".repeat(32),
            expires: 700,
        }
    }
    fn request(token: &str, packet: Option<&str>) -> RelayExchange {
        RelayExchange {
            id: "11".repeat(32),
            token: token.repeat(32),
            packet: packet.map(str::to_owned),
            cancel: false,
        }
    }
    #[test]
    fn schema_34_upgrade_preserves_existing_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.db");
        let s = Store::open(&path).unwrap();
        s.0.execute_batch("DROP TABLE link_relay; CREATE TABLE synthetic_marker(value TEXT); INSERT INTO synthetic_marker VALUES('preserved'); PRAGMA user_version=34;").unwrap();
        drop(s);
        let mut s = Store::open(&path).unwrap();
        assert_eq!(
            s.0.query_row("SELECT value FROM synthetic_marker", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "preserved"
        );
        assert_eq!(
            s.0.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            crate::store::SCHEMA_VERSION
        );
        s.link_relay_create(create(), 100).unwrap();
        let backup = dir.path().join("backup.db");
        let restored = dir.path().join("restored.db");
        s.backup(&backup).unwrap();
        Store::restore(&backup, &restored).unwrap();
        let mut restored = Store::open(&restored).unwrap();
        assert!(matches!(
            restored.link_relay_exchange(request("22", None), 101),
            Err(StoreError::NotFound)
        ));
    }
    #[test]
    fn capabilities_isolate_both_directions_and_retries_cannot_replace_packets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.db");
        let mut s = Store::open(&path).unwrap();
        s.link_relay_create(create(), 100).unwrap();
        s.link_relay_create(create(), 101).unwrap();
        let packet = "ab".repeat(100);
        assert!(
            s.link_relay_exchange(request("44", Some(&packet)), 101)
                .is_err()
        );
        assert!(
            s.link_relay_exchange(request("33", Some(&packet)), 101)
                .unwrap()
                .packet
                .is_none()
        );
        drop(s);
        let mut s = Store::open(&path).unwrap();
        assert_eq!(
            s.link_relay_exchange(request("22", None), 102)
                .unwrap()
                .packet,
            Some(packet.clone())
        );
        assert!(
            s.link_relay_exchange(request("33", Some(&packet)), 102)
                .unwrap()
                .packet
                .is_none()
        );
        assert!(matches!(
            s.link_relay_exchange(request("33", Some(&"cd".repeat(100))), 102),
            Err(StoreError::Conflict)
        ));
        let reply = "ef".repeat(100);
        s.link_relay_exchange(request("22", Some(&reply)), 102)
            .unwrap();
        assert_eq!(
            s.link_relay_exchange(request("33", None), 102)
                .unwrap()
                .packet,
            Some(reply)
        );
        assert_eq!(
            s.0.query_row("SELECT count(*) FROM devices", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        let mut cancel = request("22", None);
        cancel.cancel = true;
        s.link_relay_exchange(cancel, 103).unwrap();
        assert!(matches!(
            s.link_relay_exchange(request("33", None), 104),
            Err(StoreError::NotFound)
        ));
    }
    #[test]
    fn reservations_and_packets_are_bounded_and_expire() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::open(&dir.path().join("server.db")).unwrap();
        s.link_relay_create(create(), 100).unwrap();
        let mut changed = create();
        changed.owner = "44".repeat(32);
        assert!(s.link_relay_create(changed, 101).is_err());
        assert!(
            s.link_relay_exchange(request("22", Some(&"a".repeat(10002))), 102)
                .is_err()
        );
        assert!(s.link_relay_exchange(request("22", None), 700).is_err());
        let mut renewed = create();
        renewed.expires = 1300;
        s.link_relay_create(renewed, 700).unwrap();
        for i in 1..CAPACITY {
            let mut r = create();
            r.id = format!("{i:064x}");
            r.expires = 1300;
            s.link_relay_create(r, 700).unwrap();
        }
        let mut extra = create();
        extra.id = "ff".repeat(32);
        extra.expires = 1300;
        assert!(matches!(
            s.link_relay_create(extra, 700),
            Err(StoreError::Busy)
        ));
    }
}
