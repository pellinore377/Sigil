use crate::{
    push_config::{next, unsigned},
    service_config::SecretUpdate,
    store::{Store, StoreError},
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use zeroize::Zeroizing;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub bind: SocketAddr,
    pub advertised: SocketAddr,
    pub max_calls: u8,
    pub turn_urls: Vec<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configure {
    pub expected_revision: u64,
    pub settings: Option<Settings>,
    pub turn_secret: SecretUpdate,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub revision: u64,
    pub settings: Option<Settings>,
    pub has_turn_secret: bool,
}
#[derive(Clone)]
pub(crate) struct Stored {
    pub revision: u64,
    pub settings: Option<Settings>,
    pub secret: Option<Zeroizing<String>>,
}
pub(crate) const MIGRATION: &str = "
CREATE TABLE call_configuration(id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL CHECK(revision>=0),settings TEXT CHECK(length(settings)<=4096),secret TEXT CHECK(length(secret)<=256),clock INTEGER NOT NULL CHECK(clock>=0));
INSERT INTO call_configuration VALUES(1,0,NULL,NULL,0);
CREATE TABLE calls(id BLOB PRIMARY KEY CHECK(length(id)=32),device TEXT NOT NULL REFERENCES devices(id),expires INTEGER NOT NULL,closed INTEGER NOT NULL CHECK(closed IN (0,1)),roster BLOB NOT NULL CHECK(length(roster)<=16384));
CREATE TABLE call_connections(call BLOB NOT NULL REFERENCES calls(id) ON DELETE CASCADE,participant BLOB NOT NULL CHECK(length(participant)=32),sequence INTEGER NOT NULL CHECK(sequence>0),digest BLOB NOT NULL CHECK(length(digest)=32),updated INTEGER NOT NULL,PRIMARY KEY(call,participant));
CREATE INDEX calls_expiry ON calls(expires);
";
impl Settings {
    pub fn validate(&self) -> Result<(), StoreError> {
        if self.bind.port() == 0
            || self.advertised.port() == 0
            || self.bind.is_ipv4() != self.advertised.is_ipv4()
            || self.advertised.ip().is_unspecified()
            || self.advertised.ip().is_multicast()
            || self.advertised.ip() == std::net::IpAddr::V4(std::net::Ipv4Addr::BROADCAST)
            || !(1..=8).contains(&self.max_calls)
            || self.turn_urls.len() > 4
        {
            return Err(StoreError::Invalid("invalid calling address or capacity"));
        }
        for url in &self.turn_urls {
            sigil_calls::validate_turn_url(url)
                .map_err(|_| StoreError::Invalid("invalid TURN URL"))?;
        }
        Ok(())
    }
}
pub(crate) fn read(db: &rusqlite::Connection) -> Result<Stored, StoreError> {
    let (revision, settings, secret): (u64,Option<String>,Option<String>) = db.query_row("SELECT revision,settings,secret FROM call_configuration WHERE id=1 AND (settings IS NULL OR length(settings)<=4096) AND (secret IS NULL OR length(secret)<=256)", [], |r|Ok((unsigned(r,0)?,r.get(1)?,r.get(2)?)))?;
    let settings: Option<Settings> = settings
        .map(|s| serde_json::from_str(&s).map_err(|_| StoreError::InvalidData))
        .transpose()?;
    if let Some(v) = &settings {
        v.validate()?;
    }
    if settings.as_ref().is_some_and(|v| !v.turn_urls.is_empty()) != secret.is_some() {
        return Err(StoreError::InvalidData);
    }
    Ok(Stored {
        revision,
        settings,
        secret: secret.map(Zeroizing::new),
    })
}
impl Stored {
    pub(crate) fn public(&self) -> Configuration {
        Configuration {
            revision: self.revision,
            settings: self.settings.clone(),
            has_turn_secret: self.secret.is_some(),
        }
    }
}
impl Store {
    pub fn call_configuration(&self) -> Result<Configuration, StoreError> {
        Ok(read(&self.0)?.public())
    }
    pub fn configure_calls(&mut self, request: Configure) -> Result<Configuration, StoreError> {
        if let Some(v) = &request.settings {
            v.validate()?;
        }
        let tx = self
            .0
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let old = read(&tx)?;
        let secret = match request.turn_secret {
            SecretUpdate::Keep => {
                if old.settings.as_ref().map(|v| &v.turn_urls)
                    != request.settings.as_ref().map(|v| &v.turn_urls)
                {
                    return Err(StoreError::Invalid(
                        "changing TURN URLs requires replacing the secret",
                    ));
                }
                old.secret.clone()
            }
            SecretUpdate::Clear => None,
            SecretUpdate::Set(value) => {
                if !(32..=256).contains(&value.len())
                    || !value.is_ascii()
                    || value
                        .bytes()
                        .any(|b| b.is_ascii_control() || b.is_ascii_whitespace())
                {
                    return Err(StoreError::Invalid(
                        "TURN secret must be 32 to 256 ASCII characters",
                    ));
                }
                Some(value)
            }
        };
        if request
            .settings
            .as_ref()
            .is_some_and(|v| !v.turn_urls.is_empty())
            != secret.is_some()
        {
            return Err(StoreError::Invalid("TURN URLs require a secret"));
        }
        if old.revision != request.expected_revision {
            return if request.expected_revision.checked_add(1) == Some(old.revision)
                && old.settings == request.settings
                && old.secret == secret
            {
                Ok(old.public())
            } else {
                Err(StoreError::Conflict)
            };
        }
        let revision = next(old.revision)?;
        let settings = request
            .settings
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| StoreError::InvalidData)?;
        tx.execute(
            "UPDATE call_configuration SET revision=?1,settings=?2,secret=?3 WHERE id=1",
            (
                crate::push_config::sql(revision)?,
                settings,
                secret.as_ref().map(|v| v.as_str()),
            ),
        )?;
        // Existing calls cannot silently resume after a disable or address/policy change.
        tx.execute("UPDATE calls SET closed=1 WHERE closed=0", [])?;
        tx.execute("DELETE FROM call_connections", [])?;
        tx.commit()?;
        Ok(Stored {
            revision,
            settings: request.settings,
            secret,
        }
        .public())
    }
}
