use super::*;
use rusqlite::OptionalExtension;
use serde::Serialize;
use sigil_protocol::profile::{ContactProfile, SetPhoto, ShareProfile, MAX_PHOTO};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cached {
    value: ContactProfile,
    at: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Upload {
    id: Id,
    expected: Option<u64>,
    image: Zeroizing<String>,
}
const CACHE: usize = 16 * 1024 * 1024;
#[cfg(test)]
#[path = "mobile_profile_tests.rs"]
mod tests;
fn aad(scope: &Id, target: Id, kind: &[u8]) -> Vec<u8> {
    [b"Sigil/mobile-profile/v1".as_slice(), scope, &target, kind].concat()
}
pub(crate) fn migrate(db: &Connection, key: &StorageKey) -> Result<(), Error> {
    if !db.query_row("SELECT EXISTS(SELECT 1 FROM mobile_profiles WHERE image IS NOT NULL) OR EXISTS(SELECT 1 FROM mobile_photo_upload)", [], |r| r.get::<_, bool>(0))? { return Ok(()); }
    let scope = connection::persisted_account_scope(db, key)?;
    let mut statement =
        db.prepare("SELECT id,CASE WHEN length(image)>67266 THEN X'' ELSE image END FROM mobile_profiles WHERE image IS NOT NULL ORDER BY id")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let target: Vec<u8> = row.get(0)?;
        let target: Id = target.try_into().map_err(|_| Error::InvalidStore)?;
        let old: Vec<u8> = row.get(1)?;
        let binding = aad(&scope, target, b"image");
        let plain = key.open(&old, &binding)?;
        let sealed = if plain.is_empty() {
            None
        } else {
            Some(storage_blob::seal(key, &binding, &plain)?)
        };
        db.execute(
            "UPDATE mobile_profiles SET image=?2 WHERE id=?1",
            (target.as_slice(), sealed),
        )?;
    }
    let old: Option<Vec<u8>> = db
        .query_row(
            "SELECT CASE WHEN length(state)>263168 THEN X'' ELSE state END FROM mobile_photo_upload WHERE id=1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(old) = old {
        let binding = aad(&scope, [0; 32], b"upload");
        let plain = key.open(&old, &binding)?;
        db.execute(
            "UPDATE mobile_photo_upload SET state=?1 WHERE id=1",
            [storage_blob::seal(key, &binding, &plain)?],
        )?;
    }
    Ok(())
}
impl ClientStore {
    fn profile_aad(&self, target: Id, kind: &[u8]) -> Result<Vec<u8>, Error> {
        Ok(aad(&self.connected_account_scope()?, target, kind))
    }
    fn cached_profile(&self, target: Id) -> Result<Option<Cached>, Error> {
        let bytes: Option<Vec<u8>> = self
            .db
            .query_row(
                "SELECT CASE WHEN length(metadata)>8192 THEN X'' ELSE metadata END FROM mobile_profiles WHERE id=?1",
                [target.as_slice()],
                |r| r.get(0),
            )
            .optional()?;
        bytes
            .map(|bytes| {
                if bytes.len() > 8192 {
                    return Err(Error::InvalidStore);
                }
                serde_json::from_slice(
                    &self
                        .key
                        .open(&bytes, &self.profile_aad(target, b"metadata")?)?,
                )
                .map_err(|_| Error::InvalidStore)
            })
            .transpose()
    }
    fn cache_profile(
        &mut self,
        target: Id,
        metadata: &Cached,
        image: Option<&[u8]>,
    ) -> Result<(), Error> {
        if image.is_some_and(|bytes| bytes.len() > MAX_PHOTO) {
            return Err(Error::Limit);
        }
        let aad = self.profile_aad(target, b"metadata")?;
        let image_aad = self.profile_aad(target, b"image")?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous: Option<Vec<u8>> = tx
            .query_row(
                "SELECT CASE WHEN length(metadata)>8192 THEN X'' ELSE metadata END FROM mobile_profiles WHERE id=?1",
                [target.as_slice()],
                |r| r.get(0),
            )
            .optional()?;
        let previous: Option<Cached> = previous
            .map(|bytes| {
                if bytes.len() > 8192 {
                    return Err(Error::InvalidStore);
                }
                serde_json::from_slice(&self.key.open(&bytes, &aad)?)
                    .map_err(|_| Error::InvalidStore)
            })
            .transpose()?;
        if previous.as_ref().is_some_and(|p| {
            p.value.photo.revision > metadata.value.photo.revision
                || p.value.profile.revision > metadata.value.profile.revision
                || (p.value.photo.revision == metadata.value.photo.revision
                    && p.value.photo != metadata.value.photo)
                || (p.value.profile.revision == metadata.value.profile.revision
                    && p.value.profile != metadata.value.profile)
        }) {
            return Err(Error::Conflict);
        }
        let keep = previous
            .as_ref()
            .is_some_and(|p| p.value.photo == metadata.value.photo);
        let sealed = self.key.seal(
            &Zeroizing::new(serde_json::to_vec(metadata).map_err(|_| Error::InvalidStore)?),
            &aad,
        )?;
        let photo = image
            .filter(|image| !image.is_empty())
            .map(|image| storage_blob::seal(&self.key, &image_aad, image))
            .transpose()?;
        if tx.query_row("SELECT count(*)>=4096 AND NOT EXISTS(SELECT 1 FROM mobile_profiles WHERE id=?1) FROM mobile_profiles",[target.as_slice()],|r|r.get::<_,bool>(0))? {
            tx.execute("DELETE FROM mobile_profiles WHERE id=(SELECT id FROM mobile_profiles ORDER BY touched,id LIMIT 1)", [])?;
        }
        tx.execute("INSERT INTO mobile_profiles VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET metadata=excluded.metadata,image=CASE WHEN excluded.image IS NOT NULL THEN excluded.image WHEN ?5 THEN mobile_profiles.image ELSE NULL END,touched=excluded.touched",(target.as_slice(),sealed,photo,conversations::now().min(i64::MAX as u64) as i64,keep))?;
        while tx.query_row(
            "SELECT coalesce(sum(length(image)),0) FROM mobile_profiles",
            [],
            |r| r.get::<_, i64>(0),
        )? > CACHE as i64
        {
            if tx.execute("UPDATE mobile_profiles SET image=NULL WHERE id=(SELECT id FROM mobile_profiles WHERE image IS NOT NULL AND id!=?1 ORDER BY touched,id LIMIT 1)",[target.as_slice()])?!=1{return Err(Error::Limit);}
        }
        tx.commit()?;
        Ok(())
    }
    fn photo_upload(&self) -> Result<Option<Upload>, Error> {
        let bytes: Option<Vec<u8>> = self
            .db
            .query_row(
                "SELECT CASE WHEN length(state)>263168 THEN X'' ELSE state END FROM mobile_photo_upload WHERE id=1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        bytes
            .map(|bytes| {
                if bytes.len() > 2 * MAX_PHOTO + 1024 {
                    return Err(Error::InvalidStore);
                }
                serde_json::from_slice(&storage_blob::open(
                    &self.key,
                    &self.profile_aad([0; 32], b"upload")?,
                    &bytes,
                    2 * MAX_PHOTO + 512,
                )?)
                .map_err(|_| Error::InvalidStore)
            })
            .transpose()
    }
    fn save_photo_upload(&mut self, value: &Upload) -> Result<(), Error> {
        let bytes = Zeroizing::new(serde_json::to_vec(value).map_err(|_| Error::InvalidStore)?);
        if bytes.len() > 2 * MAX_PHOTO + 512 {
            return Err(Error::Limit);
        }
        let sealed = storage_blob::seal(&self.key, &self.profile_aad([0; 32], b"upload")?, &bytes)?;
        self.db.execute("INSERT INTO mobile_photo_upload VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET state=excluded.state",[sealed])?;
        Ok(())
    }
    fn update_photo_upload(&mut self, id: Id, value: Option<&Upload>) -> Result<bool, Error> {
        let aad = self.profile_aad([0; 32], b"upload")?;
        let replacement = value
            .map(|value| -> Result<_, Error> {
                let bytes =
                    Zeroizing::new(serde_json::to_vec(value).map_err(|_| Error::InvalidStore)?);
                if bytes.len() > 2 * MAX_PHOTO + 512 {
                    return Err(Error::Limit);
                }
                storage_blob::seal(&self.key, &aad, &bytes)
            })
            .transpose()?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: Option<Vec<u8>> = tx
            .query_row(
                "SELECT CASE WHEN length(state)>263168 THEN X'' ELSE state END FROM mobile_photo_upload WHERE id=1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let Some(current) = current else {
            return Ok(false);
        };
        if current.len() > 2 * MAX_PHOTO + 1024 {
            return Err(Error::InvalidStore);
        }
        let current: Upload = serde_json::from_slice(&storage_blob::open(
            &self.key,
            &aad,
            &current,
            2 * MAX_PHOTO + 512,
        )?)
        .map_err(|_| Error::InvalidStore)?;
        if current.id != id {
            return Ok(false);
        }
        match replacement {
            Some(bytes) => {
                tx.execute(
                    "UPDATE mobile_photo_upload SET state=?1 WHERE id=1",
                    [bytes],
                )?;
            }
            None => {
                tx.execute("DELETE FROM mobile_photo_upload WHERE id=1", [])?;
            }
        }
        tx.commit()?;
        Ok(true)
    }
    pub fn mobile_stage_photo(&mut self, image: &[u8]) -> Result<(), Error> {
        if image.len() > MAX_PHOTO {
            return Err(Error::Limit);
        }
        let mut id = [0; 32];
        getrandom::fill(&mut id).map_err(|_| sigil_crypto::Error::Entropy)?;
        let encoded = Zeroizing::new(transport::hex(image));
        sigil_protocol::profile::photo_bytes(&encoded).map_err(|_| Error::InvalidEvent)?;
        self.save_photo_upload(&Upload {
            id,
            expected: None,
            image: encoded,
        })
    }
    pub(super) fn mobile_photo_publish(&mut self) -> Result<Value, Error> {
        let Some(mut upload) = self.photo_upload()? else {
            return self.mobile_photo_status();
        };
        let network = self.connected_client()?;
        if upload.expected.is_none() {
            upload.expected = Some(network.profile_photo()?.revision);
            if !self.update_photo_upload(upload.id, Some(&upload))? {
                return self.mobile_photo_status();
            }
        }
        let request = SetPhoto {
            revision: upload.expected.ok_or(Error::InvalidStore)?,
            photo: upload.image.to_string(),
        };
        let photo = network.set_profile_photo(&request)?;
        let target = self.account_reference()?;
        let cached = Cached {
            value: ContactProfile {
                profile: network.profile()?,
                photo,
            },
            at: conversations::now(),
        };
        let bytes = Zeroizing::new(
            sigil_protocol::profile::photo_bytes(&upload.image).map_err(|_| Error::InvalidStore)?,
        );
        self.cache_profile(target, &cached, Some(&bytes))?;
        self.update_photo_upload(upload.id, None)?;
        self.mobile_photo_status()
    }
    pub(super) fn mobile_photo_retry(&mut self) -> Result<Value, Error> {
        if let Some(mut upload) = self.photo_upload()? {
            let previous = upload.id;
            getrandom::fill(&mut upload.id).map_err(|_| sigil_crypto::Error::Entropy)?;
            upload.expected = None;
            if !self.update_photo_upload(previous, Some(&upload))? {
                return self.mobile_photo_status();
            }
        }
        self.mobile_photo_publish()
    }
    pub(super) fn mobile_photo_status(&mut self) -> Result<Value, Error> {
        let target = self.account_reference()?;
        let cached = self.cached_profile(target)?;
        Ok(
            json!({"pending":self.photo_upload()?.is_some(),"photo":cached.map(|c|c.value.photo).unwrap_or_default(),"avatar":transport::hex(&target)}),
        )
    }
    pub(super) fn photo_upload_pending(&self) -> Result<bool, Error> {
        Ok(self.photo_upload()?.is_some())
    }
    pub(super) fn mobile_profile_name(&self, reference: Id) -> Result<Option<String>, Error> {
        Ok(self
            .cached_profile(reference)?
            .map(|v| v.value.profile.display_name)
            .filter(|name| !name.is_empty()))
    }
    pub(super) fn share_peer_profile(&self, peer: Id) -> Result<(), Error> {
        let peer = peers::verified(&self.db, &self.key, &peer)?;
        let own = self.connection_session()?.ok_or(Error::Unprepared)?;
        if own.account_id == transport::hex(&peer.binding.account)
            && own.address.split_once(':').ok_or(Error::InvalidStore)?.1 == peer.binding.server
        {
            return Ok(());
        }
        match self.connected_client()?.share_profile(&ShareProfile {
            server: peer.binding.server,
            account: transport::hex(&peer.binding.account),
            allowed: true,
        }) {
            Ok(()) | Err(network::Error::Status { code: 404, .. }) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
    pub fn mobile_profile_image(
        &mut self,
        reference: &str,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, Error> {
        let target = id(reference)?;
        let own = self.connection_session()?.ok_or(Error::Unprepared)?;
        let own_ref = self.account_reference()?;
        if target == own_ref {
            if let Some(upload) = self.photo_upload()? {
                let bytes = Zeroizing::new(
                    sigil_protocol::profile::photo_bytes(&upload.image)
                        .map_err(|_| Error::InvalidStore)?,
                );
                return Ok((!bytes.is_empty()).then_some(bytes));
            }
        }
        let (server, account, peer) = if target == own_ref {
            (
                own.address
                    .split_once(':')
                    .ok_or(Error::InvalidStore)?
                    .1
                    .to_owned(),
                own.account_id.clone(),
                None,
            )
        } else {
            let peer = self
                .mobile_peers()?
                .into_iter()
                .find(|p| event::account_reference(&p.binding.server, &p.binding.account) == target)
                .ok_or(Error::NotFound)?;
            self.mobile_recipients(peer.id)?;
            (
                peer.binding.server,
                transport::hex(&peer.binding.account),
                Some(peer.id),
            )
        };
        let network = self.connected_client()?;
        let now = conversations::now();
        let mut metadata = self.cached_profile(target)?;
        if metadata
            .as_ref()
            .is_none_or(|cached| now < cached.at || now - cached.at >= 300)
        {
            if let Some(peer) = peer {
                self.share_peer_profile(peer)?;
            }
            let value = match network.contact_profile(&own, &server, &account) {
                Ok(value) => value,
                Err(network::Error::Status {
                    code: 403 | 404, ..
                }) => {
                    self.db.execute(
                        "DELETE FROM mobile_profiles WHERE id=?1",
                        [target.as_slice()],
                    )?;
                    return Ok(None);
                }
                Err(error) => return Err(error.into()),
            };
            let updated = Cached { value, at: now };
            self.cache_profile(target, &updated, None)?;
            metadata = Some(updated);
        }
        let metadata = metadata.ok_or(Error::NotFound)?;
        let Some(hash) = &metadata.value.photo.hash else {
            return Ok(None);
        };
        let cached: Option<Vec<u8>> = self.db.query_row(
            "SELECT CASE WHEN length(image)>131200 THEN X'' ELSE image END FROM mobile_profiles WHERE id=?1",
            [target.as_slice()],
            |r| r.get(0),
        )?;
        let bytes = match cached {
            Some(bytes) => {
                if bytes.len() > MAX_PHOTO + 128 {
                    return Err(Error::InvalidStore);
                }
                storage_blob::open(
                    &self.key,
                    &self.profile_aad(target, b"image")?,
                    &bytes,
                    MAX_PHOTO,
                )?
            }
            None => {
                let bytes =
                    network.contact_photo(&own, &server, &account, &metadata.value.photo)?;
                if let Some(peer) = peer {
                    self.mobile_recipients(peer)?;
                }
                self.cache_profile(target, &metadata, Some(&bytes))?;
                bytes
            }
        };
        if bytes.len() != metadata.value.photo.bytes as usize
            || transport::hex(&Sha256::digest(&bytes)) != *hash
        {
            return Err(Error::InvalidStore);
        }
        Ok(Some(bytes))
    }
}
