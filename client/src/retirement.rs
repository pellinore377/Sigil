//! Bounded inactivity policy and explicit retirement; neither guarantees physical erasure.
use super::*;

impl ClientStore {
    /// Retire an obsolete session only after every queued packet is accepted or expired.
    /// Preserves message history, receipts and ID tombstones. Delayed new packets
    /// cannot decrypt afterward. Use maintain_sessions_online for the guarded grace
    /// policy. Filesystem snapshots and flash may retain prior ciphertext.
    pub fn retire_session(&mut self, id: Id) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        retire(&tx, &self.key, id)?;
        tx.commit()?;
        Ok(())
    }
}
fn retire(tx: &Transaction<'_>, key: &StorageKey, id: Id) -> Result<(), Error> {
    let (revision, state, suite, retired): (i64, Vec<u8>, i64, bool) = tx
        .query_row(
            "SELECT revision,state,suite,retired FROM sessions WHERE id=?1 AND length(state)<=?2",
            (id.as_slice(), MAX_SEALED_CHECKPOINT_LEN as i64),
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    if revision < 0 {
        return Err(Error::InvalidStore);
    }
    if suite != 2 {
        return Err(Error::UnsupportedSession);
    }
    let peer = session_peer(tx, &id)?;
    let retirement_binding = |revision| {
        let mut aad = state_binding(&id, revision, peer);
        aad.extend_from_slice(b"Sigil/retired-session/v0");
        aad
    };
    if retired {
        if key.open(&state, &retirement_binding(revision))?.as_slice() != b"retired" {
            return Err(Error::InvalidStore);
        }
        return Ok(());
    }
    if tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM outbox WHERE session=?1 AND packet IS NOT NULL)",
        [id.as_slice()],
        |r| r.get::<_, bool>(0),
    )? {
        return Err(Error::Conflict);
    }
    // Retirement remains possible for blocked/quarantined peers, but the
    // checkpoint must still authenticate its identity and revision bindings.
    Session::open_checkpoint(key, &state, &state_binding(&id, revision, peer))?;
    if let Some(peer) = peer {
        selection::retire(tx, key, &peer, &id)?;
    }
    let next = revision.checked_add(1).ok_or(Error::Limit)?;
    tx.execute(
        "DELETE FROM initial_headers WHERE session=?1",
        [id.as_slice()],
    )?;
    let sealed = key.seal(b"retired", &retirement_binding(next))?;
    if tx.execute("UPDATE sessions SET state=?1,revision=?2,retired=1 WHERE id=?3 AND revision=?4 AND retired=0", (sealed, next, id.as_slice(), revision))? != 1 {
            return Err(Error::Conflict);
        }
    Ok(())
}

/// Retain inactive sessions for a full maximum server-delivery window.
pub const INACTIVE_GRACE_SECONDS: u64 = 7 * 24 * 60 * 60;
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SessionMaintenance {
    pub scanned: usize,
    pub retired: usize,
    pub expired: usize,
    /// Private prekey slots retired after their authenticated retention deadline.
    pub prekeys_retired: usize,
}
impl ClientStore {
    /// Offline maintenance expires outbound work and tracks inactivity; it never
    /// automatically retires keys without a fresh online mailbox check.
    /// A revision change, pending packet or active selection resets the grace
    /// interval. A new/migrated session receives a full observed grace interval.
    pub fn maintain_sessions(&mut self, now: u64) -> Result<SessionMaintenance, Error> {
        self.maintain_sessions_in(now, None)
    }
    /// Process/acknowledge incoming work before invoking this bounded worker.
    /// Only a fresh empty mailbox authorizes retirement during this call. Also
    /// retires at most 16 private prekeys past their local retention deadlines.
    pub fn maintain_sessions_online(&mut self, now: u64) -> Result<SessionMaintenance, Error> {
        if now == 0 || now > i64::MAX as u64 {
            return Err(Error::InvalidEvent);
        }
        let expected: Vec<u8> = self.db.query_row(
            "SELECT state FROM connection WHERE id=1 AND length(state)<=4096",
            [],
            |r| r.get(0),
        )?;
        let empty = self.connected_client()?.mailbox_after(0)?.is_empty();
        self.maintain_sessions_in(now, empty.then_some(expected))
    }
    fn maintain_sessions_in(
        &mut self,
        now: u64,
        drained: Option<Vec<u8>>,
    ) -> Result<SessionMaintenance, Error> {
        if now == 0 || now > i64::MAX as u64 {
            return Err(Error::InvalidEvent);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(expected) = &drained {
            let current: Vec<u8> = tx.query_row(
                "SELECT state FROM connection WHERE id=1 AND length(state)<=4096",
                [],
                |r| r.get(0),
            )?;
            if current != *expected {
                return Err(Error::Conflict);
            }
        }
        let own = handshake::identity(&tx, &self.key)?.public_key();
        let cursor_aad = binding(31, &own, b"session-maintenance");
        let cursor: Option<Vec<u8>> = tx.query_row("SELECT CASE WHEN length(state)=77 THEN state END FROM session_maintenance WHERE id=1", [], |r| r.get(0)).optional()?;
        let after = if let Some(cursor) = cursor {
            let bytes = self.key.open(&cursor, &cursor_aad)?;
            if bytes.len() != 41 || bytes[0] > 1 {
                return Err(Error::InvalidStore);
            }
            let last = u64::from_be_bytes(bytes[33..].try_into().map_err(|_| Error::InvalidStore)?);
            if now < last {
                return Err(Error::Expired);
            }
            if bytes[0] == 1 {
                bytes[1..33].to_vec()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        let ids: Vec<Vec<u8>> = tx
            .prepare("SELECT id FROM sessions WHERE id>?1 ORDER BY id LIMIT 16")?
            .query_map([&after], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let mut result = SessionMaintenance::default();
        for raw in &ids {
            let id: Id = raw.as_slice().try_into().map_err(|_| Error::InvalidStore)?;
            let (revision, state, suite, retired): (i64, Vec<u8>, i64, bool) = tx.query_row(
                "SELECT revision,CASE WHEN length(state)<=?2 THEN state END,suite,retired FROM sessions WHERE id=?1",
                (raw, MAX_SEALED_CHECKPOINT_LEN as i64), |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
            result.scanned += 1;
            if suite != 2 {
                continue;
            }
            if retired {
                retire(&tx, &self.key, id)?;
                continue;
            }
            let peer = session_peer(&tx, &id)?;
            Session::open_checkpoint(&self.key, &state, &state_binding(&id, revision, peer))?;
            if revision < 0 {
                return Err(Error::InvalidStore);
            }
            let active = if let Some(peer) = peer {
                selection::record(&tx, &self.key, &peer)?
                    .is_some_and(|(selected, _)| selected == id)
            } else {
                !groups::scoped_channel(&tx, &self.key, &id)?
            }; // Only authenticated group scopes give unbound sessions a policy.
            let queued: Vec<Vec<u8>>=tx.prepare("SELECT o.id FROM outbox o JOIN deliveries d ON d.session=o.session AND d.id=o.id WHERE o.session=?1 AND o.packet IS NOT NULL ORDER BY o.rowid LIMIT 16")?
                .query_map([raw],|r|r.get(0))?.collect::<Result<_,_>>()?;
            for message in queued {
                let message: Id = message.try_into().map_err(|_| Error::InvalidStore)?;
                match transport::expire_in(&tx, &self.key, id, message, now) {
                    Ok(true) => result.expired += 1,
                    Ok(false) | Err(Error::Expired) => {}
                    Err(error) => return Err(error),
                }
            }
            let pending: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM outbox WHERE session=?1 AND packet IS NOT NULL)",
                [raw],
                |r| r.get(0),
            )?;
            let aad = binding(30, &id, &own);
            let previous: Option<Vec<u8>> = tx.query_row("SELECT CASE WHEN length(state)=60 THEN state END FROM session_activity WHERE id=?1", [raw], |r| r.get(0)).optional()?;
            let mut since = now;
            if let Some(previous) = previous {
                let bytes = self.key.open(&previous, &aad)?;
                if bytes.len() != 24 {
                    return Err(Error::InvalidStore);
                }
                let old_revision =
                    i64::from_be_bytes(bytes[..8].try_into().map_err(|_| Error::InvalidStore)?);
                let observed =
                    u64::from_be_bytes(bytes[8..16].try_into().map_err(|_| Error::InvalidStore)?);
                let eligible =
                    u64::from_be_bytes(bytes[16..].try_into().map_err(|_| Error::InvalidStore)?);
                if old_revision > revision
                    || old_revision < 0
                    || observed == 0
                    || eligible > observed
                    || now < observed
                {
                    return Err(Error::InvalidStore);
                }
                if old_revision == revision && !active && !pending && eligible != 0 {
                    since = eligible;
                }
            }
            if drained.is_some()
                && !active
                && !pending
                && now.saturating_sub(since) >= INACTIVE_GRACE_SECONDS
            {
                retire(&tx, &self.key, id)?;
                tx.execute("DELETE FROM session_activity WHERE id=?1", [raw])?;
                result.retired += 1;
            } else {
                let eligible = if active || pending { 0 } else { since };
                let bytes = [
                    revision.to_be_bytes(),
                    now.to_be_bytes(),
                    eligible.to_be_bytes(),
                ]
                .concat();
                let sealed = self.key.seal(&bytes, &aad)?;
                tx.execute("INSERT INTO session_activity VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET state=excluded.state", (raw, sealed))?;
            }
        }
        let last = if ids.len() == 16 {
            ids.last().ok_or(Error::InvalidStore)?.as_slice()
        } else {
            &[0; 32]
        };
        let sealed = self.key.seal(
            &[&[u8::from(ids.len() == 16)], last, &now.to_be_bytes()].concat(),
            &cursor_aad,
        )?;
        tx.execute("INSERT INTO session_maintenance VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET state=excluded.state", [sealed])?;
        if drained.is_some() {
            result.prekeys_retired = prekeys::retire_in(&tx, &self.key, &own, now)?;
        }
        tx.commit()?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn connection_change_invalidates_an_empty_mailbox_check() {
        let (_dir, _fixture, mut alice, _bob, now) = crate::claims::tests::pair();
        let checked: Vec<u8> = alice
            .db
            .query_row("SELECT state FROM connection WHERE id=1", [], |r| r.get(0))
            .unwrap();
        assert!(alice
            .connected_client()
            .unwrap()
            .mailbox_after(0)
            .unwrap()
            .is_empty());
        alice.prepare_credential_rotation().unwrap();
        alice.rotate_credential_online().unwrap();
        assert!(matches!(
            alice.maintain_sessions_in(now, Some(checked)),
            Err(Error::Conflict)
        ));
        assert_eq!(
            alice
                .db
                .query_row("SELECT count(*) FROM session_maintenance", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        alice.maintain_sessions_online(now).unwrap();
    }
}
