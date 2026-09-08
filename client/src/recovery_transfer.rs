use super::*;
use sigil_crypto::recovery::{Content, Record};

#[derive(Debug, PartialEq, Eq)]
pub enum MediaRecovery {
    Idle,
    Download(Id),
    Staged(Id, u32),
    Upload(Id),
    Protected(Id),
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Transfer {
    original: Id,
    source: Id,
    target: Id,
}
fn binding(id: &Id) -> Vec<u8> {
    [b"Sigil/recovery-transfer/v0".as_slice(), id].concat()
}

impl ClientStore {
    /// Re-encrypt one chunk per call; a recovered descriptor never depends on
    /// the sender retaining their original upload. Transfer scheduling is shared.
    pub fn republish_recovery_media_step(
        &mut self,
        cache: &mut Cache,
        parent: Id,
        now: u64,
    ) -> Result<MediaRecovery, Error> {
        let server = events::source(self, cache)?;
        let tx = cache
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let archive = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        match crate::recovery::read_record(&archive, &self.key, parent)?.content {
            Content::Omitted => return Ok(MediaRecovery::Idle),
            Content::Media { .. } => return Err(Error::InvalidEvent),
            _ => (),
        }
        match crate::recovery::read_record(&archive, &self.key, crate::recovery::copy_id(parent)) {
            Ok(record) if matches!(record.content, Content::Deleted) => {
                return Err(Error::Obsolete)
            }
            Ok(record) if matches!(record.content, Content::Omitted) => {
                return Ok(MediaRecovery::Idle)
            }
            Ok(_) | Err(Error::NotFound) => (),
            Err(error) => return Err(error),
        }
        let original = crate::recovery::retained_file(&archive, &self.key, cache.scope, parent)?
            .ok_or(Error::NotFound)?;
        let original_hash: Id = Sha256::digest(&original).into();
        let copy = crate::recovery::recovery_file(&archive, &self.key, cache.scope, parent)?;
        if copy != original {
            let file =
                sigil_protocol::file::File::from_bytes(&copy).map_err(|_| Error::InvalidStore)?;
            let (file_key, _) = FileKey::from_descriptor(file.descriptor)?;
            let id = download::prepare(
                &tx,
                &cache.key,
                cache.budget,
                events::descriptor(&copy, &server)?,
                now,
                true,
            )?;
            let complete = matches!(
                load(&tx, &cache.key, id)?.phase,
                Phase::Published | Phase::Complete | Phase::Local
            );
            tx.execute(
                "DELETE FROM recovery_transfers WHERE id=?1",
                [parent.as_slice()],
            )?;
            tx.commit()?;
            return Ok(if complete {
                MediaRecovery::Protected(file_key.shape().file)
            } else {
                MediaRecovery::Download(id)
            });
        }
        let file =
            sigil_protocol::file::File::from_bytes(&original).map_err(|_| Error::InvalidStore)?;
        if file.expires_at.is_some_and(|v| v <= now) {
            return Err(Error::Expired);
        }
        let raw: Option<Vec<u8>> = tx.query_row("SELECT CASE WHEN length(state)<=4096 THEN state END FROM recovery_transfers WHERE id=?1", [parent.as_slice()], |r| r.get(0)).optional()?;
        let transfer = if let Some(raw) = raw {
            let transfer: Transfer =
                serde_json::from_slice(&cache.key.open(&raw, &binding(&parent))?)
                    .map_err(|_| Error::InvalidStore)?;
            if transfer.original != original_hash {
                return Err(Error::Conflict);
            }
            transfer
        } else {
            let source = download::prepare(
                &tx,
                &cache.key,
                cache.budget,
                events::descriptor(&original, &server)?,
                now,
                true,
            )?;
            let (source_key, _) = FileKey::from_descriptor(file.descriptor)?;
            let mut target = upload_state(
                source_key.shape().length,
                Metadata {
                    name: file.name.into(),
                    media_type: file.media_type.into(),
                },
                file.expires_at,
                cache.budget,
            )?;
            target.recovery_parent = Some((parent, original_hash));
            save(&tx, &cache.key, &target, true)?;
            let transfer = Transfer {
                original: original_hash,
                source,
                target: target.file,
            };
            tx.execute(
                "INSERT INTO recovery_transfers VALUES(?1,?2)",
                (
                    parent.as_slice(),
                    cache.key.seal(
                        &serde_json::to_vec(&transfer).map_err(|_| Error::InvalidStore)?,
                        &binding(&parent),
                    )?,
                ),
            )?;
            transfer
        };
        let source = load(&tx, &cache.key, transfer.source)?;
        live(&source, now)?;
        if !matches!(source.phase, Phase::Published | Phase::Complete) {
            tx.commit()?;
            return Ok(MediaRecovery::Download(transfer.source));
        }
        let mut target = load(&tx, &cache.key, transfer.target)?;
        live(&target, now)?;
        if target.phase == Phase::Published {
            let bytes = events::content(&target, &server)?;
            let base = crate::recovery::read_record(&archive, &self.key, parent)?;
            crate::recovery::retain_new(
                &archive,
                &self.key,
                cache.scope,
                &Record {
                    id: crate::recovery::copy_id(parent),
                    revision: 1,
                    conversation: base.conversation,
                    author: base.author,
                    created_at: base.created_at,
                    direction: base.direction,
                    content: Content::Media {
                        record: parent,
                        original: original_hash,
                        file: bytes,
                    },
                },
            )?;
            archive.commit()?;
            target.managed = true;
            save(&tx, &cache.key, &target, false)?;
            tx.execute(
                "DELETE FROM recovery_transfers WHERE id=?1",
                [parent.as_slice()],
            )?;
            tx.commit()?;
            return Ok(MediaRecovery::Protected(target.file));
        }
        if target.phase != Phase::Staging {
            tx.commit()?;
            return Ok(MediaRecovery::Upload(target.file));
        }
        let index: u32 = tx.query_row(
            "SELECT count(*) FROM chunks WHERE file=?1",
            [target.file.as_slice()],
            |r| r.get(0),
        )?;
        let next = if index == target.shape().chunks()? {
            let (key, _) = target.key()?;
            target.descriptor = Some(key.descriptor(committed_root(&tx, &cache.key, &target)?));
            target.phase = Phase::Ready;
            save(&tx, &cache.key, &target, false)?;
            MediaRecovery::Upload(target.file)
        } else {
            let plaintext = download::completed(&tx, &cache.key, source.file, index, now)?;
            stage(&tx, &cache.key, &target, index, &plaintext)?;
            MediaRecovery::Staged(target.file, index)
        };
        tx.commit()?;
        Ok(next)
    }
    /// Fair archive enumeration; no file transfer starts before history import commits.
    pub fn prepare_recovery_media_step(
        &mut self,
        cache: &mut Cache,
        now: u64,
    ) -> Result<MediaRecovery, Error> {
        if !crate::recovery::configured(&self.db)? {
            return Ok(MediaRecovery::Idle);
        }
        events::source(self, cache)?;
        let tx = cache
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let raw: Option<Vec<u8>> = tx
            .query_row("SELECT state FROM recovery_cursor WHERE id=1", [], |r| {
                r.get(0)
            })
            .optional()?;
        let after = raw
            .map(|raw| cache.key.open(&raw, &binding(&cache.scope)))
            .transpose()?
            .unwrap_or_default();
        let candidate: Option<Vec<u8>> = self
            .db
            .query_row(
                "SELECT record FROM archive_media WHERE record>?1 ORDER BY record LIMIT 1",
                [after.as_slice()],
                |r| r.get(0),
            )
            .optional()?;
        let next = candidate.as_deref().unwrap_or(&[]);
        tx.execute("INSERT INTO recovery_cursor VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET state=excluded.state", [cache.key.seal(next, &binding(&cache.scope))?])?;
        tx.commit()?;
        let Some(id) = candidate else {
            return Ok(MediaRecovery::Idle);
        };
        let id: Id = id.try_into().map_err(|_| Error::InvalidStore)?;
        if matches!(self.recovery_record(id)?.content, Content::Media { .. }) {
            return Ok(MediaRecovery::Idle);
        }
        self.republish_recovery_media_step(cache, id, now)
    }
}
