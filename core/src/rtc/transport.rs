//! LiveKit service discovery and SFU JWT (lk-jwt-service).
use anyhow::{anyhow, Context};
use matrix_sdk::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

pub struct OpenIdToken {
    pub access_token: String,
    pub expires_in: u64,
    pub matrix_server_name: String,
}

pub async fn openid_token(client: &Client) -> anyhow::Result<OpenIdToken> {
    use ruma::api::client::account::request_openid_token;
    let uid = client.user_id().ok_or_else(|| anyhow!("not logged in"))?.to_owned();
    let resp = client.send(request_openid_token::v3::Request::new(uid)).await.context("request_openid_token")?;
    Ok(OpenIdToken { access_token: resp.access_token, expires_in: resp.expires_in.as_secs(), matrix_server_name: resp.matrix_server_name.to_string() })
}

fn livekit_from_transports(v: &serde_json::Value) -> Option<String> {
    v.get("rtc_transports")?
        .as_array()?
        .iter()
        .find(|t| t.get("type").and_then(|x| x.as_str()) == Some("livekit"))?
        .get("livekit_service_url")?
        .as_str()
        .map(str::to_string)
}

/// LiveKit URL from a `.well-known`: MSC4143's `org.matrix.msc4143.rtc_foci`, or the stable name.
fn livekit_from_well_known(v: &serde_json::Value) -> Option<String> {
    for key in ["org.matrix.msc4143.rtc_foci", "m.rtc_foci"] {
        let Some(foci) = v.get(key).and_then(|f| f.as_array()) else { continue };
        if let Some(url) = foci
            .iter()
            .find(|f| f.get("type").and_then(|t| t.as_str()) == Some("livekit"))
            .and_then(|f| f.get("livekit_service_url"))
            .and_then(|s| s.as_str())
        {
            return Some(url.to_string())
        }
    }
    None
}

/// `/rtc/transports` (stable, unstable) → `.well-known` `org.matrix.msc4143.rtc_foci`.
pub async fn discover_service_url(client: &Client, http: &reqwest::Client) -> anyhow::Result<String> {
    let hs = client.homeserver().to_string();
    let hs = hs.trim_end_matches('/');
    let token = client.access_token().ok_or_else(|| anyhow!("not logged in"))?;
    for path in ["/_matrix/client/v1/rtc/transports", "/_matrix/client/unstable/org.matrix.msc4143/rtc/transports"] {
        let url = format!("{hs}{path}");
        if let Ok(resp) = http.get(&url).bearer_auth(&token).send().await {
            if resp.status().is_success() {
                if let Ok(v) = resp.json::<serde_json::Value>().await {
                    if let Some(u) = livekit_from_transports(&v) {
                        info!("rtc: livekit service from {path}: {u}");
                        return Ok(u);
                    }
                }
            } else {
                debug!("rtc: {path} → {}", resp.status());
            }
        }
    }
    let server_name = client.user_id().map(|u| u.server_name().to_string()).ok_or_else(|| anyhow!("no user id"))?;
    let wk = format!("https://{server_name}/.well-known/matrix/client");
    let v: serde_json::Value = http.get(&wk).send().await.context("well-known")?.json().await.context("well-known json")?;
    let url = livekit_from_well_known(&v)
        .ok_or_else(|| anyhow!("no livekit focus in {wk}"))?;
    info!("rtc: livekit service from well-known: {url}");
    Ok(url)
}

#[derive(Serialize)]
struct OpenIdObj<'a> { access_token: &'a str, expires_in: u64, matrix_server_name: &'a str, token_type: &'static str }
#[derive(Serialize)]
struct SfuGet<'a> { openid_token: OpenIdObj<'a>, device_id: &'a str, room: &'a str }
#[derive(Deserialize)]
struct SfuResp { jwt: String, url: String }

pub struct LkTransport { pub server_url: String, pub jwt: String }

pub async fn fetch_jwt(http: &reqwest::Client, service_url: &str, room_id: &str, openid: &OpenIdToken, device_id: &str) -> anyhow::Result<LkTransport> {
    let base = service_url.trim_end_matches('/');
    let body = SfuGet { openid_token: OpenIdObj { access_token: &openid.access_token, expires_in: openid.expires_in, matrix_server_name: &openid.matrix_server_name, token_type: "Bearer" }, device_id, room: room_id };
    let resp = http.post(format!("{base}/sfu/get")).json(&body).send().await.context("POST sfu/get")?;
    if !resp.status().is_success() {
        let st = resp.status();
        let txt = resp.text().await.unwrap_or_default();
        return Err(anyhow!("lk-jwt-service {st}: {txt}"));
    }
    let r: SfuResp = resp.json().await.context("sfu/get json")?;
    Ok(LkTransport { server_url: r.url, jwt: r.jwt })
}

/// LiveKit identity is the JWT `sub`.
pub fn jwt_sub(jwt: &str) -> Option<String> {
    use base64::Engine;
    let payload = jwt.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.get("sub")?.as_str().map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synapse answers 404 M_UNRECOGNIZED on the stable `/v1/rtc/transports`.
    #[test]
    fn reads_the_unstable_transports_response() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"rtc_transports":[{"type":"livekit","livekit_service_url":"https://rtc.example.com"}]}"#,
        )
        .unwrap();
        assert_eq!(livekit_from_transports(&v).as_deref(), Some("https://rtc.example.com"));
    }

    #[test]
    fn ignores_transports_that_are_not_livekit() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"rtc_transports":[{"type":"something-else","url":"https://nope"},
                                  {"type":"livekit","livekit_service_url":"https://yes"}]}"#,
        )
        .unwrap();
        assert_eq!(livekit_from_transports(&v).as_deref(), Some("https://yes"));
        let empty: serde_json::Value = serde_json::from_str(r#"{"rtc_transports":[]}"#).unwrap();
        assert!(livekit_from_transports(&empty).is_none());
    }

    #[test]
    fn reads_the_well_known_foci() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"m.homeserver":{"base_url":"https://matrix.example.com"},
                 "org.matrix.msc4143.rtc_foci":[{"type":"livekit","livekit_service_url":"https://rtc.example.com"}],
                 "m.tile_server":{"map_style_url":"https://maps.example.com/assets/style-light.json"}}"#,
        )
        .unwrap();
        assert_eq!(livekit_from_well_known(&v).as_deref(), Some("https://rtc.example.com"));
    }

    #[test]
    fn a_well_known_without_foci_yields_nothing_rather_than_a_wrong_url() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"m.homeserver":{"base_url":"https://matrix.example"}}"#,
        )
        .unwrap();
        assert!(livekit_from_well_known(&v).is_none());
    }
}
