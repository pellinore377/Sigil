use super::*;

pub(crate) fn require_content(
    db: &Connection,
    key: &StorageKey,
    conversation: Id,
    bytes: &[u8],
) -> Result<(), Error> {
    if let Document::Action(action) = invalid(Document::from_bytes(bytes))? {
        let (scope, _) = account_context(db, key)?;
        let index = card_index(key, &scope, &conversation, &action.card)?;
        if load_card(db, key, &index)?.is_some() && !live(db, key, &index)? {
            return Err(Error::Obsolete);
        }
    }
    Ok(())
}

pub(crate) fn forget_sources(
    tx: &Transaction<'_>,
    key: &StorageKey,
    record: Id,
) -> Result<(), Error> {
    let cards = tx
        .prepare("SELECT card FROM structured_sources WHERE record=?1 LIMIT 65")?
        .query_map([record.as_slice()], |r| r.get::<_, Vec<u8>>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if cards.len() > 64 {
        return Err(Error::InvalidStore);
    }
    for card in cards {
        source(
            tx,
            key,
            &record,
            &card.try_into().map_err(|_| Error::InvalidStore)?,
            false,
        )?;
    }
    Ok(())
}

pub(crate) fn erase(
    tx: &Transaction<'_>,
    key: &StorageKey,
    result: &mut crate::JournalErasure,
) -> Result<(), Error> {
    let after = crate::erasure::cursor(tx, key, 3)?;
    let rows = tx
        .prepare("SELECT rowid,id FROM structured_cards WHERE rowid>?1 ORDER BY rowid LIMIT 4")?
        .query_map([after], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let next = rows.last().map_or(0, |r| r.0);
    for (_, id) in rows {
        result.checked += 1;
        let id: Id = id.try_into().map_err(|_| Error::InvalidStore)?;
        if live(tx, key, &id)? {
            continue;
        }
        match load_card(tx, key, &id) {
            Ok(Some(card)) => {
                alarms::cancel(tx, key, &id)?;
                polls::discard(tx, &id)?;
                tx.execute(
                    "UPDATE structured_cards SET content=?2 WHERE id=?1",
                    (
                        id.as_slice(),
                        key.seal(&crate::erasure::marker(card.id), &aad(0, &id))?,
                    ),
                )?;
                result.erased += 1;
            }
            Err(Error::Obsolete) => (),
            other => {
                other?.ok_or(Error::InvalidStore)?;
            }
        }
        let actions = tx
            .prepare("SELECT id FROM structured_actions WHERE card=?1 LIMIT 16")?
            .query_map([id.as_slice()], |r| r.get::<_, Vec<u8>>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        for action in actions {
            let action: Id = action.try_into().map_err(|_| Error::InvalidStore)?;
            let mut remaining = false;
            for table in [
                "structured_closed_pages",
                "structured_closed_voters",
                "structured_closed_totals",
            ] {
                tx.execute(&format!("DELETE FROM {table} WHERE rowid IN (SELECT rowid FROM {table} WHERE closure=?1 LIMIT 16)"), [action.as_slice()])?;
                remaining |= tx.query_row(
                    &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE closure=?1)"),
                    [action.as_slice()],
                    |r| r.get::<_, bool>(0),
                )?;
            }
            if remaining {
                continue;
            }
            tx.execute(
                "DELETE FROM structured_dependencies WHERE action=?1",
                [action.as_slice()],
            )?;
            tx.execute(
                "DELETE FROM structured_work WHERE kind=1 AND target=?1",
                [action.as_slice()],
            )?;
            tx.execute(
                "DELETE FROM structured_actions WHERE id=?1",
                [action.as_slice()],
            )?;
            result.erased += 1;
        }
        let items = tx
            .prepare(
                "SELECT DISTINCT item FROM structured_task_completions WHERE card=?1 LIMIT 16",
            )?
            .query_map([id.as_slice()], |r| r.get::<_, Vec<u8>>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        for item in items {
            tx.execute("DELETE FROM structured_task_counts WHERE item=?1", [item])?;
        }
        for table in [
            "structured_task_completions",
            "structured_heads",
            "structured_totals",
        ] {
            tx.execute(&format!("DELETE FROM {table} WHERE rowid IN (SELECT rowid FROM {table} WHERE card=?1 LIMIT 16)"), [id.as_slice()])?;
        }
        tx.execute("DELETE FROM location_jobs WHERE id=?1", [id.as_slice()])?;
        tx.execute(
            "DELETE FROM structured_work WHERE kind=0 AND target=?1",
            [id.as_slice()],
        )?;
    }
    crate::erasure::advance(tx, key, 3, next)
}
