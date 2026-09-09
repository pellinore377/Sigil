//! Account-scoped, authenticated conversation state. Transport journals stay separate.
use crate::*;
use serde::{Deserialize, Serialize};
pub use sigil_protocol::conversation::{Action, Body, Operation, Private, Reference, Version};
use sigil_protocol::event::Content;

pub(crate) const MIGRATION: &str = "
CREATE TABLE conversation_ops(id BLOB PRIMARY KEY,scope BLOB NOT NULL,target BLOB NOT NULL,stamp BLOB NOT NULL UNIQUE,kind INTEGER NOT NULL CHECK(kind BETWEEN 0 AND 3),state BLOB NOT NULL);
CREATE INDEX conversation_scope ON conversation_ops(scope,kind,id);
CREATE INDEX conversation_target ON conversation_ops(target,id);
CREATE TABLE conversation_clock(id INTEGER PRIMARY KEY CHECK(id=1),state BLOB NOT NULL);
CREATE TABLE conversation_time(id INTEGER PRIMARY KEY CHECK(id=1),state BLOB NOT NULL);
CREATE TABLE conversation_operation_ids(id BLOB PRIMARY KEY,state BLOB NOT NULL);
CREATE TABLE conversation_receipts(id BLOB PRIMARY KEY,state BLOB NOT NULL,done INTEGER NOT NULL CHECK(done IN(0,1)));
CREATE INDEX conversation_receipts_pending ON conversation_receipts(done);
CREATE TABLE conversation_receipt_cursor(id INTEGER PRIMARY KEY CHECK(id=1),state BLOB NOT NULL);
CREATE TABLE conversation_sync(id BLOB PRIMARY KEY,state BLOB NOT NULL);
CREATE TABLE conversation_sync_deferred(peer BLOB NOT NULL,entry BLOB NOT NULL,original BLOB NOT NULL,PRIMARY KEY(peer,entry));
CREATE TABLE conversation_origins(id BLOB PRIMARY KEY,state BLOB NOT NULL);
CREATE TABLE conversation_fragments(id BLOB PRIMARY KEY,state BLOB NOT NULL);
CREATE TABLE conversation_cancelled(session BLOB NOT NULL,id BLOB NOT NULL,state BLOB NOT NULL,PRIMARY KEY(session,id));
PRAGMA user_version=62;";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    conversation: Id,
    author: Id,
    identity: Id,
    timestamp: u64,
    seen: u64,
    operation: Operation,
}
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Order(u64, Id, Id);
fn order(op: &Operation) -> Order {
    Order(op.version.counter, op.version.device, op.id)
}
fn index(key: &StorageKey, domain: &[u8], parts: &[&[u8]]) -> Result<Id, Error> {
    Ok(key.commitment(&parts.concat(), domain)?)
}
fn scope(key: &StorageKey, conversation: &Id) -> Result<Id, Error> {
    index(key, b"Sigil/conversation-scope/v0", &[conversation])
}
fn target(action: &Action, author: Id, message: Id) -> Reference {
    match action {
        Action::Edit { target, .. }
        | Action::Delete { target }
        | Action::Reaction { target, .. }
        | Action::Pin { target, .. }
        | Action::Note { target, .. }
        | Action::Receipt { target, .. } => target.clone(),
        Action::Private {
            value: Private::Consumed(target) | Private::Seen(target),
            ..
        } => target.clone(),
        Action::Private {
            value: Private::Clear { .. },
            ..
        } => Reference {
            author: [0; 32],
            message: [0; 32],
        },
        _ => Reference { author, message },
    }
}
fn target_index(key: &StorageKey, conversation: &Id, target: &Reference) -> Result<Id, Error> {
    index(
        key,
        b"Sigil/conversation-target/v0",
        &[conversation, &target.author, &target.message],
    )
}
fn entry_id(key: &StorageKey, entry: &Entry) -> Result<Id, Error> {
    index(
        key,
        b"Sigil/conversation-event/v0",
        &[&entry.conversation, &entry.author, &entry.operation.id],
    )
}
fn original(
    db: &Connection,
    key: &StorageKey,
    conversation: &Id,
    reference: &Reference,
) -> Result<Option<Entry>, Error> {
    let id = index(
        key,
        b"Sigil/conversation-event/v0",
        &[conversation, &reference.author, &reference.message],
    )?;
    let raw:Option<Vec<u8>>=db.query_row("SELECT CASE WHEN length(state)<=574000 THEN state END FROM conversation_ops WHERE id=?1",[id.as_slice()],|r|r.get(0)).optional()?;
    raw.map(|raw| open(key, &id, &raw)).transpose()
}
fn copyable(db: &Connection, key: &StorageKey, e: &Entry) -> Result<Option<bool>, Error> {
    if retention::removed(db, key, &entry_id(key, e)?)?.is_some() {
        return Ok(Some(false));
    }
    if e.operation.ephemeral() || matches!(e.operation.action, Action::SyncPart { .. }) {
        return Ok(Some(false));
    }
    if let Action::Edit { target, .. } = &e.operation.action {
        return Ok(original(db, key, &e.conversation, target)?.map(|p| {
            p.author == e.author
                && matches!(
                    p.operation.action,
                    Action::Post {
                        expires_at: None,
                        view_once: false,
                        ..
                    }
                )
        }));
    }
    Ok(Some(true))
}
fn seal(key: &StorageKey, id: &Id, entry: &Entry) -> Result<Vec<u8>, Error> {
    let raw = Zeroizing::new(serde_json::to_vec(entry).map_err(|_| Error::InvalidStore)?);
    groups::storage_record::seal_record(key, &raw, &binding(90, id, b"conversation"))
}
fn open(key: &StorageKey, id: &Id, raw: &[u8]) -> Result<Entry, Error> {
    let raw = groups::storage_record::open_record(key, raw, &binding(90, id, b"conversation"))?;
    let entry: Entry = serde_json::from_slice(&raw).map_err(|_| Error::InvalidStore)?;
    if entry_id(key, &entry)? != *id {
        return Err(Error::InvalidStore);
    }
    entry
        .operation
        .validate()
        .map_err(|_| Error::InvalidStore)?;
    Ok(entry)
}
fn clock(db: &Connection, key: &StorageKey) -> Result<u64, Error> {
    let raw: Option<Vec<u8>> = db
        .query_row(
            "SELECT CASE WHEN length(state)=44 THEN state END FROM conversation_clock WHERE id=1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    raw.map(|v| {
        key.open(&v, &binding(91, &[0; 32], b"clock"))?
            .as_slice()
            .try_into()
            .map(u64::from_be_bytes)
            .map_err(|_| Error::InvalidStore)
    })
    .transpose()
    .map(|v| v.unwrap_or(0))
}
fn observe(tx: &Transaction<'_>, key: &StorageKey, counter: u64) -> Result<(), Error> {
    let value = clock(tx, key)?.max(counter);
    tx.execute("INSERT INTO conversation_clock VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET state=excluded.state",[key.seal(&value.to_be_bytes(),&binding(91,&[0;32],b"clock"))?])?;
    Ok(())
}
fn visit(
    db: &Connection,
    key: &StorageKey,
    sql: &str,
    at: &Id,
    mut f: impl FnMut(Entry) -> Result<(), Error>,
) -> Result<(), Error> {
    let mut stmt = db.prepare(sql)?;
    let mut rows = stmt.query([at.as_slice()])?;
    while let Some(row) = rows.next()? {
        let id: Vec<u8> = row.get(0)?;
        let raw: Vec<u8> = row.get(1)?;
        f(open(
            key,
            &id.try_into().map_err(|_| Error::InvalidStore)?,
            &raw,
        )?)?;
    }
    Ok(())
}
#[cfg(test)]
fn entries(db: &Connection, key: &StorageKey, sql: &str, at: &Id) -> Result<Vec<Entry>, Error> {
    let mut result = Vec::new();
    visit(db, key, sql, at, |e| {
        result.push(e);
        Ok(())
    })?;
    Ok(result)
}
fn ingest(tx: &Transaction<'_>, key: &StorageKey, entry: Entry) -> Result<(), Error> {
    ingest_only(tx, key, &entry)?;
    structured(tx, key, &entry)?;
    archive_new(tx, key, &entry)
}
fn structured(tx: &Transaction<'_>, key: &StorageKey, e: &Entry) -> Result<(), Error> {
    if let Action::Post {
        body: Body::Rich(raw),
        ..
    } = &e.operation.action
    {
        let (scope, _) = crate::structured::account_context(tx, key)?;
        crate::structured::ingest(tx, key, scope, e.conversation, history_id(e), raw)?;
    }
    Ok(())
}
fn ingest_only(tx: &Transaction<'_>, key: &StorageKey, entry: &Entry) -> Result<(), Error> {
    if entry.timestamp == 0 || entry.timestamp > i64::MAX as u64 {
        return Err(Error::InvalidEvent);
    }
    entry
        .operation
        .validate()
        .map_err(|_| Error::InvalidEvent)?;
    let id = entry_id(key, entry)?;
    let prior:Option<Vec<u8>>=tx.query_row("SELECT CASE WHEN length(state)<=574000 THEN state END FROM conversation_ops WHERE id=?1",[id.as_slice()],|r|r.get(0)).optional()?;
    if let Some(prior) = prior {
        let old = open(key, &id, &prior)?;
        let redacted = retention::removed(tx, key, &id)?;
        let same_operation = old.operation == entry.operation
            || redacted.is_some_and(|digest| {
                entry
                    .operation
                    .to_bytes()
                    .is_ok_and(|raw| <Id>::from(Sha256::digest(raw)) == digest)
            });
        if old.author != entry.author
            || old.identity != entry.identity
            || old.timestamp != entry.timestamp
            || !same_operation
        {
            return Err(Error::Conflict);
        }
        return Ok(());
    }
    let stamp = index(
        key,
        b"Sigil/conversation-stamp/v0",
        &[
            &entry.author,
            &entry.operation.version.device,
            &entry.operation.version.counter.to_be_bytes(),
        ],
    )?;
    // A device cannot equivocate on its logical clock, even across conversations.
    if tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM conversation_ops WHERE stamp=?1)",
        [stamp.as_slice()],
        |r| r.get::<_, bool>(0),
    )? {
        return Err(Error::Conflict);
    }
    let target = target(&entry.operation.action, entry.author, entry.operation.id);
    let kind = match entry.operation.action {
        Action::Post {
            body: Body::Rich(ref bytes),
            ..
        } if matches!(
            sigil_protocol::text::Document::from_bytes(bytes).map_err(|_| Error::InvalidEvent)?,
            sigil_protocol::text::Document::Action(_)
        ) =>
        {
            3
        }
        Action::Post { .. } => 0,
        Action::Private { .. } => 1,
        Action::Typing { .. } | Action::Presence { .. } => 2,
        _ => 3,
    };
    tx.execute(
        "INSERT INTO conversation_ops VALUES(?1,?2,?3,?4,?5,?6)",
        rusqlite::params![
            id.as_slice(),
            scope(key, &entry.conversation)?.as_slice(),
            target_index(key, &entry.conversation, &target)?.as_slice(),
            stamp.as_slice(),
            kind,
            seal(key, &id, entry)?
        ],
    )?;
    Ok(())
}
pub(crate) fn validate(content: Content<'_>, _message: &Id, device: &Id) -> Result<(), Error> {
    if let Content::Conversation(raw) = content {
        let op = Operation::from_bytes(raw).map_err(|_| Error::InvalidEvent)?;
        if op.version.device != *device {
            return Err(Error::InvalidEvent);
        }
    }
    Ok(())
}
pub(crate) fn retain(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: &[u8],
    conversation: Id,
    (author, identity): (Id, Id),
    timestamp: u64,
    content: Content<'_>,
) -> Result<bool, Error> {
    let Content::Conversation(raw) = content else {
        return Ok(false);
    };
    let operation = Operation::from_bytes(raw).map_err(|_| Error::InvalidEvent)?;
    let own = event::account(&peers::parse(own)?.binding);
    if own == author {
        observe(tx, key, operation.version.counter)?;
    }
    match &operation.action {
        Action::Private {
            conversation,
            value: _,
        } => {
            if author != own {
                return Err(Error::InvalidEvent);
            }
            ingest(
                tx,
                key,
                Entry {
                    conversation: *conversation,
                    author,
                    identity,
                    timestamp,
                    seen: now(),
                    operation,
                },
            )?;
        }
        Action::SyncPart { .. } => {
            if author != own {
                return Err(Error::InvalidEvent);
            }
            sync::receive(tx, key, &operation)?;
        }
        _ => ingest(
            tx,
            key,
            Entry {
                conversation,
                author,
                identity,
                timestamp,
                seen: now(),
                operation,
            },
        )?,
    }
    Ok(true)
}

pub struct Message {
    pub reference: Reference,
    pub body: Option<Body>,
    pub timestamp: u64,
    pub reply: Option<Reference>,
    pub thread: Option<Reference>,
    pub expires_at: Option<u64>,
    pub view_once: bool,
    pub deleted: bool,
    pub reactions: Vec<(Id, String)>,
    pub pinned: bool,
    pub noted: bool,
    pub delivered: Vec<Id>,
    pub read: Vec<Id>,
    pub seen: bool,
}
pub(crate) fn message(
    db: &Connection,
    key: &StorageKey,
    conversation: Id,
    reference: Reference,
    now: u64,
    own: Id,
) -> Result<Message, Error> {
    let cleared = clear_context(db, key, conversation, own)?;
    message_after_clear(db, key, conversation, reference, now, own, &cleared)
}
fn clear_context(
    db: &Connection,
    key: &StorageKey,
    conversation: Id,
    own: Id,
) -> Result<std::collections::BTreeMap<Id, u64>, Error> {
    let mut result: std::collections::BTreeMap<Id, u64> = std::collections::BTreeMap::new();
    let at = target_index(
        key,
        &conversation,
        &Reference {
            author: [0; 32],
            message: [0; 32],
        },
    )?;
    visit(db, key, "SELECT id,CASE WHEN length(state)<=574000 THEN state END FROM conversation_ops WHERE target=?1 AND kind=1 ORDER BY id", &at, |entry| {
        if entry.conversation != conversation || entry.author != own { return Err(Error::InvalidStore); }
        if let Action::Private { value: Private::Clear { observed }, .. } = entry.operation.action {
            for version in observed { let prior = result.entry(version.device).or_default(); *prior = (*prior).max(version.counter); }
        }
        Ok(())
    })?;
    Ok(result)
}
#[allow(clippy::too_many_arguments)]
fn message_after_clear(
    db: &Connection,
    key: &StorageKey,
    conversation: Id,
    reference: Reference,
    now: u64,
    own: Id,
    cleared: &std::collections::BTreeMap<Id, u64>,
) -> Result<Message, Error> {
    if now == 0 || now > i64::MAX as u64 {
        return Err(Error::Expired);
    }
    let original = original(db, key, &conversation, &reference)?.ok_or(Error::NotFound)?;
    let Action::Post {
        body,
        reply,
        thread,
        expires_at,
        view_once,
    } = &original.operation.action
    else {
        return Err(Error::NotFound);
    };
    let now = if expires_at.is_some() {
        time_floor(db, key, now)?
    } else {
        now
    };
    let mut view = Message {
        reference: reference.clone(),
        body: Some(body.clone()),
        timestamp: original.timestamp,
        reply: reply.clone(),
        thread: thread.clone(),
        expires_at: *expires_at,
        view_once: *view_once,
        deleted: original.operation.version.counter
            <= cleared
                .get(&original.operation.version.device)
                .copied()
                .unwrap_or(0)
            || expires_at.is_some_and(|v| now >= v)
            || retention::removed(db, key, &entry_id(key, &original)?)?.is_some(),
        reactions: Vec::new(),
        pinned: false,
        noted: false,
        delivered: Vec::new(),
        read: Vec::new(),
        seen: false,
    };
    let mut edit = None;
    let mut pin = None;
    let mut note = None;
    let mut reactions = std::collections::BTreeMap::new();
    let mut delivered = std::collections::BTreeSet::new();
    let mut read = std::collections::BTreeSet::new();
    visit(db,key,"SELECT id,CASE WHEN length(state)<=574000 THEN state END FROM conversation_ops WHERE target=?1 AND kind!=0 ORDER BY id",&target_index(key,&conversation,&reference)?,|entry|{
        if entry.conversation!=conversation || target(&entry.operation.action,entry.author,entry.operation.id)!=reference {return Err(Error::InvalidStore);}
        let rank=order(&entry.operation);
        match &entry.operation.action {
            Action::Delete{..} if entry.author==reference.author=>view.deleted=true,
            Action::Private{value:Private::Consumed(_),..} if entry.author==own && *view_once=>view.deleted=true,
            Action::Private{value:Private::Seen(_),..} if entry.author==own=>view.seen=true,
            Action::Edit{body:replacement,..} if entry.author==reference.author && edit.as_ref().is_none_or(|v|rank>*v) && body.editable()=>{view.body=Some(replacement.clone());edit=Some(rank);}
            Action::Reaction{emoji,active,..}=>{let value=active.then(||emoji.clone());let v=reactions.entry(entry.author).or_insert((rank.clone(),value.clone()));if rank>v.0 {*v=(rank,value);}}
            Action::Pin{active,..}=>{if pin.as_ref().is_none_or(|(v,_)|rank>*v){pin=Some((rank,*active));}}
            Action::Note{active,..}=>{if note.as_ref().is_none_or(|(v,_)|rank>*v){note=Some((rank,*active));}}
            Action::Receipt{read:is_read,..} if entry.author!=reference.author=>{delivered.insert(entry.author);if *is_read{read.insert(entry.author);}}
            _=>{}
        }
        Ok(())
    })?;
    view.pinned = pin.is_some_and(|(_, v)| v);
    view.noted = note.is_some_and(|(_, v)| v);
    view.reactions = reactions
        .into_iter()
        .filter_map(|(author, (_, emoji))| emoji.map(|e| (author, e)))
        .collect();
    view.delivered = delivered.into_iter().collect();
    view.read = read.into_iter().collect();
    if view.deleted {
        view.body = None;
        view.reactions.clear();
        view.pinned = false;
        view.noted = false;
    }
    Ok(view)
}

impl ClientStore {
    pub fn conversation_operation(&mut self, id: Id, action: Action) -> Result<Operation, Error> {
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let operation = new_operation(&tx, &self.key, own, id, action)?;
        tx.commit()?;
        Ok(operation)
    }
    pub fn clear_conversation(
        &mut self,
        conversation: Id,
        request: Id,
        timestamp: u64,
    ) -> Result<(), Error> {
        let binding_bytes = self.own_device_binding()?;
        let own = device_fingerprint(&binding_bytes)?;
        let binding = peers::parse(&binding_bytes)?.binding;
        let author = event::account(&binding);
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let batch_id = |index: u64| -> Id {
            Sha256::digest(
                [
                    b"Sigil/clear-conversation/v1".as_slice(),
                    &conversation,
                    &request,
                    &index.to_be_bytes(),
                ]
                .concat(),
            )
            .into()
        };
        if tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversation_operation_ids WHERE id=?1)",
            [batch_id(0).as_slice()],
            |row| row.get::<_, bool>(0),
        )? {
            return Ok(());
        }
        let mut observed: std::collections::BTreeMap<Id, u64> = std::collections::BTreeMap::new();
        visit(&tx, &self.key, "SELECT id,CASE WHEN length(state)<=574000 THEN state END FROM conversation_ops WHERE scope=?1 ORDER BY id", &scope(&self.key, &conversation)?, |entry| {
            if entry.conversation != conversation { return Err(Error::InvalidStore); }
            let version = entry.operation.version;
            let prior = observed.entry(version.device).or_default();
            *prior = (*prior).max(version.counter);
            Ok(())
        })?;
        // Include this device even when clearing an empty conversation.
        observed.insert(own, clock(&tx, &self.key)?.max(1));
        let observed: Vec<_> = observed
            .into_iter()
            .map(|(device, counter)| Version { device, counter })
            .collect();
        for (index, chunk) in observed.chunks(128).enumerate() {
            let operation = new_operation(
                &tx,
                &self.key,
                own,
                batch_id(index as u64),
                Action::Private {
                    conversation,
                    value: Private::Clear {
                        observed: chunk.to_vec(),
                    },
                },
            )?;
            retain(
                &tx,
                &self.key,
                &binding_bytes,
                [0; 32],
                (author, binding.identity),
                timestamp,
                Content::Conversation(&operation.to_bytes().map_err(|_| Error::InvalidEvent)?),
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn queue_peer_operation(
        &mut self,
        peer: Id,
        operation: &Operation,
        timestamp: u64,
        now: u64,
    ) -> Result<(), Error> {
        self.queue_direct_operation(&[peer], operation, timestamp, now)
    }
}
fn new_operation(
    tx: &Transaction<'_>,
    key: &StorageKey,
    own: Id,
    id: Id,
    action: Action,
) -> Result<Operation, Error> {
    let prior:Option<Vec<u8>>=tx.query_row("SELECT CASE WHEN length(state)=76 THEN state END FROM conversation_operation_ids WHERE id=?1",[id.as_slice()],|r|r.get(0)).optional()?;
    let prior = prior
        .map(|v| key.open(&v, &binding(95, &own, &id)))
        .transpose()?;
    let counter = if let Some(v) = &prior {
        if v.len() != 40 {
            return Err(Error::InvalidStore);
        }
        u64::from_be_bytes(v[..8].try_into().map_err(|_| Error::InvalidStore)?)
    } else {
        clock(tx, key)?
            .checked_add(1)
            .filter(|v| *v <= i64::MAX as u64)
            .ok_or(Error::Limit)?
    };
    let operation = Operation {
        id,
        version: Version {
            device: own,
            counter,
        },
        action,
    };
    let bytes = operation.to_bytes().map_err(|_| Error::InvalidEvent)?;
    let tag = key.commitment(&bytes, &binding(95, &own, &id))?;
    if let Some(v) = prior {
        if v[8..] != tag {
            return Err(Error::Conflict);
        }
    } else {
        tx.execute(
            "INSERT INTO conversation_operation_ids VALUES(?1,?2)",
            (
                id.as_slice(),
                key.seal(
                    &[counter.to_be_bytes().as_slice(), &tag].concat(),
                    &binding(95, &own, &id),
                )?,
            ),
        )?;
    }
    observe(tx, key, counter)?;
    Ok(operation)
}
impl ClientStore {
    /// Atomically queues the same operation to the selected devices of one account.
    pub fn queue_direct_operation(
        &mut self,
        peers: &[Id],
        operation: &Operation,
        timestamp: u64,
        now: u64,
    ) -> Result<(), Error> {
        let mut conversation = None;
        let mut recipients = Vec::new();
        for peer in peers {
            let current = self.direct_conversation(*peer)?;
            if conversation.is_some_and(|old| old != current)
                || recipients.iter().any(|(p, _)| p == peer)
            {
                return Err(Error::Conflict);
            }
            conversation = Some(current);
            let id = Sha256::digest(
                [
                    b"Sigil/conversation-delivery/v0".as_slice(),
                    &operation.id,
                    peer,
                ]
                .concat(),
            )
            .into();
            recipients.push((*peer, id));
        }
        let bytes = Zeroizing::new(operation.to_bytes().map_err(|_| Error::InvalidEvent)?);
        self.queue_peer_contents(&recipients, Content::Conversation(&bytes), timestamp, now)
    }
    pub fn queue_group_operation(
        &mut self,
        group: Id,
        operation: &Operation,
        timestamp: u64,
        now: u64,
    ) -> Result<(), Error> {
        if matches!(
            operation.action,
            Action::Private { .. } | Action::SyncPart { .. } | Action::Presence { .. }
        ) {
            return Err(Error::InvalidEvent);
        }
        let bytes = Zeroizing::new(operation.to_bytes().map_err(|_| Error::InvalidEvent)?);
        self.queue_group_content(
            group,
            operation.id,
            Content::Conversation(&bytes),
            timestamp,
            now,
        )
    }
    pub fn conversation_message(
        &mut self,
        conversation: Id,
        reference: Reference,
        now: u64,
    ) -> Result<Message, Error> {
        let (_, own) = crate::structured::account_context(&self.db, &self.key)?;
        let mut value = message(&self.db, &self.key, conversation, reference, now, own)?;
        if value.view_once {
            value.body = None;
        }
        Ok(value)
    }
    pub fn apply_private_operation(
        &mut self,
        operation: &Operation,
        timestamp: u64,
    ) -> Result<(), Error> {
        if !matches!(operation.action, Action::Private { .. }) {
            return Err(Error::InvalidEvent);
        }
        let own = self.own_device_binding()?;
        validate(
            Content::Conversation(&operation.to_bytes().map_err(|_| Error::InvalidEvent)?),
            &operation.id,
            &device_fingerprint(&own)?,
        )?;
        let account = event::account(&peers::parse(&own)?.binding);
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        retain(
            &tx,
            &self.key,
            &own,
            [0; 32],
            (account, peers::parse(&own)?.binding.identity),
            timestamp,
            Content::Conversation(&operation.to_bytes().map_err(|_| Error::InvalidEvent)?),
        )?;
        tx.commit()?;
        Ok(())
    }
}

pub(crate) fn authorize_route(content: Content<'_>, same_account: bool) -> Result<(), Error> {
    if let Content::Conversation(raw) = content {
        let op = Operation::from_bytes(raw).map_err(|_| Error::InvalidEvent)?;
        if matches!(op.action, Action::Private { .. } | Action::SyncPart { .. }) && !same_account {
            return Err(Error::InvalidEvent);
        }
    }
    Ok(())
}

pub struct Draft {
    pub version: Version,
    pub text: String,
}
pub struct Preferences {
    pub cleared: std::collections::BTreeMap<Id, u64>,
    pub pinned: bool,
    pub unread: bool,
    pub snoozed_until: Option<u64>,
    pub hidden: bool,
    pub collections_enabled: bool,
    pub read_receipts: bool,
    pub typing_indicators: bool,
    pub presence_sharing: bool,
    pub recovery_history_days: Option<u32>,
    pub collections: Vec<(Id, String)>,
    pub collection_members: Vec<Id>,
    pub drafts: Vec<Draft>,
    pub ui: std::collections::BTreeMap<String, String>,
}
fn preferences(
    db: &Connection,
    key: &StorageKey,
    conversation: Id,
    own: Id,
) -> Result<Preferences, Error> {
    let mut result = Preferences {
        cleared: std::collections::BTreeMap::new(),
        pinned: false,
        unread: false,
        snoozed_until: None,
        hidden: false,
        collections_enabled: false,
        read_receipts: true,
        typing_indicators: true,
        presence_sharing: false,
        recovery_history_days: None,
        collections: Vec::new(),
        collection_members: Vec::new(),
        drafts: Vec::new(),
        ui: std::collections::BTreeMap::new(),
    };
    let mut latest = std::collections::BTreeMap::new();
    let mut drafts: std::collections::BTreeMap<Id, (Version, String)> =
        std::collections::BTreeMap::new();
    let mut observed: std::collections::BTreeMap<Id, u64> = std::collections::BTreeMap::new();
    for conv in [
        Some(conversation),
        (conversation != [0; 32]).then_some([0; 32]),
    ]
    .into_iter()
    .flatten()
    {
        visit(db,key,"SELECT id,CASE WHEN length(state)<=574000 THEN state END FROM conversation_ops WHERE scope=?1 AND kind=1 ORDER BY id",&scope(key,&conv)?,|e|{
            if e.conversation!=conv {return Err(Error::InvalidStore);}
            if e.author!=own{return Ok(());}
            let rank=(e.conversation!=[0;32],order(&e.operation));
            let Action::Private{value,..}=e.operation.action else{return Ok(())};
            let tag=match &value {
                Private::Clear{observed} if conv==conversation=>{for version in observed {let prior=result.cleared.entry(version.device).or_default();*prior=(*prior).max(version.counter);}return Ok(());}
                Private::Draft{text,observed:seen} if conv==conversation=>{
                    let version=e.operation.version;
                    let prior=drafts.entry(version.device).or_insert((version.clone(),text.clone()));
                    if version.counter>prior.0.counter {*prior=(version,text.clone());}
                    for v in seen {let max=observed.entry(v.device).or_default();*max=(*max).max(v.counter);}
                    return Ok(());
                }
                Private::ConversationPin(_) if conv==conversation=>(0,[0;32]),
                Private::Unread(_) if conv==conversation=>(1,[0;32]),
                Private::Snooze(_) if conv==conversation=>(2,[0;32]),
                Private::Hidden(_) if conv==conversation=>(3,[0;32]),
                Private::CollectionsEnabled(_)=>(4,[0;32]),
                Private::ReadReceipts(_)=>(5,[0;32]),
                Private::TypingIndicators(_)=>(6,[0;32]),
                Private::PresenceSharing(_)=>(7,[0;32]),
                Private::RecoveryRetention(_) if conv==[0;32]=>(10,[0;32]),
                Private::Collection{id,..}=>(8,*id),
                Private::CollectionMember{id,..} if conv==conversation=>(9,*id),
                Private::UiSetting{key,..}=>(11,Sha256::digest(key.as_bytes()).into()),
                _=>return Ok(()),
            };
            let prior=latest.entry(tag).or_insert((rank.clone(),value.clone()));
            if rank>prior.0 {*prior=(rank,value);}
            Ok(())
        })?;
    }
    for (_, (_, value)) in latest {
        match value {
            Private::ConversationPin(v) => result.pinned = v,
            Private::Unread(v) => result.unread = v,
            Private::Snooze(v) => result.snoozed_until = v,
            Private::Hidden(v) => result.hidden = v,
            Private::CollectionsEnabled(v) => result.collections_enabled = v,
            Private::ReadReceipts(v) => result.read_receipts = v,
            Private::TypingIndicators(v) => result.typing_indicators = v,
            Private::PresenceSharing(v) => result.presence_sharing = v,
            Private::RecoveryRetention(v) => result.recovery_history_days = v,
            Private::UiSetting {
                key,
                value: Some(value),
            } => {
                result.ui.insert(key, value);
            }
            Private::Collection {
                id,
                name,
                present: true,
            } => result.collections.push((id, name)),
            Private::CollectionMember { id, present: true } => result.collection_members.push(id),
            _ => {}
        }
    }
    result.drafts = drafts
        .into_values()
        .filter_map(|(version, text)| {
            (version.counter > result.cleared.get(&version.device).copied().unwrap_or(0)
                && observed.get(&version.device).copied().unwrap_or(0) < version.counter)
                .then_some(Draft { version, text })
        })
        .collect();
    Ok(result)
}
pub struct Activity {
    pub author: Id,
    pub typing: bool,
    pub online: bool,
    pub status: Option<sigil_protocol::conversation::Presence>,
}
impl ClientStore {
    pub fn conversation_preferences(&mut self, conversation: Id) -> Result<Preferences, Error> {
        let (_, own) = crate::structured::account_context(&self.db, &self.key)?;
        preferences(&self.db, &self.key, conversation, own)
    }
    pub fn conversation_activity(
        &mut self,
        conversation: Id,
        now: u64,
    ) -> Result<Vec<Activity>, Error> {
        let now = time_floor(&self.db, &self.key, now)?;
        let mut latest = std::collections::BTreeMap::new();
        visit(&self.db,&self.key,"SELECT id,CASE WHEN length(state)<=574000 THEN state END FROM conversation_ops WHERE scope=?1 AND kind=2 ORDER BY id",&scope(&self.key,&conversation)?,|e|{
            if e.conversation!=conversation {return Err(Error::InvalidStore);}
            let (kind,active,until,status)=match e.operation.action {Action::Typing{active,until}=>(0,active,until,None),Action::Presence{online,until,activity}=>(1,online,until,activity),_=>return Ok(())};
            let rank=order(&e.operation);
            let active=active && now<until && now<e.seen.saturating_add(if kind==0 {30}else{120});
            let v=latest.entry((e.author,kind)).or_insert((rank.clone(),active,status));
            if rank>v.0 {*v=(rank,active,status);}
            Ok(())
        })?;
        let mut result = std::collections::BTreeMap::new();
        for ((author, kind), (_, active, status)) in latest {
            let v = result.entry(author).or_insert(Activity {
                author,
                typing: false,
                online: false,
                status: None,
            });
            if kind == 0 {
                v.typing = active
            } else {
                v.online = active;
                v.status = status.filter(|_| active);
            };
        }
        Ok(result.into_values().collect())
    }
    pub fn note_to_self(
        &mut self,
        operation: &Operation,
        timestamp: u64,
        now: u64,
    ) -> Result<Id, Error> {
        if matches!(
            operation.action,
            Action::Private { .. }
                | Action::SyncPart { .. }
                | Action::Presence { .. }
                | Action::Typing { .. }
        ) {
            return Err(Error::InvalidEvent);
        }
        let own = self.own_device_binding()?;
        let account = event::account(&peers::parse(&own)?.binding);
        let conversation =
            Sha256::digest([b"Sigil/note-to-self/v0".as_slice(), &account].concat()).into();
        let bytes = operation.to_bytes().map_err(|_| Error::InvalidEvent)?;
        validate(
            Content::Conversation(&bytes),
            &operation.id,
            &device_fingerprint(&own)?,
        )?;
        event::validate_content(
            Content::Conversation(&bytes),
            &peers::parse(&own)?.binding.server,
            &operation.id,
            &account,
            timestamp,
        )?;
        require_send(
            &self.db,
            &self.key,
            conversation,
            Content::Conversation(&bytes),
            timestamp,
            now,
        )?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        retain(
            &tx,
            &self.key,
            &own,
            conversation,
            (account, peers::parse(&own)?.binding.identity),
            timestamp,
            Content::Conversation(&bytes),
        )?;
        tx.commit()?;
        Ok(conversation)
    }
    pub fn consume_view_once(
        &mut self,
        conversation: Id,
        reference: Reference,
        id: Id,
        now: u64,
    ) -> Result<Body, Error> {
        self.consume_view_once_checked(conversation, reference, id, now, |_| Ok(()))
    }
    pub(crate) fn consume_view_once_checked(
        &mut self,
        conversation: Id,
        reference: Reference,
        id: Id,
        now: u64,
        check: impl FnOnce(&Message) -> Result<(), Error>,
    ) -> Result<Body, Error> {
        let now = time_floor(&self.db, &self.key, now)?;
        let own = self.own_device_binding()?;
        let account = event::account(&peers::parse(&own)?.binding);
        let device = device_fingerprint(&own)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let view = message(
            &tx,
            &self.key,
            conversation,
            reference.clone(),
            now,
            account,
        )?;
        if !view.view_once || view.deleted {
            return Err(Error::Obsolete);
        }
        check(&view)?;
        let counter = clock(&tx, &self.key)?.checked_add(1).ok_or(Error::Limit)?;
        let operation = Operation {
            id,
            version: Version { device, counter },
            action: Action::Private {
                conversation,
                value: Private::Consumed(reference),
            },
        };
        retain(
            &tx,
            &self.key,
            &own,
            conversation,
            (account, peers::parse(&own)?.binding.identity),
            now,
            Content::Conversation(&operation.to_bytes().map_err(|_| Error::Limit)?),
        )?;
        tx.commit()?;
        view.body.ok_or(Error::Obsolete)
    }
}

#[cfg(test)]
#[path = "history_load_tests.rs"]
mod load_tests;
#[path = "conversation_receipts.rs"]
pub(crate) mod receipts;
#[path = "conversation_retention.rs"]
mod retention;
#[path = "conversation_sync.rs"]
pub(crate) mod sync;
pub(crate) use retention::archive_tombstone;
pub use retention::HistoryCleanup;
#[cfg(test)]
#[path = "conversation_tests.rs"]
mod tests;

fn origin_index(key: &StorageKey, conversation: &Id, sender: &Id, id: &Id) -> Result<Id, Error> {
    index(
        key,
        b"Sigil/conversation-origin/v0",
        &[conversation, sender, id],
    )
}
pub(crate) fn native(
    tx: &Transaction<'_>,
    key: &StorageKey,
    conversation: Id,
    (author, identity): (Id, Id),
    (sender, id, archive): (Id, Id, Id),
    timestamp: u64,
    content: Content<'_>,
) -> Result<(), Error> {
    let logical = if let Content::Conversation(raw) = content {
        Operation::from_bytes(raw)
            .map_err(|_| Error::InvalidEvent)?
            .id
    } else {
        id
    };
    let at = origin_index(key, &conversation, &sender, &id)?;
    let raw = [author.as_slice(), &logical].concat();
    let old:Option<Vec<u8>>=tx.query_row("SELECT CASE WHEN length(state)=100 THEN state END FROM conversation_origins WHERE id=?1",[at.as_slice()],|r|r.get(0)).optional()?;
    if let Some(old) = old {
        if key.open(&old, &binding(93, &at, b"origin"))?.as_slice() != raw {
            return Err(Error::Conflict);
        }
    } else {
        tx.execute(
            "INSERT INTO conversation_origins VALUES(?1,?2)",
            (at.as_slice(), key.seal(&raw, &binding(93, &at, b"origin"))?),
        )?;
    }
    let body = match content {
        Content::Text(v) => Body::Text(v.into()),
        Content::File(v) => Body::File(v.to_vec()),
        Content::Rich(v) => Body::Rich(v.to_vec()),
        Content::Conversation(_) => return Ok(()),
    };
    let device = Sha256::digest(
        // Legacy clocks remain independent of live device counters.
        [
            b"Sigil/legacy-conversation-version/v0".as_slice(),
            &conversation,
            &author,
            &id,
        ]
        .concat(),
    )
    .into();
    retention::link(tx, key, archive, conversation, author, id)?;
    ingest_only(
        tx,
        key,
        &Entry {
            conversation,
            author,
            identity,
            timestamp,
            seen: now(),
            operation: Operation {
                id,
                version: Version { device, counter: 1 },
                action: Action::Post {
                    body,
                    reply: None,
                    thread: None,
                    expires_at: None,
                    view_once: false,
                },
            },
        },
    )
}
pub(crate) fn require_live(
    db: &Connection,
    key: &StorageKey,
    conversation: Id,
    sender: Id,
    id: Id,
    now: u64,
    allow_once: bool,
) -> Result<(), Error> {
    let at = origin_index(key, &conversation, &sender, &id)?;
    let raw:Option<Vec<u8>>=db.query_row("SELECT CASE WHEN length(state)=100 THEN state END FROM conversation_origins WHERE id=?1",[at.as_slice()],|r|r.get(0)).optional()?;
    let Some(raw) = raw else { return Ok(()) };
    let raw = key.open(&raw, &binding(93, &at, b"origin"))?;
    if raw.len() != 64 {
        return Err(Error::InvalidStore);
    }
    let target = Reference {
        author: raw[..32].try_into().map_err(|_| Error::InvalidStore)?,
        message: raw[32..].try_into().map_err(|_| Error::InvalidStore)?,
    };
    let session = connection::session_in(db, key)?.ok_or(Error::Unprepared)?;
    let server = session
        .address
        .split_once(':')
        .ok_or(Error::InvalidStore)?
        .1;
    let own = event::account_reference(server, &connection::decode_id(&session.account_id)?);
    match message(db, key, conversation, target, now, own) {
        Ok(m) if m.deleted || (m.view_once && !allow_once) => Err(Error::Obsolete),
        Ok(_) | Err(Error::NotFound) => Ok(()),
        Err(e) => Err(e),
    }
}
pub(crate) fn require_payload(
    db: &Connection,
    key: &StorageKey,
    raw: &[u8],
    now: u64,
    allow_once: bool,
) -> Result<(), Error> {
    crate::erasure::require_retained(raw)?;
    if let Ok(e) = sigil_protocol::event::Direct::from_bytes(raw) {
        require_edit(db, key, e.conversation, e.content)?;
        require_live(
            db,
            key,
            e.conversation,
            e.sender,
            e.message,
            now,
            allow_once,
        )?;
    }
    if let Ok(e) = sigil_protocol::event::Group::from_bytes(raw) {
        require_edit(db, key, e.group, e.content)?;
        require_live(db, key, e.group, e.sender, e.message, now, allow_once)?;
    }
    Ok(())
}
fn require_edit(
    db: &Connection,
    key: &StorageKey,
    conversation: Id,
    content: Content<'_>,
) -> Result<(), Error> {
    if let Content::Rich(raw) = content {
        structured::require_content(db, key, conversation, raw)?;
    }
    if let Content::Conversation(raw) = content {
        let op = Operation::from_bytes(raw).map_err(|_| Error::InvalidEvent)?;
        sync::require_part(db, key, &op)?;
        if let Action::Post {
            body: Body::Rich(raw),
            ..
        }
        | Action::Edit {
            body: Body::Rich(raw),
            ..
        } = &op.action
        {
            structured::require_content(db, key, conversation, raw)?;
        }
        if let Action::Edit { target, .. } = Operation::from_bytes(raw)
            .map_err(|_| Error::InvalidEvent)?
            .action
        {
            let (_, own) = structured::account_context(db, key)?;
            match message(db, key, conversation, target, now(), own) {
                Ok(view) if !view.deleted && !view.view_once => (),
                Ok(_) | Err(Error::NotFound) => return Err(Error::Obsolete),
                Err(error) => return Err(error),
            }
        }
    }
    Ok(())
}
pub struct Page {
    pub messages: Vec<Message>,
    pub next: Option<i64>,
}
pub struct SearchHit {
    pub conversation: Id,
    pub message: Message,
}
pub struct SearchPage {
    pub hits: Vec<SearchHit>,
    pub next: Option<i64>,
}
fn searchable(body: &Body, query: &str) -> Result<bool, Error> {
    let text = match body {
        Body::Text(v) => v.clone(),
        Body::File(v) => sigil_protocol::file::File::from_bytes(v)
            .map_err(|_| Error::InvalidStore)?
            .name
            .to_owned(),
        Body::Rich(v) => match sigil_protocol::text::Document::from_bytes(v)
            .map_err(|_| Error::InvalidStore)?
        {
            sigil_protocol::text::Document::Text(t) => t.body().to_owned(),
            sigil_protocol::text::Document::Card(c) => c.body().map_err(|_| Error::InvalidStore)?,
            sigil_protocol::text::Document::Composition(c) => {
                c.body().map_err(|_| Error::InvalidStore)?
            }
            sigil_protocol::text::Document::Action(_) => return Ok(false),
        },
    };
    Ok(text.to_lowercase().contains(query))
}
impl ClientStore {
    /// At most 64 candidates per page; the cursor also advances over filtered content.
    pub fn conversation_page(
        &mut self,
        conversation: Id,
        after: Option<i64>,
        thread: Option<&Reference>,
        search: Option<&str>,
        now: u64,
    ) -> Result<Page, Error> {
        let page =
            self.scan_conversations(Some(conversation), after, thread, search, now, false)?;
        Ok(Page {
            messages: page.hits.into_iter().map(|v| v.message).collect(),
            next: page.next,
        })
    }
    pub fn search_conversations(
        &mut self,
        query: &str,
        after: Option<i64>,
        now: u64,
    ) -> Result<SearchPage, Error> {
        self.scan_conversations(None, after, None, Some(query), now, false)
    }
    pub fn recent_search_conversations(
        &mut self,
        query: &str,
        before: Option<i64>,
        now: u64,
    ) -> Result<SearchPage, Error> {
        self.scan_conversations(None, before, None, Some(query), now, true)
    }
    pub fn conversation_position(
        &self,
        conversation: Id,
        reference: &Reference,
    ) -> Result<i64, Error> {
        let entry =
            original(&self.db, &self.key, &conversation, reference)?.ok_or(Error::NotFound)?;
        if !matches!(entry.operation.action, Action::Post { .. }) {
            return Err(Error::InvalidEvent);
        }
        self.db
            .query_row(
                "SELECT rowid FROM conversation_ops WHERE id=?1",
                [entry_id(&self.key, &entry)?.as_slice()],
                |r| r.get(0),
            )
            .map_err(Into::into)
    }
    /// Newest first, with a cursor over candidates including deleted messages.
    pub fn recent_conversation_page(
        &mut self,
        conversation: Id,
        before: Option<i64>,
        now: u64,
    ) -> Result<Page, Error> {
        let page = self.scan_conversations(Some(conversation), before, None, None, now, true)?;
        Ok(Page {
            messages: page.hits.into_iter().map(|v| v.message).collect(),
            next: page.next,
        })
    }
    pub fn recent_conversation_search(
        &mut self,
        conversation: Id,
        before: Option<i64>,
        query: &str,
        now: u64,
    ) -> Result<Page, Error> {
        let page =
            self.scan_conversations(Some(conversation), before, None, Some(query), now, true)?;
        Ok(Page {
            messages: page.hits.into_iter().map(|v| v.message).collect(),
            next: page.next,
        })
    }
    #[allow(clippy::too_many_arguments)]
    fn scan_conversations(
        &mut self,
        conversation: Option<Id>,
        after: Option<i64>,
        thread: Option<&Reference>,
        search: Option<&str>,
        now: u64,
        newest: bool,
    ) -> Result<SearchPage, Error> {
        if after.is_some_and(|v| v < 0) || search.is_some_and(|v| v.len() > 1024) {
            return Err(Error::Limit);
        }
        let now = time_floor(&self.db, &self.key, now)?;
        let (_, own) = crate::structured::account_context(&self.db, &self.key)?;
        let query = search.map(str::to_lowercase);
        let mut clears = std::collections::BTreeMap::new();
        let sql = if newest && conversation.is_none() {
            "SELECT rowid,id,CASE WHEN length(state)<=574000 THEN state END FROM conversation_ops WHERE ?1 IS NULL AND kind=0 AND rowid<?2 ORDER BY rowid DESC LIMIT 64"
        } else if newest {
            "SELECT rowid,id,CASE WHEN length(state)<=574000 THEN state END FROM conversation_ops WHERE scope=?1 AND kind=0 AND rowid<?2 ORDER BY rowid DESC LIMIT 64"
        } else if conversation.is_some() {
            "SELECT rowid,id,CASE WHEN length(state)<=574000 THEN state END FROM conversation_ops WHERE scope=?1 AND kind=0 AND rowid>?2 ORDER BY rowid LIMIT 64"
        } else {
            "SELECT rowid,id,CASE WHEN length(state)<=574000 THEN state END FROM conversation_ops WHERE ?1 IS NULL AND kind=0 AND rowid>?2 ORDER BY rowid LIMIT 64"
        };
        let at = conversation.map(|v| scope(&self.key, &v)).transpose()?;
        let mut stmt = self.db.prepare(sql)?;
        let mut rows = stmt.query((
            at.as_ref().map(Id::as_slice),
            after.unwrap_or(if newest { i64::MAX } else { 0 }),
        ))?;
        let mut hits = Vec::new();
        let mut next = None;
        let mut count = 0;
        while let Some(row) = rows.next()? {
            let sequence: i64 = row.get(0)?;
            let id: Vec<u8> = row.get(1)?;
            let raw: Vec<u8> = row.get(2)?;
            count += 1;
            next = Some(sequence);
            let e = open(
                &self.key,
                &id.try_into().map_err(|_| Error::InvalidStore)?,
                &raw,
            )?;
            if conversation.is_some_and(|v| v != e.conversation) {
                return Err(Error::InvalidStore);
            }
            if !matches!(e.operation.action, Action::Post { .. }) {
                continue;
            }
            if let Action::Post {
                body: Body::Rich(bytes),
                ..
            } = &e.operation.action
            {
                if matches!(
                    sigil_protocol::text::Document::from_bytes(bytes)
                        .map_err(|_| Error::InvalidStore)?,
                    sigil_protocol::text::Document::Action(_)
                ) {
                    continue;
                }
            }
            if let std::collections::btree_map::Entry::Vacant(entry) = clears.entry(e.conversation)
            {
                entry.insert(clear_context(&self.db, &self.key, e.conversation, own)?);
            }
            let mut view = message_after_clear(
                &self.db,
                &self.key,
                e.conversation,
                Reference {
                    author: e.author,
                    message: e.operation.id,
                },
                now,
                own,
                &clears[&e.conversation],
            )?;
            if view.deleted || thread.is_some_and(|t| view.thread.as_ref() != Some(t)) {
                continue;
            }
            if view.view_once {
                view.body = None;
            }
            if let Some(query) = &query {
                if !view
                    .body
                    .as_ref()
                    .map(|b| searchable(b, query))
                    .transpose()?
                    .unwrap_or(false)
                {
                    continue;
                }
            }
            hits.push(SearchHit {
                conversation: e.conversation,
                message: view,
            });
        }
        Ok(SearchPage {
            hits,
            next: if count == 64 { next } else { None },
        })
    }
}

pub(crate) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|v| v.as_secs())
        .unwrap_or(u64::MAX)
}
pub(crate) fn require_send(
    db: &Connection,
    key: &StorageKey,
    conversation: Id,
    content: Content<'_>,
    timestamp: u64,
    now: u64,
) -> Result<(), Error> {
    let Content::Conversation(raw) = content else {
        return Ok(());
    };
    let now = time_floor(db, key, now)?;
    let operation = Operation::from_bytes(raw).map_err(|_| Error::InvalidEvent)?;
    let session = connection::session_in(db, key)?.ok_or(Error::Unprepared)?;
    let server = session
        .address
        .split_once(':')
        .ok_or(Error::InvalidStore)?
        .1;
    let own = event::account_reference(server, &connection::decode_id(&session.account_id)?);
    match &operation.action {
        Action::Post {
            expires_at: Some(until),
            ..
        } if *until <= now => return Err(Error::Obsolete),
        Action::Edit { target, .. } => {
            if let Some(original) = original(db, key, &conversation, target)? {
                let Action::Post {
                    body, view_once, ..
                } = original.operation.action
                else {
                    return Err(Error::InvalidEvent);
                };
                if !body.editable()
                    || view_once
                    || message(db, key, conversation, target.clone(), now, own)?.deleted
                {
                    return Err(Error::Obsolete);
                }
            }
        }
        Action::Post {
            body: Body::Rich(raw),
            ..
        } => {
            if let sigil_protocol::text::Document::Action(action) =
                sigil_protocol::text::Document::from_bytes(raw).map_err(|_| Error::InvalidEvent)?
            {
                crate::structured::require_outgoing(
                    db,
                    key,
                    recovery::account_scope(server, connection::decode_id(&session.account_id)?)?,
                    conversation,
                    &action,
                )?;
            }
        }
        Action::Typing { until, .. } if *until <= now || timestamp.saturating_add(30) <= now => {
            return Err(Error::Obsolete)
        }
        Action::Presence { until, .. } if *until <= now || timestamp.saturating_add(120) <= now => {
            return Err(Error::Obsolete)
        }
        Action::Typing { .. } if !preferences(db, key, conversation, own)?.typing_indicators => {
            return Err(Error::Obsolete)
        }
        Action::Presence { .. } if !preferences(db, key, conversation, own)?.presence_sharing => {
            return Err(Error::Obsolete)
        }
        Action::Receipt { read: true, .. }
            if !preferences(db, key, conversation, own)?.read_receipts =>
        {
            return Err(Error::Obsolete)
        }
        _ => {}
    }
    Ok(())
}
pub(crate) fn check_send(
    db: &Connection,
    key: &StorageKey,
    raw: &[u8],
    now: u64,
) -> Result<(), Error> {
    crate::calls::check_retained(db, key, raw, now)?;
    require_payload(db, key, raw, now, true)?;
    if let Ok(e) = sigil_protocol::event::Direct::from_bytes(raw) {
        require_send(db, key, e.conversation, e.content, e.timestamp, now)?;
    }
    if let Ok(e) = sigil_protocol::event::Group::from_bytes(raw) {
        require_send(db, key, e.group, e.content, e.timestamp, now)?;
    }
    Ok(())
}
pub(crate) fn cancel_outgoing(
    tx: &Transaction<'_>,
    key: &StorageKey,
    session: Id,
    now: u64,
) -> Result<(), Error> {
    let rows=tx.prepare("SELECT id,content FROM outbox WHERE session=?1 AND packet IS NOT NULL ORDER BY rowid LIMIT 16")?.query_map([session.as_slice()],|r|Ok((r.get::<_,Vec<u8>>(0)?,r.get::<_,Option<Vec<u8>>>(1)?)))?.collect::<Result<Vec<_>,_>>()?;
    for (id, raw) in rows {
        let Some(raw) = raw else { continue };
        let id: Id = id.try_into().map_err(|_| Error::InvalidStore)?;
        let bytes = key.open(&raw, &binding(9, &session, &id))?;
        match check_send(tx, key, &bytes, now) {
            Ok(()) => {}
            Err(Error::Obsolete) => {
                mark_cancelled(tx, key, &session, &id)?;
                tx.execute(
                    "UPDATE outbox SET packet=NULL WHERE session=?1 AND id=?2",
                    (session.as_slice(), id.as_slice()),
                )?;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
pub(crate) fn mark_cancelled(
    tx: &Transaction<'_>,
    key: &StorageKey,
    session: &Id,
    id: &Id,
) -> Result<(), Error> {
    tx.execute(
        "INSERT INTO conversation_cancelled VALUES(?1,?2,?3) ON CONFLICT(session,id) DO NOTHING",
        (
            session.as_slice(),
            id.as_slice(),
            key.seal(b"cancelled", &binding(94, session, id))?,
        ),
    )?;
    Ok(())
}
pub(crate) fn cancelled(
    db: &Connection,
    key: &StorageKey,
    session: &Id,
    id: &Id,
) -> Result<bool, Error> {
    let raw:Option<Vec<u8>>=db.query_row("SELECT CASE WHEN length(state)=45 THEN state END FROM conversation_cancelled WHERE session=?1 AND id=?2",(session.as_slice(),id.as_slice()),|r|r.get(0)).optional()?;
    match raw {
        Some(v) => {
            if key.open(&v, &binding(94, session, id))?.as_slice() != b"cancelled" {
                return Err(Error::InvalidStore);
            };
            Ok(true)
        }
        None => Ok(false),
    }
}

fn history_id(e: &Entry) -> Id {
    Sha256::digest(
        [
            b"Sigil/conversation-history/v0".as_slice(),
            &e.conversation,
            &e.author,
            &e.operation.id,
        ]
        .concat(),
    )
    .into()
}
fn archive_new(tx: &Transaction<'_>, key: &StorageKey, e: &Entry) -> Result<(), Error> {
    if !recovery::configured(tx)? || copyable(tx, key, e)? != Some(true) {
        return Ok(());
    }
    let snapshot = sigil_protocol::conversation::Snapshot {
        conversation: e.conversation,
        author: e.author,
        timestamp: e.timestamp,
        operation: e.operation.clone(),
    };
    let raw = snapshot.to_bytes().map_err(|_| Error::InvalidStore)?;
    let own = peers::parse(&peers::own(tx, key)?)?.binding;
    let scope = recovery::account_scope(&own.server, own.account)?;
    recovery::retain_new(
        tx,
        key,
        scope,
        &sigil_crypto::recovery::Record {
            id: history_id(e),
            revision: 1,
            conversation: e.conversation,
            author: e.identity,
            created_at: e.timestamp,
            direction: if e.author == event::account(&own) {
                sigil_crypto::recovery::Direction::Outgoing
            } else {
                sigil_crypto::recovery::Direction::Incoming
            },
            content: sigil_crypto::recovery::Content::Conversation(Zeroizing::new(raw)),
        },
    )?;
    if matches!(e.operation.action, Action::Post { .. }) {
        let target = Reference {
            author: e.author,
            message: e.operation.id,
        };
        visit(tx,key,"SELECT id,CASE WHEN length(state)<=574000 THEN state END FROM conversation_ops WHERE target=?1 AND kind=3 ORDER BY id",&target_index(key,&e.conversation,&target)?,|edit|{
            if matches!(edit.operation.action,Action::Edit{..}) {archive_new(tx,key,&edit)?;}
            Ok(())
        })?;
    }
    Ok(())
}
pub(crate) fn recovery_policy(
    db: &Connection,
    key: &StorageKey,
) -> Result<recovery::RecoveryPolicy, Error> {
    if !db.query_row(
        "SELECT EXISTS(SELECT 1 FROM conversation_ops WHERE kind=1)",
        [],
        |r| r.get::<_, bool>(0),
    )? {
        return Ok(recovery::RecoveryPolicy::default());
    }
    let (_, own) = crate::structured::account_context(db, key)?;
    Ok(recovery::RecoveryPolicy {
        history_days: preferences(db, key, [0; 32], own)?.recovery_history_days,
    })
}
pub(crate) fn backfill_recovery(
    tx: &Transaction<'_>,
    key: &StorageKey,
    after: i64,
) -> Result<(usize, i64), Error> {
    let mut query = tx.prepare("SELECT rowid,id,CASE WHEN length(state)<=574000 THEN state END FROM conversation_ops WHERE rowid>?1 ORDER BY rowid LIMIT 16")?;
    let mut rows = query.query([after])?;
    let mut count = 0;
    let mut cursor = after;
    while let Some(row) = rows.next()? {
        cursor = row.get(0)?;
        let id: Vec<u8> = row.get(1)?;
        let raw: Vec<u8> = row.get(2)?;
        let e = open(key, &id.try_into().map_err(|_| Error::InvalidStore)?, &raw)?;
        archive_new(tx, key, &e)?;
        retention::backfill_link(tx, key, &e)?;
        count += 1;
    }
    Ok((count, cursor))
}
pub(crate) fn restore(
    tx: &Transaction<'_>,
    key: &StorageKey,
    record: &sigil_crypto::recovery::Record,
) -> Result<(), Error> {
    if let sigil_crypto::recovery::Content::HistoryLink {
        record: legacy,
        author,
        message,
    } = record.content
    {
        if record.id != retention::link_id(legacy) {
            return Err(Error::InvalidStore);
        }
        return retention::link(tx, key, legacy, record.conversation, author, message);
    }
    let (raw, redacted) = match &record.content {
        sigil_crypto::recovery::Content::Conversation(raw) => (raw, None),
        sigil_crypto::recovery::Content::Redacted { snapshot, original } => {
            (snapshot, Some(*original))
        }
        _ => return Ok(()),
    };
    let s =
        sigil_protocol::conversation::Snapshot::from_bytes(raw).map_err(|_| Error::InvalidStore)?;
    let e = Entry {
        conversation: s.conversation,
        author: s.author,
        identity: record.author,
        timestamp: s.timestamp,
        seen: now(),
        operation: s.operation,
    };
    if e.conversation != record.conversation
        || e.timestamp != record.created_at
        || history_id(&e) != record.id
    {
        return Err(Error::InvalidStore);
    }
    if e.author == crate::structured::account_context(tx, key)?.1 {
        observe(tx, key, e.operation.version.counter)?;
    }
    if let Some(digest) = redacted {
        retention::accept_redaction(tx, key, e, digest)
    } else {
        ingest_only(tx, key, &e)?;
        structured(tx, key, &e)
    }
}

pub(crate) fn observe_expiry(
    db: &Connection,
    key: &StorageKey,
    conversation: Id,
    reference: Reference,
    now: u64,
) -> Result<(), Error> {
    if original(db, key, &conversation, &reference)?.is_some_and(|e| {
        matches!(
            e.operation.action,
            Action::Post {
                expires_at: Some(_),
                ..
            }
        )
    }) {
        time_floor(db, key, now)?;
    }
    Ok(())
}
pub(crate) fn require_reference(
    db: &Connection,
    key: &StorageKey,
    conversation: Id,
    reference: Reference,
) -> Result<(), Error> {
    let (.., own) = crate::structured::account_context(db, key)?;
    match message(db, key, conversation, reference, now(), own) {
        Ok(m) if m.deleted || m.view_once => Err(Error::Obsolete),
        Ok(_) | Err(Error::NotFound) => Ok(()),
        Err(e) => Err(e),
    }
}

pub(crate) fn require_history(
    db: &Connection,
    key: &StorageKey,
    event: &sigil_protocol::event::Group<'_>,
    now: u64,
) -> Result<(), Error> {
    require_live(
        db,
        key,
        event.group,
        event.sender,
        event.message,
        now,
        false,
    )?;
    if let Content::Conversation(raw) = event.content {
        let op = Operation::from_bytes(raw).map_err(|_| Error::InvalidEvent)?;
        if let Action::Edit { target, .. } = &op.action {
            let p = original(db, key, &event.group, target)?.ok_or(Error::Obsolete)?;
            if !matches!(
                p.operation.action,
                Action::Post {
                    expires_at: None,
                    view_once: false,
                    ..
                }
            ) {
                return Err(Error::Obsolete);
            }
            require_reference(db, key, event.group, target.clone())?;
        }
    }
    Ok(())
}

pub(crate) fn source_for_device(
    db: &Connection,
    key: &StorageKey,
    own: &[u8],
    device: Id,
) -> Result<(Id, Id), Error> {
    let own = peers::parse(own)?.binding;
    if peers::fingerprint(&own)? == device {
        return Ok((event::account(&own), own.identity));
    }
    let mut stmt = db.prepare("SELECT id FROM peers")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let id: Vec<u8> = row.get(0)?;
        let p = peers::known(db, key, &id.try_into().map_err(|_| Error::InvalidStore)?)?;
        if p.fingerprint == device {
            return Ok((event::account(&p.binding), p.binding.identity));
        }
    }
    Err(Error::InvalidStore)
}
pub(crate) fn migrate(tx: &Transaction<'_>, key: &StorageKey) -> Result<(), Error> {
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
        let mut stmt = tx.prepare(sql)?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let session: Vec<u8> = row.get(0)?;
            let id: Vec<u8> = row.get(1)?;
            let raw: Vec<u8> = row.get(2)?;
            let raw = key.open(
                &raw,
                &binding(
                    if outgoing { 9 } else { 2 },
                    &session.try_into().map_err(|_| Error::InvalidStore)?,
                    &id,
                ),
            )?;
            let Ok(e) = sigil_protocol::event::Direct::from_bytes(&raw) else {
                continue;
            };
            let peer: Vec<u8> = row.get(3)?;
            let peer = peers::known(tx, key, &peer.try_into().map_err(|_| Error::InvalidStore)?)?;
            let own_binding = peers::parse(&own)?.binding;
            let fp = peers::fingerprint(&own_binding)?;
            let (sender, recipient) = if outgoing {
                (fp, peer.fingerprint)
            } else {
                (peer.fingerprint, fp)
            };
            if e.sender != sender
                || e.recipient != recipient
                || e.conversation
                    != event::direct_reference(
                        event::account(&own_binding),
                        event::account(&peer.binding),
                    )
            {
                continue;
            }
            let source = source_for_device(tx, key, &own, e.sender)?;
            native(
                tx,
                key,
                e.conversation,
                source,
                (e.sender, e.message, event::event_history_id(&e, &source.1)),
                e.timestamp,
                e.content,
            )?;
        }
    }
    groups::migrate_conversations(tx, key)
}
#[derive(Debug, PartialEq, Eq)]
pub enum DeliveryState {
    Queued,
    Pending,
    ServerAccepted,
    Expired,
    Cancelled,
}
impl ClientStore {
    pub fn operation_delivery_state(
        &self,
        peer: Id,
        operation: Id,
    ) -> Result<DeliveryState, Error> {
        let id: Id = Sha256::digest(
            [
                b"Sigil/conversation-delivery/v0".as_slice(),
                &operation,
                &peer,
            ]
            .concat(),
        )
        .into();
        if cancelled(&self.db, &self.key, &[0; 32], &id)? {
            return Ok(DeliveryState::Cancelled);
        }
        if self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM send_intents WHERE id=?1)",
            [id.as_slice()],
            |r| r.get::<_, bool>(0),
        )? {
            return Ok(DeliveryState::Queued);
        }
        let session: Vec<u8> = self
            .db
            .query_row(
                "SELECT session FROM deliveries WHERE id=?1",
                [id.as_slice()],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(Error::NotFound)?;
        let session: Id = session.try_into().map_err(|_| Error::InvalidStore)?;
        if cancelled(&self.db, &self.key, &session, &id)? {
            return Ok(DeliveryState::Cancelled);
        }
        if self.delivery_expired(session, id)? {
            return Ok(DeliveryState::Expired);
        }
        if self.delivery_receipt(session, id)?.is_some() {
            return Ok(DeliveryState::ServerAccepted);
        }
        Ok(DeliveryState::Pending)
    }
}

pub(crate) fn time_floor(db: &Connection, key: &StorageKey, time: u64) -> Result<u64, Error> {
    if db.is_autocommit() {
        let tx = Transaction::new_unchecked(db, TransactionBehavior::Immediate)?;
        let result = time_floor(&tx, key, time)?;
        tx.commit()?;
        return Ok(result);
    }
    if time == 0 || time > i64::MAX as u64 {
        return Err(Error::Expired);
    }
    let raw: Option<Vec<u8>> = db
        .query_row(
            "SELECT CASE WHEN length(state)=44 THEN state END FROM conversation_time WHERE id=1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let previous = raw
        .map(|v| {
            key.open(&v, &binding(96, &[0; 32], b"time"))?
                .as_slice()
                .try_into()
                .map(u64::from_be_bytes)
                .map_err(|_| Error::InvalidStore)
        })
        .transpose()?
        .unwrap_or(0);
    if time > previous {
        db.execute("INSERT INTO conversation_time VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET state=excluded.state",[key.seal(&time.to_be_bytes(),&binding(96,&[0;32],b"time"))?])?;
    }
    Ok(time.max(previous))
}

pub(crate) fn archived_file_live(
    db: &Connection,
    key: &StorageKey,
    record: &sigil_crypto::recovery::Record,
) -> Result<bool, Error> {
    let sigil_crypto::recovery::Content::Conversation(raw) = &record.content else {
        return Ok(true);
    };
    let s =
        sigil_protocol::conversation::Snapshot::from_bytes(raw).map_err(|_| Error::InvalidStore)?;
    let reference = target(&s.operation.action, s.author, s.operation.id);
    let session = connection::session_in(db, key)?.ok_or(Error::Unprepared)?;
    let own = event::account_reference(
        session
            .address
            .split_once(':')
            .ok_or(Error::InvalidStore)?
            .1,
        &connection::decode_id(&session.account_id)?,
    );
    let view = match message(db, key, s.conversation, reference, now(), own) {
        Ok(v) => v,
        Err(Error::NotFound) => return Ok(false),
        Err(e) => return Err(e),
    };
    let expected = match s.operation.action {
        Action::Post {
            body: Body::File(v),
            ..
        }
        | Action::Edit {
            body: Body::File(v),
            ..
        } => v,
        _ => return Ok(false),
    };
    Ok(matches!(view.body,Some(Body::File(v)) if v==expected) && !view.deleted && !view.view_once)
}

pub(crate) fn file_message(
    db: &Connection,
    key: &StorageKey,
    conversation: Id,
    reference: Reference,
    now: u64,
) -> Result<Message, Error> {
    let session = connection::session_in(db, key)?.ok_or(Error::Unprepared)?;
    let own = event::account_reference(
        session
            .address
            .split_once(':')
            .ok_or(Error::InvalidStore)?
            .1,
        &connection::decode_id(&session.account_id)?,
    );
    let result = message(db, key, conversation, reference, now, own)?;
    if result.deleted {
        return Err(Error::Obsolete);
    }
    if !matches!(result.body, Some(Body::File(_))) {
        return Err(Error::InvalidEvent);
    }
    Ok(result)
}
#[derive(Clone, Serialize, Deserialize)]
pub enum Destination {
    Peer(Id),
    Group(Id),
}
pub struct FilePost {
    pub id: Id,
    pub timestamp: u64,
    pub reply: Option<Reference>,
    pub thread: Option<Reference>,
    pub expires_at: Option<u64>,
    pub view_once: bool,
}
