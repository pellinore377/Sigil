//! Durable group ciphertext and logical history, independent of pairwise sessions.
use super::control::{context_bytes, context_from_bytes};
use super::*;
use crate::{recovery, ClientStore};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use sigil_crypto::{sender_keys as sk, storage::StorageKey};
use sigil_protocol::event::{Content, Group, GroupText};
use zeroize::Zeroizing;
#[path = "group_delivery.rs"]
mod delivery;
pub use delivery::{GroupDeliveryStatus, GroupSendAttempt};

pub(crate) const MIGRATION: &str = "
CREATE TABLE group_messages(id BLOB PRIMARY KEY,group_id BLOB NOT NULL REFERENCES groups(id),content BLOB NOT NULL,packet BLOB,outgoing INTEGER NOT NULL CHECK(outgoing IN (0,1)));
CREATE INDEX group_messages_pending ON group_messages(group_id) WHERE packet IS NOT NULL;
CREATE TABLE group_delivery(sequence INTEGER PRIMARY KEY AUTOINCREMENT,id BLOB UNIQUE NOT NULL,message BLOB NOT NULL REFERENCES group_messages(id),recipient BLOB NOT NULL,state BLOB NOT NULL,status INTEGER NOT NULL CHECK(status BETWEEN 0 AND 3));
CREATE INDEX group_delivery_pending ON group_delivery(recipient,sequence) WHERE status=0;
CREATE INDEX group_delivery_message ON group_delivery(message,sequence);
CREATE TABLE group_incoming(sequence INTEGER PRIMARY KEY CHECK(sequence>0),acknowledged INTEGER NOT NULL CHECK(acknowledged IN (0,1)),state BLOB NOT NULL);
CREATE INDEX group_incoming_pending ON group_incoming(sequence) WHERE acknowledged=0;
CREATE TABLE group_delivery_cursor(id INTEGER PRIMARY KEY CHECK(id=1),state BLOB NOT NULL);
PRAGMA user_version=45;
";
const MAX_PENDING: u32 = 16;

pub struct GroupMessage {
    pub context: sk::Context,
    pub plaintext: Zeroizing<Vec<u8>>,
    pub outgoing: bool,
    pub duplicate: bool,
}
impl GroupMessage {
    pub fn event(&self) -> Result<Group<'_>, Error> {
        Group::from_bytes(&self.plaintext).map_err(|_| Error::InvalidEvent)
    }
    pub fn text(&self) -> Result<GroupText<'_>, Error> {
        GroupText::from_bytes(&self.plaintext).map_err(|_| Error::InvalidEvent)
    }
}
fn aad(kind: u8, own: &Id, id: &Id) -> Vec<u8> {
    crate::binding(kind, own, id)
}
fn index(key: &StorageKey, group: &Id, author: &Id, message: &Id) -> Result<Id, Error> {
    Ok(key.commitment(
        &[group.as_slice(), author, message].concat(),
        b"Sigil/group-message-index/v0",
    )?)
}
fn transport_id(context: &sk::Context, message: &Id, recipient: &Id) -> Id {
    digest(
        b"Sigil/group-message-delivery/v0",
        &[&context_bytes(context), message, recipient],
    )
}
fn recipient_index(key: &StorageKey, group: &Id, recipient: &Id) -> Result<Id, Error> {
    Ok(key.commitment(
        &[group.as_slice(), recipient].concat(),
        b"Sigil/group-fanout-recipient/v0",
    )?)
}
pub fn group_history_id(text: &GroupText<'_>) -> Id {
    group_event_history_id(&Group {
        message: text.message,
        group: text.group,
        sender: text.sender,
        timestamp: text.timestamp,
        content: Content::Text(text.body),
    })
}
pub fn group_event_history_id(event: &Group<'_>) -> Id {
    let domain: &[u8] = match event.content {
        Content::Text(_) => b"Sigil/group-text-history/v0",
        Content::File(_) => b"Sigil/group-file-history/v0",
        Content::Rich(_) => b"Sigil/group-sigiltext-history/v0",
    };
    digest(domain, &[&event.group, &event.sender, &event.message])
}
fn retain(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &[u8],
    text: &Group<'_>,
    outgoing: bool,
    author: Id,
) -> Result<(), Error> {
    let own = peers::parse(own)?.binding;
    if let Content::Rich(bytes) = text.content {
        if outgoing {
            if let sigil_protocol::text::Document::Action(action) =
                sigil_protocol::text::Document::from_bytes(bytes)
                    .map_err(|_| Error::InvalidEvent)?
            {
                crate::structured::require_outgoing(
                    tx,
                    key,
                    recovery::account_scope(&own.server, own.account)?,
                    text.group,
                    &action,
                )?;
            }
        }
        crate::structured::ingest(
            tx,
            key,
            recovery::account_scope(&own.server, own.account)?,
            text.group,
            group_event_history_id(text),
            bytes,
        )?;
    }
    if !recovery::configured(tx)? {
        return Ok(());
    }
    recovery::retain_new(
        tx,
        key,
        recovery::account_scope(&own.server, own.account)?,
        &sigil_crypto::recovery::Record {
            id: group_event_history_id(text),
            revision: 1,
            conversation: text.group,
            author,
            created_at: text.timestamp,
            direction: if outgoing {
                sigil_crypto::recovery::Direction::Outgoing
            } else {
                sigil_crypto::recovery::Direction::Incoming
            },
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
struct Stored {
    message: GroupMessage,
    digest: Id,
}
fn load(db: &Connection, key: &StorageKey, own: &Id, id: &Id) -> Result<Option<Stored>, Error> {
    type Row = (Vec<u8>, Vec<u8>, bool);
    let row: Option<Row> = db.query_row("SELECT group_id,CASE WHEN length(content)<=65741 THEN content END,outgoing FROM group_messages WHERE id=?1", [id.as_slice()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional()?;
    let Some((group, bytes, outgoing)) = row else {
        return Ok(None);
    };
    let bytes = key.open(&bytes, &aad(50, own, id))?;
    if bytes.len() < 169 || bytes[0] > 1 || (bytes[0] == 1) != outgoing {
        return Err(Error::InvalidStore);
    }
    let context = context_from_bytes(&bytes[1..137])?;
    let digest = bytes[137..169]
        .try_into()
        .map_err(|_| Error::InvalidStore)?;
    let text = Group::from_bytes(&bytes[169..]).map_err(|_| Error::InvalidStore)?;
    if context.group.as_slice() != group
        || text.group != context.group
        || text.sender != context.sender
        || index(key, &text.group, &text.sender, &text.message)? != *id
        || (outgoing && context.sender != *own)
    {
        return Err(Error::InvalidStore);
    }
    Ok(Some(Stored {
        message: GroupMessage {
            context,
            plaintext: Zeroizing::new(bytes[169..].to_vec()),
            outgoing,
            duplicate: true,
        },
        digest,
    }))
}
fn save(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &Id,
    message: &GroupMessage,
    packet: &[u8],
) -> Result<Id, Error> {
    let text = message.event()?;
    let id = index(key, &text.group, &text.sender, &text.message)?;
    let mut bytes = Zeroizing::new(vec![message.outgoing as u8]);
    bytes.extend_from_slice(&context_bytes(&message.context));
    bytes.extend_from_slice(&Sha256::digest(packet));
    bytes.extend_from_slice(&message.plaintext);
    let packet = if message.outgoing {
        Some(key.seal(packet, &aad(51, own, &id))?)
    } else {
        None
    };
    tx.execute(
        "INSERT INTO group_messages VALUES(?1,?2,?3,?4,?5)",
        (
            id.as_slice(),
            text.group.as_slice(),
            key.seal(&bytes, &aad(50, own, &id))?,
            packet,
            message.outgoing,
        ),
    )?;
    Ok(id)
}
fn packet(
    db: &Connection,
    key: &StorageKey,
    own: &Id,
    id: &Id,
    stored: &Stored,
) -> Result<Zeroizing<Vec<u8>>, Error> {
    let sealed: Vec<u8> = db.query_row(
        "SELECT CASE WHEN length(packet)<=65840 THEN packet END FROM group_messages WHERE id=?1",
        [id.as_slice()],
        |r| r.get(0),
    )?;
    let bytes = key.open(&sealed, &aad(51, own, id))?;
    if <Id>::from(Sha256::digest(&bytes)) != stored.digest {
        return Err(Error::InvalidStore);
    }
    let packet = sk::Packet::from_bytes(&bytes)?;
    if packet.context() != stored.message.context
        || packet.message() != stored.message.event()?.message
    {
        return Err(Error::InvalidStore);
    }
    Ok(bytes)
}

impl ClientStore {
    /// Freeze one ciphertext and its complete recipient fan-out with the sender
    /// counter. Exact retries cannot change content, timestamp or membership.
    pub fn queue_group_text(
        &mut self,
        group: Id,
        message: Id,
        body: &str,
        timestamp: u64,
        now: u64,
    ) -> Result<(), Error> {
        self.queue_group_content(group, message, Content::Text(body), timestamp, now)
    }
    pub(crate) fn queue_group_content(
        &mut self,
        group: Id,
        message: Id,
        body: Content<'_>,
        timestamp: u64,
        now: u64,
    ) -> Result<(), Error> {
        if now == 0 || now > i64::MAX as u64 - 604800 {
            return Err(Error::Expired);
        }
        let own_binding = self.own_device_binding()?;
        let own = device_fingerprint(&own_binding)?;
        let own_fields = peers::parse(&own_binding)?.binding;
        crate::event::validate_content(
            body,
            &own_fields.server,
            &message,
            &crate::event::account(&own_fields),
            timestamp,
        )?;
        let text = Group {
            group,
            message,
            sender: own,
            timestamp,
            content: body,
        };
        let plaintext = Zeroizing::new(text.to_bytes().map_err(|_| Error::InvalidEvent)?);
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if matches!(body, Content::File(_) | Content::Rich(_)) {
            recovery::require_resend(
                &tx,
                &self.key,
                recovery::account_scope(&own_fields.server, own_fields.account)?,
                group_event_history_id(&text),
                body,
            )?;
        }
        let id = index(&self.key, &group, &own, &message)?;
        if let Some(prior) = load(&tx, &self.key, &own, &id)? {
            if !prior.message.outgoing || prior.message.plaintext != plaintext {
                return Err(Error::Conflict);
            }
            return Ok(());
        }
        let state = keys::current(&tx, &self.key, &own, &group)?;
        let pending: u32 = tx.query_row(
            "SELECT count(*) FROM group_messages WHERE group_id=?1 AND packet IS NOT NULL",
            [group.as_slice()],
            |r| r.get(0),
        )?;
        if pending >= MAX_PENDING {
            return Err(Error::Limit);
        }
        let (mut sender, ready, _) = match keys::load_sender(&tx, &self.key, &own, &state)? {
            Some(stored) => stored,
            None if state.members.len() == 1 && state.members[0].devices.len() == 1 => {
                let (sender, _) = sk::Sender::new(group, state.head, state.epoch, own)?;
                (sender, true, None)
            }
            None => return Err(Error::Unprepared),
        };
        if !ready {
            return Err(Error::Unprepared);
        }
        let mut recipients = Vec::new();
        let own_server = &own_fields.server;
        for member in &state.members {
            for binding in &member.devices {
                let fingerprint = peers::fingerprint(&binding.binding)?;
                if fingerprint == own {
                    continue;
                }
                let peer = peers::reference(&binding.binding.server, &binding.binding.device);
                let known = peers::verified(&tx, &self.key, &peer)?;
                if known.fingerprint != fingerprint || &known.binding.server != own_server {
                    return Err(Error::Unprepared);
                }
                let distribution_id = super::control::message_id(&sender.context(), &fingerprint);
                let (_, receipt) = keys::job(&tx, &self.key, &own, &group, &distribution_id)?
                    .ok_or(Error::Unprepared)?;
                if receipt.context != sender.context() || receipt.recipient != fingerprint {
                    return Err(Error::InvalidStore);
                }
                recipients.push((fingerprint, peer, known.binding.device));
            }
        }
        let local_only = recipients.is_empty();
        let packet = sender.seal(message, &plaintext)?.to_bytes();
        let content = GroupMessage {
            context: sender.context(),
            plaintext,
            outgoing: true,
            duplicate: false,
        };
        let stored_id = save(&tx, &self.key, &own, &content, &packet)?;
        if stored_id != id {
            return Err(Error::InvalidStore);
        }
        for (recipient, peer, device) in recipients {
            delivery::enqueue(
                &tx,
                &self.key,
                &own,
                &id,
                &content,
                delivery::Target {
                    fingerprint: recipient,
                    peer,
                    device,
                },
                now + 604800,
            )?;
        }
        keys::save_sender(&tx, &self.key, &own, &sender, true, None)?;
        if local_only {
            tx.execute(
                "UPDATE group_messages SET packet=NULL WHERE id=?1",
                [id.as_slice()],
            )?;
        }
        retain(
            &tx,
            &self.key,
            &own_binding,
            &text,
            true,
            own_fields.identity,
        )?;
        tx.commit()?;
        Ok(())
    }
    /// Read already authenticated local history without reviving retired keys.
    pub fn group_message(
        &mut self,
        group: Id,
        author: Id,
        message: Id,
    ) -> Result<GroupMessage, Error> {
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let tx = self.db.transaction()?;
        let id = index(&self.key, &group, &author, &message)?;
        let stored = load(&tx, &self.key, &own, &id)?.ok_or(Error::NotFound)?;
        Ok(stored.message)
    }
}

pub(crate) use delivery::acknowledge;
pub(super) use delivery::retire;

pub(crate) fn migrate_structured(tx: &Transaction<'_>, key: &StorageKey) -> Result<(), Error> {
    if !tx.query_row("SELECT EXISTS(SELECT 1 FROM own_device_binding)", [], |r| {
        r.get::<_, bool>(0)
    })? {
        return Ok(());
    }
    let own = peers::own(tx, key)?;
    let fingerprint = device_fingerprint(&own)?;
    let binding = peers::parse(&own)?.binding;
    let scope = recovery::account_scope(&binding.server, binding.account)?;
    let mut statement = tx.prepare("SELECT id FROM group_messages ORDER BY rowid")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let id: Vec<u8> = row.get(0)?;
        let id: Id = id.try_into().map_err(|_| Error::InvalidStore)?;
        let stored = load(tx, key, &fingerprint, &id)?.ok_or(Error::InvalidStore)?;
        let event = stored.message.event()?;
        if let Content::Rich(bytes) = event.content {
            crate::structured::ingest(
                tx,
                key,
                scope,
                event.group,
                group_event_history_id(&event),
                bytes,
            )?;
        }
    }
    Ok(())
}
