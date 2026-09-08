//! Strict client-side validation of the home's federation transport API. These
//! receipts express transport acceptance, never end-to-end identity or decryption.
use super::*;
use sigil_protocol::federation::{
    ConfigureSender, LookupReply, LookupValue, Outbound, OutboundState, ProxyLookup, Queue,
    RemoteSender, SenderPermission, ServerLookup, Submit,
};
pub(super) fn sender_valid(sender: &RemoteSender, home: &str) -> bool {
    sigil_protocol::valid_server_name(&sender.server)
        && sender.server != home
        && accounts::valid_credential(&sender.account)
        && accounts::valid_credential(&sender.device)
}
fn own_valid(home: &str, own: &accounts::Session) -> bool {
    accounts::valid_credential(&own.account_id)
        && accounts::valid_credential(&own.device_id)
        && own
            .address
            .strip_prefix('@')
            .and_then(|a| a.split_once(':'))
            .is_some_and(|(user, server)| accounts::valid_username(user) && server == home)
}
fn expected_hash(home: &str, own: &accounts::Session, request: &Queue) -> Result<String, Error> {
    if !own_valid(home, own)
        || !sigil_protocol::valid_server_name(&request.destination)
        || request.destination == home
        || !accounts::valid_credential(&request.recipient_device)
        || !accounts::valid_credential(&request.message_id)
        || !valid_hex(&request.payload, 32, mailbox::MAX_PAYLOAD_HEX)
        || !valid_time(request.expires_at)
    {
        return Err(Error::Configuration);
    }
    let submit = Submit {
        sender_account: own.account_id.clone(),
        sender_device: own.device_id.clone(),
        recipient_device: request.recipient_device.clone(),
        message_id: request.message_id.clone(),
        payload: request.payload.clone(),
        expires_at: request.expires_at,
    };
    let bytes = Zeroizing::new(serde_json::to_vec(&submit).map_err(|_| Error::Configuration)?);
    Ok(crate::transport::hex(&Sha256::digest(&bytes)))
}
fn valid_outbound(value: &Outbound, request: &Queue, hash: &str) -> bool {
    if value.message_id != request.message_id
        || value.destination != request.destination
        || value.request_hash != hash
        || value.expires_at != request.expires_at
        || !valid_time(value.not_before)
        || value.attempts > 31
    {
        return false;
    }
    match value.state {
        OutboundState::Accepted => {
            value.error.is_none()
                && value.receipt.as_ref().is_some_and(|r| {
                    r.request_hash == hash && r.expires_at == request.expires_at && r.sequence > 0
                })
        }
        OutboundState::Pending => {
            value.receipt.is_none()
                && value.error.as_deref().is_none_or(|e| {
                    matches!(
                        e,
                        "clock_unavailable"
                            | "clock_or_expiry_changed"
                            | "key_unavailable"
                            | "random_unavailable"
                            | "signature_unavailable"
                            | "request_unavailable"
                            | "transport_failed"
                            | "invalid_receipt"
                            | "remote_unauthorized"
                            | "remote_unavailable"
                    )
                })
        }
        OutboundState::Rejected => {
            value.receipt.is_none() && value.error.as_deref() == Some("remote_rejected")
        }
        OutboundState::Expired => {
            value.receipt.is_none() && value.error.as_deref() == Some("expired")
        }
        OutboundState::Revoked => {
            value.receipt.is_none()
                && matches!(
                    value.error.as_deref(),
                    Some("sender_revoked" | "peer_retired")
                )
        }
        OutboundState::Restored => {
            value.receipt.is_none() && value.error.as_deref() == Some("restored_no_replay")
        }
    }
}
impl HttpsClient {
    /// Fetch transport data through the authenticated home server. The caller must
    /// still verify the signed binding or prekey against its independently trusted
    /// identity before changing peer trust or committing a handshake. Persist a
    /// claim request ID before sending; reuse it after ambiguous HTTP failures.
    pub fn federated_lookup(
        &self,
        own: &accounts::Session,
        request: &ProxyLookup,
    ) -> Result<LookupValue, Error> {
        if !own_valid(&self.server, own)
            || !sigil_protocol::valid_server_name(&request.destination)
            || request.destination == self.server
            || !request.operation.valid()
        {
            return Err(Error::Configuration);
        }
        let body = ServerLookup {
            sender_account: if request.operation.anonymous() {
                String::new()
            } else {
                own.account_id.clone()
            },
            sender_device: if request.operation.anonymous() {
                String::new()
            } else {
                own.device_id.clone()
            },
            operation: request.operation.clone(),
        };
        self.lookup_reply(request, &body)
    }
    fn lookup_reply(
        &self,
        request: &ProxyLookup,
        body: &ServerLookup,
    ) -> Result<LookupValue, Error> {
        if !sigil_protocol::valid_server_name(&request.destination)
            || request.destination == self.server
            || !request.operation.valid()
        {
            return Err(Error::Configuration);
        }
        let bytes = Zeroizing::new(serde_json::to_vec(body).map_err(|_| Error::Configuration)?);
        let hash = crate::transport::hex(&Sha256::digest(&bytes));
        let reply: LookupReply = self.json(
            self.request(Method::POST, "/client/v0/federation/lookup", Some(request))?,
            200,
            sigil_protocol::federation::MAX_LOOKUP_RESPONSE + 8192,
        )?;
        if reply.request_hash != hash
            || !reply
                .value
                .valid_for(&request.operation, &request.destination)
        {
            return Err(Error::InvalidResponse);
        }
        Ok(reply.value)
    }
    pub(crate) fn federated_service(
        &self,
        destination: &str,
        service: sigil_protocol::federation::Service,
    ) -> Result<String, Error> {
        let mut body = ServerLookup {
            sender_account: String::new(),
            sender_device: String::new(),
            operation: sigil_protocol::federation::Lookup::Service { service },
        };
        if let sigil_protocol::federation::Lookup::Service {
            service: sigil_protocol::federation::Service::GroupCredential { binding, .. },
        } = &body.operation
        {
            let raw = sigil_protocol::device::Statement {
                statement: binding.clone(),
            }
            .bytes()
            .map_err(|_| Error::Configuration)?;
            let binding = sigil_protocol::device::SignedBinding::from_bytes(&raw)
                .map_err(|_| Error::Configuration)?
                .binding;
            if binding.server != self.server {
                return Err(Error::Configuration);
            }
            body.sender_account = crate::transport::hex(&binding.account);
            body.sender_device = crate::transport::hex(&binding.device);
        }
        let request = ProxyLookup {
            destination: destination.into(),
            operation: body.operation.clone(),
        };
        match self.lookup_reply(&request, &body)? {
            LookupValue::Service(value) => Ok(value),
            _ => Err(Error::InvalidResponse),
        }
    }
    pub fn configure_federation_sender(
        &self,
        request: &ConfigureSender,
    ) -> Result<SenderPermission, Error> {
        if !sender_valid(&request.sender, &self.server)
            || request.expected_revision >= i64::MAX as u64
        {
            return Err(Error::Configuration);
        }
        let response: SenderPermission = self.json(
            self.request(Method::PUT, "/client/v0/federation/senders", Some(request))?,
            200,
            SMALL,
        )?;
        if response.sender != request.sender
            || response.allowed != request.allowed
            || response.revision != request.expected_revision + 1
        {
            return Err(Error::InvalidResponse);
        }
        Ok(response)
    }
    pub fn federation_sender_permission(
        &self,
        server: &str,
        device: &str,
    ) -> Result<SenderPermission, Error> {
        if !sigil_protocol::valid_server_name(server)
            || server == self.server
            || !accounts::valid_credential(device)
        {
            return Err(Error::Configuration);
        }
        let path = format!("/client/v0/federation/senders/{server}/{device}");
        let response: SenderPermission =
            self.json(self.request(Method::GET, &path, None::<&()>)?, 200, SMALL)?;
        if response.sender.server != server
            || response.sender.device != device
            || !sender_valid(&response.sender, &self.server)
            || !valid_time(response.revision)
        {
            return Err(Error::InvalidResponse);
        }
        Ok(response)
    }
    pub fn federated_senders(&self) -> Result<Vec<RemoteSender>, Error> {
        let response: Vec<RemoteSender> = self.json(
            self.request(Method::GET, "/client/v0/federation/senders", None::<&()>)?,
            200,
            256 * 512,
        )?;
        if response.len() > 256
            || response.iter().any(|s| !sender_valid(s, &self.server))
            || response
                .windows(2)
                .any(|p| (&p[0].server, &p[0].device) >= (&p[1].server, &p[1].device))
        {
            return Err(Error::InvalidResponse);
        }
        Ok(response)
    }
    pub fn queue_federated_message(
        &self,
        own: &accounts::Session,
        request: &Queue,
    ) -> Result<Outbound, Error> {
        let hash = expected_hash(&self.server, own, request)?;
        let response: Outbound = self.json(
            self.request(
                Method::POST,
                "/client/v0/federation/messages",
                Some(request),
            )?,
            202,
            SMALL,
        )?;
        if !valid_outbound(&response, request, &hash) {
            return Err(Error::InvalidResponse);
        }
        Ok(response)
    }
    /// Bind status to the caller's frozen request and authenticated local account.
    /// A status fetched by message ID alone is insufficient to acknowledge a packet.
    pub fn federated_outbound(
        &self,
        own: &accounts::Session,
        request: &Queue,
    ) -> Result<Outbound, Error> {
        let hash = expected_hash(&self.server, own, request)?;
        let path = format!("/client/v0/federation/messages/{}", request.message_id);
        let response: Outbound =
            self.json(self.request(Method::GET, &path, None::<&()>)?, 200, SMALL)?;
        if !valid_outbound(&response, request, &hash) {
            return Err(Error::InvalidResponse);
        }
        Ok(response)
    }
}
#[cfg(test)]
#[path = "federation_network_tests.rs"]
mod tests;
