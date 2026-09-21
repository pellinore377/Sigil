use super::*;
use sigil_calls::{Context, Frame, KeyShare, MediaKind, Receiver, Sender, SenderHandoff};

/// Ephemeral, private worker message. Never persist or log this value.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaUpdate {
    pub call: Id,
    lease: Id,
    own: Id,
    expires: u64,
    tracks: Vec<(Id, Tracks)>,
    sender_context: Context,
    sender: Option<SenderHandoff>,
    receiver_contexts: Vec<Context>,
    receivers: Vec<KeyShare>,
}
impl ClientStore {
    /// Only an exclusive store owner with a synchronous invalidation channel may detach media.
    /// Every update is gated by that channel's revision before and after processing each frame.
    pub fn detach_call_media(&mut self, media: &mut Media, now: u64) -> Result<MediaUpdate, Error> {
        self.refresh_call_media(media, now)?;
        let record = load(&self.db, &self.key, &media.call)?;
        let checked = media.checked.as_ref().ok_or(Error::Unprepared)?;
        let sender_context = media.share.as_ref().ok_or(Error::Unprepared)?.context;
        let sender = media
            .sender
            .take()
            .map(Sender::into_handoff)
            .transpose()
            .map_err(failure)?;
        media.external = true;
        Ok(MediaUpdate {
            call: media.call,
            lease: media.lease,
            own: checked.own,
            expires: checked.expires,
            tracks: checked.tracks.clone(),
            sender_context,
            sender,
            receiver_contexts: media
                .receivers
                .values()
                .map(|(_, _, context)| *context)
                .collect(),
            receivers: record
                .shares
                .iter()
                .filter(|share| {
                    media
                        .receivers
                        .get(&share.key.context.sender)
                        .is_some_and(|(_, _, context)| *context == share.key.context)
                })
                .map(|share| share.key.clone())
                .collect(),
        })
    }
    pub fn renew_detached_media(
        &mut self,
        media: &mut Media,
        context: Context,
        now: u64,
    ) -> Result<(), Error> {
        if media.external
            && media
                .share
                .as_ref()
                .is_some_and(|share| share.context == context)
        {
            media.external = false;
            media.checked = None;
            self.refresh_call_media(media, now)?;
        }
        Ok(())
    }
}

/// No storage, networking, or codec work: a single owner retains counters and replay windows.
#[derive(Default)]
pub struct MediaProcessor {
    update: Option<MediaUpdate>,
    sender: Option<Sender>,
    receivers: BTreeMap<Id, (Context, Receiver)>,
}
impl MediaProcessor {
    pub fn apply(&mut self, mut update: MediaUpdate) -> Result<(), Error> {
        if self
            .update
            .as_ref()
            .is_some_and(|old| old.call != update.call || old.lease != update.lease)
        {
            *self = Self::default();
        }
        if let Some(sender) = update.sender.take() {
            self.sender = Some(sender.restore().map_err(failure)?);
        } else if self
            .update
            .as_ref()
            .is_none_or(|old| old.sender_context != update.sender_context)
        {
            return Err(Error::Unprepared);
        }
        self.receivers.retain(|id, (context, _)| {
            update
                .receiver_contexts
                .iter()
                .any(|key| key.sender == *id && *key == *context)
        });
        for key in &update.receivers {
            if update.receiver_contexts.contains(&key.context)
                && !self.receivers.contains_key(&key.context.sender)
            {
                self.receivers.insert(
                    key.context.sender,
                    (key.context, Receiver::new(key).map_err(failure)?),
                );
            }
        }
        self.update = Some(update);
        Ok(())
    }
    pub fn is_live(&self, now: u64) -> bool {
        self.update.as_ref().is_some_and(|u| now < u.expires)
    }
    pub fn expires_at(&self) -> Option<u64> {
        self.update.as_ref().map(|u| u.expires)
    }
    pub fn context(&self) -> Option<Context> {
        self.update.as_ref().map(|v| v.sender_context)
    }
    fn gate(&self, call: Id, sender: Option<Id>, kind: MediaKind, now: u64) -> Result<(), Error> {
        let update = self.update.as_ref().ok_or(Error::Unprepared)?;
        if update.call != call {
            return Err(Error::Obsolete);
        }
        if now >= update.expires {
            return Err(Error::Expired);
        }
        let member = sender.unwrap_or(update.own);
        if !update
            .tracks
            .iter()
            .any(|(id, tracks)| *id == member && track(*tracks, kind))
        {
            return Err(Error::Unprepared);
        }
        Ok(())
    }
    pub fn seal(
        &mut self,
        call: Id,
        kind: MediaKind,
        timestamp: u64,
        keyframe: bool,
        bytes: &[u8],
        now: u64,
    ) -> Result<Vec<u8>, Error> {
        self.gate(call, None, kind, now)?;
        self.sender
            .as_mut()
            .ok_or(Error::Unprepared)?
            .seal(kind, timestamp, keyframe, bytes)
            .map_err(failure)
    }
    pub fn open(
        &mut self,
        call: Id,
        sender: Id,
        kind: MediaKind,
        bytes: &[u8],
        now: u64,
    ) -> Result<Frame, Error> {
        self.gate(call, Some(sender), kind, now)?;
        let frame = self
            .receivers
            .get_mut(&sender)
            .ok_or(Error::Unprepared)?
            .1
            .open(bytes)
            .map_err(failure)?;
        if frame.kind != kind {
            return Err(Error::InvalidEvent);
        }
        Ok(frame)
    }
}
