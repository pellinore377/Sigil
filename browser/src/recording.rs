use crate::fail;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::JsFuture;
use web_sys::*;

struct Recording {
    recorder: MediaRecorder,
    stream: MediaStream,
    context: AudioContext,
    analyser: AnalyserNode,
    _source: MediaStreamAudioSourceNode,
    chunks: Rc<RefCell<Vec<Blob>>>,
    failed: Rc<Cell<bool>>,
    _data: Closure<dyn FnMut(BlobEvent)>,
    _error: Closure<dyn FnMut(Event)>,
    started: f64,
}
impl Drop for Recording {
    fn drop(&mut self) {
        self.recorder.set_ondataavailable(None);
        self.recorder.set_onerror(None);
        self.recorder.set_onstop(None);
        if self.recorder.state() != RecordingState::Inactive {
            let _ = self.recorder.stop();
        }
        stop_tracks(&self.stream);
        let _ = self.context.close();
    }
}
fn stop_tracks(stream: &MediaStream) {
    for track in stream.get_tracks() {
        if let Ok(track) = track.dyn_into::<MediaStreamTrack>() {
            track.stop();
        }
    }
}
thread_local! {
    static RECORDING: RefCell<Option<Recording>> = const {RefCell::new(None)};
    static GENERATION: Cell<u32> = const {Cell::new(0)};
}

#[wasm_bindgen]
pub fn voice_cancel() {
    GENERATION.with(|g| g.set(g.get().wrapping_add(1)));
    RECORDING.with(|r| r.borrow_mut().take());
}

#[wasm_bindgen]
pub async fn voice_start() -> Result<(), JsValue> {
    voice_cancel();
    let generation = GENERATION.with(Cell::get);
    let media_type = [
        "audio/webm;codecs=opus",
        "audio/ogg;codecs=opus",
        "audio/mp4",
    ]
    .into_iter()
    .find(|kind| MediaRecorder::is_type_supported(kind))
    .ok_or_else(|| fail("Voice recording is not supported by this browser"))?;
    let settings = MediaStreamConstraints::new();
    settings.set_audio(&true.into());
    settings.set_video(&false.into());
    let stream = JsFuture::from(
        window()
            .ok_or_else(|| fail("Missing window"))?
            .navigator()
            .media_devices()?
            .get_user_media_with_constraints(&settings)?,
    )
    .await?
    .dyn_into::<MediaStream>()?;
    if generation != GENERATION.with(Cell::get) {
        stop_tracks(&stream);
        return Err(fail("Recording cancelled"));
    }
    let build = (|| -> Result<Recording, JsValue> {
        let options = MediaRecorderOptions::new();
        options.set_mime_type(media_type);
        options.set_audio_bits_per_second(96000);
        let recorder =
            MediaRecorder::new_with_media_stream_and_media_recorder_options(&stream, &options)?;
        let context = AudioContext::new()?;
        let analyser = context.create_analyser()?;
        analyser.set_fft_size(256);
        let source = context.create_media_stream_source(&stream)?;
        source.connect_with_audio_node(&analyser)?;
        let chunks = Rc::new(RefCell::new(Vec::new()));
        let failed = Rc::new(Cell::new(false));
        let data_chunks = chunks.clone();
        let data_failed = failed.clone();
        let data = Closure::<dyn FnMut(BlobEvent)>::new(move |event: BlobEvent| {
            if let Some(blob) = event.data() {
                let mut chunks = data_chunks.borrow_mut();
                let size: f64 = chunks.iter().map(Blob::size).sum();
                if size + blob.size() > 32.0 * 1024.0 * 1024.0 || chunks.len() >= 620 {
                    data_failed.set(true);
                } else if blob.size() > 0.0 {
                    chunks.push(blob);
                }
            }
        });
        let error_failed = failed.clone();
        let error = Closure::<dyn FnMut(Event)>::new(move |_| {
            error_failed.set(true);
        });
        recorder.set_ondataavailable(Some(data.as_ref().unchecked_ref()));
        recorder.set_onerror(Some(error.as_ref().unchecked_ref()));
        Ok(Recording {
            recorder,
            stream: stream.clone(),
            context,
            analyser,
            _source: source,
            chunks,
            failed,
            _data: data,
            _error: error,
            started: js_sys::Date::now(),
        })
    })();
    let recording = match build {
        Ok(recording) => recording,
        Err(error) => {
            stop_tracks(&stream);
            return Err(error);
        }
    };
    JsFuture::from(recording.context.resume()?).await?;
    if generation != GENERATION.with(Cell::get) {
        return Err(fail("Recording cancelled"));
    }
    recording.recorder.start_with_time_slice(1000)?;
    RECORDING.with(|r| *r.borrow_mut() = Some(recording));
    Ok(())
}

#[wasm_bindgen]
pub fn voice_pause() -> Result<bool, JsValue> {
    RECORDING.with(|r| {
        let r = r.borrow();
        let r = r.as_ref().ok_or_else(|| fail("Not recording"))?;
        if r.recorder.state() == RecordingState::Paused {
            r.recorder.resume()?;
            Ok(false)
        } else {
            r.recorder.pause()?;
            Ok(true)
        }
    })
}

#[wasm_bindgen]
pub fn voice_level() -> Result<f32, JsValue> {
    RECORDING.with(|r| {
        let r = r.borrow();
        let r = r.as_ref().ok_or_else(|| fail("Not recording"))?;
        if r.failed.get() || js_sys::Date::now() - r.started > 600000.0 {
            return Err(fail("Recording limit reached"));
        }
        let mut bytes = [128u8; 256];
        r.analyser.get_byte_time_domain_data(&mut bytes);
        let rms = (bytes
            .iter()
            .map(|b| ((*b as f32 - 128.0) / 128.0).powi(2))
            .sum::<f32>()
            / 256.0)
            .sqrt();
        Ok((rms * 4.0).clamp(0.02, 1.0))
    })
}

#[wasm_bindgen]
pub async fn voice_finish() -> Result<File, JsValue> {
    let r = RECORDING
        .with(|r| r.borrow_mut().take())
        .ok_or_else(|| fail("Not recording"))?;
    let generation = GENERATION.with(Cell::get);
    let (send, receive) = futures_channel::oneshot::channel();
    let mut send = Some(send);
    let stop = Closure::<dyn FnMut(Event)>::new(move |_| {
        if let Some(send) = send.take() {
            let _ = send.send(());
        }
    });
    r.recorder.set_onstop(Some(stop.as_ref().unchecked_ref()));
    r.recorder.stop()?;
    stop_tracks(&r.stream);
    receive.await.map_err(|_| fail("Recording interrupted"))?;
    r.recorder.set_onstop(None);
    if generation != GENERATION.with(Cell::get) || r.failed.get() {
        return Err(fail("Recording was cancelled or could not finish"));
    }
    let chunks = js_sys::Array::new();
    for blob in r.chunks.borrow().iter() {
        chunks.push(blob);
    }
    let media_type = r.recorder.mime_type();
    let media_type = media_type.split(';').next().unwrap_or("audio/webm");
    let options = FilePropertyBag::new();
    options.set_type(media_type);
    let name = match media_type {
        "audio/mp4" => "Voice message.m4a",
        "audio/ogg" => "Voice message.ogg",
        _ => "Voice message.webm",
    };
    let file = File::new_with_blob_sequence_and_options(&chunks, name, &options)?;
    if file.size() == 0.0 {
        return Err(fail("Recording was empty"));
    }
    Ok(file)
}
