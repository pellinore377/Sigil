use super::*;
use sigil_protocol::{federation as wire, mailbox};
pub(crate) fn delivery_peer(
    db: &Connection,
    key: &StorageKey,
    home: &str,
    delivery: &mailbox::Delivery,
) -> Result<Id, Error> {
    let device = connection::decode_id(&delivery.sender_device)?;
    let server = delivery.origin.as_ref().map_or(home, |v| v.server.as_str());
    if let Some(origin) = &delivery.origin {
        if !sigil_protocol::valid_server_name(server)
            || server == home
            || origin.device != delivery.sender_device
            || !sigil_protocol::accounts::valid_credential(&origin.account)
        {
            return Err(Error::Conflict);
        }
    }
    let peer = peers::reference(server, &device);
    if let Some(origin) = &delivery.origin {
        let known = peers::known(db, key, &peer)?;
        if known.binding.account != connection::decode_id(&origin.account)? {
            return Err(Error::Conflict);
        }
    }
    Ok(peer)
}
pub(crate) fn submit(
    network: &network::HttpsClient,
    own: &sigil_protocol::accounts::Session,
    server: &str,
    request: &mailbox::Submit,
    authorize: impl FnOnce() -> Result<(), Error>,
) -> Result<Option<mailbox::Receipt>, Error> {
    let home = own.address.split_once(':').ok_or(Error::InvalidStore)?.1;
    if server == home {
        authorize()?;
        return Ok(Some(network.submit(request)?));
    }
    let queue = wire::Queue {
        destination: server.into(),
        recipient_device: request.recipient_device.clone(),
        message_id: request.message_id.clone(),
        payload: request.payload.clone(),
        expires_at: request.expires_at,
    };
    let reply = match network.federated_outbound(own, &queue) {
        Ok(value) => value,
        Err(network::Error::Status { code: 404, .. }) => {
            authorize()?;
            network.queue_federated_message(own, &queue)?
        }
        Err(error) => return Err(error.into()),
    };
    match reply.state {
        wire::OutboundState::Accepted => {
            let receipt = reply.receipt.ok_or(Error::InvalidStore)?;
            Ok(Some(mailbox::Receipt {
                sequence: receipt.sequence,
                expires_at: receipt.expires_at,
            }))
        }
        wire::OutboundState::Pending => Ok(None),
        wire::OutboundState::Expired => Err(Error::Expired),
        wire::OutboundState::Rejected
        | wire::OutboundState::Revoked
        | wire::OutboundState::Restored => Err(Error::Cancelled),
    }
}
impl ClientStore {
    pub fn discover_account_online(
        &self,
        address: &str,
    ) -> Result<sigil_protocol::admin::FoundAccount, Error> {
        let (username, server) = address
            .strip_prefix('@')
            .and_then(|v| v.split_once(':'))
            .ok_or(Error::InvalidEvent)?;
        if !sigil_protocol::accounts::valid_username(username)
            || !sigil_protocol::valid_server_name(server)
        {
            return Err(Error::InvalidEvent);
        }
        let own = self.connection_session()?.ok_or(Error::Unprepared)?;
        let network = self.connected_client()?;
        if own.address.rsplit_once(':').ok_or(Error::InvalidStore)?.1 == server {
            return Ok(network.discover_account(username)?);
        }
        match network.federated_lookup(
            &own,
            &wire::ProxyLookup {
                destination: server.into(),
                operation: wire::Lookup::Account {
                    username: username.into(),
                },
            },
        )? {
            wire::LookupValue::Account(value) => Ok(value),
            _ => Err(Error::InvalidStore),
        }
    }
    pub fn fetch_remote_peer_online(&mut self, server: &str, device: Id) -> Result<Peer, Error> {
        let own = self.connection_session()?.ok_or(Error::Unprepared)?;
        let value = self.connected_client()?.federated_lookup(
            &own,
            &wire::ProxyLookup {
                destination: server.into(),
                operation: wire::Lookup::Binding {
                    device: transport::hex(&device),
                },
            },
        )?;
        let wire::LookupValue::Binding(statement) = value else {
            return Err(Error::InvalidStore);
        };
        self.observe_peer_binding(
            &sigil_protocol::device::Statement { statement }
                .bytes()
                .map_err(|_| Error::InvalidEvent)?,
        )
    }
    pub fn allow_peer_sender_online(&self, peer: Id) -> Result<(), Error> {
        self.allow_known_sender_online(&peers::trusted(&self.db, &self.key, &peer)?)
    }
    pub(crate) fn allow_known_sender_online(&self, peer: &Peer) -> Result<(), Error> {
        let network = self.connected_client()?;
        let own = self.connection_session()?.ok_or(Error::Unprepared)?;
        let home = own.address.split_once(':').ok_or(Error::InvalidStore)?.1;
        if peer.binding.server == home {
            return Ok(network.allow_mailbox_sender(&transport::hex(&peer.binding.device))?);
        }
        let sender = wire::RemoteSender {
            server: peer.binding.server.clone(),
            account: transport::hex(&peer.binding.account),
            device: transport::hex(&peer.binding.device),
        };
        let revision = match network.federation_sender_permission(&sender.server, &sender.device) {
            Ok(prior) => {
                if prior.sender != sender {
                    return Err(Error::Conflict);
                }
                if prior.allowed {
                    return Ok(());
                }
                prior.revision
            }
            Err(network::Error::Status { code: 404, .. }) => 0,
            Err(e) => return Err(e.into()),
        };
        network.configure_federation_sender(&wire::ConfigureSender {
            expected_revision: revision,
            sender,
            allowed: true,
        })?;
        Ok(())
    }
    pub(crate) fn delivery_destination(
        &mut self,
        session: Id,
        id: Id,
        recipient: &str,
    ) -> Result<String, Error> {
        let own = self.own_device_binding()?;
        let tx = self.db.transaction()?;
        let peer = if let Some(peer) = session_peer(&tx, &session)? {
            Some(peers::trusted(&tx, &self.key, &peer)?)
        } else {
            match groups::delivery_peer(&tx, &self.key, &session, &id)? {
                Some(peer) => Some(peer),
                None => calls::delivery_peer(&tx, &self.key, &session, &id)?,
            }
        };
        match peer {
            Some(peer) if transport::hex(&peer.binding.device) == recipient => {
                Ok(peer.binding.server)
            }
            Some(_) => Err(Error::Conflict),
            None => Ok(peers::parse(&own)?.binding.server),
        }
    }
}

#[cfg(test)]
#[path = "federation_tests.rs"]
mod tests;
