use super::*;
use crate::attachments::{Cache, Phase};
use serde::Serialize;

pub(super) fn metadata(body: Option<&Body>) -> Result<Option<Value>, Error> {
    let Some(Body::File(bytes)) = body else {
        return Ok(None);
    };
    let file = sigil_protocol::file::File::from_bytes(bytes).map_err(|_| Error::InvalidStore)?;
    let key = sigil_protocol::file::KeyDescriptor::from_bytes(file.descriptor)
        .map_err(|_| Error::InvalidStore)?;
    Ok(Some(
        json!({"name":file.name,"media_type":file.media_type,"length":key.length,"caption":file.caption}),
    ))
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Upload {
    pub peer: String,
    pub request: Id,
    pub timestamp: u64,
    pub length: u64,
    pub name: String,
    pub media_type: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub caption: Zeroizing<String>,
    pub file: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<Reference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<Reference>,
}
impl ClientStore {
    pub(super) fn mobile_cache(&self) -> Result<Cache, Error> {
        let parent = std::path::Path::new(self.db.path().ok_or(Error::InvalidStore)?)
            .parent()
            .ok_or(Error::InvalidStore)?;
        self.open_attachment_cache(&parent.join("attachments.db"), 2 * 1024 * 1024 * 1024)
    }
    fn upload_aad(&self, request: Id) -> Result<Vec<u8>, Error> {
        Ok([
            b"Sigil/mobile-upload/v1".as_slice(),
            &self.connected_account_scope()?,
            &request,
        ]
        .concat())
    }
    pub(super) fn mobile_upload(&self, request: Id) -> Result<Upload, Error> {
        let bytes: Vec<u8> = self.db.query_row("SELECT CASE WHEN length(state)<=65536 THEN state END FROM mobile_uploads WHERE id=?1", [request.as_slice()], |r| r.get(0)).optional()?.ok_or(Error::NotFound)?;
        let upload: Upload =
            serde_json::from_slice(&self.key.open(&bytes, &self.upload_aad(request)?)?)
                .map_err(|_| Error::InvalidStore)?;
        if upload.request != request || upload.caption.len() > sigil_protocol::file::MAX_CAPTION {
            return Err(Error::InvalidStore);
        }
        Ok(upload)
    }
    fn upload_ids(&self) -> Result<Vec<Id>, Error> {
        let mut query = self
            .db
            .prepare("SELECT id FROM mobile_uploads ORDER BY id LIMIT 33")?;
        let ids = query
            .query_map([], |r| r.get::<_, Vec<u8>>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        if ids.len() > 32 {
            return Err(Error::Limit);
        }
        ids.into_iter()
            .map(|v| v.try_into().map_err(|_| Error::InvalidStore))
            .collect()
    }
    pub(super) fn mobile_file_begin(&mut self, mut upload: Upload) -> Result<Value, Error> {
        self.mobile_conversation(&upload.peer)?;
        if upload.peer != "self" && !upload.peer.starts_with("group:") {
            self.mobile_recipients(self.mobile_peer(&upload.peer)?)?;
        }
        let metadata = sigil_protocol::file::Metadata {
            name: upload.name.clone(),
            media_type: upload.media_type.clone(),
        };
        metadata.validate().map_err(|_| Error::InvalidEvent)?;
        if upload.timestamp == 0
            || upload.timestamp > i64::MAX as u64
            || upload.length > 1024 * 1024 * 1024
            || upload.caption.len() > sigil_protocol::file::MAX_CAPTION
        {
            return Err(Error::Limit);
        }
        match self.mobile_upload(upload.request) {
            Ok(previous) => {
                if previous.peer != upload.peer
                    || previous.length != upload.length
                    || previous.name != upload.name
                    || previous.media_type != upload.media_type
                    || previous.timestamp != upload.timestamp
                    || previous.reply != upload.reply
                    || previous.thread != upload.thread
                    || previous.draft != upload.draft
                    || previous.caption != upload.caption
                {
                    return Err(Error::Conflict);
                }
                return Ok(json!({"request":transport::hex(&upload.request)}));
            }
            Err(Error::NotFound) => {}
            Err(error) => return Err(error),
        }
        if self.upload_ids()?.len() >= 32 {
            return Err(Error::Limit);
        }
        let mut cache = self.mobile_cache()?;
        upload.file = cache.prepare_upload(upload.length, metadata, None)?;
        let bytes = Zeroizing::new(serde_json::to_vec(&upload).map_err(|_| Error::InvalidStore)?);
        let saved = (|| -> Result<(), Error> {
            let sealed = self.key.seal(&bytes, &self.upload_aad(upload.request)?)?;
            let tx = self
                .db
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let count: i64 =
                tx.query_row("SELECT count(*) FROM mobile_uploads", [], |r| r.get(0))?;
            if count >= 32 {
                return Err(Error::Limit);
            }
            tx.execute(
                "INSERT INTO mobile_uploads VALUES(?1,?2)",
                (upload.request.as_slice(), sealed),
            )?;
            tx.commit()?;
            Ok(())
        })();
        if let Err(error) = saved {
            cache.cancel(upload.file)?;
            return Err(error);
        }
        Ok(json!({"request":transport::hex(&upload.request)}))
    }
    pub fn mobile_file_stage(
        &mut self,
        request: &str,
        index: u32,
        bytes: &[u8],
    ) -> Result<(), Error> {
        if bytes.len() > sigil_protocol::file::CHUNK_SIZE {
            return Err(Error::Limit);
        }
        let upload = self.mobile_upload(id(request)?)?;
        self.mobile_cache()?
            .stage_chunk(upload.file, index, bytes)?;
        Ok(())
    }
    pub(super) fn mobile_files(&self) -> Result<Value, Error> {
        let cache = self.mobile_cache()?;
        let mut uploads = Vec::new();
        for id in self.upload_ids()? {
            let upload = self.mobile_upload(id)?;
            uploads.push(json!({"request":transport::hex(&id),"peer":upload.peer,"name":upload.name,"media_type":upload.media_type,"length":upload.length,"phase":format!("{:?}",cache.phase(upload.file)?),"draft":upload.draft,"caption":upload.caption.as_str()}));
        }
        Ok(json!({"uploads":uploads}))
    }
    pub(super) fn mobile_file_send(
        &mut self,
        request: Id,
        caption: Zeroizing<String>,
    ) -> Result<Value, Error> {
        if caption.len() > sigil_protocol::file::MAX_CAPTION {
            return Err(Error::Limit);
        }
        let upload = self.mobile_upload(request)?;
        if matches!(
            self.mobile_cache()?.phase(upload.file)?,
            Phase::Staging | Phase::Cancelled | Phase::Expired
        ) {
            return Err(Error::Unprepared);
        }
        let aad = self.upload_aad(request)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let bytes: Vec<u8> = tx.query_row(
            "SELECT CASE WHEN length(state)<=65536 THEN state END FROM mobile_uploads WHERE id=?1",
            [request.as_slice()],
            |r| r.get(0),
        )?;
        let mut current: Upload = serde_json::from_slice(&self.key.open(&bytes, &aad)?)
            .map_err(|_| Error::InvalidStore)?;
        if current.request != request || current.file != upload.file {
            return Err(Error::Conflict);
        }
        if !current.draft {
            if current.caption != caption {
                return Err(Error::Conflict);
            }
            return Ok(json!({}));
        }
        current.caption = caption;
        current.draft = false;
        current.timestamp = conversations::now();
        let encoded =
            Zeroizing::new(serde_json::to_vec(&current).map_err(|_| Error::InvalidStore)?);
        let sealed = self.key.seal(&encoded, &aad)?;
        tx.execute(
            "UPDATE mobile_uploads SET state=?1 WHERE id=?2",
            (sealed, request.as_slice()),
        )?;
        tx.commit()?;
        Ok(json!({}))
    }
    pub(super) fn mobile_file_work(&mut self) -> Result<Value, Error> {
        let mut cache = self.mobile_cache()?;
        let started = std::time::Instant::now();
        let mut result = self.sync_attachments_due_online(&mut cache)?;
        for _ in 1..8 {
            if result.scheduling_error.is_some()
                || !result.attempt.as_ref().is_some_and(|a| a.result.is_ok())
                || result.next_at > conversations::now()
                || started.elapsed().as_secs() >= 2
            {
                break;
            }
            result = self.sync_attachments_due_online(&mut cache)?;
        }
        let mut issue = result.scheduling_error.as_ref().map(error_message);
        if let Some(error) = result
            .attempt
            .as_ref()
            .and_then(|a| a.result.as_ref().err())
        {
            issue = Some(error_message(error));
        }
        let mut sent = 0;
        let mut pending = false;
        for id in self.upload_ids()? {
            let upload = self.mobile_upload(id)?;
            match cache.phase(upload.file)? {
                Phase::Published if !upload.draft => {
                    self.with_published_file(
                        &mut cache,
                        upload.file,
                        conversations::now(),
                        |store, bytes| {
                            let mut file = sigil_protocol::file::File::from_bytes(bytes)
                                .map_err(|_| Error::InvalidStore)?;
                            file.caption = &upload.caption;
                            let content = file.to_bytes().map_err(|_| Error::InvalidEvent)?;
                            store.mobile_action(
                                &upload.peer,
                                &transport::hex(&upload.request),
                                upload.timestamp,
                                Action::Post {
                                    body: Body::File(content),
                                    reply: upload.reply.clone(),
                                    thread: upload.thread.clone(),
                                    expires_at: None,
                                    view_once: false,
                                },
                            )?;
                            Ok(())
                        },
                    )?;
                    self.db
                        .execute("DELETE FROM mobile_uploads WHERE id=?1", [id.as_slice()])?;
                    sent += 1;
                }
                Phase::Cancelled | Phase::Expired => {
                    self.db
                        .execute("DELETE FROM mobile_uploads WHERE id=?1", [id.as_slice()])?;
                }
                Phase::Staging | Phase::Published => {}
                _ => pending = true,
            }
        }
        let now = conversations::now();
        let mut next = result.next_at.min(now.saturating_add(30));
        let background = (|| -> Result<(), Error> {
            self.maintain_history(now)?;
            self.erase_obsolete_journals(now)?;
            if recovery::configured(&self.db)? {
                let work = self.sync_recovery_due_online()?;
                next = next.min(work.next_at);
                if let Some(error) = work.scheduling_error {
                    return Err(error);
                }
                if let Some(progress) = work.progress {
                    pending |= !matches!(progress?, recovery::RecoveryProgress::Idle);
                }
                pending |= self.recovery_status()?.pending.is_some();
                pending |= self.history_recovery_progress()?.unprotected_records > 0;
            }
            pending |= matches!(
                self.prepare_recovery_media_step(&mut cache, now)?,
                attachments::MediaRecovery::Download(_)
                    | attachments::MediaRecovery::Staged(_, _)
                    | attachments::MediaRecovery::Upload(_)
            );
            Ok(())
        })();
        if let Err(error) = background {
            if issue.is_none() {
                issue = Some(error_message(&error));
            }
            pending = true;
        }
        if self.push_state()?.configured {
            match self.sync_push_due_online() {
                Ok(work) => {
                    next = next.min(work.next_at);
                    if let Some(error) = work
                        .scheduling_error
                        .as_ref()
                        .or_else(|| work.progress.as_ref().and_then(|p| p.as_ref().err()))
                    {
                        issue.get_or_insert(error_message(error));
                    }
                    pending |= work.next_at <= now.saturating_add(900);
                }
                Err(error) => {
                    issue.get_or_insert(error_message(&error));
                    pending = true;
                }
            }
        }
        Ok(json!({"next_at":next,"sent":sent,"issue":issue,"pending":pending}))
    }
    pub(super) fn mobile_file_get(
        &mut self,
        peer: &str,
        reference: Reference,
    ) -> Result<Value, Error> {
        let conversation = self.mobile_conversation(peer)?;
        let message =
            self.conversation_message(conversation, reference.clone(), conversations::now())?;
        if message.view_once {
            return Err(Error::Unprepared);
        }
        let Some(Body::File(bytes)) = message.body else {
            return Err(Error::InvalidEvent);
        };
        let file =
            sigil_protocol::file::File::from_bytes(&bytes).map_err(|_| Error::InvalidStore)?;
        let key = sigil_protocol::file::KeyDescriptor::from_bytes(file.descriptor)
            .map_err(|_| Error::InvalidStore)?;
        let mut cache = self.mobile_cache()?;
        let id = self.prepare_conversation_file(
            &mut cache,
            conversation,
            reference,
            conversations::now(),
        )?;
        Ok(
            json!({"name":file.name,"media_type":file.media_type,"length":key.length,"caption":file.caption,"phase":format!("{:?}",cache.phase(id)?)}),
        )
    }
    pub fn mobile_file_chunk(
        &mut self,
        peer: &str,
        author: &str,
        message: &str,
        index: u32,
    ) -> Result<Zeroizing<Vec<u8>>, Error> {
        let conversation = self.mobile_conversation(peer)?;
        self.conversation_file_chunk(
            &self.mobile_cache()?,
            conversation,
            reference(author, message)?,
            index,
            conversations::now(),
        )
    }
    pub fn mobile_file_draft_chunk(
        &self,
        request: &str,
        index: u32,
    ) -> Result<Zeroizing<Vec<u8>>, Error> {
        let upload = self.mobile_upload(id(request)?)?;
        if !upload.draft {
            return Err(Error::Unprepared);
        }
        self.mobile_cache()?
            .staged_chunk(upload.file, index, conversations::now())
    }
}
