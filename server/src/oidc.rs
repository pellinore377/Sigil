//! The OIDC gate (design B24): with `registration = "oidc"`, `name.register`
//! carries an ID token from the operator's identity provider (Pocket ID,
//! Authentik, Keycloak, …) in the bag's `gate` position instead of an
//! invite code. The server checks the token against the issuer's published
//! keys and remembers only which login (`sub`) holds which name. Nothing
//! about conversations ever meets the identity provider.

use serde::Deserialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const JWKS_TTL: Duration = Duration::from_secs(600);

pub struct Oidc {
    pub issuer: String,
    pub client_id: String,
    http: reqwest::Client,
    jwks: Mutex<Option<(Instant, jsonwebtoken::jwk::JwkSet)>>,
}

#[derive(Debug, Deserialize)]
struct Discovery {
    jwks_uri: String,
}

/// What we keep from a verified token: who. The name they go by at the
/// provider is the app's business (a suggestion), not the server's.
#[derive(Debug, Clone, Deserialize)]
pub struct Claims {
    pub sub: String,
}

impl Oidc {
    pub fn new(issuer: &str, client_id: &str) -> Oidc {
        Oidc {
            issuer: issuer.trim_end_matches('/').to_string(),
            client_id: client_id.to_string(),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .expect("http client"),
            jwks: Mutex::new(None),
        }
    }

    pub fn info_json(&self) -> String {
        serde_json::json!({"issuer": self.issuer, "client_id": self.client_id}).to_string()
    }

    async fn fetch_jwks(&self) -> anyhow::Result<jsonwebtoken::jwk::JwkSet> {
        let disc: Discovery = self
            .http
            .get(format!("{}/.well-known/openid-configuration", self.issuer))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let set: jsonwebtoken::jwk::JwkSet = self
            .http
            .get(&disc.jwks_uri)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(set)
    }

    /// The issuer's keys, fetched at most every ten minutes, or again at
    /// once when `kid` is not among them (a rotated key).
    async fn keys(&self, kid: Option<&str>) -> anyhow::Result<jsonwebtoken::jwk::JwkSet> {
        let cached = {
            let g = self.jwks.lock().unwrap();
            g.as_ref()
                .filter(|(at, _)| at.elapsed() < JWKS_TTL)
                .map(|(_, s)| s.clone())
        };
        if let Some(set) = cached {
            let known = match kid {
                Some(k) => set.find(k).is_some(),
                None => true,
            };
            if known {
                return Ok(set);
            }
        }
        let set = self.fetch_jwks().await?;
        *self.jwks.lock().unwrap() = Some((Instant::now(), set.clone()));
        Ok(set)
    }

    /// Verify one ID token: signature against the issuer's keys, issuer,
    /// audience (our client id) and expiry.
    pub async fn verify(&self, token: &str) -> anyhow::Result<Claims> {
        let header = jsonwebtoken::decode_header(token)?;
        let set = self.keys(header.kid.as_deref()).await?;
        let jwk = match &header.kid {
            Some(k) => set.find(k),
            None => set.keys.first(),
        }
        .ok_or_else(|| anyhow::anyhow!("no key for this token"))?;
        let key = jsonwebtoken::DecodingKey::from_jwk(jwk)?;
        let mut v = jsonwebtoken::Validation::new(header.alg);
        v.set_issuer(&[&self.issuer]);
        v.set_audience(&[&self.client_id]);
        v.validate_exp = true;
        v.leeway = 60;
        let data = jsonwebtoken::decode::<Claims>(token, &key, &v)?;
        Ok(data.claims)
    }
}
