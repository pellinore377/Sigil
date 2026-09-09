use super::*;
use rusqlite::OptionalExtension;
use serde::Serialize;
use sigil_protocol::{
    contacts::*,
    federation::{Lookup, LookupValue, ProxyLookup, Service},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Contact {
    server: String,
    username: String,
    account: Id,
    legacy: Option<Id>,
    outgoing: Option<RequestContact>,
    receipt: Option<RequestReceipt>,
    incoming: Option<IncomingRequest>,
    decision: Option<RequestState>,
    work_at: u64,
    blocked: bool,
    block_pending: bool,
}
impl Contact {
    fn id(&self) -> Id {
        event::account_reference(&self.server, &self.account)
    }
    fn display(&self) -> String {
        self.legacy
            .map(|id| transport::hex(&id))
            .unwrap_or_else(|| format!("dm:{}", transport::hex(&self.id())))
    }
}
fn request_id(origin: &str, account: &str, recipient: &str) -> String {
    transport::hex(&Sha256::digest(
        [
            b"Sigil/contact-request-id/v1\0".as_slice(),
            &(origin.len() as u16).to_be_bytes(),
            origin.as_bytes(),
            account.as_bytes(),
            recipient.as_bytes(),
        ]
        .concat(),
    ))
}
fn aad(id: &Id) -> Vec<u8> {
    [b"Sigil/mobile-contact/v1".as_slice(), id].concat()
}
fn waiting() -> u64 {
    i64::MAX as u64
}
impl ClientStore {
    fn contact(&self, id: Id) -> Result<Contact, Error> {
        let (at, bytes): (i64, Vec<u8>) = self
            .db
            .query_row(
                "SELECT work_at,state FROM mobile_contacts WHERE id=?1",
                [id.as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or(Error::NotFound)?;
        if bytes.len() > 16384 {
            return Err(Error::InvalidStore);
        }
        let value: Contact = serde_json::from_slice(&self.key.open(&bytes, &aad(&id))?)
            .map_err(|_| Error::InvalidStore)?;
        if value.id() != id
            || i64::try_from(value.work_at).ok() != Some(at)
            || !sigil_protocol::valid_server_name(&value.server)
            || !sigil_protocol::accounts::valid_username(&value.username)
        {
            return Err(Error::InvalidStore);
        }
        Ok(value)
    }
    fn save_contact(&mut self, value: &Contact) -> Result<(), Error> {
        let id = value.id();
        let bytes = Zeroizing::new(serde_json::to_vec(value).map_err(|_| Error::InvalidStore)?);
        if bytes.len() > 16000 || value.work_at > waiting() {
            return Err(Error::Limit);
        }
        let sealed = self.key.seal(&bytes, &aad(&id))?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.query_row("SELECT count(*)>=4096 AND NOT EXISTS(SELECT 1 FROM mobile_contacts WHERE id=?1) FROM mobile_contacts", [id.as_slice()], |r| r.get::<_,bool>(0))? { return Err(Error::Limit); }
        tx.execute("INSERT INTO mobile_contacts VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET work_at=excluded.work_at,state=excluded.state", (id.as_slice(), value.work_at as i64, sealed))?;
        tx.commit()?;
        Ok(())
    }
    fn contact_legacy(&self, server: &str, account: Id) -> Result<Option<Id>, Error> {
        Ok(self
            .mobile_peers()?
            .into_iter()
            .find(|p| p.binding.server == server && p.binding.account == account)
            .map(|p| p.id))
    }
    fn contact_for(&self, peer: &str) -> Result<Contact, Error> {
        if let Some(raw) = peer.strip_prefix("dm:") {
            return self.contact(id(raw)?);
        }
        let binding = self.peer(id(peer)?)?.binding;
        match self.contact(event::account_reference(&binding.server, &binding.account)) {
            Ok(value) => Ok(value),
            Err(Error::NotFound) => Ok(Contact {
                legacy: self.contact_legacy(&binding.server, binding.account)?,
                server: binding.server,
                username: binding.username,
                account: binding.account,
                outgoing: None,
                receipt: None,
                incoming: None,
                decision: None,
                work_at: waiting(),
                blocked: false,
                block_pending: false,
            }),
            Err(error) => Err(error),
        }
    }
    pub(super) fn mobile_contact_blocked(&self, peer: &Peer) -> Result<bool, Error> {
        match self.contact(event::account_reference(
            &peer.binding.server,
            &peer.binding.account,
        )) {
            Ok(contact) => Ok(contact.blocked),
            Err(Error::NotFound) => Ok(false),
            Err(error) => Err(error),
        }
    }
    pub(super) fn mobile_peer(&self, peer: &str) -> Result<Id, Error> {
        if !peer.starts_with("dm:") {
            return id(peer);
        }
        let contact = self.contact_for(peer)?;
        self.mobile_peers()?
            .into_iter()
            .find(|p| p.binding.server == contact.server && p.binding.account == contact.account)
            .map(|p| p.id)
            .ok_or(Error::Unprepared)
    }
    pub(super) fn mobile_peer_display(&self, peer: &Peer) -> Result<String, Error> {
        match self.contact(event::account_reference(
            &peer.binding.server,
            &peer.binding.account,
        )) {
            Ok(contact) => Ok(contact.display()),
            Err(Error::NotFound) => Ok(transport::hex(&peer.id)),
            Err(error) => Err(error),
        }
    }
    pub(super) fn mobile_contact_conversation(&mut self, peer: &str) -> Result<Id, Error> {
        let contact = self.contact_for(peer)?;
        Ok(event::direct_reference(
            self.account_reference()?,
            contact.id(),
        ))
    }
    fn contact_peers(&mut self, contact: &Contact) -> Result<(), Error> {
        let found =
            self.discover_account_online(&format!("@{}:{}", contact.username, contact.server))?;
        if found.account != transport::hex(&contact.account) {
            return Err(Error::Conflict);
        }
        let own = self.connection_session()?.ok_or(Error::Unprepared)?;
        for device in &found.devices {
            let result =
                if own.address.split_once(':').ok_or(Error::InvalidStore)?.1 == contact.server {
                    self.fetch_peer_online(id(device)?)
                } else {
                    self.fetch_remote_peer_online(&contact.server, id(device)?)
                };
            let peer = match result {
                Ok(peer) => peer,
                Err(Error::Network(network::Error::Status { code: 403, .. })) => continue,
                Err(error) => return Err(error),
            };
            if peer.binding.username != contact.username
                || peer.binding.server != contact.server
                || peer.binding.account != contact.account
            {
                return Err(Error::Conflict);
            }
        }
        Ok(())
    }
    pub(super) fn mobile_find(&mut self, address: &str) -> Result<Value, Error> {
        let found = self.discover_account_online(address)?;
        let (username, server) = address
            .strip_prefix('@')
            .and_then(|v| v.split_once(':'))
            .ok_or(Error::InvalidEvent)?;
        let account = id(&found.account)?;
        let own = self.connection_session()?.ok_or(Error::Unprepared)?;
        if address == own.address {
            return Ok(json!({"open":"self"}));
        }
        let key = event::account_reference(server, &account);
        let contact = match self.contact(key) {
            Ok(value) if value.username == username => value,
            Ok(_) => return Err(Error::Conflict),
            Err(Error::NotFound) => Contact {
                legacy: self.contact_legacy(server, account)?,
                server: server.into(),
                username: username.into(),
                account,
                outgoing: None,
                receipt: None,
                incoming: None,
                decision: None,
                work_at: waiting(),
                blocked: false,
                block_pending: false,
            },
            Err(error) => return Err(error),
        };
        self.save_contact(&contact)?;
        self.contact_peers(&contact)?;
        self.mobile_state()
    }
    pub(super) fn mobile_contact_chats(&mut self, chats: &mut Vec<Value>) -> Result<(), Error> {
        let ids = self
            .db
            .prepare("SELECT id FROM mobile_contacts ORDER BY id")?
            .query_map([], |r| r.get::<_, Vec<u8>>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        for raw in ids {
            let contact = self.contact(raw.try_into().map_err(|_| Error::InvalidStore)?)?;
            let display = contact.display();
            let index = match chats.iter().position(|c| c["id"] == display) {
                Some(index) => index,
                None => {
                    let mut chat = json!({"id":display,"address":format!("@{}:{}",contact.username,contact.server),"verified":false,"devices":[],"timestamp":0,"preview":""});
                    self.mobile_summary(&display, &mut chat)?;
                    chat["contact_only"] = json!(
                        chat["latest_message"].is_null()
                            && chat["ui"]["opened"] != "true"
                            && contact.incoming.is_none()
                    );
                    chats.push(chat);
                    chats.len() - 1
                }
            };
            if contact.blocked {
                chats[index]["verified"] = json!(false);
            }
            if contact.incoming.is_some() && !contact.blocked {
                chats[index]["contact_only"] = json!(false);
            }
            chats[index]["request"] = json!(if contact.blocked {
                "blocked"
            } else if contact.decision.is_some() {
                "resolving"
            } else if let Some(incoming) = &contact.incoming {
                match incoming.receipt.state {
                    RequestState::Pending
                        if incoming.receipt.expires_at <= conversations::now() =>
                    {
                        "expired"
                    }
                    RequestState::Pending => "incoming",
                    RequestState::Accepted => "accepted",
                    RequestState::Declined | RequestState::Blocked => "declined",
                }
            } else if let Some(receipt) = &contact.receipt {
                match receipt.state {
                    RequestState::Accepted => "accepted",
                    RequestState::Declined | RequestState::Blocked => "declined",
                    RequestState::Pending if receipt.expires_at <= conversations::now() => {
                        "expired"
                    }
                    _ => "pending",
                }
            } else if contact.outgoing.is_some() {
                "sending"
            } else {
                "none"
            });
        }
        Ok(())
    }
    pub(super) fn mobile_request(&mut self, peer: &str, action: &str) -> Result<Value, Error> {
        if action == "block" {
            return self.mobile_block(peer, true);
        }
        let now = conversations::now();
        let mut contact = self.contact_for(peer)?;
        if contact.blocked {
            return Err(Error::Unprepared);
        }
        match action {
            "send" => {
                if contact
                    .outgoing
                    .as_ref()
                    .is_none_or(|r| r.expires_at <= now)
                {
                    let mut request = RequestContact {
                        server: contact.server.clone(),
                        recipient: transport::hex(&contact.account),
                        expires_at: now.checked_add(604800).ok_or(Error::Limit)?,
                        binding: transport::hex(&self.own_device_binding()?),
                        signature: String::new(),
                    };
                    let tx = self
                        .db
                        .transaction_with_behavior(TransactionBehavior::Immediate)?;
                    request.signature = transport::hex(
                        &handshake::identity(&tx, &self.key)?
                            .sign(&request.signing_bytes().map_err(|_| Error::InvalidEvent)?)?,
                    );
                    tx.commit()?;
                    contact.outgoing = Some(request);
                    contact.receipt = None;
                }
            }
            "accept" | "decline" | "block" => {
                let incoming = contact.incoming.as_ref().ok_or(Error::NotFound)?;
                if incoming.receipt.state != RequestState::Pending
                    || incoming.receipt.expires_at <= now
                {
                    return Err(Error::Expired);
                }
                contact.decision = Some(match action {
                    "accept" => RequestState::Accepted,
                    "block" => RequestState::Blocked,
                    _ => RequestState::Declined,
                });
            }
            "refresh" => (),
            _ => return Err(Error::InvalidEvent),
        }
        contact.work_at = now;
        self.save_contact(&contact)?;
        self.contact_work(contact, now)?;
        self.mobile_state()
    }
    pub(super) fn mobile_block(&mut self, peer: &str, active: bool) -> Result<Value, Error> {
        let mut contact = self.contact_for(peer)?;
        if contact.blocked && !active {
            contact.incoming = None;
            contact.decision = None;
        }
        contact.blocked = active;
        contact.block_pending = true;
        contact.work_at = conversations::now();
        self.save_contact(&contact)?;
        for device in self.mobile_peers()? {
            if device.binding.server == contact.server && device.binding.account == contact.account
            {
                self.block_peer(device.id, active)?;
            }
        }
        self.contact_work(contact, conversations::now())?;
        self.mobile_state()
    }
    fn contact_work(&mut self, contact: Contact, now: u64) -> Result<(), Error> {
        let id = contact.id();
        let result = self.contact_work_inner(contact, now);
        if let Err(Error::Network(network::Error::Status {
            retry_after_seconds: Some(delay),
            ..
        })) = &result
        {
            let mut contact = self.contact(id)?;
            contact.work_at = conversations::now()
                .max(now)
                .saturating_add((*delay).clamp(60, 86400))
                .min(waiting());
            self.save_contact(&contact)?;
        }
        result
    }
    fn contact_work_inner(&mut self, mut contact: Contact, now: u64) -> Result<(), Error> {
        // Persist the retry delay before any network side effect.
        contact.work_at = now.saturating_add(60).min(waiting());
        self.save_contact(&contact)?;
        let network = self.connected_client()?;
        let own = self.connection_session()?.ok_or(Error::Unprepared)?;
        let home = own.address.split_once(':').ok_or(Error::InvalidStore)?.1;
        if contact.block_pending {
            network.block_contact_requests(&BlockContact {
                server: contact.server.clone(),
                account: transport::hex(&contact.account),
                blocked: contact.blocked,
            })?;
            for device in self.mobile_peers()? {
                if device.binding.server == contact.server
                    && device.binding.account == contact.account
                {
                    self.block_peer(device.id, contact.blocked)?;
                }
            }
            contact.block_pending = false;
            self.save_contact(&contact)?;
        }
        if contact.blocked {
            contact.work_at = waiting();
            return self.save_contact(&contact);
        }
        if contact.decision.is_none() {
            if let Some(incoming) = contact
                .incoming
                .as_mut()
                .filter(|r| r.receipt.state == RequestState::Pending && r.receipt.expires_at > now)
            {
                match network.incoming_contact_status(&incoming.receipt.id, &incoming.signature) {
                    Ok(receipt) => {
                        if receipt.id != incoming.receipt.id
                            || receipt.expires_at != incoming.receipt.expires_at
                            || receipt.state == RequestState::Blocked
                        {
                            return Err(Error::Network(network::Error::InvalidResponse));
                        }
                        incoming.receipt = receipt;
                    }
                    Err(network::Error::Status { code: 404, .. }) => contact.incoming = None,
                    Err(error) => return Err(error.into()),
                }
                self.save_contact(&contact)?;
            }
        }
        if contact
            .incoming
            .as_ref()
            .is_some_and(|r| r.receipt.expires_at <= now)
        {
            contact.decision = None;
        }
        if let Some(decision) = contact.decision {
            let incoming = contact.incoming.as_mut().ok_or(Error::InvalidStore)?;
            let receipt = network.resolve_contact_request(
                &incoming.receipt.id,
                decision,
                &incoming.signature,
            )?;
            if receipt.id != incoming.receipt.id
                || receipt.expires_at != incoming.receipt.expires_at
                || receipt.state
                    != if decision == RequestState::Blocked {
                        RequestState::Declined
                    } else {
                        decision
                    }
            {
                return Err(Error::Network(network::Error::InvalidResponse));
            }
            incoming.receipt = receipt;
            if decision == RequestState::Blocked {
                contact.blocked = true;
                contact.work_at = waiting();
            }
            contact.decision = None;
            self.save_contact(&contact)?;
        }
        if contact.blocked {
            return self.save_contact(&contact);
        }
        if let Some(request) = &contact.outgoing {
            if request.expires_at > now {
                let result = if contact.server == home {
                    if contact.receipt.is_none() {
                        network.request_contact(request)?
                    } else {
                        network.contact_request_status(&request.recipient)?
                    }
                } else {
                    let service = if contact.receipt.is_none() {
                        Service::ContactRequest {
                            request: request.clone(),
                        }
                    } else {
                        Service::ContactStatus {
                            recipient: request.recipient.clone(),
                        }
                    };
                    let LookupValue::Service(raw) = network.federated_lookup(
                        &own,
                        &ProxyLookup {
                            destination: contact.server.clone(),
                            operation: Lookup::Service { service },
                        },
                    )?
                    else {
                        return Err(Error::Network(network::Error::InvalidResponse));
                    };
                    serde_json::from_str(&raw)
                        .map_err(|_| Error::Network(network::Error::InvalidResponse))?
                };
                if result.id != request_id(home, &own.account_id, &request.recipient)
                    || result.expires_at > request.expires_at
                    || result.state == RequestState::Blocked
                {
                    return Err(Error::Network(network::Error::InvalidResponse));
                }
                contact.receipt = Some(result);
                self.save_contact(&contact)?;
            }
        }
        let accepted = contact
            .receipt
            .as_ref()
            .is_some_and(|r| r.state == RequestState::Accepted)
            || contact
                .incoming
                .as_ref()
                .is_some_and(|r| r.receipt.state == RequestState::Accepted);
        if accepted {
            self.contact_peers(&contact)?;
        }
        let pending = contact.incoming.as_ref().is_some_and(|r| {
            r.receipt.state == RequestState::Pending && r.receipt.expires_at > now
        }) || contact
            .receipt
            .as_ref()
            .is_some_and(|r| r.state == RequestState::Pending && r.expires_at > now)
            || contact
                .outgoing
                .as_ref()
                .is_some_and(|r| contact.receipt.is_none() && r.expires_at > now);
        let need_keys = accepted
            && contact
                .receipt
                .as_ref()
                .map(|r| r.expires_at)
                .into_iter()
                .chain(contact.incoming.as_ref().map(|r| r.receipt.expires_at))
                .any(|at| at > now)
            && self
                .mobile_peer(&contact.display())
                .and_then(|p| self.mobile_recipients(p))
                .is_err();
        if !pending && !need_keys {
            contact.work_at = waiting();
        }
        self.save_contact(&contact)
    }
    fn receive_contact(&mut self, incoming: IncomingRequest, now: u64) -> Result<(), Error> {
        let own = self.connection_session()?.ok_or(Error::Unprepared)?;
        let home = own.address.split_once(':').ok_or(Error::InvalidStore)?.1;
        let request = RequestContact {
            server: home.into(),
            recipient: own.account_id.clone(),
            expires_at: incoming.receipt.expires_at,
            binding: incoming.binding.clone(),
            signature: incoming.signature.clone(),
        };
        if incoming.receipt.state != RequestState::Pending
            || incoming.created_at > now.saturating_add(300)
            || incoming.created_at >= request.expires_at
            || request.expires_at <= now
            || request.expires_at - incoming.created_at > 604800
            || incoming.receipt.id
                != request_id(&incoming.origin, &incoming.account, &own.account_id)
        {
            return Err(Error::InvalidEvent);
        }
        let bytes = sigil_protocol::device::Statement {
            statement: request.binding.clone(),
        }
        .bytes()
        .map_err(|_| Error::InvalidEvent)?;
        let signed = peers::parse(&bytes)?;
        if signed.binding.server != incoming.origin
            || transport::hex(&signed.binding.account) != incoming.account
            || transport::hex(&signed.binding.device) != incoming.device
        {
            return Err(Error::InvalidEvent);
        }
        sigil_crypto::verify_signature(
            &signed.binding.identity,
            &request.signing_bytes().map_err(|_| Error::InvalidEvent)?,
            &request.signature_bytes().map_err(|_| Error::InvalidEvent)?,
        )?;
        let binding = signed.binding;
        let key = event::account_reference(&binding.server, &binding.account);
        let mut contact = match self.contact(key) {
            Ok(value) => value,
            Err(Error::NotFound) => Contact {
                legacy: self.contact_legacy(&binding.server, binding.account)?,
                server: binding.server,
                username: binding.username,
                account: binding.account,
                outgoing: None,
                receipt: None,
                incoming: None,
                decision: None,
                work_at: waiting(),
                blocked: false,
                block_pending: false,
            },
            Err(error) => return Err(error),
        };
        if contact.incoming.as_ref().is_some_and(|r| {
            r.receipt.id == incoming.receipt.id
                && r.receipt.expires_at == incoming.receipt.expires_at
                && r.receipt.state != RequestState::Pending
        }) {
            return Ok(());
        }
        if contact.blocked {
            return Ok(());
        }
        self.observe_peer_binding(&bytes)?;
        if contact
            .incoming
            .as_ref()
            .is_some_and(|old| old.signature != incoming.signature)
        {
            contact.decision = None;
        }
        if incoming.receipt.state == RequestState::Pending {
            contact.work_at = contact.work_at.min(now.saturating_add(60));
        }
        contact.incoming = Some(incoming);
        self.save_contact(&contact)
    }
    pub(super) fn mobile_contact_sync(&mut self, force: bool) -> Result<(), Error> {
        let now = conversations::now();
        let next: i64 = self.db.query_row(
            "SELECT next_at FROM mobile_contact_poll WHERE id=1",
            [],
            |r| r.get(0),
        )?;
        if !force && i64::try_from(now).map_err(|_| Error::Limit)? < next {
            return Ok(());
        }
        self.db.execute(
            "UPDATE mobile_contact_poll SET next_at=?1 WHERE id=1",
            [now.saturating_add(60).min(waiting()) as i64],
        )?;
        let network = self.connected_client()?;
        let mut after = None;
        for _ in 0..2 {
            let page = match network.contact_requests(after.as_deref()) {
                Ok(page) => page,
                Err(network::Error::Status { code: 404, .. }) if !force => {
                    self.db.execute(
                        "UPDATE mobile_contact_poll SET next_at=?1 WHERE id=1",
                        [now.saturating_add(3600).min(waiting()) as i64],
                    )?;
                    return Ok(());
                }
                Err(error) => return Err(error.into()),
            };
            for incoming in page.requests {
                self.receive_contact(incoming, now)?;
            }
            after = page.next;
            if after.is_none() {
                break;
            }
        }
        if after.is_some() {
            return Err(Error::Limit);
        }
        let ids = self
            .db
            .prepare(
                "SELECT id FROM mobile_contacts WHERE work_at<=?1 ORDER BY work_at,id LIMIT 4",
            )?
            .query_map([now as i64], |r| r.get::<_, Vec<u8>>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut error = None;
        for raw in ids {
            let contact = self.contact(raw.try_into().map_err(|_| Error::InvalidStore)?)?;
            if let Err(failure) = self.contact_work(contact, now) {
                error = Some(failure);
            }
        }
        error.map_or(Ok(()), Err)
    }
}

#[cfg(test)]
#[path = "mobile_contact_tests.rs"]
mod tests;
