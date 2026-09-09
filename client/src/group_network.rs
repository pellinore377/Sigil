//! Anonymous authority traffic never carries the native device credential.
//! Membership signatures and local rollback checks remain mandatory at commit.
use super::*;
use sigil_crypto::{
    group_receipt::Receipt,
    private_credentials::{Credential, GroupKey},
    private_group::{Authority, Operation as Kind, Request as Context},
};
use sigil_protocol::federation::Service;
use sigil_protocol::groups::{self, CredentialRequest, CredentialResponse, Operation, Reply};

fn decode(value: &str, limit: usize) -> Result<Vec<u8>, Error> {
    if !valid_hex(value, 0, limit * 2) {
        return Err(Error::InvalidResponse);
    }
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| {
            u8::from_str_radix(
                std::str::from_utf8(p).map_err(|_| Error::InvalidResponse)?,
                16,
            )
            .map_err(|_| Error::InvalidResponse)
        })
        .collect()
}
fn id(value: &str) -> Result<[u8; 32], Error> {
    decode(value, 32)?
        .try_into()
        .map_err(|_| Error::InvalidResponse)
}
fn hex(bytes: &[u8]) -> String {
    crate::transport::hex(bytes)
}

impl HttpsClient {
    fn anonymous_group(
        &self,
        path: &str,
        body: Option<&impl Serialize>,
    ) -> Result<Response<Body>, Error> {
        let bytes = Zeroizing::new(match body {
            Some(value) => serde_json::to_vec(value).map_err(|_| Error::Configuration)?,
            None => Vec::new(),
        });
        if bytes.len() > groups::MAX_BODY {
            return Err(Error::Limit);
        }
        let mut request = Request::builder()
            .method(if body.is_some() {
                Method::POST
            } else {
                Method::GET
            })
            .uri(format!("{}{path}", self.api_origin()?))
            .header(header::ACCEPT, "application/json")
            .header(header::ACCEPT_ENCODING, "identity");
        if body.is_some() {
            request = request.header(header::CONTENT_TYPE, "application/json");
        }
        self.send(
            request
                .body(bytes.as_slice())
                .map_err(|_| Error::Configuration)?,
        )
    }
    /// Bootstrap a new group's authority from this account's authenticated server.
    pub fn bootstrap_group_authority(&self) -> Result<Authority, Error> {
        let encoded: String = self.json(
            self.anonymous_group("/groups/v0/authority", None::<&()>)?,
            200,
            SMALL,
        )?;
        let bytes = decode(&encoded, 431)?;
        let end = bytes.len().checked_sub(96).ok_or(Error::InvalidResponse)?;
        let signing = bytes
            .get(end..end + 32)
            .ok_or(Error::InvalidResponse)?
            .try_into()
            .map_err(|_| Error::InvalidResponse)?;
        Authority::from_bytes(
            &bytes,
            &self.server,
            sigil_crypto::private_group::authority_fingerprint(signing),
        )
        .map_err(|_| Error::InvalidResponse)
    }
    /// Existing groups pin their signing fingerprint and accepted profile ID.
    pub fn group_authority(
        &self,
        fingerprint: [u8; 32],
        expected_profile: Option<[u8; 32]>,
    ) -> Result<Authority, Error> {
        self.group_authority_at(&self.server, fingerprint, expected_profile)
    }
    pub fn group_authority_at(
        &self,
        server: &str,
        fingerprint: [u8; 32],
        expected_profile: Option<[u8; 32]>,
    ) -> Result<Authority, Error> {
        let encoded: String = if server != self.server {
            serde_json::from_str(&self.federated_service(server, Service::GroupAuthority)?)
                .map_err(|_| Error::InvalidResponse)?
        } else {
            self.json(
                self.anonymous_group("/groups/v0/authority", None::<&()>)?,
                200,
                SMALL,
            )?
        };
        let profile = Authority::from_bytes(&decode(&encoded, 431)?, server, fingerprint)
            .map_err(|_| Error::InvalidResponse)?;
        if expected_profile.is_some_and(|expected| expected != profile.id()) {
            return Err(Error::InvalidResponse);
        }
        Ok(profile)
    }
    pub fn group_credential(
        &self,
        profile: &Authority,
        binding: &[u8],
        now: u64,
    ) -> Result<Credential, Error> {
        if !valid_time(now) {
            return Err(Error::Configuration);
        }
        let day = u32::try_from(now / 86400).map_err(|_| Error::Configuration)?;
        let issuance = profile
            .issuance(binding, day)
            .map_err(|_| Error::Configuration)?;
        let request = CredentialRequest {
            authority: hex(&profile.id()),
            day,
        };
        let response: CredentialResponse = if profile.server() != self.server {
            serde_json::from_str(&self.federated_service(
                profile.server(),
                Service::GroupCredential {
                    authority: request.authority.clone(),
                    day,
                    binding: hex(binding),
                },
            )?)
            .map_err(|_| Error::InvalidResponse)?
        } else {
            self.json(
                self.request(Method::POST, "/client/v0/groups/credential", Some(&request))?,
                200,
                SMALL,
            )?
        };
        if response.authority != request.authority
            || response.day != day
            || response.uid != hex(&issuance.uid)
        {
            return Err(Error::InvalidResponse);
        }
        Credential::accept(
            profile.issuer(),
            issuance.attributes().map_err(|_| Error::Configuration)?,
            day,
            issuance.context(),
            &decode(&response.response, 352)?,
        )
        .map_err(|_| Error::InvalidResponse)
    }
    /// Retain the exact encrypted operation durably before calling. Each attempt
    /// generates a fresh presentation/nonce; a lost reply retries that operation.
    pub fn group_request(
        &self,
        profile: &Authority,
        credential: &Credential,
        key: &GroupKey,
        group: [u8; 32],
        operation: &Operation,
        now: u64,
    ) -> Result<Reply, Error> {
        if !valid_time(now) {
            return Err(Error::Configuration);
        }
        let (kind, predecessor) = match operation {
            Operation::Create { .. } => (Kind::Create, [0; 32]),
            Operation::Read { .. } | Operation::Proposals { .. } => (Kind::Read, [0; 32]),
            Operation::Invite { .. } => (Kind::Advance, [0; 32]),
            Operation::Advance { predecessor, .. } | Operation::Relay { predecessor, .. } => (
                Kind::Advance,
                id(predecessor).map_err(|_| Error::Configuration)?,
            ),
        };
        let mut nonce = [0; 32];
        getrandom::fill(&mut nonce).map_err(|_| Error::Configuration)?;
        let bytes =
            Zeroizing::new(serde_json::to_vec(operation).map_err(|_| Error::Configuration)?);
        if bytes.len() > groups::MAX_BODY {
            return Err(Error::Limit);
        }
        let expires_at = now.checked_add(60).ok_or(Error::Configuration)?;
        let context = Context {
            operation: kind,
            group,
            predecessor,
            body_hash: Sha256::digest(&bytes).into(),
            nonce,
            expires_at,
        }
        .context(profile, now)
        .map_err(|_| Error::Configuration)?;
        let request = groups::Request {
            authority: hex(&profile.id()),
            group: hex(&group),
            day: u32::try_from(now / 86400).map_err(|_| Error::Configuration)?,
            nonce: hex(&nonce),
            expires_at,
            operation: operation.clone(),
            proof: hex(&credential
                .present(key, &context)
                .map_err(|_| Error::Configuration)?),
        };
        let reply: Reply = if profile.server() != self.server {
            serde_json::from_str(&self.federated_service(
                profile.server(),
                Service::GroupRequest {
                    request: serde_json::to_string(&request).map_err(|_| Error::Configuration)?,
                },
            )?)
            .map_err(|_| Error::InvalidResponse)?
        } else {
            self.json(
                self.anonymous_group("/groups/v0/request", Some(&request))?,
                200,
                groups::MAX_CONTROL * 4 + SMALL,
            )?
        };
        if reply.authority != request.authority
            || reply.group != request.group
            || reply.revision > i64::MAX as u64
            || id(&reply.head)? == [0; 32]
            || reply.commits.len() > 2
            || reply.proposals.len() > 2
        {
            return Err(Error::InvalidResponse);
        }
        if let Operation::Invite {
            id: requested,
            expires_at,
            ..
        } = operation
        {
            let invitation = reply.invitation.as_ref().ok_or(Error::InvalidResponse)?;
            if invitation.id != *requested
                || !reply.commits.is_empty()
                || !reply.proposals.is_empty()
                || reply.restored
                || (*expires_at == 0 && invitation.state == groups::InvitationState::Pending)
            {
                return Err(Error::InvalidResponse);
            }
            return Ok(reply);
        }
        if reply.invitation.is_some() {
            return Err(Error::InvalidResponse);
        }
        if let Operation::Relay {
            predecessor,
            head,
            revision,
            control,
        } = operation
        {
            if !reply.commits.is_empty() {
                if !reply.proposals.is_empty()
                    || reply.commits.len() != 1
                    || reply.revision != *revision
                    || reply.head != *head
                {
                    return Err(Error::InvalidResponse);
                }
                let record = &reply.commits[0];
                let receipt = Receipt::from_bytes(
                    &decode(
                        record.receipt.as_deref().ok_or(Error::InvalidResponse)?,
                        208,
                    )?,
                    profile.fingerprint(),
                )
                .map_err(|_| Error::InvalidResponse)?;
                if record.control != *control
                    || record.head != *head
                    || record.revision != *revision
                    || receipt.group != group
                    || receipt.head != id(head)?
                    || receipt.predecessor != id(predecessor)?
                    || receipt.revision != *revision
                {
                    return Err(Error::InvalidResponse);
                }
                return Ok(reply);
            }
        }
        if matches!(
            operation,
            Operation::Relay { .. } | Operation::Proposals { .. }
        ) {
            if !reply.commits.is_empty() {
                return Err(Error::InvalidResponse);
            }
            let mut previous = match operation {
                Operation::Proposals { after } => after.as_deref(),
                _ => None,
            };
            for proposal in &reply.proposals {
                let author = decode(&proposal.author, 64)?;
                sigil_crypto::private_credentials::validate_ciphertext(&author)
                    .map_err(|_| Error::InvalidResponse)?;
                if proposal.predecessor != reply.head
                    || proposal.revision
                        != reply
                            .revision
                            .checked_add(1)
                            .ok_or(Error::InvalidResponse)?
                    || id(&proposal.head)? == [0; 32]
                    || proposal.head == proposal.predecessor
                    || !valid_hex(&proposal.control, 2, groups::MAX_CONTROL * 2)
                    || previous.is_some_and(|p| p >= proposal.author.as_str())
                {
                    return Err(Error::InvalidResponse);
                }
                previous = Some(&proposal.author);
            }
            if let Operation::Relay {
                predecessor,
                head,
                revision,
                control,
            } = operation
            {
                if reply.proposals.len() != 1 {
                    return Err(Error::InvalidResponse);
                }
                let proposal = &reply.proposals[0];
                if proposal.predecessor != *predecessor
                    || proposal.head != *head
                    || proposal.revision != *revision
                    || proposal.control != *control
                {
                    return Err(Error::InvalidResponse);
                }
            }
            return Ok(reply);
        }
        if !reply.proposals.is_empty() {
            return Err(Error::InvalidResponse);
        }
        let mut previous: Option<&groups::Commit> = None;
        for commit in &reply.commits {
            if commit.revision > reply.revision
                || !valid_hex(&commit.control, 2, groups::MAX_CONTROL * 2)
                || previous.is_some_and(|p| p.revision.checked_add(1) != Some(commit.revision))
            {
                return Err(Error::InvalidResponse);
            }
            let head = id(&commit.head)?;
            if head == [0; 32] || (commit.revision == reply.revision && commit.head != reply.head) {
                return Err(Error::InvalidResponse);
            }
            match &commit.receipt {
                None if commit.revision == 0 => {}
                Some(bytes) if commit.revision > 0 => {
                    let receipt = Receipt::from_bytes(&decode(bytes, 208)?, profile.fingerprint())
                        .map_err(|_| Error::InvalidResponse)?;
                    if receipt.group != group
                        || receipt.head != head
                        || receipt.revision != commit.revision
                        || previous.is_some_and(|p| hex(&receipt.predecessor) != p.head)
                    {
                        return Err(Error::InvalidResponse);
                    }
                }
                _ => return Err(Error::InvalidResponse),
            }
            previous = Some(commit);
        }
        match operation {
            Operation::Relay { .. } | Operation::Proposals { .. } | Operation::Invite { .. } => {
                return Err(Error::InvalidResponse)
            }
            Operation::Read { from_revision } => {
                let count = if *from_revision > reply.revision {
                    0
                } else {
                    (reply.revision - from_revision + 1).min(2) as usize
                };
                if *from_revision > reply.revision.saturating_add(1)
                    || reply.commits.len() != count
                    || reply
                        .commits
                        .first()
                        .is_some_and(|c| c.revision != *from_revision)
                {
                    return Err(Error::InvalidResponse);
                }
            }
            Operation::Create { head, control, .. } | Operation::Advance { head, control, .. } => {
                let revision = match operation {
                    Operation::Advance { revision, .. } => *revision,
                    _ => 0,
                };
                if reply.revision != revision
                    || reply.head != *head
                    || reply.commits.len() != 1
                    || reply.commits[0].control != *control
                    || reply.commits[0].revision != revision
                {
                    return Err(Error::InvalidResponse);
                }
                if revision > 0 {
                    let receipt = Receipt::from_bytes(
                        &decode(
                            reply.commits[0]
                                .receipt
                                .as_deref()
                                .ok_or(Error::InvalidResponse)?,
                            208,
                        )?,
                        profile.fingerprint(),
                    )
                    .map_err(|_| Error::InvalidResponse)?;
                    if receipt.predecessor != predecessor {
                        return Err(Error::InvalidResponse);
                    }
                }
            }
        }
        Ok(reply)
    }
}

#[cfg(test)]
#[path = "group_network_tests.rs"]
mod tests;
