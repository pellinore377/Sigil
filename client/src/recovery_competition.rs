//! Ancestry staging that cannot overwrite an unresolved upload.
use super::*;

fn state_hash(db: &Connection) -> Result<Id, Error> {
    let bytes: Vec<u8> = db.query_row(
        "SELECT state FROM archive WHERE id=1 AND length(state)<=224",
        [],
        |r| r.get(0),
    )?;
    Ok(Sha256::digest(bytes).into())
}
fn candidate(
    db: &Connection,
    wrapping: &StorageKey,
    state: &State,
    check: bool,
) -> Result<Option<Head>, Error> {
    let bytes: Option<Vec<u8>> = db
        .query_row(
            "SELECT data FROM archive_competition WHERE id=x'' AND length(data)<=128",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let Some(bytes) = bytes else {
        return Ok(None);
    };
    let plain = wrapping.open(&bytes, &binding(10, &state.scope, b"competition/v0"))?;
    if plain.len() != 72 {
        return Err(Error::InvalidStore);
    }
    if check && plain[40..] != state_hash(db)? {
        return Err(Error::Conflict);
    }
    Ok(Some(take_head(&plain[..40])?))
}
fn missing(db: &Connection, state: &State, target: Head) -> Result<Option<Head>, Error> {
    let anchor = state.status.anchor.ok_or(Error::Conflict)?;
    if target.generation <= anchor.generation + 1 || target.generation - anchor.generation > 64 {
        return Err(Error::Conflict);
    }
    let mut cursor = target;
    while cursor.generation > anchor.generation {
        let bytes: Option<Vec<u8>> = db
            .query_row(
                "SELECT data FROM archive_competition WHERE id=?1 AND length(data)<=?2",
                (cursor.manifest.as_slice(), MAX_OBJECT_LEN as i64),
                |r| r.get(0),
            )
            .optional()?;
        let Some(bytes) = bytes else {
            return Ok(Some(cursor));
        };
        let manifest = state.key.open_manifest(&cursor, &bytes)?;
        cursor = Head {
            generation: cursor.generation - 1,
            manifest: manifest.previous.ok_or(Error::InvalidStore)?,
        };
    }
    if cursor != anchor {
        return Err(Error::Conflict);
    }
    Ok(None)
}
impl ClientStore {
    /// Pins a competing target and the current sealed archive state. Upload
    /// ciphertext, acknowledgements, local records and repair authorization stay
    /// unchanged while at most 64 ancestry manifests are staged separately.
    pub fn begin_recovery_competition(
        &mut self,
        response: &sigil_protocol::recovery::Head,
        object: &Object,
    ) -> Result<(), Error> {
        let head = server_head(response)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = load(&tx, &self.key)?;
        if matches!(state.status.pending, Some((Operation::Import, _))) {
            return Err(Error::Conflict);
        }
        let anchor = state.status.anchor.ok_or(Error::Conflict)?;
        if head.generation <= anchor.generation + 1 || head.generation - anchor.generation > 64 {
            return Err(Error::Conflict);
        }
        state.key.open_manifest(&head, object.bytes())?;
        if let Some(existing) = candidate(&tx, &self.key, &state, true)? {
            return if existing == head {
                Ok(())
            } else {
                Err(Error::Conflict)
            };
        }
        if tx.query_row("SELECT count(*) FROM archive_competition", [], |r| {
            r.get::<_, i64>(0)
        })? != 0
        {
            return Err(Error::InvalidStore);
        }
        let mut bytes = Vec::with_capacity(72);
        put_head(&mut bytes, head);
        bytes.extend_from_slice(&state_hash(&tx)?);
        tx.execute(
            "INSERT INTO archive_competition VALUES(x'',?1)",
            [self
                .key
                .seal(&bytes, &binding(10, &state.scope, b"competition/v0"))?],
        )?;
        tx.execute(
            "INSERT INTO archive_competition VALUES(?1,?2)",
            (head.manifest.as_slice(), object.bytes()),
        )?;
        tx.commit()?;
        Ok(())
    }
    /// Includes stale candidates so callers can explicitly cancel after another
    /// operation changes archive state. Advancing a stale candidate fails closed.
    pub fn recovery_competition(&self) -> Result<Option<Head>, Error> {
        let state = load(&self.db, &self.key)?;
        candidate(&self.db, &self.key, &state, false)
    }
    pub fn next_recovery_competition_manifest(&self) -> Result<Option<Head>, Error> {
        let state = load(&self.db, &self.key)?;
        let head = candidate(&self.db, &self.key, &state, true)?.ok_or(Error::Unprepared)?;
        missing(&self.db, &state, head)
    }
    pub fn stage_recovery_competition_manifest(&mut self, object: &Object) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = load(&tx, &self.key)?;
        let head = candidate(&tx, &self.key, &state, true)?.ok_or(Error::Unprepared)?;
        let expected = missing(&tx, &state, head)?.ok_or(Error::Unprepared)?;
        state.key.open_manifest(&expected, object.bytes())?;
        tx.execute(
            "INSERT INTO archive_competition VALUES(?1,?2)",
            (expected.manifest.as_slice(), object.bytes()),
        )?;
        missing(&tx, &state, head)?;
        tx.commit()?;
        Ok(())
    }
    /// Only a complete authenticated chain can replace the unresolved upload.
    /// History and the anchor still wait for normal snapshot import to commit.
    pub fn finish_recovery_competition(&mut self) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut state = load(&tx, &self.key)?;
        let head = candidate(&tx, &self.key, &state, true)?.ok_or(Error::Unprepared)?;
        if missing(&tx, &state, head)?.is_some() {
            return Err(Error::Unprepared);
        }
        let anchor = state.status.anchor.ok_or(Error::Conflict)?;
        if tx.query_row("SELECT count(*) FROM archive_competition", [], |r| {
            r.get::<_, i64>(0)
        })? != (head.generation - anchor.generation + 1) as i64
        {
            return Err(Error::InvalidStore);
        }
        clear_pending(&tx)?;
        tx.execute("INSERT INTO archive_objects(id,data) SELECT id,data FROM archive_competition WHERE length(id)=32", [])?;
        state.status.pending = Some((Operation::Import, head));
        work::dirty(&tx, &self.key, &state.scope)?;
        state.reconcile_restored = false;
        state.restore_base = None;
        save(&tx, &self.key, &state)?;
        tx.execute("DELETE FROM archive_competition", [])?;
        tx.commit()?;
        Ok(())
    }
    pub fn cancel_recovery_competition(&mut self, head: Head) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = load(&tx, &self.key)?;
        if candidate(&tx, &self.key, &state, false)? != Some(head) {
            return Err(Error::Conflict);
        }
        tx.execute("DELETE FROM archive_competition", [])?;
        tx.commit()?;
        Ok(())
    }
}
