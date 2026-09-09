//! Paginated observations; inventory never authorizes encryption trust changes.
use super::*;
use sigil_protocol::accounts::DeviceSummary;

pub struct DeviceReview {
    pub device: Id,
    pub is_current: bool,
    pub inventory: Option<DeviceSummary>,
    pub peer: Option<Peer>,
}
/// Continue both passes to include local devices omitted by the server. Entries
/// can repeat between passes: merge observations by device, retaining populated
/// fields. None means no observation in this page, not revocation or deletion.
pub struct DeviceReviewPage {
    pub devices: Vec<DeviceReview>,
    pub next: Option<DeviceReviewCursor>,
}
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceReviewCursor {
    session: sigil_protocol::accounts::Session,
    local: bool,
    after: Option<Id>,
}
impl ClientStore {
    /// One server inventory page or 16 authenticated local peers. No cumulative
    /// device-history limit; cursors are observations, not authorization tokens.
    pub fn review_devices_online(
        &self,
        cursor: Option<&DeviceReviewCursor>,
    ) -> Result<DeviceReviewPage, Error> {
        let session = self.connection_session()?.ok_or(Error::Unprepared)?;
        if cursor.is_some_and(|c| c.session != session) {
            return Err(Error::Conflict);
        }
        let current = id(&session.device_id)?;
        let account = id(&session.account_id)?;
        let (username, server) = session
            .address
            .strip_prefix('@')
            .and_then(|v| v.split_once(':'))
            .ok_or(Error::InvalidStore)?;
        let local = cursor.is_some_and(|c| c.local);
        let after = cursor.and_then(|c| c.after);
        let mut devices = Vec::new();
        let next;
        if local {
            let tx = self.db.unchecked_transaction()?;
            let ids: Vec<Vec<u8>> = tx
                .prepare("SELECT id FROM peers WHERE id>?1 ORDER BY id LIMIT 16")?
                .query_map([after.map_or(Vec::new(), |id| id.to_vec())], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            for raw in &ids {
                let reference: Id = raw.as_slice().try_into().map_err(|_| Error::InvalidStore)?;
                let peer = super::load(&tx, &self.key, &reference)?.public()?;
                if peer.binding.server != server || peer.binding.account != account {
                    continue;
                }
                if peer.binding.username != username {
                    return Err(Error::Conflict);
                }
                devices.push(DeviceReview {
                    device: peer.binding.device,
                    is_current: peer.binding.device == current,
                    inventory: None,
                    peer: Some(peer),
                });
            }
            next = ids
                .last()
                .map(|raw| {
                    Ok::<_, Error>(DeviceReviewCursor {
                        session: session.clone(),
                        local: true,
                        after: Some(raw.as_slice().try_into().map_err(|_| Error::InvalidStore)?),
                    })
                })
                .transpose()?;
            tx.commit()?;
        } else {
            let encoded = after.map(|id| transport::hex(&id));
            let page = self.connected_client()?.devices(encoded.as_deref())?;
            if page.account_id != session.account_id {
                return Err(Error::Conflict);
            }
            let last = page.devices.last().map(|d| id(&d.id)).transpose()?;
            if after.is_none_or(|a| current > a)
                && (page.next_after.is_none() || last.is_some_and(|last| current <= last))
                && !page.devices.iter().any(|d| d.id == session.device_id)
            {
                return Err(Error::Conflict);
            }
            let tx = self.db.unchecked_transaction()?;
            for entry in page.devices {
                let device = id(&entry.id)?;
                let reference = reference(server, &device);
                let peer = match super::load(&tx, &self.key, &reference) {
                    Ok(record) => {
                        let peer = record.public()?;
                        if peer.binding.server != server
                            || peer.binding.account != account
                            || peer.binding.username != username
                        {
                            return Err(Error::Conflict);
                        }
                        Some(peer)
                    }
                    Err(Error::NotFound) => None,
                    Err(error) => return Err(error),
                };
                devices.push(DeviceReview {
                    device,
                    is_current: device == current,
                    inventory: Some(entry),
                    peer,
                });
            }
            next = Some(DeviceReviewCursor {
                session: session.clone(),
                local: page.next_after.is_none(),
                after: page.next_after.as_deref().map(id).transpose()?,
            });
            tx.commit()?;
        }
        if self.connection_session()?.as_ref() != Some(&session) {
            return Err(Error::Conflict);
        }
        Ok(DeviceReviewPage { devices, next })
    }
}
