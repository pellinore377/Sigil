#![forbid(unsafe_code)]
mod codec;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use wasm_bindgen::prelude::*;
use zeroize::Zeroize;
struct Ring {
    data: Box<[f32]>,
    head: usize,
    len: usize,
}
impl Ring {
    fn new(size: usize) -> Self {
        Self {
            data: vec![0.0; size].into_boxed_slice(),
            head: 0,
            len: 0,
        }
    }
    fn clear(&mut self) {
        self.data.zeroize();
        self.head = 0;
        self.len = 0;
    }
    fn push(&mut self, v: f32) {
        if self.len == self.data.len() {
            self.pop();
        }
        let at = (self.head + self.len) % self.data.len();
        self.data[at] = if v.is_finite() {
            v.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        self.len += 1;
    }
    fn pop(&mut self) -> f32 {
        if self.len == 0 {
            return 0.0;
        }
        let value = self.data[self.head];
        self.data[self.head] = 0.0;
        self.head = (self.head + 1) % self.data.len();
        self.len -= 1;
        value
    }
}
impl Drop for Ring {
    fn drop(&mut self) {
        self.clear();
    }
}
struct Playback {
    samples: Ring,
    decoded: u64,
    ready: bool,
}
#[wasm_bindgen]
pub struct AudioEngine {
    stream: Option<cpal::Stream>,
    codec: codec::Codec,
    input: Arc<Mutex<Ring>>,
    output: Arc<Mutex<Vec<Playback>>>,
    muted: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    count: Arc<AtomicU64>,
}
#[wasm_bindgen]
impl AudioEngine {
    #[wasm_bindgen(constructor)]
    pub fn new(encoded: js_sys::Function) -> Result<AudioEngine, JsValue> {
        let error = |e: cpal::Error| JsValue::from_str(&e.to_string());
        let host = cpal::host_from_id(cpal::HostId::AudioWorklet).map_err(error)?;
        let device = host
            .default_input_device()
            .ok_or(JsValue::from_str("No audio input"))?;
        // A second of capture: the page's single thread can stall for hundreds of milliseconds in a sync pass.
        let input = Arc::new(Mutex::new(Ring::new(48000)));
        let output = Arc::new(Mutex::new(
            (0..7)
                .map(|_| Playback {
                    samples: Ring::new(48000),
                    decoded: 0,
                    ready: false,
                })
                .collect::<Vec<_>>(),
        ));
        let muted = Arc::new(AtomicBool::new(true));
        let failed = Arc::new(AtomicBool::new(false));
        let count = Arc::new(AtomicU64::new(0));
        let codec = codec::Codec::new(encoded, output.clone(), failed.clone())?;
        let (capture, playback, mute, failure, frames) = (
            input.clone(),
            output.clone(),
            muted.clone(),
            failed.clone(),
            count.clone(),
        );
        let stream = device
            .build_duplex_stream(
                cpal::DuplexStreamConfig {
                    input_channels: 1,
                    output_channels: 1,
                    sample_rate: 48000,
                    buffer_size: cpal::BufferSize::Default,
                },
                move |data: &[f32], render: &mut [f32], _: &cpal::DuplexCallbackInfo| {
                    frames.fetch_add(data.len() as u64, Ordering::Relaxed);
                    if let Ok(mut samples) = capture.try_lock() {
                        if mute.load(Ordering::Relaxed) {
                            samples.clear();
                        } else {
                            for v in data {
                                samples.push(*v);
                            }
                        }
                    }
                    render.fill(0.0);
                    if let Ok(mut voices) = playback.try_lock() {
                        for voice in voices.iter_mut() {
                            // Sixty milliseconds of margin before a voice starts; after an underrun it resumes at twenty.
                            if !voice.ready && voice.samples.len >= if voice.decoded == 0 { 2880 } else { 960 } {
                                voice.ready = true;
                            }
                            if voice.ready {
                                for sample in render.iter_mut() {
                                    *sample += voice.samples.pop();
                                }
                                if voice.samples.len == 0 {
                                    voice.ready = false;
                                }
                            }
                        }
                        for sample in render {
                            *sample = sample.clamp(-1.0, 1.0);
                        }
                    }
                },
                move |_: cpal::Error| {
                    failure.store(true, Ordering::Relaxed);
                },
                None,
            )
            .map_err(error)?;
        stream.start().map_err(error)?;
        Ok(AudioEngine {
            stream: Some(stream),
            codec,
            input,
            output,
            muted,
            failed,
            count,
        })
    }
    pub fn decoded_samples(&self) -> f64 {
        self.output
            .try_lock()
            .map(|voices| voices.iter().map(|v| v.decoded).sum::<u64>() as f64)
            .unwrap_or(0.0)
    }
    pub fn count(&self) -> f64 {
        self.count.load(Ordering::Relaxed) as f64
    }
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }
    fn read(&self) -> js_sys::Float32Array {
        let Ok(mut input) = self.input.try_lock() else {
            return js_sys::Float32Array::new_with_length(0);
        };
        if input.len < 960 {
            return js_sys::Float32Array::new_with_length(0);
        }
        let mut data = [0.0; 960];
        for v in &mut data {
            *v = input.pop();
        }
        let out = js_sys::Float32Array::from(data.as_slice());
        data.zeroize();
        out
    }
    pub fn mute(&self, value: bool) {
        self.muted.store(value, Ordering::Relaxed);
        if value && let Ok(mut input) = self.input.try_lock() {
            input.clear();
        }
    }
    pub fn pause(&self) -> Result<(), JsValue> {
        self.stream
            .as_ref()
            .ok_or(JsValue::from_str("Closed"))?
            .pause()
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }
    pub fn pump(&mut self) -> Result<bool, JsValue> {
        let samples = self.read();
        if samples.length() == 0 {
            return Ok(false);
        }
        self.codec.encode(samples)
    }
    pub fn receive(
        &mut self,
        peer: usize,
        timestamp: f64,
        bytes: js_sys::Uint8Array,
    ) -> Result<bool, JsValue> {
        self.codec.decode(peer, timestamp, bytes)
    }
    pub fn receive_frame(
        &mut self,
        peer: usize,
        frame: js_sys::Uint8Array,
    ) -> Result<bool, JsValue> {
        if frame.length() < 11
            || frame.length() > 8202
            || frame.get_index(0) != 0
            || frame.get_index(1) != 0
        {
            return Err(js_sys::Error::new("Invalid audio frame").into());
        }
        let mut timestamp = [0; 8];
        frame.subarray(2, 10).copy_to(&mut timestamp);
        let timestamp = u64::from_be_bytes(timestamp);
        if timestamp > 9_007_199_254_740_991 {
            return Err(js_sys::Error::new("Invalid audio timestamp").into());
        }
        self.codec
            .decode(peer, timestamp as f64, frame.subarray(10, frame.length()))
    }
    pub fn close(&mut self) {
        self.codec.close();
        self.stream.take();
        self.muted.store(true, Ordering::Relaxed);
        if let Ok(mut input) = self.input.try_lock() {
            input.clear();
        }
        if let Ok(mut output) = self.output.try_lock() {
            for voice in output.iter_mut() {
                voice.samples.clear();
                voice.ready = false;
            }
        }
    }
}
impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.close();
    }
}
