//! Fixed-purpose provider signatures and Web Push framing. No messaging keys.
use crate::egress;
use base64ct::{Base64, Base64UrlUnpadded as B64, Encoding};
use ring::{
    rand::SystemRandom,
    signature::{self, KeyPair},
};
use serde::{Deserialize, Serialize};
use sigil_protocol::push::{Payload, Target};
use std::time::{Duration, UNIX_EPOCH};
use ureq::http::{header, HeaderValue, Request, Uri};
use web_push_native::{p256, Auth, WebPushBuilder};
use zeroize::Zeroizing;

const OAUTH: &str = "https://oauth2.googleapis.com/token";
const SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";
pub const MAX_TTL: u64 = 7 * 24 * 60 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidTarget,
    InvalidConfiguration,
    Crypto,
    InvalidResponse,
}

fn decode<const N: usize>(text: &str) -> Result<[u8; N], Error> {
    if text.len() != (N * 8).div_ceil(6) {
        return Err(Error::InvalidTarget);
    }
    let mut bytes = [0; N];
    if B64::decode(text, &mut bytes)
        .map_err(|_| Error::InvalidTarget)?
        .len()
        != N
    {
        return Err(Error::InvalidTarget);
    }
    Ok(bytes)
}
fn public(text: &str) -> Result<p256::PublicKey, Error> {
    let bytes = decode::<65>(text)?;
    if bytes[0] != 4 {
        return Err(Error::InvalidTarget);
    }
    p256::PublicKey::from_sec1_bytes(&bytes).map_err(|_| Error::InvalidTarget)
}
pub fn validate_target(target: &Target, vapid: &str) -> Result<(), Error> {
    match target {
        Target::Fcm { token } => {
            if !sigil_protocol::push::valid_token(token) {
                return Err(Error::InvalidTarget);
            }
        }
        Target::UnifiedPush {
            endpoint,
            public_key,
            auth_secret,
            vapid_key,
        } => {
            egress::endpoint(endpoint).map_err(|_| Error::InvalidTarget)?;
            public(public_key)?;
            decode::<16>(auth_secret)?;
            public(vapid_key)?;
            if vapid_key != vapid || public_key == vapid_key {
                return Err(Error::InvalidTarget);
            }
        }
    }
    Ok(())
}

pub fn validate_contact(contact: &str) -> Result<(), Error> {
    if contact.len() > 253 || !contact.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(Error::InvalidConfiguration);
    }
    if let Some(address) = contact.strip_prefix("mailto:") {
        let (local, host) = address.split_once('@').ok_or(Error::InvalidConfiguration)?;
        if local.is_empty()
            || !local
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
            || !sigil_protocol::valid_server_name(host)
        {
            return Err(Error::InvalidConfiguration);
        }
    } else {
        egress::endpoint(contact).map_err(|_| Error::InvalidConfiguration)?;
    }
    Ok(())
}

pub struct Vapid {
    key: signature::EcdsaKeyPair,
}
impl Vapid {
    pub fn generate() -> Result<Zeroizing<Vec<u8>>, Error> {
        let key = signature::EcdsaKeyPair::generate_pkcs8(
            &signature::ECDSA_P256_SHA256_FIXED_SIGNING,
            &SystemRandom::new(),
        )
        .map_err(|_| Error::Crypto)?;
        Ok(Zeroizing::new(key.as_ref().to_vec()))
    }
    pub fn from_pkcs8(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > 512 {
            return Err(Error::InvalidConfiguration);
        }
        Ok(Self {
            key: signature::EcdsaKeyPair::from_pkcs8(
                &signature::ECDSA_P256_SHA256_FIXED_SIGNING,
                bytes,
                &SystemRandom::new(),
            )
            .map_err(|_| Error::InvalidConfiguration)?,
        })
    }
    pub fn public_key(&self) -> String {
        B64::encode_string(self.key.public_key().as_ref())
    }
    fn authorization(&self, uri: &Uri, contact: &str, now: u64) -> Result<HeaderValue, Error> {
        validate_contact(contact)?;
        let authority = uri.authority().ok_or(Error::InvalidTarget)?;
        // RFC6454 origins omit the default port; preserve IPv6 brackets.
        let authority = authority
            .as_str()
            .strip_suffix(":443")
            .unwrap_or(authority.as_str());
        let claims = serde_json::json!({"aud": format!("https://{authority}"), "exp": expiry(now, 3600)?, "sub": contact});
        let mut jwt = signing_input("ES256", &claims)?;
        let signature = self
            .key
            .sign(&SystemRandom::new(), jwt.as_bytes())
            .map_err(|_| Error::Crypto)?;
        jwt.push('.');
        jwt.push_str(&B64::encode_string(signature.as_ref()));
        sensitive(&format!(
            "vapid t={}, k={}",
            jwt.as_str(),
            self.public_key()
        ))
    }
    pub fn request(
        &self,
        target: &Target,
        contact: &str,
        payload: &Payload<'_>,
        now: u64,
        ttl: u64,
    ) -> Result<Request<Zeroizing<Vec<u8>>>, Error> {
        if ttl == 0 || ttl > MAX_TTL {
            return Err(Error::InvalidConfiguration);
        }
        validate_target(target, &self.public_key())?;
        let Target::UnifiedPush {
            endpoint,
            public_key,
            auth_secret,
            ..
        } = target
        else {
            return Err(Error::InvalidTarget);
        };
        let uri = egress::endpoint(endpoint).map_err(|_| Error::InvalidTarget)?;
        let auth = Zeroizing::new(decode::<16>(auth_secret)?);
        let mut request = WebPushBuilder::new(uri.clone(), public(public_key)?, Auth::from(*auth))
            .with_valid_duration(Duration::from_secs(ttl))
            .build(payload.to_bytes())
            .map_err(|_| Error::Crypto)?;
        // RFC8291 requires rs strictly greater than this single encrypted record.
        // Upstream 0.5.0 emits equality. rs is framing, not AEAD associated data.
        request.body_mut()[16..20].copy_from_slice(&4096u32.to_be_bytes());
        request.headers_mut().insert(
            header::AUTHORIZATION,
            self.authorization(&uri, contact, now)?,
        );
        // Challenge replacement must not coalesce with an already queued wake.
        if matches!(payload, Payload::Wake) {
            request
                .headers_mut()
                .insert("topic", HeaderValue::from_static("sigil-wake-v0"));
        }
        Ok(request.map(Zeroizing::new))
    }
}

fn expiry(now: u64, seconds: u64) -> Result<u64, Error> {
    now.checked_add(seconds)
        .filter(|n| *n <= i64::MAX as u64)
        .ok_or(Error::InvalidConfiguration)
}
fn sensitive(value: &str) -> Result<HeaderValue, Error> {
    let mut header = HeaderValue::from_str(value).map_err(|_| Error::InvalidConfiguration)?;
    header.set_sensitive(true);
    Ok(header)
}
fn signing_input(algorithm: &str, claims: &serde_json::Value) -> Result<Zeroizing<String>, Error> {
    let header = serde_json::to_vec(&serde_json::json!({"alg": algorithm, "typ": "JWT"}))
        .map_err(|_| Error::InvalidConfiguration)?;
    let claims = serde_json::to_vec(claims).map_err(|_| Error::InvalidConfiguration)?;
    Ok(Zeroizing::new(format!(
        "{}.{}",
        B64::encode_string(&header),
        B64::encode_string(&claims)
    )))
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FcmCredentials {
    pub project_id: String,
    pub client_email: String,
    pub private_key: Zeroizing<String>,
}
pub struct Fcm {
    key: signature::RsaKeyPair,
    project: String,
    email: String,
}
impl Fcm {
    pub fn new(credentials: &FcmCredentials) -> Result<Self, Error> {
        let project = &credentials.project_id;
        if !(6..=30).contains(&project.len())
            || !project.as_bytes()[0].is_ascii_lowercase()
            || project.ends_with('-')
            || !project
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(Error::InvalidConfiguration);
        }
        let email = &credentials.client_email;
        let (local, host) = email.split_once('@').ok_or(Error::InvalidConfiguration)?;
        if email.len() > 253
            || local.is_empty()
            || !local
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            || !sigil_protocol::valid_server_name(host)
            || !host.ends_with(".gserviceaccount.com")
        {
            return Err(Error::InvalidConfiguration);
        }
        let pem = &credentials.private_key;
        if pem.len() > 8192 {
            return Err(Error::InvalidConfiguration);
        }
        let contents = pem
            .trim()
            .strip_prefix("-----BEGIN PRIVATE KEY-----")
            .and_then(|s| s.strip_suffix("-----END PRIVATE KEY-----"))
            .ok_or(Error::InvalidConfiguration)?;
        let base64 = Zeroizing::new(
            contents
                .chars()
                .filter(|c| !c.is_ascii_whitespace())
                .collect::<String>(),
        );
        let der =
            Zeroizing::new(Base64::decode_vec(&base64).map_err(|_| Error::InvalidConfiguration)?);
        let key =
            signature::RsaKeyPair::from_pkcs8(&der).map_err(|_| Error::InvalidConfiguration)?;
        if !(256..=512).contains(&key.public().modulus_len()) {
            return Err(Error::InvalidConfiguration);
        }
        Ok(Self {
            key,
            project: project.clone(),
            email: email.clone(),
        })
    }
    pub fn token_request(&self, now: u64) -> Result<Request<Zeroizing<Vec<u8>>>, Error> {
        let claims = serde_json::json!({"iss": self.email, "scope": SCOPE, "aud": OAUTH, "iat": now, "exp": expiry(now, 3600)?});
        let mut jwt = signing_input("RS256", &claims)?;
        let mut signature = Zeroizing::new(vec![0; self.key.public().modulus_len()]);
        self.key
            .sign(
                &signature::RSA_PKCS1_SHA256,
                &SystemRandom::new(),
                jwt.as_bytes(),
                &mut signature,
            )
            .map_err(|_| Error::Crypto)?;
        jwt.push('.');
        jwt.push_str(&B64::encode_string(&signature));
        let body = Zeroizing::new(
            format!(
                "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer&assertion={}",
                jwt.as_str()
            )
            .into_bytes(),
        );
        Request::post(OAUTH)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(body)
            .map_err(|_| Error::InvalidConfiguration)
    }
    pub fn request(
        &self,
        token: &AccessToken,
        target: &Target,
        payload: &Payload<'_>,
        now: u64,
        ttl: u64,
    ) -> Result<Request<Zeroizing<Vec<u8>>>, Error> {
        if ttl == 0 || ttl > MAX_TTL || !token.valid_at(now) {
            return Err(Error::InvalidConfiguration);
        }
        validate_target(target, "")?;
        let Target::Fcm {
            token: registration,
        } = target
        else {
            return Err(Error::InvalidTarget);
        };
        #[derive(Serialize)]
        struct Android<'a> {
            priority: &'a str,
            ttl: String,
            collapse_key: Option<&'a str>,
        }
        #[derive(Serialize)]
        struct Data {
            sigil: Zeroizing<String>,
        }
        #[derive(Serialize)]
        struct Message<'a> {
            token: &'a str,
            data: Data,
            android: Android<'a>,
        }
        #[derive(Serialize)]
        struct Body<'a> {
            message: Message<'a>,
        }
        let body = Body {
            message: Message {
                token: registration,
                data: Data {
                    sigil: Zeroizing::new(B64::encode_string(&payload.to_bytes())),
                },
                // Silent sync can include receipts/key traffic; it cannot promise a visible notification.
                android: Android {
                    priority: "normal",
                    ttl: format!("{ttl}s"),
                    collapse_key: matches!(payload, Payload::Wake).then_some("sigil-wake-v0"),
                },
            },
        };
        let body =
            Zeroizing::new(serde_json::to_vec(&body).map_err(|_| Error::InvalidConfiguration)?);
        let authorization = Zeroizing::new(format!("Bearer {}", token.value.as_str()));
        Request::post(format!(
            "https://fcm.googleapis.com/v1/projects/{}/messages:send",
            self.project
        ))
        .header(header::AUTHORIZATION, sensitive(&authorization)?)
        .header(header::CONTENT_TYPE, "application/json")
        .body(body)
        .map_err(|_| Error::InvalidConfiguration)
    }
}
pub struct AccessToken {
    value: Zeroizing<String>,
    issued_at: u64,
    expires_at: u64,
}
impl AccessToken {
    /// Bound expiry to request start, not response arrival, so transport delay
    /// cannot extend the provider's advertised lifetime.
    pub fn from_response(response: &egress::Response, started_at: u64) -> Result<Self, Error> {
        if response.status != 200 || !json(response) {
            return Err(Error::InvalidResponse);
        }
        #[derive(Deserialize)]
        struct Token {
            access_token: Zeroizing<String>,
            token_type: String,
            expires_in: u64,
        }
        let token: Token =
            serde_json::from_slice(&response.body).map_err(|_| Error::InvalidResponse)?;
        if token.token_type != "Bearer"
            || !(61..=3600).contains(&token.expires_in)
            || token.access_token.is_empty()
            || token.access_token.len() > 4096
            || !token
                .access_token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-._~+/=".contains(&b))
        {
            return Err(Error::InvalidResponse);
        }
        Ok(Self {
            value: token.access_token,
            issued_at: started_at,
            expires_at: expiry(started_at, token.expires_in - 60)?,
        })
    }
    pub fn valid_at(&self, now: u64) -> bool {
        now >= self.issued_at && now < self.expires_at
    }
}
fn json(response: &egress::Response) -> bool {
    response.content_type.as_deref().is_some_and(|s| {
        s.split(';')
            .next()
            .is_some_and(|m| m.trim().eq_ignore_ascii_case("application/json"))
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Accepted,
    InvalidRegistration,
    Retry { not_before: u64, refresh_auth: bool },
}
pub fn classify(response: &egress::Response, fcm: bool, now: u64) -> Outcome {
    if (fcm
        && response.status == 200
        && json(response)
        && serde_json::from_slice::<serde_json::Value>(&response.body)
            .ok()
            .is_some_and(|v| {
                v.get("name")
                    .and_then(|n| n.as_str())
                    .is_some_and(|n| !n.is_empty() && n.len() <= 1024)
            }))
        || (!fcm && matches!(response.status, 201 | 202))
    {
        return Outcome::Accepted;
    }
    if !fcm && matches!(response.status, 404 | 410) {
        return Outcome::InvalidRegistration;
    }
    if fcm && matches!(response.status, 403 | 404) && json(response) {
        #[derive(Deserialize)]
        struct Body {
            error: Failure,
        }
        #[derive(Deserialize)]
        struct Failure {
            details: Vec<Detail>,
        }
        #[derive(Deserialize)]
        struct Detail {
            #[serde(rename = "@type")]
            kind: String,
            #[serde(rename = "errorCode")]
            code: Option<String>,
        }
        if let Ok(error) = serde_json::from_slice::<Body>(&response.body) {
            if error.error.details.iter().any(|d| {
                d.kind == "type.googleapis.com/google.firebase.fcm.v1.FcmError"
                    && matches!(
                        (response.status, d.code.as_deref()),
                        (404, Some("UNREGISTERED")) | (403, Some("SENDER_ID_MISMATCH"))
                    )
            }) {
                return Outcome::InvalidRegistration;
            }
        }
    }
    let minimum = if response.status == 429 { 60 } else { 5 };
    let delay = now.saturating_add(minimum);
    Outcome::Retry {
        not_before: delay.max(retry_after(response.retry_after.as_deref(), now).unwrap_or(0)),
        refresh_auth: fcm && response.status == 401,
    }
}
pub(crate) fn retry_after(value: Option<&str>, now: u64) -> Option<u64> {
    let text = value?;
    if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) {
        // An overflowing delay must never turn into an immediate retry.
        return Some(now.saturating_add(text.parse::<u64>().unwrap_or(u64::MAX)));
    }
    httpdate::parse_http_date(text)
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

#[cfg(test)]
#[path = "push_provider_tests.rs"]
mod tests;
