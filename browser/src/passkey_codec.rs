//! Pure passkey JSON and base64url codec for the WebAuthn bridge.
use base64ct::{Base64UrlUnpadded, Encoding};
use serde::Deserialize;
use zeroize::Zeroizing;

pub const PRF_LENGTH: usize = 32;
pub const UNSUPPORTED: &str = "This passkey provider can't protect Sigil recovery. Try another provider, or use a recovery code in Settings.";

pub fn encode(bytes: &[u8]) -> String {
    Base64UrlUnpadded::encode_string(bytes)
}
pub fn decode(text: &str) -> Result<Vec<u8>, &'static str> {
    Base64UrlUnpadded::decode_vec(text).map_err(|_| "Invalid passkey data from the server")
}
fn filled(text: &str) -> Result<Vec<u8>, &'static str> {
    Some(decode(text)?)
        .filter(|v| !v.is_empty())
        .ok_or("Invalid passkey data from the server")
}
#[derive(Deserialize)]
struct RawCredential {
    id: String,
    salt: String,
}
#[derive(Deserialize)]
struct RawRequest {
    rp_id: String,
    challenge: String,
    credentials: Vec<RawCredential>,
}
pub struct Allowed {
    pub key: String,
    pub id: Vec<u8>,
    pub salt: Vec<u8>,
}
pub struct GetRequest {
    pub rp_id: String,
    pub challenge: Vec<u8>,
    pub credentials: Vec<Allowed>,
}
pub fn get_request(json: &str) -> Result<GetRequest, &'static str> {
    let raw: RawRequest =
        serde_json::from_str(json).map_err(|_| "Invalid passkey request from the server")?;
    if raw.rp_id.is_empty() {
        return Err("Invalid passkey request from the server");
    }
    if raw.credentials.is_empty() {
        return Err("This account has no recovery passkeys. Use a recovery code instead.");
    }
    let credentials = raw
        .credentials
        .iter()
        .map(|c| {
            let id = filled(&c.id)?;
            Ok(Allowed {
                key: encode(&id),
                id,
                salt: filled(&c.salt)?,
            })
        })
        .collect::<Result<_, &'static str>>()?;
    Ok(GetRequest {
        rp_id: raw.rp_id,
        challenge: filled(&raw.challenge)?,
        credentials,
    })
}
#[derive(Deserialize)]
struct RawCreate {
    rp_id: String,
    rp_name: String,
    user_id: String,
    user_name: String,
    user_display: String,
    challenge: String,
    salt: String,
    #[serde(default)]
    exclude: Vec<String>,
}
pub struct CreateOptions {
    pub rp_id: String,
    pub rp_name: String,
    pub user_id: Vec<u8>,
    pub user_name: String,
    pub user_display: String,
    pub challenge: Vec<u8>,
    pub salt: Vec<u8>,
    pub exclude: Vec<Vec<u8>>,
}
pub fn create_options(json: &str) -> Result<CreateOptions, &'static str> {
    let raw: RawCreate =
        serde_json::from_str(json).map_err(|_| "Invalid passkey options from the server")?;
    if raw.rp_id.is_empty() || raw.rp_name.is_empty() || raw.user_name.is_empty() {
        return Err("Invalid passkey options from the server");
    }
    Ok(CreateOptions {
        rp_id: raw.rp_id,
        rp_name: raw.rp_name,
        user_id: filled(&raw.user_id)?,
        user_display: if raw.user_display.is_empty() {
            raw.user_name.clone()
        } else {
            raw.user_display
        },
        user_name: raw.user_name,
        challenge: filled(&raw.challenge)?,
        salt: filled(&raw.salt)?,
        exclude: raw
            .exclude
            .iter()
            .map(|v| filled(v))
            .collect::<Result<_, _>>()?,
    })
}
pub fn prf_output(bytes: Vec<u8>) -> Result<Zeroizing<Vec<u8>>, &'static str> {
    let bytes = Zeroizing::new(bytes);
    if bytes.len() == PRF_LENGTH {
        Ok(bytes)
    } else {
        Err(UNSUPPORTED)
    }
}
pub fn get_result(credential: &[u8], prf: &[u8]) -> Zeroizing<String> {
    Zeroizing::new(
        serde_json::json!({"credential":encode(credential),"prf":encode(prf)}).to_string(),
    )
}
pub fn create_result(credential: &[u8], salt: &[u8], prf: &[u8]) -> Zeroizing<String> {
    Zeroizing::new(
        serde_json::json!({"credential":encode(credential),"salt":encode(salt),"prf":encode(prf)})
            .to_string(),
    )
}
