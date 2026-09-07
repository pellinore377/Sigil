use crate::{
    auth::digest,
    store::{Store, StoreError},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};
use sigil_protocol::{
    accounts::valid_credential,
    prekeys::{ClaimedPrekey, PublishPrekey, PublishedPrekey},
};

type Assignment = (String, String, Option<Vec<u8>>, i64);

pub(crate) const MIGRATION: &str = "
CREATE TABLE encryption_identities (
 device_id TEXT PRIMARY KEY REFERENCES devices(id), public_key BLOB NOT NULL CHECK(length(public_key)=32)
);
CREATE TABLE prekeys (
 id TEXT PRIMARY KEY, device_id TEXT NOT NULL REFERENCES devices(id),
 bundle BLOB, bundle_hash BLOB NOT NULL, expires_at INTEGER NOT NULL,
 claimant TEXT REFERENCES devices(id), request_id TEXT,
 UNIQUE(claimant, request_id), CHECK((claimant IS NULL) = (request_id IS NULL))
);
CREATE INDEX prekeys_device ON prekeys(device_id);
";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn decode(value: &str) -> Result<Vec<u8>, StoreError> {
    if ![3544, 3610].contains(&value.len())
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(StoreError::Invalid("invalid experimental prekey encoding"));
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

pub(crate) fn authorize(db: &Connection, credential: &str, now: u64) -> Result<String, StoreError> {
    if !valid_credential(credential) || now > i64::MAX as u64 {
        return Err(StoreError::Unauthorized);
    }
    db.query_row("SELECT d.id FROM devices d JOIN accounts a ON a.id=d.account_id WHERE d.token_hash=?1 AND d.revoked=0 AND d.expires_at>?2 AND a.disabled=0",
        (digest(credential).as_slice(), now as i64), |r| r.get(0)).optional()?.ok_or(StoreError::Unauthorized)
}

pub(crate) fn active(db: &Connection, id: &str) -> Result<bool, StoreError> {
    // Transport credential expiry does not revoke an offline device's encryption identity.
    Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM devices d JOIN accounts a ON a.id=d.account_id WHERE d.id=?1 AND d.revoked=0 AND a.disabled=0)", [id], |r| r.get(0))?)
}

impl Store {
    /// Authorization and inventory share a transaction, including revocation races.
    pub fn prekey_inventory(
        &mut self,
        credential: &str,
        now: u64,
    ) -> Result<sigil_protocol::prekeys::PrekeyInventory, StoreError> {
        let tx = self.0.transaction()?;
        let device = authorize(&tx, credential, now)?;
        let available = tx.query_row("SELECT count(*) FROM prekeys WHERE device_id=?1 AND claimant IS NULL AND remote_server IS NULL AND bundle IS NOT NULL AND expires_at>?2", (&device, now as i64), |r| r.get(0))?;
        tx.commit()?;
        Ok(sigil_protocol::prekeys::PrekeyInventory { available })
    }
    /// Stores public bytes only. Recipients must verify signatures and trusted identity.
    pub fn publish_prekey(
        &mut self,
        credential: &str,
        id: &str,
        request: PublishPrekey,
        now: u64,
    ) -> Result<PublishedPrekey, StoreError> {
        if !valid_credential(id) || !(3600..=604800).contains(&request.expires_in_seconds) {
            return Err(StoreError::Invalid(
                "invalid prekey identifier or lifetime (1 hour to 7 days)",
            ));
        }
        let bytes = decode(&request.bundle)?;
        if &bytes[..8] != b"SGPQ\0\x01\x01\0"
            || bytes[8] != 1
            || bytes[41] != 1
            || bytes[138] != 2
            || (bytes.len() == 1772 && bytes[1771] != 0)
            || (bytes.len() == 1805 && (bytes[1771] != 1 || bytes[1772] != 1))
        {
            return Err(StoreError::Invalid("unsupported prekey framing"));
        }
        if hex(&Sha256::digest(&bytes[139..1707])) != id {
            return Err(StoreError::Invalid(
                "prekey identifier must be SHA-256 of its ML-KEM public key",
            ));
        }
        let expiry = now
            .checked_add(request.expires_in_seconds.into())
            .filter(|v| *v <= i64::MAX as u64)
            .ok_or(StoreError::InvalidData)?;
        let hash = Sha256::digest(&bytes);
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let device = authorize(&tx, credential, now)?;
        let existing: Option<(String, Vec<u8>, i64)> = tx
            .query_row(
                "SELECT device_id,bundle_hash,expires_at FROM prekeys WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        if let Some((owner, previous, expires_at)) = existing {
            return if owner == device && previous == hash.as_slice() {
                Ok(PublishedPrekey {
                    prekey_id: id.into(),
                    expires_at: u64::try_from(expires_at).map_err(|_| StoreError::InvalidData)?,
                })
            } else {
                Err(StoreError::AlreadyExists)
            };
        }
        let identity: Option<Vec<u8>> = tx
            .query_row(
                "SELECT public_key FROM encryption_identities WHERE device_id=?1",
                [&device],
                |r| r.get(0),
            )
            .optional()?;
        if identity.is_some_and(|key| key != bytes[9..41]) {
            return Err(StoreError::Invalid(
                "encryption identity changes require a new authorized device",
            ));
        }
        let live: u32 = tx.query_row("SELECT count(*) FROM prekeys WHERE device_id=?1 AND bundle IS NOT NULL AND claimant IS NULL AND remote_server IS NULL AND expires_at>?2", (&device,now as i64), |r| r.get(0))?;
        if live >= 64 {
            return Err(StoreError::Busy);
        }
        crate::storage_budget::for_device(
            &tx,
            &device,
            crate::storage_budget::PREKEY + bytes.len() as u64,
            now,
        )?;
        tx.execute(
            "INSERT INTO encryption_identities VALUES(?1,?2) ON CONFLICT(device_id) DO NOTHING",
            (&device, &bytes[9..41]),
        )?;
        tx.execute("INSERT INTO prekeys(id,device_id,bundle,bundle_hash,expires_at) VALUES(?1,?2,?3,?4,?5)", (id, device, bytes, hash.as_slice(), expiry as i64))?;
        tx.commit()?;
        Ok(PublishedPrekey {
            prekey_id: id.into(),
            expires_at: expiry,
        })
    }

    /// Atomically assigns one bundle. Repeating the same claim returns the original
    /// assignment until it expires; the ID can never claim another bundle afterward.
    pub fn claim_prekey(
        &mut self,
        credential: &str,
        target: &str,
        request_id: &str,
        now: u64,
    ) -> Result<ClaimedPrekey, StoreError> {
        if !valid_credential(target) || !valid_credential(request_id) {
            return Err(StoreError::Invalid("invalid device or claim identifier"));
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let claimant = authorize(&tx, credential, now)?;
        if !active(&tx, target)? {
            return Err(StoreError::NotFound);
        }
        crate::admission::check(&tx, &claimant, target)?;
        let assigned: Option<Assignment> = tx.query_row("SELECT id,device_id,bundle,expires_at FROM prekeys WHERE claimant=?1 AND request_id=?2", (&claimant, request_id), |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        if let Some((id, device, bytes, expiry)) = assigned {
            if device != target {
                return Err(StoreError::AlreadyExists);
            }
            if expiry <= now as i64 {
                return Err(StoreError::NotFound);
            }
            return Ok(ClaimedPrekey {
                device_id: device,
                prekey_id: id,
                bundle: hex(&bytes.ok_or(StoreError::NotFound)?),
                expires_at: expiry as u64,
            });
        }
        let available: Option<(String,Vec<u8>,i64)> = tx.query_row("SELECT id,bundle,expires_at FROM prekeys WHERE device_id=?1 AND claimant IS NULL AND remote_server IS NULL AND bundle IS NOT NULL AND expires_at>?2 ORDER BY expires_at,id LIMIT 1", (target, now as i64), |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let (id, bytes, expiry) = available.ok_or(StoreError::NotFound)?;
        tx.execute(
            "UPDATE prekeys SET claimant=?1,request_id=?2 WHERE id=?3",
            (&claimant, request_id, &id),
        )?;
        tx.commit()?;
        Ok(ClaimedPrekey {
            device_id: target.into(),
            prekey_id: id,
            bundle: hex(&bytes),
            expires_at: expiry as u64,
        })
    }
}
