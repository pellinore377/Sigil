use crate::{ClientStore, Error, Id};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sigil_calls::{Attestation, Member, Ready, Roster, Share, SignedRoster, State, Tracks};
use sigil_crypto::{storage::StorageKey, IdentityKey};
use zeroize::Zeroizing;
mod channels;
mod control;
mod history;
pub(crate) use channels::scoped as scoped_channel;
pub(crate) use history::MIGRATION as HISTORY_MIGRATION;
pub use history::{History, HistoryPerson};
pub(crate) use jobs::delivery_peer;
#[cfg(test)]
mod failure_tests;
#[cfg(test)]
mod group_tests;
mod jobs;
mod media;
#[cfg(test)]
pub(crate) mod network_tests;
#[cfg(feature = "rtc-client")]
mod rtc_transport;
mod signaling;
#[cfg(feature = "rtc-client")]
pub use rtc_transport::{ReceivedFrame, RtcCall};
#[cfg(test)]
mod tests;
pub(crate) use control::{
    install, receipt_call, receipt_message, retained, scoped_wire, validate_receipt,
};
pub(crate) use jobs::check_retained;
pub use jobs::Attempt;
pub use media::Media;
pub(crate) const MIGRATION:&str="CREATE TABLE calls(id BLOB PRIMARY KEY,content BLOB NOT NULL); CREATE TABLE call_jobs(id BLOB PRIMARY KEY,content BLOB NOT NULL); CREATE TABLE call_cursor(id INTEGER PRIMARY KEY CHECK(id=1),content BLOB NOT NULL); PRAGMA user_version=67;";
fn failure(error: sigil_calls::Error) -> Error {
    match error {
        sigil_calls::Error::Conflict | sigil_calls::Error::Replay => Error::Conflict,
        sigil_calls::Error::Expired => Error::Expired,
        sigil_calls::Error::Limit => Error::Limit,
        _ => Error::InvalidEvent,
    }
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Ringing,
    Joining,
    Active,
    Declined,
    Left,
    Ended,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Invite {
    peer: Id,
    fingerprint: Id,
    until: u64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    direct: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    transfer: Option<sigil_calls::Delegation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    control_grants: Vec<Id>,
    state: State,
    owner_peer: Option<Id>,
    own: Option<Attestation>,
    secret: Option<Vec<u8>>,
    phase: Phase,
    ring_until: Option<u64>,
    invites: Vec<Invite>,
    joining: Vec<Attestation>,
    pins: Vec<(Id, Member)>,
    commits: Vec<SignedRoster>,
    announced: Option<Id>,
    announced_media: Option<Id>,
    notify: Vec<Id>,
    ready_sequence: u64,
    lease: Id,
    generation: u64,
    shares: Vec<Share>,
    connect_sequence: u64,
}
pub struct Participant {
    pub member: Id,
    pub device: Vec<u8>,
    pub verified: bool,
    pub tracks: Option<Tracks>,
}
pub struct Call {
    pub id: Id,
    pub phase: Phase,
    pub owner: Id,
    pub expires: u64,
    pub ring_until: Option<u64>,
    pub participants: Vec<Participant>,
    pub direct: bool,
    pub created: u64,
    pub own: Option<Id>,
}
fn aad(id: &Id) -> Vec<u8> {
    [b"Sigil/native-call/v1".as_slice(), id].concat()
}
fn index(key: &StorageKey, id: &Id) -> Result<Id, Error> {
    if *id == [0; 32] {
        return Err(Error::InvalidEvent);
    }
    Ok(key.commitment(id, b"Sigil/native-call-index/v1")?)
}
fn load(db: &Connection, key: &StorageKey, id: &Id) -> Result<Record, Error> {
    load_index(db, key, &index(key, id)?)
}
fn load_index(db: &Connection, key: &StorageKey, row: &Id) -> Result<Record, Error> {
    let raw: Vec<u8> = db
        .query_row(
            "SELECT CASE WHEN length(content)<=262180 THEN content END FROM calls WHERE id=?1",
            [row.as_slice()],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    let record: Record =
        serde_json::from_slice(&key.open(&raw, &aad(row))?).map_err(|_| Error::InvalidStore)?;
    if index(key, &record.id())? != *row
        || record.pins.len() > 256
        || record.invites.len() > 32
        || record.joining.len() > 7
        || record.shares.len() > 8
        || record.commits.len() > 16
        || record.notify.len() > 256
        || record.control_grants.len() > 8
    {
        return Err(Error::InvalidStore);
    }
    record
        .state
        .roster
        .roster
        .validate()
        .map_err(|_| Error::InvalidStore)?;
    Ok(record)
}
fn save(db: &Connection, key: &StorageKey, record: &Record) -> Result<(), Error> {
    if db.is_autocommit() {
        let tx = db.unchecked_transaction()?;
        save(&tx, key, record)?;
        tx.commit()?;
        return Ok(());
    }
    let id = record.state.roster.roster.call;
    let raw = Zeroizing::new(serde_json::to_vec(record).map_err(|_| Error::InvalidStore)?);
    if raw.len() > 262144 {
        return Err(Error::Limit);
    }
    let row = index(key, &id)?;
    db.execute(
        "INSERT INTO calls VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET content=excluded.content",
        (row.as_slice(), key.seal(&raw, &aad(&row))?),
    )?;
    history::retain(db, key, record)?;
    Ok(())
}
fn peer(proof: &Attestation) -> Result<Id, Error> {
    let signed = crate::peers::parse(&proof.device)?;
    Ok(crate::peers::reference(
        &signed.binding.server,
        &signed.binding.device,
    ))
}
fn trust(db: &Connection, key: &StorageKey, proof: &Attestation) -> Result<bool, Error> {
    let id = peer(proof)?;
    match crate::peers::known(db, key, &id) {
        Ok(known) => {
            if known.blocked
                || known.fingerprint != proof.fingerprint().map_err(failure)?
                || known.changed_fingerprint.is_some()
                || known.replaced_by.is_some()
            {
                return Err(Error::Conflict);
            }
            Ok(known.verified)
        }
        Err(Error::NotFound) => Ok(false),
        Err(e) => Err(e),
    }
}
impl Record {
    fn id(&self) -> Id {
        self.state.roster.roster.call
    }
    fn key(&self, key: &StorageKey) -> Result<IdentityKey, Error> {
        Ok(IdentityKey::open_checkpoint(
            key,
            self.secret.as_ref().ok_or(Error::Unprepared)?,
            &[aad(&self.id()), b"participant".to_vec()].concat(),
        )?)
    }
    fn own_id(&self) -> Result<Id, Error> {
        Ok(self.own.as_ref().ok_or(Error::Unprepared)?.member.id)
    }
    fn active(&self, now: u64) -> Result<(), Error> {
        self.state.roster.roster.active(now).map_err(failure)?;
        if self.phase != Phase::Active {
            return Err(Error::Unprepared);
        }
        Ok(())
    }
    fn authorize(&self, db: &Connection, key: &StorageKey, now: u64) -> Result<(), Error> {
        self.active(now)?;
        if self.transfer.is_some() {
            return Err(Error::Unprepared);
        }
        for proof in &self.state.participants {
            if self
                .own
                .as_ref()
                .is_none_or(|own| own.member != proof.member)
            {
                trust(db, key, proof)?;
            }
        }
        Ok(())
    }
    fn expire(&mut self, now: u64) -> bool {
        if now >= self.state.roster.roster.expires {
            self.finish(Phase::Ended);
            return true;
        }
        if self.phase == Phase::Ringing && self.ring_until.is_some_and(|until| now >= until) {
            self.finish(Phase::Declined);
            return true;
        }
        false
    }
    fn erase_media(&mut self) {
        self.shares.clear();
        self.lease = [0; 32];
    }
    fn finish(&mut self, phase: Phase) {
        self.phase = phase;
        self.ring_until = None;
        self.secret = None;
        self.erase_media();
        self.invites.clear();
        self.joining.clear();
    }
    fn pin(&mut self, state: &State) -> Result<(), Error> {
        for proof in &state.participants {
            let fingerprint = proof.fingerprint().map_err(failure)?;
            if let Some((_, old)) = self.pins.iter().find(|(id, _)| *id == fingerprint) {
                if *old != proof.member {
                    return Err(Error::Conflict);
                }
            } else {
                if self.pins.len() >= 256 {
                    return Err(Error::Limit);
                }
                self.pins.push((fingerprint, proof.member.clone()));
            }
        }
        Ok(())
    }
    fn change(&mut self, key: &StorageKey, mut state: State) -> Result<(), Error> {
        state.epoch = self.state.epoch.checked_add(1).ok_or(Error::Limit)?;
        state = state.sign(&self.key(key)?).map_err(failure)?;
        self.state.successor(&state).map_err(failure)?;
        self.pin(&state)?;
        if self.state.roster.roster != state.roster.roster {
            if self.commits.len() >= 16 {
                return Err(Error::Limit);
            }
            self.commits.push(state.roster.clone());
        }
        let own = peer(self.own.as_ref().ok_or(Error::Unprepared)?)?;
        for proof in self.state.participants.iter().chain(&state.participants) {
            let id = peer(proof)?;
            if id != own && !self.notify.contains(&id) {
                if self.notify.len() >= 256 {
                    return Err(Error::Limit);
                }
                self.notify.push(id);
            }
        }
        self.state = state;
        self.shares.clear();
        self.announced = None;
        self.announced_media = None;
        Ok(())
    }
    fn view(&self, db: &Connection, key: &StorageKey) -> Result<Call, Error> {
        let mut participants = Vec::new();
        for proof in &self.state.participants {
            participants.push(Participant {
                member: proof.member.id,
                device: proof.device.clone(),
                verified: self
                    .own
                    .as_ref()
                    .is_some_and(|own| own.member == proof.member)
                    || match trust(db, key, proof) {
                        Ok(value) => value,
                        Err(Error::Conflict) => false,
                        Err(e) => return Err(e),
                    },
                tracks: self
                    .state
                    .ready
                    .iter()
                    .find(|r| r.member == proof.member.id)
                    .map(|r| r.tracks),
            });
        }
        Ok(Call {
            id: self.id(),
            phase: self.phase,
            owner: self.state.roster.roster.controller(),
            expires: self.state.roster.roster.expires,
            ring_until: self.ring_until,
            participants,
            direct: self.direct,
            created: self.state.roster.roster.created,
            own: self.own.as_ref().map(|v| v.member.id),
        })
    }
}
impl ClientStore {
    pub fn create_call(&mut self, id: Id, now: u64, lifetime: u32) -> Result<Call, Error> {
        self.create_call_kind(id, now, lifetime, false, false, &[])
    }
    pub fn create_direct_call(&mut self, id: Id, now: u64, lifetime: u32) -> Result<Call, Error> {
        self.create_call_kind(id, now, lifetime, true, false, &[])
    }
    pub fn create_group_call(&mut self, id: Id, now: u64, lifetime: u32) -> Result<Call, Error> {
        self.create_call_kind(id, now, lifetime, false, true, &[])
    }
    pub(crate) fn start_call(
        &mut self,
        id: Id,
        now: u64,
        direct: bool,
        recipients: &[Id],
    ) -> Result<Call, Error> {
        if recipients.is_empty() || recipients.len() > 7 || direct && recipients.len() != 1 {
            return Err(Error::Limit);
        }
        self.create_call_kind(id, now, 86400, direct, !direct, recipients)
    }
    #[allow(clippy::too_many_arguments)]
    fn create_call_kind(
        &mut self,
        id: Id,
        now: u64,
        lifetime: u32,
        direct: bool,
        continuation: bool,
        recipients: &[Id],
    ) -> Result<Call, Error> {
        if !(60..=86400).contains(&lifetime) {
            return Err(Error::InvalidEvent);
        }
        let own = self.own_device_binding()?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = crate::conversations::time_floor(&tx, &self.key, now)?;
        match load(&tx, &self.key, &id) {
            Ok(_) => return Err(Error::Conflict),
            Err(Error::NotFound) => (),
            Err(e) => return Err(e),
        }
        if tx.query_row("SELECT count(*) FROM calls", [], |r| r.get::<_, u32>(0))? >= 256 {
            return Err(Error::Limit);
        }
        let binding = crate::peers::parse(&own)?;
        let identity = crate::handshake::identity(&tx, &self.key)?;
        let secret = IdentityKey::generate()?;
        let roster = Roster {
            controller: continuation.then(|| secret.public_key()),
            version: if continuation { 2 } else { 1 },
            call: id,
            server: binding.binding.server,
            owner: secret.public_key(),
            created: now,
            expires: now.checked_add(u64::from(lifetime)).ok_or(Error::Limit)?,
            revision: 0,
            previous: None,
            members: vec![Member::new(secret.public_key())],
            closed: false,
        }
        .sign(&secret)
        .map_err(failure)?;
        let proof = Attestation::sign(
            &roster.roster,
            roster.roster.members[0].clone(),
            own,
            &identity,
        )
        .map_err(failure)?;
        let state = State {
            roster: roster.clone(),
            epoch: 0,
            participants: vec![proof.clone()],
            ready: Vec::new(),
            signature: Vec::new(),
        }
        .sign(&secret)
        .map_err(failure)?;
        let mut record = Record {
            direct,
            transfer: None,
            control_grants: Vec::new(),
            state,
            owner_peer: None,
            own: Some(proof.clone()),
            secret: Some(
                secret.seal_checkpoint(&self.key, &[aad(&id), b"participant".to_vec()].concat())?,
            ),
            phase: Phase::Active,
            ring_until: None,
            invites: Vec::new(),
            joining: Vec::new(),
            pins: vec![(proof.fingerprint().map_err(failure)?, proof.member)],
            commits: vec![roster],
            announced: None,
            announced_media: None,
            notify: Vec::new(),
            ready_sequence: 0,
            lease: [0; 32],
            generation: 0,
            shares: Vec::new(),
            connect_sequence: 0,
        };
        for recipient in recipients {
            invite(&tx, &self.key, &mut record, *recipient, now)?;
        }
        save(&tx, &self.key, &record)?;
        let view = record.view(&tx, &self.key)?;
        tx.commit()?;
        Ok(view)
    }
    pub fn call(&mut self, id: Id, now: u64) -> Result<Call, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = crate::conversations::time_floor(&tx, &self.key, now)?;
        let mut record = load(&tx, &self.key, &id)?;
        if record.expire(now) {
            save(&tx, &self.key, &record)?;
        }
        let view = record.view(&tx, &self.key)?;
        tx.commit()?;
        Ok(view)
    }
    pub fn invite_to_call(&mut self, id: Id, recipient: Id, now: u64) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = crate::conversations::time_floor(&tx, &self.key, now)?;
        let mut record = load(&tx, &self.key, &id)?;
        invite(&tx, &self.key, &mut record, recipient, now)?;
        save(&tx, &self.key, &record)?;
        tx.commit()?;
        Ok(())
    }
    pub fn answer_call(&mut self, id: Id, accept: bool, now: u64) -> Result<(), Error> {
        let own = self.own_device_binding()?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = crate::conversations::time_floor(&tx, &self.key, now)?;
        let mut record = load(&tx, &self.key, &id)?;
        record.state.roster.roster.active(now).map_err(failure)?;
        let owner = record.owner_peer.ok_or(Error::Unprepared)?;
        if accept {
            crate::peers::verified(&tx, &self.key, &owner)?;
        }
        if record.ring_until.is_some_and(|until| now >= until) {
            return Err(Error::Expired);
        }
        if record.phase != Phase::Ringing {
            return Err(Error::Conflict);
        }
        if accept {
            let secret = IdentityKey::generate()?;
            let identity = crate::handshake::identity(&tx, &self.key)?;
            let proof = Attestation::sign(
                &record.state.roster.roster,
                Member::new(secret.public_key()),
                own,
                &identity,
            )
            .map_err(failure)?;
            record.secret = Some(
                secret.seal_checkpoint(&self.key, &[aad(&id), b"participant".to_vec()].concat())?,
            );
            record.own = Some(proof.clone());
            record.phase = Phase::Joining;
            record.ring_until = None;
            jobs::queue(
                &tx,
                &self.key,
                &record,
                owner,
                control::Body::Join(proof),
                now,
            )?;
        } else {
            record.finish(Phase::Declined);
            jobs::leave(&tx, &self.key, &record, owner, now)?;
        }
        save(&tx, &self.key, &record)?;
        tx.commit()?;
        Ok(())
    }
    pub fn leave_call(&mut self, id: Id, now: u64) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = crate::conversations::time_floor(&tx, &self.key, now)?;
        let mut record = load(&tx, &self.key, &id)?;
        if matches!(record.phase, Phase::Declined | Phase::Left | Phase::Ended) {
            return Ok(());
        }
        if let Some(owner) = record.owner_peer {
            record.finish(Phase::Left);
            jobs::leave(&tx, &self.key, &record, owner, now)?;
        } else {
            record.active(now)?;
            if !record.direct
                && record.state.roster.roster.version == 2
                && record.state.participants.len() > 1
            {
                let successor = record
                    .state
                    .participants
                    .iter()
                    .filter(|p| p.member.id != record.own_id().unwrap_or([0; 32]))
                    .filter(|p| {
                        peer(p).is_ok_and(|id| {
                            channels::authorized(&tx, &self.key, &record, &id).is_ok()
                        })
                    })
                    .min_by_key(|p| {
                        (
                            !record.state.ready.iter().any(|r| r.member == p.member.id),
                            p.member.id,
                        )
                    });
                if let Some(successor) = successor {
                    let proof = record
                        .state
                        .roster
                        .delegate(successor.member.key, &record.key(&self.key)?)
                        .map_err(failure)?;
                    for participant in &record.state.participants {
                        let peer = peer(participant)?;
                        if participant.member.id != record.own_id()?
                            && channels::authorized(&tx, &self.key, &record, &peer).is_ok()
                        {
                            jobs::queue(
                                &tx,
                                &self.key,
                                &record,
                                peer,
                                control::Body::Handoff {
                                    state: record.state.clone(),
                                    proof: proof.clone(),
                                },
                                now,
                            )?;
                        }
                    }
                    for peer in pending_peers(&record)? {
                        jobs::queue(
                            &tx,
                            &self.key,
                            &record,
                            peer,
                            control::Body::CancelInvite,
                            now,
                        )?;
                    }
                    record.transfer = Some(proof);
                    record.finish(Phase::Left);
                } else {
                    end(&self.key, &mut record)?;
                }
            } else {
                end(&self.key, &mut record)?;
            }
        }
        save(&tx, &self.key, &record)?;
        tx.commit()?;
        Ok(())
    }
    pub fn remove_call_participant(&mut self, id: Id, member: Id, now: u64) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = crate::conversations::time_floor(&tx, &self.key, now)?;
        let mut record = load(&tx, &self.key, &id)?;
        record.active(now)?;
        if record.owner_peer.is_some() || record.own_id()? == member {
            return Err(Error::InvalidEvent);
        }
        remove(&self.key, &mut record, member)?;
        save(&tx, &self.key, &record)?;
        tx.commit()?;
        Ok(())
    }
}
fn invite(
    tx: &Transaction<'_>,
    key: &StorageKey,
    record: &mut Record,
    recipient: Id,
    now: u64,
) -> Result<(), Error> {
    record.active(now)?;
    if record.owner_peer.is_some() {
        return Err(Error::Unprepared);
    }
    let known = crate::peers::verified(tx, key, &recipient)?;
    if record
        .state
        .participants
        .iter()
        .any(|p| p.fingerprint().ok() == Some(known.fingerprint))
    {
        return Ok(());
    }
    record.invites.retain(|i| i.until > now);
    if record.direct
        && (record.state.participants.len() > 1
            || record.invites.iter().any(|v| v.peer != recipient)
            || record
                .joining
                .iter()
                .any(|v| v.fingerprint().ok() != Some(known.fingerprint)))
    {
        return Err(Error::Conflict);
    }
    if !record.invites.iter().any(|i| i.peer == recipient) {
        if record.invites.len() >= 32 {
            return Err(Error::Limit);
        }
        record.invites.push(Invite {
            peer: recipient,
            fingerprint: known.fingerprint,
            until: now
                .saturating_add(60)
                .min(record.state.roster.roster.expires),
        });
    }
    jobs::queue(
        tx,
        key,
        record,
        recipient,
        if record.direct {
            control::Body::DirectInvite(record.state.clone())
        } else {
            control::Body::Invite(record.state.clone())
        },
        now,
    )
}
fn end(key: &StorageKey, record: &mut Record) -> Result<(), Error> {
    for peer in pending_peers(record)? {
        if !record.notify.contains(&peer) {
            if record.notify.len() == 256 {
                return Err(Error::Limit);
            }
            record.notify.push(peer);
        }
    }
    let mut state = record.state.clone();
    state.roster.roster.previous = Some(state.roster.roster.digest().map_err(failure)?);
    state.roster.roster.revision = state
        .roster
        .roster
        .revision
        .checked_add(1)
        .ok_or(Error::Limit)?;
    state.roster.roster.closed = true;
    state.roster = record
        .state
        .roster
        .update(state.roster.roster, &record.key(key)?)
        .map_err(failure)?;
    record.change(key, state)?;
    record.finish(Phase::Ended);
    Ok(())
}
fn pending_peers(record: &Record) -> Result<Vec<Id>, Error> {
    let mut peers = record
        .invites
        .iter()
        .map(|invite| invite.peer)
        .collect::<Vec<_>>();
    for proof in &record.joining {
        peers.push(peer(proof)?);
    }
    peers.sort();
    peers.dedup();
    Ok(peers)
}
fn remove(key: &StorageKey, record: &mut Record, member: Id) -> Result<(), Error> {
    record.state.roster.roster.member(member).map_err(failure)?;
    let mut state = record.state.clone();
    state.roster.roster.previous = Some(state.roster.roster.digest().map_err(failure)?);
    state.roster.roster.revision += 1;
    state.roster.roster.members.retain(|m| m.id != member);
    state.participants.retain(|m| m.member.id != member);
    state.ready.retain(|m| m.member != member);
    state.roster = record
        .state
        .roster
        .update(state.roster.roster, &record.key(key)?)
        .map_err(failure)?;
    record.change(key, state)
}
