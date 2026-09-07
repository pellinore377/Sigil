//! Derived structured state. Every write shares the message/history transaction.
use super::*;
use serde::{Deserialize, Serialize};
use sigil_protocol::text::{
    action::{Action, Change, Reference, Register},
    structured::{Card, Construct, Disclosure},
    Document,
};
#[path = "structured_tasks.rs"]
pub(crate) mod tasks;
pub use tasks::{TaskCompletion, TaskPage, TaskState};
#[path = "structured_recurring.rs"]
mod recurring;
pub use recurring::RecurringState;

pub(crate) const MIGRATION:&str="
CREATE TABLE structured_cards(id BLOB PRIMARY KEY,content BLOB NOT NULL);
CREATE TABLE structured_actions(id BLOB PRIMARY KEY,card BLOB NOT NULL,register_id BLOB NOT NULL,parent BLOB,status INTEGER NOT NULL CHECK(status IN(0,1,2)),content BLOB NOT NULL);
CREATE INDEX structured_actions_card ON structured_actions(card,status);
CREATE INDEX structured_actions_parent ON structured_actions(parent,status);
CREATE TABLE structured_work(id BLOB PRIMARY KEY,kind INTEGER NOT NULL CHECK(kind IN(0,1)),target BLOB NOT NULL,cursor INTEGER NOT NULL,content BLOB NOT NULL);
CREATE TABLE structured_heads(card BLOB NOT NULL REFERENCES structured_cards(id),register_id BLOB NOT NULL,content BLOB NOT NULL,PRIMARY KEY(card,register_id));
CREATE TABLE structured_totals(card BLOB PRIMARY KEY REFERENCES structured_cards(id),content BLOB NOT NULL);
CREATE TABLE structured_sources(record BLOB PRIMARY KEY,card BLOB NOT NULL REFERENCES structured_cards(id),live INTEGER NOT NULL CHECK(live IN(0,1)),content BLOB NOT NULL);
CREATE INDEX structured_sources_card ON structured_sources(card,live);
PRAGMA user_version=52;";
const BATCH: usize = 64;
#[cfg(test)]
#[path = "structured_tests.rs"]
mod tests;
fn aad(kind: u8, id: &Id) -> Vec<u8> {
    [b"Sigil/structured-store/v1".as_slice(), &[kind], id].concat()
}
fn card_index(
    key: &StorageKey,
    scope: &Id,
    conversation: &Id,
    reference: &Reference,
) -> Result<Id, Error> {
    Ok(key.commitment(
        &[
            scope.as_slice(),
            conversation,
            &reference.id,
            &reference.creator,
            &reference.digest,
        ]
        .concat(),
        b"Sigil/structured-card-index/v1",
    )?)
}
fn op_index(key: &StorageKey, card: &Id, id: &Id) -> Result<Id, Error> {
    Ok(key.commitment(
        &[card.as_slice(), id].concat(),
        b"Sigil/structured-op-index/v1",
    )?)
}
fn register_index(key: &StorageKey, card: &Id, register: Register) -> Result<Id, Error> {
    let (kind, id) = match register {
        Register::Item(id) => (0u8, id),
        Register::Ballot(id) => (1, id),
        Register::Completion(id) => (2, id),
        Register::Recurring(id) => (3, id),
    };
    Ok(key.commitment(
        &[card.as_slice(), &[kind], &id].concat(),
        b"Sigil/structured-register-index/v1",
    )?)
}
fn invalid<T>(result: Result<T, sigil_protocol::text::Error>) -> Result<T, Error> {
    result.map_err(|_| Error::InvalidEvent)
}
fn load_card(db: &Connection, key: &StorageKey, index: &Id) -> Result<Option<Card>, Error> {
    let bytes:Option<Vec<u8>>=db.query_row("SELECT CASE WHEN length(content)<=61476 THEN content END FROM structured_cards WHERE id=?1",[index.as_slice()],|r|r.get(0)).optional()?;
    bytes
        .map(|bytes| {
            Card::from_bytes(&key.open(&bytes, &aad(0, index))?).map_err(|_| Error::InvalidStore)
        })
        .transpose()
}
fn visible_card(
    db: &Connection,
    key: &StorageKey,
    scope: &Id,
    conversation: &Id,
    reference: &Reference,
) -> Result<(Id, Card), Error> {
    let index = card_index(key, scope, conversation, reference)?;
    let card = load_card(db, key, &index)?.ok_or(Error::Unprepared)?;
    if invalid(Reference::of(&card))? != *reference {
        return Err(Error::InvalidStore);
    }
    if !live(db, key, &index)? {
        return Err(Error::Obsolete);
    }
    Ok((index, card))
}
struct Node {
    action: Action,
    status: u8,
    depth: u64,
    card: Id,
    register: Id,
    parent: Option<Id>,
}
fn load_node(db: &Connection, key: &StorageKey, index: &Id) -> Result<Option<Node>, Error> {
    type Row = (Vec<u8>, Vec<u8>, Option<Vec<u8>>, u8, Vec<u8>);
    let row:Option<Row>=db.query_row("SELECT card,register_id,parent,status,CASE WHEN length(content)<=61485 THEN content END FROM structured_actions WHERE id=?1",[index.as_slice()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
    let Some((card, register, parent, status, bytes)) = row else {
        return Ok(None);
    };
    let card: Id = card.try_into().map_err(|_| Error::InvalidStore)?;
    let register: Id = register.try_into().map_err(|_| Error::InvalidStore)?;
    let parent = parent
        .map(|id| id.try_into().map_err(|_| Error::InvalidStore))
        .transpose()?;
    let bytes = key.open(&bytes, &aad(1, index))?;
    if bytes.len() < 10 || bytes[0] != status || status > 2 {
        return Err(Error::InvalidStore);
    }
    let depth = u64::from_be_bytes(bytes[1..9].try_into().map_err(|_| Error::InvalidStore)?);
    if (status == 1) != (depth > 0) || depth > i64::MAX as u64 {
        return Err(Error::InvalidStore);
    }
    let action = Action::from_bytes(&bytes[9..]).map_err(|_| Error::InvalidStore)?;
    if op_index(key, &card, &invalid(action.id())?)? != *index
        || register_index(key, &card, invalid(action.register())?)? != register
        || action
            .previous
            .map(|id| op_index(key, &card, &id))
            .transpose()?
            != parent
    {
        return Err(Error::InvalidStore);
    }
    Ok(Some(Node {
        action,
        status,
        depth,
        card,
        register,
        parent,
    }))
}
fn node_bytes(
    key: &StorageKey,
    index: &Id,
    action: &Action,
    status: u8,
    depth: u64,
) -> Result<Vec<u8>, Error> {
    let mut bytes = Zeroizing::new(vec![status]);
    bytes.extend_from_slice(&depth.to_be_bytes());
    bytes.extend_from_slice(&invalid(action.to_bytes())?);
    Ok(key.seal(&bytes, &aad(1, index))?)
}
fn head(
    db: &Connection,
    key: &StorageKey,
    card: &Id,
    register: &Id,
) -> Result<Option<Node>, Error> {
    let bytes:Option<Vec<u8>>=db.query_row("SELECT CASE WHEN length(content)=68 THEN content END FROM structured_heads WHERE card=?1 AND register_id=?2",(card.as_slice(),register.as_slice()),|r|r.get(0)).optional()?;
    let Some(bytes) = bytes else { return Ok(None) };
    let id: Id = key
        .open(&bytes, &aad(2, register))?
        .as_slice()
        .try_into()
        .map_err(|_| Error::InvalidStore)?;
    let node = load_node(db, key, &id)?.ok_or(Error::InvalidStore)?;
    if node.card != *card || node.register != *register || node.status != 1 {
        return Err(Error::InvalidStore);
    }
    Ok(Some(node))
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Totals {
    voters: u64,
    counts: Vec<u64>,
    pending: u64,
    rejected: u64,
}
fn options(card: &Card) -> usize {
    match &card.content {
        Construct::Poll(poll) => poll.options.len(),
        _ => 0,
    }
}
fn totals(db: &Connection, key: &StorageKey, index: &Id, options: usize) -> Result<Totals, Error> {
    let bytes:Vec<u8>=db.query_row("SELECT CASE WHEN length(content)<=4096 THEN content END FROM structured_totals WHERE card=?1",[index.as_slice()],|r|r.get(0))?;
    let value: Totals = serde_json::from_slice(&key.open(&bytes, &aad(3, index))?)
        .map_err(|_| Error::InvalidStore)?;
    if value.counts.len() != options || value.counts.iter().any(|n| *n > value.voters) {
        return Err(Error::InvalidStore);
    }
    Ok(value)
}
fn save_totals(
    tx: &Transaction<'_>,
    key: &StorageKey,
    index: &Id,
    value: &Totals,
) -> Result<(), Error> {
    let bytes = Zeroizing::new(serde_json::to_vec(value).map_err(|_| Error::InvalidStore)?);
    tx.execute("INSERT INTO structured_totals VALUES(?1,?2) ON CONFLICT(card) DO UPDATE SET content=excluded.content",(index.as_slice(),key.seal(&bytes,&aad(3,index))?))?;
    Ok(())
}
fn source(
    tx: &Transaction<'_>,
    key: &StorageKey,
    record: &Id,
    card: &Id,
    live: bool,
) -> Result<(), Error> {
    let old: Option<(Vec<u8>, bool, Vec<u8>)> = tx
        .query_row(
            "SELECT card,live,content FROM structured_sources WHERE record=?1",
            [record.as_slice()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    if let Some((old, was_live, bytes)) = old {
        if old != *card
            || key.open(&bytes, &aad(4, record))?.as_slice()
                != [card.as_slice(), &[u8::from(was_live)]].concat()
        {
            return Err(Error::InvalidStore);
        }
        if was_live == live || !was_live {
            return Ok(());
        }
    }
    tx.execute("INSERT INTO structured_sources VALUES(?1,?2,?3,?4) ON CONFLICT(record) DO UPDATE SET live=excluded.live,content=excluded.content",(record.as_slice(),card.as_slice(),live,key.seal(&[card.as_slice(),&[u8::from(live)]].concat(),&aad(4,record))?))?;
    Ok(())
}
fn live(db: &Connection, key: &StorageKey, card: &Id) -> Result<bool, Error> {
    let row: Option<(Vec<u8>, Vec<u8>)> = db
        .query_row(
            "SELECT record,content FROM structured_sources WHERE card=?1 AND live=1 LIMIT 1",
            [card.as_slice()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((record, bytes)) = row else {
        return Ok(false);
    };
    let record: Id = record.try_into().map_err(|_| Error::InvalidStore)?;
    if key.open(&bytes, &aad(4, &record))?.as_slice() != [card.as_slice(), &[1]].concat() {
        return Err(Error::InvalidStore);
    }
    Ok(true)
}
/// Called only with previously authenticated application content or explicitly
/// trusted owner-encrypted recovery records. Does not authenticate a sender itself.
pub(crate) fn ingest(
    tx: &Transaction<'_>,
    key: &StorageKey,
    scope: Id,
    conversation: Id,
    record: Id,
    bytes: &[u8],
) -> Result<(), Error> {
    match invalid(Document::from_bytes(bytes))? {
        Document::Text(_) => return Ok(()),
        Document::Card(card) => {
            let reference = invalid(Reference::of(&card))?;
            let index = card_index(key, &scope, &conversation, &reference)?;
            if let Some(prior) = load_card(tx, key, &index)? {
                if prior != card {
                    return Err(Error::Conflict);
                }
            } else {
                tx.execute(
                    "INSERT INTO structured_cards VALUES(?1,?2)",
                    (index.as_slice(), key.seal(bytes, &aad(0, &index))?),
                )?;
                let pending: i64 = tx.query_row(
                    "SELECT count(*) FROM structured_actions WHERE card=?1 AND status=0",
                    [index.as_slice()],
                    |r| r.get(0),
                )?;
                save_totals(
                    tx,
                    key,
                    &index,
                    &Totals {
                        counts: vec![0; options(&card)],
                        pending: pending as u64,
                        ..Default::default()
                    },
                )?;
                enqueue(tx, key, 0, &index)?;
            }
            source(
                tx,
                key,
                &record,
                &index,
                !recovery::record_deleted(tx, key, scope, record)?,
            )?;
        }
        Document::Action(action) => {
            let card = card_index(key, &scope, &conversation, &action.card)?;
            let id = op_index(key, &card, &invalid(action.id())?)?;
            if let Some(prior) = load_node(tx, key, &id)? {
                if prior.action != action || prior.card != card {
                    return Err(Error::Conflict);
                }
            } else {
                let register = register_index(key, &card, invalid(action.register())?)?;
                let parent = action
                    .previous
                    .map(|id| op_index(key, &card, &id))
                    .transpose()?;
                tx.execute(
                    "INSERT INTO structured_actions VALUES(?1,?2,?3,?4,0,?5)",
                    (
                        id.as_slice(),
                        card.as_slice(),
                        register.as_slice(),
                        parent.map(|id| id.to_vec()),
                        node_bytes(key, &id, &action, 0, 0)?,
                    ),
                )?;
                if let Some(object) = load_card(tx, key, &card)? {
                    let mut counts = totals(tx, key, &card, options(&object))?;
                    counts.pending = counts.pending.checked_add(1).ok_or(Error::Limit)?;
                    save_totals(tx, key, &card, &counts)?;
                }
                apply(tx, key, &id)?;
            }
        }
    }
    advance(tx, key, BATCH)?;
    Ok(())
}
pub(crate) fn archive(
    tx: &Transaction<'_>,
    key: &StorageKey,
    scope: Id,
    record: &sigil_crypto::recovery::Record,
) -> Result<(), Error> {
    match &record.content {
        sigil_crypto::recovery::Content::Rich(bytes) => {
            ingest(tx, key, scope, record.conversation, record.id, bytes)
        }
        sigil_crypto::recovery::Content::Deleted => {
            let card: Option<Vec<u8>> = tx
                .query_row(
                    "SELECT card FROM structured_sources WHERE record=?1",
                    [record.id.as_slice()],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(card) = card {
                source(
                    tx,
                    key,
                    &record.id,
                    &card.try_into().map_err(|_| Error::InvalidStore)?,
                    false,
                )?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}
fn apply(tx: &Transaction<'_>, key: &StorageKey, id: &Id) -> Result<(), Error> {
    let node = load_node(tx, key, id)?.ok_or(Error::InvalidStore)?;
    if node.status != 0 {
        return Ok(());
    }
    let Some(card) = load_card(tx, key, &node.card)? else {
        return Ok(());
    };
    let parent = node
        .parent
        .map(|id| load_node(tx, key, &id))
        .transpose()?
        .flatten();
    if node.parent.is_some() && parent.as_ref().is_none_or(|p| p.status == 0) {
        return Ok(());
    }
    let depth = node.action.validate_for(&card).and_then(|_| {
        if parent.as_ref().is_some_and(|p| p.status != 1) {
            return Err(sigil_protocol::text::Error::Invalid);
        }
        node.action
            .depth_after(parent.as_ref().map(|p| (&p.action, p.depth)))
    });
    let (status, depth) = match depth {
        Ok(depth) => (1u8, depth),
        Err(_) => (2, 0),
    };
    let mut counts = totals(tx, key, &node.card, options(&card))?;
    counts.pending = counts.pending.checked_sub(1).ok_or(Error::InvalidStore)?;
    if status == 2 {
        counts.rejected = counts.rejected.checked_add(1).ok_or(Error::Limit)?;
    }
    tx.execute(
        "UPDATE structured_actions SET status=?2,content=?3 WHERE id=?1",
        (
            id.as_slice(),
            status,
            node_bytes(key, id, &node.action, status, depth)?,
        ),
    )?;
    if status == 1 {
        let prior = head(tx, key, &node.card, &node.register)?;
        let wins = prior
            .as_ref()
            .map(|p| invalid(node.action.wins_over(depth, &p.action, p.depth)))
            .transpose()?
            .unwrap_or(true);
        if wins {
            tasks::apply(tx, key, &node, prior.as_ref())?;
            if let (Construct::Poll(poll), Change::Vote { choices }) =
                (&card.content, &node.action.change)
            {
                if let Some(prior) = &prior {
                    let Change::Vote { choices } = &prior.action.change else {
                        return Err(Error::InvalidStore);
                    };
                    if !choices.is_empty() {
                        counts.voters = counts.voters.checked_sub(1).ok_or(Error::InvalidStore)?;
                    }
                    for choice in choices {
                        let position = poll
                            .options
                            .iter()
                            .position(|option| &option.id == choice)
                            .ok_or(Error::InvalidStore)?;
                        counts.counts[position] = counts.counts[position]
                            .checked_sub(1)
                            .ok_or(Error::InvalidStore)?;
                    }
                }
                if !choices.is_empty() {
                    counts.voters = counts.voters.checked_add(1).ok_or(Error::Limit)?;
                }
                for choice in choices {
                    let position = poll
                        .options
                        .iter()
                        .position(|option| &option.id == choice)
                        .ok_or(Error::InvalidStore)?;
                    counts.counts[position] =
                        counts.counts[position].checked_add(1).ok_or(Error::Limit)?;
                }
            }
            tx.execute("INSERT INTO structured_heads VALUES(?1,?2,?3) ON CONFLICT(card,register_id) DO UPDATE SET content=excluded.content",(node.card.as_slice(),node.register.as_slice(),key.seal(id,&aad(2,&node.register))?))?;
        }
    }
    save_totals(tx, key, &node.card, &counts)?;
    enqueue(tx, key, 1, id)?;
    Ok(())
}
fn work_bytes(kind: u8, target: &Id, cursor: i64) -> Vec<u8> {
    [&[kind], target.as_slice(), &cursor.to_be_bytes()].concat()
}
fn enqueue(tx: &Transaction<'_>, key: &StorageKey, kind: u8, target: &Id) -> Result<(), Error> {
    let id = key.commitment(
        &[&[kind], target.as_slice()].concat(),
        b"Sigil/structured-work-index/v1",
    )?;
    tx.execute(
        "INSERT INTO structured_work VALUES(?1,?2,?3,0,?4) ON CONFLICT(id) DO NOTHING",
        (
            id.as_slice(),
            kind,
            target.as_slice(),
            key.seal(&work_bytes(kind, target, 0), &aad(5, &id))?,
        ),
    )?;
    Ok(())
}
fn advance(tx: &Transaction<'_>, key: &StorageKey, limit: usize) -> Result<usize, Error> {
    for skipped in 0..limit {
        type Work = (Vec<u8>, u8, Vec<u8>, i64, Vec<u8>);
        let work: Option<Work> = tx
            .query_row(
                "SELECT id,kind,target,cursor,content FROM structured_work ORDER BY rowid LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let Some((id, kind, target, cursor, bytes)) = work else {
            return Ok(skipped);
        };
        let id: Id = id.try_into().map_err(|_| Error::InvalidStore)?;
        let target: Id = target.try_into().map_err(|_| Error::InvalidStore)?;
        if kind > 1
            || cursor < 0
            || key.open(&bytes, &aad(5, &id))?.as_slice() != work_bytes(kind, &target, cursor)
            || key.commitment(
                &[&[kind], target.as_slice()].concat(),
                b"Sigil/structured-work-index/v1",
            )? != id
        {
            return Err(Error::InvalidStore);
        }
        let sql = if kind == 0 {
            "SELECT rowid,id FROM structured_actions WHERE card=?1 AND status=0 AND rowid>?2 ORDER BY rowid LIMIT ?3"
        } else {
            "SELECT rowid,id FROM structured_actions WHERE parent=?1 AND status=0 AND rowid>?2 ORDER BY rowid LIMIT ?3"
        };
        let mut statement = tx.prepare(sql)?;
        let rows: Vec<(i64, Vec<u8>)> = statement
            .query_map((target.as_slice(), cursor, (limit - skipped) as i64), |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<Result<_, _>>()?;
        tx.execute("DELETE FROM structured_work WHERE id=?1", [id.as_slice()])?;
        let Some((last, _)) = rows.last() else {
            continue;
        };
        for (_, id) in &rows {
            apply(
                tx,
                key,
                &id.as_slice().try_into().map_err(|_| Error::InvalidStore)?,
            )?;
        }
        // Reinsert at the end of the work queue so one large fan-out cannot starve other cards.
        tx.execute(
            "INSERT INTO structured_work VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO NOTHING",
            (
                id.as_slice(),
                kind,
                target.as_slice(),
                last,
                key.seal(&work_bytes(kind, &target, *last), &aad(5, &id))?,
            ),
        )?;
        return Ok(skipped + rows.len());
    }
    Ok(limit)
}
#[derive(Clone)]
pub struct CheckState {
    pub item: Id,
    pub checked: bool,
    pub operation: Option<Id>,
    pub actor: Option<Id>,
}
pub struct PollState {
    pub choices: Vec<Id>,
    pub operation: Option<Id>,
    pub counts: Option<Vec<(Id, u64)>>,
    pub voters: Option<u64>,
}
pub struct CardState {
    pub card: Card,
    pub checks: Vec<CheckState>,
    pub tasks: Vec<TaskState>,
    pub poll: Option<PollState>,
    pub pending: u64,
    pub rejected: u64,
}
pub(crate) fn account_context(db: &Connection, key: &StorageKey) -> Result<(Id, Id), Error> {
    let session = connection::session_in(db, key)?.ok_or(Error::Unprepared)?;
    let (_, server) = session.address.split_once(':').ok_or(Error::InvalidStore)?;
    let account = connection::decode_id(&session.account_id)?;
    Ok((
        recovery::account_scope(server, account)?,
        crate::event::account_reference(server, &account),
    ))
}
impl ClientStore {
    /// One bounded offline pass, including children unblocked by the prior pass.
    pub fn advance_structured_actions(&mut self) -> Result<usize, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let count = advance(&tx, &self.key, BATCH)?;
        tx.commit()?;
        Ok(count)
    }
    pub fn card_state(&self, conversation: Id, reference: Reference) -> Result<CardState, Error> {
        let tx = self.db.unchecked_transaction()?;
        let (scope, viewer) = account_context(&tx, &self.key)?;
        let (index, card) = visible_card(&tx, &self.key, &scope, &conversation, &reference)?;
        let mut checks = Vec::new();
        let mut task_states = Vec::new();
        let mut poll_state = None;
        let counts = totals(&tx, &self.key, &index, options(&card))?;
        match &card.content {
            Construct::Checklist(list)
                if list.mode == sigil_protocol::text::structured::ListMode::Task =>
            {
                task_states = tasks::states(&tx, &self.key, &index, &card)?;
            }
            Construct::Checklist(list)
                if matches!(
                    list.mode,
                    sigil_protocol::text::structured::ListMode::Recurring(_)
                ) => {}
            Construct::Checklist(list) => {
                for item in &list.items {
                    let register = register_index(&self.key, &index, Register::Item(item.id))?;
                    let prior = head(&tx, &self.key, &index, &register)?;
                    let (checked, operation, actor) = if let Some(prior) = prior {
                        let Change::Check { checked, .. } = prior.action.change else {
                            return Err(Error::InvalidStore);
                        };
                        (
                            checked,
                            Some(invalid(prior.action.id())?),
                            Some(prior.action.actor),
                        )
                    } else {
                        (item.checked, None, item.checked.then_some(card.creator))
                    };
                    checks.push(CheckState {
                        item: item.id,
                        checked,
                        operation,
                        actor,
                    });
                }
            }
            Construct::Poll(poll) => {
                let register = register_index(&self.key, &index, Register::Ballot(viewer))?;
                let prior = head(&tx, &self.key, &index, &register)?;
                let operation = prior.as_ref().map(|p| invalid(p.action.id())).transpose()?;
                let choices = match prior {
                    Some(Node {
                        action:
                            Action {
                                change: Change::Vote { choices },
                                ..
                            },
                        ..
                    }) => choices,
                    None => Vec::new(),
                    _ => return Err(Error::InvalidStore),
                };
                let show = poll.disclosure == Disclosure::Open || !choices.is_empty();
                poll_state = Some(PollState {
                    choices,
                    operation,
                    counts: show.then(|| {
                        poll.options
                            .iter()
                            .zip(counts.counts)
                            .map(|(o, n)| (o.id, n))
                            .collect()
                    }),
                    voters: show.then_some(counts.voters),
                });
            }
            Construct::Note(_) => {}
        }
        tx.commit()?;
        Ok(CardState {
            card,
            checks,
            tasks: task_states,
            poll: poll_state,
            pending: counts.pending,
            rejected: counts.rejected,
        })
    }
    pub(crate) fn require_action(&self, conversation: Id, action: &Action) -> Result<(), Error> {
        let (scope, actor) = account_context(&self.db, &self.key)?;
        if action.actor != actor {
            return Err(Error::InvalidEvent);
        }
        require_outgoing(&self.db, &self.key, scope, conversation, action)
    }
}
pub(crate) fn require_outgoing(
    db: &Connection,
    key: &StorageKey,
    scope: Id,
    conversation: Id,
    action: &Action,
) -> Result<(), Error> {
    let index = card_index(key, &scope, &conversation, &action.card)?;
    let card = load_card(db, key, &index)?.ok_or(Error::Unprepared)?;
    if !live(db, key, &index)? {
        return Err(Error::Obsolete);
    }
    invalid(action.validate_for(&card))?;
    let parent = action
        .previous
        .map(|id| load_node(db, key, &op_index(key, &index, &id)?)?.ok_or(Error::Unprepared))
        .transpose()?;
    if parent.as_ref().is_some_and(|p| p.status != 1) {
        return Err(Error::Unprepared);
    }
    invalid(action.depth_after(parent.as_ref().map(|p| (&p.action, p.depth))))?;
    Ok(())
}
