use super::*;
use serde::{Deserialize, Serialize};

pub(crate) const MIGRATION: &str = "
CREATE TABLE archive_lifecycle(id INTEGER PRIMARY KEY CHECK(id=1),state BLOB NOT NULL);
CREATE TABLE archive_protected(id BLOB PRIMARY KEY,object BLOB NOT NULL);
CREATE TABLE archive_remote(id BLOB PRIMARY KEY,state BLOB NOT NULL);
CREATE TABLE archive_garbage(id BLOB PRIMARY KEY,state BLOB NOT NULL);
CREATE TABLE archive_local(id BLOB PRIMARY KEY,state BLOB NOT NULL);
CREATE TABLE archive_media_garbage(id BLOB PRIMARY KEY,state BLOB NOT NULL);
CREATE TABLE archive_media_removed(id BLOB PRIMARY KEY,state BLOB NOT NULL);
CREATE TABLE conversation_removed(id BLOB PRIMARY KEY,state BLOB NOT NULL);
CREATE TABLE conversation_cleanup(id INTEGER PRIMARY KEY CHECK(id=1),state BLOB NOT NULL);
CREATE TABLE conversation_archive_refs(record BLOB PRIMARY KEY,entry BLOB NOT NULL,state BLOB NOT NULL);
CREATE INDEX conversation_archive_entry ON conversation_archive_refs(entry,record);
PRAGMA user_version=63;";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryPolicy {
    /// None retains remote history until explicit deletion. Local history is unaffected.
    pub history_days: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claims::tests::pair;
    use crate::conversations::{Action, Body};
    fn publish(a: &mut ClientStore) {
        for _ in 0..60 {
            match a.upload_recovery_step() {
                Ok(Some(_)) => return,
                Ok(None) => (),
                Err(Error::Network(crate::network::Error::Status {
                    code: 429,
                    retry_after_seconds: Some(delay),
                })) => std::thread::sleep(std::time::Duration::from_secs(delay.min(5))),
                Err(error) => panic!("{error:?}"),
            }
        }
        panic!("publication did not complete");
    }
    fn cleanup(a: &mut ClientStore) -> Result<usize, Error> {
        for _ in 0..10 {
            match a.cleanup_recovery_online() {
                Err(Error::Network(crate::network::Error::Status {
                    code: 429,
                    retry_after_seconds: Some(delay),
                })) => std::thread::sleep(std::time::Duration::from_secs(delay.min(5))),
                result => return result,
            }
        }
        panic!("cleanup remained rate limited");
    }

    #[test]
    fn backfill_retention_progress_and_remote_cleanup_survive_failed_commits() {
        let (dir, _fixture, mut a, _b, now) = pair();
        for id in 1..=19 {
            let op = a
                .conversation_operation(
                    [id; 32],
                    Action::Post {
                        body: Body::Text("retained local notebook".into()),
                        reply: None,
                        thread: None,
                        expires_at: None,
                        view_once: false,
                    },
                )
                .unwrap();
            a.note_to_self(&op, now, now).unwrap();
        }
        let secret = a.enable_history_recovery().unwrap();
        assert_ne!(*secret, [0; 32]);
        assert!(a.enable_history_recovery().is_err());
        a.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON archive_lifecycle BEGIN SELECT RAISE(ABORT,'synthetic'); END").unwrap();
        assert!(a.maintain_recovery(now).is_err());
        assert_eq!(a.recovery_records(None).unwrap().len(), 0);
        a.db.execute_batch("DROP TRIGGER fail").unwrap();
        assert_eq!(a.maintain_recovery(now).unwrap().backfilled, 16);
        assert!(!a.history_recovery_progress().unwrap().backfill_complete);
        drop(a);
        let mut a = ClientStore::open(
            &dir.path().join("alice.db"),
            StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
        )
        .unwrap();
        assert_eq!(a.maintain_recovery(now).unwrap().backfilled, 3);
        assert!(a.history_recovery_progress().unwrap().backfill_complete);
        assert_eq!(
            a.history_recovery_progress().unwrap().unprotected_records,
            19
        );
        a.prepare_recovery_upload(now).unwrap();
        publish(&mut a);
        let before = a.account_storage_online().unwrap();
        assert_eq!(
            a.history_recovery_progress().unwrap().unprotected_records,
            0
        );
        assert_eq!(
            a.history_recovery_progress().unwrap().last_checkpoint_at,
            Some(now)
        );
        let obsolete = a.recovery_records(None).unwrap()[0].id;
        let old = existing(&a.db, obsolete).unwrap().unwrap().0.object;
        a.set_recovery_policy(RecoveryPolicy {
            history_days: Some(1),
        })
        .unwrap();
        a.prepare_recovery_upload(now + 86400).unwrap();
        publish(&mut a);
        a.db.execute_batch("CREATE TRIGGER fail BEFORE DELETE ON archive_garbage BEGIN SELECT RAISE(ABORT,'synthetic'); END").unwrap();
        assert!(matches!(cleanup(&mut a), Err(Error::Storage(_))));
        a.db.execute_batch("DROP TRIGGER fail").unwrap();
        assert!(cleanup(&mut a).unwrap() >= 19);
        assert!(matches!(
            a.connected_client().unwrap().download_recovery_object(old),
            Err(crate::network::Error::Status { code: 404, .. })
        ));
        assert_eq!(a.cleanup_recovery_online().unwrap(), 0);
        assert!(a.account_storage_online().unwrap().used_bytes < before.used_bytes);
        // The retention policy removes remote history, not the local notebook.
        let page = a
            .search_conversations("retained local notebook", None, now + 86400)
            .unwrap();
        assert_eq!(page.hits.len(), 19);
        assert!(matches!(
            a.recovery_record(obsolete).unwrap().content,
            Content::Omitted
        ));
        assert!(matches!(
            a.retained_history_record(obsolete).unwrap().content,
            Content::Conversation(_)
        ));
        assert!(a.history_recovery_progress().unwrap().unprotected_records == 0);
    }
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Lifecycle {
    pub backfill: i64,
}
pub(super) fn read(db: &Connection, key: &StorageKey, scope: &Id) -> Result<Lifecycle, Error> {
    let raw: Option<Vec<u8>> = db
        .query_row(
            "SELECT CASE WHEN length(state)<=4096 THEN state END FROM archive_lifecycle WHERE id=1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let state: Lifecycle = match raw {
        Some(raw) => {
            serde_json::from_slice(&key.open(&raw, &binding(96, scope, b"recovery lifecycle"))?)
                .map_err(|_| Error::InvalidStore)?
        }
        None => Lifecycle::default(),
    };
    if state.backfill < 0 {
        return Err(Error::InvalidStore);
    }
    Ok(state)
}
pub(super) fn save(
    db: &Connection,
    key: &StorageKey,
    scope: &Id,
    state: &Lifecycle,
) -> Result<(), Error> {
    let raw = Zeroizing::new(serde_json::to_vec(state).map_err(|_| Error::InvalidStore)?);
    db.execute("INSERT INTO archive_lifecycle VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET state=excluded.state",
        [key.seal(&raw, &binding(96, scope, b"recovery lifecycle"))?])?;
    Ok(())
}
pub(super) fn protect(db: &Connection, state: &State, operation: Operation) -> Result<(), Error> {
    // Snapshot references, not current local edits, define the confirmed protection boundary.
    let manifest = manifest(db, state, operation)?;
    db.execute("DELETE FROM archive_protected", [])?;
    for page in &manifest.pages {
        let bytes: Vec<u8> = db.query_row(
            "SELECT data FROM archive_objects WHERE id=?1 UNION ALL SELECT data FROM archive_pages WHERE object=?1 LIMIT 1",
            [page.object.as_slice()], |r| r.get(0))?;
        for reference in state.key.open_page(page, &bytes)? {
            db.execute(
                "INSERT INTO archive_protected VALUES(?1,?2)",
                (reference.id.as_slice(), reference.object.as_slice()),
            )?;
        }
    }
    Ok(())
}
pub struct HistoryProgress {
    pub status: Status,
    pub last_checkpoint_at: Option<u64>,
    pub backfill_complete: bool,
    pub committed_records: u64,
    pub record_limit: u64,
    pub unprotected_records: u64,
    pub pending_objects: u64,
    pub staged_records: u64,
    pub staged_pages: u64,
}
pub struct MediaCheckpoint {
    pub record: Id,
    pub protected: bool,
    pub excluded: bool,
}
pub struct MediaCheckpointPage {
    pub records: Vec<MediaCheckpoint>,
    pub next: Option<Id>,
}
impl ClientStore {
    /// Bounded status for media not yet included in a confirmed backup.
    pub fn recovery_media_checkpoints(
        &self,
        after: Option<Id>,
    ) -> Result<MediaCheckpointPage, Error> {
        let tx = self.db.unchecked_transaction()?;
        let state = load(&tx, &self.key)?;
        if matches!(state.status.pending, Some((Operation::Import, _))) {
            return Err(Error::Unprepared);
        }
        let mut query = tx
            .prepare("SELECT record FROM archive_media WHERE record>?1 ORDER BY record LIMIT 16")?;
        let mut rows = query.query([after.map(|v| v.to_vec()).unwrap_or_default()])?;
        let mut records = Vec::new();
        let mut next = None;
        let mut count = 0;
        while let Some(row) = rows.next()? {
            let id: Vec<u8> = row.get(0)?;
            let id: Id = id.try_into().map_err(|_| Error::InvalidStore)?;
            next = Some(id);
            count += 1;
            let record = local_record(&tx, &self.key, id)?;
            if matches!(record.content, Content::Media { .. }) {
                continue;
            }
            if super::media::file_bytes(&record.content)?.is_none() {
                return Err(Error::InvalidStore);
            }
            let mut excluded = matches!(
                read_record(&tx, &self.key, id)?.content,
                Content::Omitted | Content::Deleted
            );
            let mut protected = false;
            if let Some((reference, raw)) = existing(&tx, copy_id(id))? {
                let copy = state.key.open_record(&reference, &raw)?;
                excluded |= matches!(copy.content, Content::Deleted | Content::Omitted);
                if matches!(copy.content,Content::Media { record:parent, .. } if parent==id) {
                    protected = tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM archive_protected WHERE id=?1 AND object=?2)",
                        (reference.id.as_slice(), reference.object.as_slice()),
                        |r| r.get(0),
                    )?;
                }
            }
            records.push(MediaCheckpoint {
                record: id,
                protected,
                excluded,
            });
        }
        drop(rows);
        drop(query);
        tx.commit()?;
        Ok(MediaCheckpointPage {
            records,
            next: if count == 16 { next } else { None },
        })
    }
    /// Return the separately generated secret once for offline safekeeping.
    pub fn enable_history_recovery(&mut self) -> Result<Zeroizing<Id>, Error> {
        let own = peers::parse(&self.own_device_binding()?)?.binding;
        let mut exported = Zeroizing::new([0; 32]);
        getrandom::fill(exported.as_mut()).map_err(|_| sigil_crypto::Error::Entropy)?;
        self.configure_recovery(&own.server, own.account, Secret32::from_bytes(*exported))?;
        Ok(exported)
    }
    pub fn recovery_policy(&self) -> Result<RecoveryPolicy, Error> {
        load(&self.db, &self.key)?;
        crate::conversations::recovery_policy(&self.db, &self.key)
    }
    pub fn set_recovery_policy(&mut self, policy: RecoveryPolicy) -> Result<(), Error> {
        if policy.history_days == Some(0) {
            return Err(Error::InvalidEvent);
        }
        let state = load(&self.db, &self.key)?;
        idle(&state)?;
        let mut id = [0; 32];
        getrandom::fill(&mut id).map_err(|_| sigil_crypto::Error::Entropy)?;
        let op = self.conversation_operation(
            id,
            crate::conversations::Action::Private {
                conversation: [0; 32],
                value: crate::conversations::Private::RecoveryRetention(policy.history_days),
            },
        )?;
        self.apply_private_operation(&op, crate::conversations::now())
    }
    pub fn history_recovery_progress(&self) -> Result<HistoryProgress, Error> {
        let tx = self.db.unchecked_transaction()?;
        let state = load(&tx, &self.key)?;
        let last_checkpoint_at = state
            .status
            .anchor
            .map(|head| {
                let raw: Vec<u8> = tx.query_row(
                    "SELECT data FROM archive_checkpoints WHERE generation=?1 AND manifest=?2",
                    (head.generation as i64, head.manifest.as_slice()),
                    |r| r.get(0),
                )?;
                Ok::<_, Error>(state.key.open_manifest(&head, &raw)?.created_at)
            })
            .transpose()?;
        let progress = HistoryProgress {
            status: state.status,
            last_checkpoint_at,
            backfill_complete: !tx.query_row("SELECT EXISTS(SELECT 1 FROM conversation_ops WHERE rowid>?1)",
                [read(&tx, &self.key, &state.scope)?.backfill], |r| r.get::<_, bool>(0))?,
            committed_records: tx.query_row("SELECT count(*) FROM archive_records", [], |r| r.get::<_, i64>(0))? as u64,
            record_limit: MAX_RECORDS as u64,
            unprotected_records: tx.query_row("SELECT count(*) FROM archive_records r LEFT JOIN archive_protected p ON r.id=p.id AND r.object=p.object WHERE p.id IS NULL", [], |r| r.get::<_, i64>(0))? as u64,
            pending_objects: tx.query_row("SELECT count(*) FROM archive_objects WHERE uploaded=0", [], |r| r.get::<_, i64>(0))? as u64,
            staged_records: tx.query_row("SELECT count(*) FROM archive_import", [], |r| r.get::<_, i64>(0))? as u64,
            staged_pages: tx.query_row("SELECT count(*) FROM archive_pages", [], |r| r.get::<_, i64>(0))? as u64,
        };
        tx.commit()?;
        Ok(progress)
    }
}
