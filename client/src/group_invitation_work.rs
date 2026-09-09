use super::*;
use crate::{claims, selection, send_in, transport};
use sigil_protocol::groups::{InvitationState, Operation};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PacketRecord {
    session: Id,
    receipt: Vec<u8>,
    cancelled: bool,
}
fn packet_aad(own: &Id, id: &Id) -> Vec<u8> {
    [b"Sigil/group-invitation-packet/v0".as_slice(), own, id].concat()
}
fn packet(
    db: &Connection,
    key: &StorageKey,
    own: &Id,
    id: &Id,
) -> Result<Option<PacketRecord>, Error> {
    let row:Option<(Vec<u8>,Vec<u8>)>=db.query_row("SELECT invitation,CASE WHEN length(state)<=4096 THEN state END FROM group_invitation_packets WHERE id=?1",[id.as_slice()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    row.map(|(request, sealed)| {
        let record: PacketRecord =
            serde_json::from_slice(&key.open(&sealed, &packet_aad(own, id))?)
                .map_err(|_| Error::InvalidStore)?;
        let receipt = ControlReceipt::parse(&record.receipt)?.ok_or(Error::InvalidStore)?;
        if receipt.message != *id || receipt.request.as_slice() != request || receipt.sender != *own
        {
            return Err(Error::InvalidStore);
        }
        Ok(record)
    })
    .transpose()
}
fn save_packet(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    id: &Id,
    record: &PacketRecord,
) -> Result<(), Error> {
    let receipt = ControlReceipt::parse(&record.receipt)?.ok_or(Error::InvalidStore)?;
    let bytes = serde_json::to_vec(record).map_err(|_| Error::InvalidStore)?;
    tx.execute("INSERT INTO group_invitation_packets VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET state=excluded.state",(id.as_slice(),receipt.request.as_slice(),key.seal(&bytes,&packet_aad(own,id))?))?;
    Ok(())
}
#[derive(Debug)]
pub struct InvitationAttempt {
    pub id: Id,
    pub result: Result<InvitationNotice, Error>,
}
pub(crate) fn cancelled_packet(
    tx: &Transaction<'_>,
    key: &StorageKey,
    session: &Id,
    id: &Id,
) -> Result<bool, Error> {
    if !tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM group_invitation_packets WHERE id=?1)",
        [id.as_slice()],
        |r| r.get::<_, bool>(0),
    )? {
        return Ok(false);
    }
    let own = device_fingerprint(&peers::own(tx, key)?)?;
    let record = packet(tx, key, &own, id)?.ok_or(Error::InvalidStore)?;
    if record.session != *session {
        return Err(Error::InvalidStore);
    }
    Ok(record.cancelled)
}
fn cancel_packets(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    id: &Id,
) -> Result<usize, Error> {
    let ids = tx.prepare("SELECT p.id FROM group_invitation_packets p JOIN outbox o ON o.id=p.id WHERE p.invitation=?1 AND o.packet IS NOT NULL ORDER BY p.id LIMIT 16")?.query_map([id.as_slice()], |r| r.get::<_, Vec<u8>>(0))?.collect::<Result<Vec<_>, _>>()?;
    for bytes in &ids {
        let message: Id = bytes
            .as_slice()
            .try_into()
            .map_err(|_| Error::InvalidStore)?;
        let mut record = packet(tx, key, own, &message)?.ok_or(Error::InvalidStore)?;
        if ControlReceipt::parse(&record.receipt)?
            .ok_or(Error::InvalidStore)?
            .request
            != *id
        {
            return Err(Error::InvalidStore);
        }
        record.cancelled = true;
        save_packet(tx, key, own, &message, &record)?;
        tx.execute(
            "UPDATE outbox SET packet=NULL WHERE id=?1 AND session=?2",
            (message.as_slice(), record.session.as_slice()),
        )?;
    }
    Ok(ids.len())
}
impl ClientStore {
    pub fn cancel_group_invitation(&mut self, id: Id) -> Result<usize, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let own = device_fingerprint(&peers::own(&tx, &self.key)?)?;
        let mut record = load(&tx, &self.key, &own, &id)?;
        if record.status == InvitationStatus::Joined {
            return Err(Error::Obsolete);
        }
        record.status = InvitationStatus::Cancelled;
        record.approval = None;
        let capsule = Capsule::parse(&record.capsule)?;
        if capsule.device && !record.outgoing {
            bootstrap::clear_waiting(&tx, &self.key, &own, &capsule.group, &id)?;
        }
        save(&tx, &self.key, &own, &id, &record)?;
        let count = cancel_packets(&tx, &self.key, &own, &id)?;
        tx.commit()?;
        Ok(count)
    }
    pub(crate) fn check_group_invitation_send(
        &mut self,
        session: Id,
        id: Id,
        now: u64,
    ) -> Result<(), Error> {
        let tx = self.db.transaction()?;
        if !tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM group_invitation_packets WHERE id=?1)",
            [id.as_slice()],
            |r| r.get::<_, bool>(0),
        )? {
            return Ok(());
        }
        let own = device_fingerprint(&peers::own(&tx, &self.key)?)?;
        let record = packet(&tx, &self.key, &own, &id)?.ok_or(Error::InvalidStore)?;
        if record.session != session {
            return Err(Error::InvalidStore);
        }
        let receipt = ControlReceipt::parse(&record.receipt)?.ok_or(Error::InvalidStore)?;
        let invitation = load(&tx, &self.key, &own, &receipt.request)?;
        if record.cancelled || invitation.status == InvitationStatus::Cancelled {
            return Err(Error::Cancelled);
        }
        let capsule = Capsule::parse(&invitation.capsule)?;
        tx.commit()?;
        if invitation.outgoing {
            self.refresh_group_authority_for_send(capsule.group, now)?;
            let tx = self.db.transaction()?;
            let record = packet(&tx, &self.key, &own, &id)?.ok_or(Error::InvalidStore)?;
            let invitation = load(&tx, &self.key, &own, &receipt.request)?;
            if record.cancelled || invitation.status == InvitationStatus::Cancelled {
                return Err(Error::Cancelled);
            }
            let state = keys::current(&tx, &self.key, &own, &capsule.group)?;
            let member = state.device(own)?.0;
            if (capsule.device && member.id != capsule.member)
                || (!capsule.device && member.role != Role::Admin)
            {
                return Err(Error::Unprepared);
            }
            tx.commit()?;
        }
        Ok(())
    }
    fn revoke_group_invitation_online(
        &mut self,
        id: Id,
        now: u64,
    ) -> Result<InvitationNotice, Error> {
        self.cancel_group_invitation(id)?;
        let binding = self.own_device_binding()?;
        let own = device_fingerprint(&binding)?;
        let record = load(&self.db, &self.key, &own, &id)?;
        let capsule = Capsule::parse(&record.capsule)?;
        if !record.cancelled_remote && capsule.expires > now {
            let genesis = Genesis::from_invitation(&capsule.genesis)?;
            let master = Zeroizing::new(
                <Id>::try_from(&capsule.context[..32]).map_err(|_| Error::InvalidStore)?,
            );
            let access = service::Access {
                profile: Authority::from_pinned_bytes(
                    &capsule.context[32..],
                    genesis.state.authority,
                )?,
                key: sigil_crypto::private_credentials::GroupKey::from_master(
                    sigil_crypto::Secret32::from_bytes(*master),
                )?,
                encryption: StorageKey::new(sigil_crypto::Secret32::from_bytes(*master))?,
            };
            let client = access.client(self)?;
            let credential =
                self.cached_group_credential(&client, &access.profile, &binding, now)?;
            let statement = if record.outgoing {
                peers::statement(&self.db, &self.key, &record.peer)?
            } else {
                binding
            };
            let issuance = access.profile.issuance(
                &statement,
                u32::try_from(now / 86400).map_err(|_| Error::Expired)?,
            )?;
            let reply = client.group_request(
                &access.profile,
                &credential,
                &access.key,
                capsule.group,
                &Operation::Invite {
                    id: hex(&id),
                    target: hex(&access.key.ciphertext(&issuance.attributes()?)),
                    expires_at: 0,
                },
                now,
            )?;
            let invitation = reply.invitation.ok_or(Error::InvalidStore)?;
            if invitation.id != hex(&id) {
                return Err(Error::Conflict);
            }
            match invitation.state {
                InvitationState::Cancelled | InvitationState::Expired => {}
                InvitationState::Consumed => return Err(Error::Obsolete),
                InvitationState::Pending => return Err(Error::Conflict),
            }
        }
        let tx = self.db.transaction()?;
        let mut record = load(&tx, &self.key, &own, &id)?;
        record.cancelled_remote = true;
        save(&tx, &self.key, &own, &id, &record)?;
        tx.commit()?;
        self.group_invitation(id)
    }
    pub(super) fn queue_invitation_control_online(
        &mut self,
        peer: Id,
        message: Id,
        bytes: &[u8],
        expires: u64,
        now: u64,
    ) -> Result<(), Error> {
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let receipt = retained(&self.key, bytes)?.ok_or(Error::InvalidEvent)?;
        let control = ControlReceipt::parse(&receipt)?.ok_or(Error::InvalidStore)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        peers::trusted(&tx, &self.key, &peer)?;
        if load(&tx, &self.key, &own, &control.request)?.status == InvitationStatus::Cancelled {
            return Err(Error::Cancelled);
        }
        if let Some(prior) = packet(&tx, &self.key, &own, &message)? {
            if prior.receipt != receipt || crate::session_peer(&tx, &prior.session)? != Some(peer) {
                return Err(Error::Conflict);
            }
            return if prior.cancelled {
                Err(Error::Cancelled)
            } else {
                Ok(())
            };
        }
        let needs_claim = match selection::for_send(&tx, &self.key, &peer, now) {
            Ok(_) => false,
            Err(Error::Unprepared) => true,
            Err(e) => return Err(e),
        };
        tx.commit()?;
        let claim = digest(b"Sigil/group-control-claim/v0", &[&own, &message]);
        if needs_claim {
            self.prepare_peer_claim(claim, peer)?;
            self.claim_prekey_online(claim, now)?;
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let known = peers::trusted(&tx, &self.key, &peer)?;
        let authorization = load(&tx, &self.key, &own, &control.request)?;
        if authorization.status == InvitationStatus::Cancelled {
            return Err(Error::Cancelled);
        }
        if authorization.peer != peer
            || control.sender != own
            || control.target != known.fingerprint
        {
            return Err(Error::Conflict);
        }
        if let Some(prior) = packet(&tx, &self.key, &own, &message)? {
            if prior.receipt != receipt {
                return Err(Error::Conflict);
            }
            return if prior.cancelled {
                Err(Error::Cancelled)
            } else {
                Ok(())
            };
        }
        let session = match selection::for_send(&tx, &self.key, &peer, now) {
            Ok(session) => {
                let state = crate::load(&tx, &self.key, &session)?.1;
                let expiry = if !state.peer_confirmed() {
                    handshake::prepare_expiry(&tx, &self.key, &session, None, now)?.min(expires)
                } else {
                    expires
                };
                send_in(&tx, &self.key, session, message, bytes)?;
                transport::prepare(
                    &tx,
                    &self.key,
                    session,
                    message,
                    known.binding.device,
                    Some(expiry),
                    now,
                )?;
                session
            }
            Err(Error::Unprepared) if needs_claim => {
                let session = digest(b"Sigil/group-control-session/v0", &[&own, &message]);
                claims::start_in(
                    &tx,
                    &self.key,
                    claim,
                    session,
                    message,
                    (bytes, None, Some(expires)),
                    now,
                )?;
                session
            }
            Err(error) => return Err(error),
        };
        save_packet(
            &tx,
            &self.key,
            &own,
            &message,
            &PacketRecord {
                session,
                receipt,
                cancelled: false,
            },
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn resume_group_invitations_online(
        &mut self,
        now: u64,
    ) -> Result<Vec<InvitationAttempt>, Error> {
        if now == 0 || now > i64::MAX as u64 {
            return Err(Error::Expired);
        }
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let tx = self.db.transaction()?;
        let cursor = super::super::work::load(&tx, &self.key, &own, b"invitations")?;
        let after = cursor.after.map(|v| v.to_vec()).unwrap_or_default();
        let next: Option<Vec<u8>> = tx
            .query_row(
                "SELECT id FROM group_invitations WHERE id>?1 ORDER BY id LIMIT 1",
                [after],
                |r| r.get(0),
            )
            .optional()?;
        let id = next
            .map(|v| v.try_into().map_err(|_| Error::InvalidStore))
            .transpose()?;
        super::super::work::save(
            &tx,
            &self.key,
            &own,
            b"invitations",
            &super::super::work::Cursor {
                after: id,
                relay: None,
            },
        )?;
        tx.commit()?;
        Ok(id
            .map(|id| InvitationAttempt {
                id,
                result: self.advance_group_invitation_online(id, now),
            })
            .into_iter()
            .collect())
    }
    pub fn advance_group_invitation_online(
        &mut self,
        id: Id,
        now: u64,
    ) -> Result<InvitationNotice, Error> {
        if now == 0 || now > i64::MAX as u64 {
            return Err(Error::Expired);
        }
        let binding = self.own_device_binding()?;
        let own = device_fingerprint(&binding)?;
        let mut record = load(&self.db, &self.key, &own, &id)?;
        let capsule = Capsule::parse(&record.capsule)?;
        if capsule.device {
            return self.advance_group_device_invitation_online(id, now);
        }
        if record.status == InvitationStatus::Cancelled {
            return self.revoke_group_invitation_online(id, now);
        }
        if matches!(
            record.status,
            InvitationStatus::Offered | InvitationStatus::Joined
        ) {
            return self.group_invitation(id);
        }
        if capsule.expires <= now {
            return Err(Error::Expired);
        }
        if !record.outgoing {
            capsule.verify(
                &peers::trusted(&self.db, &self.key, &record.peer)?,
                &own,
                now,
            )?;
            let tx = self
                .db
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            store::save_genesis(
                &tx,
                &self.key,
                &own,
                &Genesis::from_invitation(&capsule.genesis)?,
            )?;
            tx.commit()?;
            let master = Zeroizing::new(
                <Id>::try_from(&capsule.context[..32]).map_err(|_| Error::InvalidEvent)?,
            );
            self.pin_group_service(capsule.group, &capsule.context[32..], master)?;
        }
        if !self.sync_group_service_online(capsule.group, now)? {
            return self.group_invitation(id);
        }
        let state = self.group_status(capsule.group)?.state;
        if state.device(capsule.target).is_ok() {
            record.status = InvitationStatus::Joined;
            record.approval = None;
            let tx = self.db.transaction()?;
            save(&tx, &self.key, &own, &id, &record)?;
            tx.commit()?;
            return self.group_invitation(id);
        }
        if state.device(capsule.sender)?.0.role != Role::Admin {
            return Err(Error::Unprepared);
        }
        if record.outgoing {
            let target = peers::trusted(&self.db, &self.key, &record.peer)?;
            if target.fingerprint != capsule.target {
                return Err(Error::Conflict);
            }
            let statement = peers::statement(&self.db, &self.key, &record.peer)?;
            let access = service::load(&self.db, &self.key, &capsule.group, &own, state.authority)?;
            let client = access.client(self)?;
            let credential =
                self.cached_group_credential(&client, &access.profile, &binding, now)?;
            let issuance = access.profile.issuance(
                &statement,
                u32::try_from(now / 86400).map_err(|_| Error::Expired)?,
            )?;
            let operation = Operation::Invite {
                id: hex(&id),
                target: hex(&access.key.ciphertext(&issuance.attributes()?)),
                expires_at: capsule.expires,
            };
            let reply = client.group_request(
                &access.profile,
                &credential,
                &access.key,
                capsule.group,
                &operation,
                now,
            )?;
            let invitation = reply.invitation.ok_or(Error::InvalidStore)?;
            if invitation.id != hex(&id) {
                return Err(Error::Conflict);
            }
            match invitation.state {
                InvitationState::Pending => {}
                InvitationState::Consumed => return Err(Error::Unprepared),
                InvitationState::Cancelled | InvitationState::Expired => {
                    self.cancel_group_invitation(id)?;
                    let tx = self.db.transaction()?;
                    let mut cancelled = load(&tx, &self.key, &own, &id)?;
                    cancelled.cancelled_remote = true;
                    save(&tx, &self.key, &own, &id, &cancelled)?;
                    tx.commit()?;
                    return self.group_invitation(id);
                }
            }
            let tx = self.db.transaction()?;
            record = load(&tx, &self.key, &own, &id)?;
            if record.status == InvitationStatus::Cancelled {
                return Err(Error::Cancelled);
            }
            tx.commit()?;
            self.queue_invitation_control_online(
                record.peer,
                capsule.message(),
                &record.capsule,
                capsule.expires,
                now,
            )?;
            if let Some(bytes) = record.approval {
                if self.group_service_request_pending(capsule.group)? {
                    self.submit_group_service_online(capsule.group, now)?;
                    return self.group_invitation(id);
                }
                let approval = Approval::parse(&bytes)?;
                if approval.head != state.head {
                    return self.group_invitation(id);
                }
                let member = Member::new(capsule.member, capsule.role, &[statement])?;
                let expected = state.propose(own, Change::Add(member))?;
                let mut proposal = state.proposal_from_bytes(&approval.proposal)?;
                if expected.signing_bytes() != proposal.signing_bytes()
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
                self.prepare_group_service_request(capsule.group, Some(&proposal.to_bytes()?))?;
                self.submit_group_service_online(capsule.group, now)?;
            }
        } else {
            let member = Member::new(capsule.member, capsule.role, &[binding])?;
            let expected = state.propose(capsule.sender, Change::Add(member))?;
            let tx = self
                .db
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            record = load(&tx, &self.key, &own, &id)?;
            if record.status != InvitationStatus::Accepted {
                return Err(Error::Cancelled);
            }
            if store::load(&tx, &self.key, &own, &capsule.group)?
                .state
                .head
                != state.head
            {
                return Err(Error::Obsolete);
            }
            let previous = record
                .approval
                .as_deref()
                .map(Approval::parse)
                .transpose()?;
            let approval = if let Some(prior) = previous.filter(|p| p.head == state.head) {
                let proposal = state.proposal_from_bytes(&prior.proposal)?;
                if prior.request != id
                    || prior.group != capsule.group
                    || prior.sender != own
                    || prior.target != capsule.sender
                    || proposal.signing_bytes() != expected.signing_bytes()
                    || !proposal.signatures.iter().any(|(id, _)| *id == own)
                {
                    return Err(Error::InvalidStore);
                }
                prior
            } else {
                let mut proposal = expected;
                proposal.sign(own, &handshake::identity(&tx, &self.key)?)?;
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
        self.group_invitation(id)
    }
}
