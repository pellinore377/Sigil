//! Sound in and out for calls: the microphone as 20 ms frames of 48 kHz
//! mono, and a speaker fed from a mix. `cpal` talks to the device; whatever
//! rate and channel count it wants is converted here. Without any device
//! (a test machine, a headless box) or with `SIGIL_FAKE_AUDIO` set, a tone
//! stands in for the microphone and the speaker only measures what it is
//! given, so the whole path from one app to another can be checked.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

pub const RATE: u32 = 48_000;
/// 20 ms at 48 kHz.
pub const FRAME: usize = 960;

#[derive(Default, Clone, Debug)]
pub struct DeviceList {
    pub mics: Vec<(String, String)>,
    pub speakers: Vec<(String, String)>,
}

pub fn fake() -> bool {
    std::env::var_os("SIGIL_FAKE_AUDIO").is_some()
}

/// Every microphone and speaker the host knows, as (id, label). The id is
/// the device's name; cpal has nothing more stable.
pub fn devices() -> DeviceList {
    if fake() {
        return DeviceList {
            mics: vec![("fake".into(), "Test tone".into())],
            speakers: vec![("fake".into(), "Silent speaker".into())],
        };
    }
    let host = cpal::default_host();
    let mut out = DeviceList::default();
    if let Ok(devs) = host.input_devices() {
        for d in devs {
            if let Ok(n) = d.name() {
                out.mics.push((n.clone(), n));
            }
        }
    }
    if let Ok(devs) = host.output_devices() {
        for d in devs {
            if let Ok(n) = d.name() {
                out.speakers.push((n.clone(), n));
            }
        }
    }
    out
}

/// Linear resampling of interleaved frames to mono at `RATE`.
fn to_mono_48k(input: &[f32], channels: usize, rate: u32, out: &mut Vec<f32>) {
    let frames = input.len() / channels.max(1);
    if frames == 0 {
        return;
    }
    let ratio = rate as f64 / RATE as f64;
    let n_out = ((frames as f64) / ratio).floor() as usize;
    for i in 0..n_out {
        let pos = i as f64 * ratio;
        let j = pos.floor() as usize;
        let f = (pos - j as f64) as f32;
        let a = mono_at(input, channels, j.min(frames - 1));
        let b = mono_at(input, channels, (j + 1).min(frames - 1));
        out.push(a + (b - a) * f);
    }
}

fn mono_at(input: &[f32], channels: usize, frame: usize) -> f32 {
    let c = channels.max(1);
    let s = frame * c;
    input[s..s + c].iter().sum::<f32>() / c as f32
}

/// The microphone, delivering `FRAME`-sized frames on `tx`.
pub struct Capture {
    _stream: Option<cpal::Stream>,
    stop: Arc<AtomicBool>,
}

impl Capture {
    pub fn start(device: Option<&str>, tx: Sender<Vec<f32>>) -> anyhow::Result<Capture> {
        let stop = Arc::new(AtomicBool::new(false));
        if fake() {
            let stop2 = stop.clone();
            std::thread::Builder::new()
                .name("sigil-fake-mic".into())
                .spawn(move || {
                    let mut phase = 0.0f32;
                    let step = 440.0 * std::f32::consts::TAU / RATE as f32;
                    let mut next = std::time::Instant::now();
                    while !stop2.load(Ordering::Relaxed) {
                        let frame: Vec<f32> = (0..FRAME)
                            .map(|_| {
                                phase += step;
                                (phase.sin()) * 0.1
                            })
                            .collect();
                        if tx.send(frame).is_err() {
                            break;
                        }
                        next += Duration::from_millis(20);
                        let now = std::time::Instant::now();
                        if next > now {
                            std::thread::sleep(next - now);
                        }
                    }
                })?;
            return Ok(Capture {
                _stream: None,
                stop,
            });
        }
        let host = cpal::default_host();
        let dev = match device {
            Some(name) => host
                .input_devices()?
                .find(|d| d.name().map(|n| n == name).unwrap_or(false))
                .or_else(|| host.default_input_device()),
            None => host.default_input_device(),
        }
        .ok_or_else(|| anyhow::anyhow!("no microphone"))?;
        let config = dev.default_input_config()?;
        let channels = config.channels() as usize;
        let rate = config.sample_rate().0;
        let pending = Arc::new(Mutex::new(Vec::<f32>::new()));
        let err = |e| tracing::warn!("microphone: {e}");
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => {
                let pending = pending.clone();
                dev.build_input_stream(
                    &config.into(),
                    move |data: &[f32], _| feed(&pending, data, channels, rate, &tx),
                    err,
                    None,
                )?
            }
            cpal::SampleFormat::I16 => {
                let pending = pending.clone();
                dev.build_input_stream(
                    &config.into(),
                    move |data: &[i16], _| {
                        let f: Vec<f32> = data.iter().map(|s| *s as f32 / 32768.0).collect();
                        feed(&pending, &f, channels, rate, &tx)
                    },
                    err,
                    None,
                )?
            }
            other => anyhow::bail!("unsupported microphone format {other:?}"),
        };
        stream.play()?;
        Ok(Capture {
            _stream: Some(stream),
            stop,
        })
    }
}

fn feed(
    pending: &Mutex<Vec<f32>>,
    data: &[f32],
    channels: usize,
    rate: u32,
    tx: &Sender<Vec<f32>>,
) {
    let mut p = pending.lock().unwrap();
    to_mono_48k(data, channels, rate, &mut p);
    while p.len() >= FRAME {
        let frame: Vec<f32> = p.drain(..FRAME).collect();
        let _ = tx.send(frame);
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// The speaker: a mix buffer at 48 kHz mono that the device drains, and
/// the loudness of what went through it.
pub struct Playback {
    _stream: Option<cpal::Stream>,
    mix: Arc<Mutex<VecDeque<f32>>>,
    level: Arc<Mutex<f32>>,
}

impl Playback {
    pub fn start(device: Option<&str>) -> anyhow::Result<Playback> {
        let mix = Arc::new(Mutex::new(VecDeque::<f32>::new()));
        let level = Arc::new(Mutex::new(0.0f32));
        if fake() {
            return Ok(Playback {
                _stream: None,
                mix,
                level,
            });
        }
        let host = cpal::default_host();
        let dev = match device {
            Some(name) => host
                .output_devices()?
                .find(|d| d.name().map(|n| n == name).unwrap_or(false))
                .or_else(|| host.default_output_device()),
            None => host.default_output_device(),
        }
        .ok_or_else(|| anyhow::anyhow!("no speaker"))?;
        let config = dev.default_output_config()?;
        let channels = config.channels() as usize;
        let rate = config.sample_rate().0;
        let ratio = RATE as f64 / rate as f64;
        let src = mix.clone();
        let err = |e| tracing::warn!("speaker: {e}");
        let mut carry = 0.0f64;
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => dev.build_output_stream(
                &config.into(),
                move |out: &mut [f32], _| {
                    let mut q = src.lock().unwrap();
                    for frame in out.chunks_mut(channels) {
                        carry += ratio;
                        let mut s = 0.0;
                        while carry >= 1.0 {
                            s = q.pop_front().unwrap_or(0.0);
                            carry -= 1.0;
                        }
                        for v in frame.iter_mut() {
                            *v = s;
                        }
                    }
                },
                err,
                None,
            )?,
            cpal::SampleFormat::I16 => dev.build_output_stream(
                &config.into(),
                move |out: &mut [i16], _| {
                    let mut q = src.lock().unwrap();
                    for frame in out.chunks_mut(channels) {
                        carry += ratio;
                        let mut s = 0.0;
                        while carry >= 1.0 {
                            s = q.pop_front().unwrap_or(0.0);
                            carry -= 1.0;
                        }
                        for v in frame.iter_mut() {
                            *v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
                        }
                    }
                },
                err,
                None,
            )?,
            other => anyhow::bail!("unsupported speaker format {other:?}"),
        };
        stream.play()?;
        Ok(Playback {
            _stream: Some(stream),
            mix,
            level,
        })
    }

    /// Add one decoded frame into the mix, at the front of what is queued:
    /// several people talking at once sum instead of queueing behind one
    /// another.
    pub fn push(&self, frame: &[f32]) {
        let rms = (frame.iter().map(|s| s * s).sum::<f32>() / frame.len().max(1) as f32).sqrt();
        {
            let mut l = self.level.lock().unwrap();
            *l = l.max(rms);
        }
        let mut q = self.mix.lock().unwrap();
        // never let the queue run away when nothing drains it
        let cap = FRAME * 10;
        for (i, s) in frame.iter().enumerate() {
            if i < q.len() {
                q[i] += *s;
            } else {
                q.push_back(*s);
            }
        }
        while q.len() > cap {
            q.pop_front();
        }
        if fake() {
            // nothing drains a fake speaker: keep only what a real one would
            let keep = FRAME * 2;
            while q.len() > keep {
                q.pop_front();
            }
        }
    }

    /// The loudest thing played since the last call, then reset.
    pub fn take_level(&self) -> f32 {
        let mut l = self.level.lock().unwrap();
        let v = *l;
        *l = 0.0;
        v
    }
}

pub fn rms(frame: &[f32]) -> f32 {
    (frame.iter().map(|s| s * s).sum::<f32>() / frame.len().max(1) as f32).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_at_44100_becomes_mono_at_48000() {
        let input: Vec<f32> = (0..441 * 2)
            .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
            .collect();
        let mut out = Vec::new();
        to_mono_48k(&input, 2, 44_100, &mut out);
        assert!((out.len() as i64 - 480).abs() <= 1, "{}", out.len());
        assert!(out.iter().all(|s| (*s - 0.5).abs() < 1e-5));
    }
}
