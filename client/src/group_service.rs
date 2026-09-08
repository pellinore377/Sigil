//! Pinned authority context, encrypted controls and a restartable submission slot.
//! Group invitation transport and foreign-authority credential issuance are separate.
use super::*;
use crate::{network::HttpsClient, transport::hex, ClientStore};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sigil_crypto::{
    private_credentials::GroupKey, private_group::Authority, storage::StorageKey, Secret32,
};
use sigil_protocol::groups::{Commit, Member as EncryptedMember, Operation};
use zeroize::Zeroizing;

pub(crate) const MIGRATION:&str="
CREATE TABLE group_service(group_id BLOB PRIMARY KEY REFERENCES groups(id),state BLOB NOT NULL);
CREATE TABLE group_service_outbox(group_id BLOB PRIMARY KEY REFERENCES group_service(group_id),state BLOB NOT NULL);
PRAGMA user_version=55;";
const MAX: usize = codec::MAX_CHECKPOINT_BYTES;
pub(super) fn aad(group: &Id, own: &Id, kind: &[u8]) -> Vec<u8> {
    [
        b"Sigil/native/group-service/v0".as_slice(),
        group,
        own,
        kind,
    ]
    .concat()
}
pub(super) fn decode(value: &str, limit: usize) -> Result<Vec<u8>, Error> {
    if value.len() > limit * 2
        || !value.len().is_multiple_of(2)
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::InvalidEvent);
    }
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| {
            u8::from_str_radix(std::str::from_utf8(p).map_err(|_| Error::InvalidEvent)?, 16)
                .map_err(|_| Error::InvalidEvent)
        })
        .collect()
}
fn id(value: &str) -> Result<Id, Error> {
    decode(value, 32)?
        .try_into()
        .map_err(|_| Error::InvalidEvent)
}
pub(super) struct Access {
    pub(super) profile: Authority,
    pub(super) key: GroupKey,
    pub(super) encryption: StorageKey,
}
pub(super) fn load(
    db: &Connection,
    key: &StorageKey,
    group: &Id,
    own: &Id,
    authority: Id,
) -> Result<Access, Error> {
    let sealed:Vec<u8>=db.query_row("SELECT CASE WHEN length(state)<=512 THEN state END FROM group_service WHERE group_id=?1",[group.as_slice()],|r|r.get(0)).optional()?.ok_or(Error::Unprepared)?;
    let raw = key.open(&sealed, &aad(group, own, b"context"))?;
    let (master, profile) = raw.split_at_checked(32).ok_or(Error::InvalidStore)?;
    let master = Zeroizing::new(<Id>::try_from(master).map_err(|_| Error::InvalidStore)?);
    Ok(Access {
        profile: Authority::from_pinned_bytes(profile, authority)?,
        key: GroupKey::from_master(Secret32::from_bytes(*master))?,
        encryption: StorageKey::new(Secret32::from_bytes(*master))?,
    })
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    operation: Operation,
    status: Submission,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Submission {
    Pending,
    Accepted,
    Superseded,
}
fn pending(
    db: &Connection,
    key: &StorageKey,
    group: &Id,
    own: &Id,
) -> Result<Option<Pending>, Error> {
    let sealed:Option<Vec<u8>>=db.query_row("SELECT CASE WHEN length(state)<=?2 THEN state END FROM group_service_outbox WHERE group_id=?1",(group.as_slice(),storage_record::sealed_limit(MAX) as u32),|r|r.get(0)).optional()?;
    sealed
        .map(|bytes| {
            let raw = storage_record::open_record(key, &bytes, &aad(group, own, b"outbox"))?;
            serde_json::from_slice(&raw).map_err(|_| Error::InvalidStore)
        })
        .transpose()
}
fn save_pending(
    db: &Connection,
    key: &StorageKey,
    group: &Id,
    own: &Id,
    value: &Pending,
) -> Result<(), Error> {
    let raw = Zeroizing::new(serde_json::to_vec(value).map_err(|_| Error::InvalidStore)?);
    if raw.len() > MAX {
        return Err(Error::Limit);
    }
    let sealed = storage_record::seal_record(key, &raw, &aad(group, own, b"outbox"))?;
    db.execute("INSERT INTO group_service_outbox VALUES(?1,?2) ON CONFLICT(group_id) DO UPDATE SET state=excluded.state",(group.as_slice(),sealed))?;
    Ok(())
}
fn fields(operation: &Operation) -> Result<(u64, Id, Id, &str), Error> {
    match operation {
        Operation::Create { head, control, .. } => Ok((0, [0; 32], id(head)?, control)),
        Operation::Advance {
            revision,
            predecessor,
            head,
            control,
            ..
        } => Ok((*revision, id(predecessor)?, id(head)?, control)),
        _ => Err(Error::InvalidStore),
    }
}
impl Access {
    fn context(&self, group: Id, revision: u64, predecessor: Id, head: Id) -> Vec<u8> {
        [
            b"Sigil/private-group/control/v0".as_slice(),
            &self.profile.id(),
            &group,
            &revision.to_be_bytes(),
            &predecessor,
            &head,
        ]
        .concat()
    }
    fn open(&self, group: Id, operation: &Operation) -> Result<Zeroizing<Vec<u8>>, Error> {
        let (revision, predecessor, head, control) = fields(operation)?;
        storage_record::open_record(
            &self.encryption,
            &decode(control, sigil_protocol::groups::MAX_CONTROL)?,
            &self.context(group, revision, predecessor, head),
        )
    }
    fn roster(&self, state: &State) -> Result<Vec<EncryptedMember>, Error> {
        let mut roster = Vec::new();
        for member in &state.members {
            for device in &member.devices {
                let issuance = self
                    .profile
                    .issuance(&device.to_bytes().map_err(|_| Error::InvalidEvent)?, 0)?;
                roster.push(EncryptedMember {
                    ciphertext: hex(&self.key.ciphertext(&issuance.attributes()?)),
                    admin: member.role == Role::Admin,
                });
            }
        }
        roster.sort_by(|a, b| a.ciphertext.cmp(&b.ciphertext));
        if roster
            .windows(2)
            .any(|p| p[0].ciphertext == p[1].ciphertext)
        {
            return Err(Error::Conflict);
        }
        Ok(roster)
    }
    fn prepare(&self, state: &State, predecessor: Id, raw: &[u8]) -> Result<Operation, Error> {
        let control = hex(&storage_record::seal_record(
            &self.encryption,
            raw,
            &self.context(state.group, state.revision, predecessor, state.head),
        )?);
        let members = self.roster(state)?;
        Ok(if state.revision == 0 {
            Operation::Create {
                public: hex(&self.key.public()),
                head: hex(&state.head),
                control,
                members,
            }
        } else {
            Operation::Advance {
                predecessor: hex(&predecessor),
                head: hex(&state.head),
                revision: state.revision,
                control,
                members,
            }
        })
    }
    fn open_commit(
        &self,
        group: Id,
        commit: &Commit,
    ) -> Result<(Zeroizing<Vec<u8>>, Vec<u8>), Error> {
        let receipt = decode(commit.receipt.as_deref().ok_or(Error::InvalidEvent)?, 208)?;
        let parsed = Receipt::from_bytes(&receipt, self.profile.fingerprint())?;
        if parsed.group != group
            || parsed.revision != commit.revision
            || hex(&parsed.head) != commit.head
        {
            return Err(Error::Conflict);
        }
        let control = storage_record::open_record(
            &self.encryption,
            &decode(&commit.control, sigil_protocol::groups::MAX_CONTROL)?,
            &self.context(group, parsed.revision, parsed.predecessor, parsed.head),
        )?;
        Ok((control, receipt))
    }
    pub(super) fn client(&self, store: &ClientStore) -> Result<HttpsClient, Error> {
        let client = store.connected_client()?;
        client.group_authority_at(
            self.profile.server(),
            self.profile.fingerprint(),
            Some(self.profile.id()),
        )?;
        Ok(client)
    }
}
impl ClientStore {
    /// Relay a fully signed, durably prepared member proposal without advancing
    /// the ordering head. Pending remains true until an ordered commit is seen.
    /// False means awaiting an administrator; true means the receipt committed.
    pub fn submit_group_relay_online(&mut self, group: Id, now: u64) -> Result<bool, Error> {
        let binding = self.own_device_binding()?;
        let own = device_fingerprint(&binding)?;
        let status = self.group_status(group)?;
        if status.frozen {
            return Err(Error::Conflict);
        }
        let access = load(&self.db, &self.key, &group, &own, status.state.authority)?;
        let mut job = pending(&self.db, &self.key, &group, &own)?.ok_or(Error::Unprepared)?;
        match job.status {
            Submission::Accepted => return Ok(true),
            Submission::Superseded => return Err(Error::Obsolete),
            Submission::Pending => {}
        }
        let raw = access.open(group, &job.operation)?;
        let (_, previous, head, _) = fields(&job.operation)?;
        if status.state.head == previous {
            let proposal = status.state.proposal_from_bytes(&raw)?;
            status.state.authorize(&proposal)?;
            if proposal.author != own {
                return Err(Error::Unprepared);
            }
        } else if status.state.head != head {
            return Err(Error::Obsolete);
        }
        let Operation::Advance {
            predecessor,
            head,
            revision,
            control,
            ..
        } = job.operation.clone()
        else {
            return Err(Error::Unprepared);
        };
        let client = access.client(self)?;
        let credential = self.cached_group_credential(&client, &access.profile, &binding, now)?;
        let reply = client.group_request(
            &access.profile,
            &credential,
            &access.key,
            group,
            &Operation::Relay {
                predecessor,
                head,
                revision,
                control,
            },
            now,
        )?;
        if reply.restored {
            return Err(Error::Conflict);
        }
        if let Some(commit) = reply.commits.first() {
            let (raw, receipt) = access.open_commit(group, commit)?;
            if self.commit_group_proposal(group, &raw, &receipt)? == CommitResult::Frozen {
                return Err(Error::Conflict);
            }
            let tx = self
                .db
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let current = pending(&tx, &self.key, &group, &own)?.ok_or(Error::Conflict)?;
            if serde_json::to_vec(&current.operation).map_err(|_| Error::InvalidStore)?
                != serde_json::to_vec(&job.operation).map_err(|_| Error::InvalidStore)?
            {
                return Err(Error::Conflict);
            }
            job.status = Submission::Accepted;
            save_pending(&tx, &self.key, &group, &own, &job)?;
            tx.commit()?;
            return Ok(true);
        }
        if reply.head != hex(&status.state.head)
            || reply.proposals[0].author
                != hex(&access
                    .key
                    .ciphertext(&access.profile.issuance(&binding, 0)?.attributes()?))
        {
            return Err(Error::Conflict);
        }
        Ok(false)
    }
    /// Two opaque member submissions, ordered by the returned author cursor.
    /// Each proposal still needs end-to-end validation before it can be ordered.
    pub fn group_relay_proposals_online(
        &mut self,
        group: Id,
        after: Option<String>,
        now: u64,
    ) -> Result<Vec<sigil_protocol::groups::Relayed>, Error> {
        self.refresh_group_authority_for_send(group, now)?;
        let binding = self.own_device_binding()?;
        let own = device_fingerprint(&binding)?;
        let status = self.group_status(group)?;
        if status.state.device(own)?.0.role != Role::Admin {
            return Err(Error::Unprepared);
        }
        let access = load(&self.db, &self.key, &group, &own, status.state.authority)?;
        let client = access.client(self)?;
        let credential = self.cached_group_credential(&client, &access.profile, &binding, now)?;
        let reply = client.group_request(
            &access.profile,
            &credential,
            &access.key,
            group,
            &Operation::Proposals { after },
            now,
        )?;
        if reply.restored || reply.head != hex(&status.state.head) {
            return Err(Error::Conflict);
        }
        Ok(reply.proposals)
    }
    /// An administrator validates a relayed member signature and freezes the
    /// original ciphertext. Receiving arbitrary opaque bytes grants no approval.
    pub fn prepare_group_relay_commit(
        &mut self,
        group: Id,
        relay: &sigil_protocol::groups::Relayed,
    ) -> Result<(), Error> {
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let status = store::load(&tx, &self.key, &own, &group)?;
        if status.frozen || status.state.device(own)?.0.role != Role::Admin {
            return Err(Error::Unprepared);
        }
        let access = load(&tx, &self.key, &group, &own, status.state.authority)?;
        if relay.predecessor != hex(&status.state.head)
            || relay.revision != status.state.revision.checked_add(1).ok_or(Error::Limit)?
        {
            return Err(Error::Conflict);
        }
        let raw = storage_record::open_record(
            &access.encryption,
            &decode(&relay.control, sigil_protocol::groups::MAX_CONTROL)?,
            &access.context(
                group,
                relay.revision,
                id(&relay.predecessor)?,
                id(&relay.head)?,
            ),
        )?;
        let proposal = status.state.proposal_from_bytes(&raw)?;
        let next = status.state.authorize(&proposal)?;
        let author = status
            .state
            .members
            .iter()
            .flat_map(|m| &m.devices)
            .find(|d| peers::fingerprint(&d.binding).ok() == Some(proposal.author))
            .ok_or(Error::Unprepared)?;
        let expected = access.key.ciphertext(
            &access
                .profile
                .issuance(&author.to_bytes().map_err(|_| Error::InvalidEvent)?, 0)?
                .attributes()?,
        );
        if relay.author != hex(&expected) || relay.head != hex(&next.head) {
            return Err(Error::Conflict);
        }
        let operation = Operation::Advance {
            predecessor: relay.predecessor.clone(),
            head: relay.head.clone(),
            revision: relay.revision,
            control: relay.control.clone(),
            members: access.roster(&next)?,
        };
        if let Some(previous) = pending(&tx, &self.key, &group, &own)? {
            if previous.status == Submission::Pending
                && serde_json::to_vec(&previous.operation).map_err(|_| Error::InvalidStore)?
                    != serde_json::to_vec(&operation).map_err(|_| Error::InvalidStore)?
            {
                return Err(Error::Conflict);
            }
        }
        save_pending(
            &tx,
            &self.key,
            &group,
            &own,
            &Pending {
                operation,
                status: Submission::Pending,
            },
        )?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn refresh_group_authority_for_send(
        &mut self,
        group: Id,
        now: u64,
    ) -> Result<(), Error> {
        let pinned = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM group_service WHERE group_id=?1)",
            [group.as_slice()],
            |r| r.get::<_, bool>(0),
        )?;
        if pinned && !self.sync_group_service_online(group, now)? {
            return Err(Error::Unprepared);
        }
        Ok(())
    }
    /// Context must come from the authenticated group invitation. Pins are
    /// immutable: another master or issuer profile cannot replace this context.
    pub fn pin_group_service(
        &mut self,
        group: Id,
        profile: &[u8],
        master: Zeroizing<Id>,
    ) -> Result<(), Error> {
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let status = store::load(&tx, &self.key, &own, &group)?;
        if status.frozen {
            return Err(Error::Conflict);
        }
        let profile = Authority::from_pinned_bytes(profile, status.state.authority)?;
        let previous:Option<Vec<u8>>=tx.query_row("SELECT CASE WHEN length(state)<=512 THEN state END FROM group_service WHERE group_id=?1",[group.as_slice()],|r|r.get(0)).optional()?;
        if let Some(previous) = previous {
            let raw = self.key.open(&previous, &aad(&group, &own, b"context"))?;
            if raw.len() < 32 || raw[..32] != *master {
                return Err(Error::Conflict);
            }
            let old = Authority::from_pinned_bytes(&raw[32..], status.state.authority)?;
            return if old.id() == profile.id() {
                Ok(())
            } else {
                Err(Error::Conflict)
            };
        }
        GroupKey::from_master(Secret32::from_bytes(*master))?;
        let mut raw = Zeroizing::new(master.to_vec());
        raw.extend_from_slice(&profile.to_bytes());
        tx.execute(
            "INSERT INTO group_service VALUES(?1,?2)",
            (
                group.as_slice(),
                self.key.seal(&raw, &aad(&group, &own, b"context"))?,
            ),
        )?;
        super::envelope::register(&tx, &self.key, group)?;
        tx.commit()?;
        Ok(())
    }
    /// None prepares genesis; Some prepares an already fully approved proposal.
    /// Repeating preparation preserves the original randomized ciphertext.
    pub fn prepare_group_service_request(
        &mut self,
        group: Id,
        proposal: Option<&[u8]>,
    ) -> Result<(), Error> {
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let genesis = if proposal.is_none() {
            Some(Zeroizing::new(self.group_genesis(group)?))
        } else {
            None
        };
        let raw = proposal
            .or(genesis.as_deref().map(Vec::as_slice))
            .ok_or(Error::InvalidEvent)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let status = store::load(&tx, &self.key, &own, &group)?;
        if status.frozen {
            return Err(Error::Conflict);
        }
        let access = load(&tx, &self.key, &group, &own, status.state.authority)?;
        if let Some(previous) = pending(&tx, &self.key, &group, &own)? {
            if access.open(group, &previous.operation)?.as_slice() == raw {
                return if previous.status == Submission::Superseded {
                    Err(Error::Obsolete)
                } else {
                    Ok(())
                };
            }
            if previous.status == Submission::Pending {
                return Err(Error::Conflict);
            }
        }
        let (next, predecessor) = if proposal.is_some() {
            let proposal = status.state.proposal_from_bytes(raw)?;
            (status.state.authorize(&proposal)?, status.state.head)
        } else {
            let state = Genesis::from_bytes(raw, own)?.accept(own)?;
            if state.head != status.state.head || state.revision != 0 {
                return Err(Error::Conflict);
            }
            (state, [0; 32])
        };
        let operation = access.prepare(&next, predecessor, raw)?;
        save_pending(
            &tx,
            &self.key,
            &group,
            &own,
            &Pending {
                operation,
                status: Submission::Pending,
            },
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn group_service_request_pending(&mut self, group: Id) -> Result<bool, Error> {
        let own = device_fingerprint(&self.own_device_binding()?)?;
        Ok(pending(&self.db, &self.key, &group, &own)?
            .is_some_and(|p| p.status == Submission::Pending))
    }
    pub fn submit_group_service_online(
        &mut self,
        group: Id,
        now: u64,
    ) -> Result<CommitResult, Error> {
        let binding = self.own_device_binding()?;
        let own = device_fingerprint(&binding)?;
        let status = self.group_status(group)?;
        if status.frozen {
            return Err(Error::Conflict);
        }
        let access = load(&self.db, &self.key, &group, &own, status.state.authority)?;
        let mut job = pending(&self.db, &self.key, &group, &own)?.ok_or(Error::Unprepared)?;
        match job.status {
            Submission::Accepted => return Ok(CommitResult::Duplicate),
            Submission::Superseded => return Err(Error::Obsolete),
            Submission::Pending => {}
        }
        let client = access.client(self)?;
        let credential = self.cached_group_credential(&client, &access.profile, &binding, now)?;
        let reply = client.group_request(
            &access.profile,
            &credential,
            &access.key,
            group,
            &job.operation,
            now,
        )?;
        if reply.restored {
            return Err(Error::Conflict);
        }
        let result = if reply.revision == 0 {
            CommitResult::Applied
        } else {
            let (raw, receipt) = access.open_commit(group, &reply.commits[0])?;
            self.commit_group_proposal(group, &raw, &receipt)?
        };
        if result == CommitResult::Frozen {
            return Ok(result);
        }
        // A crash between the membership transaction and this acknowledgement
        // leaves the exact request pending; the next attempt commits Duplicate.
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = pending(&tx, &self.key, &group, &own)?.ok_or(Error::Conflict)?;
        if serde_json::to_vec(&current.operation).map_err(|_| Error::InvalidStore)?
            != serde_json::to_vec(&job.operation).map_err(|_| Error::InvalidStore)?
        {
            return Err(Error::Conflict);
        }
        job.status = Submission::Accepted;
        save_pending(&tx, &self.key, &group, &own, &job)?;
        tx.commit()?;
        Ok(result)
    }
    /// Applies at most two authenticated changes. False means another page is
    /// needed. A service restore or rollback never authorizes outgoing work.
    pub fn sync_group_service_online(&mut self, group: Id, now: u64) -> Result<bool, Error> {
        let binding = self.own_device_binding()?;
        let own = device_fingerprint(&binding)?;
        let status = self.group_status(group)?;
        if status.frozen {
            return Err(Error::Conflict);
        }
        let access = load(&self.db, &self.key, &group, &own, status.state.authority)?;
        let client = access.client(self)?;
        let credential = self.cached_group_credential(&client, &access.profile, &binding, now)?;
        let mut next = status.state.revision.checked_add(1).ok_or(Error::Limit)?;
        if let Some(job) = pending(&self.db, &self.key, &group, &own)? {
            let (revision, ..) = fields(&job.operation)?;
            if job.status == Submission::Pending && revision > 0 {
                next = next.min(revision);
            }
        }
        let reply = client.group_request(
            &access.profile,
            &credential,
            &access.key,
            group,
            &Operation::Read {
                from_revision: next,
            },
            now,
        )?;
        if reply.restored
            || reply.revision < status.state.revision
            || (reply.revision == status.state.revision && reply.head != hex(&status.state.head))
        {
            return Err(Error::Conflict);
        }
        for commit in reply.commits {
            let (raw, receipt) = access.open_commit(group, &commit)?;
            if self.commit_group_proposal(group, &raw, &receipt)? == CommitResult::Frozen {
                return Err(Error::Conflict);
            }
            // Resolve the exact queued revision using an authenticated commit.
            // If this acknowledgement fails, the next page starts here again;
            // the membership journal safely accepts the duplicate first.
            let tx = self
                .db
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            if let Some(mut job) = pending(&tx, &self.key, &group, &own)? {
                let (revision, _, head, control) = fields(&job.operation)?;
                if job.status == Submission::Pending && revision == commit.revision {
                    job.status = if hex(&head) == commit.head && control == commit.control {
                        Submission::Accepted
                    } else {
                        Submission::Superseded
                    };
                    save_pending(&tx, &self.key, &group, &own, &job)?;
                }
            }
            tx.commit()?;
        }
        let status = self.group_status(group)?;
        Ok(status.state.revision == reply.revision && hex(&status.state.head) == reply.head)
    }
}

#[cfg(test)]
#[path = "group_service_tests.rs"]
pub(super) mod tests;
