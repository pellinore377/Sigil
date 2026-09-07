use crate::{
    egress,
    push_provider::{Fcm, FcmCredentials, Vapid},
    store::{Store, StoreError},
};
use rusqlite::{Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

pub const MAX_BODY: usize = 64 * 1024;
#[derive(Clone, Deserialize, Serialize)]
#[serde(
    tag = "action",
    content = "credentials",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum FcmUpdate {
    Keep,
    Disable,
    Configure(FcmCredentials),
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Configure {
    pub expected_revision: u64,
    pub unified_push: bool,
    pub contact: Option<String>,
    pub exceptions: Vec<egress::Exception>,
    pub rotate_vapid: bool,
    pub fcm: FcmUpdate,
}
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub revision: u64,
    pub unified_push: bool,
    pub contact: Option<String>,
    pub exceptions: Vec<egress::Exception>,
    pub vapid_public_key: Option<String>,
    pub fcm_project_id: Option<String>,
    pub fcm_client_email: Option<String>,
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    pub unified: bool,
    pub contact: Option<String>,
    pub exceptions: Vec<egress::Exception>,
    pub vapid: Option<Zeroizing<Vec<u8>>>,
    pub fcm: Option<FcmCredentials>,
}
pub(crate) struct Stored {
    pub revision: u64,
    pub unified_generation: u64,
    pub fcm_generation: u64,
    pub fcm_not_before: u64,
    pub settings: Settings,
    hash: Vec<u8>,
}
pub(crate) const MIGRATION: &str = "
CREATE TABLE push_configuration(id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL CHECK(revision>=0),unified_generation INTEGER NOT NULL CHECK(unified_generation>=0),fcm_generation INTEGER NOT NULL CHECK(fcm_generation>=0),settings TEXT NOT NULL CHECK(length(settings)<=65536),request_hash BLOB NOT NULL,fcm_not_before INTEGER NOT NULL CHECK(fcm_not_before>=0));
INSERT INTO push_configuration VALUES(1,0,0,0,'{\"unified\":false,\"contact\":null,\"exceptions\":[],\"vapid\":null,\"fcm\":null}',X'',0);
";
pub(crate) fn read(db: &Connection) -> Result<Stored, StoreError> {
    let (revision, unified_generation, fcm_generation, settings, hash, fcm_not_before): (u64,u64,u64,String,Vec<u8>,u64) = db.query_row(
        "SELECT revision,unified_generation,fcm_generation,settings,request_hash,fcm_not_before FROM push_configuration WHERE id=1 AND length(settings)<=65536",
        [], |r| Ok((unsigned(r,0)?,unsigned(r,1)?,unsigned(r,2)?,r.get(3)?,r.get(4)?,unsigned(r,5)?)))?;
    let settings = Zeroizing::new(settings);
    let settings: Settings =
        serde_json::from_str(&settings).map_err(|_| StoreError::InvalidData)?;
    if settings.unified && (settings.vapid.is_none() || settings.contact.is_none()) {
        return Err(StoreError::InvalidData);
    }
    Ok(Stored {
        revision,
        unified_generation,
        fcm_generation,
        fcm_not_before,
        settings,
        hash,
    })
}
pub(crate) fn unsigned(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    u64::try_from(row.get::<_, i64>(index)?).map_err(|_| rusqlite::Error::InvalidQuery)
}
pub(crate) fn optional_unsigned(
    row: &rusqlite::Row<'_>,
    index: usize,
) -> rusqlite::Result<Option<u64>> {
    row.get::<_, Option<i64>>(index)?
        .map(|v| u64::try_from(v).map_err(|_| rusqlite::Error::InvalidQuery))
        .transpose()
}
pub(crate) fn sql(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::InvalidData)
}
impl Stored {
    pub(crate) fn generation(&self, provider: u8) -> Result<u64, StoreError> {
        match provider {
            0 if self.settings.fcm.is_some() => Ok(self.fcm_generation),
            1 if self.settings.unified => Ok(self.unified_generation),
            _ => Err(StoreError::Forbidden),
        }
    }
    fn public(&self) -> Result<Configuration, StoreError> {
        Ok(Configuration {
            revision: self.revision,
            unified_push: self.settings.unified,
            contact: self.settings.contact.clone(),
            exceptions: self.settings.exceptions.clone(),
            vapid_public_key: self
                .settings
                .vapid
                .as_ref()
                .map(|k| Vapid::from_pkcs8(k).map(|v| v.public_key()))
                .transpose()
                .map_err(|_| StoreError::InvalidData)?,
            fcm_project_id: self.settings.fcm.as_ref().map(|c| c.project_id.clone()),
            fcm_client_email: self.settings.fcm.as_ref().map(|c| c.client_email.clone()),
        })
    }
}
pub(crate) fn next(value: u64) -> Result<u64, StoreError> {
    value
        .checked_add(1)
        .filter(|v| *v <= i64::MAX as u64)
        .ok_or(StoreError::InvalidData)
}
pub(crate) fn hash<T: Serialize>(domain: &[u8], value: &T) -> Result<[u8; 32], StoreError> {
    let bytes = Zeroizing::new(serde_json::to_vec(value).map_err(|_| StoreError::InvalidData)?);
    let mut hash = Sha256::new();
    hash.update(domain);
    hash.update(bytes.as_slice());
    Ok(hash.finalize().into())
}
impl Store {
    pub fn push_configuration(&self) -> Result<Configuration, StoreError> {
        read(&self.0)?.public()
    }
    pub fn configure_push(&mut self, request: Configure) -> Result<Configuration, StoreError> {
        if self.configuration()?.settings.is_none() {
            return Err(StoreError::Forbidden);
        }
        let request_hash = hash(b"Sigil/push-configuration/v0", &request)?;
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut stored = read(&tx)?;
        if request.expected_revision != stored.revision {
            return if request.expected_revision.checked_add(1) == Some(stored.revision)
                && stored.hash == request_hash
            {
                stored.public()
            } else {
                Err(StoreError::Conflict)
            };
        }
        egress::Policy::new(request.exceptions.clone())
            .map_err(|_| StoreError::Invalid("invalid push egress policy"))?;
        if let Some(contact) = &request.contact {
            crate::push_provider::validate_contact(contact)
                .map_err(|_| StoreError::Invalid("invalid VAPID contact"))?;
        }
        if request.unified_push && request.contact.is_none() {
            return Err(StoreError::Invalid("UnifiedPush requires a VAPID contact"));
        }
        let old_unified = stored.settings.unified;
        let old_project = stored.settings.fcm.as_ref().map(|c| c.project_id.clone());
        stored.settings.unified = request.unified_push;
        stored.settings.contact = request.contact;
        stored.settings.exceptions = request.exceptions;
        match request.fcm {
            FcmUpdate::Keep => {}
            FcmUpdate::Disable => stored.settings.fcm = None,
            FcmUpdate::Configure(credentials) => {
                Fcm::new(&credentials).map_err(|_| {
                    StoreError::Invalid("invalid FCM service-account configuration")
                })?;
                stored.settings.fcm = Some(credentials);
            }
        }
        if request.rotate_vapid || (stored.settings.unified && stored.settings.vapid.is_none()) {
            stored.settings.vapid = Some(Vapid::generate().map_err(|_| StoreError::InvalidData)?);
            stored.unified_generation = next(stored.unified_generation)?;
        } else if old_unified && !stored.settings.unified {
            stored.unified_generation = next(stored.unified_generation)?;
        }
        if old_project.as_deref() != stored.settings.fcm.as_ref().map(|c| c.project_id.as_str()) {
            stored.fcm_generation = next(stored.fcm_generation)?;
            stored.fcm_not_before = 0;
        }
        stored.revision = next(stored.revision)?;
        let settings = Zeroizing::new(
            serde_json::to_string(&stored.settings).map_err(|_| StoreError::InvalidData)?,
        );
        if settings.len() > MAX_BODY {
            return Err(StoreError::Invalid("push configuration is too large"));
        }
        tx.execute("UPDATE push_configuration SET revision=?1,unified_generation=?2,fcm_generation=?3,settings=?4,request_hash=?5,fcm_not_before=?6 WHERE id=1",
            (sql(stored.revision)?,sql(stored.unified_generation)?,sql(stored.fcm_generation)?,settings.as_str(),request_hash.as_slice(),sql(stored.fcm_not_before)?))?;
        let response = stored.public()?;
        tx.commit()?;
        Ok(response)
    }
}
/// Restored capability snapshots must never resume provider side effects.
pub(crate) fn reset_after_restore(db: &Connection) -> Result<(), StoreError> {
    let previous = read(db)?;
    let empty = Zeroizing::new(
        serde_json::to_string(&Settings::default()).map_err(|_| StoreError::InvalidData)?,
    );
    db.execute("UPDATE push_configuration SET revision=?1,unified_generation=?2,fcm_generation=?3,settings=?4,request_hash=X'',fcm_not_before=0 WHERE id=1",
        (sql(next(previous.revision)?)?,sql(next(previous.unified_generation)?)?,sql(next(previous.fcm_generation)?)?,empty.as_str()))?;
    Ok(())
}
