use super::*;
use crate::groups::{Change, GroupProfile, Role};

impl ClientStore {
    pub(super) fn mobile_leave_group(&mut self, peer: &str) -> Result<Value, Error> {
        let group = id(peer.strip_prefix("group:").ok_or(Error::InvalidEvent)?)?;
        let now = conversations::now();
        if self.group_service_request_pending(group)? {
            self.submit_group_service_online(group, now)?;
        }
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let state = self.group_status(group)?.state;
        if state.is_closed() {
            return Ok(json!({}));
        }
        let Some(member) = state.members().iter().find(|member| {
            member
                .device_fingerprints()
                .is_ok_and(|devices| devices.contains(&own))
        }) else {
            return Ok(json!({}));
        };
        let change = if state.members().len() == 1 {
            Change::Close
        } else {
            Change::Remove(member.id())
        };
        let proposal = self.prepare_group_change(group, change)?;
        self.prepare_group_service_request(group, Some(&proposal))?;
        self.submit_group_service_online(group, now)?;
        Ok(json!({}))
    }
    pub(super) fn mobile_create_group(
        &mut self,
        request: Id,
        timestamp: u64,
        name: String,
        description: String,
        peers: Vec<String>,
    ) -> Result<Value, Error> {
        if peers.is_empty() || peers.len() >= groups::MAX_MEMBERS {
            return Err(Error::Limit);
        }
        let now = conversations::now();
        let expires = timestamp
            .checked_add(604800)
            .filter(|v| *v > now && timestamp <= now)
            .ok_or(Error::Expired)?;
        let mut recipients = std::collections::BTreeSet::new();
        for peer in peers {
            let peers = self.mobile_recipients(self.mobile_peer(&peer)?)?;
            let peer = *peers.first().ok_or(Error::Unprepared)?;
            if !recipients.insert(peer) {
                return Err(Error::Conflict);
            }
        }
        let profile = GroupProfile { name, description };
        profile.validate()?;
        // Creating a group trusts this account's HTTPS server as its initial authority.
        let authority = self.connected_client()?.bootstrap_group_authority()?;
        let group = self.create_group_with_nonce(authority.fingerprint(), request)?;
        let pinned: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM group_service WHERE group_id=?1)",
            [group.as_slice()],
            |r| r.get(0),
        )?;
        if !pinned {
            let mut master = Zeroizing::new([0; 32]);
            getrandom::fill(master.as_mut())
                .map_err(|_| Error::Crypto(sigil_crypto::Error::Entropy))?;
            self.pin_group_service(group, &authority.to_bytes(), master)?;
        }
        let state = self.group_status(group)?.state;
        if state.revision() == 0 {
            self.prepare_group_service_request(group, None)?;
            self.submit_group_service_online(group, now)?;
        }
        let state = self.group_status(group)?.state;
        match state.profile() {
            Some(existing) if existing != &profile => return Err(Error::Conflict),
            Some(_) => {}
            None => {
                let proposal = self.prepare_group_change(group, Change::Profile(profile))?;
                self.prepare_group_service_request(group, Some(&proposal))?;
                self.submit_group_service_online(group, now)?;
            }
        }
        for peer in recipients {
            let invitation: Id = Sha256::digest(
                [
                    b"Sigil/mobile-group-invitation/v1".as_slice(),
                    &request,
                    &peer,
                ]
                .concat(),
            )
            .into();
            match self.group_invitation(invitation) {
                Ok(_) => {}
                Err(Error::NotFound) => {
                    self.prepare_group_invitation(
                        group,
                        peer,
                        invitation,
                        Role::Member,
                        expires,
                        now,
                    )?;
                }
                Err(error) => return Err(error),
            }
        }
        Ok(json!({"open":format!("group:{}", transport::hex(&group))}))
    }
    pub(super) fn mobile_groups(&mut self) -> Result<Vec<Value>, Error> {
        let ids = {
            let mut query = self.db.prepare("SELECT id FROM groups ORDER BY id")?;
            let ids = query
                .query_map([], |r| r.get::<_, Vec<u8>>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            ids
        };
        let own = device_fingerprint(&self.own_device_binding()?)?;
        let mut chats = Vec::new();
        for raw in ids {
            let group: Id = raw.try_into().map_err(|_| Error::InvalidStore)?;
            let status = self.group_status(group)?;
            if status.state.is_closed()
                || !status
                    .state
                    .members()
                    .iter()
                    .any(|m| m.device_fingerprints().is_ok_and(|v| v.contains(&own)))
            {
                continue;
            }
            let page = self.recent_conversation_page(group, None, conversations::now())?;
            let message = page.messages.first();
            let peer = format!("group:{}", transport::hex(&group));
            let mut chat = json!({"id":peer,"address":"","name":status.state.profile().map(|p|p.name.as_str()).unwrap_or("Group conversation"),"description":status.state.profile().map(|p|p.description.as_str()).unwrap_or(""),"group":true,"verified":!status.frozen,"devices":[],"timestamp":message.map(|m|m.timestamp).unwrap_or(0),"preview":message.and_then(|m|m.body.as_ref()).map(body_text).transpose()?.unwrap_or_default(),"members":status.state.members().len()});
            self.mobile_summary(&peer, &mut chat)?;
            chats.push(chat);
        }
        Ok(chats)
    }
    pub(super) fn mobile_group_invitations(&mut self) -> Result<Vec<Value>, Error> {
        let ids = {
            let mut query = self
                .db
                .prepare("SELECT id FROM group_invitations ORDER BY id")?;
            let ids = query
                .query_map([], |r| r.get::<_, Vec<u8>>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            ids
        };
        let mut invitations = Vec::new();
        for raw in ids {
            let id = raw.try_into().map_err(|_| Error::InvalidStore)?;
            let notice = self.group_invitation(id)?;
            if !notice.outgoing
                && notice.status == groups::InvitationStatus::Offered
                && notice.expires_at > conversations::now()
            {
                invitations.push(json!({"id":transport::hex(&id),"peer":transport::hex(&notice.peer),"group":transport::hex(&notice.group)}));
            }
        }
        Ok(invitations)
    }
}
