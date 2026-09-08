use super::*;
use sigil_protocol::{event::Content, text::Text};
#[cfg(test)]
#[path = "rich_text_tests.rs"]
mod tests;

pub struct SigilTextDraft<'a> {
    pub conversation: Id,
    pub message: Id,
    pub source: &'a str,
    pub created_at: u64,
    pub timezone: Option<&'a str>,
    pub date_order: Option<sigil_protocol::text::time::DateOrder>,
}

impl ClientStore {
    /// Commits resolved dates/randomness before returning sendable content.
    pub fn prepare_sigiltext(
        &mut self,
        request: SigilTextDraft<'_>,
    ) -> Result<sigil_protocol::text::Document, Error> {
        self.prepare_sigiltext_with_contacts(request, &[])
    }
    pub fn prepare_sigiltext_with_contacts(
        &mut self,
        request: SigilTextDraft<'_>,
        known: &[sigil_protocol::text::contact::Contact],
    ) -> Result<sigil_protocol::text::Document, Error> {
        use sigil_protocol::text::{composition, Document, Origin};
        let (scope, creator) = crate::structured::account_context(&self.db, &self.key)?;
        if request.conversation == [0; 32]
            || request.message == [0; 32]
            || request.source.len() > 32768
            || request.created_at == 0
            || request.created_at > i64::MAX as u64
            || request.timezone.is_some_and(|value| value.len() > 64)
        {
            return Err(Error::InvalidEvent);
        }
        let index = self.key.commitment(
            &[scope.as_slice(), &request.conversation, &request.message].concat(),
            b"Sigil/structured-draft-index/v1",
        )?;
        let aad = [b"Sigil/structured-draft/v1".as_slice(), &index].concat();
        let input = Zeroizing::new(
            serde_json::to_vec(&(
                request.source,
                request.created_at,
                request.timezone,
                request.date_order,
            ))
            .map_err(|_| Error::InvalidEvent)?,
        );
        let fingerprint = self
            .key
            .commitment(&input, b"Sigil/structured-draft-input/v1")?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let prior: Option<Vec<u8>> = tx.query_row("SELECT CASE WHEN length(content)<=61508 THEN content END FROM structured_drafts WHERE id=?1", [index.as_slice()], |r| r.get(0)).optional()?;
        let document = if let Some(bytes) = prior {
            let bytes = self.key.open(&bytes, &aad)?;
            if bytes.len() < 33 {
                return Err(Error::InvalidStore);
            }
            if bytes[..32] != fingerprint {
                return Err(Error::Conflict);
            }
            Document::from_bytes(&bytes[32..]).map_err(|_| Error::InvalidStore)?
        } else {
            let count: i64 =
                tx.query_row("SELECT count(*) FROM structured_drafts", [], |r| r.get(0))?;
            if count >= 256 {
                return Err(Error::Limit);
            }
            let document = composition::parse_with_contacts(
                request.source,
                Origin {
                    message: request.message,
                    creator,
                    created_at: request.created_at,
                    timezone: request.timezone,
                },
                Default::default(),
                request.date_order,
                known,
            )
            .map_err(|_| Error::InvalidEvent)?
            .content;
            let mut bytes = Zeroizing::new(fingerprint.to_vec());
            bytes.extend_from_slice(&document.to_bytes().map_err(|_| Error::InvalidEvent)?);
            tx.execute(
                "INSERT INTO structured_drafts VALUES(?1,?2)",
                (index.as_slice(), self.key.seal(&bytes, &aad)?),
            )?;
            document
        };
        document
            .authorize_origin(&request.message, &creator, request.created_at)
            .map_err(|_| Error::InvalidStore)?;
        tx.commit()?;
        Ok(document)
    }
    /// Discard only after durable queuing or abandonment; a new draft needs a new message ID.
    pub fn discard_sigiltext_draft(&mut self, conversation: Id, message: Id) -> Result<(), Error> {
        let (scope, _) = crate::structured::account_context(&self.db, &self.key)?;
        let index = self.key.commitment(
            &[scope.as_slice(), &conversation, &message].concat(),
            b"Sigil/structured-draft-index/v1",
        )?;
        self.db.execute(
            "DELETE FROM structured_drafts WHERE id=?1",
            [index.as_slice()],
        )?;
        Ok(())
    }
    pub fn queue_peer_composition(
        &mut self,
        peer: Id,
        value: &sigil_protocol::text::composition::Composition,
        now: u64,
    ) -> Result<(), Error> {
        let bytes = Zeroizing::new(value.to_bytes().map_err(|_| Error::InvalidEvent)?);
        self.queue_peer_content(peer, value.id, Content::Rich(&bytes), value.created_at, now)
    }
    pub fn queue_group_composition(
        &mut self,
        group: Id,
        value: &sigil_protocol::text::composition::Composition,
        now: u64,
    ) -> Result<(), Error> {
        let bytes = Zeroizing::new(value.to_bytes().map_err(|_| Error::InvalidEvent)?);
        self.queue_group_content(
            group,
            value.id,
            Content::Rich(&bytes),
            value.created_at,
            now,
        )
    }
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
