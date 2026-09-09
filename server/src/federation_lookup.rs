use crate::{
    federation_admission, federation_auth as auth, federation_config,
    prekeys::{active, authorize},
    push_config::{sql, unsigned},
    store::{Store, StoreError},
};
use axum::http::HeaderMap;
use rusqlite::{OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};
use sigil_protocol::{
    accounts::valid_credential,
    federation::{
        Lookup, LookupReply, LookupValue, ProxyLookup, ServerLookup, Service, LOOKUP_PATH,
        MAX_LOOKUP_BODY,
    },
    prekeys::ClaimedPrekey,
};
use zeroize::Zeroizing;
pub(crate) const MIGRATION:&str="
ALTER TABLE prekeys ADD COLUMN remote_server TEXT REFERENCES federation_peers(server);
ALTER TABLE prekeys ADD COLUMN remote_account TEXT;
ALTER TABLE prekeys ADD COLUMN remote_device TEXT;
ALTER TABLE prekeys ADD COLUMN remote_request TEXT CHECK((remote_server IS NULL AND remote_account IS NULL AND remote_device IS NULL AND remote_request IS NULL) OR (remote_server IS NOT NULL AND remote_account IS NOT NULL AND remote_device IS NOT NULL AND remote_request IS NOT NULL AND claimant IS NULL AND request_id IS NULL));
CREATE UNIQUE INDEX prekeys_remote_claim ON prekeys(remote_server,remote_device,remote_request) WHERE remote_server IS NOT NULL;
DROP INDEX prekeys_available;
CREATE INDEX prekeys_available ON prekeys(device_id,expires_at) WHERE bundle IS NOT NULL AND claimant IS NULL AND remote_server IS NULL;
";
impl Store {
    pub(crate) fn admit_federation_call(
        &mut self,
        body: &[u8],
        headers: &HeaderMap,
        now: u64,
    ) -> Result<(String, Service), StoreError> {
        if body.len() > MAX_LOOKUP_BODY {
            return Err(StoreError::Invalid("lookup request too large"));
        }
        let request: ServerLookup = serde_json::from_slice(body)
            .map_err(|_| StoreError::Invalid("invalid federation lookup"))?;
        if !request.sender_account.is_empty()
            || !request.sender_device.is_empty()
            || !request.operation.valid()
        {
            return Err(StoreError::Invalid("invalid call lookup"));
        }
        let Lookup::Service { service } = request.operation else {
            return Err(StoreError::Invalid("invalid call lookup"));
        };
        if !matches!(
            service,
            Service::CallConnect { .. } | Service::CallRelay { .. } | Service::CallUpdate { .. }
        ) {
            return Err(StoreError::Invalid("invalid call lookup"));
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let hash = federation_admission::admit(&tx, LOOKUP_PATH, body, headers, now)?;
        tx.commit()?;
        Ok((hash, service))
    }
    pub fn receive_federation_lookup(
        &mut self,
        body: &[u8],
        headers: &HeaderMap,
        now: u64,
    ) -> Result<LookupReply, StoreError> {
        if body.len() > MAX_LOOKUP_BODY {
            return Err(StoreError::Invalid("lookup request too large"));
        }
        let request: ServerLookup = serde_json::from_slice(body)
            .map_err(|_| StoreError::Invalid("invalid federation lookup"))?;
        if (if request.operation.anonymous() {
            !request.sender_account.is_empty() || !request.sender_device.is_empty()
        } else {
            !valid_credential(&request.sender_account) || !valid_credential(&request.sender_device)
        }) || !request.operation.valid()
            || serde_json::to_vec(&request).map_err(|_| StoreError::InvalidData)? != body
        {
            return Err(StoreError::Invalid("invalid federation lookup"));
        }
        let (origin, destination, _) =
            auth::inspect(headers).map_err(|_| StoreError::Unauthorized)?;
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let hash = federation_admission::admit(&tx, LOOKUP_PATH, body, headers, now)?;
        if let Some(target) = request.operation.device() {
            if !active(&tx, target)? {
                return Err(StoreError::NotFound);
            }
            crate::federation_mailbox::grant_for(
                &tx,
                origin,
                &request.sender_account,
                &request.sender_device,
                target,
            )?;
        }
        let value = match &request.operation {
            Lookup::Account { username } => {
                LookupValue::Account(crate::admin::discover(&tx, username, now)?)
            }
            Lookup::Service { service } => LookupValue::Service(match service {
                Service::ContactRequest { request: contact } => {
                    let value = crate::contact_requests::request_in(
                        &tx,
                        origin,
                        &request.sender_account,
                        &request.sender_device,
                        contact,
                        now,
                    )?;
                    serde_json::to_string(&value).map_err(|_| StoreError::InvalidData)?
                }
                Service::ContactStatus { recipient } => {
                    let value = crate::contact_requests::status_in(
                        &tx,
                        origin,
                        &request.sender_account,
                        recipient,
                        now,
                    )?;
                    serde_json::to_string(&value).map_err(|_| StoreError::InvalidData)?
                }
                Service::GroupAuthority => {
                    let stored = crate::group_authority::read(&tx)?;
                    if !stored.enabled {
                        return Err(StoreError::Forbidden);
                    }
                    serde_json::to_string(&auth::hex(
                        &stored.authority.ok_or(StoreError::InvalidData)?.to_bytes(),
                    ))
                    .map_err(|_| StoreError::InvalidData)?
                }
                Service::GroupCredential {
                    authority,
                    day,
                    binding,
                } => {
                    let bytes = crate::group_authority::decode(binding, 512)?;
                    let signed = sigil_protocol::device::SignedBinding::from_bytes(&bytes)
                        .map_err(|_| StoreError::InvalidData)?;
                    if signed.binding.server != origin
                        || auth::hex(&signed.binding.account) != request.sender_account
                        || auth::hex(&signed.binding.device) != request.sender_device
                    {
                        return Err(StoreError::Unauthorized);
                    }
                    let value = crate::group_authority::issue_in(
                        &tx,
                        &bytes,
                        sigil_protocol::groups::CredentialRequest {
                            authority: authority.clone(),
                            day: *day,
                        },
                        now,
                    )?;
                    serde_json::to_string(&value).map_err(|_| StoreError::InvalidData)?
                }
                Service::GroupRequest { request } => {
                    let request = serde_json::from_str(request)
                        .map_err(|_| StoreError::Invalid("invalid group request"))?;
                    let value = crate::group_operations::request_in(&tx, request, now)?;
                    serde_json::to_string(&value).map_err(|_| StoreError::InvalidData)?
                }
                Service::CallConnect { .. }
                | Service::CallRelay { .. }
                | Service::CallUpdate { .. } => {
                    return Err(StoreError::Invalid("calling requires the media runtime"))
                }
                Service::AttachmentChunk {
                    file,
                    index,
                    access,
                } => auth::hex(&crate::attachments::chunk_in(
                    &tx, file, *index, access, now,
                )?),
            }),
            Lookup::Binding { device: target } => {
                let bytes:Vec<u8>=tx.query_row("SELECT CASE WHEN length(statement)<=512 THEN statement END FROM device_bindings WHERE device=?1",[target],|r|r.get(0)).optional()?.ok_or(StoreError::NotFound)?;
                LookupValue::Binding(auth::hex(&bytes))
            }
            Lookup::Claim {
                device: target,
                request_id,
            } => {
                type Assignment = (String, String, String, Option<Vec<u8>>, u64);
                let old:Option<Assignment>=tx.query_row("SELECT id,device_id,remote_account,bundle,expires_at FROM prekeys WHERE remote_server=?1 AND remote_device=?2 AND remote_request=?3",(origin,&request.sender_device,request_id),|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,unsigned(r,4)?))).optional()?;
                let value = if let Some((id, device, account, bytes, expires)) = old {
                    if device != *target || account != request.sender_account {
                        return Err(StoreError::AlreadyExists);
                    }
                    if expires <= now {
                        return Err(StoreError::NotFound);
                    }
                    ClaimedPrekey {
                        device_id: device,
                        prekey_id: id,
                        bundle: auth::hex(&bytes.ok_or(StoreError::NotFound)?),
                        expires_at: expires,
                    }
                } else {
                    let available:Option<(String,Vec<u8>,u64)>=tx.query_row("SELECT id,bundle,expires_at FROM prekeys WHERE device_id=?1 AND claimant IS NULL AND remote_server IS NULL AND bundle IS NOT NULL AND expires_at>?2 ORDER BY expires_at,id LIMIT 1",(target,sql(now)?),|r|Ok((r.get(0)?,r.get(1)?,unsigned(r,2)?))).optional()?;
                    let (id, bytes, expires) = available.ok_or(StoreError::NotFound)?;
                    crate::federation_mailbox::reserve_remote(
                        &tx,
                        origin,
                        target,
                        crate::federation_mailbox::METADATA,
                        0,
                        now,
                    )?;
                    tx.execute("UPDATE prekeys SET remote_server=?2,remote_account=?3,remote_device=?4,remote_request=?5 WHERE id=?1",(&id,origin,&request.sender_account,&request.sender_device,request_id))?;
                    ClaimedPrekey {
                        device_id: target.into(),
                        prekey_id: id,
                        bundle: auth::hex(&bytes),
                        expires_at: expires,
                    }
                };
                LookupValue::Prekey(value)
            }
        };
        if !value.valid_for(&request.operation, destination) {
            return Err(StoreError::InvalidData);
        }
        tx.commit()?;
        Ok(LookupReply {
            request_hash: hash,
            value,
        })
    }
    pub(crate) fn prepare_federation_lookup(
        &mut self,
        credential: &str,
        request: ProxyLookup,
        now: u64,
    ) -> Result<Proxy, StoreError> {
        if !sigil_protocol::valid_server_name(&request.destination) || !request.operation.valid() {
            return Err(StoreError::Invalid("invalid lookup target"));
        }
        let tx = self.0.transaction()?;
        let device = authorize(&tx, credential, now)?;
        let account: String = tx.query_row(
            "SELECT account_id FROM devices WHERE id=?1",
            [&device],
            |r| r.get(0),
        )?;
        let config = federation_config::read(&tx)?;
        let peer =
            federation_config::peer(&tx, &request.destination)?.ok_or(StoreError::Forbidden)?;
        if !config.enabled
            || !peer.allowed
            || peer.pinned.is_none()
            || peer.error.is_some()
            || peer.checked_at == 0
            || peer.checked_at > now
            || now - peer.checked_at > 3600
        {
            return Err(StoreError::Forbidden);
        }
        if let Lookup::Service {
            service: Service::GroupCredential { binding, .. },
        } = &request.operation
        {
            let published: Vec<u8> = tx
                .query_row(
                    "SELECT statement FROM device_bindings WHERE device=?1",
                    [&device],
                    |r| r.get(0),
                )
                .optional()?
                .ok_or(StoreError::NotFound)?;
            if auth::hex(&published) != *binding {
                return Err(StoreError::Forbidden);
            }
        }
        if let Lookup::Service {
            service: Service::ContactRequest { request: contact },
        } = &request.operation
        {
            let published: Vec<u8> = tx
                .query_row(
                    "SELECT statement FROM device_bindings WHERE device=?1",
                    [&device],
                    |r| r.get(0),
                )
                .optional()?
                .ok_or(StoreError::NotFound)?;
            if contact.server != request.destination || auth::hex(&published) != contact.binding {
                return Err(StoreError::Forbidden);
            }
        }
        let wire = ServerLookup {
            sender_account: if request.operation.anonymous() {
                String::new()
            } else {
                account
            },
            sender_device: if request.operation.anonymous() {
                String::new()
            } else {
                device.clone()
            },
            operation: request.operation.clone(),
        };
        let body = Zeroizing::new(serde_json::to_vec(&wire).map_err(|_| StoreError::InvalidData)?);
        let hash = auth::hex(&Sha256::digest(body.as_slice()));
        tx.commit()?;
        Ok(Proxy {
            device,
            operation: request.operation,
            body,
            hash,
            config,
            peer,
            started: now,
        })
    }
    pub(crate) fn finish_federation_lookup(
        &mut self,
        proxy: &Proxy,
        result: LookupReply,
        now: u64,
    ) -> Result<LookupReply, StoreError> {
        let tx = self.0.transaction()?;
        let config = federation_config::read(&tx)?;
        let peer =
            federation_config::peer(&tx, &proxy.peer.server)?.ok_or(StoreError::Forbidden)?;
        if now < proxy.started
            || !active(&tx, &proxy.device)?
            || config.revision != proxy.config.revision
            || !config.enabled
            || peer.revision != proxy.peer.revision
            || !peer.allowed
            || peer.error.is_some()
            || peer.pinned != proxy.peer.pinned
        {
            return Err(StoreError::Conflict);
        }
        if result.request_hash != proxy.hash
            || !result.value.valid_for(&proxy.operation, &peer.server)
        {
            return Err(StoreError::InvalidData);
        }
        tx.commit()?;
        Ok(result)
    }
}
pub(crate) struct Proxy {
    device: String,
    operation: Lookup,
    body: Zeroizing<Vec<u8>>,
    hash: String,
    config: federation_config::Stored,
    peer: federation_config::Peer,
    started: u64,
}
pub(crate) enum Failure {
    Transport,
    Remote { status: u16, retry_at: u64 },
}
impl Proxy {
    pub(crate) fn send(&self) -> Result<LookupReply, Failure> {
        self.send_with(crate::enrollment::now, |request| {
            self.config.policy.federation(request)
        })
    }
    fn send_with(
        &self,
        clock: impl Fn() -> Result<u64, StoreError>,
        mut send: impl FnMut(
            ureq::http::Request<&[u8]>,
        ) -> Result<crate::egress::Response, crate::egress::Error>,
    ) -> Result<LookupReply, Failure> {
        let now = clock().map_err(|_| Failure::Transport)?;
        if now < self.started {
            return Err(Failure::Transport);
        }
        let origin = self.config.server.as_deref().ok_or(Failure::Transport)?;
        let key = self.config.key.as_ref().ok_or(Failure::Transport)?;
        let mut nonce = [0; 32];
        getrandom::fill(&mut nonce).map_err(|_| Failure::Transport)?;
        let headers = auth::sign(
            key,
            &auth::Request {
                origin,
                destination: &self.peer.server,
                path: LOOKUP_PATH,
                body: self.body.as_slice(),
            },
            now,
            nonce,
        )
        .map_err(|_| Failure::Transport)?;
        let url = format!(
            "https://{}:{}{}",
            self.peer.server, self.peer.port, LOOKUP_PATH
        );
        let mut request = ureq::http::Request::post(url)
            .body(self.body.as_slice())
            .map_err(|_| Failure::Transport)?;
        *request.headers_mut() = headers;
        let response = send(request).map_err(|_| Failure::Transport)?;
        if response.status != 200 {
            let completed = clock().map_err(|_| Failure::Transport)?;
            return Err(Failure::Remote {
                status: response.status,
                retry_at: crate::push_provider::retry_after(
                    response.retry_after.as_deref(),
                    completed,
                )
                .unwrap_or(0),
            });
        }
        if response.content_type.as_deref() != Some("application/json") {
            return Err(Failure::Transport);
        }
        let result: LookupReply =
            serde_json::from_slice(&response.body).map_err(|_| Failure::Transport)?;
        if result.request_hash != self.hash
            || !result.value.valid_for(&self.operation, &self.peer.server)
        {
            return Err(Failure::Transport);
        }
        Ok(result)
    }
}

#[cfg(test)]
#[path = "federation_lookup_tests.rs"]
mod tests;
