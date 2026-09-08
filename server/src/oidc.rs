use crate::{
    auth::{digest, random_secret},
    egress,
    push_config::{sql, unsigned},
    store::{Store, StoreError},
};
use openidconnect::{
    core::{CoreAuthenticationFlow, CoreClient, CoreJwsSigningAlgorithm, CoreProviderMetadata},
    AccessTokenHash, AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce,
    OAuth2TokenResponse, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, TokenResponse,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sigil_protocol::{accounts::valid_credential, oidc::*};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

pub(crate) const MIGRATION: &str = "
CREATE TABLE oidc_configuration(id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL,value TEXT);
INSERT INTO oidc_configuration VALUES(1,0,NULL);
CREATE TABLE oidc_flows(id TEXT PRIMARY KEY,request_hash BLOB NOT NULL,secret_hash BLOB NOT NULL,state TEXT NOT NULL UNIQUE,revision INTEGER NOT NULL,expires INTEGER NOT NULL,value TEXT NOT NULL,status INTEGER NOT NULL DEFAULT 0,subject TEXT,lease INTEGER NOT NULL DEFAULT 0,completion_hash BLOB);
CREATE INDEX oidc_flows_expiry ON oidc_flows(expires);
CREATE TABLE oidc_bindings(issuer TEXT NOT NULL,subject TEXT NOT NULL,account TEXT NOT NULL REFERENCES accounts(id),PRIMARY KEY(issuer,subject),UNIQUE(issuer,account));
CREATE TABLE oidc_grants(token_hash BLOB PRIMARY KEY,issuer TEXT NOT NULL,subject TEXT NOT NULL,username TEXT NOT NULL,reauthorize INTEGER NOT NULL,expires INTEGER NOT NULL);
";
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: Option<Zeroizing<String>>,
    pub exceptions: Vec<egress::Exception>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Configure {
    pub expected_revision: u64,
    pub provider: Option<Provider>,
    pub confirm: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub revision: u64,
    pub enabled: bool,
    pub issuer: Option<String>,
    pub client_id: Option<String>,
    pub secret_configured: bool,
    pub redirect_uri: Option<String>,
    pub exceptions: Vec<egress::Exception>,
}
#[derive(Clone, Deserialize, Serialize)]
struct Stored {
    provider: Provider,
    metadata: CoreProviderMetadata,
    redirect: String,
}
#[derive(Deserialize, Serialize)]
struct Flow {
    username: Option<String>,
    replace_devices: bool,
    link_device: Option<String>,
    nonce: Zeroizing<String>,
    verifier: Zeroizing<String>,
    authorization_url: String,
}
fn decode<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, StoreError> {
    serde_json::from_str(text).map_err(|_| StoreError::InvalidData)
}
fn encode<T: Serialize>(value: &T) -> Result<Zeroizing<String>, StoreError> {
    serde_json::to_string(value)
        .map(Zeroizing::new)
        .map_err(|_| StoreError::InvalidData)
}
fn read(db: &Connection) -> Result<(u64, Option<Stored>), StoreError> {
    let (revision, value): (u64, Option<String>) = db.query_row(
        "SELECT revision,value FROM oidc_configuration WHERE id=1",
        [],
        |r| Ok((unsigned(r, 0)?, r.get(1)?)),
    )?;
    Ok((
        revision,
        value.map(Zeroizing::new).map(|v| decode(&v)).transpose()?,
    ))
}
fn http(
    provider: &Provider,
    request: openidconnect::HttpRequest,
) -> Result<openidconnect::HttpResponse, std::io::Error> {
    let (parts, body) = request.into_parts();
    let result = egress::Policy::new(provider.exceptions.clone())
        .and_then(|p| p.service(ureq::http::Request::from_parts(parts, &body)))
        .map_err(|_| std::io::Error::other("OIDC transport failed"))?;
    let mut response = ureq::http::Response::builder().status(result.status);
    if let Some(content_type) = result.content_type {
        response = response.header("content-type", content_type);
    }
    response
        .body(result.body.to_vec())
        .map_err(|_| std::io::Error::other("invalid OIDC response"))
}
fn metadata(provider: &Provider) -> Result<CoreProviderMetadata, StoreError> {
    let metadata = CoreProviderMetadata::discover(
        &IssuerUrl::new(provider.issuer.clone())
            .map_err(|_| StoreError::Invalid("invalid issuer"))?,
        &|r| http(provider, r),
    )
    .map_err(|_| StoreError::Invalid("OIDC discovery or JWKS validation failed"))?;
    for endpoint in [
        metadata.authorization_endpoint().url(),
        metadata.jwks_uri().url(),
        metadata
            .token_endpoint()
            .ok_or(StoreError::Invalid("OIDC token endpoint required"))?
            .url(),
    ] {
        egress::endpoint(endpoint.as_str())
            .map_err(|_| StoreError::Invalid("OIDC endpoints must use HTTPS"))?;
        if endpoint.origin() != metadata.issuer().url().origin() {
            return Err(StoreError::Invalid(
                "OIDC endpoints must share the issuer origin",
            ));
        }
    }
    if metadata.jwks().keys().is_empty() || metadata.jwks().keys().len() > 16 {
        return Err(StoreError::Invalid("OIDC requires 1–16 signing keys"));
    }
    authentication(provider, &metadata)?;
    Ok(metadata)
}
fn authentication(
    provider: &Provider,
    metadata: &CoreProviderMetadata,
) -> Result<openidconnect::AuthType, StoreError> {
    use openidconnect::{core::CoreClientAuthMethod as Method, AuthType};
    if provider.client_secret.is_none() {
        return Ok(AuthType::RequestBody);
    }
    match metadata.token_endpoint_auth_methods_supported() {
        None => Ok(AuthType::BasicAuth),
        Some(methods) if methods.contains(&Method::ClientSecretBasic) => Ok(AuthType::BasicAuth),
        Some(methods) if methods.contains(&Method::ClientSecretPost) => Ok(AuthType::RequestBody),
        _ => Err(StoreError::Invalid(
            "unsupported OIDC client authentication method",
        )),
    }
}
impl Store {
    pub fn oidc_configuration(&self) -> Result<Configuration, StoreError> {
        let (revision, stored) = read(&self.0)?;
        Ok(Configuration {
            revision,
            enabled: stored.is_some(),
            issuer: stored.as_ref().map(|s| s.provider.issuer.clone()),
            client_id: stored.as_ref().map(|s| s.provider.client_id.clone()),
            secret_configured: stored
                .as_ref()
                .is_some_and(|s| s.provider.client_secret.is_some()),
            exceptions: stored
                .as_ref()
                .map(|s| s.provider.exceptions.clone())
                .unwrap_or_default(),
            redirect_uri: stored.map(|s| s.redirect),
        })
    }
    pub(crate) fn oidc_install(
        &mut self,
        update: Configure,
        metadata: Option<CoreProviderMetadata>,
    ) -> Result<Configuration, StoreError> {
        if !update.confirm {
            return Err(StoreError::Invalid(
                "confirm the authentication provider change",
            ));
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (revision, _) = read(&tx)?;
        if revision != update.expected_revision {
            return Err(StoreError::Conflict);
        }
        let password_login: bool =
            tx.query_row("SELECT password_login FROM web_owner", [], |r| r.get(0))?;
        if !password_login {
            return Err(StoreError::Invalid(
                "Enable administrator password login before changing OIDC",
            ));
        }
        let stored = match update.provider {
            Some(provider) => Some(Stored {
                provider,
                metadata: metadata.ok_or(StoreError::InvalidData)?,
                redirect: format!(
                    "{}/auth/v0/oidc/callback",
                    crate::admin::policy(&tx)?
                        .public_origin
                        .ok_or(StoreError::Invalid("configure public_origin before OIDC"))?
                ),
            }),
            None => None,
        };
        let value = stored.as_ref().map(encode).transpose()?;
        tx.execute(
            "UPDATE oidc_configuration SET revision=?1,value=?2 WHERE id=1",
            (
                sql(crate::push_config::next(revision)?)?,
                value.as_deref().map(|v| v.as_str()),
            ),
        )?;
        tx.execute("DELETE FROM oidc_flows", [])?;
        tx.execute("DELETE FROM oidc_grants", [])?;
        tx.execute("DELETE FROM web_oidc", [])?;
        tx.commit()?;
        self.oidc_configuration()
    }
    pub fn oidc_start(
        &mut self,
        request: Start,
        link: Option<&str>,
        now: u64,
    ) -> Result<Started, StoreError> {
        self.oidc_begin(request, link, now, false)
    }
    pub(crate) fn oidc_begin(
        &mut self,
        request: Start,
        link: Option<&str>,
        now: u64,
        profile: bool,
    ) -> Result<Started, StoreError> {
        if !valid_credential(&request.request_id)
            || !valid_credential(&request.secret)
            || request
                .username
                .as_ref()
                .is_some_and(|v| !sigil_protocol::accounts::valid_username(v))
        {
            return Err(StoreError::Invalid("invalid OIDC request"));
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (revision, stored) = read(&tx)?;
        let stored = stored.ok_or(StoreError::Forbidden)?;
        if Some(format!(
            "{}/auth/v0/oidc/callback",
            crate::admin::policy(&tx)?
                .public_origin
                .ok_or(StoreError::Forbidden)?
        )) != Some(stored.redirect.clone())
        {
            return Err(StoreError::Forbidden);
        }
        let link_device = link
            .map(|c| crate::prekeys::authorize(&tx, c, now))
            .transpose()?;
        if link_device.is_some() && (request.username.is_some() || request.replace_devices) {
            return Err(StoreError::Invalid("linking cannot replace devices"));
        }
        let hash = crate::push_config::hash(b"Sigil/OIDC/start/v0", &(&request, &link_device))?;
        tx.execute("DELETE FROM oidc_flows WHERE expires<=?1", [sql(now)?])?;
        let previous: Option<(Vec<u8>, u64, String)> = tx
            .query_row(
                "SELECT request_hash,expires,value FROM oidc_flows WHERE id=?1",
                [&request.request_id],
                |r| Ok((r.get(0)?, unsigned(r, 1)?, r.get(2)?)),
            )
            .optional()?;
        if let Some((old, expires_at, value)) = previous {
            if old != hash {
                return Err(StoreError::Conflict);
            }
            return Ok(Started {
                authorization_url: decode::<Flow>(&value)?.authorization_url,
                expires_at,
            });
        }
        if tx.query_row("SELECT count(*) FROM oidc_flows", [], |r| {
            r.get::<_, u32>(0)
        })? >= 128
        {
            return Err(StoreError::Busy);
        }
        let nonce = random_secret().map_err(|_| StoreError::InvalidData)?;
        let state = random_secret().map_err(|_| StoreError::InvalidData)?;
        let verifier = Zeroizing::new(random_secret().map_err(|_| StoreError::InvalidData)?);
        let client = CoreClient::from_provider_metadata(
            stored.metadata,
            ClientId::new(stored.provider.client_id),
            stored
                .provider
                .client_secret
                .map(|s| ClientSecret::new(s.to_string())),
        )
        .set_redirect_uri(RedirectUrl::new(stored.redirect).map_err(|_| StoreError::InvalidData)?);
        let csrf = state.clone();
        let token_nonce = nonce.clone();
        let (url, _, _) = client
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                move || CsrfToken::new(csrf),
                move || Nonce::new(token_nonce),
            )
            .set_pkce_challenge(PkceCodeChallenge::from_code_verifier_sha256(
                &PkceCodeVerifier::new(verifier.to_string()),
            ))
            .add_scopes(profile.then(|| openidconnect::Scope::new("profile".into())))
            .url();
        let authorization_url = url.to_string();
        let flow = Flow {
            username: request.username,
            replace_devices: request.replace_devices,
            link_device,
            nonce: Zeroizing::new(nonce),
            verifier,
            authorization_url: authorization_url.clone(),
        };
        let expires_at = crate::push::deadline(now, 600)?;
        tx.execute("INSERT INTO oidc_flows(id,request_hash,secret_hash,state,revision,expires,value) VALUES(?1,?2,?3,?4,?5,?6,?7)",(&request.request_id,hash.as_slice(),digest(&request.secret).as_slice(),state,sql(revision)?,sql(expires_at)?,encode(&flow)?.as_str()))?;
        tx.commit()?;
        Ok(Started {
            authorization_url,
            expires_at,
        })
    }
    pub(crate) fn oidc_claim(
        &mut self,
        state: &str,
        now: u64,
    ) -> Result<Option<Callback>, StoreError> {
        if !valid_credential(state) {
            return Err(StoreError::Unauthorized);
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (revision, stored) = read(&tx)?;
        let (id,value,status,lease):(String,String,u8,u64)=tx.query_row("SELECT id,value,status,lease FROM oidc_flows WHERE state=?1 AND revision=?2 AND expires>?3",(state,sql(revision)?,sql(now)?),|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,unsigned(r,3)?))).optional()?.ok_or(StoreError::Unauthorized)?;
        if status != 0 {
            return Ok(None);
        }
        if lease > now {
            return Err(StoreError::Busy);
        }
        tx.execute(
            "UPDATE oidc_flows SET lease=?2 WHERE id=?1",
            (&id, sql(crate::push::deadline(now, 60)?)?),
        )?;
        tx.commit()?;
        Ok(Some(Callback {
            id,
            revision,
            stored: stored.ok_or(StoreError::Forbidden)?,
            flow: decode(&value)?,
        }))
    }
    pub(crate) fn oidc_verified(
        &mut self,
        callback: Callback,
        subject: Result<String, StoreError>,
        now: u64,
    ) -> Result<Option<Completion>, StoreError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if read(&tx)?.0 != callback.revision {
            return Err(StoreError::Conflict);
        }
        let (status, subject) = match subject {
            Ok(v) => (1, Some(v)),
            Err(_) => (3, None),
        };
        let completion = if status == 1 {
            Some(Completion {
                request_id: callback.id.clone(),
                secret: random_secret().map_err(|_| StoreError::InvalidData)?,
            })
        } else {
            None
        };
        if tx.execute("UPDATE oidc_flows SET status=?2,subject=?3,value=?4,lease=0 WHERE id=?1 AND status=0 AND expires>?5",(&callback.id,status,subject,encode(&Flow {verifier:Zeroizing::new(String::new()),nonce:Zeroizing::new(String::new()),..callback.flow})?.as_str(),sql(now)?))?!=1 {return Err(StoreError::Conflict);}
        tx.execute(
            "UPDATE oidc_flows SET completion_hash=?2 WHERE id=?1",
            (
                &callback.id,
                completion.as_ref().map(|v| digest(&v.secret).to_vec()),
            ),
        )?;
        tx.commit()?;
        Ok(completion)
    }
    pub fn oidc_finish(&mut self, request: Finish, now: u64) -> Result<Progress, StoreError> {
        if !valid_credential(&request.request_id) || !valid_credential(&request.secret) {
            return Err(StoreError::Unauthorized);
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (revision, stored) = read(&tx)?;
        let issuer = stored.ok_or(StoreError::Forbidden)?.provider.issuer;
        let (hash,status,value,subject,expires):(Vec<u8>,u8,String,Option<String>,u64)=tx.query_row("SELECT secret_hash,status,value,subject,expires FROM oidc_flows WHERE id=?1 AND revision=?2 AND expires>?3",(&request.request_id,sql(revision)?,sql(now)?),|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,unsigned(r,4)?))).optional()?.ok_or(StoreError::Unauthorized)?;
        if !bool::from(hash.as_slice().ct_eq(&digest(&request.secret))) {
            return Err(StoreError::Unauthorized);
        }
        if status == 0 {
            return Ok(Progress::Pending);
        }
        if status == 3 {
            return Ok(Progress::Failed);
        }
        let Some(completion) = request.completion else {
            return Ok(Progress::Pending);
        };
        let expected: Vec<u8> = tx.query_row(
            "SELECT completion_hash FROM oidc_flows WHERE id=?1",
            [&request.request_id],
            |r| r.get(0),
        )?;
        if !valid_credential(&completion)
            || !bool::from(expected.as_slice().ct_eq(&digest(&completion)))
        {
            return Err(StoreError::Unauthorized);
        }
        let flow: Flow = decode(&value)?;
        let subject = subject.ok_or(StoreError::InvalidData)?;
        if let Some(device) = flow.link_device {
            if !crate::prekeys::active(&tx, &device)? {
                return Err(StoreError::Unauthorized);
            }
            let account: String = tx
                .query_row(
                    "SELECT account_id FROM devices WHERE id=?1 AND expires_at>?2",
                    (device, sql(now)?),
                    |r| r.get(0),
                )
                .optional()?
                .ok_or(StoreError::Unauthorized)?;
            if status == 2 {
                let bound:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM oidc_bindings WHERE issuer=?1 AND subject=?2 AND account=?3)",(&issuer,&subject,&account),|r|r.get(0))?;
                return if bound {
                    Ok(Progress::Linked)
                } else {
                    Err(StoreError::Conflict)
                };
            }
            bind(&tx, &issuer, &subject, &account)?;
            tx.execute(
                "UPDATE oidc_flows SET status=2 WHERE id=?1",
                [&request.request_id],
            )?;
            tx.commit()?;
            return Ok(Progress::Linked);
        }
        let bound:Option<(String,bool)>=tx.query_row("SELECT a.username,a.disabled FROM oidc_bindings b JOIN accounts a ON a.id=b.account WHERE b.issuer=?1 AND b.subject=?2",(&issuer,&subject),|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let (username, reauthorize) = match bound {
            Some((name, false))
                if flow.replace_devices && flow.username.as_ref().is_none_or(|u| u == &name) =>
            {
                (name, true)
            }
            Some(_) => return Err(StoreError::Forbidden),
            None if crate::admin::policy(&tx)?.registration
                == sigil_protocol::admin::Registration::Oidc =>
            {
                (
                    flow.username
                        .ok_or(StoreError::Invalid("choose a username for registration"))?,
                    false,
                )
            }
            None => return Err(StoreError::Forbidden),
        };
        if status == 1 {
            let existing: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM accounts WHERE username=?1)",
                [&username],
                |r| r.get(0),
            )?;
            if existing != reauthorize {
                return Err(StoreError::Conflict);
            }
            tx.execute("DELETE FROM oidc_grants WHERE expires<=?1", [sql(now)?])?;
            tx.execute(
                "INSERT INTO oidc_grants VALUES(?1,?2,?3,?4,?5,?6)",
                (
                    digest(&request.secret).as_slice(),
                    issuer,
                    subject,
                    username,
                    reauthorize,
                    sql(expires)?,
                ),
            )?;
            tx.execute(
                "UPDATE oidc_flows SET status=2 WHERE id=?1",
                [request.request_id],
            )?;
        }
        tx.commit()?;
        Ok(Progress::Ready {
            reauthorize,
            expires_at: expires,
        })
    }
    pub fn oidc_bindings(&self, credential: &str, now: u64) -> Result<Vec<String>, StoreError> {
        let account = self.session(credential, now)?.account_id;
        Ok(self
            .0
            .prepare("SELECT issuer FROM oidc_bindings WHERE account=?1 ORDER BY issuer LIMIT 17")?
            .query_map([account], |r| r.get(0))?
            .collect::<Result<_, _>>()?)
    }
    pub fn unlink_oidc(
        &mut self,
        credential: &str,
        issuer: &str,
        now: u64,
    ) -> Result<(), StoreError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let device = crate::prekeys::authorize(&tx, credential, now)?;
        let account: String = tx.query_row(
            "SELECT account_id FROM devices WHERE id=?1",
            [device],
            |r| r.get(0),
        )?;
        tx.execute("DELETE FROM oidc_grants WHERE issuer=?1 AND subject IN (SELECT subject FROM oidc_bindings WHERE issuer=?1 AND account=?2)",(issuer,&account))?;
        tx.execute("DELETE FROM oidc_flows WHERE json_extract(value,'$.link_device') IN (SELECT id FROM devices WHERE account_id=?1) OR (subject IN (SELECT subject FROM oidc_bindings WHERE issuer=?2 AND account=?1) AND ?2=(SELECT json_extract(value,'$.provider.issuer') FROM oidc_configuration WHERE id=1))",(&account,issuer))?;
        tx.execute(
            "DELETE FROM oidc_bindings WHERE issuer=?1 AND account=?2",
            (issuer, account),
        )?;
        tx.commit()?;
        Ok(())
    }
}
pub(crate) fn bind(
    db: &Connection,
    issuer: &str,
    subject: &str,
    account: &str,
) -> Result<(), StoreError> {
    let old: Option<String> = db
        .query_row(
            "SELECT account FROM oidc_bindings WHERE issuer=?1 AND subject=?2",
            (issuer, subject),
            |r| r.get(0),
        )
        .optional()?;
    if let Some(old) = old {
        return if old == account {
            Ok(())
        } else {
            Err(StoreError::Conflict)
        };
    }
    let occupied: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM oidc_bindings WHERE issuer=?1 AND account=?2)",
        (issuer, account),
        |r| r.get(0),
    )?;
    if occupied {
        return Err(StoreError::Conflict);
    }
    if db.query_row(
        "SELECT count(*) FROM oidc_bindings WHERE account=?1",
        [account],
        |r| r.get::<_, u32>(0),
    )? >= 16
    {
        return Err(StoreError::Busy);
    }
    db.execute(
        "INSERT INTO oidc_bindings VALUES(?1,?2,?3)",
        (issuer, subject, account),
    )?;
    Ok(())
}
pub(crate) struct Callback {
    id: String,
    revision: u64,
    stored: Stored,
    flow: Flow,
}
pub(crate) struct Completion {
    pub request_id: String,
    pub secret: String,
}
impl Callback {
    pub(crate) fn issuer(&self) -> &str {
        &self.stored.provider.issuer
    }
    #[cfg(test)]
    pub(crate) fn verify(&self, code: &str) -> Result<String, StoreError> {
        self.verify_profile(code).map(|v| v.subject)
    }
    pub(crate) fn id(&self) -> &str {
        &self.id
    }
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }
    pub(crate) fn verify_profile(
        &self,
        code: &str,
    ) -> Result<crate::web_admin::Identity, StoreError> {
        if code.is_empty() || code.len() > 4096 || code.chars().any(char::is_control) {
            return Err(StoreError::Unauthorized);
        }
        let provider = &self.stored.provider;
        let metadata = metadata(provider)?;
        if metadata.authorization_endpoint() != self.stored.metadata.authorization_endpoint()
            || metadata.token_endpoint() != self.stored.metadata.token_endpoint()
            || metadata.jwks_uri() != self.stored.metadata.jwks_uri()
        {
            return Err(StoreError::Conflict);
        }
        let auth_type = authentication(provider, &metadata)?;
        let client = CoreClient::from_provider_metadata(
            metadata,
            ClientId::new(provider.client_id.clone()),
            provider
                .client_secret
                .as_ref()
                .map(|s| ClientSecret::new(s.to_string())),
        )
        .set_auth_type(auth_type)
        .set_redirect_uri(
            RedirectUrl::new(self.stored.redirect.clone()).map_err(|_| StoreError::InvalidData)?,
        );
        let response = client
            .exchange_code(AuthorizationCode::new(code.to_owned()))
            .map_err(|_| StoreError::InvalidData)?
            .set_pkce_verifier(PkceCodeVerifier::new(self.flow.verifier.to_string()))
            .request(&|r| http(provider, r))
            .map_err(|_| StoreError::Unauthorized)?;
        let token = response.id_token().ok_or(StoreError::Unauthorized)?;
        let verifier = client.id_token_verifier().set_allowed_algs([
            CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
            CoreJwsSigningAlgorithm::EcdsaP256Sha256,
            CoreJwsSigningAlgorithm::EdDsa,
        ]);
        let claims = token
            .claims(&verifier, &Nonce::new(self.flow.nonce.to_string()))
            .map_err(|_| StoreError::Unauthorized)?;
        if let Some(expected) = claims.access_token_hash() {
            let actual = AccessTokenHash::from_token(
                response.access_token(),
                token.signing_alg().map_err(|_| StoreError::Unauthorized)?,
                token
                    .signing_key(&verifier)
                    .map_err(|_| StoreError::Unauthorized)?,
            )
            .map_err(|_| StoreError::Unauthorized)?;
            if *expected != actual {
                return Err(StoreError::Unauthorized);
            }
        }
        let subject = claims.subject().as_str();
        let now = crate::enrollment::now()?;
        let issued =
            u64::try_from(claims.issue_time().timestamp()).map_err(|_| StoreError::Unauthorized)?;
        if issued > now.saturating_add(60) || issued < now.saturating_sub(660) {
            return Err(StoreError::Unauthorized);
        }
        if subject.is_empty()
            || subject.len() > 255
            || !subject.is_ascii()
            || subject.chars().any(char::is_control)
        {
            return Err(StoreError::Unauthorized);
        }
        Ok(crate::web_admin::Identity {
            subject: subject.to_owned(),
            username: claims.preferred_username().map(|v| v.as_str().to_owned()),
            picture: claims
                .picture()
                .and_then(|v| v.get(None))
                .map(|v| v.as_str().to_owned()),
        })
    }
}

#[cfg(test)]
#[path = "oidc_tests.rs"]
mod tests;
pub(crate) fn check(update: &Configure) -> Result<Option<CoreProviderMetadata>, StoreError> {
    if !update.confirm {
        return Err(StoreError::Invalid("confirm the authentication change"));
    }
    update
        .provider
        .as_ref()
        .map(|p| {
            egress::endpoint(&p.issuer).map_err(|_| StoreError::Invalid("invalid HTTPS issuer"))?;
            if p.client_id.is_empty()
                || p.client_id.len() > 255
                || p.client_id.chars().any(char::is_control)
                || p.client_secret
                    .as_ref()
                    .is_some_and(|s| s.is_empty() || s.len() > 4096)
            {
                return Err(StoreError::Invalid("invalid OIDC client"));
            }
            metadata(p)
        })
        .transpose()
}

impl Store {
    pub(crate) fn web_picture_request(
        &self,
    ) -> Result<Option<(String, Vec<egress::Exception>)>, StoreError> {
        let picture: Option<String> =
            self.0
                .query_row("SELECT picture FROM web_owner", [], |r| r.get(0))?;
        let Some(picture) = picture else {
            return Ok(None);
        };
        let (_, provider) = read(&self.0)?;
        let provider = provider.ok_or(StoreError::NotFound)?;
        Ok(Some((picture, provider.provider.exceptions)))
    }
}
