use super::*;
use sigil_protocol::profile::{ContactProfile, Photo, SetPhoto, ShareProfile, MAX_PHOTO};
fn valid(photo: &Photo) -> bool {
    photo.revision <= i64::MAX as u64
        && (photo.revision != 0 || photo.hash.is_none())
        && photo.bytes as usize <= MAX_PHOTO
        && match &photo.hash {
            Some(hash) => photo.bytes > 0 && accounts::valid_credential(hash),
            None => photo.bytes == 0,
        }
}
impl HttpsClient {
    pub fn profile_photo(&self) -> Result<Photo, Error> {
        let photo: Photo = self.json(
            self.request(Method::GET, "/client/v0/profile/photo", None::<&()>)?,
            200,
            SMALL,
        )?;
        if !valid(&photo) {
            return Err(Error::InvalidResponse);
        }
        Ok(photo)
    }
    pub fn set_profile_photo(&self, request: &SetPhoto) -> Result<Photo, Error> {
        let bytes = sigil_protocol::profile::photo_bytes(&request.photo)
            .map_err(|_| Error::Configuration)?;
        let body = Zeroizing::new(serde_json::to_vec(request).map_err(|_| Error::Configuration)?);
        if body.len() > 2 * MAX_PHOTO + 1024 {
            return Err(Error::Limit);
        }
        let response: Photo = self.json(
            self.request_bytes(
                Method::PUT,
                "/client/v0/profile/photo",
                &body,
                Some("application/json"),
                None,
            )?,
            200,
            SMALL,
        )?;
        let hash = (!bytes.is_empty()).then(|| crate::transport::hex(&Sha256::digest(&bytes)));
        if !valid(&response)
            || request.revision.checked_add(1) != Some(response.revision)
            || response.bytes as usize != bytes.len()
            || response.hash != hash
        {
            return Err(Error::InvalidResponse);
        }
        Ok(response)
    }
    pub fn share_profile(&self, request: &ShareProfile) -> Result<(), Error> {
        if !sigil_protocol::valid_server_name(&request.server)
            || !accounts::valid_credential(&request.account)
        {
            return Err(Error::Configuration);
        }
        self.empty(self.request(Method::PUT, "/client/v0/profile/shares", Some(request))?)
    }
    pub fn contact_profile(
        &self,
        own: &accounts::Session,
        server: &str,
        account: &str,
    ) -> Result<ContactProfile, Error> {
        if !accounts::valid_credential(account) {
            return Err(Error::Configuration);
        }
        let response: ContactProfile = if server == self.server {
            self.json(
                self.request(
                    Method::GET,
                    &format!("/client/v0/profiles/{account}"),
                    None::<&()>,
                )?,
                200,
                SMALL,
            )?
        } else {
            let value = self.federated_lookup(
                own,
                &sigil_protocol::federation::ProxyLookup {
                    destination: server.into(),
                    operation: sigil_protocol::federation::Lookup::Service {
                        service: sigil_protocol::federation::Service::ContactProfile {
                            account: account.into(),
                        },
                    },
                },
            )?;
            let sigil_protocol::federation::LookupValue::Service(raw) = value else {
                return Err(Error::InvalidResponse);
            };
            if raw.len() > SMALL {
                return Err(Error::Limit);
            }
            serde_json::from_str(&raw).map_err(|_| Error::InvalidResponse)?
        };
        if response.profile.revision > i64::MAX as u64
            || !sigil_protocol::profile::valid_name(&response.profile.display_name)
            || !valid(&response.photo)
        {
            return Err(Error::InvalidResponse);
        }
        Ok(response)
    }
    pub fn contact_photo(
        &self,
        own: &accounts::Session,
        server: &str,
        account: &str,
        photo: &Photo,
    ) -> Result<Zeroizing<Vec<u8>>, Error> {
        if !valid(photo) || !accounts::valid_credential(account) {
            return Err(Error::Configuration);
        }
        let hash = photo.hash.as_ref().ok_or(Error::Configuration)?;
        let bytes = if server == self.server {
            self.response_bytes(
                self.request(
                    Method::GET,
                    &format!("/client/v0/profiles/{account}/photo/{hash}"),
                    None::<&()>,
                )?,
                200,
                MAX_PHOTO,
                "image/jpeg",
            )?
        } else {
            let value = self.federated_lookup(
                own,
                &sigil_protocol::federation::ProxyLookup {
                    destination: server.into(),
                    operation: sigil_protocol::federation::Lookup::Service {
                        service: sigil_protocol::federation::Service::ContactPhoto {
                            account: account.into(),
                            hash: hash.clone(),
                        },
                    },
                },
            )?;
            let sigil_protocol::federation::LookupValue::Service(raw) = value else {
                return Err(Error::InvalidResponse);
            };
            Zeroizing::new(
                sigil_protocol::profile::photo_bytes(&raw).map_err(|_| Error::InvalidResponse)?,
            )
        };
        if bytes.len() != photo.bytes as usize
            || crate::transport::hex(&Sha256::digest(&bytes)) != *hash
        {
            return Err(Error::InvalidResponse);
        }
        Ok(bytes)
    }
}
