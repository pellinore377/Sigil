//! Verified same-account devices replay signed membership before approving addition.
use super::super::storage_record::{open_record, seal_record};
use super::*;
const WIRE: &[u8; 8] = b"SGGB\0\x01\0\0";
const CHUNK: usize = 3072;
const LIMIT: usize = MAX_PROPOSAL_BYTES + 208;
pub(crate) const MIGRATION: &str = "CREATE TABLE group_bootstrap(id BLOB PRIMARY KEY REFERENCES group_invitations(id),state BLOB NOT NULL); PRAGMA user_version=59;";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Frame {
    request: Id,
    group: Id,
    sender: Id,
    target: Id,
    head: Id,
    root: Id,
    offset: u32,
    total: u32,
    kind: u8,
    payload: Vec<u8>,
}
impl Frame {
    fn bytes(&self) -> Result<Vec<u8>, Error> {
        if self.kind > 2
            || self.total as usize > LIMIT
            || self.offset > self.total
            || self.payload.len() > CHUNK
            || self.offset as usize + self.payload.len() > self.total as usize
            || (self.kind != 1 && !self.payload.is_empty())
            || (self.kind == 1 && self.payload.is_empty())
            || (self.kind == 2 && (self.offset != 0 || self.total != 0 || self.root != [0; 32]))
        {
            return Err(Error::InvalidEvent);
        }
        let mut out = WIRE.to_vec();
        for id in [
            &self.request,
            &self.group,
            &self.sender,
            &self.target,
            &self.head,
            &self.root,
        ] {
            out.extend_from_slice(id);
        }
        out.extend_from_slice(&self.offset.to_be_bytes());
        out.extend_from_slice(&self.total.to_be_bytes());
        out.push(self.kind);
        out.extend_from_slice(&self.payload);
        Ok(out)
    }
    fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > CHUNK + 209 {
            return Err(Error::Limit);
        }
        let mut r = codec::Reader(bytes);
        if r.take(8)? != WIRE {
            return Err(Error::InvalidEvent);
        }
        let value = Self {
            request: r.array()?,
            group: r.array()?,
            sender: r.array()?,
            target: r.array()?,
            head: r.array()?,
            root: r.array()?,
            offset: u32::from_be_bytes(r.array()?),
            total: u32::from_be_bytes(r.array()?),
            kind: r.array::<1>()?[0],
            payload: r.0.to_vec(),
        };
        if value.bytes()?.as_slice() != bytes {
            return Err(Error::InvalidEvent);
        }
        Ok(value)
    }
    fn message(&self) -> Result<Id, Error> {
        Ok(digest(
            b"Sigil/group-bootstrap-message/v0",
            &[&self.bytes()?],
        ))
    }
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Progress {
    request: Option<Frame>,
    head: Option<Id>,
    root: Id,
    total: u32,
    #[serde(skip)]
    bytes: Zeroizing<Vec<u8>>,
    ready: bool,
}
fn binding(own: &Id, id: &Id) -> Vec<u8> {
    [b"Sigil/group-bootstrap-store/v0".as_slice(), own, id].concat()
}
fn progress(db: &Connection, key: &StorageKey, own: &Id, id: &Id) -> Result<Progress, Error> {
    let value: Option<Vec<u8>> = db.query_row("SELECT CASE WHEN length(state)<=1048576 THEN state END FROM group_bootstrap WHERE id=?1", [id.as_slice()], |r|r.get(0)).optional()?;
    let value: Progress = match value {
        None => Progress::default(),
        Some(v) => {
            let raw = open_record(key, &v, &binding(own, id))?;
            let (size, rest) = raw.split_at_checked(4).ok_or(Error::InvalidStore)?;
            let size =
                u32::from_be_bytes(size.try_into().map_err(|_| Error::InvalidStore)?) as usize;
            let (header, bytes) = rest.split_at_checked(size).ok_or(Error::InvalidStore)?;
            let mut p: Progress =
                serde_json::from_slice(header).map_err(|_| Error::InvalidStore)?;
            p.bytes.extend_from_slice(bytes);
            p
        }
    };
    if value.total as usize > LIMIT || value.bytes.len() > value.total as usize {
        return Err(Error::InvalidStore);
    }
    Ok(value)
}
fn save_progress(
    db: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    id: &Id,
    value: &Progress,
) -> Result<(), Error> {
    let header = Zeroizing::new(serde_json::to_vec(value).map_err(|_| Error::InvalidStore)?);
    let mut bytes = Zeroizing::new((header.len() as u32).to_be_bytes().to_vec());
    bytes.extend_from_slice(&header);
    bytes.extend_from_slice(&value.bytes);
    let sealed = seal_record(key, &bytes, &binding(own, id))?;
    if sealed.len() > 1048576 {
        return Err(Error::Limit);
    }
    db.execute("INSERT INTO group_bootstrap VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET state=excluded.state", (id.as_slice(),sealed))?;
    Ok(())
}
pub(super) fn is_wire(bytes: &[u8]) -> bool {
    bytes.starts_with(&WIRE[..4])
}
pub(super) fn retained(key: &StorageKey, bytes: &[u8]) -> Result<Option<Vec<u8>>, Error> {
    if !is_wire(bytes) {
        return Ok(None);
    }
    let f = Frame::parse(bytes)?;
    Ok(Some(
        ControlReceipt {
            kind: 2,
            message: f.message()?,
            request: f.request,
            group: f.group,
            sender: f.sender,
            target: f.target,
            tag: key.commitment(bytes, b"Sigil/invitation-control-tag/v0")?,
        }
        .bytes(),
    ))
}
fn same_account(own: &[u8], peer: &peers::Peer) -> Result<(), Error> {
    let own = peers::parse(own)?.binding;
    if own.server != peer.binding.server
        || own.account != peer.binding.account
        || own.username != peer.binding.username
    {
        return Err(Error::Unprepared);
    }
    Ok(())
}
fn waiting_key(group: &Id) -> Vec<u8> {
    [b"device-bootstrap".as_slice(), group].concat()
}
pub(super) fn waiting(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    group: &Id,
) -> Result<bool, Error> {
    let cursor = super::super::work::load(tx, key, own, &waiting_key(group))?;
    let Some(id) = cursor.after else {
        return Ok(false);
    };
    let record = load(tx, key, own, &id)?;
    let capsule = Capsule::parse(&record.capsule)?;
    if record.outgoing || !capsule.device || capsule.group != *group || capsule.target != *own {
        return Err(Error::InvalidStore);
    }
    Ok(record.status == InvitationStatus::Accepted)
}
pub(super) fn clear_waiting(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    group: &Id,
    id: &Id,
) -> Result<(), Error> {
    let name = waiting_key(group);
    if super::super::work::load(tx, key, own, &name)?.after == Some(*id) {
        tx.execute("DELETE FROM group_work WHERE id=?1", [name])?;
    }
    Ok(())
}
pub(super) fn install(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    peer: &Id,
    bytes: &[u8],
    expires: u64,
) -> Result<(), Error> {
    let f = Frame::parse(bytes)?;
    let record = load(tx, key, own, &f.request)?;
    let capsule = Capsule::parse(&record.capsule)?;
    let known = peers::trusted(tx, key, peer)?;
    same_account(&peers::own(tx, key)?, &known)?;
    if !capsule.device
        || record.peer != *peer
        || capsule.group != f.group
        || f.sender != known.fingerprint
        || f.target != *own
        || expires > capsule.expires
    {
        return Err(Error::Conflict);
    }
    if record.status == InvitationStatus::Cancelled
        || (!record.outgoing && record.status == InvitationStatus::Joined)
    {
        return Ok(());
    }
    let mut p = progress(tx, key, own, &f.request)?;
    if record.outgoing {
        if f.kind != 0 {
            return Err(Error::Conflict);
        }
        p.request = Some(f);
    } else {
        if record.status != InvitationStatus::Accepted || f.kind == 0 {
            return Err(Error::Unprepared);
        }
        let state = store::load(tx, key, own, &capsule.group)?;
        if state.frozen {
            return Err(Error::Conflict);
        }
        if f.head != state.state.head {
            return Ok(());
        }
        if p.head != Some(f.head) {
            p = Progress {
                head: Some(f.head),
                ..Progress::default()
            };
        }
        if f.kind == 2 {
            if !p.bytes.is_empty() {
                return Err(Error::Conflict);
            }
            p.ready = true;
        } else {
            if f.offset as usize != p.bytes.len() {
                return Err(Error::Conflict);
            }
            if f.offset == 0 {
                p.root = f.root;
                p.total = f.total;
            }
            if f.root != p.root || f.total != p.total {
                return Err(Error::Conflict);
            }
            p.ready = false;
            p.bytes.extend_from_slice(&f.payload);
        }
    }
    save_progress(tx, key, own, &capsule.id, &p)
}
impl Capsule {
    fn device_change(&self, state: &State, target_binding: &[u8]) -> Result<Change, Error> {
        let member = state.device(self.sender)?.0;
        if !self.device || member.id != self.member {
            return Err(Error::Unprepared);
        }
        let target = peers::parse(target_binding)?.binding;
        let first = &member.devices[0].binding;
        if target.server != first.server
            || target.account != first.account
            || target.username != first.username
            || device_fingerprint(target_binding)? != self.target
        {
            return Err(Error::Conflict);
        }
        Ok(Change::AddDevice {
            member: self.member,
            binding: target_binding.to_vec(),
        })
    }
}
impl ClientStore {
    pub(super) fn advance_group_device_invitation_online(
        &mut self,
        id: Id,
        now: u64,
    ) -> Result<InvitationNotice, Error> {
        let binding = self.own_device_binding()?;
        let own = device_fingerprint(&binding)?;
        let mut record = load(&self.db, &self.key, &own, &id)?;
        let capsule = Capsule::parse(&record.capsule)?;
        let peer = peers::trusted(&self.db, &self.key, &record.peer)?;
        same_account(&binding, &peer)?;
        if record.status == InvitationStatus::Cancelled {
            self.cancel_group_invitation(id)?;
            return self.group_invitation(id);
        }
        if record.status == InvitationStatus::Offered
            || (!record.outgoing && record.status == InvitationStatus::Joined)
        {
            return self.group_invitation(id);
        }
        if capsule.expires <= now {
            return Err(Error::Expired);
        }
        if record.outgoing {
            if !self.sync_group_service_online(capsule.group, now)? {
                return self.group_invitation(id);
            }
            let state = self.group_status(capsule.group)?.state;
            if state.closed || state.device(own)?.0.id != capsule.member {
                return Err(Error::Unprepared);
            }
            self.queue_invitation_control_online(
                record.peer,
                capsule.message(),
                &record.capsule,
                capsule.expires,
                now,
            )?;
            if let Some(bytes) = record.approval.as_deref() {
                let approval = Approval::parse(bytes)?;
                if approval.head == state.head && state.device(capsule.target).is_err() {
                    if self.group_service_request_pending(capsule.group)? {
                        if state.device(own)?.0.role == Role::Admin {
                            self.submit_group_service_online(capsule.group, now)?;
                        } else {
                            self.submit_group_relay_online(capsule.group, now)?;
                        }
                    } else {
                        let target = peers::statement(&self.db, &self.key, &record.peer)?;
                        let expected =
                            state.propose(own, capsule.device_change(&state, &target)?)?;
                        let mut proposal = state.proposal_from_bytes(&approval.proposal)?;
                        if proposal.signing_bytes() != expected.signing_bytes()
                            || !proposal
                                .signatures
                                .iter()
                                .any(|(id, _)| *id == capsule.target)
                        {
                            return Err(Error::Conflict);
                        }
                        let tx = self.db.transaction()?;
                        proposal.sign(own, &handshake::identity(&tx, &self.key)?)?;
                        tx.commit()?;
                        state.authorize(&proposal)?;
                        self.prepare_group_service_request(
                            capsule.group,
                            Some(&proposal.to_bytes()?),
                        )?;
                        if state.device(own)?.0.role == Role::Admin {
                            self.submit_group_service_online(capsule.group, now)?;
                        } else {
                            self.submit_group_relay_online(capsule.group, now)?;
                        }
                    }
                }
            }
            let p = progress(&self.db, &self.key, &own, &id)?;
            if let Some(request) = p.request {
                let current = self.group_status(capsule.group)?.state;
                let bytes = self.group_commit_after(capsule.group, request.head)?;
                let response = if let Some(bytes) = bytes {
                    let root = digest(b"Sigil/group-bootstrap-commit/v0", &[&bytes]);
                    let offset = request.offset as usize;
                    if offset >= bytes.len()
                        || (offset != 0
                            && (request.root != root || request.total as usize != bytes.len()))
                    {
                        return Err(Error::Conflict);
                    }
                    Frame {
                        request: id,
                        group: capsule.group,
                        sender: own,
                        target: capsule.target,
                        head: request.head,
                        root,
                        offset: request.offset,
                        total: bytes.len() as u32,
                        kind: 1,
                        payload: bytes[offset..bytes.len().min(offset + CHUNK)].to_vec(),
                    }
                } else {
                    if request.head != current.head || request.offset != 0 {
                        return Err(Error::Conflict);
                    }
                    Frame {
                        request: id,
                        group: capsule.group,
                        sender: own,
                        target: capsule.target,
                        head: request.head,
                        root: [0; 32],
                        offset: 0,
                        total: 0,
                        kind: 2,
                        payload: Vec::new(),
                    }
                };
                self.queue_invitation_control_online(
                    record.peer,
                    response.message()?,
                    &response.bytes()?,
                    capsule.expires,
                    now,
                )?;
                if current.device(capsule.target).is_ok() && request.head == current.head {
                    record.status = InvitationStatus::Joined;
                    record.approval = None;
                    let tx = self.db.transaction()?;
                    save(&tx, &self.key, &own, &id, &record)?;
                    tx.commit()?;
                }
            }
        } else {
            capsule.verify(&peer, &own, now)?;
            let tx = self
                .db
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            store::save_genesis(
                &tx,
                &self.key,
                &own,
                &Genesis::from_invitation(&capsule.genesis)?,
            )?;
            super::super::work::save(
                &tx,
                &self.key,
                &own,
                &waiting_key(&capsule.group),
                &super::super::work::Cursor {
                    after: Some(id),
                    relay: None,
                },
            )?;
            tx.commit()?;
            let master = Zeroizing::new(
                <Id>::try_from(&capsule.context[..32]).map_err(|_| Error::InvalidStore)?,
            );
            self.pin_group_service(capsule.group, &capsule.context[32..], master)?;
            let mut state = self.group_status(capsule.group)?.state;
            let mut p = progress(&self.db, &self.key, &own, &id)?;
            if p.head == Some(state.head) && p.total != 0 && p.bytes.len() == p.total as usize {
                if digest(b"Sigil/group-bootstrap-commit/v0", &[&p.bytes]) != p.root
                    || p.bytes.len() < 208
                {
                    return Err(Error::Conflict);
                }
                if self.commit_group_proposal(capsule.group, &p.bytes[208..], &p.bytes[..208])?
                    == CommitResult::Frozen
                {
                    return Err(Error::Conflict);
                }
                state = self.group_status(capsule.group)?.state;
            }
            if p.head != Some(state.head) {
                p = Progress {
                    head: Some(state.head),
                    ..Progress::default()
                };
                let tx = self.db.transaction()?;
                save_progress(&tx, &self.key, &own, &id, &p)?;
                tx.commit()?;
            }
            let request = Frame {
                request: id,
                group: capsule.group,
                sender: own,
                target: capsule.sender,
                head: state.head,
                root: p.root,
                offset: p.bytes.len() as u32,
                total: p.total,
                kind: 0,
                payload: Vec::new(),
            };
            self.queue_invitation_control_online(
                record.peer,
                request.message()?,
                &request.bytes()?,
                capsule.expires,
                now,
            )?;
            if state.device(own).is_ok() {
                if !self.sync_group_service_online(capsule.group, now)? {
                    return self.group_invitation(id);
                }
                let state = self.group_status(capsule.group)?.state;
                if state.device(own)?.0.id != capsule.member {
                    return Err(Error::Conflict);
                }
                record.status = InvitationStatus::Joined;
                record.approval = None;
                let tx = self.db.transaction()?;
                clear_waiting(&tx, &self.key, &own, &capsule.group, &id)?;
                save(&tx, &self.key, &own, &id, &record)?;
                tx.commit()?;
            } else if p.ready {
                let expected =
                    state.propose(capsule.sender, capsule.device_change(&state, &binding)?)?;
                let prior = record
                    .approval
                    .as_deref()
                    .map(Approval::parse)
                    .transpose()?;
                let approval = if let Some(prior) = prior.filter(|a| a.head == state.head) {
                    prior
                } else {
                    let tx = self.db.transaction()?;
                    let mut proposal = expected;
                    proposal.sign(own, &handshake::identity(&tx, &self.key)?)?;
                    tx.commit()?;
                    Approval {
                        request: id,
                        group: capsule.group,
                        sender: own,
                        target: capsule.sender,
                        head: state.head,
                        proposal: proposal.to_bytes()?,
                    }
                };
                record.approval = Some(approval.bytes()?);
                let tx = self.db.transaction()?;
                save(&tx, &self.key, &own, &id, &record)?;
                tx.commit()?;
                self.queue_invitation_control_online(
                    record.peer,
                    approval.message(),
                    &approval.bytes()?,
                    capsule.expires,
                    now,
                )?;
            }
        }
        self.group_invitation(id)
    }
}
