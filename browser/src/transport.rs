use crate::{fail, get, set};
use http::{Request, Response};
use js_sys::{Int32Array, Object, SharedArrayBuffer, Uint8Array};
use sigil_client::{browser_transport::Body, network::Error};
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::JsFuture;

const HEADER: usize = 8192;
const BODY: usize = 4 * 1024 * 1024;
const PREFIX: usize = 16;

pub fn send(request: Request<&[u8]>) -> Result<Response<Body>, Error> {
    worker_request(request).map_err(|_| Error::Transport)
}
fn worker_request(request: Request<&[u8]>) -> Result<Response<Body>, JsValue> {
    if request.body().len() > BODY {
        return Err(fail("Request too large"));
    }
    let buffer = SharedArrayBuffer::new((PREFIX + HEADER + BODY) as u32);
    let state = Int32Array::new(&buffer);
    let packet = Object::new();
    set(&packet, "transport", &true.into())?;
    set(&packet, "url", &request.uri().to_string().into())?;
    set(&packet, "method", &request.method().as_str().into())?;
    let headers: Vec<_> = request
        .headers()
        .iter()
        .map(|(k, v)| Ok((k.as_str(), v.to_str().map_err(|_| fail("Invalid header"))?)))
        .collect::<Result<_, JsValue>>()?;
    set(
        &packet,
        "headers",
        &serde_json::to_string(&headers)
            .map_err(|_| fail("Invalid headers"))?
            .into(),
    )?;
    set(&packet, "body", &Uint8Array::from(*request.body()))?;
    set(&packet, "buffer", &buffer)?;
    js_sys::global()
        .dyn_into::<web_sys::DedicatedWorkerGlobalScope>()?
        .post_message(&packet)?;
    js_sys::Atomics::wait_with_timeout(&state, 0, 0, 30_000.0)?;
    if js_sys::Atomics::load(&state, 0)? != 1 {
        let _ = js_sys::Atomics::compare_exchange(&state, 0, 0, 2);
        return Err(fail("Network request failed or timed out"));
    }
    let status = js_sys::Atomics::load(&state, 1)?;
    let header_len = js_sys::Atomics::load(&state, 2)?;
    let body_len = js_sys::Atomics::load(&state, 3)?;
    if !(100..=599).contains(&status)
        || !(0..=HEADER as i32).contains(&header_len)
        || !(0..=BODY as i32).contains(&body_len)
    {
        return Err(fail("Invalid transport response"));
    }
    let bytes = Uint8Array::new(&buffer);
    let headers: Vec<(String, String)> = serde_json::from_slice(
        &bytes
            .slice(PREFIX as u32, (PREFIX + header_len as usize) as u32)
            .to_vec(),
    )
    .map_err(|_| fail("Invalid response headers"))?;
    let mut response = Response::builder().status(status as u16);
    for (key, value) in headers {
        response = response.header(key, value);
    }
    let body = bytes
        .slice(
            (PREFIX + HEADER) as u32,
            (PREFIX + HEADER + body_len as usize) as u32,
        )
        .to_vec();
    bytes.fill(0, 0, bytes.length());
    response
        .body(Body::new(body))
        .map_err(|_| fail("Invalid response"))
}

pub async fn fetch(packet: JsValue) -> Result<(), JsValue> {
    let buffer = get(&packet, "buffer")?.dyn_into::<SharedArrayBuffer>()?;
    if buffer.byte_length() as usize != PREFIX + HEADER + BODY {
        return Err(fail("Invalid transport buffer"));
    }
    let state = Int32Array::new(&buffer);
    let result = fetch_response(&packet).await;
    if js_sys::Atomics::load(&state, 0)? != 0 {
        return Ok(());
    }
    match result {
        Ok((status, headers, body)) => {
            let bytes = Uint8Array::new(&buffer);
            bytes.set(&Uint8Array::from(headers.as_slice()), PREFIX as u32);
            bytes.set(&Uint8Array::from(body.as_slice()), (PREFIX + HEADER) as u32);
            js_sys::Atomics::store(&state, 1, status as i32)?;
            js_sys::Atomics::store(&state, 2, headers.len() as i32)?;
            js_sys::Atomics::store(&state, 3, body.len() as i32)?;
            js_sys::Atomics::compare_exchange(&state, 0, 0, 1)?;
        }
        Err(_) => {
            js_sys::Atomics::compare_exchange(&state, 0, 0, -1)?;
        }
    }
    js_sys::Atomics::notify(&state, 0)?;
    Ok(())
}
async fn fetch_response(packet: &JsValue) -> Result<(u16, Vec<u8>, Vec<u8>), JsValue> {
    let url = get(packet, "url")?
        .as_string()
        .ok_or_else(|| fail("Missing URL"))?;
    let uri = url.parse::<http::Uri>().map_err(|_| fail("Invalid URL"))?;
    if uri.scheme_str() != Some("https") || uri.authority().is_none_or(|v| v.as_str().contains('@'))
    {
        return Err(fail("HTTPS required"));
    }
    let method = get(packet, "method")?
        .as_string()
        .ok_or_else(|| fail("Missing method"))?;
    let headers: Vec<(String, String)> = serde_json::from_str(
        &get(packet, "headers")?
            .as_string()
            .ok_or_else(|| fail("Missing headers"))?,
    )
    .map_err(|_| fail("Invalid headers"))?;
    let body = get(packet, "body")?.dyn_into::<Uint8Array>()?;
    if body.length() as usize > BODY {
        return Err(fail("Request too large"));
    }
    let options = web_sys::RequestInit::new();
    options.set_method(&method);
    options.set_redirect(web_sys::RequestRedirect::Error);
    options.set_credentials(web_sys::RequestCredentials::Omit);
    let h = web_sys::Headers::new()?;
    if uri.path() != "/.well-known/sigil" {
        h.set("X-Sigil-Client", "1")?;
    }
    for (k, v) in headers {
        h.append(&k, &v)?;
    }
    options.set_headers(&h);
    if body.length() > 0 {
        options.set_body(&body);
    }
    let controller = web_sys::AbortController::new()?;
    options.set_signal(Some(&controller.signal()));
    let window = web_sys::window().ok_or_else(|| fail("Missing window"))?;
    let abort = controller.clone();
    let timeout = Closure::<dyn FnMut()>::new(move || abort.abort());
    let id = window.set_timeout_with_callback_and_timeout_and_arguments_0(
        timeout.as_ref().unchecked_ref(),
        25_000,
    )?;
    let response = async {
        let response = JsFuture::from(window.fetch_with_str_and_init(&url, &options))
            .await?
            .dyn_into::<web_sys::Response>()?;
        if response.redirected() {
            return Err(fail("Redirect refused"));
        }
        let mut headers = Vec::new();
        for entry in
            js_sys::try_iter(&response.headers())?.ok_or_else(|| fail("Invalid headers"))?
        {
            let entry = js_sys::Array::from(&entry?);
            let key = entry
                .get(0)
                .as_string()
                .ok_or_else(|| fail("Invalid header"))?;
            if key == "content-encoding" || key == "content-length" {
                continue;
            }
            headers.push((
                key,
                entry
                    .get(1)
                    .as_string()
                    .ok_or_else(|| fail("Invalid header"))?,
            ));
        }
        let headers = serde_json::to_vec(&headers).map_err(|_| fail("Invalid headers"))?;
        if headers.len() > HEADER {
            return Err(fail("Response headers too large"));
        }
        let mut body = Vec::new();
        if let Some(stream) = response.body() {
            let reader = stream
                .get_reader()
                .dyn_into::<web_sys::ReadableStreamDefaultReader>()?;
            loop {
                let chunk = JsFuture::from(reader.read()).await?;
                if get(&chunk, "done")?.as_bool() == Some(true) {
                    break;
                }
                let bytes = get(&chunk, "value")?.dyn_into::<Uint8Array>()?;
                if bytes.length() as usize > BODY - body.len() {
                    controller.abort();
                    return Err(fail("Response too large"));
                }
                body.extend_from_slice(&bytes.to_vec());
            }
            reader.release_lock();
        }
        Ok((response.status(), headers, body))
    }
    .await;
    window.clear_timeout_with_handle(id);
    if response.is_err() {
        controller.abort();
    }
    response
}
