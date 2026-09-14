use crate::{fail, get, host, set, ERASING, STORE};
use js_sys::Uint8Array;
use serde::Deserialize;
use wasm_bindgen::{prelude::*, JsCast};
use zeroize::Zeroizing;

const CHUNK: u32 = 1024 * 1024;
#[wasm_bindgen]
pub async fn pick_file(photos: bool) -> Result<JsValue, JsValue> {
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| fail("Missing document"))?;
    let input = document
        .create_element("input")?
        .dyn_into::<web_sys::HtmlInputElement>()?;
    input.set_type("file");
    input.set_hidden(true);
    if photos {
        input.set_accept("image/jpeg,image/png,image/webp,image/gif,video/*");
    }
    document
        .body()
        .ok_or_else(|| fail("Missing document body"))?
        .append_child(&input)?;
    let (send, receive) = futures_channel::oneshot::channel();
    let pending = std::rc::Rc::new(std::cell::RefCell::new(Some(send)));
    let changed = pending.clone();
    let selected = input.clone();
    let change = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        if let Some(reply) = changed.borrow_mut().take() {
            let _ = reply.send(selected.files().and_then(|files| files.get(0)));
        }
    });
    let cancel = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        if let Some(reply) = pending.borrow_mut().take() {
            let _ = reply.send(None);
        }
    });
    input.add_event_listener_with_callback("change", change.as_ref().unchecked_ref())?;
    input.add_event_listener_with_callback("cancel", cancel.as_ref().unchecked_ref())?;
    input.click();
    let file = receive
        .await
        .map_err(|_| fail("File selection interrupted"));
    let _ = input.remove_event_listener_with_callback("change", change.as_ref().unchecked_ref());
    let _ = input.remove_event_listener_with_callback("cancel", cancel.as_ref().unchecked_ref());
    input.remove();
    let Some(file) = file? else {
        return Ok(JsValue::NULL);
    };
    if file.size() > 1024.0 * 1024.0 * 1024.0 {
        return Err(fail("Files must be at most 1 GiB"));
    }
    Ok(file.into())
}

#[wasm_bindgen]
pub fn file_metadata(file: web_sys::File) -> String {
    serde_json::json!({"name":file.name(),"media_type":if file.type_().is_empty(){"application/octet-stream".into()}else{file.type_()},"length":file.size() as u64}).to_string()
}

#[wasm_bindgen]
pub async fn file_slice(file: web_sys::Blob, index: u32) -> Result<Uint8Array, JsValue> {
    let start = f64::from(index) * f64::from(CHUNK);
    if start >= file.size() || file.size() > 1024.0 * 1024.0 * 1024.0 {
        return Err(fail("Invalid file range"));
    }
    let blob = file.slice_with_f64_and_f64(start, (start + f64::from(CHUNK)).min(file.size()))?;
    Ok(Uint8Array::new(
        &wasm_bindgen_futures::JsFuture::from(blob.array_buffer()).await?,
    ))
}

#[wasm_bindgen]
pub fn release_bytes(bytes: Uint8Array) {
    bytes.fill(0, 0, bytes.length());
}

#[wasm_bindgen]
pub async fn file_url(
    peer: String,
    author: String,
    message: String,
    draft: String,
    length: f64,
    media_type: String,
) -> Result<String, JsValue> {
    if !length.is_finite()
        || length < 0.0
        || length.fract() != 0.0
        || length > 128.0 * 1024.0 * 1024.0
    {
        return Err(fail(
            "Browser viewing and saving currently supports files up to 128 MiB",
        ));
    }
    let chunks = js_sys::Array::new();
    let mut total = 0u64;
    let mut index = 0;
    while total < length as u64 {
        let bytes = if draft.is_empty() {
            file_read(peer.clone(), author.clone(), message.clone(), index).await?
        } else {
            file_draft(draft.clone(), index).await?
        };
        let expected = (length as u64 - total).min(u64::from(CHUNK)) as u32;
        if bytes.length() != expected {
            release_bytes(bytes);
            return Err(fail("Incomplete attachment"));
        }
        // Blob copies each authenticated chunk so the mutable plaintext can be cleared immediately.
        let part = js_sys::Array::new();
        part.push(&bytes);
        let blob = web_sys::Blob::new_with_u8_array_sequence(&part);
        release_bytes(bytes);
        chunks.push(&blob?.into());
        total += u64::from(expected);
        index += 1;
    }
    let properties = web_sys::BlobPropertyBag::new();
    let safe_type = match media_type.as_str() {
        "image/jpeg" | "image/png" | "image/webp" | "image/gif" | "audio/mpeg" | "audio/ogg"
        | "audio/webm" | "audio/mp4" | "audio/wav" | "video/mp4" | "video/webm" | "video/ogg" => {
            media_type.as_str()
        }
        _ => "application/octet-stream",
    };
    properties.set_type(safe_type);
    let blob = web_sys::Blob::new_with_blob_sequence_and_options(&chunks, &properties)?;
    web_sys::Url::create_object_url_with_blob(&blob)
}

#[wasm_bindgen]
pub fn revoke_file_url(url: String) {
    let _ = web_sys::Url::revoke_object_url(&url);
}

#[wasm_bindgen]
pub fn save_file_url(url: String, name: String) -> Result<(), JsValue> {
    if !url.starts_with("blob:") {
        return Err(fail("Invalid attachment URL"));
    }
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| fail("Missing document"))?;
    let link = document
        .create_element("a")?
        .dyn_into::<web_sys::HtmlAnchorElement>()?;
    link.set_href(&url);
    link.set_download(&name);
    link.set_rel("noopener noreferrer");
    link.click();
    Ok(())
}
#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Operation {
    WallpaperStage {
        peer: String,
    },
    WallpaperRead {
        peer: String,
    },
    PhotoStage,
    PhotoRead {
        reference: String,
    },
    Stage {
        request: String,
        index: u32,
    },
    Read {
        peer: String,
        author: String,
        message: String,
        index: u32,
    },
    Draft {
        request: String,
        index: u32,
    },
}

#[wasm_bindgen]
pub async fn profile_image(reference: String) -> Result<String, JsValue> {
    use base64ct::{Base64, Encoding};
    let bytes = read(serde_json::json!({"operation":"photo_read","reference":reference})).await?;
    let plain = Zeroizing::new(bytes.to_vec());
    release_bytes(bytes);
    Ok(Base64::encode_string(&plain))
}

#[wasm_bindgen]
pub async fn profile_stage(file: Option<web_sys::File>) -> Result<(), JsValue> {
    let bytes = normalize_image(file, true).await?;
    let result = host::rpc(
        serde_json::json!({"operation":"photo_stage"}).to_string(),
        Some(bytes.clone()),
    )
    .await;
    release_bytes(bytes);
    result.map(|_| ())
}

#[wasm_bindgen]
pub async fn wallpaper_stage(peer: String, file: Option<web_sys::File>) -> Result<(), JsValue> {
    let bytes = normalize_image(file, false).await?;
    let result = host::rpc(
        serde_json::json!({"operation":"wallpaper_stage","peer":peer}).to_string(),
        Some(bytes.clone()),
    )
    .await;
    release_bytes(bytes);
    result.map(|_| ())
}

#[wasm_bindgen]
pub async fn wallpaper_image(peer: String) -> Result<String, JsValue> {
    use base64ct::{Base64, Encoding};
    let bytes = read(serde_json::json!({"operation":"wallpaper_read","peer":peer})).await?;
    let plain = Zeroizing::new(bytes.to_vec());
    release_bytes(bytes);
    Ok(Base64::encode_string(&plain))
}

async fn normalize_image(
    file: Option<web_sys::File>,
    profile: bool,
) -> Result<Uint8Array, JsValue> {
    use wasm_bindgen_futures::JsFuture;
    let bytes = if let Some(file) = file {
        if file.size() > 16.0 * 1024.0 * 1024.0 {
            return Err(fail("Images must be at most 16 MiB"));
        }
        let window = web_sys::window().ok_or_else(|| fail("Missing window"))?;
        let bitmap = JsFuture::from(window.create_image_bitmap_with_blob(&file)?)
            .await?
            .dyn_into::<web_sys::ImageBitmap>()?;
        let encoded = async {
            let canvas = window
                .document()
                .ok_or_else(|| fail("Missing document"))?
                .create_element("canvas")?
                .dyn_into::<web_sys::HtmlCanvasElement>()?;
            let context = canvas
                .get_context("2d")?
                .ok_or_else(|| fail("Image conversion unavailable"))?
                .dyn_into::<web_sys::CanvasRenderingContext2d>()?;
            let width = f64::from(bitmap.width());
            let height = f64::from(bitmap.height());
            let side = width.min(height);
            if side < 1.0 {
                return Err(fail("Invalid image"));
            }
            let scale = (1600.0 / width.max(height)).min(1.0);
            let (target_width, target_height) = if profile {
                (512, 512)
            } else {
                (
                    (width * scale).round().max(1.0) as u32,
                    (height * scale).round().max(1.0) as u32,
                )
            };
            canvas.set_width(target_width);
            canvas.set_height(target_height);
            context.set_fill_style_str("#ffffff");
            context.fill_rect(0.0, 0.0, target_width.into(), target_height.into());
            context.draw_image_with_image_bitmap_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
                &bitmap,
                if profile { (width - side) / 2.0 } else { 0.0 },
                if profile { (height - side) / 2.0 } else { 0.0 },
                if profile { side } else { width },
                if profile { side } else { height },
                0.0,
                0.0,
                target_width.into(),
                target_height.into(),
            )?;
            for quality in [0.88, 0.75, 0.60, 0.40] {
                let (send, receive) = futures_channel::oneshot::channel();
                let mut send = Some(send);
                let callback = Closure::<dyn FnMut(JsValue)>::new(move |blob| {
                    if let Some(send) = send.take() {
                        let _ = send.send(blob);
                    }
                });
                canvas.to_blob_with_type_and_encoder_options(
                    callback.as_ref().unchecked_ref(),
                    "image/jpeg",
                    &quality.into(),
                )?;
                let blob = receive
                    .await
                    .map_err(|_| fail("Image conversion interrupted"))?
                    .dyn_into::<web_sys::Blob>()?;
                if blob.size()
                    <= if profile {
                        128.0 * 1024.0
                    } else {
                        f64::from(CHUNK)
                    }
                {
                    return Ok(Uint8Array::new(&JsFuture::from(blob.array_buffer()).await?));
                }
            }
            Err(fail("Image is too complex; choose a smaller image"))
        }
        .await;
        bitmap.close();
        encoded?
    } else {
        Uint8Array::new_with_length(0)
    };
    Ok(bytes)
}

#[wasm_bindgen]
pub async fn file_stage(request: String, index: u32, bytes: Uint8Array) -> Result<(), JsValue> {
    if bytes.length() > CHUNK {
        return Err(fail("Attachment chunk too large"));
    }
    host::rpc(
        serde_json::json!({"operation":"stage","request":request,"index":index}).to_string(),
        Some(bytes),
    )
    .await?;
    Ok(())
}

#[wasm_bindgen]
pub async fn file_read(
    peer: String,
    author: String,
    message: String,
    index: u32,
) -> Result<Uint8Array, JsValue> {
    read(serde_json::json!({"operation":"read","peer":peer,"author":author,"message":message,"index":index})).await
}

#[wasm_bindgen]
pub async fn file_draft(request: String, index: u32) -> Result<Uint8Array, JsValue> {
    read(serde_json::json!({"operation":"draft","request":request,"index":index})).await
}

async fn read(request: serde_json::Value) -> Result<Uint8Array, JsValue> {
    let bytes = host::rpc(request.to_string(), Some(Uint8Array::new_with_length(0)))
        .await?
        .dyn_into::<Uint8Array>()?;
    if bytes.length() > CHUNK {
        return Err(fail("Invalid attachment chunk"));
    }
    Ok(bytes)
}

pub(crate) fn receive(event: &web_sys::MessageEvent) -> bool {
    let data = event.data();
    if get(&data, "binary").ok().and_then(|v| v.as_bool()) != Some(true) {
        return false;
    }
    let Ok(id) = get(&data, "id") else {
        return true;
    };
    if !id.as_string().is_some_and(|s| s.parse::<u64>().is_ok()) {
        return true;
    }
    let result = (|| -> Result<Uint8Array, JsValue> {
        let bytes = get(&data, "data")?.dyn_into::<Uint8Array>()?;
        if bytes.length() > CHUNK {
            return Err(fail("Attachment chunk too large"));
        }
        let raw = get(&data, "request")?
            .as_string()
            .ok_or_else(|| fail("Missing file command"))?;
        if raw.len() > 2048 || ERASING.with(std::cell::Cell::get) {
            return Err(fail("Unavailable"));
        }
        let operation =
            serde_json::from_str::<Operation>(&raw).map_err(|_| fail("Invalid file command"))?;
        STORE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let store = slot.as_mut().ok_or_else(|| fail("Browser is locked"))?;
            let result = match operation {
                Operation::WallpaperStage { peer } => {
                    let plain = Zeroizing::new(bytes.to_vec());
                    release_bytes(bytes);
                    store
                        .mobile_set_wallpaper(&peer, &plain)
                        .map(|()| Zeroizing::new(Vec::new()))
                }
                Operation::WallpaperRead { peer } if bytes.length() == 0 => store
                    .mobile_wallpaper(&peer)
                    .map(|image| image.unwrap_or_else(|| Zeroizing::new(Vec::new()))),
                Operation::PhotoStage => {
                    let plain = Zeroizing::new(bytes.to_vec());
                    bytes.fill(0, 0, bytes.length());
                    store
                        .mobile_stage_photo(&plain)
                        .map(|()| Zeroizing::new(Vec::new()))
                }
                Operation::PhotoRead { reference } if bytes.length() == 0 => store
                    .mobile_profile_image(&reference)
                    .map(|image| image.unwrap_or_else(|| Zeroizing::new(Vec::new()))),
                Operation::Stage { request, index } => {
                    let plain = Zeroizing::new(bytes.to_vec());
                    bytes.fill(0, 0, bytes.length());
                    store
                        .mobile_file_stage(&request, index, &plain)
                        .map(|()| Zeroizing::new(Vec::new()))
                }
                Operation::Read {
                    peer,
                    author,
                    message,
                    index,
                } if bytes.length() == 0 => {
                    store.mobile_file_chunk(&peer, &author, &message, index)
                }
                Operation::Draft { request, index } if bytes.length() == 0 => {
                    store.mobile_file_draft_chunk(&request, index)
                }
                _ => return Err(fail("Invalid file payload")),
            }
            .map_err(|_| fail("Attachment is unavailable"))?;
            Ok(Uint8Array::from(result.as_slice()))
        })
    })();
    let reply = js_sys::Object::new();
    let _ = set(&reply, "binary", &true.into());
    let _ = set(&reply, "id", &id);
    let _ = set(&reply, "ok", &result.is_ok().into());
    let bytes = result.unwrap_or_else(|_| Uint8Array::new_with_length(0));
    let _ = set(&reply, "data", &bytes);
    let worker = js_sys::global().unchecked_into::<web_sys::DedicatedWorkerGlobalScope>();
    let transfers = js_sys::Array::new();
    transfers.push(&bytes.buffer());
    let _ = worker.post_message_with_transfer(&reply, &transfers);
    true
}

#[wasm_bindgen]
extern "C" {
    type SaveWindow;
    #[wasm_bindgen(method,catch,js_name=showSaveFilePicker)]
    fn save_picker(this: &SaveWindow, options: &JsValue) -> Result<js_sys::Promise, JsValue>;
    type SaveHandle;
    #[wasm_bindgen(method,catch,js_name=createWritable)]
    fn writable(this: &SaveHandle) -> Result<js_sys::Promise, JsValue>;
    type SaveStream;
    #[wasm_bindgen(method, catch)]
    fn write(this: &SaveStream, data: &Uint8Array) -> Result<js_sys::Promise, JsValue>;
    #[wasm_bindgen(method, catch)]
    fn close(this: &SaveStream) -> Result<js_sys::Promise, JsValue>;
    #[wasm_bindgen(method, catch)]
    fn abort(this: &SaveStream) -> Result<js_sys::Promise, JsValue>;
}
#[wasm_bindgen]
pub fn file_stream_supported() -> bool {
    web_sys::window()
        .is_some_and(|w| get(w.as_ref(), "showSaveFilePicker").is_ok_and(|f| f.is_function()))
}
#[wasm_bindgen]
pub fn file_destination(name: String) -> Result<js_sys::Promise, JsValue> {
    let name: String = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("Attachment")
        .chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect();
    let options = js_sys::Object::new();
    set(
        &options,
        "suggestedName",
        &JsValue::from_str(if name.is_empty() { "Attachment" } else { &name }),
    )?;
    let window = web_sys::window().ok_or_else(|| fail("Missing window"))?;
    let pending = window.unchecked_ref::<SaveWindow>().save_picker(&options)?;
    Ok(wasm_bindgen_futures::future_to_promise(async move {
        match wasm_bindgen_futures::JsFuture::from(pending).await {
            Err(error)
                if get(&error, "name")
                    .ok()
                    .and_then(|v| v.as_string())
                    .as_deref()
                    == Some("AbortError") =>
            {
                Ok(JsValue::NULL)
            }
            result => result,
        }
    }))
}
#[wasm_bindgen]
pub async fn file_save(
    handle: JsValue,
    peer: String,
    author: String,
    message: String,
    draft: String,
    length: f64,
) -> Result<(), JsValue> {
    use wasm_bindgen_futures::JsFuture;
    if !length.is_finite() || length < 0. || length.fract() != 0. || length > 1024. * 1024. * 1024.
    {
        return Err(fail("Invalid attachment size"));
    }
    let stream: SaveStream = JsFuture::from(handle.unchecked_ref::<SaveHandle>().writable()?)
        .await?
        .unchecked_into();
    let result = async {
        let mut total = 0u64;
        let mut index = 0;
        while total < length as u64 {
            let bytes = if draft.is_empty() {
                file_read(peer.clone(), author.clone(), message.clone(), index).await?
            } else {
                file_draft(draft.clone(), index).await?
            };
            let expected = (length as u64 - total).min(u64::from(CHUNK)) as u32;
            if bytes.length() != expected {
                release_bytes(bytes);
                return Err(fail("Incomplete attachment"));
            }
            let written = match stream.write(&bytes) {
                Ok(p) => JsFuture::from(p).await,
                Err(e) => Err(e),
            };
            release_bytes(bytes);
            written?;
            total += u64::from(expected);
            index += 1;
        }
        JsFuture::from(stream.close()?).await?;
        Ok(())
    }
    .await;
    if result.is_err() {
        if let Ok(abort) = stream.abort() {
            let _ = JsFuture::from(abort).await;
        }
    }
    result
}
