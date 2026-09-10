use super::*;
#[cfg(test)]
#[path = "media_tests.rs"]
mod tests;
use std::collections::BTreeMap;
pub struct Media {
    pub(super) call: Id,
    pub(super) lease: Id,
    state: Option<Id>,
    sender: Option<sigil_calls::Sender>,
    receivers: BTreeMap<Id, (u64, sigil_calls::Receiver)>,
    assembly: sigil_calls::Assembly,
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
        record.lease = sigil_calls::random_id().map_err(failure)?;
        record.shares.clear();
        ready(&tx, &self.key, &mut record, tracks, now)?;
        let media = Media {
            call: id,
            lease: record.lease,
            state: None,
            sender: None,
            receivers: BTreeMap::new(),
            assembly: sigil_calls::Assembly::default(),
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
        ready(&tx, &self.key, &mut record, tracks, now)?;
        save(&tx, &self.key, &record)?;
        tx.commit()?;
        media.state = None;
        media.sender = None;
        media.receivers.clear();
        media.assembly.clear();
        Ok(())
    }
    pub fn refresh_call_media(&mut self, media: &mut Media, now: u64) -> Result<usize, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = crate::conversations::time_floor(&tx, &self.key, now)?;
        let mut record = load(&tx, &self.key, &media.call)?;
        record.authorize(&tx, &self.key, now)?;
        if record.lease != media.lease {
            return Err(Error::Obsolete);
        }
        let own = record.own_id()?;
        if !record.state.ready.iter().any(|r| {
            r.member == own && r.sequence == record.ready_sequence && r.challenge == media.lease
        }) {
            return Err(Error::Unprepared);
        }
        let digest = record.state.digest().map_err(failure)?;
        let mut replacement = None;
        if media.state != Some(digest) || media.sender.is_none() {
            record.generation = record.generation.checked_add(1).ok_or(Error::Limit)?;
            let (sender, key) = sigil_calls::Sender::generate(sigil_calls::Context {
                call: record.id(),
                roster: record.state.roster.roster.digest().map_err(failure)?,
                sender: own,
                incarnation: sigil_calls::random_id().map_err(failure)?,
            })
            .map_err(failure)?;
            let share = Share::sign(
                &record.state,
                record.generation,
                key,
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
            replacement = Some(sender);
        }
        let mut incoming = Vec::new();
        for share in &record.shares {
            let sender = share.key.context.sender;
            if sender == own {
                continue;
            }
            share.verify(&record.state).map_err(failure)?;
            if media.state != Some(digest)
                || media
                    .receivers
                    .get(&sender)
                    .is_none_or(|(generation, _)| *generation < share.generation)
            {
                incoming.push((
                    sender,
                    share.generation,
                    sigil_calls::Receiver::new(&share.key).map_err(failure)?,
                ));
            }
        }
        tx.commit()?;
        if let Some(sender) = replacement {
            media.sender = Some(sender);
            if media.state != Some(digest) {
                media.receivers.clear();
                media.assembly.clear();
            }
            media.state = Some(digest);
        }
        for (id, generation, receiver) in incoming {
            media.receivers.insert(id, (generation, receiver));
        }
        Ok(media.receivers.len())
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
        self.refresh_call_media(media, now)?;
        let record = load(&self.db, &self.key, &media.call)?;
        authority(&record, media, now)?;
        if !enabled(&record, record.own_id()?, kind) {
            return Err(Error::Unprepared);
        }
        let result = media
            .sender
            .as_mut()
            .ok_or(Error::Unprepared)?
            .seal(kind, timestamp, keyframe, encoded);
        match result {
            Err(sigil_calls::Error::Expired) => {
                media.sender = None;
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
        self.refresh_call_media(media, now)?;
        let record = load(&self.db, &self.key, &media.call)?;
        authority(&record, media, now)?;
        if !enabled(&record, sender, kind) {
            return Err(Error::Unprepared);
        }
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
        self.refresh_call_media(media, now)?;
        let record = load(&self.db, &self.key, &media.call)?;
        authority(&record, media, now)?;
        if !enabled(&record, sender, kind) {
            return Err(Error::Unprepared);
        }
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
        let encrypted = media
            .assembly
            .push(sender, kind, packet, std::time::Instant::now())
            .map_err(failure)?;
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
fn enabled(record: &Record, sender: Id, kind: sigil_calls::MediaKind) -> bool {
    record
        .state
        .ready
        .iter()
        .find(|r| r.member == sender)
        .is_some_and(|r| match kind {
            sigil_calls::MediaKind::Audio => r.tracks.audio,
            sigil_calls::MediaKind::Camera => r.tracks.camera,
            sigil_calls::MediaKind::Screen => r.tracks.screen,
        })
}
fn ready(
    tx: &Transaction<'_>,
    key: &StorageKey,
    record: &mut Record,
    tracks: Tracks,
    now: u64,
) -> Result<(), Error> {
    record.ready_sequence = record.ready_sequence.checked_add(1).ok_or(Error::Limit)?;
    let ready = Ready::sign(
        &record.state.roster.roster,
        record.ready_sequence,
        record.lease,
        tracks,
        &record.key(key)?,
    )
    .map_err(failure)?;
    if let Some(owner) = record.owner_peer {
        jobs::queue(tx, key, record, owner, control::Body::Ready(ready), now)?;
    } else {
        let mut state = record.state.clone();
        state.ready.retain(|r| r.member != ready.member);
        state.ready.push(ready);
        state.ready.sort_by_key(|r| r.member);
        record.change(key, state)?;
    }
    Ok(())
}
