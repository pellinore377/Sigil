use super::*;
use sigil_protocol::text::poll_close::{self, Ballot, Closure, Page, PAGE_SIZE};

pub(crate) const MIGRATION: &str = "
CREATE TABLE structured_closed_pages(closure BLOB NOT NULL,number INTEGER NOT NULL,content BLOB NOT NULL,PRIMARY KEY(closure,number));
CREATE TABLE structured_closed_voters(closure BLOB NOT NULL,voter BLOB NOT NULL,content BLOB NOT NULL,PRIMARY KEY(closure,voter));
CREATE TABLE structured_closed_totals(closure BLOB PRIMARY KEY,content BLOB NOT NULL);
CREATE TABLE structured_close_drafts(card BLOB PRIMARY KEY,content BLOB NOT NULL);
CREATE TABLE structured_close_parts(card BLOB NOT NULL,number INTEGER NOT NULL,content BLOB NOT NULL,PRIMARY KEY(card,number));
CREATE TABLE structured_close_tree(card BLOB NOT NULL,level INTEGER NOT NULL,number INTEGER NOT NULL,content BLOB NOT NULL,PRIMARY KEY(card,level,number));";
fn context(kind: u8, index: &Id, number: u64) -> Vec<u8> {
    [
        b"Sigil/poll-close-store/v1".as_slice(),
        &[kind],
        index,
        &number.to_be_bytes(),
    ]
    .concat()
}
fn voter(key: &StorageKey, close: &Id, actor: &Id) -> Result<Id, Error> {
    Ok(key.commitment(
        &[close.as_slice(), actor].concat(),
        b"Sigil/closed-voter-index/v1",
    )?)
}
fn sql(value: u64) -> Result<i64, Error> {
    i64::try_from(value).map_err(|_| Error::Limit)
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Counts {
    counts: Vec<u64>,
    voters: u64,
    pages: u64,
}
fn counts(
    db: &Connection,
    key: &StorageKey,
    close: &Id,
    closure: &Closure,
    options: usize,
) -> Result<Counts, Error> {
    let bytes: Option<Vec<u8>> = db.query_row("SELECT CASE WHEN length(content)<=4096 THEN content END FROM structured_closed_totals WHERE closure=?1", [close.as_slice()], |r| r.get(0)).optional()?;
    let counts: Counts = if let Some(bytes) = bytes {
        serde_json::from_slice(&key.open(&bytes, &context(0, close, 0))?)
            .map_err(|_| Error::InvalidStore)?
    } else {
        Counts {
            counts: vec![0; options],
            voters: 0,
            pages: 0,
        }
    };
    if counts.counts.len() != options
        || counts.voters > closure.voters
        || counts.pages > closure.pages()
        || counts.counts.iter().any(|count| *count > counts.voters)
        || (counts.pages == closure.pages() && counts.voters != closure.voters)
    {
        return Err(Error::InvalidStore);
    }
    Ok(counts)
}
pub(super) fn closed(db: &Connection, key: &StorageKey, card: &Id) -> Result<Option<Node>, Error> {
    head(
        db,
        key,
        card,
        &register_index(key, card, Register::PollClose)?,
    )
}
pub(super) fn validate(
    db: &Connection,
    key: &StorageKey,
    node: &Node,
    card: &Card,
    dependencies: &[(Id, Node)],
) -> Result<bool, Error> {
    let Change::PollPage(page) = &node.action.change else {
        return Ok(true);
    };
    let Some((_, close)) = dependencies.iter().find(|(id, _)| *id == page.closure) else {
        return Ok(false);
    };
    let Some(votes) = page
        .ballots
        .iter()
        .map(|ballot| {
            dependencies
                .iter()
                .find(|(id, _)| *id == ballot.action)
                .map(|(_, node)| &node.action)
        })
        .collect::<Option<Vec<_>>>()
    else {
        return Ok(false);
    };
    if page.validate_votes(card, &close.action, &votes).is_err() {
        return Ok(false);
    }
    let index = op_index(key, &node.card, &page.closure)?;
    let existing: Option<Vec<u8>> = db.query_row("SELECT CASE WHEN length(content)=68 THEN content END FROM structured_closed_pages WHERE closure=?1 AND number=?2", (index.as_slice(), sql(page.index)?), |r| r.get(0)).optional()?;
    if let Some(bytes) = existing {
        if key
            .open(&bytes, &context(1, &index, page.index))?
            .as_slice()
            != invalid(poll_close::leaf(page.index, &page.ballots))?
        {
            return Err(Error::InvalidStore);
        }
        return Ok(true);
    }
    for ballot in &page.ballots {
        if db.query_row(
            "SELECT EXISTS(SELECT 1 FROM structured_closed_voters WHERE closure=?1 AND voter=?2)",
            (
                index.as_slice(),
                voter(key, &index, &ballot.actor)?.as_slice(),
            ),
            |r| r.get::<_, bool>(0),
        )? {
            return Ok(false);
        }
    }
    Ok(true)
}
pub(super) fn apply(
    tx: &Transaction<'_>,
    key: &StorageKey,
    node: &Node,
    card: &Card,
    dependencies: &[(Id, Node)],
) -> Result<(), Error> {
    let Change::PollPage(page) = &node.action.change else {
        return Ok(());
    };
    let index = op_index(key, &node.card, &page.closure)?;
    if tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM structured_closed_pages WHERE closure=?1 AND number=?2)",
        (index.as_slice(), sql(page.index)?),
        |r| r.get::<_, bool>(0),
    )? {
        return Ok(());
    }
    let close = dependencies
        .iter()
        .find(|(id, _)| *id == page.closure)
        .ok_or(Error::InvalidStore)?;
    let Change::ClosePoll(closure) = &close.1.action.change else {
        return Err(Error::InvalidStore);
    };
    let Construct::Poll(poll) = &card.content else {
        return Err(Error::InvalidStore);
    };
    let mut totals = counts(tx, key, &index, closure, poll.options.len())?;
    for ballot in &page.ballots {
        let vote = dependencies
            .iter()
            .find(|(id, _)| *id == ballot.action)
            .ok_or(Error::InvalidStore)?;
        let Change::Vote { choices } = &vote.1.action.change else {
            return Err(Error::InvalidStore);
        };
        for choice in choices {
            let position = poll
                .options
                .iter()
                .position(|option| option.id == *choice)
                .ok_or(Error::InvalidStore)?;
            totals.counts[position] = totals.counts[position].checked_add(1).ok_or(Error::Limit)?;
        }
        totals.voters = totals.voters.checked_add(1).ok_or(Error::Limit)?;
        let actor = voter(key, &index, &ballot.actor)?;
        tx.execute(
            "INSERT INTO structured_closed_voters VALUES(?1,?2,?3)",
            (
                index.as_slice(),
                actor.as_slice(),
                key.seal(&ballot.action, &context(2, &actor, 0))?,
            ),
        )?;
    }
    totals.pages = totals.pages.checked_add(1).ok_or(Error::Limit)?;
    tx.execute(
        "INSERT INTO structured_closed_pages VALUES(?1,?2,?3)",
        (
            index.as_slice(),
            sql(page.index)?,
            key.seal(
                &invalid(poll_close::leaf(page.index, &page.ballots))?,
                &context(1, &index, page.index),
            )?,
        ),
    )?;
    tx.execute("INSERT INTO structured_closed_totals VALUES(?1,?2) ON CONFLICT(closure) DO UPDATE SET content=excluded.content",
        (index.as_slice(), key.seal(&Zeroizing::new(serde_json::to_vec(&totals).map_err(|_| Error::InvalidStore)?), &context(0, &index, 0))?))?;
    counts(tx, key, &index, closure, poll.options.len())?;
    Ok(())
}
pub(super) fn state(
    db: &Connection,
    key: &StorageKey,
    card_index: &Id,
    card: &Card,
    viewer: &Id,
) -> Result<Option<PollState>, Error> {
    let Some(close) = closed(db, key, card_index)? else {
        return Ok(None);
    };
    let Change::ClosePoll(closure) = &close.action.change else {
        return Err(Error::InvalidStore);
    };
    let Construct::Poll(poll) = &card.content else {
        return Err(Error::InvalidStore);
    };
    let index = op_index(key, card_index, &invalid(close.action.id())?)?;
    let totals = counts(db, key, &index, closure, poll.options.len())?;
    let actor = voter(key, &index, viewer)?;
    let bytes: Option<Vec<u8>> = db.query_row("SELECT CASE WHEN length(content)=68 THEN content END FROM structured_closed_voters WHERE closure=?1 AND voter=?2", (index.as_slice(), actor.as_slice()), |r| r.get(0)).optional()?;
    let (choices, operation) = if let Some(bytes) = bytes {
        let id = key
            .open(&bytes, &context(2, &actor, 0))?
            .as_slice()
            .try_into()
            .map_err(|_| Error::InvalidStore)?;
        let vote =
            load_node(db, key, &op_index(key, card_index, &id)?)?.ok_or(Error::InvalidStore)?;
        if vote.status != 1 || vote.action.actor != *viewer || vote.card != *card_index {
            return Err(Error::InvalidStore);
        }
        let Change::Vote { choices } = vote.action.change else {
            return Err(Error::InvalidStore);
        };
        (choices, Some(id))
    } else {
        (Vec::new(), None)
    };
    let complete = totals.pages == closure.pages();
    Ok(Some(PollState {
        choices,
        operation,
        closed: true,
        pending_pages: closure.pages() - totals.pages,
        counts: complete.then(|| {
            poll.options
                .iter()
                .zip(totals.counts)
                .map(|(option, count)| (option.id, count))
                .collect()
        }),
        voters: complete.then_some(totals.voters),
    }))
}
fn tree_context(card: &Id, level: u8, number: u64) -> Vec<u8> {
    [context(5, card, number).as_slice(), &[level]].concat()
}
fn save_hash(
    tx: &Transaction<'_>,
    key: &StorageKey,
    card: &Id,
    level: u8,
    number: u64,
    value: &Id,
) -> Result<(), Error> {
    tx.execute(
        "INSERT INTO structured_close_tree VALUES(?1,?2,?3,?4)",
        (
            card.as_slice(),
            level,
            sql(number)?,
            key.seal(value, &tree_context(card, level, number))?,
        ),
    )?;
    Ok(())
}
fn tree_hash(
    db: &Connection,
    key: &StorageKey,
    card: &Id,
    level: u8,
    number: u64,
) -> Result<Id, Error> {
    let bytes: Vec<u8> = db.query_row("SELECT CASE WHEN length(content)=68 THEN content END FROM structured_close_tree WHERE card=?1 AND level=?2 AND number=?3", (card.as_slice(), level, sql(number)?), |r| r.get(0))?;
    key.open(&bytes, &tree_context(card, level, number))?
        .as_slice()
        .try_into()
        .map_err(|_| Error::InvalidStore)
}
fn save_part(
    tx: &Transaction<'_>,
    key: &StorageKey,
    card: &Id,
    number: u64,
    ballots: &mut [Ballot],
) -> Result<(), Error> {
    ballots.sort_by_key(|b| b.actor);
    let bytes = Zeroizing::new(serde_json::to_vec(ballots).map_err(|_| Error::InvalidStore)?);
    tx.execute(
        "INSERT INTO structured_close_parts VALUES(?1,?2,?3)",
        (
            card.as_slice(),
            sql(number)?,
            key.seal(&bytes, &context(4, card, number))?,
        ),
    )?;
    save_hash(
        tx,
        key,
        card,
        0,
        number,
        &invalid(poll_close::leaf(number, ballots))?,
    )
}
fn draft(db: &Connection, key: &StorageKey, index: &Id) -> Result<Option<Action>, Error> {
    let bytes: Option<Vec<u8>> = db.query_row("SELECT CASE WHEN length(content)<=61476 THEN content END FROM structured_close_drafts WHERE card=?1", [index.as_slice()], |r| r.get(0)).optional()?;
    bytes
        .map(|bytes| {
            Action::from_bytes(&key.open(&bytes, &context(3, index, 0))?)
                .map_err(|_| Error::InvalidStore)
        })
        .transpose()
}
pub(super) fn discard(tx: &Connection, index: &Id) -> Result<(), Error> {
    for table in [
        "structured_close_tree",
        "structured_close_parts",
        "structured_close_drafts",
    ] {
        tx.execute(
            &format!("DELETE FROM {table} WHERE card=?1"),
            [index.as_slice()],
        )?;
    }
    Ok(())
}
impl ClientStore {
    /// Freeze observed ballots atomically. Only one page is resident while building the snapshot.
    pub fn prepare_poll_close(
        &mut self,
        conversation: Id,
        reference: Reference,
        created_at: u64,
    ) -> Result<Action, Error> {
        let (scope, actor) = account_context(&self.db, &self.key)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (index, card) = visible_card(&tx, &self.key, &scope, &conversation, &reference)?;
        if actor != card.creator || !matches!(card.content, Construct::Poll(_)) {
            return Err(Error::InvalidEvent);
        }
        if let Some(action) = draft(&tx, &self.key, &index)? {
            if action.card != reference || action.actor != actor {
                return Err(Error::InvalidStore);
            }
            if action.created_at != created_at {
                return Err(Error::Conflict);
            }
            return Ok(action);
        }
        if closed(&tx, &self.key, &index)?.is_some() {
            return Err(Error::Obsolete);
        }
        if tx.query_row("SELECT count(*) FROM structured_close_drafts", [], |r| {
            r.get::<_, i64>(0)
        })? >= 16
        {
            return Err(Error::Limit);
        }
        let mut page = Vec::with_capacity(PAGE_SIZE);
        let mut voters = 0u64;
        let mut pages = 0u64;
        {
            let mut query = tx.prepare(
                "SELECT register_id FROM structured_heads WHERE card=?1 ORDER BY register_id",
            )?;
            let mut rows = query.query([index.as_slice()])?;
            while let Some(row) = rows.next()? {
                let register: Vec<u8> = row.get(0)?;
                let register = register
                    .as_slice()
                    .try_into()
                    .map_err(|_| Error::InvalidStore)?;
                let node = head(&tx, &self.key, &index, &register)?.ok_or(Error::InvalidStore)?;
                let Change::Vote { choices } = &node.action.change else {
                    continue;
                };
                if choices.is_empty() {
                    continue;
                }
                page.push(Ballot {
                    actor: node.action.actor,
                    action: invalid(node.action.id())?,
                });
                voters = voters
                    .checked_add(1)
                    .filter(|n| *n <= i64::MAX as u64)
                    .ok_or(Error::Limit)?;
                if page.len() == PAGE_SIZE {
                    save_part(&tx, &self.key, &index, pages, &mut page)?;
                    pages += 1;
                    page.clear();
                }
            }
        }
        if !page.is_empty() {
            save_part(&tx, &self.key, &index, pages, &mut page)?;
            pages += 1;
        }
        let mut width = pages;
        let mut level = 0;
        while width > 1 {
            for number in 0..width.div_ceil(2) {
                let left = tree_hash(&tx, &self.key, &index, level, number * 2)?;
                let right = tree_hash(
                    &tx,
                    &self.key,
                    &index,
                    level,
                    (number * 2 + 1).min(width - 1),
                )?;
                save_hash(
                    &tx,
                    &self.key,
                    &index,
                    level + 1,
                    number,
                    &poll_close::branch(&left, &right),
                )?;
            }
            width = width.div_ceil(2);
            level += 1;
        }
        let root = if pages == 0 {
            poll_close::empty_root()
        } else {
            tree_hash(&tx, &self.key, &index, level, 0)?
        };
        let action = Action {
            card: reference,
            actor,
            created_at,
            previous: None,
            revision: None,
            change: Change::ClosePoll(Closure { root, voters }),
        };
        invalid(action.validate_for(&card))?;
        let bytes = Zeroizing::new(invalid(action.to_bytes())?);
        tx.execute(
            "INSERT INTO structured_close_drafts VALUES(?1,?2)",
            (
                index.as_slice(),
                self.key.seal(&bytes, &context(3, &index, 0))?,
            ),
        )?;
        tx.commit()?;
        Ok(action)
    }
    pub fn poll_close_page(
        &self,
        conversation: Id,
        reference: Reference,
        number: u64,
    ) -> Result<Action, Error> {
        let (scope, actor) = account_context(&self.db, &self.key)?;
        let (index, card) = visible_card(&self.db, &self.key, &scope, &conversation, &reference)?;
        let close = draft(&self.db, &self.key, &index)?.ok_or(Error::Unprepared)?;
        if close.card != reference || close.actor != actor {
            return Err(Error::InvalidStore);
        }
        let Change::ClosePoll(closure) = &close.change else {
            return Err(Error::InvalidStore);
        };
        if number >= closure.pages() {
            return Err(Error::InvalidEvent);
        }
        let bytes: Vec<u8> = self.db.query_row("SELECT CASE WHEN length(content)<=16384 THEN content END FROM structured_close_parts WHERE card=?1 AND number=?2", (index.as_slice(), sql(number)?), |r| r.get(0))?;
        let ballots: Vec<Ballot> =
            serde_json::from_slice(&self.key.open(&bytes, &context(4, &index, number))?)
                .map_err(|_| Error::InvalidStore)?;
        let mut width = closure.pages();
        let mut at = number;
        let mut level = 0;
        let mut proof = Vec::new();
        while width > 1 {
            proof.push(tree_hash(
                &self.db,
                &self.key,
                &index,
                level,
                (at ^ 1).min(width - 1),
            )?);
            width = width.div_ceil(2);
            at /= 2;
            level += 1;
        }
        let page = Page {
            closure: invalid(close.id())?,
            index: number,
            ballots,
            proof,
        };
        page.verify(closure).map_err(|_| Error::InvalidStore)?;
        let action = Action {
            card: reference,
            actor,
            created_at: close.created_at,
            previous: None,
            revision: None,
            change: Change::PollPage(page),
        };
        invalid(action.validate_for(&card))?;
        invalid(action.to_bytes())?;
        Ok(action)
    }
    pub fn discard_poll_close(
        &mut self,
        conversation: Id,
        reference: Reference,
    ) -> Result<(), Error> {
        let (scope, _) = account_context(&self.db, &self.key)?;
        let index = card_index(&self.key, &scope, &conversation, &reference)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        discard(&tx, &index)?;
        tx.commit()?;
        Ok(())
    }
}
