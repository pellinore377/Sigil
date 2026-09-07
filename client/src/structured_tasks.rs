//! Task completions form an observed set: an undo removes only its author's
//! exact completion. Concurrent completions cannot undo each other.
use super::*;
use sigil_protocol::text::structured::ListMode;

pub(crate) const MIGRATION: &str = "
CREATE TABLE structured_task_counts(item BLOB PRIMARY KEY,content BLOB NOT NULL);
CREATE TABLE structured_task_completions(card BLOB NOT NULL REFERENCES structured_cards(id),item BLOB NOT NULL,register_id BLOB PRIMARY KEY,active INTEGER NOT NULL CHECK(active IN(0,1)),content BLOB NOT NULL);
CREATE INDEX structured_task_active ON structured_task_completions(card,item,register_id) WHERE active=1;
PRAGMA user_version=53;";

pub struct TaskState {
    pub item: Id,
    pub completed: bool,
    pub initially_completed: bool,
    pub active_completions: u64,
}
pub struct TaskCompletion {
    pub operation: Id,
    pub actor: Id,
    pub created_at: u64,
    pub undo_until: u64,
}
pub struct TaskPage {
    pub completions: Vec<TaskCompletion>,
    /// Opaque local cursor, valid only for this card/item. Restart a scan to see
    /// concurrent insertions/removals; separate pages are not one fixed snapshot.
    pub next: Option<Id>,
    pub total: u64,
}
fn count(db: &Connection, key: &StorageKey, item: &Id) -> Result<u64, Error> {
    let bytes: Option<Vec<u8>> = db.query_row("SELECT CASE WHEN length(content)=44 THEN content END FROM structured_task_counts WHERE item=?1", [item.as_slice()], |r|r.get(0)).optional()?;
    let Some(bytes) = bytes else {
        return Ok(0);
    };
    let raw = key.open(&bytes, &aad(6, item))?;
    let value = u64::from_be_bytes(raw.as_slice().try_into().map_err(|_| Error::InvalidStore)?);
    if value > i64::MAX as u64 {
        return Err(Error::InvalidStore);
    }
    Ok(value)
}
fn fields(card: &Id, item: &Id, active: bool) -> Vec<u8> {
    [card.as_slice(), item, &[u8::from(active)]].concat()
}
pub(super) fn apply(
    tx: &Transaction<'_>,
    key: &StorageKey,
    node: &Node,
    prior: Option<&Node>,
) -> Result<(), Error> {
    let (item, active) = match node.action.change {
        Change::Complete { item } => (item, true),
        Change::Undo { item, .. } => (item, false),
        _ => return Ok(()),
    };
    let item = register_index(key, &node.card, Register::Item(item))?;
    let old: Option<(bool,Vec<u8>)> = tx.query_row("SELECT active,CASE WHEN length(content)=101 THEN content END FROM structured_task_completions WHERE register_id=?1 AND card=?2 AND item=?3",(node.register.as_slice(),node.card.as_slice(),item.as_slice()),|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    let was_active = match (old, prior) {
        (None, None) if active => false,
        (Some((old, bytes)), Some(prior)) => {
            if key.open(&bytes, &aad(7, &node.register))?.as_slice()
                != fields(&node.card, &item, old)
                || old != matches!(prior.action.change, Change::Complete { .. })
            {
                return Err(Error::InvalidStore);
            }
            old
        }
        _ => return Err(Error::InvalidStore),
    };
    let old_count = count(tx, key, &item)?;
    let new_count = match (was_active, active) {
        (false, true) => old_count
            .checked_add(1)
            .filter(|n| *n <= i64::MAX as u64)
            .ok_or(Error::Limit)?,
        (true, false) => old_count.checked_sub(1).ok_or(Error::InvalidStore)?,
        _ => old_count,
    };
    tx.execute("INSERT INTO structured_task_counts VALUES(?1,?2) ON CONFLICT(item) DO UPDATE SET content=excluded.content",(item.as_slice(),key.seal(&new_count.to_be_bytes(),&aad(6,&item))?))?;
    tx.execute("INSERT INTO structured_task_completions VALUES(?1,?2,?3,?4,?5) ON CONFLICT(register_id) DO UPDATE SET active=excluded.active,content=excluded.content",(node.card.as_slice(),item.as_slice(),node.register.as_slice(),active,key.seal(&fields(&node.card,&item,active),&aad(7,&node.register))?))?;
    Ok(())
}
pub(super) fn states(
    db: &Connection,
    key: &StorageKey,
    index: &Id,
    card: &Card,
) -> Result<Vec<TaskState>, Error> {
    let Construct::Checklist(list) = &card.content else {
        return Err(Error::InvalidStore);
    };
    list.items
        .iter()
        .map(|item| {
            let index = register_index(key, index, Register::Item(item.id))?;
            let total = count(db, key, &index)?;
            Ok(TaskState {
                item: item.id,
                completed: item.checked || total > 0,
                initially_completed: item.checked,
                active_completions: total,
            })
        })
        .collect()
}
impl ClientStore {
    /// At most 64 attributed active completions. Caller compares its account
    /// reference and trusted current time with `undo_until` before offering undo.
    /// Incoming action timestamps remain application claims, not trusted clocks.
    pub fn task_completions(
        &self,
        conversation: Id,
        reference: Reference,
        item: Id,
        after: Option<Id>,
    ) -> Result<TaskPage, Error> {
        let tx = self.db.unchecked_transaction()?;
        let (scope, _) = account_context(&tx, &self.key)?;
        let (index, card) = visible_card(&tx, &self.key, &scope, &conversation, &reference)?;
        if !matches!(&card.content,Construct::Checklist(list) if list.mode == ListMode::Task && list.items.iter().any(|i|i.id==item))
        {
            return Err(Error::InvalidEvent);
        }
        let item_index = register_index(&self.key, &index, Register::Item(item))?;
        let total = count(&tx, &self.key, &item_index)?;
        let mut completions = Vec::new();
        let mut next = None;
        {
            let mut query = tx.prepare("SELECT register_id,CASE WHEN length(content)=101 THEN content END FROM structured_task_completions WHERE card=?1 AND item=?2 AND active=1 AND (?3 IS NULL OR register_id>?3) ORDER BY register_id LIMIT 65")?;
            let mut rows = query.query((
                index.as_slice(),
                item_index.as_slice(),
                after.as_ref().map(|v| v.as_slice()),
            ))?;
            let mut last = None;
            while let Some(row) = rows.next()? {
                if completions.len() == 64 {
                    next = last;
                    break;
                }
                let register: Vec<u8> = row.get(0)?;
                let register: Id = register.try_into().map_err(|_| Error::InvalidStore)?;
                let bytes: Vec<u8> = row.get(1)?;
                if self.key.open(&bytes, &aad(7, &register))?.as_slice()
                    != fields(&index, &item_index, true)
                {
                    return Err(Error::InvalidStore);
                }
                let head = head(&tx, &self.key, &index, &register)?.ok_or(Error::InvalidStore)?;
                if head.action.change != (Change::Complete { item }) || head.depth != 1 {
                    return Err(Error::InvalidStore);
                }
                completions.push(TaskCompletion {
                    operation: invalid(head.action.id())?,
                    actor: head.action.actor,
                    created_at: head.action.created_at,
                    undo_until: head
                        .action
                        .created_at
                        .checked_add(30)
                        .ok_or(Error::InvalidStore)?,
                });
                last = Some(register);
            }
        }
        if completions.len() as u64 > total
            || (after.is_none() && next.is_none() && completions.len() as u64 != total)
        {
            return Err(Error::InvalidStore);
        }
        tx.commit()?;
        Ok(TaskPage {
            completions,
            next,
            total,
        })
    }
}
