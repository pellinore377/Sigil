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
    pub fn push_android(&self) -> Result<push::AndroidProvider, Error> {
        let response: push::AndroidProvider = self.json(
            self.request(Method::GET, "/client/v0/push/android", None::<&()>)?,
            200,
            SMALL,
        )?;
        if response.android.as_ref().is_some_and(|c| !c.valid()) {
            return Err(Error::InvalidResponse);
        }
        Ok(response)
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::tests::{Fixture, CA};
    use axum::{routing::get, Json, Router};
    use std::sync::{Arc, Mutex};
    #[test]
    fn android_configuration_rejects_unknown_destinations_and_mismatched_sender_ids() {
        let value=Arc::new(Mutex::new(serde_json::json!({"android":null})));
        let response=value.clone();
        let fixture=Fixture::new(Router::new().route("/client/v0/push/android",get(move || {
            let value=response.lock().unwrap().clone();
            async move {Json(value)}
        })));
        let client=HttpsClient::new("chat.example",fixture.port(),&"ab".repeat(32),&[CA.to_vec()]).unwrap();
        assert!(client.push_android().unwrap().android.is_none());
        let good=serde_json::json!({"android":{
            "project_id":"synthetic-project","application_id":"1:123456789:android:0123456789abcdef",
            "api_key":format!("AIza{}","x".repeat(35)),"sender_id":"123456789"
        }});
        *value.lock().unwrap()=good.clone();
        assert_eq!(client.push_android().unwrap().android.unwrap().project_id,"synthetic-project");
        for (field,bad) in [("sender_id","987654321"),("api_key","invalid"),("token_uri","https://attacker.example")]{
            let mut changed=good.clone();changed["android"][field]=bad.into();
            *value.lock().unwrap()=changed;
            assert_eq!(client.push_android(),Err(Error::InvalidResponse));
        }
    }
}
