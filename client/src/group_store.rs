//! Encrypted local membership/ordering journal shared by service and offline controls.
use super::storage_record::{open_record, seal_record, sealed_limit};
use super::*;
use crate::{binding, handshake, ClientStore};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior};
use sigil_crypto::storage::StorageKey;
use zeroize::Zeroizing;

pub(crate) const MIGRATION: &str = "
CREATE TABLE groups(id BLOB PRIMARY KEY,state BLOB NOT NULL);
CREATE TABLE group_genesis(id BLOB PRIMARY KEY REFERENCES groups(id),state BLOB NOT NULL);
CREATE TABLE group_commits(group_id BLOB NOT NULL REFERENCES groups(id),predecessor BLOB NOT NULL,state BLOB NOT NULL,PRIMARY KEY(group_id,predecessor));
CREATE TABLE group_proposals(group_id BLOB NOT NULL REFERENCES groups(id),head BLOB NOT NULL,state BLOB NOT NULL,PRIMARY KEY(group_id,head));
CREATE TABLE group_forks(group_id BLOB PRIMARY KEY REFERENCES groups(id),state BLOB NOT NULL);
PRAGMA user_version=43;
";

pub struct GroupStatus {
    pub state: State,
    pub frozen: bool,
}
#[derive(Debug, PartialEq, Eq)]
pub enum CommitResult {
    Applied,
    Duplicate,
    Frozen,
}

fn aad(kind: u8, group: &Id, own: &Id, record: &[u8]) -> Vec<u8> {
    binding(kind, group, &[own.as_slice(), record].concat())
}
pub(super) fn load(
    db: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    group: &Id,
) -> Result<GroupStatus, Error> {
    if device_fingerprint(&peers::own(db, key)?)? != *own {
        return Err(Error::Conflict);
    }
    let sealed: Vec<u8> = db
        .query_row(
            "SELECT CASE WHEN length(state)<=?2 THEN state END FROM groups WHERE id=?1",
            (
                group.as_slice(),
                sealed_limit(codec::MAX_CHECKPOINT_BYTES + 1) as u32,
            ),
            |r| r.get(0),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    let bytes = open_record(key, &sealed, &aad(40, group, own, b"state"))?;
    let (flag, checkpoint) = bytes.split_first().ok_or(Error::InvalidStore)?;
    if *flag > 1 {
        return Err(Error::InvalidStore);
    }
    let state = State::from_checkpoint(checkpoint)?;
    if state.group != *group {
        return Err(Error::InvalidStore);
    }
    let frozen = *flag == 1;
    let evidence: Option<Vec<u8>> = db
        .query_row(
            "SELECT CASE WHEN length(state)=452 THEN state END FROM group_forks WHERE group_id=?1",
            [group.as_slice()],
            |r| r.get(0),
        )
        .optional()?;
    match (frozen, evidence) {
        (true, Some(sealed)) => {
            let raw = key.open(&sealed, &aad(43, group, own, b"fork"))?;
            if raw.len() != 416 {
                return Err(Error::InvalidStore);
            }
            let a = Receipt::from_bytes(&raw[..208], state.authority)?;
            let b = Receipt::from_bytes(&raw[208..], state.authority)?;
            if a.group != *group
                || b.group != *group
                || a.predecessor != b.predecessor
                || (a.head == b.head && a.revision == b.revision)
            {
                return Err(Error::InvalidStore);
            }
        }
        (false, None) => {}
        _ => return Err(Error::InvalidStore),
    }
    Ok(GroupStatus { state, frozen })
}
fn save(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    state: &State,
    frozen: bool,
) -> Result<(), Error> {
    let mut bytes = Zeroizing::new(vec![frozen as u8]);
    bytes.extend_from_slice(&Zeroizing::new(state.checkpoint()?));
    let sealed = seal_record(key, &bytes, &aad(40, &state.group, own, b"state"))?;
    tx.execute(
        "INSERT INTO groups VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET state=excluded.state",
        (state.group.as_slice(), sealed),
    )?;
    Ok(())
}
pub(super) fn save_genesis(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    genesis: &Genesis,
) -> Result<Id, Error> {
    if device_fingerprint(&peers::own(tx, key)?)? != *own {
        return Err(Error::Conflict);
    }
    let group = genesis.state.group;
    let previous: Option<Vec<u8>> = tx
        .query_row(
            "SELECT CASE WHEN length(state)<=718 THEN state END FROM group_genesis WHERE id=?1",
            [group.as_slice()],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(sealed) = previous {
        let raw = key.open(&sealed, &aad(41, &group, own, b"genesis"))?;
        let previous = Genesis::from_bytes(&raw, genesis.creator)?;
        if previous.state.head != genesis.state.head {
            return Err(Error::Conflict);
        }
        load(tx, key, own, &group)?;
        return Ok(group);
    }
    if tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM groups WHERE id=?1)",
        [group.as_slice()],
        |r| r.get::<_, bool>(0),
    )? {
        return Err(Error::InvalidStore);
    }
    save(tx, key, own, &genesis.state, false)?;
    let sealed = key.seal(
        &Zeroizing::new(genesis.to_bytes()?),
        &aad(41, &group, own, b"genesis"),
    )?;
    tx.execute(
        "INSERT INTO group_genesis VALUES(?1,?2)",
        (group.as_slice(), sealed),
    )?;
    Ok(group)
}
fn prepared(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    state: &State,
    mut proposal: Proposal,
) -> Result<Vec<u8>, Error> {
    let old: Option<Vec<u8>> = tx.query_row("SELECT CASE WHEN length(state)<=?3 THEN state END FROM group_proposals WHERE group_id=?1 AND head=?2", (state.group.as_slice(), proposal.head().as_slice(), sealed_limit(MAX_PROPOSAL_BYTES) as u32), |r| r.get(0)).optional()?;
    if let Some(sealed) = old {
        let raw = open_record(key, &sealed, &aad(44, &state.group, own, &proposal.head()))?;
        let mut previous = state.proposal_from_bytes(&raw)?;
        for (id, signature) in proposal.signatures {
            previous.approve(id, signature)?;
        }
        proposal = previous;
    } else if tx.query_row(
        "SELECT count(*) FROM group_proposals WHERE group_id=?1",
        [state.group.as_slice()],
        |r| r.get::<_, u32>(0),
    )? >= 16
    {
        return Err(Error::Limit);
    }
    let bytes = proposal.to_bytes()?;
    let sealed = seal_record(key, &bytes, &aad(44, &state.group, own, &proposal.head()))?;
    tx.execute("INSERT INTO group_proposals VALUES(?1,?2,?3) ON CONFLICT(group_id,head) DO UPDATE SET state=excluded.state", (state.group.as_slice(), proposal.head().as_slice(), sealed))?;
    Ok(bytes)
}

impl ClientStore {
    pub(super) fn group_commit_after(
        &mut self,
        group: Id,
        predecessor: Id,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, Error> {
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let tx = self.db.transaction()?;
        load(&tx, &self.key, &own, &group)?;
        let sealed: Option<Vec<u8>> = tx.query_row("SELECT CASE WHEN length(state)<=?3 THEN state END FROM group_commits WHERE group_id=?1 AND predecessor=?2", (group.as_slice(), predecessor.as_slice(), sealed_limit(MAX_PROPOSAL_BYTES + 208) as u32), |r| r.get(0)).optional()?;
        sealed
            .map(|v| open_record(&self.key, &v, &aad(42, &group, &own, &predecessor)))
            .transpose()
    }
    /// Creates local genesis. Pin its service context and prepare/submit genesis
    /// separately before sharing the group through the authority.
    pub fn create_group(&mut self, authority: Id) -> Result<Id, Error> {
        let own_binding = self.own_device_binding()?;
        let own = device_fingerprint(&own_binding)?;
        let mut nonce = [0; 32];
        getrandom::fill(&mut nonce).map_err(|_| Error::Crypto(sigil_crypto::Error::Entropy))?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let identity = handshake::identity(&tx, &self.key)?;
        let genesis = Genesis::create(
            nonce,
            authority,
            digest(b"Sigil/group-creator-member/v0", &[&nonce, &own]),
            &own_binding,
            &identity,
        )?;
        let group = save_genesis(&tx, &self.key, &own, &genesis)?;
        tx.commit()?;
        Ok(group)
    }
    pub fn group_status(&mut self, group: Id) -> Result<GroupStatus, Error> {
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let tx = self.db.transaction()?;
        load(&tx, &self.key, &own, &group)
    }
    pub fn group_genesis(&mut self, group: Id) -> Result<Vec<u8>, Error> {
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let tx = self.db.transaction()?;
        load(&tx, &self.key, &own, &group)?;
        let sealed: Vec<u8> = tx.query_row(
            "SELECT CASE WHEN length(state)<=718 THEN state END FROM group_genesis WHERE id=?1",
            [group.as_slice()],
            |r| r.get(0),
        )?;
        Ok(self
            .key
            .open(&sealed, &aad(41, &group, &own, b"genesis"))?
            .to_vec())
    }
    /// Pins an independently verified inviter; staging genesis does not make the
    /// local device a member. A committed, explicitly approved join is required.
    pub fn accept_group_genesis(&mut self, creator_peer: Id, bytes: &[u8]) -> Result<Id, Error> {
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let creator = peers::verified(&tx, &self.key, &creator_peer)?;
        let genesis = Genesis::from_bytes(bytes, creator.fingerprint)?;
        let group = save_genesis(&tx, &self.key, &own, &genesis)?;
        tx.commit()?;
        Ok(group)
    }
    /// Explicit local approval, frozen durably before any external submission.
    pub fn prepare_group_change(&mut self, group: Id, change: Change) -> Result<Vec<u8>, Error> {
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let status = load(&tx, &self.key, &own, &group)?;
        if status.frozen {
            return Err(Error::Conflict);
        }
        let mut proposal = status.state.propose(own, change)?;
        proposal.sign(own, &handshake::identity(&tx, &self.key)?)?;
        let bytes = prepared(&tx, &self.key, &own, &status.state, proposal)?;
        tx.commit()?;
        Ok(bytes)
    }
    pub fn approve_group_proposal(&mut self, group: Id, bytes: &[u8]) -> Result<Vec<u8>, Error> {
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let status = load(&tx, &self.key, &own, &group)?;
        if status.frozen {
            return Err(Error::Conflict);
        }
        let mut proposal = status.state.proposal_from_bytes(bytes)?;
        proposal.sign(own, &handshake::identity(&tx, &self.key)?)?;
        let bytes = prepared(&tx, &self.key, &own, &status.state, proposal)?;
        tx.commit()?;
        Ok(bytes)
    }
    pub fn pending_group_proposal(&mut self, group: Id, head: Id) -> Result<Vec<u8>, Error> {
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let tx = self.db.transaction()?;
        let status = load(&tx, &self.key, &own, &group)?;
        if status.frozen {
            return Err(Error::Conflict);
        }
        let sealed: Vec<u8> = tx.query_row("SELECT CASE WHEN length(state)<=?3 THEN state END FROM group_proposals WHERE group_id=?1 AND head=?2", (group.as_slice(), head.as_slice(), sealed_limit(MAX_PROPOSAL_BYTES) as u32), |r| r.get(0)).optional()?.ok_or(Error::NotFound)?;
        let bytes = open_record(&self.key, &sealed, &aad(44, &group, &own, &head))?;
        if status.state.proposal_from_bytes(&bytes)?.head() != head {
            return Err(Error::InvalidStore);
        }
        Ok(bytes.to_vec())
    }
    /// The pending set is bounded to 16; enumerate it after restart without
    /// requiring a separate caller-maintained list of proposal identifiers.
    pub fn pending_group_proposals(&mut self, group: Id) -> Result<Vec<Id>, Error> {
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let tx = self.db.transaction()?;
        let status = load(&tx, &self.key, &own, &group)?;
        if status.frozen {
            return Err(Error::Conflict);
        }
        let rows = tx.prepare("SELECT head,CASE WHEN length(state)<=?2 THEN state END FROM group_proposals WHERE group_id=?1 ORDER BY head LIMIT 17")?
            .query_map((group.as_slice(), sealed_limit(MAX_PROPOSAL_BYTES) as u32), |r| Ok((r.get::<_,Vec<u8>>(0)?, r.get::<_,Vec<u8>>(1)?)))?
            .collect::<Result<Vec<_>,_>>()?;
        if rows.len() > 16 {
            return Err(Error::InvalidStore);
        }
        let mut heads = Vec::with_capacity(rows.len());
        for (head, sealed) in rows {
            let head: Id = head.try_into().map_err(|_| Error::InvalidStore)?;
            let raw = open_record(&self.key, &sealed, &aad(44, &group, &own, &head))?;
            if status.state.proposal_from_bytes(&raw)?.head() != head {
                return Err(Error::InvalidStore);
            }
            heads.push(head);
        }
        Ok(heads)
    }
    /// Drops only the local queued copy. An approval already shared remains
    /// valid against its predecessor until another state revision commits.
    pub fn discard_local_group_proposal(&mut self, group: Id, head: Id) -> Result<bool, Error> {
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        load(&tx, &self.key, &own, &group)?;
        let removed = tx.execute(
            "DELETE FROM group_proposals WHERE group_id=?1 AND head=?2",
            (group.as_slice(), head.as_slice()),
        )? != 0;
        tx.commit()?;
        Ok(removed)
    }
    /// Both device authorization and pinned authority ordering are mandatory.
    /// Conflicting signed authority receipts freeze the existing branch; they
    /// never authorize the conflicting proposal or expose new sender keys.
    pub fn commit_group_proposal(
        &mut self,
        group: Id,
        proposal: &[u8],
        receipt: &[u8],
    ) -> Result<CommitResult, Error> {
        if proposal.len() > MAX_PROPOSAL_BYTES {
            return Err(Error::Limit);
        }
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let status = load(&tx, &self.key, &own, &group)?;
        let receipt = Receipt::from_bytes(receipt, status.state.authority)?;
        if receipt.group != group {
            return Err(Error::Conflict);
        }
        if status.frozen {
            return Ok(CommitResult::Frozen);
        }
        let previous: Option<Vec<u8>> = tx.query_row("SELECT CASE WHEN length(state)<=?3 THEN state END FROM group_commits WHERE group_id=?1 AND predecessor=?2", (group.as_slice(), receipt.predecessor.as_slice(), sealed_limit(MAX_PROPOSAL_BYTES + 208) as u32), |r| r.get(0)).optional()?;
        if let Some(sealed) = previous {
            let raw = open_record(
                &self.key,
                &sealed,
                &aad(42, &group, &own, &receipt.predecessor),
            )?;
            if raw.len() < 208 {
                return Err(Error::InvalidStore);
            }
            let old = Receipt::from_bytes(&raw[..208], status.state.authority)?;
            if old.group != group || old.predecessor != receipt.predecessor {
                return Err(Error::InvalidStore);
            }
            if old.head == receipt.head && old.revision == receipt.revision {
                if &raw[208..] != proposal {
                    return Err(Error::Conflict);
                }
                return Ok(CommitResult::Duplicate);
            }
            let evidence = [old.to_bytes(), receipt.to_bytes()].concat();
            let sealed = self.key.seal(&evidence, &aad(43, &group, &own, b"fork"))?;
            tx.execute(
                "INSERT INTO group_forks VALUES(?1,?2)",
                (group.as_slice(), sealed),
            )?;
            save(&tx, &self.key, &own, &status.state, true)?;
            keys::retire(&tx, &self.key, &own, &group)?;
            tx.commit()?;
            return Ok(CommitResult::Frozen);
        }
        let proposed = status.state.proposal_from_bytes(proposal)?;
        let next = status.state.authorize(&proposed)?;
        if receipt.predecessor != status.state.head
            || receipt.head != next.head
            || receipt.revision != next.revision
        {
            return Err(Error::Conflict);
        }
        let mut record = Zeroizing::new(receipt.to_bytes());
        record.extend_from_slice(proposal);
        let sealed = seal_record(
            &self.key,
            &record,
            &aad(42, &group, &own, &receipt.predecessor),
        )?;
        tx.execute(
            "INSERT INTO group_commits VALUES(?1,?2,?3)",
            (group.as_slice(), receipt.predecessor.as_slice(), sealed),
        )?;
        save(&tx, &self.key, &own, &next, false)?;
        keys::retire(&tx, &self.key, &own, &group)?;
        // All pending proposals were approved against the consumed predecessor.
        tx.execute(
            "DELETE FROM group_proposals WHERE group_id=?1",
            [group.as_slice()],
        )?;
        tx.commit()?;
        Ok(CommitResult::Applied)
    }
}

#[cfg(test)]
#[path = "group_store_tests.rs"]
pub(crate) mod tests;
