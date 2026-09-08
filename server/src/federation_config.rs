use crate::{
    egress,
    federation_auth::{self as auth, SigningKey},
    push_config::{sql, unsigned},
    store::{Store, StoreError},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sigil_protocol::federation::{Discovery, Key, Rotation};
use zeroize::Zeroizing;
pub const MAX_BODY: usize = 64 * 1024;
pub(crate) const PEER_METADATA: u64 = 33792;
pub(crate) const METADATA_BUDGET: u64 = 64 * 1024 * 1024;
pub(crate) const MIGRATION:&str="
CREATE TABLE federation_configuration(id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL,settings TEXT NOT NULL,request_hash BLOB NOT NULL);
INSERT INTO federation_configuration VALUES(1,0,'{\"enabled\":false,\"exceptions\":[],\"peer_quota_bytes\":67108864,\"key\":null}',X'');
CREATE TABLE federation_peers(server TEXT PRIMARY KEY,revision INTEGER NOT NULL,allowed INTEGER NOT NULL CHECK(allowed IN(0,1)),port INTEGER NOT NULL,approval TEXT,pinned TEXT,observed TEXT,checked_at INTEGER NOT NULL,not_before INTEGER NOT NULL,error TEXT,request_hash BLOB NOT NULL);
CREATE TABLE federation_nonces(server TEXT NOT NULL REFERENCES federation_peers(server),nonce TEXT NOT NULL,expires_at INTEGER NOT NULL,PRIMARY KEY(server,nonce));
CREATE INDEX federation_peers_refresh ON federation_peers(allowed,not_before,server);
CREATE INDEX federation_nonces_expiry ON federation_nonces(expires_at);
CREATE TABLE federation_admission(server TEXT PRIMARY KEY REFERENCES federation_peers(server),nonce_bytes INTEGER NOT NULL DEFAULT 0 CHECK(nonce_bytes>=0),credit INTEGER NOT NULL DEFAULT 20 CHECK(credit BETWEEN 0 AND 20),updated_at INTEGER NOT NULL DEFAULT 0);
CREATE TABLE federation_usage(id INTEGER PRIMARY KEY CHECK(id=1),nonce_bytes INTEGER NOT NULL CHECK(nonce_bytes>=0));
INSERT INTO federation_usage VALUES(1,0);
";
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Configure {
    pub expected_revision: u64,
    pub enabled: bool,
    pub exceptions: Vec<egress::Exception>,
    pub peer_quota_bytes: u64,
    pub rotate_key: bool,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub revision: u64,
    pub enabled: bool,
    pub exceptions: Vec<egress::Exception>,
    pub peer_quota_bytes: u64,
    pub discovery: Option<Discovery>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Material {
    seed: Zeroizing<Vec<u8>>,
    current: Key,
    rotation: Option<Rotation>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    enabled: bool,
    exceptions: Vec<egress::Exception>,
    peer_quota_bytes: u64,
    key: Option<Material>,
}
pub(crate) struct Stored {
    pub revision: u64,
    pub server: Option<String>,
    pub enabled: bool,
    pub policy: egress::Policy,
    pub peer_quota_bytes: u64,
    pub key: Option<SigningKey>,
    settings: Settings,
    hash: Vec<u8>,
}
impl Stored {
    pub(crate) fn discovery(&self) -> Result<Discovery, StoreError> {
        let server = self.server.clone().ok_or(StoreError::Forbidden)?;
        let material = self.settings.key.as_ref().ok_or(StoreError::Forbidden)?;
        Ok(Discovery {
            version: 0,
            server,
            current: material.current.clone(),
            rotation: material.rotation.clone(),
        })
    }
    fn public(&self) -> Result<Configuration, StoreError> {
        Ok(Configuration {
            revision: self.revision,
            enabled: self.enabled,
            exceptions: self.settings.exceptions.clone(),
            peer_quota_bytes: self.peer_quota_bytes,
            discovery: self
                .settings
                .key
                .as_ref()
                .map(|_| self.discovery())
                .transpose()?,
        })
    }
}
pub(crate) fn read(db: &Connection) -> Result<Stored, StoreError> {
    let(revision,bytes,hash):(u64,String,Vec<u8>)=db.query_row("SELECT revision,CASE WHEN length(settings)<=65536 THEN settings END,request_hash FROM federation_configuration WHERE id=1",[],|r|Ok((unsigned(r,0)?,r.get(1)?,r.get(2)?)))?;
    let bytes = Zeroizing::new(bytes);
    let settings: Settings = serde_json::from_str(&bytes).map_err(|_| StoreError::InvalidData)?;
    let server = crate::store::read_configuration(db)?
        .settings
        .map(|s| s.server_name);
    let policy =
        egress::Policy::new(settings.exceptions.clone()).map_err(|_| StoreError::InvalidData)?;
    if !(1024 * 1024..=1024 * 1024 * 1024).contains(&settings.peer_quota_bytes) {
        return Err(StoreError::InvalidData);
    }
    let key = settings
        .key
        .as_ref()
        .map(|material| {
            let key = SigningKey::from_seed(
                material.seed.clone(),
                material.current.generation,
                material.current.not_before,
            )
            .map_err(|_| StoreError::InvalidData)?;
            if key.descriptor() != &material.current {
                return Err(StoreError::InvalidData);
            }
            let name = server.as_deref().ok_or(StoreError::InvalidData)?;
            auth::validate_discovery(
                &Discovery {
                    version: 0,
                    server: name.into(),
                    current: material.current.clone(),
                    rotation: material.rotation.clone(),
                },
                name,
                material.current.not_before,
            )
            .map_err(|_| StoreError::InvalidData)?;
            Ok(key)
        })
        .transpose()?;
    if settings.enabled && (server.is_none() || key.is_none()) {
        return Err(StoreError::InvalidData);
    }
    Ok(Stored {
        revision,
        server,
        enabled: settings.enabled,
        policy,
        peer_quota_bytes: settings.peer_quota_bytes,
        key,
        settings,
        hash,
    })
}
fn next(value: u64) -> Result<u64, StoreError> {
    value
        .checked_add(1)
        .filter(|v| *v <= i64::MAX as u64)
        .ok_or(StoreError::InvalidData)
}
fn save(db: &Connection, stored: &Stored) -> Result<(), StoreError> {
    let bytes = Zeroizing::new(
        serde_json::to_string(&stored.settings).map_err(|_| StoreError::InvalidData)?,
    );
    if bytes.len() > MAX_BODY {
        return Err(StoreError::Invalid("federation configuration too large"));
    }
    db.execute(
        "UPDATE federation_configuration SET revision=?1,settings=?2,request_hash=?3 WHERE id=1",
        (sql(stored.revision)?, bytes.as_str(), &stored.hash),
    )?;
    Ok(())
}
impl Store {
    pub(crate) fn claim_federation_refresh(
        &mut self,
        now: u64,
    ) -> Result<Option<(Stored, Peer)>, StoreError> {
        let candidate:Option<String>=self.0.query_row("SELECT server FROM federation_peers WHERE allowed=1 AND not_before<=?1 ORDER BY not_before,server LIMIT 1",[sql(now)?],|r|r.get(0)).optional()?;
        if !read(&self.0)?.enabled {
            return Ok(None);
        }
        candidate
            .map(|server| self.begin_federation_refresh(&server, now))
            .transpose()
    }
    pub fn federation_configuration(&self) -> Result<Configuration, StoreError> {
        read(&self.0)?.public()
    }
    pub fn configure_federation(
        &mut self,
        request: Configure,
        now: u64,
    ) -> Result<Configuration, StoreError> {
        let policy = egress::Policy::new(request.exceptions.clone())
            .map_err(|_| StoreError::Invalid("invalid federation network policy"))?;
        if !(1024 * 1024..=1024 * 1024 * 1024).contains(&request.peer_quota_bytes) {
            return Err(StoreError::Invalid("invalid federation peer quota"));
        }
        let hash =
            Sha256::digest(serde_json::to_vec(&request).map_err(|_| StoreError::InvalidData)?)
                .to_vec();
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut stored = read(&tx)?;
        if stored.revision != request.expected_revision {
            if request.expected_revision.checked_add(1) == Some(stored.revision)
                && stored.hash == hash
            {
                return stored.public();
            }
            return Err(StoreError::Conflict);
        }
        let server = stored.server.as_deref().ok_or(StoreError::Forbidden)?;
        if request.rotate_key || (request.enabled && stored.key.is_none()) {
            let generation = stored
                .key
                .as_ref()
                .map(|key| next(key.descriptor().generation))
                .transpose()?
                .unwrap_or(1);
            let key = SigningKey::generate(generation, now)
                .map_err(|_| StoreError::Invalid("invalid federation key timestamp"))?;
            let rotation = stored
                .key
                .as_ref()
                .map(|old| -> Result<Rotation, StoreError> {
                    Ok(Rotation {
                        previous: old.descriptor().clone(),
                        signature: old.transition(server, key.descriptor()).map_err(|_| {
                            StoreError::Invalid("rotation requires a later timestamp")
                        })?,
                    })
                })
                .transpose()?;
            stored.settings.key = Some(Material {
                seed: Zeroizing::new(key.seed().to_vec()),
                current: key.descriptor().clone(),
                rotation,
            });
            stored.key = Some(key);
        }
        stored.enabled = request.enabled;
        stored.settings.enabled = request.enabled;
        stored.settings.exceptions = request.exceptions;
        stored.settings.peer_quota_bytes = request.peer_quota_bytes;
        stored.peer_quota_bytes = request.peer_quota_bytes;
        stored.policy = policy;
        stored.revision = next(stored.revision)?;
        stored.hash = hash;
        save(&tx, &stored)?;
        let public = stored.public()?;
        tx.commit()?;
        Ok(public)
    }
    pub fn federation_discovery(&self) -> Result<Discovery, StoreError> {
        let stored = read(&self.0)?;
        if !stored.enabled {
            return Err(StoreError::Forbidden);
        }
        stored.discovery()
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurePeer {
    pub expected_revision: u64,
    pub allowed: bool,
    pub port: u16,
    pub approve_key: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Peer {
    pub server: String,
    pub revision: u64,
    pub allowed: bool,
    pub port: u16,
    pub pending_approval: Option<String>,
    pub pinned: Option<Discovery>,
    pub observed: Option<Discovery>,
    pub checked_at: u64,
    pub not_before: u64,
    pub error: Option<String>,
}
pub(crate) fn peer(db: &Connection, server: &str) -> Result<Option<Peer>, StoreError> {
    type Row = (
        u64,
        bool,
        u16,
        Option<String>,
        Option<String>,
        Option<String>,
        u64,
        u64,
        Option<String>,
    );
    let row:Option<Row>=db.query_row("SELECT revision,allowed,port,approval,CASE WHEN length(pinned)<=16384 THEN pinned WHEN pinned IS NOT NULL THEN '' END,CASE WHEN length(observed)<=16384 THEN observed WHEN observed IS NOT NULL THEN '' END,checked_at,not_before,error FROM federation_peers WHERE server=?1",[server],|r|Ok((unsigned(r,0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,unsigned(r,6)?,unsigned(r,7)?,r.get(8)?))).optional()?;
    let Some((
        revision,
        allowed,
        port,
        pending_approval,
        pinned,
        observed,
        checked_at,
        not_before,
        error,
    )) = row
    else {
        return Ok(None);
    };
    let decode = |text: Option<String>| -> Result<Option<Discovery>, StoreError> {
        text.map(|text| {
            let value: Discovery =
                serde_json::from_str(&text).map_err(|_| StoreError::InvalidData)?;
            auth::validate_discovery(&value, server, value.current.not_before)
                .map_err(|_| StoreError::InvalidData)?;
            Ok(value)
        })
        .transpose()
    };
    if !sigil_protocol::valid_server_name(server)
        || port == 0
        || pending_approval
            .as_ref()
            .is_some_and(|v| auth::bytes32(v).is_err())
    {
        return Err(StoreError::InvalidData);
    }
    Ok(Some(Peer {
        server: server.into(),
        revision,
        allowed,
        port,
        pending_approval,
        pinned: decode(pinned)?,
        observed: decode(observed)?,
        checked_at,
        not_before,
        error,
    }))
}
impl Store {
    pub fn federation_peer(&self, server: &str) -> Result<Peer, StoreError> {
        peer(&self.0, server)?.ok_or(StoreError::NotFound)
    }
    pub fn federation_peers(&self, after: Option<&str>) -> Result<Vec<Peer>, StoreError> {
        if after.is_some_and(|v| !sigil_protocol::valid_server_name(v)) {
            return Err(StoreError::Invalid("invalid peer cursor"));
        }
        let servers: Vec<String> = self
            .0
            .prepare(
                "SELECT server FROM federation_peers WHERE server>?1 ORDER BY server LIMIT 64",
            )?
            .query_map([after.unwrap_or("")], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        servers
            .into_iter()
            .map(|server| self.federation_peer(&server))
            .collect()
    }
    pub fn configure_federation_peer(
        &mut self,
        server: &str,
        request: ConfigurePeer,
    ) -> Result<Peer, StoreError> {
        if !sigil_protocol::valid_server_name(server)
            || request.port == 0
            || request
                .approve_key
                .as_ref()
                .is_some_and(|v| auth::bytes32(v).is_err())
        {
            return Err(StoreError::Invalid("invalid federation peer"));
        }
        let hash =
            Sha256::digest(serde_json::to_vec(&request).map_err(|_| StoreError::InvalidData)?);
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let stored = read(&tx)?;
        if stored.server.as_deref() == Some(server) || stored.server.is_none() {
            return Err(StoreError::Invalid("invalid remote server"));
        }
        if request.port != 443
            && !stored
                .settings
                .exceptions
                .iter()
                .any(|e| e.host == server && e.port == request.port)
        {
            return Err(StoreError::Invalid(
                "nonstandard peer ports require an explicit network policy",
            ));
        }
        let old = peer(&tx, server)?;
        if old
            .as_ref()
            .is_some_and(|p| p.error.as_deref() == Some("retired"))
            && retirement::pending(&tx, server)?
        {
            return Err(StoreError::Busy);
        }
        let revision = old.as_ref().map_or(0, |p| p.revision);
        if revision != request.expected_revision {
            if old.is_none() {
                return Err(StoreError::Conflict);
            }
            let old_hash: Vec<u8> = tx.query_row(
                "SELECT request_hash FROM federation_peers WHERE server=?1",
                [server],
                |r| r.get(0),
            )?;
            if request.expected_revision.checked_add(1) == Some(revision)
                && old_hash == hash.as_slice()
            {
                return old.ok_or(StoreError::InvalidData);
            }
            return Err(StoreError::Conflict);
        }
        if old.is_none() {
            // Reserve the maximum two discovery documents and row overhead for
            // every peer, including peers whose first discovery has not arrived.
            let size: u64 = tx.query_row(
                "SELECT count(*)*?1 FROM federation_peers",
                [sql(PEER_METADATA)?],
                |r| unsigned(r, 0),
            )?;
            if size > METADATA_BUDGET - PEER_METADATA {
                return Err(StoreError::Invalid(
                    "federation peer metadata budget exhausted",
                ));
            }
        }
        let changed = old
            .as_ref()
            .is_none_or(|p| p.port != request.port || p.allowed != request.allowed)
            || request.approve_key.is_some();
        tx.execute("INSERT INTO federation_peers VALUES(?1,?2,?3,?4,?5,NULL,NULL,0,0,NULL,?6) ON CONFLICT(server) DO UPDATE SET revision=excluded.revision,allowed=excluded.allowed,port=excluded.port,approval=excluded.approval,checked_at=CASE WHEN ?7 THEN 0 ELSE checked_at END,not_before=0,error=NULL,request_hash=excluded.request_hash",(server,sql(next(revision)?)?,request.allowed,request.port,request.approve_key,hash.as_slice(),changed))?;
        tx.execute(
            "INSERT OR IGNORE INTO federation_admission(server) VALUES(?1)",
            [server],
        )?;
        let result = peer(&tx, server)?.ok_or(StoreError::InvalidData)?;
        tx.commit()?;
        Ok(result)
    }
    /// Snapshot before HTTPS; completion compares the policy revision again.
    pub(crate) fn begin_federation_refresh(
        &mut self,
        server: &str,
        now: u64,
    ) -> Result<(Stored, Peer), StoreError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let config = read(&tx)?;
        let mut peer = peer(&tx, server)?.ok_or(StoreError::Forbidden)?;
        if !config.enabled || !peer.allowed {
            return Err(StoreError::Forbidden);
        }
        if peer.not_before > now {
            return Err(StoreError::Busy);
        }
        peer.not_before = now.checked_add(60).ok_or(StoreError::InvalidData)?;
        tx.execute(
            "UPDATE federation_peers SET not_before=?2 WHERE server=?1",
            (server, sql(peer.not_before)?),
        )?;
        tx.commit()?;
        Ok((config, peer))
    }
    pub(crate) fn finish_federation_refresh(
        &mut self,
        configuration_revision: u64,
        previous: &Peer,
        observed: Option<Discovery>,
        now: u64,
    ) -> Result<Peer, StoreError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = peer(&tx, &previous.server)?.ok_or(StoreError::Forbidden)?;
        let config = read(&tx)?;
        if config.revision != configuration_revision
            || current.revision != previous.revision
            || current.port != previous.port
            || current.not_before != previous.not_before
            || !config.enabled
            || !current.allowed
        {
            return Err(StoreError::Conflict);
        }
        let mut approved = false;
        let mut observation = None;
        if let Some(observed) = observed {
            auth::validate_discovery(&observed, &previous.server, now)
                .map_err(|_| StoreError::Invalid("invalid federation discovery"))?;
            approved = match (&current.pinned, &current.pending_approval) {
                (_, Some(expected)) if *expected == observed.current.id => true,
                (None, None) => true,
                (Some(pinned), None) => {
                    auth::follows(&pinned.current, &observed, &previous.server, now).is_ok()
                }
                _ => false,
            };
            observation =
                Some(serde_json::to_string(&observed).map_err(|_| StoreError::InvalidData)?);
        }
        let error = if approved {
            None
        } else if observation.is_some() {
            Some("key_change_requires_approval")
        } else {
            Some("discovery_failed")
        };
        let next_check = now
            .checked_add(if approved { 3600 } else { 60 })
            .ok_or(StoreError::InvalidData)?;
        tx.execute("UPDATE federation_peers SET observed=coalesce(?2,observed),pinned=CASE WHEN ?3 THEN ?2 ELSE pinned END,approval=CASE WHEN ?3 THEN NULL ELSE approval END,checked_at=CASE WHEN ?3 THEN ?4 ELSE 0 END,error=?5,not_before=?6 WHERE server=?1",(&previous.server,observation,approved,sql(now)?,error,sql(next_check)?))?;
        let result = peer(&tx, &previous.server)?.ok_or(StoreError::InvalidData)?;
        tx.commit()?;
        Ok(result)
    }
}
pub(crate) fn reset_after_restore(db: &Connection) -> Result<(), StoreError> {
    let mut stored = read(db)?;
    stored.revision = next(stored.revision)?;
    stored.enabled = false;
    stored.settings.enabled = false;
    stored.settings.key = None;
    stored.key = None;
    stored.hash.clear();
    save(db, &stored)?;
    db.execute_batch("DELETE FROM federation_nonces; UPDATE federation_usage SET nonce_bytes=0; UPDATE federation_admission SET nonce_bytes=0,credit=20,updated_at=0; UPDATE federation_peers SET checked_at=0,not_before=0,approval=NULL,error='restored_peer_requires_refresh';")?;
    Ok(())
}

#[cfg(test)]
#[path = "federation_config_tests.rs"]
pub(crate) mod tests;

#[path = "federation_retirement.rs"]
mod retirement;
pub use retirement::{PeerRetirement, RetirePeer};
