use crate::{fail, get, transport};
use futures_channel::oneshot;
use std::{cell::RefCell, collections::BTreeMap};
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::spawn_local;
use web_sys::{ErrorEvent, MessageEvent, Worker, WorkerOptions, WorkerType};

type Reply = oneshot::Sender<Result<JsValue, JsValue>>;
struct Host {
    worker: Worker,
    next: u64,
    pending: BTreeMap<u64, Reply>,
    ready: Option<Reply>,
    _messages: Closure<dyn FnMut(MessageEvent)>,
    _errors: Closure<dyn FnMut(ErrorEvent)>,
    _timeout: Closure<dyn FnMut()>,
    timeout_id: i32,
}
thread_local! {static HOST:RefCell<Option<Host>>=const {RefCell::new(None)};}
fn shutdown() {
    HOST.with(|slot| {
        if let Some(mut host) = slot.borrow_mut().take() {
            if let Some(window) = web_sys::window() {
                window.clear_timeout_with_handle(host.timeout_id);
            }
            host.worker.set_onmessage(None);
            host.worker.set_onerror(None);
            host.worker.terminate();
            if let Some(reply) = host.ready.take() {
                let _ = reply.send(Err(fail("Browser client could not start")));
            }
            for (_, reply) in host.pending {
                let _ = reply.send(Err(fail("Browser client stopped")));
            }
        }
    });
}
#[wasm_bindgen]
pub async fn start_browser() -> Result<(), JsValue> {
    if HOST.with(|slot| slot.borrow().is_some()) {
        return Err(fail("Browser client already started"));
    }
    let options = WorkerOptions::new();
    options.set_type(WorkerType::Module);
    let worker = Worker::new_with_options("/web/sigil-worker.mjs", &options)?;
    let (ready, receive) = oneshot::channel();
    let messages = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let data = event.data();
        if get(&data, "transport").ok().and_then(|v| v.as_bool()) == Some(true) {
            spawn_local(async move {
                let _ = transport::fetch(data).await;
            });
            return;
        }
        if get(&data, "binary").ok().and_then(|v| v.as_bool()) == Some(true) {
            let id = get(&data, "id")
                .ok()
                .and_then(|v| v.as_string())
                .and_then(|s| s.parse::<u64>().ok());
            HOST.with(|slot| {
                if let (Some(host), Some(id)) = (slot.borrow_mut().as_mut(), id) {
                    if let Some(reply) = host.pending.remove(&id) {
                        let result =
                            if get(&data, "ok").ok().and_then(|v| v.as_bool()) == Some(true) {
                                get(&data, "data")
                            } else {
                                Err(fail("Attachment operation failed"))
                            };
                        let _ = reply.send(result);
                    }
                }
            });
            return;
        }
        let Some(raw) = data.as_string() else { return };

        let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
            return;
        };
        HOST.with(|slot| {
            let mut slot = slot.borrow_mut();
            let Some(host) = slot.as_mut() else { return };
            if value["ready"] == true {
                if let Some(window) = web_sys::window() {
                    window.clear_timeout_with_handle(host.timeout_id);
                }
                if let Some(reply) = host.ready.take() {
                    let _ = reply.send(Ok(JsValue::NULL));
                }
                return;
            }
            if let (Some(id), Some(result)) = (value["id"].as_u64(), value["result"].as_str()) {
                if let Some(reply) = host.pending.remove(&id) {
                    let _ = reply.send(Ok(result.into()));
                }
            }
        });
    });
    let errors = Closure::<dyn FnMut(ErrorEvent)>::new(move |_| {
        spawn_local(async {
            shutdown();
        });
    });
    let timeout = Closure::<dyn FnMut()>::new(move || {
        spawn_local(async {
            shutdown();
        });
    });
    let timeout_id = web_sys::window()
        .ok_or_else(|| fail("Missing window"))?
        .set_timeout_with_callback_and_timeout_and_arguments_0(
            timeout.as_ref().unchecked_ref(),
            30_000,
        )?;
    worker.set_onmessage(Some(messages.as_ref().unchecked_ref()));
    worker.set_onerror(Some(errors.as_ref().unchecked_ref()));
    if let Err(error) = worker.post_message(&wasm_bindgen::module()) {
        worker.terminate();
        worker.set_onmessage(None);
        worker.set_onerror(None);
        if let Some(window) = web_sys::window() {
            window.clear_timeout_with_handle(timeout_id);
        }
        return Err(error);
    }
    HOST.with(|slot| {
        *slot.borrow_mut() = Some(Host {
            worker,
            next: 1,
            pending: BTreeMap::new(),
            ready: Some(ready),
            _messages: messages,
            _errors: errors,
            _timeout: timeout,
            timeout_id,
        })
    });
    receive
        .await
        .map_err(|_| fail("Browser initialization interrupted"))??;
    crate::auth::listen()?;
    lifecycle()?;
    Ok(())
}
#[wasm_bindgen]
pub async fn browser_command(request: String) -> Result<String, JsValue> {
    rpc(request, None)
        .await?
        .as_string()
        .ok_or_else(|| fail("Invalid command response"))
}
pub(crate) async fn rpc(
    request: String,
    bytes: Option<js_sys::Uint8Array>,
) -> Result<JsValue, JsValue> {
    if request.len() > crate::MAX_COMMAND {
        return Err(fail("Command too large"));
    }
    let (reply, receive) = oneshot::channel();
    HOST.with(|slot| -> Result<(), JsValue> {
        let mut slot = slot.borrow_mut();
        let host = slot.as_mut().ok_or_else(|| fail("Browser is not ready"))?;
        if host.ready.is_some() || host.pending.len() >= 32 {
            return Err(fail("Browser is busy"));
        }
        let id = host.next;
        host.next = host
            .next
            .checked_add(1)
            .ok_or_else(|| fail("Restart browser client"))?;
        if let Some(bytes) = bytes {
            let data = js_sys::Object::new();
            crate::set(&data, "binary", &true.into())?;
            crate::set(&data, "id", &id.to_string().into())?;
            crate::set(&data, "request", &request.into())?;
            crate::set(&data, "data", &bytes)?;
            host.worker.post_message(&data)?;
        } else {
            host.worker.post_message(
                &serde_json::json!({"id":id,"request":request})
                    .to_string()
                    .into(),
            )?;
        }
        host.pending.insert(id, reply);
        Ok(())
    })?;
    receive
        .await
        .map_err(|_| fail("Browser command interrupted"))?
}

#[wasm_bindgen]
pub fn request_id() -> Result<String, JsValue> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| fail("Randomness unavailable"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn lifecycle() -> Result<(), JsValue> {
    thread_local! {static INSTALLED:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
    if INSTALLED.with(|v| v.get()) {
        return Ok(());
    }
    let window = web_sys::window().ok_or_else(|| fail("Missing window"))?;
    let hide = Closure::<dyn FnMut(web_sys::Event)>::new(|_| {
        crate::camera::camera_stop();
        crate::recording::voice_cancel();
        crate::auth::stop();
        shutdown();
    });
    let show = Closure::<dyn FnMut(web_sys::Event)>::new(|event: web_sys::Event| {
        if get(&event, "persisted").ok().and_then(|v| v.as_bool()) == Some(true) {
            if let Some(window) = web_sys::window() {
                let _ = window.location().reload();
            }
        }
    });
    window.add_event_listener_with_callback("pagehide", hide.as_ref().unchecked_ref())?;
    hide.forget();
    window.add_event_listener_with_callback("pageshow", show.as_ref().unchecked_ref())?;
    show.forget();
    INSTALLED.with(|v| v.set(true));
    Ok(())
}
