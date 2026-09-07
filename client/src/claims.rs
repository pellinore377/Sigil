//! Persist claim identity before a one-time prekey can be assigned remotely.
use super::*;
pub(super) fn peer(db: &Connection, key: &StorageKey, id: &Id, own: &Id) -> Result<Id, Error> {
    load(db, key, id, own)?.0.peer.ok_or(Error::Unprepared)
}
use serde::{Deserialize, Serialize};
use sigil_crypto::handshake::Bundle;
use sigil_protocol::prekeys::ClaimedPrekey;

pub(crate) const MIGRATION: &str = "
CREATE TABLE prekey_claims(id BLOB PRIMARY KEY, phase INTEGER NOT NULL CHECK(phase IN (0,1,2)), state BLOB NOT NULL);
PRAGMA user_version=11;";
#[cfg(test)]
#[path = "claim_tests.rs"]
pub(crate) mod tests;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Claim {
    recipient: Id,
    expected_identity: Id,
    #[serde(default)]
    peer: Option<Id>,
    phase: Phase,
}
#[derive(Serialize, Deserialize)]
enum Phase {
    Pending,
    Ready(ClaimedPrekey),
    Abandoned,
}
impl Phase {
    fn code(&self) -> i64 {
        match self {
            Self::Pending => 0,
            Self::Ready(_) => 1,
            Self::Abandoned => 2,
        }
    }
}
fn authorize_peer(db: &Connection, key: &StorageKey, claim: &Claim) -> Result<(), Error> {
    if let Some(peer) = claim.peer {
        peers::require(db, key, &peer, &claim.recipient, &claim.expected_identity)?;
    }
    Ok(())
}
fn validate(claim: &Claim, response: &ClaimedPrekey) -> Result<(), Error> {
    let bundle = Bundle::from_bytes(
        &prekeys::decode(&response.bundle)?,
        &claim.expected_identity,
    )?;
    if response.device_id != transport::hex(&claim.recipient)
        || response.prekey_id != transport::hex(&bundle.prekey_id())
        || response.expires_at == 0
        || response.expires_at > i64::MAX as u64 - 604800
    {
        return Err(Error::Conflict);
    }
    Ok(())
}
fn load(
    db: &Connection,
    key: &StorageKey,
    id: &Id,
    identity: &Id,
) -> Result<(Claim, Vec<u8>), Error> {
    let (phase, sealed): (i64, Vec<u8>) = db
        .query_row(
            "SELECT phase,state FROM prekey_claims WHERE id=?1 AND length(state)<=4608",
            [id.as_slice()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    let bytes = key.open(&sealed, &binding(13, id, identity))?;
    let claim: Claim = serde_json::from_slice(&bytes).map_err(|_| Error::InvalidStore)?;
    if claim.phase.code() != phase {
        return Err(Error::InvalidStore);
    }
    if let Phase::Ready(response) = &claim.phase {
        validate(&claim, response)?;
    }
    Ok((claim, sealed))
}
fn seal(key: &StorageKey, id: &Id, identity: &Id, claim: &Claim) -> Result<Vec<u8>, Error> {
    let bytes = Zeroizing::new(serde_json::to_vec(claim).map_err(|_| Error::InvalidStore)?);
    if bytes.len() + 36 > 4608 {
        return Err(Error::Limit);
    }
    Ok(key.seal(&bytes, &binding(13, id, identity))?)
}
fn save(
    db: &Connection,
    key: &StorageKey,
    id: &Id,
    identity: &Id,
    claim: &Claim,
    before: &[u8],
) -> Result<(), Error> {
    if db.execute(
        "UPDATE prekey_claims SET phase=?1,state=?2 WHERE id=?3 AND state=?4",
        (
            claim.phase.code(),
            seal(key, id, identity, claim)?,
            id.as_slice(),
            before,
        ),
    )? != 1
    {
        return Err(Error::Conflict);
    }
    Ok(())
}
impl ClientStore {
    /// Expected identity must come from independent peer verification, never the
    /// claim response. Reusing a request ID cannot change either target field.
    pub fn prepare_prekey_claim(
        &mut self,
        id: Id,
        recipient: Id,
        expected_identity: Id,
    ) -> Result<(), Error> {
        self.connected_client()?;
        let identity = self.identity()?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        prepare(
            &tx,
            &self.key,
            &identity,
            id,
            recipient,
            expected_identity,
            None,
        )?;
        tx.commit()?;
        Ok(())
    }

    /// An HTTP success is not exposed until signatures, frozen identity and KEM
    /// identifier validate and the exact assigned bundle commits locally.
    pub fn claim_prekey_online(&mut self, id: Id, now: u64) -> Result<ClaimedPrekey, Error> {
        if now > i64::MAX as u64 {
            return Err(Error::Limit);
        }
        let client = self.connected_client()?;
        let identity = self.identity()?;
        let (mut claim, before) = load(&self.db, &self.key, &id, &identity)?;
        authorize_peer(&self.db, &self.key, &claim)?;
        match &claim.phase {
            Phase::Ready(response) => {
                return if response.expires_at > now {
                    Ok(response.clone())
                } else {
                    Err(Error::Expired)
                };
            }
            Phase::Abandoned => return Err(Error::AlreadyDelivered),
            Phase::Pending => {}
        }
        let response =
            client.claim_prekey(&transport::hex(&claim.recipient), &transport::hex(&id))?;
        validate(&claim, &response)?;
        if response.expires_at <= now {
            return Err(Error::Expired);
        }
        claim.phase = Phase::Ready(response.clone());
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (current, sealed) = load(&tx, &self.key, &id, &identity)?;
        authorize_peer(&tx, &self.key, &current)?;
        if sealed != before {
            if matches!(&current.phase, Phase::Ready(value) if value == &response) {
                return Ok(response);
            }
            return Err(Error::Conflict);
        }
        save(&tx, &self.key, &id, &identity, &claim, &before)?;
        tx.commit()?;
        Ok(response)
    }

    /// Abandonment retains a local request-ID tombstone; it cannot unclaim a
    /// remote key or authorize reuse of that request ID for another assignment.
    pub fn abandon_prekey_claim(&mut self, id: Id) -> Result<(), Error> {
        let identity = self.identity()?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        abandon(&tx, &self.key, &id, &identity)?;
        tx.commit()?;
        Ok(())
    }

    /// Create a session using the durably selected peer identity. After the
    /// bundle expires, only an already committed initial packet may be retried.
    pub fn start_claimed_initial(
        &mut self,
        claim_id: Id,
        session: Id,
        message: Id,
        plaintext: &[u8],
        now: u64,
    ) -> Result<Vec<u8>, Error> {
        self.start_claimed_content(claim_id, session, message, (plaintext, None), now)
    }
    pub(super) fn start_claimed_content(
        &mut self,
        claim_id: Id,
        session: Id,
        message: Id,
        content: (&[u8], Option<&[u8]>),
        now: u64,
    ) -> Result<Vec<u8>, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let packet = start_in(
            &tx,
            &self.key,
            claim_id,
            session,
            message,
            (content.0, content.1, None),
            now,
        )?;
        tx.commit()?;
        Ok(packet)
    }
}

pub(super) fn abandon(
    tx: &Transaction<'_>,
    key: &StorageKey,
    id: &Id,
    identity: &Id,
) -> Result<(), Error> {
    let (mut claim, before) = load(tx, key, id, identity)?;
    if !matches!(claim.phase, Phase::Abandoned) {
        claim.phase = Phase::Abandoned;
        save(tx, key, id, identity, &claim, &before)?;
    }
    Ok(())
}

pub(super) fn start_in(
    tx: &Transaction<'_>,
    key: &StorageKey,
    claim_id: Id,
    session: Id,
    message: Id,
    content: (&[u8], Option<&[u8]>, Option<u64>),
    now: u64,
) -> Result<Vec<u8>, Error> {
    let (plaintext, own_statement, expires) = content;
    if now > i64::MAX as u64 {
        return Err(Error::Limit);
    }
    let identity = handshake::identity(tx, key)?.public_key();
    let (claim, _) = load(tx, key, &claim_id, &identity)?;
    authorize_peer(tx, key, &claim)?;
    let Phase::Ready(response) = claim.phase else {
        return Err(Error::Unprepared);
    };
    if response.expires_at <= now
        && !tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM outbox WHERE session=?1 AND id=?2)",
            (session.as_slice(), message.as_slice()),
            |r| r.get::<_, bool>(0),
        )?
    {
        return Err(Error::Expired);
    }
    let packet = handshake::start(
        tx,
        key,
        session,
        message,
        claim.expected_identity,
        &prekeys::decode(&response.bundle)?,
        plaintext,
    )?;
    if let Some(peer) = claim.peer {
        peers::bind_session(tx, key, &session, &peer)?;
    }
    let delivery = transport::prepare(tx, key, session, message, claim.recipient, expires, now)?;
    if delivery.expires_at > response.expires_at + 604800 {
        return Err(Error::Expired);
    }
    if let Some(own) = own_statement {
        event::retain(
            tx,
            key,
            own,
            &claim.peer.ok_or(Error::Unprepared)?,
            plaintext,
            true,
        )?;
    }
    Ok(packet)
}

pub(super) fn prepare(
    tx: &Transaction<'_>,
    key: &StorageKey,
    identity: &Id,
    id: Id,
    recipient: Id,
    expected_identity: Id,
    peer: Option<Id>,
) -> Result<(), Error> {
    match load(tx, key, &id, identity) {
        Ok((prior, _)) => {
            if prior.recipient != recipient
                || prior.expected_identity != expected_identity
                || prior.peer != peer
            {
                return Err(Error::Conflict);
            }
            if matches!(prior.phase, Phase::Abandoned) {
                return Err(Error::AlreadyDelivered);
            }
            authorize_peer(tx, key, &prior)?;
            return Ok(());
        }
        Err(Error::NotFound) => {}
        Err(error) => return Err(error),
    }
    let pending: i64 = tx.query_row(
        "SELECT count(*) FROM prekey_claims WHERE phase=0",
        [],
        |r| r.get(0),
    )?;
    if pending >= 64 {
        return Err(Error::Limit);
    }
    let claim = Claim {
        recipient,
        expected_identity,
        peer,
        phase: Phase::Pending,
    };
    authorize_peer(tx, key, &claim)?;
    tx.execute(
        "INSERT INTO prekey_claims VALUES(?1,0,?2)",
        (id.as_slice(), seal(key, &id, identity, &claim)?),
    )?;
    Ok(())
}
