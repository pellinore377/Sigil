use super::*;
use sigil_protocol::push::{self, Confirm, Disable, Providers, Register, State, Status, Target};

pub(crate) fn valid_status(status: &Status) -> bool {
    if status.revision > i64::MAX as u64
        || status
            .channel
            .as_ref()
            .is_some_and(|c| !accounts::valid_credential(c))
        || status.expires_at.is_some_and(|v| !valid_time(v))
    {
        return false;
    }
    match status.state {
        State::Disabled => status.channel.is_none() && status.expires_at.is_none(),
        State::Pending | State::Active => {
            status.revision > 0 && status.channel.is_some() && status.expires_at.is_some()
        }
        State::Invalid | State::Expired => status.revision > 0 && status.expires_at.is_none(),
    }
}
pub(crate) fn target_fields(target: &Target) -> bool {
    match target {
        Target::Fcm { token } => push::valid_token(token),
        Target::UnifiedPush {
            endpoint,
            public_key,
            auth_secret,
            vapid_key,
        } => {
            let uri = endpoint.parse::<ureq::http::Uri>();
            endpoint.len() <= push::MAX_ENDPOINT
                && endpoint.is_ascii()
                && !endpoint.contains('#')
                && uri.is_ok_and(|u| {
                    u.scheme_str() == Some("https")
                        && u.host().is_some_and(|h| {
                            let h = h.trim_start_matches('[').trim_end_matches(']');
                            sigil_protocol::valid_server_name(h)
                                || h.parse::<std::net::IpAddr>().is_ok()
                        })
                        && u.port_u16() != Some(0)
                        && u.authority().is_some_and(|a| !a.as_str().contains('@'))
                })
                && public_key.len() == 87
                && auth_secret.len() == 22
                && vapid_key.len() == 87
        }
    }
}
impl HttpsClient {
    pub fn push_providers(&self) -> Result<Providers, Error> {
        let response: Providers = self.json(
            self.request(Method::GET, "/client/v0/push/providers", None::<&()>)?,
            200,
            SMALL,
        )?;
        if response.unified_push != response.vapid_public_key.is_some()
            || response
                .vapid_public_key
                .as_ref()
                .is_some_and(|k| k.len() != 87)
        {
            return Err(Error::InvalidResponse);
        }
        Ok(response)
    }
    pub fn push_status(&self) -> Result<Status, Error> {
        let response: Status = self.json(
            self.request(Method::GET, "/client/v0/push", None::<&()>)?,
            200,
            SMALL,
        )?;
        if !valid_status(&response) {
            return Err(Error::InvalidResponse);
        }
        Ok(response)
    }
    pub fn register_push(&self, request: &Register) -> Result<Status, Error> {
        if request.expected_revision >= i64::MAX as u64 || !target_fields(&request.target) {
            return Err(Error::Configuration);
        }
        let response: Status = self.json(
            self.request(Method::PUT, "/client/v0/push", Some(request))?,
            200,
            SMALL,
        )?;
        if !valid_status(&response)
            || response.revision != request.expected_revision + 1
            || response.state == State::Disabled
        {
            return Err(Error::InvalidResponse);
        }
        Ok(response)
    }
    pub fn confirm_push(&self, request: &Confirm) -> Result<Status, Error> {
        if request.revision == 0
            || request.revision > i64::MAX as u64
            || !accounts::valid_credential(&request.channel)
            || !accounts::valid_credential(&request.proof)
        {
            return Err(Error::Configuration);
        }
        let response: Status = self.json(
            self.request(Method::POST, "/client/v0/push/confirm", Some(request))?,
            200,
            SMALL,
        )?;
        if !valid_status(&response)
            || response.revision != request.revision
            || response.channel.as_ref() != Some(&request.channel)
            || response.state != State::Active
        {
            return Err(Error::InvalidResponse);
        }
        Ok(response)
    }
    pub fn disable_push(&self, request: &Disable) -> Result<Status, Error> {
        if request.expected_revision >= i64::MAX as u64 {
            return Err(Error::Configuration);
        }
        let response: Status = self.json(
            self.request(Method::DELETE, "/client/v0/push", Some(request))?,
            200,
            SMALL,
        )?;
        if !valid_status(&response)
            || response.revision != request.expected_revision + 1
            || response.state != State::Disabled
        {
            return Err(Error::InvalidResponse);
        }
        Ok(response)
    }
}
