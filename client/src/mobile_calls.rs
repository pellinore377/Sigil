use super::*;
impl ClientStore {
    pub(super) fn mobile_call_redial(
        &mut self,
        call: Id,
        request: Id,
        timestamp: u64,
    ) -> Result<Value, Error> {
        let now = conversations::now();
        if timestamp > now || now - timestamp > 60 {
            return Err(Error::InvalidEvent);
        }
        let history = self
            .call_history()?
            .into_iter()
            .find(|entry| entry.id == call)
            .ok_or(Error::NotFound)?;
        let recipients = history
            .people
            .iter()
            .filter(|person| !person.own)
            .map(|person| person.peer)
            .collect::<Vec<_>>();
        if recipients.is_empty() || recipients.len() > 7 || history.direct && recipients.len() != 1
        {
            return Err(Error::Limit);
        }
        for peer in &recipients {
            peers::trusted(&self.db, &self.key, peer)?;
        }
        self.start_call(request, now, history.direct, &recipients)?;
        Ok(json!({"call":transport::hex(&request)}))
    }
    pub(super) fn mobile_call_start(
        &mut self,
        peer: &str,
        request: Id,
        timestamp: u64,
    ) -> Result<Value, Error> {
        let now = conversations::now();
        if timestamp > now || now - timestamp > 60 || peer == "self" {
            return Err(Error::InvalidEvent);
        }
        let recipients = if let Some(group) = peer.strip_prefix("group:") {
            let state = self.group_status(id(group)?)?;
            if state.frozen || state.state.is_closed() {
                return Err(Error::Unprepared);
            }
            let members: std::collections::BTreeSet<_> = state
                .state
                .members()
                .iter()
                .map(|m| m.device_fingerprints())
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .flatten()
                .collect();
            let own = device_fingerprint(&self.own_device_binding()?)?;
            if !members.contains(&own) {
                return Err(Error::Unprepared);
            }
            let peers: Vec<_> = self
                .mobile_peers()?
                .into_iter()
                .filter(|p| members.contains(&p.fingerprint) && p.fingerprint != own)
                .collect();
            if peers
                .iter()
                .any(|p| !p.trusted || p.blocked || p.changed_fingerprint.is_some())
                || peers.len() + 1 != members.len()
            {
                return Err(Error::Unprepared);
            }
            peers.into_iter().map(|p| p.id).collect::<Vec<_>>()
        } else {
            // A direct call targets the selected device; accepting more than one would be a group call.
            let peer = self.mobile_peer(peer)?;
            self.mobile_recipients(peer)?;
            peers::trusted(&self.db, &self.key, &peer)?;
            vec![peer]
        };
        if recipients.is_empty() || recipients.len() > 7 {
            return Err(Error::Limit);
        }
        match self.call(request, now) {
            Ok(call) if call.direct != !peer.starts_with("group:") => return Err(Error::Conflict),
            Ok(_) => {
                for recipient in recipients {
                    self.invite_to_call(request, recipient, now)?;
                }
            }
            Err(Error::NotFound) => {
                self.start_call(request, now, !peer.starts_with("group:"), &recipients)?;
            }
            Err(error) => return Err(error),
        }
        Ok(json!({"call":transport::hex(&request)}))
    }
    pub(super) fn mobile_calls(&mut self) -> Result<Value, Error> {
        let now = conversations::now();
        let mut values = Vec::new();
        for call in self.calls(now)? {
            let mut participants = Vec::new();
            for member in &call.participants {
                let device = peers::parse(&member.device)?;
                let peer = peers::reference(&device.binding.server, &device.binding.device);
                let display = match self.peer(peer) {
                    Ok(peer) => self.mobile_peer_display(&peer)?,
                    Err(Error::NotFound) => transport::hex(&peer),
                    Err(error) => return Err(error),
                };
                participants.push(json!({"member":transport::hex(&member.member),"peer":display,"name":device.binding.username,
                    "address":format!("@{}:{}", device.binding.username, device.binding.server),"own":call.own == Some(member.member),"verified":member.verified,"fingerprint":transport::hex(&device_fingerprint(&member.device)?),
                    "audio":member.tracks.is_some_and(|t|t.audio),"camera":member.tracks.is_some_and(|t|t.camera),"screen":member.tracks.is_some_and(|t|t.screen)}));
            }
            values.push(json!({"id":transport::hex(&call.id),"phase":call.phase,"direct":call.direct,"created":call.created,"participants":participants,"can_invite":!call.direct && call.own == Some(sigil_calls::Member::new(call.owner).id)}));
        }
        for history in self.call_history()? {
            let id = transport::hex(&history.id);
            let name = history
                .people
                .iter()
                .filter(|person| !person.own)
                .map(|person| person.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            if let Some(value) = values.iter_mut().find(|v| v["id"] == id) {
                value["outgoing"] = json!(history.outgoing);
                value["name"] = json!(name);
                continue;
            }
            let participants = history.people.iter().map(|person| json!({"member":transport::hex(&person.peer),"peer":transport::hex(&person.peer),"name":person.name,"address":person.address,"own":person.own,"verified":false,"fingerprint":"","audio":false,"camera":false,"screen":false})).collect::<Vec<_>>();
            values.push(json!({"id":id,"phase":match history.phase { calls::Phase::Active | calls::Phase::Joining | calls::Phase::Ringing => calls::Phase::Ended, phase => phase },"direct":history.direct,"created":history.created,"participants":participants,"can_invite":false,"outgoing":history.outgoing,"name":name}));
        }
        values.sort_by_key(|v| std::cmp::Reverse(v["created"].as_u64().unwrap_or(0)));
        Ok(json!({"calls":values}))
    }
}
