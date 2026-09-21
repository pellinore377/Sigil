use crate::{fail, get, transport};
use futures_channel::oneshot;
use std::{cell::RefCell, collections::BTreeMap};
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::spawn_local;
use web_sys::{ErrorEvent, MessageEvent, Worker, WorkerOptions, WorkerType};

type Reply = oneshot::Sender<Result<JsValue, JsValue>>;
struct Host {
    worker: Worker,
    media: Worker,
    gate: js_sys::Int32Array,
    next: u64,
    pending: BTreeMap<u64, Reply>,
    ready: Option<Reply>,
    _messages: Closure<dyn FnMut(MessageEvent)>,
    _errors: Closure<dyn FnMut(ErrorEvent)>,
    _timeout: Closure<dyn FnMut()>,
    timeout_id: i32,
}
thread_local! {static HOST:RefCell<Option<Host>>=const {RefCell::new(None)};}
thread_local! {static WATCH:RefCell<Option<web_sys::AbortController>>=const {RefCell::new(None)};}
fn shutdown() {
    crate::files::clear_media_cache();
    WATCH.with(|slot| {
        if let Some(controller) = slot.borrow_mut().take() {
            controller.abort();
        }
    });
    crate::rtc::browser_call_close();
    HOST.with(|slot| {
        if let Some(mut host) = slot.borrow_mut().take() {
            if let Some(window) = web_sys::window() {
                window.clear_timeout_with_handle(host.timeout_id);
            }
            host.worker.set_onmessage(None);
            host.worker.set_onerror(None);
            host.worker.terminate();
            host.media.set_onmessage(None);
            host.media.set_onerror(None);
            host.media.terminate();
            if let Some(reply) = host.ready.take() {
                let _ = reply.send(Err(fail("Browser client could not start")));
            }
            for (_, reply) in host.pending {
                let _ = reply.send(Err(fail("Browser client stopped")));
            }
        }
    });
}
/// The worker that holds the call keys; encoded transforms run inside it.
pub(crate) fn worker() -> Option<Worker> {
    HOST.with(|slot| slot.borrow().as_ref().map(|host| host.media.clone()))
}
#[wasm_bindgen]
pub async fn start_browser() -> Result<(), JsValue> {
    if HOST.with(|slot| slot.borrow().is_some()) {
        return Err(fail("Browser client already started"));
    }
    let options = WorkerOptions::new();
    options.set_type(WorkerType::Module);
    let channel = web_sys::MessageChannel::new()?;
    let worker = Worker::new_with_options("/web/sigil-worker.mjs", &options)?;
    let media = match Worker::new_with_options("/web/sigil-media-worker.mjs", &options) {
        Ok(media) => media,
        Err(error) => { worker.terminate(); return Err(error); }
    };
    let gate = js_sys::Int32Array::new(&js_sys::SharedArrayBuffer::new(12));
    let (ready, receive) = oneshot::channel();
    let messages = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let data = event.data();
        if let Some(timing) = get(&data, "media_timing").ok().and_then(|v| v.as_string()) {
            if timing.len() <= 512 && timing.starts_with("SigilTiming ") {
                web_sys::console::log_1(&timing.into());
            }
            return;
        }
        if get(&data,"video_frame").ok().and_then(|v|v.as_bool())==Some(true) {
            let frame=get(&data,"frame").unwrap_or(JsValue::UNDEFINED);
            let valid=HOST.with(|h|h.borrow().as_ref().is_some_and(|h|js_sys::Atomics::load(&h.gate,0).ok().map(f64::from)==get(&data,"revision").ok().and_then(|v|v.as_f64())));
            let valid=valid && get(&data,"until").ok().and_then(|v|v.as_f64()).is_some_and(|until|js_sys::Date::now()<until);
            let native=valid && crate::rtc::receive_decoded_video(&data).unwrap_or(false);
            let _=crate::rtc::invoke(&frame,"close",&[]);
            let _=crate::set(&data,"frame",&JsValue::UNDEFINED);let _=crate::set(&data,"video_ack",&true.into());let _=crate::set(&data,"native",&native.into());
            if let Some(worker)=crate::host::worker(){let _=worker.post_message(&data);}
            return;
        }
        if get(&data, "video_shape").ok().and_then(|v| v.as_bool()) == Some(true) {
            let _ = crate::rtc::receive_video(&data);
            return;
        }
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
                                let detail = get(&data, "error").ok().and_then(|v| v.as_string()).filter(|v| !v.is_empty());
                                Err(fail(&detail.map_or_else(|| "Media operation could not complete".to_owned(), |v| format!("Media operation could not complete: {v}"))))
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
    let timeout_id = match web_sys::window()
        .ok_or_else(|| fail("Missing window"))
        .and_then(|window| window.set_timeout_with_callback_and_timeout_and_arguments_0(
            timeout.as_ref().unchecked_ref(),
            30_000,
        )) {
            Ok(id) => id,
            Err(error) => { worker.terminate(); media.terminate(); return Err(error); }
        };
    media.set_onmessage(Some(messages.as_ref().unchecked_ref()));
    media.set_onerror(Some(errors.as_ref().unchecked_ref()));
    worker.set_onmessage(Some(messages.as_ref().unchecked_ref()));
    worker.set_onerror(Some(errors.as_ref().unchecked_ref()));
    HOST.with(|slot| {
        *slot.borrow_mut() = Some(Host {
            worker: worker.clone(),
            media: media.clone(),
            gate: gate.clone(),
            next: 1,
            pending: BTreeMap::new(),
            ready: Some(ready),
            _messages: messages,
            _errors: errors,
            _timeout: timeout,
            timeout_id,
        })
    });
    let initialized=(||->Result<(),JsValue>{
        let init=js_sys::Object::new();
        crate::set(&init,"module",&wasm_bindgen::module())?;
        crate::set(&init,"port",&channel.port2())?;
        crate::set(&init,"gate",&gate.buffer())?;
        media.post_message_with_transfer(&init,&js_sys::Array::of1(&channel.port2()))?;
        worker.post_message(&wasm_bindgen::module())
    })();
    if let Err(error)=initialized {shutdown();return Err(error);}
    receive
        .await
        .map_err(|_| fail("Browser initialization interrupted"))??;
    let linked=(||->Result<(),JsValue>{
        let link=js_sys::Object::new();
        crate::set(&link,"media_link",&true.into())?;
        crate::set(&link,"port",&channel.port1())?;
        crate::set(&link,"gate",&gate.buffer())?;
        worker.post_message_with_transfer(&link,&js_sys::Array::of1(&channel.port1()))?;
        crate::auth::listen()?;
        lifecycle()
    })();
    if let Err(error)=linked {shutdown();return Err(error);}
    Ok(())
}
#[wasm_bindgen]
pub async fn browser_command(request: String) -> Result<String, JsValue> {
    if serde_json::from_str::<serde_json::Value>(&request).ok().and_then(|v|v["command"].as_str().map(str::to_owned)).is_some_and(|command| matches!(command.as_str(),"browser_sign_out"|"browser_erase")) {
        crate::files::clear_media_cache();
    }
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
        if serde_json::from_str::<serde_json::Value>(&request).ok().is_some_and(|v|
            v["operation"] == "call_stop" || matches!(v["command"].as_str(), Some("browser_sign_out" | "browser_erase"))) {
            let _ = js_sys::Atomics::add(&host.gate, 0, 1);
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
            crate::set(&data, "request", &JsValue::from_str(&request))?;
            crate::set(&data, "data", &bytes)?;
            let operation=serde_json::from_str::<serde_json::Value>(&request).ok();
            let media=operation.as_ref().and_then(|v|v["operation"].as_str()).is_some_and(|v| matches!(v,"call_seal"|"call_open"));
            if media { host.media.post_message(&data)?; } else { host.worker.post_message(&data)?; }
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

/// Holds a mailbox wait on the main thread so the worker stays free; true when mail is waiting.
#[wasm_bindgen]
pub async fn mailbox_watch() -> Result<bool, JsValue> {
    let reply = rpc(r#"{"command":"watch_target"}"#.into(), None)
        .await?
        .as_string()
        .ok_or_else(|| fail("Invalid command response"))?;
    let reply: serde_json::Value =
        serde_json::from_str(&reply).map_err(|_| fail("Invalid command response"))?;
    if reply["ok"] != true {
        return Err(fail(reply["error"].as_str().unwrap_or("Not connected")));
    }
    let target = &reply["value"];
    let (Some(origin), Some(credential), Some(after)) = (
        target["origin"].as_str(),
        target["credential"].as_str(),
        target["after"].as_i64(),
    ) else {
        return Err(fail("Invalid watch target"));
    };
    let url = format!("{origin}/client/v0/mailbox/wait?after={after}&timeout=25");
    let options = web_sys::RequestInit::new();
    options.set_cache(web_sys::RequestCache::NoStore);
    options.set_method("GET");
    options.set_redirect(web_sys::RequestRedirect::Error);
    options.set_credentials(web_sys::RequestCredentials::Omit);
    let headers = web_sys::Headers::new()?;
    headers.set("X-Sigil-Client", "1")?;
    headers.set("Authorization", &format!("Bearer {credential}"))?;
    headers.set("Accept", "application/json")?;
    options.set_headers(&headers);
    let controller = web_sys::AbortController::new()?;
    options.set_signal(Some(&controller.signal()));
    WATCH.with(|slot| {
        if let Some(previous) = slot.borrow_mut().replace(controller.clone()) {
            previous.abort();
        }
    });
    let window = web_sys::window().ok_or_else(|| fail("Missing window"))?;
    let abort = controller.clone();
    let timeout = Closure::<dyn FnMut()>::new(move || abort.abort());
    let id = window.set_timeout_with_callback_and_timeout_and_arguments_0(
        timeout.as_ref().unchecked_ref(),
        35_000,
    )?;
    let result = wasm_bindgen_futures::JsFuture::from(window.fetch_with_str_and_init(&url, &options)).await;
    window.clear_timeout_with_handle(id);
    drop(timeout);
    WATCH.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.as_ref().is_some_and(|current| current == &controller) {
            *slot = None;
        }
    });
    let response = result?.dyn_into::<web_sys::Response>()?;
    if response.status() != 200 {
        return Err(fail(&format!("Mailbox wait failed (HTTP {})", response.status())));
    }
    let text = wasm_bindgen_futures::JsFuture::from(response.text()?)
        .await?
        .as_string()
        .ok_or_else(|| fail("Invalid mailbox wait response"))?;
    Ok(text.contains('{'))
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
    let document=window.document().ok_or_else(||fail("Missing document"))?;
    crate::files::media_cache_visibility(get(&document,"visibilityState")?.as_string().as_deref()==Some("visible"));
    let source=document.clone();
    let visibility=Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        crate::files::media_cache_visibility(get(&source,"visibilityState").ok().and_then(|v|v.as_string()).as_deref()==Some("visible"));
    });
    document.add_event_listener_with_callback("visibilitychange",visibility.as_ref().unchecked_ref())?;
    visibility.forget();
    window.add_event_listener_with_callback("pagehide", hide.as_ref().unchecked_ref())?;
    hide.forget();
    window.add_event_listener_with_callback("pageshow", show.as_ref().unchecked_ref())?;
    show.forget();
    INSTALLED.with(|v| v.set(true));
    Ok(())
}
