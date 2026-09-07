//! Durable publication and delayed-delivery retention of one-time private slots.
use super::*;
use serde::{Deserialize, Serialize};
use sigil_crypto::handshake::{Bundle, InitialMessage};
use sigil_protocol::prekeys::{PublishPrekey, PublishedPrekey};

pub(crate) const MIGRATION: &str = "
CREATE TABLE prekey_publications(
 slot BLOB PRIMARY KEY REFERENCES prekeys(id), bundle_id BLOB NOT NULL UNIQUE,
 expires_at INTEGER, state BLOB NOT NULL
);
CREATE INDEX prekey_publications_expiry ON prekey_publications(expires_at);
PRAGMA user_version=10;";
pub(crate) const RETENTION_MIGRATION: &str = "
ALTER TABLE prekey_publications ADD COLUMN retire_at INTEGER;
DROP INDEX prekey_publications_expiry;
CREATE INDEX prekey_publications_retirement ON prekey_publications(retire_at);
PRAGMA user_version=12;";
const DELIVERY_LIFETIME: u64 = 604800;
#[cfg(test)]
#[path = "prekey_tests.rs"]
mod tests;
#[path = "prekey_work.rs"]
mod work;
pub use work::{PrekeyAttempt, PrekeySupply};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Publication {
    bundle: String,
    lifetime: u32,
    expiry: Option<u64>,
    #[serde(default)]
    retire_at: Option<u64>,
    retired: bool,
}
pub(crate) fn decode(text: &str) -> Result<Vec<u8>, Error> {
    if ![3544, 3610].contains(&text.len())
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::InvalidStore);
    }
    text.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            u8::from_str_radix(
                std::str::from_utf8(pair).map_err(|_| Error::InvalidStore)?,
                16,
            )
            .map_err(|_| Error::InvalidStore)
        })
        .collect()
}
fn load(
    db: &Connection,
    key: &StorageKey,
    slot: &Id,
    identity: &Id,
) -> Result<(Publication, Bundle, Vec<u8>), Error> {
    let (id, expiry, retirement, sealed): (Vec<u8>, Option<i64>, Option<i64>, Vec<u8>) = db.query_row(
        "SELECT bundle_id,expires_at,retire_at,state FROM prekey_publications WHERE slot=?1 AND length(bundle_id)=32 AND length(state)<=4096",
        [slot.as_slice()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
    ).optional()?.ok_or(Error::NotFound)?;
    let expiry = expiry
        .map(u64::try_from)
        .transpose()
        .map_err(|_| Error::InvalidStore)?;
    let bytes = key.open(&sealed, &binding(12, slot, identity))?;
    let publication: Publication =
        serde_json::from_slice(&bytes).map_err(|_| Error::InvalidStore)?;
    if !(3600..=604800).contains(&publication.lifetime)
        || publication.expiry != expiry
        || publication.retire_at
            != retirement
                .map(u64::try_from)
                .transpose()
                .map_err(|_| Error::InvalidStore)?
        || publication
            .retire_at
            .is_some_and(|v| v == 0 || v > i64::MAX as u64)
        || (publication.expiry.is_none() && publication.retire_at.is_some())
        || expiry.is_some_and(|v| v == 0 || v > i64::MAX as u64 - DELIVERY_LIFETIME)
        || (publication.retired && expiry.is_none())
    {
        return Err(Error::InvalidStore);
    }
    let bundle = Bundle::from_bytes(&decode(&publication.bundle)?, identity)?;
    if id != bundle.id() {
        return Err(Error::InvalidStore);
    }
    Ok((publication, bundle, sealed))
}
fn seal(key: &StorageKey, slot: &Id, identity: &Id, value: &Publication) -> Result<Vec<u8>, Error> {
    let bytes = Zeroizing::new(serde_json::to_vec(value).map_err(|_| Error::InvalidStore)?);
    if bytes.len() + 36 > 4096 {
        return Err(Error::Limit);
    }
    Ok(key.seal(&bytes, &binding(12, slot, identity))?)
}
fn update(
    db: &Connection,
    key: &StorageKey,
    slot: &Id,
    identity: &Id,
    value: &Publication,
    expected: &[u8],
) -> Result<(), Error> {
    let sealed = seal(key, slot, identity, value)?;
    if db.execute(
        "UPDATE prekey_publications SET expires_at=?1,state=?2,retire_at=?5 WHERE slot=?3 AND state=?4",
        (
            value
                .expiry
                .map(i64::try_from)
                .transpose()
                .map_err(|_| Error::InvalidStore)?,
            sealed,
            slot.as_slice(),
            expected,
            value.retire_at.map(i64::try_from).transpose().map_err(|_| Error::InvalidStore)?,
        ),
    )? != 1
    {
        return Err(Error::Conflict);
    }
    Ok(())
}
fn live(db: &Connection, slot: &Id) -> Result<bool, Error> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM prekeys WHERE id=?1 AND state IS NOT NULL)",
        [slot.as_slice()],
        |r| r.get(0),
    )?)
}
fn receipt(publication: &Publication, bundle: &Bundle) -> Option<PublishedPrekey> {
    publication.expiry.map(|expires_at| PublishedPrekey {
        prekey_id: transport::hex(&bundle.prekey_id()),
        expires_at,
    })
}
pub(super) fn retire_in(
    tx: &Transaction<'_>,
    key: &StorageKey,
    identity: &Id,
    now: u64,
) -> Result<usize, Error> {
    let mut statement = tx.prepare("SELECT p.slot FROM prekey_publications p JOIN prekeys k ON k.id=p.slot WHERE p.retire_at<=?1 AND k.state IS NOT NULL ORDER BY p.retire_at,p.slot LIMIT 16")?;
    let slots = statement
        .query_map([now as i64], |r| r.get::<_, Vec<u8>>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for slot in &slots {
        let slot: Id = slot
            .as_slice()
            .try_into()
            .map_err(|_| Error::InvalidStore)?;
        let (mut publication, _, before) = load(tx, key, &slot, identity)?;
        if !publication.retire_at.is_some_and(|v| v <= now) || publication.retired {
            return Err(Error::InvalidStore);
        }
        publication.retired = true;
        update(tx, key, &slot, identity, &publication, &before)?;
        tx.execute(
            "UPDATE prekeys SET state=NULL WHERE id=?1",
            [slot.as_slice()],
        )?;
    }
    Ok(slots.len())
}

fn prepare_in(
    tx: &Transaction<'_>,
    key: &StorageKey,
    slot: Id,
    include_ec: bool,
    lifetime: u32,
) -> Result<Id, Error> {
    let identity = handshake::identity(tx, key)?.public_key();
    match load(tx, key, &slot, &identity) {
        Ok((publication, bundle, _)) => {
            if publication.lifetime != lifetime || (publication.bundle.len() == 3610) != include_ec
            {
                return Err(Error::Conflict);
            }
            return Ok(bundle.prekey_id());
        }
        Err(Error::NotFound) => {}
        Err(error) => return Err(error),
    }
    let bytes = handshake::create_prekey_in(tx, key, slot, include_ec)?;
    let bundle = Bundle::from_bytes(&bytes, &identity)?;
    let publication = Publication {
        bundle: transport::hex(&bytes),
        lifetime,
        expiry: None,
        retire_at: None,
        retired: false,
    };
    tx.execute(
        "INSERT INTO prekey_publications(slot,bundle_id,state) VALUES(?1,?2,?3)",
        (
            slot.as_slice(),
            bundle.id().as_slice(),
            seal(key, &slot, &identity, &publication)?,
        ),
    )?;
    Ok(bundle.prekey_id())
}

impl ClientStore {
    /// Private material and frozen publication metadata commit together before HTTP.
    pub fn prepare_prekey_publication(
        &mut self,
        slot: Id,
        include_ec: bool,
        lifetime: u32,
    ) -> Result<Id, Error> {
        if !(3600..=604800).contains(&lifetime) {
            return Err(Error::Limit);
        }
        self.connected_client()?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = prepare_in(&tx, &self.key, slot, include_ec, lifetime)?;
        tx.commit()?;
        Ok(id)
    }

    /// Ambiguous publication retries use the same bundle and original lifetime.
    pub fn publish_prekey_online(&mut self, slot: Id) -> Result<PublishedPrekey, Error> {
        self.publish_prekey_with_clock(slot, || {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|v| v.as_secs())
                .map_err(|_| Error::InvalidStore)
        })
    }

    fn publish_prekey_with_clock(
        &mut self,
        slot: Id,
        clock: impl FnOnce() -> Result<u64, Error>,
    ) -> Result<PublishedPrekey, Error> {
        let client = self.connected_client()?;
        let identity = self.identity()?;
        let (mut publication, bundle, before) = load(&self.db, &self.key, &slot, &identity)?;
        let prior = receipt(&publication, &bundle);
        if let Some(receipt) = &prior {
            if publication.retire_at.is_some() || publication.retired {
                return Ok(PublishedPrekey {
                    prekey_id: receipt.prekey_id.clone(),
                    expires_at: receipt.expires_at,
                });
            }
        }
        if prior.is_none() && !live(&self.db, &slot)? {
            return Err(Error::AlreadyDelivered);
        }
        let id = transport::hex(&bundle.prekey_id());
        let response = if let Some(receipt) = prior {
            receipt
        } else {
            client.publish_prekey(
                &id,
                &PublishPrekey {
                    bundle: publication.bundle.clone(),
                    expires_in_seconds: publication.lifetime,
                },
            )?
        };
        if response.expires_at > i64::MAX as u64 - DELIVERY_LIFETIME {
            return Err(Error::Conflict);
        }
        publication.expiry = Some(response.expires_at);
        // Start after acknowledgement. A full advertised lifetime plus maximum
        // delivery latency is conservative without synchronizing server clocks.
        publication.retire_at = Some(
            clock()?
                .checked_add(u64::from(publication.lifetime) + DELIVERY_LIFETIME)
                .filter(|v| *v <= i64::MAX as u64)
                .ok_or(Error::Limit)?,
        );
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (current, current_bundle, sealed) = load(&tx, &self.key, &slot, &identity)?;
        if sealed != before {
            if receipt(&current, &current_bundle).as_ref() == Some(&response)
                && (current.retire_at.is_some() || current.retired)
            {
                return Ok(response);
            }
            return Err(Error::Conflict);
        }
        update(&tx, &self.key, &slot, &identity, &publication, &before)?;
        tx.commit()?;
        Ok(response)
    }

    /// Retire only after the locally recorded retention deadline. Call with the
    /// trusted local wall clock. Records lacking a deadline remain protected.
    pub fn retire_prekeys(&mut self, now: u64) -> Result<usize, Error> {
        if now > i64::MAX as u64 {
            return Err(Error::Limit);
        }
        let identity = self.identity()?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let retired = retire_in(&tx, &self.key, &identity, now)?;
        tx.commit()?;
        Ok(retired)
    }

    /// Resolve an unauthenticated hint only. The caller must still independently
    /// select the sender identity and pass the full packet to accept_initial.
    pub fn initial_prekey_slot(&mut self, packet: &[u8]) -> Result<Id, Error> {
        let (raw, _) =
            sigil_protocol::initial::decode(packet).map_err(|_| Error::UnsupportedSession)?;
        let initial = InitialMessage::from_bytes(raw)?;
        if !initial.is_triple_ratchet() {
            return Err(Error::UnsupportedSession);
        }
        let identity = self.identity()?;
        let slot: Vec<u8> = self
            .db
            .query_row(
                "SELECT slot FROM prekey_publications WHERE bundle_id=?1 AND length(slot)=32",
                [initial.recipient_bundle_id().as_slice()],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(Error::NotFound)?;
        let slot: Id = slot.try_into().map_err(|_| Error::InvalidStore)?;
        let (_, bundle, _) = load(&self.db, &self.key, &slot, &identity)?;
        if bundle.id() != initial.recipient_bundle_id() {
            return Err(Error::InvalidStore);
        }
        Ok(slot)
    }
}
