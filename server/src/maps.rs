use crate::{
    prekeys::authorize,
    store::{Store, StoreError},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub archive: String,
    pub assets: String,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub revision: u64,
    pub settings: Option<Settings>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configure {
    pub expected_revision: u64,
    pub settings: Option<Settings>,
}
pub(crate) const MIGRATION:&str="CREATE TABLE map_configuration(id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL CHECK(revision>=0),settings TEXT CHECK(length(settings)<=4096)); INSERT INTO map_configuration VALUES(1,0,NULL);";
fn read(db: &rusqlite::Connection) -> Result<Configuration, StoreError> {
    let (revision,settings):(u64,Option<String>)=db.query_row("SELECT revision,settings FROM map_configuration WHERE id=1 AND (settings IS NULL OR length(settings)<=4096)",[],|r|Ok((crate::push_config::unsigned(r,0)?,r.get(1)?)))?;
    Ok(Configuration {
        revision,
        settings: settings
            .map(|s| serde_json::from_str(&s).map_err(|_| StoreError::InvalidData))
            .transpose()?,
    })
}
impl Store {
    pub fn map_configuration(&self) -> Result<Configuration, StoreError> {
        read(&self.0)
    }
    pub fn configure_maps(&mut self, request: Configure) -> Result<Configuration, StoreError> {
        if let Some(settings) = &request.settings {
            for path in [&settings.archive, &settings.assets] {
                if path.len() > 1024
                    || path.contains('\0')
                    || !std::path::Path::new(path).is_absolute()
                {
                    return Err(StoreError::Invalid("maps require absolute local paths"));
                }
            }
        }
        let tx = self
            .0
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current = read(&tx)?;
        if current.revision != request.expected_revision {
            return if request.expected_revision.checked_add(1) == Some(current.revision)
                && request.settings == current.settings
            {
                Ok(current)
            } else {
                Err(StoreError::Conflict)
            };
        }
        let revision = crate::push_config::next(current.revision)?;
        let settings = request
            .settings
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| StoreError::InvalidData)?;
        tx.execute(
            "UPDATE map_configuration SET revision=?1,settings=?2 WHERE id=1",
            (crate::push_config::sql(revision)?, settings),
        )?;
        tx.commit()?;
        Ok(Configuration {
            revision,
            settings: request.settings,
        })
    }
    pub fn available_maps(
        &mut self,
        credential: &str,
        now: u64,
    ) -> Result<Configuration, StoreError> {
        let tx = self.0.transaction()?;
        authorize(&tx, credential, now)?;
        read(&tx)
    }
}
