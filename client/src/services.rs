use crate::{ClientStore, Error, Id};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sigil_crypto::storage::StorageKey;
use sigil_protocol::{
    services::{Catalog, Query, Resolve, Resolved, MAX_RESPONSE},
    text::structured::{Card, Construct},
};
use std::sync::atomic::{AtomicBool, Ordering};
use zeroize::Zeroizing;
#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
pub(crate) const MIGRATION:&str="CREATE TABLE service_queries(id BLOB PRIMARY KEY,content BLOB NOT NULL); PRAGMA user_version=66;";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Draft {
    request: Resolve,
    result: Option<Resolved>,
}
pub struct Disclosure {
    pub home_server: String,
    pub request: Resolve,
}
fn index(db: &Connection, key: &StorageKey, id: Id) -> Result<Id, Error> {
    if id == [0; 32] {
        return Err(Error::InvalidEvent);
    }
    let (scope, _) = crate::structured::account_context(db, key)?;
    Ok(key.commitment(
        &[scope.as_slice(), &id].concat(),
        b"Sigil/service-query-index/v1",
    )?)
}
fn aad(id: &Id) -> Vec<u8> {
    [b"Sigil/service-query/v1".as_slice(), id].concat()
}
fn load(db: &Connection, key: &StorageKey, id: &Id) -> Result<Option<Draft>, Error> {
    let bytes:Option<Vec<u8>>=db.query_row("SELECT CASE WHEN length(content)<=327680 THEN content END FROM service_queries WHERE id=?1",[id.as_slice()],|r|r.get(0)).optional()?;
    bytes
        .map(|bytes| {
            let draft: Draft = serde_json::from_slice(&key.open(&bytes, &aad(id))?)
                .map_err(|_| Error::InvalidStore)?;
            draft
                .request
                .query
                .validate()
                .map_err(|_| Error::InvalidStore)?;
            if let Some(result) = &draft.result {
                result
                    .validate_for(&draft.request.provider, &draft.request.query)
                    .map_err(|_| Error::InvalidStore)?;
            }
            Ok(draft)
        })
        .transpose()
}
fn save(db: &Connection, key: &StorageKey, id: &Id, draft: &Draft) -> Result<(), Error> {
    let bytes = Zeroizing::new(serde_json::to_vec(draft).map_err(|_| Error::InvalidEvent)?);
    if bytes.len() > MAX_RESPONSE + 32768 {
        return Err(Error::Limit);
    }
    db.execute("INSERT INTO service_queries VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET content=excluded.content",(id.as_slice(),key.seal(&bytes,&aad(id))?))?;
    Ok(())
}
impl ClientStore {
    /// Creates a durable disclosure. This never contacts a provider or sends a message.
    pub fn prepare_service_query(
        &mut self,
        id: Id,
        catalog: &Catalog,
        provider: &str,
        query: Query,
        refresh: bool,
    ) -> Result<Disclosure, Error> {
        query.validate().map_err(|_| Error::InvalidEvent)?;
        let provider = catalog
            .providers
            .iter()
            .find(|p| p.id == provider)
            .ok_or(Error::NotFound)?
            .clone();
        provider.validate().map_err(|_| Error::InvalidEvent)?;
        if !query.compatible(provider.kind) {
            return Err(Error::InvalidEvent);
        }
        let request = Resolve {
            revision: catalog.revision,
            provider,
            query,
            refresh,
        };
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let key = index(&tx, &self.key, id)?;
        if let Some(old) = load(&tx, &self.key, &key)? {
            if old.request != request {
                return Err(Error::Conflict);
            }
        } else {
            if tx.query_row("SELECT count(*) FROM service_queries", [], |r| {
                r.get::<_, i64>(0)
            })? >= 256
            {
                return Err(Error::Limit);
            }
            save(
                &tx,
                &self.key,
                &key,
                &Draft {
                    request: request.clone(),
                    result: None,
                },
            )?;
        }
        let session = crate::connection::session_in(&tx, &self.key)?.ok_or(Error::Unprepared)?;
        let home_server = session
            .address
            .split_once(':')
            .ok_or(Error::InvalidStore)?
            .1
            .to_owned();
        tx.commit()?;
        Ok(Disclosure {
            home_server,
            request,
        })
    }
    /// Call only after the user accepts the disclosure; run on a worker thread.
    pub fn resolve_service_query(
        &mut self,
        id: Id,
        cancel: &AtomicBool,
    ) -> Result<Resolved, Error> {
        if cancel.load(Ordering::Acquire) {
            return Err(Error::Cancelled);
        }
        let key = index(&self.db, &self.key, id)?;
        let mut draft = load(&self.db, &self.key, &key)?.ok_or(Error::Unprepared)?;
        if let Some(result) = draft.result {
            return Ok(result);
        }
        let result = self.connected_client()?.resolve_service(&draft.request)?;
        if cancel.load(Ordering::Acquire) {
            return Err(Error::Cancelled);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if index(&tx, &self.key, id)? != key {
            return Err(Error::Obsolete);
        }
        let current = load(&tx, &self.key, &key)?.ok_or(Error::Obsolete)?;
        if current.request != draft.request {
            return Err(Error::Conflict);
        }
        if let Some(result) = current.result {
            return Ok(result);
        }
        draft.result = Some(result.clone());
        save(&tx, &self.key, &key, &draft)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn service_card(&self, query: Id, message: Id, created_at: u64) -> Result<Card, Error> {
        let key = index(&self.db, &self.key, query)?;
        let draft = load(&self.db, &self.key, &key)?.ok_or(Error::Unprepared)?;
        let Some(Resolved::Snapshot(snapshot)) = draft.result else {
            return Err(Error::Unprepared);
        };
        if snapshot.resolved_at > created_at {
            return Err(Error::InvalidEvent);
        }
        let card = Card {
            id: message,
            creator: self.account_reference()?,
            created_at,
            content: Construct::Service(snapshot),
        };
        card.to_bytes().map_err(|_| Error::Limit)?;
        Ok(card)
    }
    /// Discard after queuing or abandoning the draft. Refresh requires a new query ID.
    pub fn discard_service_query(&mut self, id: Id) -> Result<(), Error> {
        let key = index(&self.db, &self.key, id)?;
        self.db
            .execute("DELETE FROM service_queries WHERE id=?1", [key.as_slice()])?;
        Ok(())
    }
}
