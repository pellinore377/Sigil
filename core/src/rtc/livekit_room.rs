//! One LiveKit session: connect (optionally E2EE), publish mic/camera/screen, pump remote video into shm.
use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context;
use futures_util::StreamExt;
use livekit::e2ee::key_provider::{KeyDerivationAlgorithm, KeyProvider, KeyProviderOptions};
use livekit::e2ee::{E2eeOptions, EncryptionType};
use livekit::options::{TrackPublishOptions, VideoEncoding};
use livekit::prelude::*;
use livekit::webrtc::video_frame::VideoBuffer;
use livekit::webrtc::native::yuv_helper;
use livekit::webrtc::video_source::native::NativeVideoSource;
use livekit::webrtc::video_source::{RtcVideoSource, VideoResolution};
use livekit::webrtc::video_stream::native::NativeVideoStream;
use livekit::{AudioProcessingOptions, PlatformAudio};
use parking_lot::Mutex;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;
use tracing::{debug, info, warn};

use super::camera::CameraHandle;
use super::screen::ScreenHandle;
use super::shm::ShmWriter;

#[derive(Debug, Clone)]
pub enum SessionEvent {
    ParticipantJoined { identity: String },
    ParticipantLeft { identity: String },
    TrackAdded { identity: String, kind: &'static str, key: String, path: String, width: u32, height: u32 },
    TrackRemoved { identity: String, key: String },
    Muted { identity: String, kind: &'static str, muted: bool },
    /// (identity, audio level 0..1) for everyone LiveKit currently counts as speaking.
    Speaking { levels: Vec<(String, f32)> },
    Quality { identity: String, quality: String },
    Reconnecting,
    Reconnected,
    Disconnected { reason: String },
    Reaction { identity: String, emoji: String },
}

pub struct LkSession {
    pub room: Arc<Room>,
    pub identity: String,
    pub key_provider: Option<KeyProvider>,
    platform_audio: Mutex<Option<PlatformAudio>>,
    audio_pub: LocalTrackPublication,
    video_source: NativeVideoSource,
    video_pub: Mutex<Option<LocalTrackPublication>>,
    camera: Mutex<Option<CameraHandle>>,
    pub preview: Arc<Mutex<Option<ShmWriter>>>,
    screen_source: Mutex<Option<NativeVideoSource>>,
    screen_pub: Mutex<Option<LocalTrackPublication>>,
    screen: Mutex<Option<ScreenHandle>>,
    pub screen_preview: Arc<Mutex<Option<ShmWriter>>>,
    pumps: Arc<Mutex<HashMap<String, AbortHandle>>>,
    event_task: Mutex<Option<AbortHandle>>,
}

impl LkSession {
    /// Send an in-call reaction to everyone, over a reliable data packet.
    pub async fn send_reaction(&self, emoji: &str) -> anyhow::Result<()> {
        let body = serde_json::json!({ "type": "reaction", "emoji": emoji }).to_string();
        self.room
            .local_participant()
            .publish_data(livekit::DataPacket {
                payload: body.into_bytes(),
                reliable: true,
                ..Default::default()
            })
            .await?;
        Ok(())
    }
}

fn source_kind(src: TrackSource) -> &'static str {
    match src { TrackSource::Screenshare => "screen", TrackSource::Microphone => "mic", _ => "camera" }
}

impl LkSession {
    pub async fn connect(url: &str, jwt: &str, encrypted: bool, audio_route: (Option<String>, Option<String>), tx: mpsc::UnboundedSender<SessionEvent>) -> anyhow::Result<Arc<Self>> {
        // Follow the system default: a pinned device that is powered off records silence.
        let _ = audio_route;
        std::env::remove_var("PULSE_SOURCE");
        std::env::remove_var("PULSE_SINK");
        let key_provider = encrypted.then(|| KeyProvider::new(KeyProviderOptions {
            key_derivation_algorithm: KeyDerivationAlgorithm::HKDF,
            ratchet_window_size: 8,
            failure_tolerance: 10,
            key_ring_size: 16,
            ..KeyProviderOptions::default()
        }));
        let mut opts = RoomOptions::default();
        // adaptive_stream waits for a renderer to report dimensions; the shm pump never does.
        opts.adaptive_stream = false;
        opts.dynacast = true;
        if let Some(kp) = &key_provider {
            opts.encryption = Some(E2eeOptions { encryption_type: EncryptionType::Gcm, key_provider: kp.clone() });
        }
        let (room, events) = Room::connect(url, jwt, opts).await.context("livekit connect")?;
        let room = Arc::new(room);

        // Mic via the platform ADM (AEC/NS/AGC); playout stays on the ADM too.
        let platform_audio = PlatformAudio::new().map_err(|e| anyhow::anyhow!("PlatformAudio: {e}"))?;
        if let Err(e) = platform_audio.configure_audio_processing(AudioProcessingOptions::default()) { warn!("audio processing config: {e}"); }
        let mic = LocalAudioTrack::create_audio_track("mic", platform_audio.rtc_source());
        let audio_pub = room.local_participant().publish_track(LocalTrack::Audio(mic), TrackPublishOptions { dtx: false, red: false, simulcast: false, source: TrackSource::Microphone, ..Default::default() }).await.context("publish mic")?;

        let video_source = NativeVideoSource::new(VideoResolution { width: 1280, height: 720 }, false);
        let identity = room.local_participant().identity().as_str().to_owned();
        debug!("rtc: livekit connected as {identity}");

        let this = Arc::new(LkSession {
            room: room.clone(), identity, key_provider,
            platform_audio: Mutex::new(Some(platform_audio)), audio_pub,
            video_source, video_pub: Mutex::new(None), camera: Mutex::new(None), preview: Arc::new(Mutex::new(None)),
            screen_source: Mutex::new(None), screen_pub: Mutex::new(None), screen: Mutex::new(None), screen_preview: Arc::new(Mutex::new(None)),
            pumps: Arc::new(Mutex::new(HashMap::new())), event_task: Mutex::new(None),
        });
        let task = tokio::spawn(event_loop(room, events, tx, this.pumps.clone()));
        *this.event_task.lock() = Some(task.abort_handle());
        Ok(this)
    }

    pub fn set_key(&self, identity: &str, index: i32, key: Vec<u8>) {
        if let Some(kp) = &self.key_provider {
            kp.set_key(&ParticipantIdentity::from(identity.to_string()), index, key);
        }
    }

    /// Apply a peer's key to every connected participant of that Matrix user.
    pub fn set_key_for_user(&self, user_id: &str, index: i32, key: &[u8]) -> usize {
        let Some(kp) = &self.key_provider else { return 0 };
        let prefix = format!("{user_id}:");
        let mut n = 0;
        for (identity, _) in self.room.remote_participants() {
            if identity.as_str().starts_with(&prefix) || identity.as_str() == user_id {
                kp.set_key(&identity, index, key.to_vec());
                n += 1;
            }
        }
        n
    }

    pub fn set_mic_muted(&self, muted: bool) {
        if muted { self.audio_pub.mute() } else { self.audio_pub.unmute() }
    }
    pub fn mic_muted(&self) -> bool { self.audio_pub.is_muted() }
    pub fn camera_on(&self) -> bool { self.camera.lock().is_some() }
    pub fn screen_on(&self) -> bool { self.screen.lock().is_some() }

    pub async fn set_camera(self: &Arc<Self>, on: bool, device: &str, on_error: impl Fn(String) + Send + 'static) -> anyhow::Result<()> {
        if !on {
            if let Some(h) = self.camera.lock().take() { h.stop(); }
            *self.preview.lock() = None;
            if let Some(p) = self.video_pub.lock().as_ref() { p.mute(); }
            return Ok(());
        }
        if self.camera.lock().is_some() { return Ok(()); }
        if self.video_pub.lock().is_none() {
            let track = LocalVideoTrack::create_video_track("camera", RtcVideoSource::Native(self.video_source.clone()));
            let publication = self.room.local_participant().publish_track(LocalTrack::Video(track), TrackPublishOptions { simulcast: false, source: TrackSource::Camera, video_encoding: Some(VideoEncoding { max_bitrate: 2_000_000, max_framerate: 30.0 }), ..Default::default() }).await.context("publish camera")?;
            *self.video_pub.lock() = Some(publication);
        } else if let Some(p) = self.video_pub.lock().as_ref() { p.unmute(); }
        *self.preview.lock() = Some(ShmWriter::create("local-camera", 1280, 720)?);
        let h = super::camera::start(device, 1280, 720, 30, self.video_source.clone(), self.preview.clone(), on_error);
        *self.camera.lock() = Some(h);
        Ok(())
    }

    pub async fn set_screen(self: &Arc<Self>, on: bool, on_error: impl Fn(String) + Send + 'static) -> anyhow::Result<()> {
        if !on {
            if let Some(h) = self.screen.lock().take() { h.stop(); }
            *self.screen_preview.lock() = None;
            let pubn = self.screen_pub.lock().take();
            if let Some(p) = pubn { let _ = self.room.local_participant().unpublish_track(&p.sid()).await; }
            *self.screen_source.lock() = None;
            return Ok(());
        }
        if self.screen.lock().is_some() { return Ok(()); }
        let source = NativeVideoSource::new(VideoResolution { width: 1920, height: 1080 }, true);
        *self.screen_preview.lock() = Some(ShmWriter::create("local-screen", 1920, 1080)?);
        let handle = super::screen::start(source.clone(), self.screen_preview.clone(), on_error).await?;
        let track = LocalVideoTrack::create_video_track("screen", RtcVideoSource::Native(source.clone()));
        let publication = self.room.local_participant().publish_track(LocalTrack::Video(track), TrackPublishOptions { simulcast: false, source: TrackSource::Screenshare, video_encoding: Some(VideoEncoding { max_bitrate: 4_000_000, max_framerate: 30.0 }), ..Default::default() }).await.context("publish screen")?;
        *self.screen_source.lock() = Some(source);
        *self.screen_pub.lock() = Some(publication);
        *self.screen.lock() = Some(handle);
        Ok(())
    }

    pub fn audio(&self) -> parking_lot::MutexGuard<'_, Option<PlatformAudio>> { self.platform_audio.lock() }

    pub async fn disconnect(self: &Arc<Self>) {
        if let Some(h) = self.camera.lock().take() { h.stop(); }
        if let Some(h) = self.screen.lock().take() { h.stop(); }
        *self.preview.lock() = None;
        *self.screen_preview.lock() = None;
        for (_, h) in self.pumps.lock().drain() { h.abort(); }
        let _ = self.room.close().await;
        if let Some(t) = self.event_task.lock().take() { t.abort(); }
        if let Some(pa) = self.platform_audio.lock().take() { pa.release(); }
    }
}

async fn event_loop(room: Arc<Room>, mut events: mpsc::UnboundedReceiver<RoomEvent>, tx: mpsc::UnboundedSender<SessionEvent>, pumps: Arc<Mutex<HashMap<String, AbortHandle>>>) {
    for (identity, _) in room.remote_participants() {
        let _ = tx.send(SessionEvent::ParticipantJoined { identity: identity.as_str().to_owned() });
    }
    while let Some(ev) = events.recv().await {
        match ev {
            RoomEvent::ParticipantConnected(p) => { let _ = tx.send(SessionEvent::ParticipantJoined { identity: p.identity().as_str().to_owned() }); }
            RoomEvent::ParticipantDisconnected(p) => { let _ = tx.send(SessionEvent::ParticipantLeft { identity: p.identity().as_str().to_owned() }); }
            RoomEvent::TrackSubscribed { track, publication, participant } => {
                let identity = participant.identity().as_str().to_owned();
                let kind = source_kind(publication.source());
                match track {
                    RemoteTrack::Video(v) => {
                        let sid = v.sid().to_string();
                        let key = format!("{}-{}", sid, kind);
                        let tx2 = tx.clone();
                        let id2 = identity.clone();
                        let key2 = key.clone();
                        let rtc_track = v.rtc_track();
                        let h = tokio::spawn(async move {
                            let mut writer = match ShmWriter::create(&key2, 1280, 720) { Ok(w) => w, Err(e) => { warn!("shm create failed: {e}"); return } };
                            let mut stream = NativeVideoStream::new(rtc_track);
                            let mut announced = false;
                            let mut n: u64 = 0;
                            while let Some(frame) = stream.next().await {
                                let buf = frame.buffer.to_i420();
                                let (w, h) = (buf.width(), buf.height());
                                if w == 0 || h == 0 { continue; }
                                match writer.ensure_capacity(w, h) {
                                    Ok(true) => { announced = false; }
                                    Ok(false) => {}
                                    Err(e) => { warn!("shm grow failed: {e}"); continue; }
                                }
                                let (sy, su, sv) = buf.strides();
                                let (y, u, vv) = buf.data();
                                writer.write_with(w, h, false, |dst, stride| yuv_helper::i420_to_abgr(y, sy, u, su, vv, sv, dst, stride as u32, w as i32, h as i32));
                                if !announced {
                                    announced = true;
                                    let _ = tx2.send(SessionEvent::TrackAdded { identity: id2.clone(), kind, key: key2.clone(), path: writer.path().to_string_lossy().into_owned(), width: w, height: h });
                                }
                                n += 1;
                                if n % 600 == 0 { info!("rtc: {key2} {n} frames"); }
                            }
                            info!("rtc: video stream ended for {key2}");
                        });
                        pumps.lock().insert(sid, h.abort_handle());
                        let _ = kind;
                    }
                    RemoteTrack::Audio(_) => {
                        // Played by the platform ADM.
                    }
                }
            }
            RoomEvent::TrackUnsubscribed { track, publication, participant } => {
                let identity = participant.identity().as_str().to_owned();
                if let RemoteTrack::Video(v) = track {
                    let sid = v.sid().to_string();
                    if let Some(h) = pumps.lock().remove(&sid) { h.abort(); }
                    let _ = tx.send(SessionEvent::TrackRemoved { identity, key: format!("{}-{}", sid, source_kind(publication.source())) });
                }
            }
            RoomEvent::TrackMuted { participant, publication } => {
                let _ = tx.send(SessionEvent::Muted { identity: participant.identity().as_str().to_owned(), kind: source_kind(publication.source()), muted: true });
            }
            RoomEvent::TrackUnmuted { participant, publication } => {
                let _ = tx.send(SessionEvent::Muted { identity: participant.identity().as_str().to_owned(), kind: source_kind(publication.source()), muted: false });
            }
            RoomEvent::TrackUnpublished { publication, participant } => {
                if publication.source() == TrackSource::Screenshare {
                    let _ = tx.send(SessionEvent::Muted { identity: participant.identity().as_str().to_owned(), kind: "screen", muted: true });
                }
            }
            RoomEvent::ActiveSpeakersChanged { speakers } => {
                let levels = speakers.iter()
                    .map(|p| (p.identity().as_str().to_owned(), p.audio_level()))
                    .collect();
                let _ = tx.send(SessionEvent::Speaking { levels });
            }
            RoomEvent::ConnectionQualityChanged { quality, participant } => {
                let q = match quality { ConnectionQuality::Excellent => "excellent", ConnectionQuality::Good => "good", ConnectionQuality::Poor => "poor", ConnectionQuality::Lost => "lost", #[allow(unreachable_patterns)] _ => "unknown" };
                let _ = tx.send(SessionEvent::Quality { identity: participant.identity().as_str().to_owned(), quality: q.into() });
            }
            // Reactions ride the LiveKit data channel, not the Matrix room: ephemeral.
            RoomEvent::DataReceived { payload, participant, topic, .. } => {
                // `debug`: the packet carries participant identities and arbitrary text.
                tracing::debug!(
                    target: "sigil::rtc::capture",
                    "data packet: topic={:?} from={:?} payload={}",
                    topic,
                    participant.as_ref().map(|p| p.identity().as_str().to_owned()),
                    String::from_utf8_lossy(&payload)
                );
                let Ok(v) = serde_json::from_slice::<serde_json::Value>(&payload) else { continue };
                if v.get("type").and_then(|t| t.as_str()) != Some("reaction") { continue }
                let Some(emoji) = v.get("emoji").and_then(|e| e.as_str()) else { continue };
                let identity = participant.map(|p| p.identity().as_str().to_owned()).unwrap_or_default();
                let _ = tx.send(SessionEvent::Reaction { identity, emoji: emoji.to_owned() });
            }
            RoomEvent::Reconnecting => { let _ = tx.send(SessionEvent::Reconnecting); }
            RoomEvent::Reconnected => { let _ = tx.send(SessionEvent::Reconnected); }
            RoomEvent::Disconnected { reason } => { let _ = tx.send(SessionEvent::Disconnected { reason: format!("{reason:?}") }); break; }
            _ => {}
        }
    }
}
