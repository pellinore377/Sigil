//! Bounded background group ordering, admission and key-distribution work.
use super::*;
use crate::ClientStore;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use sigil_crypto::storage::StorageKey;

pub(crate) const MIGRATION: &str = "
CREATE TABLE group_work(id BLOB PRIMARY KEY,state BLOB NOT NULL);
CREATE TABLE group_credentials(id BLOB PRIMARY KEY,state BLOB NOT NULL);
CREATE TABLE group_channels(id BLOB PRIMARY KEY REFERENCES sessions(id),state BLOB NOT NULL);
PRAGMA user_version=57;";
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Cursor {
    pub(super) after: Option<Id>,
    pub(super) relay: Option<String>,
}
pub(super) fn load(
    db: &rusqlite::Connection,
    key: &StorageKey,
    own: &Id,
    id: &[u8],
) -> Result<Cursor, Error> {
    let value: Option<Vec<u8>> = db
        .query_row(
            "SELECT CASE WHEN length(state)<=512 THEN state END FROM group_work WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    match value {
        None => Ok(Cursor::default()),
        Some(value) => {
            let value: Cursor =
                serde_json::from_slice(&key.open(&value, &crate::binding(57, own, id))?)
                    .map_err(|_| Error::InvalidStore)?;
            if value.relay.as_ref().is_some_and(|v| {
                v.len() != 128
                    || !v
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            }) {
                return Err(Error::InvalidStore);
            }
            Ok(value)
        }
    }
}
pub(super) fn save(
    db: &rusqlite::Connection,
    key: &StorageKey,
    own: &Id,
    id: &[u8],
    cursor: &Cursor,
) -> Result<(), Error> {
    let raw = serde_json::to_vec(cursor).map_err(|_| Error::InvalidStore)?;
    db.execute(
        "INSERT INTO group_work VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET state=excluded.state",
        (id, key.seal(&raw, &crate::binding(57, own, id))?),
    )?;
    Ok(())
}
#[derive(Debug, Default)]
pub struct GroupWork {
    pub synchronized: bool,
    pub ordered: bool,
    pub distribution: Option<Id>,
}
#[derive(Debug)]
pub struct GroupWorkAttempt {
    pub group: Id,
    pub result: Result<GroupWork, Error>,
}
impl ClientStore {
    /// Process one pinned group per pass and one member device within it.
    /// Durable cursors advance before work, so malformed state or an unavailable
    /// member cannot starve other groups/devices. Network errors retain journals.
    pub fn resume_groups_online(&mut self, now: u64) -> Result<Vec<GroupWorkAttempt>, Error> {
        if now == 0 || now > i64::MAX as u64 {
            return Err(Error::Expired);
        }
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let tx = self.db.transaction()?;
        let cursor = load(&tx, &self.key, &own, b"")?;
        let after = cursor.after.map(|v| v.to_vec()).unwrap_or_default();
        let next: Option<Vec<u8>> = tx
            .query_row(
                "SELECT group_id FROM group_service WHERE group_id>?1 ORDER BY group_id LIMIT 1",
                [after],
                |r| r.get(0),
            )
            .optional()?;
        let group = next
            .map(|v| v.try_into().map_err(|_| Error::InvalidStore))
            .transpose()?;
        save(
            &tx,
            &self.key,
            &own,
            b"",
            &Cursor {
                after: group,
                relay: None,
            },
        )?;
        tx.commit()?;
        Ok(group
            .map(|group| GroupWorkAttempt {
                group,
                result: self.group_work_online(group, now),
            })
            .into_iter()
            .collect())
    }
    fn group_work_online(&mut self, group: Id, now: u64) -> Result<GroupWork, Error> {
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let status = self.group_status(group)?;
        if status.frozen {
            return Err(Error::Conflict);
        }
        let mut progress = GroupWork::default();
        if status.state.device(own).is_err() {
            let tx = self.db.transaction()?;
            if invitation::awaiting_bootstrap(&tx, &self.key, &own, &group)? {
                return Ok(progress);
            }
        }
        if self.group_service_request_pending(group)? {
            let role = status.state.device(own).ok().map(|(m, _)| m.role);
            let result = if role == Some(Role::Member) {
                self.submit_group_relay_online(group, now)
            } else {
                match self.submit_group_service_online(group, now) {
                    Ok(_) => Ok(true),
                    Err(Error::Network(crate::network::Error::Status {
                        code: 403 | 409, ..
                    })) if role.is_none() => self.submit_group_relay_online(group, now),
                    Err(e) => Err(e),
                }
            };
            match result {
                Ok(ordered) => progress.ordered = ordered,
                Err(Error::Network(crate::network::Error::Status { code: 409, .. })) => {
                    progress.synchronized = self.sync_group_service_online(group, now)?;
                    return Ok(progress);
                }
                Err(error) => return Err(error),
            }
        }
        let status = self.group_status(group)?;
        if status.state.closed || (progress.ordered && status.state.device(own).is_err()) {
            return Ok(progress);
        }
        progress.synchronized = self.sync_group_service_online(group, now)?;
        if !progress.synchronized {
            return Ok(progress);
        }
        let status = self.group_status(group)?;
        if status.state.closed || status.state.device(own).is_err() {
            return Ok(progress);
        }
        let mut cursor = load(&self.db, &self.key, &own, &group)?;
        if status.state.device(own)?.0.role == Role::Admin
            && !self.group_service_request_pending(group)?
        {
            let proposals = self.group_relay_proposals_online(group, cursor.relay.clone(), now)?;
            cursor.relay = proposals.first().map(|p| p.author.clone());
            save(&self.db, &self.key, &own, &group, &cursor)?;
            if let Some(proposal) = proposals.first() {
                self.prepare_group_relay_commit(group, proposal)?;
                self.submit_group_service_online(group, now)?;
                progress.ordered = true;
            }
        }
        let state = self.group_status(group)?.state;
        if state.closed || state.device(own).is_err() {
            return Ok(progress);
        }
        let mut targets = state
            .members
            .iter()
            .flat_map(|m| &m.devices)
            .map(|v| Ok((peers::fingerprint(&v.binding)?, v)))
            .collect::<Result<Vec<_>, Error>>()?;
        targets.sort_by_key(|v| v.0);
        let target = targets
            .into_iter()
            .find(|(fp, _)| *fp != own && cursor.after.is_none_or(|after| *fp > after));
        cursor.after = target.as_ref().map(|v| v.0);
        save(&self.db, &self.key, &own, &group, &cursor)?;
        if let Some((_, target)) = target {
            let peer =
                self.observe_peer_binding(&target.to_bytes().map_err(|_| Error::InvalidEvent)?)?;
            let tx = self.db.transaction()?;
            let current = keys::current(&tx, &self.key, &own, &group)?;
            let peer = keys::authorized_peer(&tx, &self.key, &current, &peer.id)?;
            tx.commit()?;
            self.allow_known_sender_online(&peer)?;
            progress.distribution =
                Some(self.prepare_group_distribution_online(group, peer.id, now)?);
            self.group_key_recovery_work_online(group, peer.id, now)?;
        }
        Ok(progress)
    }
}

impl ClientStore {
    pub(super) fn cached_group_credential(
        &mut self,
        client: &crate::network::HttpsClient,
        profile: &sigil_crypto::private_group::Authority,
        binding: &[u8],
        now: u64,
    ) -> Result<sigil_crypto::private_credentials::Credential, Error> {
        use sigil_crypto::private_credentials::Credential;
        let day = u32::try_from(now / 86400).map_err(|_| Error::Expired)?;
        let own = device_fingerprint(binding)?;
        let id = profile.id();
        let aad = crate::binding(58, &own, &id);
        let cached: Option<Vec<u8>> = self.db.query_row("SELECT CASE WHEN length(state)=312 THEN state END FROM group_credentials WHERE id=?1",[id.as_slice()],|r|r.get(0)).optional()?;
        if let Some(cached) = cached {
            let raw = self.key.open(&cached, &aad)?;
            let (encoded_day, sealed) = raw.split_at_checked(4).ok_or(Error::InvalidStore)?;
            let stored_day =
                u32::from_be_bytes(encoded_day.try_into().map_err(|_| Error::InvalidStore)?);
            let issuance = profile.issuance(binding, stored_day)?;
            let credential = Credential::open_checkpoint(
                &self.key,
                &aad,
                sealed,
                profile.issuer(),
                &issuance.uid,
                stored_day,
            )?;
            if stored_day > day {
                return Err(Error::Expired);
            }
            if stored_day == day {
                return Ok(credential);
            }
        }
        let credential = client.group_credential(profile, binding, now)?;
        let mut bytes = zeroize::Zeroizing::new(day.to_be_bytes().to_vec());
        bytes.extend_from_slice(&credential.seal_checkpoint(&self.key, &aad)?);
        // A concurrent worker may have cached a newer day while this request ran.
        let tx = self.db.transaction()?;
        let prior: Option<Vec<u8>> = tx.query_row("SELECT CASE WHEN length(state)=312 THEN state END FROM group_credentials WHERE id=?1",[id.as_slice()],|r|r.get(0)).optional()?;
        if let Some(prior) = prior {
            let raw = self.key.open(&prior, &aad)?;
            if raw.len() != 276 {
                return Err(Error::InvalidStore);
            }
            let prior_day =
                u32::from_be_bytes(raw[..4].try_into().map_err(|_| Error::InvalidStore)?);
            if prior_day > day {
                return Err(Error::Expired);
            }
        }
        tx.execute("INSERT INTO group_credentials VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET state=excluded.state",(id.as_slice(),self.key.seal(&bytes,&aad)?))?;
        tx.commit()?;
        Ok(credential)
    }
}
