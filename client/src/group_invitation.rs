//! Authenticated invitation contexts and explicit, durable joining consent.
use super::*;
use crate::{handshake, transport::hex, ClientStore};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sigil_crypto::{private_group::Authority, storage::StorageKey};
use zeroize::Zeroizing;
const OFFER: &[u8; 8] = b"SGGI\0\x01\0\0";
const DEVICE: &[u8; 8] = b"SGGD\0\x01\0\0";
const APPROVAL: &[u8; 8] = b"SGGA\0\x01\0\0";
const MARKER: &[u8; 8] = b"SGIM\0\x01\0\0";
const MAX: usize = 4096;
const RECEIPT: usize = 201;
pub(crate) const MIGRATION: &str = "
CREATE TABLE group_invitations(id BLOB PRIMARY KEY,group_id BLOB NOT NULL,state BLOB NOT NULL);
CREATE INDEX group_invitations_group ON group_invitations(group_id,id);
CREATE TABLE group_invitation_packets(id BLOB PRIMARY KEY REFERENCES deliveries(id),invitation BLOB NOT NULL REFERENCES group_invitations(id),state BLOB NOT NULL);
CREATE INDEX group_invitation_packets_invitation ON group_invitation_packets(invitation,id);
PRAGMA user_version=58;";

struct Capsule {
    device: bool,
    id: Id,
    group: Id,
    sender: Id,
    target: Id,
    member: Id,
    role: Role,
    expires: u64,
    genesis: Vec<u8>,
    context: Zeroizing<Vec<u8>>,
    signature: [u8; 64],
}
impl Capsule {
    fn unsigned(&self) -> Result<Zeroizing<Vec<u8>>, Error> {
        if self.genesis.len() > 682
            || self.context.len() > 463
            || self.context.len() < 32
            || self.id == [0; 32]
            || self.expires == 0
            || self.expires > i64::MAX as u64
        {
            return Err(Error::InvalidEvent);
        }
        let mut out = Zeroizing::new(if self.device { DEVICE } else { OFFER }.to_vec());
        for id in [
            &self.id,
            &self.group,
            &self.sender,
            &self.target,
            &self.member,
        ] {
            out.extend_from_slice(id);
        }
        out.push(u8::from(self.role == Role::Admin));
        out.extend_from_slice(&self.expires.to_be_bytes());
        for part in [&self.genesis[..], &self.context[..]] {
            out.extend_from_slice(&(part.len() as u16).to_be_bytes());
            out.extend_from_slice(part);
        }
        Ok(out)
    }
    fn statement(&self) -> Result<Id, Error> {
        Ok(digest(
            b"Sigil/group-invitation-signature/v0",
            &[&self.unsigned()?],
        ))
    }
    fn bytes(&self) -> Result<Zeroizing<Vec<u8>>, Error> {
        let mut out = self.unsigned()?;
        out.extend_from_slice(&self.signature);
        Ok(out)
    }
    fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX {
            return Err(Error::Limit);
        }
        let mut r = codec::Reader(bytes);
        let header = r.take(8)?;
        if header != OFFER && header != DEVICE {
            return Err(Error::InvalidEvent);
        }
        let device = header == DEVICE;
        let id = r.array()?;
        let group = r.array()?;
        let sender = r.array()?;
        let target = r.array()?;
        let member = r.array()?;
        let role = r.role()?;
        let expires = u64::from_be_bytes(r.array()?);
        let n = r.count(682)?;
        let genesis = r.take(n)?.to_vec();
        let n = r.count(463)?;
        let context = Zeroizing::new(r.take(n)?.to_vec());
        let signature = r.array()?;
        let value = Self {
            device,
            id,
            group,
            sender,
            target,
            member,
            role,
            expires,
            genesis,
            context,
            signature,
        };
        if !r.0.is_empty()
            || value.bytes()?.as_slice() != bytes
            || (!device && member != digest(b"Sigil/invited-member/v0", &[&group, &id, &target]))
        {
            return Err(Error::InvalidEvent);
        }
        if Genesis::from_invitation(&value.genesis)?.state.group != group {
            return Err(Error::Conflict);
        }
        Ok(value)
    }
    fn verify(&self, peer: &peers::Peer, target: &Id, now: u64) -> Result<(), Error> {
        if !peer.verified || peer.fingerprint != self.sender || *target != self.target {
            return Err(Error::Unprepared);
        }
        if now != 0 && (self.expires <= now || self.expires > now.saturating_add(604800)) {
            return Err(Error::Expired);
        }
        verify_signature(&peer.binding.identity, &self.statement()?, &self.signature)?;
        let genesis = Genesis::from_invitation(&self.genesis)?;
        Authority::from_pinned_bytes(&self.context[32..], genesis.state.authority)?;
        Ok(())
    }
    fn message(&self) -> Id {
        digest(
            b"Sigil/group-invitation-message/v0",
            &[&self.id, &self.group, &self.sender, &self.target],
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InvitationStatus {
    Offered,
    Accepted,
    Waiting,
    Joined,
    Cancelled,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    capsule: Zeroizing<Vec<u8>>,
    peer: Id,
    outgoing: bool,
    status: InvitationStatus,
    cancelled_remote: bool,
    approval: Option<Vec<u8>>,
}
#[derive(Debug)]
pub struct InvitationNotice {
    pub id: Id,
    pub group: Id,
    pub peer: Id,
    pub status: InvitationStatus,
    pub outgoing: bool,
    pub expires_at: u64,
}
fn aad(own: &Id, id: &Id) -> Vec<u8> {
    [b"Sigil/group-invitation-store/v0".as_slice(), own, id].concat()
}
fn load(db: &Connection, key: &StorageKey, own: &Id, id: &Id) -> Result<Record, Error> {
    let (group,sealed):(Vec<u8>,Vec<u8>)=db.query_row("SELECT group_id,CASE WHEN length(state)<=32768 THEN state END FROM group_invitations WHERE id=?1",[id.as_slice()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?.ok_or(Error::NotFound)?;
    let record: Record = serde_json::from_slice(&key.open(&sealed, &aad(own, id))?)
        .map_err(|_| Error::InvalidStore)?;
    let capsule = Capsule::parse(&record.capsule)?;
    if capsule.id != *id
        || group != capsule.group
        || (if record.outgoing {
            capsule.sender
        } else {
            capsule.target
        }) != *own
    {
        return Err(Error::InvalidStore);
    }
    Ok(record)
}
fn save(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    id: &Id,
    record: &Record,
) -> Result<(), Error> {
    let capsule = Capsule::parse(&record.capsule)?;
    if capsule.id != *id {
        return Err(Error::Conflict);
    }
    let bytes = Zeroizing::new(serde_json::to_vec(record).map_err(|_| Error::InvalidStore)?);
    if bytes.len() + 36 > 32768 {
        return Err(Error::Limit);
    }
    tx.execute("INSERT INTO group_invitations VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET state=excluded.state",(id.as_slice(),capsule.group.as_slice(),key.seal(&bytes,&aad(own,id))?))?;
    Ok(())
}
pub(crate) fn is_wire(bytes: &[u8]) -> bool {
    bytes.starts_with(&OFFER[..4])
        || bytes.starts_with(&DEVICE[..4])
        || bytes.starts_with(&APPROVAL[..4])
        || bootstrap::is_wire(bytes)
}
struct ControlReceipt {
    kind: u8,
    message: Id,
    request: Id,
    group: Id,
    sender: Id,
    target: Id,
    tag: Id,
}
impl ControlReceipt {
    fn bytes(&self) -> Vec<u8> {
        let mut out = MARKER.to_vec();
        out.push(self.kind);
        for id in [
            &self.message,
            &self.request,
            &self.group,
            &self.sender,
            &self.target,
            &self.tag,
        ] {
            out.extend_from_slice(id);
        }
        out
    }
    fn parse(bytes: &[u8]) -> Result<Option<Self>, Error> {
        if !bytes.starts_with(&MARKER[..4]) {
            return Ok(None);
        }
        if bytes.len() != RECEIPT || &bytes[..8] != MARKER || bytes[8] > 2 {
            return Err(Error::InvalidEvent);
        }
        let mut r = codec::Reader(&bytes[9..]);
        Ok(Some(Self {
            kind: bytes[8],
            message: r.array()?,
            request: r.array()?,
            group: r.array()?,
            sender: r.array()?,
            target: r.array()?,
            tag: r.array()?,
        }))
    }
}
fn receipt_aad(own: &Id, receipt: &ControlReceipt) -> Vec<u8> {
    [
        b"Sigil/group-invitation-receipt/v0".as_slice(),
        own,
        &receipt.group,
        &receipt.message,
    ]
    .concat()
}
pub(crate) fn retained(key: &StorageKey, bytes: &[u8]) -> Result<Option<Vec<u8>>, Error> {
    if let Some(receipt) = bootstrap::retained(key, bytes)? {
        return Ok(Some(receipt));
    }
    if bytes.starts_with(&OFFER[..4]) || bytes.starts_with(&DEVICE[..4]) {
        let capsule = Capsule::parse(bytes)?;
        return Ok(Some(
            ControlReceipt {
                kind: 0,
                message: capsule.message(),
                request: capsule.id,
                group: capsule.group,
                sender: capsule.sender,
                target: capsule.target,
                tag: key.commitment(bytes, b"Sigil/invitation-control-tag/v0")?,
            }
            .bytes(),
        ));
    }
    if bytes.starts_with(&APPROVAL[..4]) {
        let approval = Approval::parse(bytes)?;
        return Ok(Some(
            ControlReceipt {
                kind: 1,
                message: approval.message(),
                request: approval.request,
                group: approval.group,
                sender: approval.sender,
                target: approval.target,
                tag: key.commitment(bytes, b"Sigil/invitation-control-tag/v0")?,
            }
            .bytes(),
        ));
    }
    Ok(None)
}
pub(crate) fn validate_receipt(
    db: &Connection,
    key: &StorageKey,
    own: &[u8],
    peer: &Id,
    message: &Id,
    bytes: &[u8],
) -> Result<bool, Error> {
    let Some(receipt) = ControlReceipt::parse(bytes)? else {
        return Ok(false);
    };
    let own = device_fingerprint(own)?;
    if receipt.message != *message
        || receipt.target != own
        || peers::known(db, key, peer)?.fingerprint != receipt.sender
    {
        return Err(Error::Conflict);
    }
    let sealed: Vec<u8> = db
        .query_row(
            "SELECT CASE WHEN length(state)=237 THEN state END FROM group_controls WHERE id=?1",
            [message.as_slice()],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(Error::InvalidStore)?;
    if key.open(&sealed, &receipt_aad(&own, &receipt))?.as_slice() != bytes {
        return Err(Error::InvalidStore);
    }
    Ok(true)
}
pub(crate) fn receipt_message(bytes: &[u8]) -> Result<Option<Id>, Error> {
    Ok(ControlReceipt::parse(bytes)?.map(|r| r.message))
}
pub(crate) fn install(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own_binding: &[u8],
    peer: &Id,
    message: &Id,
    bytes: &[u8],
    expires: u64,
) -> Result<Option<Vec<u8>>, Error> {
    if !is_wire(bytes) {
        return Ok(None);
    }
    let known = peers::verified(tx, key, peer)?;
    let own = device_fingerprint(own_binding)?;
    let marker = retained(key, bytes)?.ok_or(Error::InvalidEvent)?;
    let receipt = ControlReceipt::parse(&marker)?.ok_or(Error::InvalidStore)?;
    if receipt.message != *message || receipt.sender != known.fingerprint || receipt.target != own {
        return Err(Error::Conflict);
    }
    if receipt.kind == 0 {
        let capsule = Capsule::parse(bytes)?;
        capsule.verify(&known, &own, 0)?;
        if capsule.device {
            let fields = peers::parse(own_binding)?.binding;
            if fields.server != known.binding.server
                || fields.account != known.binding.account
                || fields.username != known.binding.username
            {
                return Err(Error::Unprepared);
            }
        }
        if expires > capsule.expires {
            return Err(Error::Conflict);
        }
        match load(tx, key, &own, &capsule.id) {
            Ok(prior)
                if prior.peer == *peer && !prior.outgoing && prior.capsule.as_slice() == bytes => {}
            Ok(_) => return Err(Error::Conflict),
            Err(Error::NotFound) => save(
                tx,
                key,
                &own,
                &capsule.id,
                &Record {
                    capsule: Zeroizing::new(bytes.to_vec()),
                    peer: *peer,
                    outgoing: false,
                    status: InvitationStatus::Offered,
                    cancelled_remote: false,
                    approval: None,
                },
            )?,
            Err(error) => return Err(error),
        }
    } else if receipt.kind == 2 {
        bootstrap::install(tx, key, &own, peer, bytes, expires)?;
    } else {
        let approval = Approval::parse(bytes)?;
        let mut record = load(tx, key, &own, &approval.request)?;
        let capsule = Capsule::parse(&record.capsule)?;
        if !record.outgoing
            || record.peer != *peer
            || capsule.group != approval.group
            || capsule.target != approval.sender
            || capsule.sender != approval.target
            || expires > capsule.expires
        {
            return Err(Error::Conflict);
        }
        if record.status == InvitationStatus::Waiting {
            record.approval = Some(bytes.to_vec());
            save(tx, key, &own, &approval.request, &record)?;
        }
    }
    let prior: Option<Vec<u8>> = tx
        .query_row(
            "SELECT CASE WHEN length(state)=237 THEN state END FROM group_controls WHERE id=?1",
            [message.as_slice()],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(prior) = prior {
        if key.open(&prior, &receipt_aad(&own, &receipt))?.as_slice() != marker {
            return Err(Error::Conflict);
        }
    } else {
        tx.execute(
            "INSERT INTO group_controls VALUES(?1,?2)",
            (
                message.as_slice(),
                key.seal(&marker, &receipt_aad(&own, &receipt))?,
            ),
        )?;
    }
    Ok(Some(marker))
}
impl ClientStore {
    pub fn group_invitation(&mut self, id: Id) -> Result<InvitationNotice, Error> {
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let record = load(&self.db, &self.key, &own, &id)?;
        let capsule = Capsule::parse(&record.capsule)?;
        Ok(InvitationNotice {
            id,
            group: capsule.group,
            peer: record.peer,
            status: record.status,
            outgoing: record.outgoing,
            expires_at: capsule.expires,
        })
    }
    pub fn prepare_group_invitation(
        &mut self,
        group: Id,
        peer: Id,
        id: Id,
        role: Role,
        expires: u64,
        now: u64,
    ) -> Result<InvitationNotice, Error> {
        self.prepare_invitation(group, peer, id, Some(role), expires, now)
    }
    pub fn prepare_group_device_invitation(
        &mut self,
        group: Id,
        peer: Id,
        id: Id,
        expires: u64,
        now: u64,
    ) -> Result<InvitationNotice, Error> {
        self.prepare_invitation(group, peer, id, None, expires, now)
    }
    fn prepare_invitation(
        &mut self,
        group: Id,
        peer: Id,
        id: Id,
        role: Option<Role>,
        expires: u64,
        now: u64,
    ) -> Result<InvitationNotice, Error> {
        if now == 0 || expires <= now || expires > now.saturating_add(604800) {
            return Err(Error::Expired);
        }
        self.refresh_group_authority_for_send(group, now)?;
        let binding = self.own_device_binding()?;
        let own = device_fingerprint(&binding)?;
        let own_fields = peers::parse(&binding)?.binding;
        let genesis = self.group_genesis(group)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = keys::current(&tx, &self.key, &own, &group)?;
        let member = state.device(own)?.0;
        if role.is_some() && member.role != Role::Admin {
            return Err(Error::Unprepared);
        }
        let target = peers::verified(&tx, &self.key, &peer)?;
        if target.fingerprint == own
            || state.device(target.fingerprint).is_ok()
            || (role.is_none()
                && (target.binding.server != own_fields.server
                    || target.binding.account != own_fields.account
                    || target.binding.username != own_fields.username))
        {
            return Err(Error::Unprepared);
        }
        let sealed:Vec<u8>=tx.query_row("SELECT CASE WHEN length(state)<=512 THEN state END FROM group_service WHERE group_id=?1",[group.as_slice()],|r|r.get(0))?;
        let context = self
            .key
            .open(&sealed, &service::aad(&group, &own, b"context"))?;
        service::load(&tx, &self.key, &group, &own, state.authority)?;
        let mut capsule = Capsule {
            device: role.is_none(),
            id,
            group,
            sender: own,
            target: target.fingerprint,
            member: if role.is_none() {
                member.id
            } else {
                digest(
                    b"Sigil/invited-member/v0",
                    &[&group, &id, &target.fingerprint],
                )
            },
            role: role.unwrap_or(member.role),
            expires,
            genesis,
            context,
            signature: [0; 64],
        };
        match load(&tx, &self.key, &own, &id) {
            Ok(previous) => {
                let old = Capsule::parse(&previous.capsule)?;
                if !previous.outgoing
                    || previous.peer != peer
                    || old.unsigned()?.as_slice() != capsule.unsigned()?.as_slice()
                {
                    return Err(Error::Conflict);
                }
            }
            Err(Error::NotFound) => {
                capsule.signature =
                    handshake::identity(&tx, &self.key)?.sign(&capsule.statement()?)?;
                save(
                    &tx,
                    &self.key,
                    &own,
                    &id,
                    &Record {
                        capsule: capsule.bytes()?,
                        peer,
                        outgoing: true,
                        status: InvitationStatus::Waiting,
                        cancelled_remote: false,
                        approval: None,
                    },
                )?;
            }
            Err(error) => return Err(error),
        }
        tx.commit()?;
        self.group_invitation(id)
    }
    /// Persist explicit joining consent; receiving an offer never joins a group.
    pub fn accept_group_invitation(&mut self, id: Id, now: u64) -> Result<(), Error> {
        if now == 0 || now > i64::MAX as u64 {
            return Err(Error::Expired);
        }
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut record = load(&tx, &self.key, &own, &id)?;
        let capsule = Capsule::parse(&record.capsule)?;
        capsule.verify(&peers::verified(&tx, &self.key, &record.peer)?, &own, now)?;
        if record.status == InvitationStatus::Cancelled {
            return Err(Error::Cancelled);
        }
        if record.outgoing {
            return Err(Error::Unprepared);
        }
        if record.status == InvitationStatus::Offered {
            record.status = InvitationStatus::Accepted;
            save(&tx, &self.key, &own, &id, &record)?;
        }
        tx.commit()?;
        Ok(())
    }
}

struct Approval {
    request: Id,
    group: Id,
    sender: Id,
    target: Id,
    head: Id,
    proposal: Vec<u8>,
}
impl Approval {
    fn message(&self) -> Id {
        digest(
            b"Sigil/group-join-approval-message/v0",
            &[
                &self.request,
                &self.group,
                &self.sender,
                &self.target,
                &self.head,
            ],
        )
    }
    fn bytes(&self) -> Result<Vec<u8>, Error> {
        if self.proposal.len() > MAX - 170 {
            return Err(Error::Limit);
        }
        let mut out = APPROVAL.to_vec();
        for id in [
            &self.request,
            &self.group,
            &self.sender,
            &self.target,
            &self.head,
        ] {
            out.extend_from_slice(id);
        }
        out.extend_from_slice(&(self.proposal.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.proposal);
        Ok(out)
    }
    fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX {
            return Err(Error::Limit);
        }
        let mut r = codec::Reader(bytes);
        if r.take(8)? != APPROVAL {
            return Err(Error::InvalidEvent);
        }
        let request = r.array()?;
        let group = r.array()?;
        let sender = r.array()?;
        let target = r.array()?;
        let head = r.array()?;
        let n = r.count(MAX - 170)?;
        let proposal = r.take(n)?.to_vec();
        if !r.0.is_empty() {
            return Err(Error::InvalidEvent);
        }
        Ok(Self {
            request,
            group,
            sender,
            target,
            head,
            proposal,
        })
    }
}
#[path = "group_bootstrap.rs"]
mod bootstrap;
#[path = "group_invitation_work.rs"]
mod work;
pub(crate) use bootstrap::MIGRATION as BOOTSTRAP_MIGRATION;
pub(super) fn awaiting_bootstrap(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    group: &Id,
) -> Result<bool, Error> {
    bootstrap::waiting(tx, key, own, group)
}
pub(crate) use work::cancelled_packet;
pub use work::InvitationAttempt;

pub(crate) fn reference(bytes: &[u8]) -> Result<Option<Id>, Error> {
    Ok(ControlReceipt::parse(bytes)?.map(|r| r.request))
}
#[cfg(test)]
#[path = "group_invitation_tests.rs"]
mod tests;
