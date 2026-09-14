use crate::{fail, set};
use std::cell::{Cell, RefCell};
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::JsFuture;
use web_sys::*;
struct Camera {
    stream: MediaStream,
    video: HtmlVideoElement,
    canvas: HtmlCanvasElement,
    context: CanvasRenderingContext2d,
}
impl Drop for Camera {
    fn drop(&mut self) {
        stop(&self.stream);
        self.video.set_src_object(None);
    }
}
thread_local! {static CAMERA:RefCell<Option<Camera>>=const{RefCell::new(None)};static GENERATION:Cell<u32>=const{Cell::new(0)};}
fn stop(stream: &MediaStream) {
    for track in stream.get_tracks() {
        if let Ok(track) = track.dyn_into::<MediaStreamTrack>() {
            track.stop();
        }
    }
}
#[wasm_bindgen]
pub fn camera_stop() {
    GENERATION.with(|g| g.set(g.get().wrapping_add(1)));
    CAMERA.with(|c| c.borrow_mut().take());
}
#[wasm_bindgen]
pub async fn camera_start(video: HtmlVideoElement) -> Result<(), JsValue> {
    start(video, 640).await
}
#[wasm_bindgen]
pub async fn camera_start_photo(video: HtmlVideoElement) -> Result<(), JsValue> {
    start(video, 1280).await
}
async fn start(video: HtmlVideoElement, resolution: u32) -> Result<(), JsValue> {
    camera_stop();
    let generation = GENERATION.with(Cell::get);
    let window = web_sys::window().ok_or_else(|| fail("Missing window"))?;
    let document = window.document().ok_or_else(|| fail("Missing document"))?;
    let canvas = document
        .create_element("canvas")?
        .dyn_into::<HtmlCanvasElement>()?;
    let context = canvas
        .get_context("2d")?
        .ok_or_else(|| fail("Camera pixels unavailable"))?
        .dyn_into::<CanvasRenderingContext2d>()?;
    let constraints = MediaStreamConstraints::new();
    constraints.set_audio(&false.into());
    let settings = js_sys::Object::new();
    set(&settings, "facingMode", &"environment".into())?;
    set(&settings, "width", &resolution.into())?;
    set(&settings, "height", &resolution.into())?;
    constraints.set_video(&settings);
    let stream = JsFuture::from(
        window
            .navigator()
            .media_devices()?
            .get_user_media_with_constraints(&constraints)?,
    )
    .await?
    .dyn_into::<MediaStream>()?;
    if generation != GENERATION.with(Cell::get) {
        stop(&stream);
        return Err(fail("Camera cancelled"));
    }
    let camera = Camera {
        stream,
        video,
        canvas,
        context,
    };
    camera.video.set_muted(true);
    camera.video.set_attribute("playsinline", "")?;
    camera.video.set_src_object(Some(&camera.stream));
    JsFuture::from(camera.video.play()?).await?;
    if generation != GENERATION.with(Cell::get) {
        return Err(fail("Camera cancelled"));
    }
    CAMERA.with(|c| *c.borrow_mut() = Some(camera));
    Ok(())
}
#[wasm_bindgen]
pub fn camera_scan() -> Result<Option<String>, JsValue> {
    CAMERA.with(|c| {
        let c = c.borrow();
        let Some(camera) = c.as_ref() else {
            return Ok(None);
        };
        let width = camera.video.video_width();
        let height = camera.video.video_height();
        if width < 64 || height < 64 {
            return Ok(None);
        }
        let scale = 640.0 / f64::from(width.max(height));
        let w = (f64::from(width) * scale.min(1.0)) as u32;
        let h = (f64::from(height) * scale.min(1.0)) as u32;
        camera.canvas.set_width(w);
        camera.canvas.set_height(h);
        camera
            .context
            .draw_image_with_html_video_element_and_dw_and_dh(
                &camera.video,
                0.0,
                0.0,
                f64::from(w),
                f64::from(h),
            )?;
        let rgba = camera
            .context
            .get_image_data(0.0, 0.0, f64::from(w), f64::from(h))?
            .data();
        let grey = zeroize::Zeroizing::new(
            rgba.as_chunks::<4>()
                .0
                .iter()
                .map(|p| {
                    ((u32::from(p[0]) * 77 + u32::from(p[1]) * 150 + u32::from(p[2]) * 29) >> 8)
                        as u8
                })
                .collect::<Vec<_>>(),
        );
        Ok(sigil_client::link::scan_frame(
            w as usize, h as usize, &grey,
        ))
    })
}

#[wasm_bindgen]
pub async fn camera_photo() -> Result<File, JsValue> {
    let canvas = CAMERA.with(|c| -> Result<HtmlCanvasElement, JsValue> {
        let c = c.borrow();
        let camera = c.as_ref().ok_or_else(|| fail("Camera is not ready"))?;
        let width = camera.video.video_width();
        let height = camera.video.video_height();
        if width == 0 || height == 0 {
            return Err(fail("Camera is not ready"));
        }
        let scale = (1920.0 / f64::from(width.max(height))).min(1.0);
        let w = (f64::from(width) * scale) as u32;
        let h = (f64::from(height) * scale) as u32;
        camera.canvas.set_width(w);
        camera.canvas.set_height(h);
        camera
            .context
            .draw_image_with_html_video_element_and_dw_and_dh(
                &camera.video,
                0.0,
                0.0,
                f64::from(w),
                f64::from(h),
            )?;
        Ok(camera.canvas.clone())
    })?;
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
        &0.9.into(),
    )?;
    let blob = receive
        .await
        .map_err(|_| fail("Capture interrupted"))?
        .dyn_into::<Blob>()?;
    let parts = js_sys::Array::new();
    parts.push(&blob);
    let options = FilePropertyBag::new();
    options.set_type("image/jpeg");
    File::new_with_blob_sequence_and_options(&parts, "Photo.jpg", &options)
}

#[wasm_bindgen]
pub fn camera_photo_url(file: File) -> Result<String, JsValue> {
    Url::create_object_url_with_blob(&file)
}
