#![forbid(unsafe_code)]
#![cfg(target_arch = "wasm32")]
use js_sys::{Int32Array, Reflect, SharedArrayBuffer};
use sigil_client::ClientStore;
const MAX_COMMAND: usize = 131072;
use std::{cell::RefCell, time::Duration};
use wasm_bindgen::{prelude::*, JsCast};
mod auth;
mod camera;
mod location;
mod call;
mod rtc;
mod display;
mod files;
mod host;
mod recording;
mod transport;
mod vault;
fn fail(message: &str) -> JsValue {
    js_sys::Error::new(message).into()
}
fn get(object: &JsValue, key: &str) -> Result<JsValue, JsValue> {
    Reflect::get(object, &key.into())
}
fn set(object: &JsValue, key: &str, value: &JsValue) -> Result<(), JsValue> {
    Reflect::set(object, &key.into(), value).map(|_| ())
}
thread_local! {static STORE:RefCell<Option<ClientStore>>=const {RefCell::new(None)};}
thread_local! {static ERASING:std::cell::Cell<bool>=const {std::cell::Cell::new(false)};}
struct BrowserOs;
impl rsqlite_vfs::OsCallback for BrowserOs {
    fn sleep(duration: Duration) {
        let array = Int32Array::new(&SharedArrayBuffer::new(4));
        let _ =
            js_sys::Atomics::wait_with_timeout(&array, 0, 0, duration.as_millis().min(5000) as f64);
    }
    fn random(bytes: &mut [u8]) {
        getrandom::fill(bytes).expect("browser randomness unavailable");
    }
    fn epoch_timestamp_in_ms() -> i64 {
        js_sys::Date::now() as i64
    }
}
#[wasm_bindgen]
pub async fn worker_start() -> Result<(), JsValue> {
    let global = js_sys::global().dyn_into::<web_sys::DedicatedWorkerGlobalScope>()?;
    if get(&global, "crossOriginIsolated")?.as_bool() != Some(true) {
        return Err(fail("Browser isolation headers are required"));
    }
    let options = sqlite_wasm_vfs::sahpool::OpfsSAHPoolCfgBuilder::new()
        .vfs_name("sigil-opfs")
        .directory("sigil-device-data")
        .initial_capacity(8)
        .build();
    let pool = sqlite_wasm_vfs::sahpool::install::<BrowserOs>(&options, false)
        .await
        .map_err(|_| fail("Browser storage is unavailable or another Sigil tab owns it"))?;
    if vault::removing().await? {
        pool.clear_all()
            .await
            .map_err(|_| fail("Could not finish removing local data"))?;
        vault::clear().await?;
    }
    let pool = std::rc::Rc::new(pool);
    let master = vault::unlock(
        pool.exists("/sigil/messages.db")
            .map_err(|_| fail("Cannot inspect browser storage"))?,
    )
    .await?;
    let key = sigil_crypto::storage::StorageKey::new(sigil_crypto::Secret32::from_bytes(
        master
            .as_slice()
            .try_into()
            .map_err(|_| fail("Invalid browser key"))?,
    ))
    .map_err(|_| fail("Cannot initialize encryption"))?;
    let store = ClientStore::open(std::path::Path::new("/sigil/messages.db"), key)
        .map_err(|_| fail("Cannot open encrypted browser storage"))?;
    sigil_client::browser_transport::install(transport::send);
    STORE.with(|slot| *slot.borrow_mut() = Some(store));
    let handler = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(
        move |event: web_sys::MessageEvent| {
            if call::receive(&event) || files::receive(&event) {
                return;
            }
            let Some(raw) = event.data().as_string() else {
                return;
            };
            if raw.len() > 2 * MAX_COMMAND + 128 {
                return;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
                return;
            };
            let Some(id) = value["id"].as_u64() else {
                return;
            };
            let Some(request) = value["request"].as_str() else {
                return;
            };
            if ERASING.with(std::cell::Cell::get) {
                let reply=serde_json::json!({"id":id,"result":"{\"ok\":false,\"error\":\"Signing out\"}"}).to_string();
                let global =
                    js_sys::global().unchecked_into::<web_sys::DedicatedWorkerGlobalScope>();
                let _ = global.post_message(&reply.into());
                return;
            }
            if matches!(
                serde_json::from_str::<serde_json::Value>(request)
                    .ok()
                    .as_ref()
                    .and_then(|v| v["command"].as_str()),
                Some("browser_sign_out" | "browser_erase")
            ) {
                let revoke = serde_json::from_str::<serde_json::Value>(request)
                    .is_ok_and(|v| v["command"] == "browser_sign_out");
                ERASING.with(|busy| busy.set(true));
                let pool = pool.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    let result = erase(&pool, revoke).await;
                    ERASING.with(|busy| busy.set(false));
                    let result = match result {
                        Ok(()) => serde_json::json!({"ok":true,"value":{"erased":true}}),
                        Err(error) => {
                            serde_json::json!({"ok":false,"error":error.as_string().unwrap_or_else(||"Could not finish signing out. Retry, or explicitly remove local data without server confirmation.".into())})
                        }
                    };
                    let reply =
                        serde_json::json!({"id":id,"result":result.to_string()}).to_string();
                    let global =
                        js_sys::global().unchecked_into::<web_sys::DedicatedWorkerGlobalScope>();
                    let _ = global.post_message(&reply.into());
                });
                return;
            }
            let result = STORE
                .with(|slot| {
                    slot.borrow_mut()
                        .as_mut()
                        .map(|store| store.mobile_command(request))
                })
                .unwrap_or_else(|| "{\"ok\":false,\"error\":\"Browser is locked\"}".into());
            let reply = serde_json::json!({"id":id,"result":result}).to_string();
            let global = js_sys::global().unchecked_into::<web_sys::DedicatedWorkerGlobalScope>();
            let _ = global.post_message(&reply.into());
        },
    );
    global.set_onmessage(Some(handler.as_ref().unchecked_ref()));
    handler.forget();
    global.post_message(&"{\"ready\":true}".into())?;
    Ok(())
}

async fn erase(
    pool: &sqlite_wasm_vfs::sahpool::OpfsSAHPoolUtil,
    revoke: bool,
) -> Result<(), JsValue> {
    if revoke {
        let result = STORE
            .with(|slot| {
                slot.borrow_mut()
                    .as_mut()
                    .map(|store| store.mobile_command("{\"command\":\"sign_out\"}"))
            })
            .ok_or_else(|| fail("Browser is locked"))?;
        let result: serde_json::Value =
            serde_json::from_str(&result).map_err(|_| fail("Invalid sign-out response"))?;
        if result["ok"] != true {
            return Err(result["error"]
                .as_str()
                .unwrap_or("Server revocation is unconfirmed")
                .into());
        }
    }
    vault::mark_removal().await?;
    call::clear();
    STORE.with(|slot| slot.borrow_mut().take());
    pool.clear_all()
        .await
        .map_err(|_| fail("Could not remove local messages"))?;
    vault::clear().await
}
