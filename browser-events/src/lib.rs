#![forbid(unsafe_code)]
#![cfg(target_arch = "wasm32")]
use base64ct::{Base64UrlUnpadded as B64, Encoding};
use js_sys::{Array, Reflect, Uint8Array};
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::JsFuture;
use web_sys::*;
mod storage;
fn fail(message: &str) -> JsValue {
    js_sys::Error::new(message).into()
}
fn window() -> Result<Window, JsValue> {
    web_sys::window().ok_or_else(|| fail("Missing window"))
}
async fn registration() -> Result<ServiceWorkerRegistration, JsValue> {
    let container = window()?.navigator().service_worker();
    let options = RegistrationOptions::new();
    options.set_type("module");
    options.set_scope("/web/");
    JsFuture::from(container.register_with_options("/web/sigil-notifications.mjs", &options))
        .await?;
    // This worker owns push only; it never intercepts page requests or holds messaging keys.
    JsFuture::from(container.get_registration_with_document_url("/web/"))
        .await?
        .dyn_into()
}
async fn active_registration() -> Result<ServiceWorkerRegistration, JsValue> {
    let registration = registration().await?;
    for _ in 0..100 {
        if registration.active().is_some() {
            return Ok(registration);
        }
        let (send, receive) = futures_channel::oneshot::channel();
        let callback = Closure::once(move || {
            let _ = send.send(());
        });
        window()?.set_timeout_with_callback_and_timeout_and_arguments_0(
            callback.as_ref().unchecked_ref(),
            100,
        )?;
        receive
            .await
            .map_err(|_| fail("Notification setup interrupted"))?;
    }
    Err(fail("Notification worker did not activate"))
}
#[wasm_bindgen]
pub fn notifications_supported() -> bool {
    ["Notification", "PushManager", "ServiceWorker"]
        .iter()
        .all(|key| Reflect::has(&js_sys::global(), &(*key).into()).unwrap_or(false))
}
#[wasm_bindgen]
pub fn notifications_permission() -> String {
    if !notifications_supported() {
        return "unsupported".into();
    }
    match Notification::permission() {
        NotificationPermission::Granted => "granted",
        NotificationPermission::Denied => "denied",
        _ => "default",
    }
    .into()
}
#[wasm_bindgen]
pub fn notifications_request() -> Result<js_sys::Promise, JsValue> {
    Notification::request_permission()
}
#[wasm_bindgen]
pub async fn notifications_subscribe(vapid: String) -> Result<String, JsValue> {
    if vapid.len() != 87 {
        return Err(fail("Invalid notification key"));
    }
    let key = B64::decode_vec(&vapid).map_err(|_| fail("Invalid notification key"))?;
    if key.len() != 65
        || key[0] != 4
        || Notification::permission() != NotificationPermission::Granted
    {
        return Err(fail("Allow notifications first"));
    }
    let registration = active_registration().await?;
    let manager = registration.push_manager()?;
    let existing = JsFuture::from(manager.get_subscription()?).await?;
    let subscription = if existing.is_null() || existing.is_undefined() {
        None
    } else {
        Some(existing.dyn_into::<PushSubscription>()?)
    };
    let subscription = match subscription {
        Some(old)
            if old
                .options()
                .application_server_key()
                .ok()
                .flatten()
                .is_some_and(|value| Uint8Array::new(&value).to_vec() == key) =>
        {
            old
        }
        old => {
            storage::set("endpoint", &JsValue::NULL).await?;
            if let Some(old) = old {
                if JsFuture::from(old.unsubscribe()?).await?.as_bool() != Some(true) {
                    return Err(fail("Could not replace notification subscription"));
                }
            }
            let options = PushSubscriptionOptionsInit::new();
            options.set_user_visible_only(true);
            options.set_application_server_key(&Uint8Array::from(key.as_slice()).into());
            JsFuture::from(manager.subscribe_with_options(&options)?)
                .await?
                .dyn_into::<PushSubscription>()?
        }
    };
    let endpoint = subscription.endpoint();
    if endpoint.len() > sigil_protocol::push::MAX_ENDPOINT {
        return Err(fail("Invalid notification endpoint"));
    }
    let key = |kind| -> Result<String, JsValue> {
        Ok(B64::encode_string(
            &Uint8Array::new(
                &subscription
                    .get_key(kind)?
                    .ok_or_else(|| fail("Missing push key"))?
                    .into(),
            )
            .to_vec(),
        ))
    };
    let target = serde_json::json!({"provider":"unified_push","endpoint":endpoint,"public_key":key(PushEncryptionKeyName::P256dh)?,"auth_secret":key(PushEncryptionKeyName::Auth)?,"vapid_key":vapid});
    storage::set("endpoint", &endpoint.into()).await?;
    Ok(target.to_string())
}
#[wasm_bindgen]
pub async fn notifications_pending() -> Result<String, JsValue> {
    let value = storage::get("challenge")
        .await?
        .as_string()
        .unwrap_or_default();
    if value.is_empty() {
        return Ok(value);
    }
    let valid = value.len() <= 2048
        && serde_json::from_str::<serde_json::Value>(&value)
            .ok()
            .is_some_and(|v| {
                v["received_at"].as_f64().is_some_and(|at| {
                    at <= js_sys::Date::now() && js_sys::Date::now() - at < 600_000.0
                })
            });
    if valid {
        Ok(value)
    } else {
        storage::compare_clear("challenge", &value).await?;
        Ok(String::new())
    }
}
#[wasm_bindgen]
pub async fn notifications_clear(payload: String) -> Result<(), JsValue> {
    storage::compare_clear("challenge", &payload).await
}
#[wasm_bindgen]
pub async fn notifications_enabled() -> Result<bool, JsValue> {
    Ok(storage::get("endpoint").await?.as_string().is_some())
}
#[wasm_bindgen]
pub async fn notifications_disable() -> Result<(), JsValue> {
    storage::set("endpoint", &JsValue::NULL).await?;
    storage::set("challenge", &JsValue::NULL).await?;
    let registration = JsFuture::from(
        window()?
            .navigator()
            .service_worker()
            .get_registration_with_document_url("/web/"),
    )
    .await?;
    if registration.is_undefined() || registration.is_null() {
        return Ok(());
    }
    let registration = registration.dyn_into::<ServiceWorkerRegistration>()?;
    let subscription = JsFuture::from(registration.push_manager()?.get_subscription()?).await?;
    if !subscription.is_null() && !subscription.is_undefined() {
        JsFuture::from(subscription.dyn_into::<PushSubscription>()?.unsubscribe()?).await?;
    }
    for value in Array::from(&JsFuture::from(registration.get_notifications()?).await?).iter() {
        value.dyn_into::<Notification>()?.close();
    }
    Ok(())
}
#[wasm_bindgen]
pub async fn notification_push(event: PushEvent) -> Result<(), JsValue> {
    let bytes = Uint8Array::new(
        &event
            .data()
            .ok_or_else(|| fail("Missing push payload"))?
            .array_buffer()?
            .into(),
    );
    if bytes.length() > 73 {
        return Err(fail("Invalid push size"));
    }
    let bytes = bytes.to_vec();
    let payload = sigil_protocol::push::Payload::from_bytes(&bytes).map_err(fail)?;
    let Some(endpoint) = storage::get("endpoint").await?.as_string() else {
        return Ok(());
    };
    let worker = js_sys::global().dyn_into::<ServiceWorkerGlobalScope>()?;
    let current = JsFuture::from(worker.registration().push_manager()?.get_subscription()?).await?;
    if current.is_null()
        || current.is_undefined()
        || current.dyn_into::<PushSubscription>()?.endpoint() != endpoint
    {
        return Ok(());
    }
    let challenge = matches!(payload, sigil_protocol::push::Payload::Challenge { .. });
    if challenge {
        let value = serde_json::json!({"endpoint":endpoint,"payload":B64::encode_string(&bytes),"received_at":js_sys::Date::now()})
            .to_string();
        storage::challenge(&endpoint, &value).await?;
    }
    if storage::get("endpoint").await?.as_string().as_deref() != Some(&endpoint) {
        return Ok(());
    }
    let options = NotificationOptions::new();
    options.set_tag("sigil-update");
    options.set_body(if challenge {
        "Open Sigil to finish setting up notifications."
    } else {
        "Open Sigil to check for updates."
    });
    JsFuture::from(
        worker
            .registration()
            .show_notification_with_options("Sigil", &options)?,
    )
    .await?;
    if storage::get("endpoint").await?.as_string().as_deref() != Some(&endpoint) {
        for value in
            Array::from(&JsFuture::from(worker.registration().get_notifications()?).await?).iter()
        {
            value.dyn_into::<Notification>()?.close();
        }
    }
    Ok(())
}
#[wasm_bindgen]
pub async fn notification_open(event: NotificationEvent) -> Result<(), JsValue> {
    event.notification().close();
    let worker = js_sys::global().dyn_into::<ServiceWorkerGlobalScope>()?;
    let origin = worker.location().origin();
    let options = ClientQueryOptions::new();
    options.set_include_uncontrolled(true);
    options.set_type(ClientType::Window);
    for item in
        Array::from(&JsFuture::from(worker.clients().match_all_with_options(&options)).await?)
            .iter()
    {
        let client = item.dyn_into::<WindowClient>()?;
        if client.url() == format!("{origin}/messenger") || client.url() == format!("{origin}/") {
            JsFuture::from(client.focus()?).await?;
            return Ok(());
        }
    }
    JsFuture::from(worker.clients().open_window("/messenger")).await?;
    Ok(())
}
