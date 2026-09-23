//! WebAuthn passkey ceremonies with the PRF extension for account recovery.
use crate::{fail, get, passkey_codec as codec, rtc::invoke, set};
use js_sys::{Array, Object, Uint8Array};
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::JsFuture;
use web_sys::{CredentialCreationOptions, CredentialRequestOptions, PublicKeyCredential};
use zeroize::Zeroizing;

const TIMEOUT_MS: f64 = 120_000.0;

fn buffer(bytes: &[u8]) -> JsValue {
    Uint8Array::from(bytes).buffer().into()
}
fn record(entries: &[(&str, JsValue)]) -> Result<JsValue, JsValue> {
    let value: JsValue = Object::new().into();
    for (key, entry) in entries {
        set(&value, key, entry)?;
    }
    Ok(value)
}
fn descriptor(id: &[u8]) -> Result<JsValue, JsValue> {
    record(&[("type", "public-key".into()), ("id", buffer(id))])
}
fn credentials() -> Result<web_sys::CredentialsContainer, JsValue> {
    if !passkey_supported() {
        return Err(fail("This browser does not support passkeys."));
    }
    Ok(web_sys::window()
        .ok_or_else(|| fail("Missing window"))?
        .navigator()
        .credentials())
}
// Maps the browser's cancel and timeout DOMException to one readable message.
fn ceremony(error: JsValue) -> JsValue {
    match get(&error, "name")
        .ok()
        .and_then(|v| v.as_string())
        .as_deref()
    {
        Some("NotAllowedError" | "AbortError") => {
            fail("The passkey request was cancelled or timed out.")
        }
        Some("InvalidStateError") => fail("This passkey is already registered for your account."),
        Some("SecurityError") => fail("This page's address cannot use passkeys for this server."),
        _ => error,
    }
}
fn prf_results(credential: &PublicKeyCredential) -> Result<(bool, Option<Vec<u8>>), JsValue> {
    let prf = get(
        &invoke(credential, "getClientExtensionResults", &[])?,
        "prf",
    )?;
    if prf.is_undefined() || prf.is_null() {
        return Ok((false, None));
    }
    let enabled = get(&prf, "enabled")?.as_bool() == Some(true);
    let results = get(&prf, "results")?;
    let first = if results.is_undefined() || results.is_null() {
        JsValue::UNDEFINED
    } else {
        get(&results, "first")?
    };
    Ok((
        enabled,
        (!first.is_undefined() && !first.is_null()).then(|| Uint8Array::new(&first).to_vec()),
    ))
}
async fn assert(
    rp_id: &str,
    challenge: &[u8],
    allowed: &[(&str, &[u8], &[u8])],
    missing: &str,
) -> Result<(Vec<u8>, Zeroizing<Vec<u8>>), JsValue> {
    let allow = Array::new();
    let evaluations: JsValue = Object::new().into();
    for (key, id, salt) in allowed {
        allow.push(&descriptor(id)?);
        set(&evaluations, key, &record(&[("first", buffer(salt))])?)?;
    }
    let extensions = record(&[("prf", record(&[("evalByCredential", evaluations)])?)])?;
    let public_key = record(&[
        ("rpId", rp_id.into()),
        ("challenge", buffer(challenge)),
        ("allowCredentials", allow.into()),
        ("userVerification", "required".into()),
        ("timeout", TIMEOUT_MS.into()),
        ("extensions", extensions),
    ])?;
    let options = CredentialRequestOptions::new();
    set(&options, "publicKey", &public_key)?;
    let credential: PublicKeyCredential =
        JsFuture::from(credentials()?.get_with_options(&options)?)
            .await
            .map_err(ceremony)?
            .dyn_into()?;
    let (_, first) = prf_results(&credential)?;
    let prf = first.ok_or_else(|| fail(missing))?;
    Ok((
        Uint8Array::new(&credential.raw_id()).to_vec(),
        codec::prf_output(prf).map_err(fail)?,
    ))
}
#[wasm_bindgen]
pub fn passkey_supported() -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    get(&window.navigator(), "credentials").is_ok_and(|v| !v.is_undefined() && !v.is_null())
        && get(&window, "PublicKeyCredential").is_ok_and(|v| v.is_function())
}
#[wasm_bindgen]
pub async fn passkey_get(request_json: String) -> Result<String, JsValue> {
    let request = codec::get_request(&request_json).map_err(fail)?;
    let allowed: Vec<_> = request
        .credentials
        .iter()
        .map(|c| (c.key.as_str(), c.id.as_slice(), c.salt.as_slice()))
        .collect();
    let (credential, prf) = assert(
        &request.rp_id,
        &request.challenge,
        &allowed,
        "This passkey did not return its recovery key. Try another passkey, or use a recovery code.",
    )
    .await?;
    let known = request.credentials.iter().any(|c| c.id == credential);
    if !known {
        return Err(fail("That passkey is not registered for this account."));
    }
    Ok(codec::get_result(&credential, &prf).to_string())
}
#[wasm_bindgen]
pub async fn passkey_create(options_json: String) -> Result<String, JsValue> {
    let options = codec::create_options(&options_json).map_err(fail)?;
    let exclude = Array::new();
    for id in &options.exclude {
        exclude.push(&descriptor(id)?);
    }
    let parameters = Array::new();
    for alg in [-7, -257] {
        parameters.push(&record(&[
            ("type", "public-key".into()),
            ("alg", alg.into()),
        ])?);
    }
    let public_key = record(&[
        (
            "rp",
            record(&[
                ("id", options.rp_id.as_str().into()),
                ("name", options.rp_name.as_str().into()),
            ])?,
        ),
        (
            "user",
            record(&[
                ("id", buffer(&options.user_id)),
                ("name", options.user_name.as_str().into()),
                ("displayName", options.user_display.as_str().into()),
            ])?,
        ),
        ("challenge", buffer(&options.challenge)),
        ("pubKeyCredParams", parameters.into()),
        (
            "authenticatorSelection",
            record(&[
                ("residentKey", "required".into()),
                ("requireResidentKey", true.into()),
                ("userVerification", "required".into()),
            ])?,
        ),
        ("attestation", "none".into()),
        ("timeout", TIMEOUT_MS.into()),
        ("excludeCredentials", exclude.into()),
        (
            "extensions",
            record(&[(
                "prf",
                record(&[("eval", record(&[("first", buffer(&options.salt))])?)])?,
            )])?,
        ),
    ])?;
    let request = CredentialCreationOptions::new();
    set(&request, "publicKey", &public_key)?;
    let credential: PublicKeyCredential =
        JsFuture::from(credentials()?.create_with_options(&request)?)
            .await
            .map_err(ceremony)?
            .dyn_into()?;
    let id = Uint8Array::new(&credential.raw_id()).to_vec();
    let prf = match prf_results(&credential)? {
        (_, Some(first)) => codec::prf_output(first).map_err(fail)?,
        // Providers that only report support evaluate PRF on the first assertion.
        (true, None) => {
            let mut challenge = [0u8; 32];
            getrandom::fill(&mut challenge).map_err(|_| fail("Browser randomness unavailable"))?;
            let key = codec::encode(&id);
            let (asserted, prf) = assert(
                &options.rp_id,
                &challenge,
                &[(key.as_str(), id.as_slice(), options.salt.as_slice())],
                codec::UNSUPPORTED,
            )
            .await?;
            if asserted != id {
                return Err(fail("A different passkey answered. Try again."));
            }
            prf
        }
        (false, None) => return Err(fail(codec::UNSUPPORTED)),
    };
    Ok(codec::create_result(&id, &options.salt, &prf).to_string())
}
