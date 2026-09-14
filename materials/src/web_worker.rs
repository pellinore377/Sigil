use futures_channel::oneshot;
use std::{cell::RefCell, collections::BTreeMap};
use wasm_bindgen::{prelude::*, JsCast};
use web_sys::{MessageEvent, Worker, WorkerOptions, WorkerType};
type Reply = oneshot::Sender<Result<String, JsValue>>;
thread_local! {static HOST:RefCell<Host>=RefCell::new(Host{worker:None,pending:BTreeMap::new(),next:0});}
type Running = (
    Worker,
    Closure<dyn FnMut(MessageEvent)>,
    Closure<dyn FnMut(web_sys::Event)>,
);
struct Host {
    worker: Option<Running>,
    pending: BTreeMap<u32, Reply>,
    next: u32,
}
fn fail() -> JsValue {
    JsValue::from_str("Material animation unavailable")
}
#[wasm_bindgen]
pub fn material_shutdown() {
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        if let Some((worker, _, _)) = host.worker.take() {
            worker.terminate();
        }
        for (_, reply) in std::mem::take(&mut host.pending) {
            let _ = reply.send(Err(fail()));
        }
    });
}
#[wasm_bindgen]
pub async fn material_record_async(input: String) -> Result<String, JsValue> {
    if input.len() > 8192 {
        return Err(fail());
    }
    let (send, receive) = oneshot::channel();
    let id = HOST.with(|host| -> Result<u32, JsValue> {
        let mut host = host.borrow_mut();
        if host.pending.len() >= 8 {
            return Err(fail());
        }
        if host.worker.is_none() {
            let options = WorkerOptions::new();
            options.set_type(WorkerType::Module);
            let worker = Worker::new_with_options("/web/sigil-material-worker.mjs", &options)?;
            let message = Closure::<dyn FnMut(MessageEvent)>::new(|event: MessageEvent| {
                let array = js_sys::Array::from(&event.data());
                let Some(id) = array
                    .get(0)
                    .as_f64()
                    .filter(|v| *v >= 1. && *v <= u32::MAX as f64 && v.fract() == 0.)
                else {
                    return;
                };
                let result = array
                    .get(1)
                    .as_string()
                    .filter(|s| s.len() <= 2 * 1024 * 1024)
                    .ok_or_else(fail);
                HOST.with(|host| {
                    if let Some(reply) = host.borrow_mut().pending.remove(&(id as u32)) {
                        let _ = reply.send(result);
                    }
                });
            });
            let error = Closure::<dyn FnMut(web_sys::Event)>::new(|_| material_shutdown());
            worker.set_onmessage(Some(message.as_ref().unchecked_ref()));
            worker.set_onerror(Some(error.as_ref().unchecked_ref()));
            host.worker = Some((worker, message, error));
        }
        host.next = host.next.checked_add(1).ok_or_else(fail)?;
        let id = host.next;
        let packet = js_sys::Array::new();
        packet.push(&JsValue::from_f64(id as f64));
        packet.push(&JsValue::from_str(&input));
        host.worker
            .as_ref()
            .ok_or_else(fail)?
            .0
            .post_message(&packet)?;
        host.pending.insert(id, send);
        Ok(id)
    })?;
    let timeout = Closure::<dyn FnMut()>::new(move || {
        HOST.with(|host| {
            if let Some(reply) = host.borrow_mut().pending.remove(&id) {
                let _ = reply.send(Err(fail()));
            }
        });
    });
    let window = web_sys::window().ok_or_else(fail)?;
    let timer = window.set_timeout_with_callback_and_timeout_and_arguments_0(
        timeout.as_ref().unchecked_ref(),
        15000,
    )?;
    let result = receive.await.map_err(|_| fail())?;
    window.clear_timeout_with_handle(timer);
    result
}
#[wasm_bindgen]
pub fn material_worker_receive(data: JsValue) {
    let packet = js_sys::Array::from(&data);
    let Some(id) = packet
        .get(0)
        .as_f64()
        .filter(|v| *v >= 1. && *v <= u32::MAX as f64 && v.fract() == 0.)
    else {
        return;
    };
    let result = packet
        .get(1)
        .as_string()
        .ok_or_else(fail)
        .and_then(|s| crate::web::material_record(&s));
    let reply = js_sys::Array::new();
    reply.push(&JsValue::from_f64(id));
    reply.push(&result.map(JsValue::from).unwrap_or(JsValue::NULL));
    if let Ok(worker) = js_sys::global().dyn_into::<web_sys::DedicatedWorkerGlobalScope>() {
        let _ = worker.post_message(&reply);
    }
}
