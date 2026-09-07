use super::*;
use sigil_protocol::{event::Content, text::Text};
#[cfg(test)]
#[path = "rich_text_tests.rs"]
mod tests;

impl ClientStore {
    pub fn queue_peer_action(
        &mut self,
        peer: Id,
        action: &sigil_protocol::text::action::Action,
        now: u64,
    ) -> Result<(), Error> {
        let conversation = self.direct_conversation(peer)?;
        self.require_action(conversation, action)?;
        let bytes = Zeroizing::new(action.to_bytes().map_err(|_| Error::InvalidEvent)?);
        self.queue_peer_content(
            peer,
            action.id().map_err(|_| Error::InvalidEvent)?,
            Content::Rich(&bytes),
            action.created_at,
            now,
        )
    }
    pub fn queue_group_action(
        &mut self,
        group: Id,
        action: &sigil_protocol::text::action::Action,
        now: u64,
    ) -> Result<(), Error> {
        self.require_action(group, action)?;
        let bytes = Zeroizing::new(action.to_bytes().map_err(|_| Error::InvalidEvent)?);
        self.queue_group_content(
            group,
            action.id().map_err(|_| Error::InvalidEvent)?,
            Content::Rich(&bytes),
            action.created_at,
            now,
        )
    }
    /// Stable across this account's devices, scoped to its authenticated server.
    pub fn account_reference(&self) -> Result<Id, Error> {
        Ok(crate::structured::account_context(&self.db, &self.key)?.1)
    }
    pub fn queue_peer_card(
        &mut self,
        peer: Id,
        card: &sigil_protocol::text::structured::Card,
        now: u64,
    ) -> Result<(), Error> {
        let bytes = Zeroizing::new(card.to_bytes().map_err(|_| Error::InvalidEvent)?);
        self.queue_peer_content(peer, card.id, Content::Rich(&bytes), card.created_at, now)
    }
    pub fn queue_group_card(
        &mut self,
        group: Id,
        card: &sigil_protocol::text::structured::Card,
        now: u64,
    ) -> Result<(), Error> {
        let bytes = Zeroizing::new(card.to_bytes().map_err(|_| Error::InvalidEvent)?);
        self.queue_group_content(group, card.id, Content::Rich(&bytes), card.created_at, now)
    }
    pub fn queue_peer_sigiltext(
        &mut self,
        peer: Id,
        message: Id,
        text: &Text,
        timestamp: u64,
        now: u64,
    ) -> Result<(), Error> {
        let bytes = Zeroizing::new(text.to_bytes().map_err(|_| Error::InvalidEvent)?);
        self.queue_peer_content(peer, message, Content::Rich(&bytes), timestamp, now)
    }
    pub fn send_peer_sigiltext(
        &mut self,
        peer: Id,
        message: Id,
        text: &Text,
        timestamp: u64,
        now: u64,
    ) -> Result<(Id, Vec<u8>), Error> {
        let bytes = Zeroizing::new(text.to_bytes().map_err(|_| Error::InvalidEvent)?);
        self.send_peer_content(peer, message, Content::Rich(&bytes), timestamp, now)
    }
    pub fn queue_group_sigiltext(
        &mut self,
        group: Id,
        message: Id,
        text: &Text,
        timestamp: u64,
        now: u64,
    ) -> Result<(), Error> {
        let bytes = Zeroizing::new(text.to_bytes().map_err(|_| Error::InvalidEvent)?);
        self.queue_group_content(group, message, Content::Rich(&bytes), timestamp, now)
    }
}
