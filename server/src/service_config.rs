use crate::{
    egress,
    prekeys::authorize,
    push_config::{next, sql, unsigned},
    store::{Store, StoreError},
};
use serde::{Deserialize, Serialize};
use sigil_protocol::services::{Catalog, Provider, Resolve};
use zeroize::Zeroizing;
#[derive(Clone, Serialize, Deserialize)]
#[serde(
    tag = "action",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum SecretUpdate {
    Keep,
    Clear,
    Set(Zeroizing<String>),
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryUpdate {
    pub provider: Provider,
    pub secret: SecretUpdate,
    pub exceptions: Vec<egress::Exception>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configure {
    pub expected_revision: u64,
    pub per_account_daily: u32,
    pub total_daily: u32,
    pub providers: Vec<EntryUpdate>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub provider: Provider,
    pub has_secret: bool,
    pub exceptions: Vec<egress::Exception>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub revision: u64,
    pub per_account_daily: u32,
    pub total_daily: u32,
    pub providers: Vec<Entry>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredEntry {
    pub provider: Provider,
    pub secret: Option<Zeroizing<String>>,
    pub exceptions: Vec<egress::Exception>,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    per_account_daily: u32,
    total_daily: u32,
    providers: Vec<StoredEntry>,
}
pub(crate) const MIGRATION:&str="CREATE TABLE service_configuration(id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL CHECK(revision>=0),settings TEXT NOT NULL CHECK(length(settings)<=65536)); INSERT INTO service_configuration VALUES(1,0,'{\"per_account_daily\":0,\"total_daily\":0,\"providers\":[]}'); CREATE TABLE service_budgets(account TEXT PRIMARY KEY,day INTEGER NOT NULL,used INTEGER NOT NULL CHECK(used>=0));";
fn read(db: &rusqlite::Connection) -> Result<(u64, Settings), StoreError> {
    let (revision,bytes):(u64,String)=db.query_row("SELECT revision,settings FROM service_configuration WHERE id=1 AND length(settings)<=65536",[],|r|Ok((unsigned(r,0)?,r.get(1)?)))?;
    let bytes = Zeroizing::new(bytes);
    Ok((
        revision,
        serde_json::from_str(&bytes).map_err(|_| StoreError::InvalidData)?,
    ))
}
fn public(revision: u64, settings: Settings) -> Configuration {
    Configuration {
        revision,
        per_account_daily: settings.per_account_daily,
        total_daily: settings.total_daily,
        providers: settings
            .providers
            .into_iter()
            .map(|e| Entry {
                provider: e.provider,
                has_secret: e.secret.is_some(),
                exceptions: e.exceptions,
            })
            .collect(),
    }
}
pub(crate) fn current(db: &rusqlite::Connection, revision: u64) -> Result<(), StoreError> {
    if read(db)?.0 == revision {
        Ok(())
    } else {
        Err(StoreError::Conflict)
    }
}
impl Store {
    pub fn service_configuration(&self) -> Result<Configuration, StoreError> {
        let (revision, settings) = read(&self.0)?;
        Ok(public(revision, settings))
    }
    pub fn configure_services(&mut self, request: Configure) -> Result<Configuration, StoreError> {
        if request.providers.len() > 8
            || request.per_account_daily > 100000
            || request.total_daily > 10000000
            || (!request.providers.is_empty()
                && (request.per_account_daily == 0 || request.total_daily == 0))
        {
            return Err(StoreError::Invalid("invalid service limits"));
        }
        let tx = self
            .0
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (revision, old) = read(&tx)?;
        let mut settings = Settings {
            per_account_daily: request.per_account_daily,
            total_daily: request.total_daily,
            providers: Vec::new(),
        };
        let mut ids = std::collections::BTreeSet::new();
        for entry in request.providers {
            entry.provider.validate().map_err(StoreError::Invalid)?;
            let uri = egress::endpoint(&entry.provider.endpoint)
                .map_err(|_| StoreError::Invalid("invalid provider endpoint"))?;
            if uri.query().is_some() || !ids.insert(entry.provider.id.clone()) {
                return Err(StoreError::Invalid(
                    "provider endpoints must omit queries; IDs must be unique",
                ));
            }
            egress::Policy::new(entry.exceptions.clone())
                .map_err(|_| StoreError::Invalid("invalid provider egress policy"))?;
            let prior = old
                .providers
                .iter()
                .find(|p| p.provider.id == entry.provider.id);
            let secret = match entry.secret {
                SecretUpdate::Keep => {
                    if prior.is_some_and(|p| {
                        p.provider.endpoint != entry.provider.endpoint
                            || p.provider.kind != entry.provider.kind
                    }) {
                        return Err(StoreError::Invalid(
                            "supply or clear credentials when changing an endpoint",
                        ));
                    }
                    prior.and_then(|p| p.secret.clone())
                }
                SecretUpdate::Clear => None,
                SecretUpdate::Set(value) => {
                    if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control)
                    {
                        return Err(StoreError::Invalid("invalid provider credential"));
                    }
                    Some(value)
                }
            };
            settings.providers.push(StoredEntry {
                provider: entry.provider,
                secret,
                exceptions: entry.exceptions,
            });
        }
        let bytes =
            Zeroizing::new(serde_json::to_string(&settings).map_err(|_| StoreError::InvalidData)?);
        if bytes.len() > 65536 {
            return Err(StoreError::Invalid("service configuration too large"));
        }
        if revision != request.expected_revision {
            return if request.expected_revision.checked_add(1) == Some(revision)
                && bytes.as_str()
                    == Zeroizing::new(
                        serde_json::to_string(&old).map_err(|_| StoreError::InvalidData)?,
                    )
                    .as_str()
            {
                Ok(public(revision, settings))
            } else {
                Err(StoreError::Conflict)
            };
        }
        let revision = next(revision)?;
        tx.execute(
            "UPDATE service_configuration SET revision=?1,settings=?2 WHERE id=1",
            (sql(revision)?, bytes.as_str()),
        )?;
        tx.commit()?;
        Ok(public(revision, settings))
    }
    pub fn service_catalog(&mut self, credential: &str, now: u64) -> Result<Catalog, StoreError> {
        let tx = self.0.transaction()?;
        authorize(&tx, credential, now)?;
        let (revision, settings) = read(&tx)?;
        Ok(Catalog {
            revision,
            providers: settings.providers.into_iter().map(|e| e.provider).collect(),
        })
    }
    pub(crate) fn prepare_service(
        &mut self,
        credential: &str,
        request: &Resolve,
        now: u64,
    ) -> Result<StoredEntry, StoreError> {
        request.query.validate().map_err(StoreError::Invalid)?;
        let tx = self
            .0
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let device = authorize(&tx, credential, now)?;
        let account: String = tx.query_row(
            "SELECT account_id FROM devices WHERE id=?1",
            [device],
            |r| r.get(0),
        )?;
        let (revision, settings) = read(&tx)?;
        if revision != request.revision {
            return Err(StoreError::Conflict);
        }
        let entry = settings
            .providers
            .into_iter()
            .find(|p| p.provider.id == request.provider.id)
            .ok_or(StoreError::NotFound)?;
        if entry.provider != request.provider {
            return Err(StoreError::Conflict);
        }
        if !request.query.compatible(entry.provider.kind) {
            return Err(StoreError::Invalid("query does not match provider"));
        }
        let day = sql(now / 86400)?;
        for (scope, limit) in [
            (account, settings.per_account_daily),
            (String::new(), settings.total_daily),
        ] {
            tx.execute("INSERT INTO service_budgets VALUES(?1,?2,0) ON CONFLICT(account) DO UPDATE SET day=excluded.day,used=0 WHERE day<excluded.day",(&scope,day))?;
            if tx.execute(
                "UPDATE service_budgets SET used=used+1 WHERE account=?1 AND day=?2 AND used<?3",
                (&scope, day, limit),
            )? != 1
            {
                return Err(StoreError::Forbidden);
            }
        }
        tx.execute(
            "DELETE FROM service_budgets WHERE day<?1",
            [day.saturating_sub(1)],
        )?;
        tx.commit()?;
        Ok(entry)
    }
}
