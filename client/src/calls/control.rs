use super::*;
use std::borrow::Cow;
const WIRE: &[u8; 8] = b"SGCC\0\x01\0\0";
const RECEIPT: &[u8; 8] = b"SGCM\0\x01\0\0";
#[derive(Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub(super) enum Body {
    Invite(State),
    DirectInvite(State),
    Join(Attestation),
    State(State),
    Ready(Ready),
    Shares(Vec<Share>),
    Handoff {
        state: State,
        proof: sigil_calls::Delegation,
    },
    CancelInvite,
    Leave,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Wire {
    pub message: Id,
    pub call: Id,
    pub sender: Id,
    pub recipient: Id,
    pub expires: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub scoped: bool,
    pub body: Body,
}
impl Wire {
    pub fn bytes(&self) -> Result<Zeroizing<Vec<u8>>, Error> {
        let mut bytes = Zeroizing::new(WIRE.to_vec());
        serde_json::to_writer(&mut *bytes, self).map_err(|_| Error::InvalidEvent)?;
        if bytes.len() > 65536
            || self.message == [0; 32]
            || self.call == [0; 32]
            || self.sender == self.recipient
            || self.expires == 0
            || self.expires > i64::MAX as u64
        {
            return Err(Error::InvalidEvent);
        }
        Ok(bytes)
    }
}
fn parse(raw: &[u8]) -> Result<Option<Wire>, Error> {
    if !raw.starts_with(&WIRE[..4]) {
        return Ok(None);
    }
    if raw.len() > 65536 || !raw.starts_with(WIRE) {
        return Err(Error::InvalidEvent);
    }
    let value: Wire = serde_json::from_slice(&raw[8..]).map_err(|_| Error::InvalidEvent)?;
    if value.bytes()?.as_slice() != raw {
        return Err(Error::InvalidEvent);
    }
    Ok(Some(value))
}
pub(crate) fn scoped_wire(raw: &[u8]) -> Result<bool, Error> {
    Ok(parse(raw)?.is_some_and(|wire| wire.scoped))
}
pub(crate) fn retained<'a>(key: &StorageKey, raw: &'a [u8]) -> Result<Cow<'a, [u8]>, Error> {
    let Some(wire) = parse(raw)? else {
        return Ok(Cow::Borrowed(raw));
    };
    let mut bytes = RECEIPT.to_vec();
    for id in [&wire.message, &wire.call, &wire.sender, &wire.recipient] {
        bytes.extend_from_slice(id);
    }
    bytes.extend_from_slice(&wire.expires.to_be_bytes());
    bytes.extend_from_slice(&key.commitment(raw, b"Sigil/call-control-content/v1")?);
    let tag = key.commitment(&bytes, b"Sigil/call-control-receipt/v1")?;
    bytes.extend_from_slice(&tag);
    Ok(Cow::Owned(bytes))
}
fn marker(raw: &[u8]) -> Result<bool, Error> {
    if !raw.starts_with(&RECEIPT[..4]) {
        return Ok(false);
    }
    if raw.len() != 208 || !raw.starts_with(RECEIPT) {
        return Err(Error::InvalidEvent);
    }
    Ok(true)
}
fn array(raw: &[u8]) -> Result<Id, Error> {
    raw.try_into().map_err(|_| Error::InvalidEvent)
}
pub(crate) fn receipt_message(raw: &[u8]) -> Result<Option<Id>, Error> {
    if !marker(raw)? {
        return Ok(None);
    }
    Ok(Some(array(&raw[8..40])?))
}
pub(crate) fn receipt_call(raw: &[u8]) -> Result<Option<Id>, Error> {
    if !marker(raw)? {
        return Ok(None);
    }
    Ok(Some(array(&raw[40..72])?))
}
pub(super) fn authenticate_marker(key: &StorageKey, raw: &[u8]) -> Result<bool, Error> {
    if !marker(raw)? {
        return Ok(false);
    }
    if key
        .commitment(&raw[..176], b"Sigil/call-control-receipt/v1")?
        .as_slice()
        != &raw[176..]
    {
        return Err(Error::InvalidEvent);
    }
    Ok(true)
}
pub(crate) fn validate_receipt(
    db: &Connection,
    key: &StorageKey,
    own: &[u8],
    peer: &Id,
    message: &Id,
    raw: &[u8],
) -> Result<bool, Error> {
    if !authenticate_marker(key, raw)? {
        return Ok(false);
    }
    let known = crate::peers::known(db, key, peer)?;
    if array(&raw[8..40])? != *message
        || array(&raw[72..104])? != known.fingerprint
        || array(&raw[104..136])? != crate::device_fingerprint(own)?
    {
        return Err(Error::InvalidEvent);
    }
    Ok(true)
}
pub(crate) fn install(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &[u8],
    peer_id: &Id,
    message: &Id,
    raw: &[u8],
    now: u64,
) -> Result<Option<Vec<u8>>, Error> {
    let Some(wire) = parse(raw)? else {
        return Ok(None);
    };
    let known = crate::peers::known(tx, key, peer_id)?;
    let fingerprint = crate::device_fingerprint(own)?;
    if wire.message != *message || wire.sender != known.fingerprint || wire.recipient != fingerprint
    {
        return Err(Error::InvalidEvent);
    }
    let now = crate::conversations::time_floor(tx, key, now)?;
    if !known.verified {
        if !wire.scoped {
            return Err(Error::Unprepared);
        }
        let record = load(tx, key, &wire.call)?;
        record.active(now)?;
        channels::authorized(tx, key, &record, peer_id)?;
    }
    let marker = retained(key, raw)?.into_owned();
    if wire.expires <= now {
        return Ok(Some(marker));
    }
    if wire.expires > now.saturating_add(86400) {
        return Err(Error::InvalidEvent);
    }
    let mut record = match load(tx, key, &wire.call) {
        Ok(record) => record,
        Err(Error::NotFound) => {
            if !known.verified {
                return Err(Error::Unprepared);
            }
            let (Body::Invite(state) | Body::DirectInvite(state)) = &wire.body else {
                return Err(Error::NotFound);
            };
            state.verify().map_err(failure)?;
            state.roster.roster.active(now).map_err(failure)?;
            let direct = matches!(wire.body, Body::DirectInvite(_));
            if direct && state.participants.len() != 1 {
                return Err(Error::InvalidEvent);
            }
            let creator = state
                .participants
                .iter()
                .find(|p| p.member.key == state.roster.roster.controller())
                .ok_or(Error::InvalidEvent)?;
            if creator.fingerprint().map_err(failure)? != known.fingerprint
                || state.roster.roster.call != wire.call
                || state.roster.roster.expires < wire.expires
                || wire.expires > now.saturating_add(60)
                || state
                    .participants
                    .iter()
                    .any(|p| p.fingerprint().ok() == Some(fingerprint))
            {
                return Err(Error::InvalidEvent);
            }
            if tx.query_row("SELECT count(*) FROM calls", [], |r| r.get::<_, u32>(0))? >= 256 {
                return Err(Error::Limit);
            }
            let mut record = Record {
                direct,
                transfer: None,
                control_grants: Vec::new(),
                state: state.clone(),
                owner_peer: Some(*peer_id),
                own: None,
                secret: None,
                phase: Phase::Ringing,
                ring_until: Some(wire.expires),
                invites: Vec::new(),
                joining: Vec::new(),
                pins: Vec::new(),
                commits: Vec::new(),
                announced: None,
                announced_media: None,
                notify: Vec::new(),
                ready_sequence: 0,
                lease: [0; 32],
                generation: 0,
                shares: Vec::new(),
                connect_sequence: 0,
            };
            record.pin(state)?;
            for p in &state.participants {
                trust(tx, key, p)?;
                if p.fingerprint().map_err(failure)? != fingerprint {
                    crate::peers::observe(tx, key, &p.device)?;
                }
            }
            save(tx, key, &record)?;
            return Ok(Some(marker));
        }
        Err(e) => return Err(e),
    };
    channels::authorized(tx, key, &record, peer_id)?;
    if !known.verified && !wire.scoped {
        return Err(Error::Unprepared);
    }
    if !matches!(wire.body, Body::Invite(_) | Body::DirectInvite(_))
        && record.state.roster.roster.expires != wire.expires
    {
        return Err(Error::Conflict);
    }
    if matches!(wire.body, Body::Invite(_)) && record.direct
        || matches!(wire.body, Body::DirectInvite(_)) && !record.direct
    {
        return Err(Error::Conflict);
    }
    if record.expire(now) {
        save(tx, key, &record)?;
    }
    if matches!(record.phase, Phase::Declined | Phase::Left | Phase::Ended)
        || record.state.roster.roster.active(now).is_err()
    {
        return Ok(Some(marker));
    }
    match wire.body {
        Body::Invite(state) | Body::DirectInvite(state) | Body::State(state) => {
            state.verify().map_err(failure)?;
            if state.roster.roster.call != wire.call {
                return Err(Error::InvalidEvent);
            }
            if record.direct && state.participants.len() > 2 {
                return Err(Error::InvalidEvent);
            }
            if state.epoch < record.state.epoch {
                return Ok(Some(marker));
            }
            if state.epoch == record.state.epoch {
                if state.digest().map_err(failure)? != record.state.digest().map_err(failure)? {
                    return Err(Error::Conflict);
                }
                return Ok(Some(marker));
            }
            if record.owner_peer != Some(*peer_id) {
                return Err(Error::InvalidEvent);
            }
            if let Some(proof) = &record.transfer {
                if state
                    .roster
                    .delegations
                    .get(record.state.roster.delegations.len())
                    != Some(proof)
                {
                    return Err(Error::Conflict);
                }
            }
            record.state.successor(&state).map_err(failure)?;
            record.pin(&state)?;
            for p in &state.participants {
                trust(tx, key, p)?;
                if p.fingerprint().map_err(failure)? != fingerprint {
                    crate::peers::observe(tx, key, &p.device)?;
                }
            }
            record.state = state;
            record.transfer = None;
            record.shares.clear();
            if record.state.roster.roster.closed {
                record.finish(Phase::Ended);
            } else if let Some(own) = &record.own {
                if let Some(member) = record
                    .state
                    .participants
                    .iter()
                    .find(|p| p.member.id == own.member.id)
                {
                    if member.fingerprint().map_err(failure)? != fingerprint
                        || member.member != own.member
                    {
                        return Err(Error::Conflict);
                    }
                    record.phase = Phase::Active;
                    record.ring_until = None;
                } else if record.phase == Phase::Active {
                    record.finish(Phase::Left);
                }
            }
        }
        Body::Handoff { state, proof } => {
            if record.direct
                || record.owner_peer != Some(*peer_id)
                || record.transfer.is_some()
                || record.phase != Phase::Active
            {
                return Err(Error::InvalidEvent);
            }
            state.verify().map_err(failure)?;
            if state.epoch < record.state.epoch {
                return Ok(Some(marker));
            }
            if state.epoch == record.state.epoch {
                if state.digest().map_err(failure)? != record.state.digest().map_err(failure)? {
                    return Err(Error::Conflict);
                }
            } else {
                record.state.successor(&state).map_err(failure)?;
            }
            state.roster.verify_delegation(&proof).map_err(failure)?;
            record.pin(&state)?;
            for participant in &state.participants {
                trust(tx, key, participant)?;
                if participant.fingerprint().map_err(failure)? != fingerprint {
                    crate::peers::observe(tx, key, &participant.device)?;
                }
            }
            let successor = state
                .participants
                .iter()
                .find(|p| p.member.key == proof.controller)
                .ok_or(Error::InvalidEvent)?;
            let controller = peer(successor)?;
            record.state = state;
            record.erase_media();
            if record
                .own
                .as_ref()
                .is_some_and(|own| own.member.key == proof.controller)
            {
                let mut next = record.state.clone();
                next.roster = next
                    .roster
                    .transfer(proof, &record.key(key)?)
                    .map_err(failure)?;
                next.participants
                    .retain(|p| next.roster.roster.member(p.member.id).is_ok());
                next.ready
                    .retain(|p| next.roster.roster.member(p.member).is_ok());
                record.owner_peer = None;
                record.change(key, next)?;
            } else {
                record.owner_peer = Some(controller);
                record.transfer = Some(proof);
            }
        }
        Body::CancelInvite => {
            if record.owner_peer != Some(*peer_id) {
                return Err(Error::InvalidEvent);
            }
            if matches!(record.phase, Phase::Ringing | Phase::Joining) {
                record.finish(Phase::Declined);
            }
        }
        Body::Join(proof) => {
            if record.owner_peer.is_some() {
                return Err(Error::InvalidEvent);
            }
            proof.verify(&record.state.roster.roster).map_err(failure)?;
            if proof.fingerprint().map_err(failure)? != known.fingerprint {
                return Err(Error::InvalidEvent);
            }
            if let Some(prior) = record
                .state
                .participants
                .iter()
                .find(|p| p.fingerprint().ok() == Some(known.fingerprint))
            {
                if prior.member != proof.member {
                    return Err(Error::Conflict);
                }
                return Ok(Some(marker));
            }
            if !record
                .invites
                .iter()
                .any(|i| i.peer == *peer_id && i.fingerprint == known.fingerprint && i.until > now)
            {
                return Ok(Some(marker));
            }
            if let Some(old) = record
                .joining
                .iter()
                .find(|p| p.fingerprint().ok() == Some(known.fingerprint))
            {
                if old.member != proof.member {
                    return Err(Error::Conflict);
                }
                return Ok(Some(marker));
            }
            if record.state.participants.len() + record.joining.len() >= 8 {
                return Err(Error::Limit);
            }
            record.joining.push(proof);
            record.invites.retain(|i| i.peer != *peer_id);
        }
        Body::Ready(ready) => {
            if record.owner_peer.is_some() {
                return Err(Error::InvalidEvent);
            }
            let Some(proof) = record
                .state
                .participants
                .iter()
                .find(|p| p.fingerprint().ok() == Some(known.fingerprint))
            else {
                return Ok(Some(marker));
            };
            if ready.member != proof.member.id {
                return Err(Error::InvalidEvent);
            }
            ready.verify(&record.state.roster.roster).map_err(failure)?;
            if let Some(old) = record.state.ready.iter().find(|v| v.member == ready.member) {
                if old.sequence > ready.sequence {
                    return Ok(Some(marker));
                }
                if old.sequence == ready.sequence {
                    if *old != ready {
                        return Err(Error::Conflict);
                    }
                    return Ok(Some(marker));
                }
            }
            let mut state = record.state.clone();
            state.ready.retain(|v| v.member != ready.member);
            state.ready.push(ready);
            state.ready.sort_by_key(|v| v.member);
            record.change(key, state)?;
        }
        Body::Shares(shares) => {
            if shares.is_empty()
                || shares.len() > 8
                || (record.owner_peer.is_none() && shares.len() != 1)
            {
                return Err(Error::InvalidEvent);
            }
            let mut senders = std::collections::BTreeSet::new();
            for share in shares {
                if !senders.insert(share.key.context.sender) {
                    return Err(Error::InvalidEvent);
                }
                if share.state != record.state.digest().map_err(failure)? {
                    return Ok(Some(marker));
                }
                share.verify(&record.state).map_err(failure)?;
                if let Some(owner) = record.owner_peer {
                    if owner != *peer_id {
                        return Err(Error::InvalidEvent);
                    }
                } else {
                    let proof = record
                        .state
                        .participants
                        .iter()
                        .find(|p| p.member.id == share.key.context.sender)
                        .ok_or(Error::InvalidEvent)?;
                    if proof.fingerprint().map_err(failure)? != known.fingerprint {
                        return Err(Error::InvalidEvent);
                    }
                }
                if let Some(old) = record
                    .shares
                    .iter()
                    .find(|v| v.key.context.sender == share.key.context.sender)
                {
                    if old.generation > share.generation {
                        continue;
                    }
                    if old.generation == share.generation {
                        if old.key.context != share.key.context || old.signature != share.signature
                        {
                            return Err(Error::Conflict);
                        }
                        continue;
                    }
                }
                record
                    .shares
                    .retain(|v| v.key.context.sender != share.key.context.sender);
                record.shares.push(share);
            }
        }
        Body::Leave => {
            if record.owner_peer.is_some() {
                return Err(Error::InvalidEvent);
            }
            let invited = record
                .invites
                .iter()
                .any(|i| i.peer == *peer_id && i.fingerprint == known.fingerprint);
            record.invites.retain(|i| i.peer != *peer_id);
            record
                .joining
                .retain(|p| p.fingerprint().ok() != Some(known.fingerprint));
            if let Some(member) = record
                .state
                .participants
                .iter()
                .find(|p| p.fingerprint().ok() == Some(known.fingerprint))
                .map(|p| p.member.id)
            {
                if record.direct {
                    end(key, &mut record)?;
                } else {
                    remove(key, &mut record, member)?;
                }
            } else if record.direct && invited {
                end(key, &mut record)?;
            }
        }
    }
    save(tx, key, &record)?;
    Ok(Some(marker))
}
