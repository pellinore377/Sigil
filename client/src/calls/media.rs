use super::*;
mod detached;
pub use detached::{MediaUpdate, MediaProcessor};
#[cfg(test)]
#[path = "media_tests.rs"]
mod tests;
use std::collections::BTreeMap;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;
pub struct Media {
    pub(super) call: Id,
    pub(super) lease: Id,
    state: Option<Id>,
    sender: Option<sigil_calls::Sender>,
    external: bool,
    /// The sender key as last shared, re-signed rather than replaced when only readiness changes.
    share: Option<sigil_calls::KeyShare>,
    /// The roster the receivers were established under; only a new roster discards them.
    roster: Option<Id>,
    /// Each member's receiver challenge as last seen; a new one anywhere means fresh keys.
    challenges: Vec<(Id, Id)>,
    receivers: BTreeMap<Id, (u64, sigil_calls::Receiver, sigil_calls::Context)>,
    assembly: sigil_calls::Assembly,
    checked: Option<Checked>,
}
/// What sealing and opening need from the record. Media runs at tens of frames a second in each
/// direction and the record only moves when the mailbox brings a new state, so reading it per
/// frame put a write transaction, and a full state digest, in front of every frame. On a device
/// whose sync pass holds the same database for hundreds of milliseconds that is where the
/// picture fell behind.
struct Checked {
    at: Instant,
    /// The store's authority revision when this was read; a peer blocked or re-identified since
    /// moves it, and the check is taken again before the next frame rather than on a timer.
    authority: u64,
    expires: u64,
    own: Id,
    tracks: Vec<(Id, Tracks)>,
}
/// How long an authority check stands. The refresh loop already re-reads once a second.
const RECHECK: std::time::Duration = std::time::Duration::from_millis(200);
impl Media {
    pub fn call(&self) -> Id {
        self.call
    }
    pub(super) fn assemble(
        &mut self,
        sender: Id,
        kind: sigil_calls::MediaKind,
        packet: &[u8],
    ) -> Result<Option<Vec<u8>>, Error> {
        self.assembly
            .push(sender, kind, packet, Instant::now())
            .map_err(failure)
    }
}
impl ClientStore {
    /// Each new handle demands a fresh receiver challenge, including after a crash.
    pub fn start_call_media(&mut self, id: Id, tracks: Tracks, now: u64) -> Result<Media, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = crate::conversations::time_floor(&tx, &self.key, now)?;
        let mut record = load(&tx, &self.key, &id)?;
        record.authorize(&tx, &self.key, now)?;
        // The lease declared with the answer serves the first handle; the refresh takes it from there.
        if record.owner_peer.is_some() && record.early.take() == Some(tracks) {
            let media = Media {
                call: id,
                lease: record.lease,
                state: None,
                sender: None,
                external: false,
                share: None,
                roster: Some(record.state.roster.roster.digest().map_err(failure)?),
                challenges: challenges(&record.state),
                receivers: BTreeMap::new(),
                assembly: sigil_calls::Assembly::default(),
                checked: None,
            };
            save(&tx, &self.key, &record)?;
            tx.commit()?;
            return Ok(media);
        }
        record.lease = sigil_calls::random_id().map_err(failure)?;
        record.shares.clear();
        let declared = ready(&tx, &self.key, &mut record, tracks, None, now)?;
        let (state, sender, share) = match declared {
            Some((digest, sender, key)) => (Some(digest), sender, Some(key)),
            None => (None, None, None),
        };
        let media = Media {
            call: id,
            lease: record.lease,
            state,
            sender,
            external: false,
            share,
            roster: Some(record.state.roster.roster.digest().map_err(failure)?),
            challenges: challenges(&record.state),
            receivers: BTreeMap::new(),
            assembly: sigil_calls::Assembly::default(),
            checked: None,
        };
        save(&tx, &self.key, &record)?;
        tx.commit()?;
        Ok(media)
    }
    pub fn set_call_tracks(
        &mut self,
        media: &mut Media,
        tracks: Tracks,
        now: u64,
    ) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = crate::conversations::time_floor(&tx, &self.key, now)?;
        let mut record = load(&tx, &self.key, &media.call)?;
        record.authorize(&tx, &self.key, now)?;
        if record.lease != media.lease {
            return Err(Error::Obsolete);
        }
        // Tracks changing on the same roster keep the sender key; the members already hold it.
        let reuse = media.share.clone().filter(|_| media.sender.is_some() || media.external);
        let declared = ready(&tx, &self.key, &mut record, tracks, reuse, now)?;
        save(&tx, &self.key, &record)?;
        tx.commit()?;
        media.checked = None;
        match declared {
            Some((digest, sender, key)) => {
                media.state = Some(digest);
                media.share = Some(key);
                if let Some(sender) = sender {
                    media.sender = Some(sender);
                    media.external = false;
                    media.receivers.clear();
                    media.assembly.clear();
                }
            }
            None if record.owner_peer.is_none() => {
                // The owner commits readiness directly; refresh re-signs the existing key.
                media.state = None;
            }
            None => {
                media.state = None;
                media.sender = None;
                media.external = false;
                media.share = None;
                media.receivers.clear();
                media.assembly.clear();
            }
        }
        Ok(())
    }
    pub fn refresh_call_media(&mut self, media: &mut Media, now: u64) -> Result<usize, Error> {
        media.checked = None;
        let record = self.refresh_media_record(media, now)?;
        authority(&record, media, now)?;
        media.checked = Some(Checked {
            at: Instant::now(),
            authority: self.authority,
            expires: record.state.roster.roster.expires,
            own: record.own_id()?,
            tracks: record.state.ready.iter().map(|r| (r.member, r.tracks)).collect(),
        });
        Ok(media.receivers.len())
    }
    pub(super) fn checked_call_media(&mut self, media: &mut Media, now: u64) -> Result<usize, Error> {
        if media.checked.as_ref().is_some_and(|checked|
            checked.at.elapsed() < RECHECK && checked.authority == self.authority && now < checked.expires
        ) {
            return Ok(media.receivers.len());
        }
        self.refresh_call_media(media, now)
    }
    /// The authority behind one frame: the stored record when the check has lapsed, the last
    /// check while it stands. Returns our own member id, which sealing needs.
    fn media_gate(
        &mut self,
        media: &mut Media,
        sender: Option<Id>,
        kind: sigil_calls::MediaKind,
        now: u64,
    ) -> Result<Id, Error> {
        self.checked_call_media(media, now)?;
        let checked = media.checked.as_ref().ok_or(Error::Unprepared)?;
        match checked.tracks.iter().find(|(id, _)| *id == sender.unwrap_or(checked.own)) {
            Some((_, tracks)) if track(*tracks, kind) => Ok(checked.own),
            _ => Err(Error::Unprepared),
        }
    }
    fn refresh_media_record(&mut self, media: &mut Media, now: u64) -> Result<Record, Error> {
        // An unchanged call and clock floor only need reads. Reserve the single writer
        // only if the refresh actually needs to persist a change.
        match self.refresh_media_record_with(media, now, TransactionBehavior::Deferred) {
            Err(Error::Storage(rusqlite::Error::SqliteFailure(error, _)))
                if error.code == rusqlite::ErrorCode::DatabaseBusy =>
                self.refresh_media_record_with(media, now, TransactionBehavior::Immediate),
            result => result,
        }
    }
    fn refresh_media_record_with(&mut self, media: &mut Media, now: u64, behavior: TransactionBehavior) -> Result<Record, Error> {
        let tx = self
            .db
            .transaction_with_behavior(behavior)?;
        let now = crate::conversations::time_floor(&tx, &self.key, now)?;
        let mut record = load(&tx, &self.key, &media.call)?;
        record.authorize(&tx, &self.key, now)?;
        if record.lease != media.lease {
            return Err(Error::Obsolete);
        }
        let own = record.own_id()?;
        // Readiness for this lease must be in the signed state; a later declaration for the same
        // lease (a track change) may still be on its way without stopping media meanwhile.
        if !record.state.ready.iter().any(|r| {
            r.member == own && r.sequence <= record.ready_sequence && r.challenge == media.lease
        }) {
            return Err(Error::Unprepared);
        }
        let digest = record.state.digest().map_err(failure)?;
        let roster_digest = record.state.roster.roster.digest().map_err(failure)?;
        let current = challenges(&record.state);
        // Keys stay valid only while the members and every receiver challenge are the ones they were
        // issued for: a track toggle re-signs them, a new member or a fresh handle rotates them.
        let same_keys = media.roster == Some(roster_digest) && media.challenges == current;
        let mut replacement = None;
        // The owner's fan-out for this state lacking our key means it never got it.
        let unshared = record.owner_peer.is_some()
            && !record.shares.is_empty()
            && !record.shares.iter().any(|s| s.key.context.sender == own);
        if media.state != Some(digest) || (media.sender.is_none() && !media.external) || unshared {
            record.generation = record.generation.checked_add(1).ok_or(Error::Limit)?;
            // A state that only changed readiness keeps the sender key: the same members already hold it,
            // and frames bind the roster, not the state. A new roster or a missing share starts fresh.
            let reuse = media.share.clone().filter(|_| same_keys && !unshared && (media.sender.is_some() || media.external));
            let (sender, key) = match reuse {
                Some(key) => (None, key),
                None => {
                    let (sender, key) = sigil_calls::Sender::generate(sigil_calls::Context {
                        call: record.id(),
                        roster: roster_digest,
                        sender: own,
                        incarnation: sigil_calls::random_id().map_err(failure)?,
                    })
                    .map_err(failure)?;
                    (Some(sender), key)
                }
            };
            let share = Share::sign(
                &record.state,
                record.generation,
                key.clone(),
                &record.key(&self.key)?,
            )
            .map_err(failure)?;
            record.shares.retain(|s| s.key.context.sender != own);
            record.shares.push(share.clone());
            if let Some(owner) = record.owner_peer {
                jobs::queue(
                    &tx,
                    &self.key,
                    &record,
                    owner,
                    control::Body::Shares(vec![share]),
                    now,
                )?;
            }
            save(&tx, &self.key, &record)?;
            replacement = Some((sender, key));
        }
        let mut incoming = Vec::new();
        let mut bumps = Vec::new();
        for share in &record.shares {
            let sender = share.key.context.sender;
            if sender == own {
                continue;
            }
            share.verify(&record.state).map_err(failure)?;
            match media.receivers.get(&sender) {
                // The same key re-signed for a new state keeps its receiver and its replay window.
                Some((generation, _, context)) if *context == share.key.context => {
                    if *generation < share.generation {
                        bumps.push((sender, share.generation));
                    }
                }
                Some((generation, _, _)) if *generation >= share.generation && media.state == Some(digest) => {}
                _ => incoming.push((
                    sender,
                    share.generation,
                    sigil_calls::Receiver::new(&share.key).map_err(failure)?,
                    share.key.context,
                )),
            }
        }
        tx.commit()?;
        if let Some((sender, key)) = replacement {
            if let Some(sender) = sender {
                media.sender = Some(sender);
                media.external = false;
            }
            media.share = Some(key);
            if media.state != Some(digest) && !same_keys {
                media.receivers.clear();
                media.assembly.clear();
            }
            media.state = Some(digest);
            media.roster = Some(roster_digest);
            media.challenges = current;
        }
        for (id, generation) in bumps {
            if let Some(entry) = media.receivers.get_mut(&id) {
                entry.0 = generation;
            }
        }
        for (id, generation, receiver, context) in incoming {
            media.receivers.insert(id, (generation, receiver, context));
        }
        Ok(record)
    }
    pub fn seal_call_frame(
        &mut self,
        media: &mut Media,
        kind: sigil_calls::MediaKind,
        timestamp: u64,
        keyframe: bool,
        encoded: &[u8],
        now: u64,
    ) -> Result<Vec<u8>, Error> {
        self.media_gate(media, None, kind, now)?;
        let result = media
            .sender
            .as_mut()
            .ok_or(Error::Unprepared)?
            .seal(kind, timestamp, keyframe, encoded);
        match result {
            Err(sigil_calls::Error::Expired) => {
                media.sender = None;
                media.external = false;
                media.checked = None;
                self.refresh_call_media(media, now)?;
                media
                    .sender
                    .as_mut()
                    .ok_or(Error::Unprepared)?
                    .seal(kind, timestamp, keyframe, encoded)
                    .map_err(failure)
            }
            value => value.map_err(failure),
        }
    }
    pub fn open_call_frame(
        &mut self,
        media: &mut Media,
        sender: Id,
        kind: sigil_calls::MediaKind,
        encrypted: &[u8],
        now: u64,
    ) -> Result<sigil_calls::Frame, Error> {
        self.media_gate(media, Some(sender), kind, now)?;
        let frame = media
            .receivers
            .get_mut(&sender)
            .ok_or(Error::Unprepared)?
            .1
            .open(encrypted)
            .map_err(failure)?;
        if frame.kind != kind {
            return Err(Error::InvalidEvent);
        }
        Ok(frame)
    }
    pub fn seal_call_packets(
        &mut self,
        media: &mut Media,
        kind: sigil_calls::MediaKind,
        timestamp: u64,
        keyframe: bool,
        encoded: &[u8],
        now: u64,
    ) -> Result<Vec<Vec<u8>>, Error> {
        let encrypted = self.seal_call_frame(media, kind, timestamp, keyframe, encoded, now)?;
        sigil_calls::packetize(kind, &encrypted).map_err(failure)
    }
    pub fn open_call_packet(
        &mut self,
        media: &mut Media,
        sender: Id,
        kind: sigil_calls::MediaKind,
        packet: &[u8],
        now: u64,
    ) -> Result<Option<sigil_calls::Frame>, Error> {
        self.media_gate(media, Some(sender), kind, now)?;
        self.assemble_call_packet(media, sender, kind, packet, now)
    }
    pub(super) fn assemble_call_packet(
        &mut self,
        media: &mut Media,
        sender: Id,
        kind: sigil_calls::MediaKind,
        packet: &[u8],
        now: u64,
    ) -> Result<Option<sigil_calls::Frame>, Error> {
        // Bounded ciphertext assembly precedes the full authority check on each completed frame.
        let encrypted = media.assemble(sender, kind, packet)?;
        encrypted
            .map(|bytes| self.open_call_frame(media, sender, kind, &bytes, now))
            .transpose()
    }
}
fn authority(record: &Record, media: &Media, now: u64) -> Result<(), Error> {
    record.active(now)?;
    if record.lease != media.lease || Some(record.state.digest().map_err(failure)?) != media.state {
        return Err(Error::Obsolete);
    }
    Ok(())
}
fn track(tracks: Tracks, kind: sigil_calls::MediaKind) -> bool {
    match kind {
        sigil_calls::MediaKind::Audio => tracks.audio,
        sigil_calls::MediaKind::Camera => tracks.camera,
        sigil_calls::MediaKind::Screen => tracks.screen,
    }
}
/// Every member's receiver challenge in a state, in member order.
fn challenges(state: &State) -> Vec<(Id, Id)> {
    state.ready.iter().map(|r| (r.member, r.challenge)).collect()
}

/// A declaration of readiness, plus the sender key signed against the state the
/// owner will commit for it. Sending both together saves a full round trip.
type Declaration = Option<(Id, Option<sigil_calls::Sender>, sigil_calls::KeyShare)>;

pub(super) fn ready(
    tx: &Transaction<'_>,
    key: &StorageKey,
    record: &mut Record,
    tracks: Tracks,
    reuse: Option<sigil_calls::KeyShare>,
    now: u64,
) -> Result<Declaration, Error> {
    record.ready_sequence = record.ready_sequence.checked_add(1).ok_or(Error::Limit)?;
    let ready = Ready::sign(
        &record.state.roster.roster,
        record.ready_sequence,
        record.lease,
        tracks,
        &record.key(key)?,
    )
    .map_err(failure)?;
    let Some(owner) = record.owner_peer else {
        let mut state = record.state.clone();
        state.ready.retain(|r| r.member != ready.member);
        state.ready.push(ready);
        state.ready.sort_by_key(|r| r.member);
        record.change(key, state)?;
        return Ok(None);
    };
    jobs::queue(tx, key, record, owner, control::Body::Ready(ready.clone()), now)?;
    Ok(match anticipated(record, &ready) {
        Some(state) => {
            let declared = declare(tx, key, record, &state, reuse, now)?;
            Some(declared)
        }
        None => None,
    })
}

/// The state the owner commits when it applies our readiness: the digest covers the
/// roster, epoch, participants and ready list, none of which we have to guess.
fn anticipated(record: &Record, ready: &Ready) -> Option<State> {
    let mut state = record.state.clone();
    state.epoch = record.state.epoch.checked_add(1)?;
    state.ready.retain(|r| r.member != ready.member);
    state.ready.push(ready.clone());
    state.ready.sort_by_key(|r| r.member);
    state.digest().ok()?;
    Some(state)
}

/// Sign a sender key against an anticipated state and send it with the declaration.
/// The owner applies its usual checks; a state it does not commit simply drops the
/// share and the ordinary refresh issues another one. A share that overtakes its
/// declaration is dropped too; the refresh notices the owner's fan-out lacks it.
fn declare(
    tx: &Transaction<'_>,
    key: &StorageKey,
    record: &mut Record,
    state: &State,
    reuse: Option<sigil_calls::KeyShare>,
    now: u64,
) -> Result<(Id, Option<sigil_calls::Sender>, sigil_calls::KeyShare), Error> {
    let own = record.own_id()?;
    record.generation = record.generation.checked_add(1).ok_or(Error::Limit)?;
    let (sender, share_key) = match reuse {
        Some(key) => (None, key),
        None => {
            let (sender, key) = sigil_calls::Sender::generate(sigil_calls::Context {
                call: record.id(),
                roster: record.state.roster.roster.digest().map_err(failure)?,
                sender: own,
                incarnation: sigil_calls::random_id().map_err(failure)?,
            })
            .map_err(failure)?;
            (Some(sender), key)
        }
    };
    let share = Share::sign(state, record.generation, share_key.clone(), &record.key(key)?)
        .map_err(failure)?;
    let digest = state.digest().map_err(failure)?;
    record.shares.retain(|s| s.key.context.sender != own);
    record.shares.push(share.clone());
    record.anticipated = Some(digest);
    if let Some(owner) = record.owner_peer {
        jobs::queue(
            tx,
            key,
            record,
            owner,
            control::Body::Shares(vec![share]),
            now,
        )?;
    }
    Ok((digest, sender, share_key))
}
