use crate::{
    push_config::{self, next, sql, unsigned},
    store::{Store, StoreError},
};
use rusqlite::{Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sigil_protocol::push::{AndroidConfig, AndroidProvider};

pub(crate) const MIGRATION: &str = "
CREATE TABLE push_android(id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL CHECK(revision>=0),generation INTEGER NOT NULL CHECK(generation>=0),value TEXT CHECK(length(value)<=1024),request_hash BLOB NOT NULL);
INSERT INTO push_android VALUES(1,0,0,NULL,X'');
";

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub revision: u64,
    pub push_revision: u64,
    pub android: Option<AndroidConfig>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Configure {
    pub expected_revision: u64,
    pub expected_push_revision: u64,
    pub android: Option<AndroidConfig>,
}
fn read(db: &Connection) -> Result<Configuration, StoreError> {
    let parent = push_config::read(db)?;
    let (revision, generation, value): (u64, u64, Option<String>) = db.query_row(
        "SELECT revision,generation,value FROM push_android WHERE id=1",
        [],
        |r| Ok((unsigned(r, 0)?, unsigned(r, 1)?, r.get(2)?)),
    )?;
    let android: Option<AndroidConfig> = value
        .map(|v| serde_json::from_str(&v))
        .transpose()
        .map_err(|_| StoreError::InvalidData)?;
    if android.as_ref().is_some_and(|v| !v.valid()) {
        return Err(StoreError::InvalidData);
    }
    let android = android.filter(|v| {
        parent.fcm_generation == generation
            && parent
                .settings
                .fcm
                .as_ref()
                .is_some_and(|c| c.project_id == v.project_id)
    });
    Ok(Configuration {
        revision,
        push_revision: parent.revision,
        android,
    })
}
impl Store {
    pub fn push_android_configuration(&mut self) -> Result<Configuration, StoreError> {
        let tx = self.0.transaction()?;
        read(&tx)
    }
    pub fn configure_push_android(
        &mut self,
        request: Configure,
    ) -> Result<Configuration, StoreError> {
        if self.configuration()?.settings.is_none() {
            return Err(StoreError::Forbidden);
        }
        let hash = push_config::hash(b"Sigil/push-android/v0", &request)?;
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = read(&tx)?;
        if current.push_revision != request.expected_push_revision {
            return Err(StoreError::Conflict);
        }
        if request.expected_revision != current.revision {
            let previous: Vec<u8> = tx.query_row(
                "SELECT request_hash FROM push_android WHERE id=1",
                [],
                |r| r.get(0),
            )?;
            return if request.expected_revision.checked_add(1) == Some(current.revision)
                && previous == hash
            {
                Ok(current)
            } else {
                Err(StoreError::Conflict)
            };
        }
        let parent = push_config::read(&tx)?;
        if let Some(config) = &request.android {
            if !config.valid()
                || !parent
                    .settings
                    .fcm
                    .as_ref()
                    .is_some_and(|c| c.project_id == config.project_id)
            {
                return Err(StoreError::Invalid(
                    "Android Firebase configuration must match the enabled Google project",
                ));
            }
        }
        let value = request
            .android
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| StoreError::InvalidData)?;
        tx.execute(
            "UPDATE push_android SET revision=?1,generation=?2,value=?3,request_hash=?4 WHERE id=1",
            (
                sql(next(current.revision)?)?,
                sql(parent.fcm_generation)?,
                value,
                hash.as_slice(),
            ),
        )?;
        let response = read(&tx)?;
        tx.commit()?;
        Ok(response)
    }
    pub fn push_android(
        &mut self,
        credential: &str,
        now: u64,
    ) -> Result<AndroidProvider, StoreError> {
        let tx = self.0.transaction()?;
        crate::prekeys::authorize(&tx, credential, now)?;
        Ok(AndroidProvider {
            android: read(&tx)?.android,
        })
    }
}
pub(crate) fn reset(db: &Connection) -> Result<(), StoreError> {
    let current = read(db)?;
    db.execute(
        "UPDATE push_android SET revision=?1,generation=0,value=NULL,request_hash=X'' WHERE id=1",
        [sql(next(current.revision)?)?],
    )?;
    Ok(())
}
