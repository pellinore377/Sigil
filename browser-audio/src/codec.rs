use crate::Playback;
use js_sys::{Array, Float32Array, Function, Reflect, Uint8Array};
use std::{
    cell::Cell,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use wasm_bindgen::{JsCast, prelude::*};
use zeroize::Zeroize;
fn get(v: &JsValue, k: &str) -> Result<JsValue, JsValue> {
    Reflect::get(v, &k.into())
}
fn set(v: &JsValue, k: &str, value: &JsValue) -> Result<(), JsValue> {
    Reflect::set(v, &k.into(), value).map(|_| ())
}
fn invoke(v: &JsValue, k: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
    get(v, k)?
        .dyn_into::<Function>()?
        .apply(v, &args.iter().collect::<Array>())
}
fn construct(name: &str, options: &JsValue) -> Result<JsValue, JsValue> {
    Reflect::construct(
        &get(&js_sys::global(), name)?.dyn_into::<Function>()?,
        &Array::of1(options),
    )
}
fn object(raw: &str) -> Result<JsValue, JsValue> {
    js_sys::JSON::parse(raw)
}
fn failure() -> JsValue {
    js_sys::Error::new("Audio codec is unavailable").into()
}
pub struct Codec {
    encoder: JsValue,
    decoders: Vec<JsValue>,
    clock: u64,
    last: [Option<f64>; 7],
    closed: Rc<Cell<bool>>,
    _encoded: Closure<dyn FnMut(JsValue)>,
    _decoded: Vec<Closure<dyn FnMut(JsValue)>>,
    _error: Closure<dyn FnMut(JsValue)>,
}
impl Codec {
    pub fn new(
        output: Function,
        playback: Arc<Mutex<Vec<Playback>>>,
        failed: Arc<AtomicBool>,
    ) -> Result<Self, JsValue> {
        let closed = Rc::new(Cell::new(false));
        let active = closed.clone();
        let failed_output = failed.clone();
        let encoded = Closure::<dyn FnMut(JsValue)>::new(move |chunk| {
            if active.get() {
                return;
            }
            let result = (|| {
                let size = get(&chunk, "byteLength")?.as_f64().ok_or_else(failure)?;
                if !(1.0..=8192.0).contains(&size) {
                    return Err(failure());
                }
                let bytes = Uint8Array::new_with_length(size as u32);
                invoke(&chunk, "copyTo", &[bytes.clone().into()])?;
                let result = output.call2(&JsValue::NULL, &get(&chunk, "timestamp")?, &bytes);
                bytes.fill(0, 0, bytes.length());
                result.map(|_| ())
            })();
            if result.is_err() {
                failed_output.store(true, Ordering::Relaxed);
            }
        });
        let error = Closure::<dyn FnMut(JsValue)>::new(move |_| {
            failed.store(true, Ordering::Relaxed);
        });
        let options = js_sys::Object::new();
        set(&options, "output", encoded.as_ref())?;
        set(&options, "error", error.as_ref())?;
        let encoder = construct("AudioEncoder", &options)?;
        let mut codec = Self {
            encoder,
            decoders: Vec::new(),
            clock: 0,
            last: [None; 7],
            closed,
            _encoded: encoded,
            _decoded: Vec::new(),
            _error: error,
        };
        invoke(
            &codec.encoder,
            "configure",
            &[object(
                r#"{"codec":"opus","sampleRate":48000,"numberOfChannels":1,"bitrate":32000}"#,
            )?],
        )?;
        for peer in 0..7 {
            let queue = playback.clone();
            let active = codec.closed.clone();
            let decoded = Closure::<dyn FnMut(JsValue)>::new(move |frame| {
                let _ = (|| {
                    if active.get() {
                        return Ok::<(), JsValue>(());
                    }
                    let size = get(&frame, "numberOfFrames")?
                        .as_f64()
                        .ok_or_else(failure)?;
                    if !(1.0..=5760.0).contains(&size)
                        || get(&frame, "sampleRate")?.as_f64() != Some(48000.0)
                        || get(&frame, "numberOfChannels")?.as_f64() != Some(1.0)
                    {
                        return Err(failure());
                    }
                    let pcm = Float32Array::new_with_length(size as u32);
                    invoke(
                        &frame,
                        "copyTo",
                        &[
                            pcm.clone().into(),
                            object(r#"{"planeIndex":0,"format":"f32-planar"}"#)?,
                        ],
                    )?;
                    if let Ok(mut voices) = queue.try_lock() {
                        voices[peer].decoded =
                            voices[peer].decoded.saturating_add(pcm.length() as u64);
                        let mut samples = pcm.to_vec();
                        for sample in &samples {
                            voices[peer].samples.push(*sample);
                        }
                        samples.zeroize();
                    }
                    pcm.fill(0.0, 0, pcm.length());
                    Ok(())
                })();
                let _ = invoke(&frame, "close", &[]);
            });
            let options = js_sys::Object::new();
            set(&options, "output", decoded.as_ref())?;
            set(&options, "error", codec._error.as_ref())?;
            let decoder = construct("AudioDecoder", &options)?;
            codec.decoders.push(decoder.clone());
            codec._decoded.push(decoded);
            invoke(
                &decoder,
                "configure",
                &[object(
                    r#"{"codec":"opus","sampleRate":48000,"numberOfChannels":1}"#,
                )?],
            )?;
        }
        Ok(codec)
    }
    pub fn encode(&mut self, pcm: Float32Array) -> Result<bool, JsValue> {
        if self.closed.get() || pcm.length() != 960 {
            return Err(failure());
        }
        let timestamp = self.clock;
        self.clock = self.clock.checked_add(20000).ok_or_else(failure)?;
        if get(&self.encoder, "encodeQueueSize")?
            .as_f64()
            .unwrap_or(10.0)
            > 3.0
        {
            pcm.fill(0.0, 0, pcm.length());
            return Ok(false);
        }
        let options = object(
            r#"{"format":"f32-planar","sampleRate":48000,"numberOfFrames":960,"numberOfChannels":1}"#,
        )?;
        set(&options, "timestamp", &(timestamp as f64).into())?;
        set(&options, "data", &pcm)?;
        let frame = construct("AudioData", &options);
        pcm.fill(0.0, 0, pcm.length());
        let frame = frame?;
        let result = invoke(&self.encoder, "encode", std::slice::from_ref(&frame));
        let _ = invoke(&frame, "close", &[]);
        result.map(|_| true)
    }
    pub fn decode(
        &mut self,
        peer: usize,
        timestamp: f64,
        bytes: Uint8Array,
    ) -> Result<bool, JsValue> {
        if self.closed.get()
            || peer >= 7
            || !timestamp.is_finite()
            || timestamp < 0.0
            || timestamp.fract() != 0.0
            || timestamp > 9_007_199_254_740_991.0
            || bytes.length() == 0
            || bytes.length() > 8192
        {
            return Err(failure());
        }
        if self.last[peer].is_some_and(|old| timestamp <= old)
            || get(&self.decoders[peer], "decodeQueueSize")?
                .as_f64()
                .unwrap_or(10.0)
                > 7.0
        {
            return Ok(false);
        }
        let options = object(r#"{"type":"key"}"#)?;
        set(&options, "timestamp", &timestamp.into())?;
        set(&options, "data", &bytes)?;
        let chunk = construct("EncodedAudioChunk", &options)?;
        invoke(&self.decoders[peer], "decode", &[chunk])?;
        self.last[peer] = Some(timestamp);
        Ok(true)
    }
    pub fn close(&mut self) {
        self.closed.set(true);
        let _ = invoke(&self.encoder, "close", &[]);
        for decoder in &self.decoders {
            let _ = invoke(decoder, "close", &[]);
        }
    }
}
impl Drop for Codec {
    fn drop(&mut self) {
        self.close();
    }
}

#[wasm_bindgen]
pub async fn audio_supported() -> bool {
    async fn check() -> Result<bool, JsValue> {
        for (name, config) in [
            (
                "AudioEncoder",
                r#"{"codec":"opus","sampleRate":48000,"numberOfChannels":1,"bitrate":32000}"#,
            ),
            (
                "AudioDecoder",
                r#"{"codec":"opus","sampleRate":48000,"numberOfChannels":1}"#,
            ),
        ] {
            let ctor = get(&js_sys::global(), name)?;
            let result = invoke(&ctor, "isConfigSupported", &[object(config)?])?;
            let result =
                wasm_bindgen_futures::JsFuture::from(result.dyn_into::<js_sys::Promise>()?).await?;
            if get(&result, "supported")?.as_bool() != Some(true) {
                return Ok(false);
            }
        }
        Ok(get(&js_sys::global(), "crossOriginIsolated")?.as_bool() == Some(true))
    }
    check().await.unwrap_or(false)
}
