use super::*;
use sigil_protocol::contacts::*;

impl HttpsClient {
    pub fn contact_requests(&self, after: Option<&str>) -> Result<RequestPage, Error> {
        if after.is_some_and(|v| !accounts::valid_credential(v)) {
            return Err(Error::Configuration);
        }
        let path = after
            .map(|v| format!("/client/v0/contact-requests?after={v}"))
            .unwrap_or_else(|| "/client/v0/contact-requests".into());
        let page: RequestPage =
            self.json(self.request(Method::GET, &path, None::<&()>)?, 200, 131072)?;
        if page.requests.len() > 32
            || page.next.as_ref().is_some_and(|v| {
                !accounts::valid_credential(v) || after.is_some_and(|a| v.as_str() <= a)
            })
        {
            return Err(Error::InvalidResponse);
        }
        let mut previous = after.unwrap_or("");
        for request in &page.requests {
            if !accounts::valid_credential(&request.receipt.id)
                || request.receipt.id.as_str() <= previous
                || request.receipt.state != RequestState::Pending
            {
                return Err(Error::InvalidResponse);
            }
            previous = &request.receipt.id;
        }
        if page.next.as_ref().is_some_and(|v| v != previous) {
            return Err(Error::InvalidResponse);
        }
        Ok(page)
    }
    pub fn request_contact(&self, request: &RequestContact) -> Result<RequestReceipt, Error> {
        if !request.valid() || request.server != self.server {
            return Err(Error::Configuration);
        }
        self.json(
            self.request(Method::POST, "/client/v0/contact-requests", Some(request))?,
            200,
            SMALL,
        )
    }
    pub fn contact_request_status(&self, recipient: &str) -> Result<RequestReceipt, Error> {
        if !accounts::valid_credential(recipient) {
            return Err(Error::Configuration);
        }
        self.json(
            self.request(
                Method::GET,
                &format!("/client/v0/contact-requests/outgoing/{recipient}"),
                None::<&()>,
            )?,
            200,
            SMALL,
        )
    }
    pub fn incoming_contact_status(
        &self,
        id: &str,
        signature: &str,
    ) -> Result<RequestReceipt, Error> {
        if !accounts::valid_credential(id)
            || signature.len() != 128
            || !signature
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Error::Configuration);
        }
        self.json(
            self.request(
                Method::GET,
                &format!("/client/v0/contact-requests/{id}?signature={signature}"),
                None::<&()>,
            )?,
            200,
            SMALL,
        )
    }
    pub fn resolve_contact_request(
        &self,
        id: &str,
        state: RequestState,
        signature: &str,
    ) -> Result<RequestReceipt, Error> {
        if !accounts::valid_credential(id)
            || state == RequestState::Pending
            || signature.len() != 128
            || !signature
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Error::Configuration);
        }
        self.json(
            self.request(
                Method::PUT,
                &format!("/client/v0/contact-requests/{id}"),
                Some(&ResolveRequest {
                    state,
                    signature: signature.into(),
                }),
            )?,
            200,
            SMALL,
        )
    }
    pub fn contact_request_policy(
        &self,
        policy: Option<&RequestPolicy>,
    ) -> Result<RequestPolicy, Error> {
        self.json(
            self.request(
                if policy.is_some() {
                    Method::PUT
                } else {
                    Method::GET
                },
                "/client/v0/contact-requests/policy",
                policy,
            )?,
            200,
            SMALL,
        )
    }
    pub fn block_contact_requests(&self, request: &BlockContact) -> Result<(), Error> {
        if !sigil_protocol::valid_server_name(&request.server)
            || !accounts::valid_credential(&request.account)
        {
            return Err(Error::Configuration);
        }
        self.empty(self.request(
            Method::PUT,
            "/client/v0/contact-requests/blocked",
            Some(request),
        )?)
    }
}
