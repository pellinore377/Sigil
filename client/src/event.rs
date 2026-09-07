//! Direct-text application binding, separate from opaque cryptographic APIs.
use super::*;
use sigil_protocol::{
    device::{Binding, SignedBinding},
    event::{Content, Direct, Text},
};
#[path = "send_intents.rs"]
mod intents;
#[cfg(test)]
#[path = "text_recovery_tests.rs"]
mod recovery_tests;
#[cfg(test)]
#[path = "event_tests.rs"]
mod tests;
pub use intents::SendIntentAttempt;

struct Context {
    own: Id,
    own_identity: Id,
    own_account: Id,
    own_reference: Id,
    recovery_scope: Id,
    peer: Peer,
    conversation: Id,
}
pub(crate) fn account(binding: &Binding) -> Id {
    account_reference(&binding.server, &binding.account)
}
pub(crate) fn account_reference(server: &str, account: &Id) -> Id {
    Sha256::digest(
        [
            b"Sigil/account-reference/v0".as_slice(),
            &(server.len() as u16).to_be_bytes(),
            server.as_bytes(),
            account,
        ]
        .concat(),
    )
    .into()
}
fn context(db: &Connection, key: &StorageKey, own: &[u8], peer: &Id) -> Result<Context, Error> {
    let own_fingerprint = device_fingerprint(own)?;
    let own = SignedBinding::from_bytes(own)
        .map_err(|_| Error::InvalidStore)?
        .binding;
    let peer = peers::known(db, key, peer)?;
    if own.server != peer.binding.server {
        return Err(Error::Unprepared);
    }
    let mut accounts = [account(&own), account(&peer.binding)];
    accounts.sort();
    let conversation = Sha256::digest(
        [
            b"Sigil/direct-conversation/v0".as_slice(),
            &accounts[0],
            &accounts[1],
        ]
        .concat(),
    )
    .into();
    Ok(Context {
        own: own_fingerprint,
        own_identity: own.identity,
        own_account: own.account,
        own_reference: account(&own),
        recovery_scope: recovery::account_scope(&own.server, own.account)?,
        peer,
        conversation,
    })
}
impl Context {
    fn encode(
        &self,
        message: Id,
        body: Content<'_>,
        timestamp: u64,
    ) -> Result<Zeroizing<Vec<u8>>, Error> {
        if !self.peer.verified {
            return Err(Error::Unprepared);
        }
        validate_content(
            body,
            &self.peer.binding.server,
            &message,
            &self.own_reference,
            timestamp,
        )?;
        Ok(Zeroizing::new(
            Direct {
                message,
                conversation: self.conversation,
                sender: self.own,
                recipient: self.peer.fingerprint,
                timestamp,
                content: body,
            }
            .to_bytes()
            .map_err(|_| Error::InvalidEvent)?,
        ))
    }
}
pub(super) fn validate_content(
    content: Content<'_>,
    server: &str,
    message: &Id,
    creator: &Id,
    timestamp: u64,
) -> Result<(), Error> {
    if let Content::File(bytes) = content {
        let file =
            sigil_protocol::file::File::from_bytes(bytes).map_err(|_| Error::InvalidEvent)?;
        if file.source != server {
            return Err(Error::InvalidEvent);
        }
    }
    if let Content::Rich(bytes) = content {
        match sigil_protocol::text::Document::from_bytes(bytes).map_err(|_| Error::InvalidEvent)? {
            sigil_protocol::text::Document::Card(card) => card
                .authorize_origin(message, creator, timestamp)
                .map_err(|_| Error::InvalidEvent)?,
            sigil_protocol::text::Document::Action(action) => action
                .authorize_origin(message, creator, timestamp)
                .map_err(|_| Error::InvalidEvent)?,
            sigil_protocol::text::Document::Text(_) => {}
        }
    }
    Ok(())
}
pub(super) fn validate(
    db: &Connection,
    key: &StorageKey,
    own: &[u8],
    peer: &Id,
    message: &Id,
    plaintext: &[u8],
    expires: u64,
) -> Result<(), Error> {
    if groups::validate_distribution_receipt(db, key, own, peer, message, plaintext)? {
        return Ok(());
    }
    let context = context(db, key, own, peer)?;
    let text = Direct::from_bytes(plaintext).map_err(|_| Error::InvalidEvent)?;
    validate_content(
        text.content,
        &context.peer.binding.server,
        &text.message,
        &account(&context.peer.binding),
        text.timestamp,
    )?;
    if text.conversation != context.conversation
        || text.sender != context.peer.fingerprint
        || text.recipient != context.own
    {
        return Err(Error::InvalidEvent);
    }
    if text.message != *message {
        retry::response_allowed(
            db,
            key,
            &context.own,
            &context.peer,
            *message,
            &text,
            expires,
        )?;
        require_resend(db, key, own, peer, &text, false)?;
    }
    Ok(())
}
impl Incoming {
    pub fn event(&self) -> Result<Direct<'_>, Error> {
        Direct::from_bytes(&self.plaintext).map_err(|_| Error::InvalidEvent)
    }
    pub fn text(&self) -> Result<Text<'_>, Error> {
        Text::from_bytes(&self.plaintext).map_err(|_| Error::InvalidEvent)
    }
}
impl ClientStore {
    pub fn direct_conversation(&mut self, peer: Id) -> Result<Id, Error> {
        let own = self.own_device_binding()?;
        Ok(context(&self.db, &self.key, &own, &peer)?.conversation)
    }
    /// Starts a peer-aware direct-text session. Persist the original timestamp
    /// with the request ID: changing either content or timestamp is not a retry.
    pub fn start_claimed_text(
        &mut self,
        claim: Id,
        session: Id,
        message: Id,
        body: &str,
        timestamp: u64,
        now: u64,
    ) -> Result<Vec<u8>, Error> {
        let own = self.own_device_binding()?;
        let identity = SignedBinding::from_bytes(&own)
            .map_err(|_| Error::InvalidStore)?
            .binding
            .identity;
        let peer = claims::peer(&self.db, &self.key, &claim, &identity)?;
        let plaintext = context(&self.db, &self.key, &own, &peer)?.encode(
            message,
            Content::Text(body),
            timestamp,
        )?;
        // The shared claimed-handshake transaction rechecks the frozen identity
        // and current trust status before committing any session/delivery state.
        self.start_claimed_content(claim, session, message, (&plaintext, Some(&own)), now)
    }
    /// Encrypts a direct-text event and freezes its recipient/delivery retry in
    /// the same transaction. Call transport only after this method succeeds.
    pub fn send_text(
        &mut self,
        session: Id,
        message: Id,
        body: &str,
        timestamp: u64,
        now: u64,
    ) -> Result<Vec<u8>, Error> {
        let own = self.own_device_binding()?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let packet = send_content_in(
            &tx,
            &self.key,
            &own,
            session,
            message,
            (Content::Text(body), timestamp),
            now,
        )?;
        tx.commit()?;
        Ok(packet)
    }

    /// Select for new text, repairing expired initials with a confirmed alternative.
    /// Exact retries remain bound to their original session.
    pub fn send_peer_text(
        &mut self,
        peer: Id,
        message: Id,
        body: &str,
        timestamp: u64,
        now: u64,
    ) -> Result<(Id, Vec<u8>), Error> {
        self.send_peer_content(peer, message, Content::Text(body), timestamp, now)
    }
    pub(super) fn send_peer_content(
        &mut self,
        peer: Id,
        message: Id,
        body: Content<'_>,
        timestamp: u64,
        now: u64,
    ) -> Result<(Id, Vec<u8>), Error> {
        let own = self.own_device_binding()?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let prior: Option<Vec<u8>> = tx
            .query_row(
                "SELECT session FROM deliveries WHERE id=?1 AND length(session)=32",
                [message.as_slice()],
                |r| r.get(0),
            )
            .optional()?;
        let session = match prior {
            Some(session) => session.try_into().map_err(|_| Error::InvalidStore)?,
            None => selection::for_send(&tx, &self.key, &peer, now)?,
        };
        if session_peer(&tx, &session)? != Some(peer) {
            return Err(Error::Conflict);
        }
        let packet = send_content_in(
            &tx,
            &self.key,
            &own,
            session,
            message,
            (body, timestamp),
            now,
        )?;
        tx.commit()?;
        Ok((session, packet))
    }
}

fn send_content_in(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &[u8],
    session: Id,
    message: Id,
    (body, timestamp): (Content<'_>, u64),
    now: u64,
) -> Result<Vec<u8>, Error> {
    let peer = session_peer(tx, &session)?.ok_or(Error::Unprepared)?;
    let context = context(tx, key, own, &peer)?;
    let plaintext = context.encode(message, body, timestamp)?;
    let packet = send_in(tx, key, session, message, &plaintext)?;
    transport::prepare(
        tx,
        key,
        session,
        message,
        context.peer.binding.device,
        None,
        now,
    )?;
    retain(tx, key, own, &peer, &plaintext, true)?;
    Ok(packet)
}

/// Separate from the per-packet retry journal: a logical event cannot be
/// silently redefined on another session. No plaintext digest is stored clear.
pub(super) fn remember(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &[u8],
    peer: &Id,
    session: &Id,
    plaintext: &[u8],
) -> Result<bool, Error> {
    if let Some(receipt) = groups::distribution_receipt(plaintext)? {
        groups::validate_distribution_receipt(tx, key, own, peer, &receipt.message, plaintext)?;
        return Ok(true);
    }
    let context = context(tx, key, own, peer)?;
    let own = SignedBinding::from_bytes(own)
        .map_err(|_| Error::InvalidStore)?
        .binding
        .device;
    let text = Direct::from_bytes(plaintext).map_err(|_| Error::InvalidEvent)?;
    validate_content(
        text.content,
        &context.peer.binding.server,
        &text.message,
        &account(&context.peer.binding),
        text.timestamp,
    )?;
    let id: Id = Sha256::digest(
        [
            b"Sigil/direct-text-retry/v0".as_slice(),
            &own,
            peer,
            &text.message,
        ]
        .concat(),
    )
    .into();
    let aad = binding(18, &id, b"text event");
    let digest: Id = Sha256::digest(plaintext).into();
    let expected = Zeroizing::new([session.as_slice(), &digest].concat());
    let prior: Option<Vec<u8>> = tx
        .query_row(
            "SELECT CASE WHEN length(state)=100 THEN state END FROM text_events WHERE id=?1",
            [id.as_slice()],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(prior) = prior {
        let prior = key.open(&prior, &aad)?;
        if prior.len() != 64
            || prior[32..] != expected[32..]
            || (prior[..32] != expected[..32]
                && !retry::requested(tx, key, &context.own, &context.peer, text.message)?)
        {
            return Err(Error::Conflict);
        }
        return Ok(true);
    }
    tx.execute(
        "INSERT INTO text_events VALUES(?1,?2)",
        (id.as_slice(), key.seal(&expected, &aad)?),
    )?;
    Ok(false)
}

pub(super) fn require_resend(
    db: &Connection,
    key: &StorageKey,
    own: &[u8],
    peer: &Id,
    text: &Direct<'_>,
    outgoing: bool,
) -> Result<(), Error> {
    let context = context(db, key, own, peer)?;
    let author = if outgoing {
        context.own_identity
    } else {
        context.peer.binding.identity
    };
    recovery::require_resend(
        db,
        key,
        context.recovery_scope,
        event_history_id(text, &author),
        text.content,
    )
}

pub(super) fn retain(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &[u8],
    peer: &Id,
    plaintext: &[u8],
    outgoing: bool,
) -> Result<(), Error> {
    if let Some(receipt) = groups::distribution_receipt(plaintext)? {
        if outgoing {
            return Err(Error::InvalidEvent);
        }
        groups::validate_distribution_receipt(tx, key, own, peer, &receipt.message, plaintext)?;
        return Ok(());
    }
    let context = context(tx, key, own, peer)?;
    let text = Direct::from_bytes(plaintext).map_err(|_| Error::InvalidEvent)?;
    validate_content(
        text.content,
        &context.peer.binding.server,
        &text.message,
        &if outgoing {
            context.own_reference
        } else {
            account(&context.peer.binding)
        },
        text.timestamp,
    )?;
    let author = if outgoing {
        context.own_identity
    } else {
        context.peer.binding.identity
    };
    let id = event_history_id(&text, &author);
    if let Content::Rich(bytes) = text.content {
        if outgoing {
            if let sigil_protocol::text::Document::Action(action) =
                sigil_protocol::text::Document::from_bytes(bytes)
                    .map_err(|_| Error::InvalidEvent)?
            {
                crate::structured::require_outgoing(
                    tx,
                    key,
                    context.recovery_scope,
                    text.conversation,
                    &action,
                )?;
            }
        }
        crate::structured::ingest(
            tx,
            key,
            context.recovery_scope,
            text.conversation,
            id,
            bytes,
        )?;
    }
    if !recovery::configured(tx)? {
        return Ok(());
    }
    if outgoing && matches!(text.content, Content::Rich(_)) {
        recovery::require_resend(tx, key, context.recovery_scope, id, text.content)?;
    }
    // Direction is relative to the owning account, so copies received on another
    // own device agree with the original sender's recovery record.
    let direction = if outgoing || context.own_account == context.peer.binding.account {
        sigil_crypto::recovery::Direction::Outgoing
    } else {
        sigil_crypto::recovery::Direction::Incoming
    };
    recovery::retain_new(
        tx,
        key,
        context.recovery_scope,
        &sigil_crypto::recovery::Record {
            id,
            revision: 1,
            conversation: text.conversation,
            author,
            created_at: text.timestamp,
            direction,
            content: match text.content {
                Content::Text(body) => sigil_crypto::recovery::Content::Retained(Zeroizing::new(
                    body.as_bytes().to_vec(),
                )),
                Content::File(bytes) => {
                    sigil_crypto::recovery::Content::File(Zeroizing::new(bytes.to_vec()))
                }
                Content::Rich(bytes) => {
                    sigil_crypto::recovery::Content::Rich(Zeroizing::new(bytes.to_vec()))
                }
            },
        },
    )
}
/// Stable across local sessions and recipient-device fan-out. The author is the
/// authenticated encryption identity, not a server-supplied display name.
pub fn text_history_id(text: &Text<'_>, author: &Id) -> Id {
    event_history_id(
        &Direct {
            message: text.message,
            conversation: text.conversation,
            sender: text.sender,
            recipient: text.recipient,
            timestamp: text.timestamp,
            content: Content::Text(text.body),
        },
        author,
    )
}
pub fn event_history_id(event: &Direct<'_>, author: &Id) -> Id {
    let domain: &[u8] = match event.content {
        Content::Text(_) => b"Sigil/direct-text-history/v0",
        Content::File(_) => b"Sigil/direct-file-history/v0",
        Content::Rich(_) => b"Sigil/direct-sigiltext-history/v0",
    };
    Sha256::digest([domain, &event.conversation, author, &event.message].concat()).into()
}

pub(crate) fn migrate_structured(tx: &Transaction<'_>, key: &StorageKey) -> Result<(), Error> {
    if !tx.query_row("SELECT EXISTS(SELECT 1 FROM own_device_binding)", [], |r| {
        r.get::<_, bool>(0)
    })? {
        return Ok(());
    }
    let own = peers::own(tx, key)?;
    for outgoing in [false, true] {
        let sql = if outgoing {
            "SELECT o.session,o.id,o.content,s.peer FROM outbox o JOIN sessions s ON s.id=o.session WHERE o.content IS NOT NULL AND s.peer IS NOT NULL"
        } else {
            "SELECT o.session,o.id,o.content,s.peer FROM inbox o JOIN sessions s ON s.id=o.session WHERE s.peer IS NOT NULL"
        };
        let mut statement = tx.prepare(sql)?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let session: Vec<u8> = row.get(0)?;
            let id: Vec<u8> = row.get(1)?;
            let bytes: Vec<u8> = row.get(2)?;
            let peer: Vec<u8> = row.get(3)?;
            let session: Id = session.try_into().map_err(|_| Error::InvalidStore)?;
            let peer: Id = peer.try_into().map_err(|_| Error::InvalidStore)?;
            let plaintext = key.open(
                &bytes,
                &binding(if outgoing { 9 } else { 2 }, &session, &id),
            )?;
            // Raw cryptographic APIs historically allowed arbitrary plaintext.
            let Ok(event) = Direct::from_bytes(&plaintext) else {
                continue;
            };
            let Content::Rich(bytes) = event.content else {
                continue;
            };
            let context = context(tx, key, &own, &peer)?;
            let (sender, recipient, creator, author) = if outgoing {
                (
                    context.own,
                    context.peer.fingerprint,
                    context.own_reference,
                    context.own_identity,
                )
            } else {
                (
                    context.peer.fingerprint,
                    context.own,
                    account(&context.peer.binding),
                    context.peer.binding.identity,
                )
            };
            if event.message.as_slice() != id
                || event.sender != sender
                || event.recipient != recipient
                || event.conversation != context.conversation
                || validate_content(
                    event.content,
                    &context.peer.binding.server,
                    &event.message,
                    &creator,
                    event.timestamp,
                )
                .is_err()
            {
                continue;
            }
            crate::structured::ingest(
                tx,
                key,
                context.recovery_scope,
                event.conversation,
                event_history_id(&event, &author),
                bytes,
            )?;
        }
    }
    Ok(())
}
