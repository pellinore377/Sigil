//! Private credential issuance and opaque group authority storage.
use crate::{
    federation_auth::hex,
    prekeys::authorize,
    push_config::{next, sql, unsigned},
    store::{read_configuration, Store, StoreError},
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sigil_crypto::{
    private_credentials::Issuer,
    private_group::{authority_fingerprint, Authority},
    storage::StorageKey,
    IdentityKey, Secret32,
};
use sigil_protocol::groups::{Configuration, Configure, CredentialRequest, CredentialResponse};
use zeroize::Zeroizing;

pub(crate) const MIGRATION: &str = "
CREATE TABLE group_authority(id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL,enabled INTEGER NOT NULL CHECK(enabled IN(0,1)),quota INTEGER NOT NULL CHECK(quota>=0),used INTEGER NOT NULL CHECK(used>=0),material TEXT);
INSERT INTO group_authority VALUES(1,0,0,1073741824,0,NULL);
CREATE TABLE group_credential_uids(uid BLOB PRIMARY KEY CHECK(length(uid)=16),fingerprint BLOB UNIQUE NOT NULL CHECK(length(fingerprint)=32));
CREATE TABLE private_groups(id BLOB PRIMARY KEY CHECK(length(id)=32),public BLOB NOT NULL CHECK(length(public)=32),revision INTEGER NOT NULL,head BLOB NOT NULL CHECK(length(head)=32),restored INTEGER NOT NULL CHECK(restored IN(0,1)));
CREATE TABLE private_group_members(group_id BLOB NOT NULL REFERENCES private_groups(id),ciphertext BLOB NOT NULL CHECK(length(ciphertext)=64),admin INTEGER NOT NULL CHECK(admin IN(0,1)),PRIMARY KEY(group_id,ciphertext));
CREATE TABLE private_group_commits(group_id BLOB NOT NULL REFERENCES private_groups(id),revision INTEGER NOT NULL,head BLOB NOT NULL CHECK(length(head)=32),operation_hash BLOB NOT NULL CHECK(length(operation_hash)=32),author BLOB NOT NULL CHECK(length(author)=64),control BLOB NOT NULL,receipt BLOB,PRIMARY KEY(group_id,revision));
CREATE TABLE private_group_nonces(nonce BLOB PRIMARY KEY CHECK(length(nonce)=32),expires_at INTEGER NOT NULL);
CREATE INDEX private_group_nonces_expiry ON private_group_nonces(expires_at);
";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Material {
    wrapping: Zeroizing<[u8; 32]>,
    issuer: Vec<u8>,
    signing: Vec<u8>,
    signing_public: [u8; 32],
    profile: Vec<u8>,
}

pub(crate) struct Stored {
    pub revision: u64,
    pub enabled: bool,
    pub quota: u64,
    pub used: u64,
    pub authority: Option<Authority>,
    pub issuer: Option<Issuer>,
    pub signing: Option<IdentityKey>,
}

pub(crate) fn crypto(_: sigil_crypto::Error) -> StoreError {
    StoreError::InvalidData
}

pub(crate) fn decode(value: &str, limit: usize) -> Result<Vec<u8>, StoreError> {
    if !value.len().is_multiple_of(2)
        || value.len() > limit * 2
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(StoreError::Invalid("invalid group field"));
    }
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            u8::from_str_radix(
                std::str::from_utf8(pair).map_err(|_| StoreError::InvalidData)?,
                16,
            )
            .map_err(|_| StoreError::InvalidData)
        })
        .collect()
}

pub(crate) fn read(db: &Connection) -> Result<Stored, StoreError> {
    let (revision, enabled, quota, used, material): (u64,bool,u64,u64,Option<String>) = db.query_row("SELECT revision,enabled,quota,used,CASE WHEN material IS NULL THEN NULL WHEN length(material)<=4096 THEN material ELSE '' END FROM group_authority WHERE id=1",[],|r|Ok((unsigned(r,0)?,r.get(1)?,unsigned(r,2)?,unsigned(r,3)?,r.get(4)?)))?;
    let mut result = Stored {
        revision,
        enabled,
        quota,
        used,
        authority: None,
        issuer: None,
        signing: None,
    };
    if let Some(material) = material {
        let bytes = Zeroizing::new(material);
        let material: Material =
            serde_json::from_str(&bytes).map_err(|_| StoreError::InvalidData)?;
        let key = StorageKey::new(Secret32::from_bytes(*material.wrapping)).map_err(crypto)?;
        let server = read_configuration(db)?
            .settings
            .ok_or(StoreError::InvalidData)?
            .server_name;
        let authority = Authority::from_bytes(
            &material.profile,
            &server,
            authority_fingerprint(&material.signing_public),
        )
        .map_err(crypto)?;
        let issuer = Issuer::open_checkpoint(
            &key,
            b"Sigil/server/group-issuer/v0",
            &material.issuer,
            authority.issuer(),
        )
        .map_err(crypto)?;
        let signing =
            IdentityKey::open_checkpoint(&key, &material.signing, b"Sigil/server/group-signing/v0")
                .map_err(crypto)?;
        if signing.public_key() != material.signing_public {
            return Err(StoreError::InvalidData);
        }
        result.authority = Some(authority);
        result.issuer = Some(issuer);
        result.signing = Some(signing);
    } else if enabled {
        return Err(StoreError::InvalidData);
    }
    Ok(result)
}

impl Stored {
    fn configuration(&self) -> Configuration {
        Configuration {
            revision: self.revision,
            enabled: self.enabled,
            storage_limit_bytes: self.quota,
            used_bytes: self.used,
            authority: self.authority.as_ref().map(|a| hex(&a.to_bytes())),
        }
    }
}

pub(crate) fn reserve(tx: &Transaction<'_>, bytes: u64) -> Result<(), StoreError> {
    if tx.execute(
        "UPDATE group_authority SET used=used+?1 WHERE id=1 AND used<=quota AND ?1<=quota-used",
        [sql(bytes)?],
    )? != 1
    {
        return Err(StoreError::Busy);
    }
    Ok(())
}

pub(crate) fn reset_after_restore(tx: &Transaction<'_>) -> Result<(), StoreError> {
    let revision = tx.query_row("SELECT revision FROM group_authority WHERE id=1", [], |r| {
        unsigned(r, 0)
    })?;
    tx.execute(
        "UPDATE group_authority SET enabled=0,revision=?1",
        [sql(next(revision)?)?],
    )?;
    tx.execute("UPDATE private_groups SET restored=1", [])?;
    Ok(())
}

impl Store {
    pub fn group_configuration(&self) -> Result<Configuration, StoreError> {
        Ok(read(&self.0)?.configuration())
    }

    pub fn configure_groups(&mut self, request: Configure) -> Result<Configuration, StoreError> {
        if !(1024 * 1024..=1024 * 1024 * 1024 * 1024).contains(&request.storage_limit_bytes) {
            return Err(StoreError::Invalid("invalid group storage limit"));
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old = read(&tx)?;
        if old.revision != request.expected_revision {
            return if old.revision == next(request.expected_revision)?
                && old.enabled == request.enabled
                && old.quota == request.storage_limit_bytes
            {
                Ok(old.configuration())
            } else {
                Err(StoreError::Conflict)
            };
        }
        if request.enabled && old.authority.is_none() {
            let server = read_configuration(&tx)?
                .settings
                .ok_or(StoreError::Forbidden)?
                .server_name;
            let mut wrapping = Zeroizing::new([0; 32]);
            getrandom::fill(wrapping.as_mut()).map_err(|_| StoreError::InvalidData)?;
            let key = StorageKey::new(Secret32::from_bytes(*wrapping)).map_err(crypto)?;
            let issuer = Issuer::generate().map_err(crypto)?;
            let signing = IdentityKey::generate().map_err(crypto)?;
            let profile = Authority::sign(&server, 1, issuer.public(), &signing)
                .map_err(crypto)?
                .to_bytes();
            let material = Material {
                wrapping,
                issuer: issuer
                    .seal_checkpoint(&key, b"Sigil/server/group-issuer/v0")
                    .map_err(crypto)?,
                signing: signing
                    .seal_checkpoint(&key, b"Sigil/server/group-signing/v0")
                    .map_err(crypto)?,
                signing_public: signing.public_key(),
                profile,
            };
            let encoded = Zeroizing::new(
                serde_json::to_string(&material).map_err(|_| StoreError::InvalidData)?,
            );
            tx.execute(
                "UPDATE group_authority SET material=?1 WHERE id=1",
                [encoded.as_str()],
            )?;
        }
        tx.execute(
            "UPDATE group_authority SET revision=?1,enabled=?2,quota=?3 WHERE id=1",
            (
                sql(next(old.revision)?)?,
                request.enabled,
                sql(request.storage_limit_bytes)?,
            ),
        )?;
        let result = read(&tx)?.configuration();
        tx.commit()?;
        Ok(result)
    }

    pub fn group_authority(&self) -> Result<Vec<u8>, StoreError> {
        let stored = read(&self.0)?;
        if !stored.enabled {
            return Err(StoreError::Forbidden);
        }
        Ok(stored.authority.ok_or(StoreError::InvalidData)?.to_bytes())
    }

    pub fn issue_group_credential(
        &mut self,
        credential: &str,
        request: CredentialRequest,
        now: u64,
    ) -> Result<CredentialResponse, StoreError> {
        if now == 0 || now > i64::MAX as u64 || request.day as u64 != now / 86400 {
            return Err(StoreError::Invalid("credential must be for today"));
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let device = authorize(&tx, credential, now)?;
        let stored = read(&tx)?;
        if !stored.enabled {
            return Err(StoreError::Forbidden);
        }
        let authority = stored.authority.ok_or(StoreError::InvalidData)?;
        if request.authority != hex(&authority.id()) {
            return Err(StoreError::Conflict);
        }
        let binding: Vec<u8> = tx.query_row("SELECT CASE WHEN length(statement)<=512 THEN statement END FROM device_bindings WHERE device=?1",[&device],|r|r.get(0)).optional()?.ok_or(StoreError::NotFound)?;
        let issuance = authority.issuance(&binding, request.day).map_err(crypto)?;
        let previous: Option<Vec<u8>> = tx
            .query_row(
                "SELECT fingerprint FROM group_credential_uids WHERE uid=?1",
                [issuance.uid.as_slice()],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(previous) = previous {
            if previous != issuance.fingerprint {
                return Err(StoreError::Conflict);
            }
        } else {
            reserve(&tx, 512)?;
            tx.execute(
                "INSERT INTO group_credential_uids VALUES(?1,?2)",
                (issuance.uid.as_slice(), issuance.fingerprint.as_slice()),
            )?;
        }
        let response = stored
            .issuer
            .ok_or(StoreError::InvalidData)?
            .issue(
                &issuance.attributes().map_err(crypto)?,
                issuance.day,
                issuance.context(),
            )
            .map_err(crypto)?;
        let result = CredentialResponse {
            authority: hex(&authority.id()),
            uid: hex(&issuance.uid),
            day: issuance.day,
            response: hex(&response),
        };
        tx.commit()?;
        Ok(result)
    }
}
