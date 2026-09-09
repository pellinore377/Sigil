use super::*;
use sigil_protocol::{admin::*, oidc};
impl HttpsClient {
    pub fn oidc_access(&self) -> Result<oidc::Access, Error> {
        self.json(
            self.request(Method::GET, "/client/v0/oidc/access", None::<&()>)?,
            200,
            SMALL,
        )
    }
    pub fn acknowledge_oidc_fallback(
        &self,
        value: &oidc::AcknowledgeFallback,
    ) -> Result<oidc::Access, Error> {
        let result: oidc::Access = self.json(
            self.request(Method::POST, "/client/v0/oidc/access", Some(value))?,
            200,
            SMALL,
        )?;
        if !result.invitation_fallback_acknowledged
            || result.configuration_revision != value.configuration_revision
            || result.transition_revision != value.transition_revision
        {
            return Err(Error::InvalidResponse);
        }
        Ok(result)
    }
    pub fn profile(&self) -> Result<sigil_protocol::profile::Profile, Error> {
        self.json(
            self.request(Method::GET, "/client/v0/profile", None::<&()>)?,
            200,
            SMALL,
        )
    }
    pub fn set_profile(
        &self,
        value: &sigil_protocol::profile::Profile,
    ) -> Result<sigil_protocol::profile::Profile, Error> {
        if !sigil_protocol::profile::valid_name(&value.display_name) {
            return Err(Error::Configuration);
        }
        let updated: sigil_protocol::profile::Profile = self.json(
            self.request(Method::PUT, "/client/v0/profile", Some(value))?,
            200,
            SMALL,
        )?;
        if value.revision.checked_add(1) != Some(updated.revision)
            || value.display_name != updated.display_name
        {
            return Err(Error::InvalidResponse);
        }
        Ok(updated)
    }
    pub fn discover_account(&self, username: &str) -> Result<FoundAccount, Error> {
        if !accounts::valid_username(username) {
            return Err(Error::Configuration);
        }
        let value: FoundAccount = self.json(
            self.request(
                Method::POST,
                "/client/v0/discovery",
                Some(&serde_json::json!({"username":username})),
            )?,
            200,
            16384,
        )?;
        if !value.valid_for(username, &self.server) {
            return Err(Error::InvalidResponse);
        }
        Ok(value)
    }
    pub fn discovery_preference(&self) -> Result<DiscoveryPreference, Error> {
        self.json(
            self.request(Method::GET, "/client/v0/discovery/preference", None::<&()>)?,
            200,
            SMALL,
        )
    }
    pub fn set_discovery_preference(
        &self,
        value: &DiscoveryPreference,
    ) -> Result<DiscoveryPreference, Error> {
        let updated: DiscoveryPreference = self.json(
            self.request(Method::PUT, "/client/v0/discovery/preference", Some(value))?,
            200,
            SMALL,
        )?;
        if value.revision.checked_add(1) != Some(updated.revision)
            || value.discoverable != updated.discoverable
        {
            return Err(Error::InvalidResponse);
        }
        Ok(updated)
    }
    pub fn start_oidc(&self, request: &oidc::Start) -> Result<oidc::Started, Error> {
        let value: oidc::Started = self.json(
            self.request(Method::POST, "/client/v0/oidc/start", Some(request))?,
            200,
            8192,
        )?;
        let url = value
            .authorization_url
            .parse::<ureq::http::Uri>()
            .map_err(|_| Error::InvalidResponse)?;
        if url.scheme_str() != Some("https")
            || url.authority().is_none_or(|a| a.as_str().contains('@'))
            || value.authorization_url.contains('#')
            || value.authorization_url.len() > 4096
            || !valid_time(value.expires_at)
        {
            return Err(Error::InvalidResponse);
        }
        Ok(value)
    }
    pub fn finish_oidc(&self, request: &oidc::Finish) -> Result<oidc::Progress, Error> {
        self.json(
            self.request(Method::POST, "/client/v0/oidc/finish", Some(request))?,
            200,
            SMALL,
        )
    }
    pub fn oidc_registration_name(&self, request: &oidc::RegistrationName) -> Result<(), Error> {
        self.json(
            self.request(Method::POST, "/client/v0/oidc/username", Some(request))?,
            200,
            SMALL,
        )
    }
    pub fn link_oidc(&self, request: &oidc::Start) -> Result<oidc::Started, Error> {
        self.json(
            self.request(Method::POST, "/client/v0/oidc/link", Some(request))?,
            200,
            8192,
        )
    }
    pub fn oidc_bindings(&self) -> Result<Vec<String>, Error> {
        self.json(
            self.request(Method::GET, "/client/v0/oidc/bindings", None::<&()>)?,
            200,
            SMALL,
        )
    }
    pub fn unlink_oidc(&self, issuer: &str) -> Result<(), Error> {
        self.json(
            self.request(
                Method::DELETE,
                "/client/v0/oidc/bindings",
                Some(&serde_json::json!({"issuer":issuer,"confirm":true})),
            )?,
            200,
            SMALL,
        )
    }
}
