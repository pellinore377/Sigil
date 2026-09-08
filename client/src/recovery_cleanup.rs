use super::*;

fn aad(scope: &Id, id: &Id) -> Vec<u8> {
    binding(97, scope, id)
}
fn garbage_aad(scope: &Id, id: &Id) -> Vec<u8> {
    binding(98, scope, id)
}
fn proof(id: &Id, head: Head) -> Vec<u8> {
    [
        id.as_slice(),
        &head.generation.to_be_bytes(),
        &head.manifest,
    ]
    .concat()
}
pub(super) fn checkpoint(
    db: &Connection,
    key: &StorageKey,
    state: &State,
    operation: Operation,
) -> Result<(), Error> {
    // Called only after the complete snapshot and its ancestry have authenticated.
    let scope = &state.scope;
    let head = pending(state, operation)?;
    let mut live = std::collections::BTreeSet::from([head.manifest]);
    for page in manifest(db, state, operation)?.pages {
        live.insert(page.object);
        let raw: Vec<u8> = db.query_row("SELECT data FROM archive_objects WHERE id=?1 UNION ALL SELECT data FROM archive_pages WHERE object=?1 LIMIT 1", [page.object.as_slice()], |r| r.get(0))?;
        live.extend(
            state
                .key
                .open_page(&page, &raw)?
                .into_iter()
                .map(|v| v.object),
        );
    }
    let mut checkpoints = db.prepare("SELECT generation,manifest,data FROM archive_checkpoints")?;
    let mut rows = checkpoints.query([])?;
    while let Some(row) = rows.next()? {
        let generation: i64 = row.get(0)?;
        let id: Vec<u8> = row.get(1)?;
        let checkpoint = Head {
            generation: generation.try_into().map_err(|_| Error::InvalidStore)?,
            manifest: id.try_into().map_err(|_| Error::InvalidStore)?,
        };
        state
            .key
            .open_manifest(&checkpoint, &row.get::<_, Vec<u8>>(2)?)?;
        live.insert(checkpoint.manifest);
    }
    drop(rows);
    drop(checkpoints);
    let mut previous = db.prepare("SELECT id,state,0 FROM archive_remote UNION ALL SELECT id,state,1 FROM archive_garbage ORDER BY id")?;
    let mut rows = previous.query([])?;
    while let Some(row) = rows.next()? {
        let id: Vec<u8> = row.get(0)?;
        let id: Id = id.try_into().map_err(|_| Error::InvalidStore)?;
        let sealed: Vec<u8> = row.get(1)?;
        let garbage: bool = row.get(2)?;
        let raw = key.open(
            &sealed,
            &if garbage {
                garbage_aad(scope, &id)
            } else {
                aad(scope, &id)
            },
        )?;
        if (!garbage && raw.as_slice() != id) || (garbage && (raw.len() != 72 || raw[..32] != id)) {
            return Err(Error::InvalidStore);
        }
        db.execute("INSERT INTO archive_garbage VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET state=excluded.state",
            (id.as_slice(), key.seal(&proof(&id,head), &garbage_aad(scope, &id))?))?;
    }
    drop(rows);
    drop(previous);
    db.execute("DELETE FROM archive_remote", [])?;
    for id in live {
        db.execute(
            "INSERT INTO archive_remote VALUES(?1,?2)",
            (id.as_slice(), key.seal(&id, &aad(scope, &id))?),
        )?;
        db.execute("DELETE FROM archive_garbage WHERE id=?1", [id.as_slice()])?;
    }
    Ok(())
}
impl ClientStore {
    /// Delete at most 64 superseded objects under the exact confirmed head.
    /// The server CAS and local write reservation prevent stale cleanup.
    pub fn cleanup_recovery_online(&mut self) -> Result<usize, Error> {
        let scope = self.connected_account_scope()?;
        let client = self.connected_client()?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = load(&tx, &self.key)?;
        if state.scope != scope {
            return Err(Error::Conflict);
        }
        idle(&state)?;
        let Some(head) = state.status.anchor else {
            return Ok(0);
        };
        let mut query = tx.prepare("SELECT id,state FROM archive_garbage ORDER BY id LIMIT 64")?;
        let mut rows = query.query([])?;
        let mut objects = Vec::new();
        while let Some(row) = rows.next()? {
            let id: Vec<u8> = row.get(0)?;
            let id: Id = id.try_into().map_err(|_| Error::InvalidStore)?;
            let sealed: Vec<u8> = row.get(1)?;
            if self
                .key
                .open(&sealed, &garbage_aad(&scope, &id))?
                .as_slice()
                != proof(&id, head)
            {
                return Err(Error::InvalidStore);
            }
            if id == head.manifest || tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM archive_remote WHERE id=?1) OR EXISTS(SELECT 1 FROM archive_records WHERE object=?1) OR EXISTS(SELECT 1 FROM archive_checkpoints WHERE manifest=?1)",
                [id.as_slice()], |r| r.get::<_, bool>(0))? { return Err(Error::Conflict); }
            objects.push(crate::transport::hex(&id));
        }
        drop(rows);
        drop(query);
        if objects.is_empty() {
            let removed = super::media_cleanup::cleanup(&tx, &self.key, &client, head)?;
            tx.commit()?;
            return Ok(removed);
        }
        client.delete_recovery_objects(&sigil_protocol::recovery::DeleteObjects {
            expected_generation: head.generation,
            expected_manifest: crate::transport::hex(&head.manifest),
            objects: objects.clone(),
        })?;
        for id in &objects {
            tx.execute(
                "DELETE FROM archive_garbage WHERE id=?1",
                [crate::connection::decode_id(id)?.as_slice()],
            )?;
        }
        tx.commit()?;
        Ok(objects.len())
    }
}
