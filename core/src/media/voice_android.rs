//! Android voice capture: AAudio (NDK, API 26+) straight over FFI — no ffmpeg
//! on a phone. Records 48k mono PCM, streams the same `voice.level` events as
//! the desktop path, and finalises a WAV the send path uploads as audio/wav.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::json;

use crate::engine::SharedEngine;

#[allow(non_camel_case_types)]
type aaudio_result_t = i32;

#[repr(C)]
struct AAudioStreamBuilder {
    _p: [u8; 0],
}
#[repr(C)]
struct AAudioStream {
    _p: [u8; 0],
}

const AAUDIO_DIRECTION_INPUT: i32 = 1;
const AAUDIO_FORMAT_PCM_I16: i32 = 1;

#[link(name = "aaudio")]
extern "C" {
    fn AAudio_createStreamBuilder(builder: *mut *mut AAudioStreamBuilder) -> aaudio_result_t;
    fn AAudioStreamBuilder_setDirection(builder: *mut AAudioStreamBuilder, direction: i32);
    fn AAudioStreamBuilder_setSampleRate(builder: *mut AAudioStreamBuilder, rate: i32);
    fn AAudioStreamBuilder_setChannelCount(builder: *mut AAudioStreamBuilder, count: i32);
    fn AAudioStreamBuilder_setFormat(builder: *mut AAudioStreamBuilder, format: i32);
    fn AAudioStreamBuilder_openStream(
        builder: *mut AAudioStreamBuilder,
        stream: *mut *mut AAudioStream,
    ) -> aaudio_result_t;
    fn AAudioStreamBuilder_delete(builder: *mut AAudioStreamBuilder) -> aaudio_result_t;
    fn AAudioStream_requestStart(stream: *mut AAudioStream) -> aaudio_result_t;
    fn AAudioStream_requestStop(stream: *mut AAudioStream) -> aaudio_result_t;
    fn AAudioStream_close(stream: *mut AAudioStream) -> aaudio_result_t;
    fn AAudioStream_read(
        stream: *mut AAudioStream,
        buffer: *mut core::ffi::c_void,
        num_frames: i32,
        timeout_nanos: i64,
    ) -> aaudio_result_t;
}

pub const SAMPLE_RATE: usize = 48_000;

pub struct Recorder {
    stop: Arc<AtomicBool>,
    samples: Arc<Mutex<Vec<i16>>>,
    thread: Option<std::thread::JoinHandle<()>>,
    pub path: PathBuf,
}

pub fn start(engine: &SharedEngine, path: PathBuf) -> anyhow::Result<Recorder> {
    let stop = Arc::new(AtomicBool::new(false));
    let samples = Arc::new(Mutex::new(Vec::<i16>::new()));

    // Open on the caller so failure (no permission, no mic) reports upward.
    let stream = unsafe {
        let mut builder: *mut AAudioStreamBuilder = core::ptr::null_mut();
        if AAudio_createStreamBuilder(&mut builder) != 0 || builder.is_null() {
            anyhow::bail!("AAudio builder failed");
        }
        AAudioStreamBuilder_setDirection(builder, AAUDIO_DIRECTION_INPUT);
        AAudioStreamBuilder_setSampleRate(builder, SAMPLE_RATE as i32);
        AAudioStreamBuilder_setChannelCount(builder, 1);
        AAudioStreamBuilder_setFormat(builder, AAUDIO_FORMAT_PCM_I16);
        let mut stream: *mut AAudioStream = core::ptr::null_mut();
        let rc = AAudioStreamBuilder_openStream(builder, &mut stream);
        AAudioStreamBuilder_delete(builder);
        if rc != 0 || stream.is_null() {
            anyhow::bail!("microphone stream failed to open ({rc}) — is the permission granted?");
        }
        if AAudioStream_requestStart(stream) != 0 {
            AAudioStream_close(stream);
            anyhow::bail!("microphone stream failed to start");
        }
        stream as usize // usize crosses the thread boundary; raw pointers are !Send
    };

    let stop2 = stop.clone();
    let samples2 = samples.clone();
    let eng = engine.clone();
    let thread = std::thread::spawn(move || {
        let stream = stream as *mut AAudioStream;
        let mut buf = vec![0i16; SAMPLE_RATE / 50]; // 20ms
        let mut level_window: Vec<i16> = Vec::with_capacity(SAMPLE_RATE / 10);
        loop {
            if stop2.load(Ordering::Relaxed) {
                break;
            }
            let n = unsafe {
                AAudioStream_read(
                    stream,
                    buf.as_mut_ptr() as *mut core::ffi::c_void,
                    buf.len() as i32,
                    200_000_000, // 200ms timeout
                )
            };
            if n < 0 {
                break;
            }
            let got = &buf[..n as usize];
            samples2.lock().unwrap().extend_from_slice(got);
            level_window.extend_from_slice(got);
            if level_window.len() >= SAMPLE_RATE / 10 {
                // RMS → dBFS → the desktop path's perceptual mapping, so the
                // recorder's bars behave the same.
                let sum: f64 =
                    level_window.iter().map(|s| (*s as f64 / 32768.0).powi(2)).sum();
                let rms = (sum / level_window.len() as f64).sqrt();
                let db = if rms > 0.0 { 20.0 * rms.log10() } else { -100.0 };
                let level = ((db + 40.0) / 32.0).clamp(0.0, 1.0).powf(0.8);
                eng.hub.broadcast(json!({"event": "voice.level", "level": level}));
                level_window.clear();
            }
        }
        unsafe {
            AAudioStream_requestStop(stream);
            AAudioStream_close(stream);
        }
    });

    Ok(Recorder { stop, samples, thread: Some(thread), path })
}

impl Recorder {
    /// Stop, write the WAV, and hand back (samples-len-derived) duration + data.
    pub fn finish(mut self) -> (PathBuf, Vec<i16>) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let samples = std::mem::take(&mut *self.samples.lock().unwrap());
        write_wav(&self.path, &samples);
        (self.path.clone(), samples)
    }
}

/// Minimal 16-bit mono PCM WAV.
fn write_wav(path: &std::path::Path, samples: &[i16]) {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&(SAMPLE_RATE as u32).to_le_bytes());
    out.extend_from_slice(&((SAMPLE_RATE * 2) as u32).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    let _ = std::fs::write(path, out);
}

// ---- playback: AMediaExtractor + AMediaCodec decode → AAudio out ---------
// Plays whatever the platform can decode (ogg/opus voice notes, mp3, m4a) —
// the Android stand-in for ffplay.

#[allow(non_camel_case_types)]
type media_status_t = i32;

#[repr(C)]
struct AMediaExtractor { _p: [u8; 0] }
#[repr(C)]
struct AMediaCodec { _p: [u8; 0] }
#[repr(C)]
struct AMediaFormat { _p: [u8; 0] }

#[repr(C)]
struct AMediaCodecBufferInfo {
    offset: i32,
    size: i32,
    presentation_time_us: i64,
    flags: u32,
}

const AMEDIACODEC_BUFFER_FLAG_END_OF_STREAM: u32 = 4;
const SEEK_PREVIOUS_SYNC: i32 = 0;

#[link(name = "mediandk")]
extern "C" {
    fn AMediaExtractor_new() -> *mut AMediaExtractor;
    fn AMediaExtractor_delete(e: *mut AMediaExtractor) -> media_status_t;
    fn AMediaExtractor_setDataSourceFd(e: *mut AMediaExtractor, fd: i32, offset: i64, length: i64) -> media_status_t;
    fn AMediaExtractor_getTrackCount(e: *mut AMediaExtractor) -> usize;
    fn AMediaExtractor_getTrackFormat(e: *mut AMediaExtractor, idx: usize) -> *mut AMediaFormat;
    fn AMediaExtractor_selectTrack(e: *mut AMediaExtractor, idx: usize) -> media_status_t;
    fn AMediaExtractor_readSampleData(e: *mut AMediaExtractor, buf: *mut u8, cap: usize) -> isize;
    fn AMediaExtractor_getSampleTime(e: *mut AMediaExtractor) -> i64;
    fn AMediaExtractor_advance(e: *mut AMediaExtractor) -> bool;
    fn AMediaExtractor_seekTo(e: *mut AMediaExtractor, us: i64, mode: i32) -> media_status_t;

    fn AMediaFormat_delete(f: *mut AMediaFormat) -> media_status_t;
    fn AMediaFormat_getString(f: *mut AMediaFormat, name: *const core::ffi::c_char, out: *mut *const core::ffi::c_char) -> bool;
    fn AMediaFormat_getInt32(f: *mut AMediaFormat, name: *const core::ffi::c_char, out: *mut i32) -> bool;

    fn AMediaCodec_createDecoderByType(mime: *const core::ffi::c_char) -> *mut AMediaCodec;
    fn AMediaCodec_delete(c: *mut AMediaCodec) -> media_status_t;
    fn AMediaCodec_configure(c: *mut AMediaCodec, fmt: *mut AMediaFormat, surface: *mut core::ffi::c_void, crypto: *mut core::ffi::c_void, flags: u32) -> media_status_t;
    fn AMediaCodec_start(c: *mut AMediaCodec) -> media_status_t;
    fn AMediaCodec_stop(c: *mut AMediaCodec) -> media_status_t;
    fn AMediaCodec_dequeueInputBuffer(c: *mut AMediaCodec, timeout_us: i64) -> isize;
    fn AMediaCodec_getInputBuffer(c: *mut AMediaCodec, idx: usize, out_size: *mut usize) -> *mut u8;
    fn AMediaCodec_queueInputBuffer(c: *mut AMediaCodec, idx: usize, offset: i64, size: usize, time_us: u64, flags: u32) -> media_status_t;
    fn AMediaCodec_dequeueOutputBuffer(c: *mut AMediaCodec, info: *mut AMediaCodecBufferInfo, timeout_us: i64) -> isize;
    fn AMediaCodec_getOutputBuffer(c: *mut AMediaCodec, idx: usize, out_size: *mut usize) -> *mut u8;
    fn AMediaCodec_releaseOutputBuffer(c: *mut AMediaCodec, idx: usize, render: bool) -> media_status_t;
    fn AMediaCodec_getOutputFormat(c: *mut AMediaCodec) -> *mut AMediaFormat;
}

fn open_output(rate: i32, channels: i32) -> Option<*mut AAudioStream> {
    unsafe {
        let mut builder: *mut AAudioStreamBuilder = core::ptr::null_mut();
        if AAudio_createStreamBuilder(&mut builder) != 0 || builder.is_null() {
            return None;
        }
        // Direction OUTPUT is 0.
        AAudioStreamBuilder_setDirection(builder, 0);
        AAudioStreamBuilder_setSampleRate(builder, rate);
        AAudioStreamBuilder_setChannelCount(builder, channels);
        AAudioStreamBuilder_setFormat(builder, AAUDIO_FORMAT_PCM_I16);
        let mut stream: *mut AAudioStream = core::ptr::null_mut();
        let rc = AAudioStreamBuilder_openStream(builder, &mut stream);
        AAudioStreamBuilder_delete(builder);
        if rc != 0 || stream.is_null() {
            return None;
        }
        if AAudioStream_requestStart(stream) != 0 {
            AAudioStream_close(stream);
            return None;
        }
        Some(stream)
    }
}

#[link(name = "aaudio")]
extern "C" {
    fn AAudioStream_write(stream: *mut AAudioStream, buffer: *const core::ffi::c_void, num_frames: i32, timeout_nanos: i64) -> aaudio_result_t;
}

pub struct Playback {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// How far the writer has reached, in microseconds from the clip's start
    /// (the seek included). The UI polls this to paint the waveform.
    pos_us: Arc<AtomicU64>,
    /// Set once the loop has run off the end rather than been stopped.
    done: Arc<AtomicBool>,
}

impl Playback {
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }

    pub fn position(&self) -> f64 {
        self.pos_us.load(Ordering::Relaxed) as f64 / 1_000_000.0
    }

    pub fn finished(&self) -> bool {
        self.done.load(Ordering::Relaxed)
    }
}

pub fn play(file: &std::path::Path, seek: f64) -> anyhow::Result<Playback> {
    use std::os::fd::IntoRawFd;
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    // The playhead starts where the seek put it; each written block moves it.
    let pos_us = Arc::new(AtomicU64::new((seek.max(0.0) * 1_000_000.0) as u64));
    let done = Arc::new(AtomicBool::new(false));
    let (pos2, done2) = (pos_us.clone(), done.clone());
    // Our own recordings (and their composer preview) are plain WAV, which
    // needs no codec round trip — and a sample-exact seek comes free. Any
    // other RIFF flavour falls through to the extractor.
    if is_wav(file) {
        let bytes = std::fs::read(file)?;
        if wav_pcm16(&bytes).is_some() {
            let thread = std::thread::spawn(move || {
                wav_loop(&bytes, seek, &stop2, &pos2);
                done2.store(true, Ordering::Relaxed);
            });
            return Ok(Playback { stop, thread: Some(thread), pos_us, done });
        }
    }
    let f = std::fs::File::open(file)?;
    let len = f.metadata()?.len() as i64;
    let fd = f.into_raw_fd();
    let thread = std::thread::spawn(move || {
        decode_loop(fd, len, seek, &stop2, &pos2);
        done2.store(true, Ordering::Relaxed);
        unsafe { libc::close(fd) };
    });
    Ok(Playback { stop, thread: Some(thread), pos_us, done })
}

fn is_wav(file: &std::path::Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 12];
    match std::fs::File::open(file).and_then(|mut f| f.read_exact(&mut head).map(|_| head)) {
        Ok(h) => &h[..4] == b"RIFF" && &h[8..12] == b"WAVE",
        Err(_) => false,
    }
}

/// (rate, channels, data offset, data length) when the file is a 16-bit PCM
/// WAV — the shape write_wav produces. fmt + data chunks; sizes are
/// little-endian, chunks word-aligned.
fn wav_pcm16(bytes: &[u8]) -> Option<(u32, usize, usize, usize)> {
    let (mut rate, mut channels, mut ok) = (0u32, 0usize, false);
    let mut data: Option<(usize, usize)> = None;
    let mut pos = 12usize;
    while pos + 8 <= bytes.len() {
        let sz = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = pos + 8;
        match &bytes[pos..pos + 4] {
            b"fmt " if body + 16 <= bytes.len() => {
                let tag = u16::from_le_bytes(bytes[body..body + 2].try_into().unwrap());
                channels = u16::from_le_bytes(bytes[body + 2..body + 4].try_into().unwrap()) as usize;
                rate = u32::from_le_bytes(bytes[body + 4..body + 8].try_into().unwrap());
                let bits = u16::from_le_bytes(bytes[body + 14..body + 16].try_into().unwrap());
                ok = tag == 1 && bits == 16;
            }
            b"data" => data = Some((body, sz.min(bytes.len().saturating_sub(body)))),
            _ => {}
        }
        pos = body + sz + (sz & 1);
    }
    let (off, len) = data?;
    (ok && channels > 0 && rate > 0).then_some((rate, channels, off, len))
}

/// Play a 16-bit PCM WAV straight to AAudio.
fn wav_loop(bytes: &[u8], seek: f64, stop: &AtomicBool, pos_us: &AtomicU64) {
    let Some((rate, channels, off, len)) = wav_pcm16(bytes) else { return };
    let Some(out) = open_output(rate as i32, channels as i32) else { return };
    let frame_bytes = 2 * channels;
    let end = off + len - (len % frame_bytes);
    let mut p = off + ((seek.max(0.0) * rate as f64) as usize) * frame_bytes;
    let block_frames = (rate as usize / 50).max(1); // 20ms per write
    let mut block = vec![0i16; block_frames * channels];
    while p < end && !stop.load(Ordering::Relaxed) {
        let frames = ((end - p) / frame_bytes).min(block_frames);
        for (i, b) in block.iter_mut().take(frames * channels).enumerate() {
            *b = i16::from_le_bytes([bytes[p + i * 2], bytes[p + i * 2 + 1]]);
        }
        unsafe {
            AAudioStream_write(
                out,
                block.as_ptr() as *const core::ffi::c_void,
                frames as i32,
                500_000_000,
            );
        }
        p += frames * frame_bytes;
        // AAudioStream_write blocks on a full buffer, so frames-written is
        // within one buffer of what the ear has heard.
        let played = ((p - off) / frame_bytes) as u64;
        pos_us.store(played * 1_000_000 / rate as u64, Ordering::Relaxed);
    }
    unsafe {
        // Let the tail drain rather than clipping the last word.
        std::thread::sleep(std::time::Duration::from_millis(120));
        AAudioStream_requestStop(out);
        AAudioStream_close(out);
    }
}

fn cstr(s: &[u8]) -> *const core::ffi::c_char {
    s.as_ptr() as *const core::ffi::c_char
}

fn decode_loop(fd: i32, len: i64, seek: f64, stop: &AtomicBool, pos_us: &AtomicU64) {
    unsafe {
        let ex = AMediaExtractor_new();
        if ex.is_null() {
            return;
        }
        if AMediaExtractor_setDataSourceFd(ex, fd, 0, len) != 0 {
            AMediaExtractor_delete(ex);
            return;
        }
        // First audio track wins.
        let mut codec: *mut AMediaCodec = core::ptr::null_mut();
        let mut rate = 48_000i32;
        let mut channels = 1i32;
        for i in 0..AMediaExtractor_getTrackCount(ex) {
            let fmt = AMediaExtractor_getTrackFormat(ex, i);
            if fmt.is_null() {
                continue;
            }
            let mut mime: *const core::ffi::c_char = core::ptr::null();
            let has = AMediaFormat_getString(fmt, cstr(b"mime\0"), &mut mime);
            let mime_str = if has && !mime.is_null() {
                std::ffi::CStr::from_ptr(mime).to_string_lossy().into_owned()
            } else {
                String::new()
            };
            if has && mime_str.starts_with("audio/") {
                AMediaFormat_getInt32(fmt, cstr(b"sample-rate\0"), &mut rate);
                AMediaFormat_getInt32(fmt, cstr(b"channel-count\0"), &mut channels);
                let c = AMediaCodec_createDecoderByType(mime);
                if !c.is_null()
                    && AMediaCodec_configure(c, fmt, core::ptr::null_mut(), core::ptr::null_mut(), 0) == 0
                    && AMediaCodec_start(c) == 0
                {
                    AMediaExtractor_selectTrack(ex, i);
                    codec = c;
                } else if !c.is_null() {
                    AMediaCodec_delete(c);
                }
            }
            AMediaFormat_delete(fmt);
            if !codec.is_null() {
                break;
            }
        }
        if codec.is_null() {
            AMediaExtractor_delete(ex);
            return;
        }
        if seek > 0.0 {
            AMediaExtractor_seekTo(ex, (seek * 1_000_000.0) as i64, SEEK_PREVIOUS_SYNC);
        }
        let mut out_stream: *mut AAudioStream = core::ptr::null_mut();
        let mut input_done = false;
        let mut info = AMediaCodecBufferInfo { offset: 0, size: 0, presentation_time_us: 0, flags: 0 };
        loop {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            if !input_done {
                let idx = AMediaCodec_dequeueInputBuffer(codec, 10_000);
                if idx >= 0 {
                    let mut cap = 0usize;
                    let buf = AMediaCodec_getInputBuffer(codec, idx as usize, &mut cap);
                    let n = if buf.is_null() { -1 } else { AMediaExtractor_readSampleData(ex, buf, cap) };
                    if n < 0 {
                        AMediaCodec_queueInputBuffer(codec, idx as usize, 0, 0, 0, AMEDIACODEC_BUFFER_FLAG_END_OF_STREAM);
                        input_done = true;
                    } else {
                        let t = AMediaExtractor_getSampleTime(ex);
                        AMediaCodec_queueInputBuffer(codec, idx as usize, 0, n as usize, t.max(0) as u64, 0);
                        AMediaExtractor_advance(ex);
                    }
                }
            }
            let oidx = AMediaCodec_dequeueOutputBuffer(codec, &mut info, 10_000);
            if oidx >= 0 {
                if out_stream.is_null() {
                    // The decoder's real output format can differ from the track's.
                    let of = AMediaCodec_getOutputFormat(codec);
                    if !of.is_null() {
                        AMediaFormat_getInt32(of, cstr(b"sample-rate\0"), &mut rate);
                        AMediaFormat_getInt32(of, cstr(b"channel-count\0"), &mut channels);
                        AMediaFormat_delete(of);
                    }
                    out_stream = match open_output(rate, channels.max(1)) {
                        Some(s) => s,
                        None => break,
                    };
                }
                let mut cap = 0usize;
                let buf = AMediaCodec_getOutputBuffer(codec, oidx as usize, &mut cap);
                if !buf.is_null() && info.size > 0 {
                    let frames = info.size / 2 / channels.max(1);
                    let data = buf.add(info.offset as usize) as *const core::ffi::c_void;
                    AAudioStream_write(out_stream, data, frames, 500_000_000);
                    // The extractor's timestamps run from the file's start, so
                    // this already accounts for the seek.
                    pos_us.store(info.presentation_time_us.max(0) as u64, Ordering::Relaxed);
                }
                let eos = info.flags & AMEDIACODEC_BUFFER_FLAG_END_OF_STREAM != 0;
                AMediaCodec_releaseOutputBuffer(codec, oidx as usize, false);
                if eos {
                    break;
                }
            }
        }
        if !out_stream.is_null() {
            // Let the tail drain rather than clipping the last word.
            std::thread::sleep(std::time::Duration::from_millis(120));
            AAudioStream_requestStop(out_stream);
            AAudioStream_close(out_stream);
        }
        AMediaCodec_stop(codec);
        AMediaCodec_delete(codec);
        AMediaExtractor_delete(ex);
    }
}
