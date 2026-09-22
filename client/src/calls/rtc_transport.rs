use super::*;
use rtc::{
    interceptor::{Registry, NackGeneratorBuilder, NackResponderBuilder},
    media_stream::MediaStreamTrack,
    peer_connection::{
        configuration::{
            media_engine::MediaEngine, setting_engine::SettingEngine, RTCConfigurationBuilder,
            RTCIceServer,
        },
        sdp::RTCSessionDescription,
    },
    rtp_transceiver::{
        rtp_sender::{RTCRtpCodec, RTCRtpCodecParameters, RTCRtpCodingParameters, RTCRtpEncodingParameters, RtpCodecKind},
        RTCRtpTransceiverDirection, RTCRtpTransceiverInit,
    },
};
use sigil_calls::{Downstream, Layout, MediaKind, Track};
use std::{
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::mpsc;
use webrtc::{
    media_stream::{
        track_local::{static_rtp::TrackLocalStaticRTP, TrackLocal},
        track_remote::{TrackRemote, TrackRemoteEvent},
    },
    peer_connection::{
        PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCIceGatheringState,
        RTCPeerConnectionState,
    },
};
#[path = "encoder_feedback.rs"]
mod encoder_feedback;
pub use encoder_feedback::Uplink;
#[path = "packet_order.rs"]
mod packet_order;
#[path = "ready_frames.rs"]
mod ready_frames;
#[cfg(test)]
#[path = "rtc_tests.rs"]
mod tests;
#[path = "turn_stream.rs"]
mod turn_stream;

struct Packet {
    ssrc: u32,
    sequence: u16,
    timestamp: u32,
    marker: bool,
    payload: Vec<u8>,
}
type KeyRequests = Arc<std::sync::Mutex<std::collections::BTreeSet<u32>>>;
struct Handler {
    gathered: mpsc::Sender<()>,
    packets: mpsc::Sender<Packet>,
    connection: Arc<AtomicU8>,
    key_requests: KeyRequests,
}
#[async_trait::async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        if state == RTCIceGatheringState::Complete {
            let _ = self.gathered.try_send(());
        }
    }
    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        self.connection.store(
            match state {
                RTCPeerConnectionState::Connected => 1,
                RTCPeerConnectionState::Disconnected => 2,
                RTCPeerConnectionState::Failed | RTCPeerConnectionState::Closed => 3,
                _ => 0,
            },
            Ordering::Relaxed,
        );
    }
    async fn on_track(&self, track: Arc<dyn TrackRemote>) {
        let packets = self.packets.clone();
        let key_requests = self.key_requests.clone();
        tokio::spawn(async move {
            let video = track.kind().await == RtpCodecKind::Video;
            let mut key_requested = None::<std::time::Instant>;
            let mut seen = 0u64;
            let mut refused = 0u64;
            while let Some(event) = track.poll().await {
                if let TrackRemoteEvent::OnRtpPacket(packet) = event {
                    let now = std::time::Instant::now();
                    let repair = video && key_requested.is_none_or(|at| now.duration_since(at) >= Duration::from_millis(200))
                        && key_requests.lock().is_ok_and(|mut requests| requests.remove(&packet.header.ssrc));
                    if video && (repair || key_requested.is_none()) {
                        key_requested = Some(now);
                        let request = rtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication {
                            sender_ssrc: 0, media_ssrc: packet.header.ssrc,
                        };
                        let sent = track.write_rtcp(vec![Box::new(request)]).await.is_ok();
                        if repair && sent { crate::perf::note("call rx recovery_request".into()); }
                    }
                    if packet.payload.len() <= 2048 {
                        seen += 1;
                        if packets
                            .try_send(Packet {
                                ssrc: packet.header.ssrc,
                                sequence: packet.header.sequence_number,
                                timestamp: packet.header.timestamp,
                                marker: packet.header.marker,
                                payload: packet.payload.to_vec(),
                            })
                            .is_err()
                        {
                            refused += 1;
                            if refused.is_power_of_two() {
                                crate::perf::note(format!("call rx channel_full={refused} seen={seen}"));
                            }
                        }
                    }
                }
            }
        });
    }
}
struct Transport {
    pc: Arc<dyn PeerConnection>,
    gathered: mpsc::Receiver<()>,
    connection: Arc<AtomicU8>,
    runtime: tokio::runtime::Handle,
    encoder_requests: Arc<AtomicU8>,
    uplink: Arc<encoder_feedback::Uplink>,
}
impl Drop for Transport {
    fn drop(&mut self) {
        let pc = self.pc.clone();
        self.runtime.spawn(async move {
            let _ = pc.close().await;
        });
    }
}
pub struct ReceivedFrame {
    pub sender: Id,
    pub frame: sigil_calls::Frame,
}
pub(crate) struct RtcAuthority {
    call: Id,
    lease: Id,
    roster: Id,
    created: u64,
    expires: u64,
    version: i64,
    changes: u64,
    checked: std::time::Instant,
}
pub struct RtcCall {
    transport: Transport,
    media: Media,
    roster: Id,
    send: Option<RtcSend>,
    receive: Option<RtcReceive>,
}
/// Per-frame send state. Holds no storage; authority comes from the caller's `FrameCrypto`.
pub struct RtcSend {
    uploads: Vec<Arc<TrackLocalStaticRTP>>,
    sequence: [u16; 3],
    connection: Arc<AtomicU8>,
}
/// Per-frame receive state. Holds no storage; authority comes from the caller's `FrameCrypto`.
pub struct RtcReceive {
    packets: mpsc::Receiver<Packet>,
    held: Option<Packet>,
    runtime: tokio::runtime::Handle,
    key_requests: KeyRequests,
    streams: Vec<Downstream>,
    incoming: std::collections::BTreeMap<u32, packet_order::PacketOrder<Packet>>,
    video_gaps: std::collections::BTreeSet<u32>,
    ready: ready_frames::ReadyFrames,
    camera_assembly: std::collections::BTreeMap<u32, sigil_calls::av1::Assembly>,
    assembly: sigil_calls::Assembly,
    ready_cursor: usize,
    incoming_cursor: usize,
    tally: Tally,
}
/// Seals and opens frames under whatever authority its owner holds.
pub trait FrameCrypto {
    /// Whether frames may flow now; checked before any packet is consumed.
    fn ready(&mut self) -> Result<(), Error>;
    fn seal(&mut self, kind: MediaKind, timestamp: u64, keyframe: bool, bytes: &[u8]) -> Result<Vec<u8>, Error>;
    fn open(&mut self, sender: Id, kind: MediaKind, bytes: &[u8]) -> Result<sigil_calls::Frame, Error>;
}
/// Authority read from storage on the frame path; tests and single-threaded hosts.
struct Stored<'a> {
    store: &'a mut ClientStore,
    media: &'a mut Media,
    connection: &'a AtomicU8,
    roster: Id,
    now: u64,
}
impl FrameCrypto for Stored<'_> {
    fn ready(&mut self) -> Result<(), Error> {
        if self.store.rtc_connection_check(self.media, self.roster, self.connection, self.now)? != "connected" {
            return Err(Error::Unprepared);
        }
        self.store.checked_call_media(self.media, self.now).map(|_| ())
    }
    fn seal(&mut self, kind: MediaKind, timestamp: u64, keyframe: bool, bytes: &[u8]) -> Result<Vec<u8>, Error> {
        self.store.seal_call_frame(self.media, kind, timestamp, keyframe, bytes, self.now)
    }
    fn open(&mut self, sender: Id, kind: MediaKind, bytes: &[u8]) -> Result<sigil_calls::Frame, Error> {
        self.store.open_call_frame(self.media, sender, kind, bytes, self.now)
    }
}
/// Authority published by a store owner on another thread. Frames never touch storage;
/// a lapsed deadline refuses them until the owner publishes again.
#[derive(Default)]
pub struct MediaGate {
    processor: MediaProcessor,
    until: Option<std::time::Instant>,
    renew: Option<sigil_calls::Context>,
}
impl MediaGate {
    pub fn publish(&mut self, update: MediaUpdate, lifetime: Duration) -> Result<(), Error> {
        self.processor.apply(update)?;
        self.until = Some(std::time::Instant::now() + lifetime);
        Ok(())
    }
    pub fn revoke(&mut self) {
        self.until = None;
    }
    fn live(&self, now: u64) -> Result<Id, Error> {
        if !self.until.is_some_and(|until| std::time::Instant::now() < until) || !self.processor.is_live(now) {
            return Err(Error::Unprepared);
        }
        self.processor.context().map(|c| c.call).ok_or(Error::Unprepared)
    }
}
/// Locks the gate per frame only, for microseconds of signing or verification.
pub struct GateCrypto<'a> {
    pub gate: &'a std::sync::Mutex<MediaGate>,
    pub now: u64,
}
impl GateCrypto<'_> {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, MediaGate>, Error> {
        self.gate.lock().map_err(|_| Error::Unprepared)
    }
}
impl FrameCrypto for GateCrypto<'_> {
    fn ready(&mut self) -> Result<(), Error> {
        self.lock()?.live(self.now).map(|_| ())
    }
    fn seal(&mut self, kind: MediaKind, timestamp: u64, keyframe: bool, bytes: &[u8]) -> Result<Vec<u8>, Error> {
        let mut gate = self.lock()?;
        let call = gate.live(self.now)?;
        let result = gate.processor.seal(call, kind, timestamp, keyframe, bytes, self.now);
        if matches!(result, Err(Error::Expired)) {
            gate.renew = gate.processor.context();
        }
        result
    }
    fn open(&mut self, sender: Id, kind: MediaKind, bytes: &[u8]) -> Result<sigil_calls::Frame, Error> {
        let mut gate = self.lock()?;
        let call = gate.live(self.now)?;
        gate.processor.open(call, sender, kind, bytes, self.now)
    }
}
/// Counts only, so a silent direction can be traced to the step that discards it.
#[derive(Default)]
struct Tally {
    packets: u64,
    unknown: u64,
    assembled: u64,
    rejected: u64,
    opened: u64,
    refused: u64,
    delivered: [u64; 3],
    waiting_key: u64,
    stalled: u64,
    reported: Option<crate::clock::Instant>,
}
/// An authenticated frame whose RTP sequence numbers have already been reserved.
pub struct RtcTransmission {
    track: Arc<TrackLocalStaticRTP>,
    packets: Vec<rtc::rtp::packet::Packet>,
}
impl RtcTransmission {
    pub async fn send(self) -> Result<(), Error> {
        let track = self.track;
        send_packets(self.packets, |packet| {
            let track = track.clone();
            async move {
                track
                    .write_rtp(packet)
                    .await
                    .map(|_| ())
                    .map_err(|_| Error::Unprepared)
            }
        })
        .await
    }
}
fn video_codecs() -> Vec<RTCRtpCodecParameters> {
    let codec = |mime: &str, fmtp: &str, payload_type| RTCRtpCodecParameters {
        rtp_codec: RTCRtpCodec { mime_type: mime.into(), clock_rate: 90000, channels: 0, sdp_fmtp_line: fmtp.into(), rtcp_feedback: vec![] },
        payload_type,
    };
    vec![codec("video/AV1", "profile-id=0", 41), codec("video/rtx", "apt=41", 106)]
}
/// About 26 Mbit/s: a keyframe spreads over a frame interval or so instead of leaving as one
/// burst, which shallow queues on tunnels and home routers drop wholesale. Deadlines run from
/// the first packet, so timer overshoot never accumulates.
const PACE_BYTES_PER_MS: u64 = 3300;
async fn send_packets<F, U>(
    packets: Vec<rtc::rtp::packet::Packet>,
    mut write: F,
) -> Result<(), Error>
where
    F: FnMut(rtc::rtp::packet::Packet) -> U,
    U: std::future::Future<Output = Result<(), Error>>,
{
    let started = tokio::time::Instant::now();
    let mut sent = 0u64;
    for packet in packets {
        let due = started + Duration::from_micros(sent * 1000 / PACE_BYTES_PER_MS);
        if due > tokio::time::Instant::now() + Duration::from_millis(1) {
            tokio::time::sleep_until(due).await;
        }
        sent += packet.payload.len() as u64;
        write(packet).await?;
    }
    Ok(())
}
impl RtcCall {
    /// Consume coalesced remote video recovery requests.
    pub fn take_video_requests(&self) -> u8 {
        self.transport.encoder_requests.swap(0, Ordering::Relaxed)
    }
    /// Remote recovery requests, readable without the owner of this call.
    pub fn video_requests(&self) -> Arc<AtomicU8> {
        self.transport.encoder_requests.clone()
    }
    /// Forwarder reports on the camera upload, readable without the owner of this call.
    pub fn uplink(&self) -> Arc<encoder_feedback::Uplink> {
        self.transport.uplink.clone()
    }
    /// A newly attached or reset decoder needs an authenticated keyframe.
    pub fn request_video_keyframe(&mut self, sender: Id, kind: MediaKind) {
        if let Some(receive) = self.receive.as_mut() { receive.request_video_keyframe(sender, kind); }
    }
    /// Queue readiness only; callers must still authorize through rtc_receive before delivery.
    pub fn has_pending_receive(&self) -> bool {
        self.receive.as_ref().is_some_and(RtcReceive::has_pending)
    }
    /// Hands the frame paths to other threads; the store owner keeps connection and authority.
    pub fn detach(&mut self) -> Option<(RtcSend, RtcReceive)> {
        Some((self.send.take()?, self.receive.take()?))
    }
    /// The media handle outlives the transport: a rebuild reuses it through `connect_rtc_call_with`.
    pub fn into_media(self) -> Media {
        self.media
    }
}
impl RtcSend {
    pub fn connected(&self) -> bool {
        self.connection.load(Ordering::Relaxed) == 1
    }
    /// Seals and reserves sequence numbers; transmission happens after the caller releases this.
    pub fn prepare(
        &mut self,
        crypto: &mut impl FrameCrypto,
        kind: MediaKind,
        timestamp: u64,
        keyframe: bool,
        encoded: &[u8],
    ) -> Result<RtcTransmission, Error> {
        if !self.connected() {
            return Err(Error::Unprepared);
        }
        let index = kind as usize;
        let sealed = crypto.seal(kind, timestamp, keyframe, encoded)?;
        let packets = if kind == MediaKind::Camera {
            sigil_calls::av1::packetize(&sealed, keyframe)
        } else {
            sigil_calls::packetize(kind, &sealed)
        }
        .map_err(failure)?;
        let mut wire = Vec::with_capacity(packets.len());
        for (n, payload) in packets.iter().enumerate() {
            wire.push(rtc::rtp::packet::Packet {
                header: rtc::rtp::header::Header {
                    version: 2,
                    marker: n + 1 == packets.len(),
                    payload_type: if index == 0 { 111 } else { 41 },
                    sequence_number: self.sequence[index],
                    timestamp: (timestamp.wrapping_mul(if index == 0 { 48 } else { 90 }) / 1000) as u32,
                    ssrc: index as u32 + 1,
                    ..Default::default()
                },
                payload: payload.clone().into(),
            });
            self.sequence[index] = self.sequence[index].wrapping_add(1);
        }
        Ok(RtcTransmission { track: self.uploads[index].clone(), packets: wire })
    }
}
impl RtcReceive {
    pub fn has_pending(&self) -> bool {
        self.held.is_some()
            || !self.packets.is_empty()
            || !self.ready.is_empty()
            || self.incoming.values().any(|queue| !queue.is_empty())
    }
    /// Blocks until a packet arrives or `timeout` passes; call from outside the async runtime.
    pub fn wait(&mut self, timeout: Duration) -> bool {
        if self.has_pending() {
            return true;
        }
        let packets = &mut self.packets;
        self.held = self.runtime.block_on(async { tokio::time::timeout(timeout, packets.recv()).await.ok().flatten() });
        self.held.is_some()
    }
    pub fn request_video_keyframe(&mut self, sender: Id, kind: MediaKind) {
        if kind == MediaKind::Audio { return; }
        for stream in &self.streams {
            if stream.track.sender == sender && stream.track.kind == kind {
                self.video_gaps.insert(stream.ssrc);
                if let Ok(mut requests) = self.key_requests.lock() { requests.insert(stream.ssrc); }
            }
        }
    }
    /// Assembles, authenticates and orders frames; nothing is consumed while `crypto` refuses.
    pub fn receive(&mut self, crypto: &mut impl FrameCrypto) -> Result<Vec<ReceivedFrame>, Error> {
        if let Err(error) = crypto.ready() {
            self.tally.stalled += 1;
            if self.tally.stalled % 50 == 1 {
                crate::perf::note(format!("call rx stalled={} {error:?}", self.tally.stalled));
            }
            return Err(error);
        }
        let mut frames = Vec::new();
        let mut bytes = 0;
        let clock = crate::clock::Instant::now();
        for _ in 0..128 {
            let Some(packet) = self.held.take().or_else(|| self.packets.try_recv().ok()) else {
                break;
            };
            self.tally.packets += 1;
            if self.streams.iter().any(|s| s.ssrc == packet.ssrc) {
                self.incoming
                    .entry(packet.ssrc)
                    .or_default()
                    .push(packet.sequence, packet, clock);
            } else {
                self.tally.unknown += 1;
            }
        }
        for _ in 0..128 {
            let mut candidate = None;
            for _ in 0..self.streams.len() {
                let stream = &self.streams[self.incoming_cursor];
                self.incoming_cursor = (self.incoming_cursor + 1) % self.streams.len();
                // Drain complete frames before assembling more from a burst. Overflow here
                // used to discard reference frames before the decoder ever had a chance.
                if self.ready.full(stream.ssrc, stream.track.kind == MediaKind::Audio) {
                    continue;
                }
                let ssrc = stream.ssrc;
                // Give video retransmission one bounded round trip before discarding references.
                let repair_wait = Duration::from_millis(if stream.track.kind == MediaKind::Audio { 40 } else { 120 });
                if let Some(packet) = self.incoming.get_mut(&ssrc).and_then(|q| q.pop(clock, repair_wait)) {
                    candidate = Some(packet);
                    break;
                }
            }
            let Some((packet, gap)) = candidate else {
                break;
            };
            let stream = self
                .streams
                .iter()
                .find(|s| s.ssrc == packet.ssrc)
                .ok_or(Error::InvalidStore)?;
            if gap && stream.track.kind != MediaKind::Audio {
                self.ready.clear(packet.ssrc);
                self.video_gaps.insert(packet.ssrc);
            }
            let assembled = if stream.track.kind == MediaKind::Camera {
                self.camera_assembly.entry(packet.ssrc).or_default()
                    .push(packet.sequence, packet.timestamp, packet.marker, &packet.payload).map_err(failure)
            } else { self.assembly.push(stream.track.sender, stream.track.kind, &packet.payload, clock).map_err(failure) };
            match assembled {
                Ok(Some(encrypted)) => {
                    self.tally.assembled += 1;
                    if self.ready.push(
                        packet.ssrc,
                        stream.track.kind == MediaKind::Audio,
                        encrypted,
                        clock,
                    ) && stream.track.kind != MediaKind::Audio
                    {
                        self.video_gaps.insert(packet.ssrc);
                    }
                }
                Ok(None) => (),
                Err(Error::InvalidEvent | Error::Conflict | Error::Unprepared | Error::Limit) => {
                    self.tally.rejected += 1;
                    if self.tally.rejected % 200 == 1 {
                        // The leading bytes tell a sealed packet apart from a bare codec frame.
                        let head: String = packet
                            .payload
                            .iter()
                            .take(6)
                            .map(|b| format!("{b:02x}"))
                            .collect();
                        crate::perf::note(format!(
                            "call rx reject kind={:?} len={} head={head}",
                            stream.track.kind,
                            packet.payload.len()
                        ));
                    }
                }
                Err(error) => return Err(error),
            }
        }
        for _ in 0..128 {
            let mut candidate = None;
            for _ in 0..self.streams.len() {
                let stream = &self.streams[self.ready_cursor];
                self.ready_cursor = (self.ready_cursor + 1) % self.streams.len();
                let (encrypted, dropped) = self.ready.pop(stream.ssrc, clock);
                if dropped && stream.track.kind != MediaKind::Audio {
                    self.video_gaps.insert(stream.ssrc);
                }
                if let Some(encrypted) = encrypted {
                    candidate = Some((stream, encrypted));
                    break;
                }
            }
            let Some((stream, encrypted)) = candidate else {
                break;
            };
            match crypto.open(stream.track.sender, stream.track.kind, &encrypted) {
                Ok(frame) => {
                    self.tally.opened += 1;
                    if !self.video_gaps.contains(&stream.ssrc) || frame.keyframe {
                        self.video_gaps.remove(&stream.ssrc);
                        bytes += frame.data.len();
                        frames.push(ReceivedFrame {
                            sender: stream.track.sender,
                            frame,
                        });
                        self.tally.delivered[stream.track.kind as usize] += 1;
                    } else {
                        self.tally.waiting_key += 1;
                    }
                }
                Err(Error::InvalidEvent | Error::Conflict | Error::Unprepared) => {
                    self.tally.refused += 1;
                    if stream.track.kind != MediaKind::Audio {
                        self.video_gaps.insert(stream.ssrc);
                    }
                }
                Err(error) => return Err(error),
            }
            if bytes >= 4 * 1024 * 1024 || clock.elapsed() >= Duration::from_millis(8) {
                break;
            }
        }
        // Request keyframes only while starting or recovering a damaged dependency
        // chain. Healthy streams keep their cadence and bitrate for delta frames.
        if let Ok(mut requests) = self.key_requests.lock() {
            requests.clone_from(&self.video_gaps);
        }
        let due = self
            .tally
            .reported
            .is_none_or(|at| at.elapsed() >= Duration::from_secs(1));
        if due && self.tally.packets > 0 {
            self.tally.reported = Some(crate::clock::Instant::now());
            crate::perf::note(format!(
                "call rx packets={} unknown={} assembled={} rejected={} opened={} refused={} delivered={:?} waiting_key={}",
                self.tally.packets,
                self.tally.unknown,
                self.tally.assembled,
                self.tally.rejected,
                self.tally.opened,
                self.tally.refused,
                self.tally.delivered,
                self.tally.waiting_key
            ));
        }
        Ok(frames)
    }
}
impl ClientStore {
    pub async fn connect_rtc_call(
        &mut self,
        id: Id,
        tracks: Tracks,
        now: u64,
    ) -> Result<RtcCall, Error> {
        self.connect_rtc_call_with(id, tracks, None, now).await
    }
    /// A transport rebuilt after a roster change keeps its media handle: same lease, same keys,
    /// no fresh declaration for the others to wait on. A handle whose lease lapsed starts over.
    pub async fn connect_rtc_call_with(
        &mut self,
        id: Id,
        tracks: Tracks,
        reuse: Option<Media>,
        now: u64,
    ) -> Result<RtcCall, Error> {
        let record = load(&self.db, &self.key, &id)?;
        record.authorize(&self.db, &self.key, now)?;
        let roster = record.state.roster.roster;
        let own = record.own.as_ref().ok_or(Error::Unprepared)?.member.id;
        // The readiness and key exchange runs over the mailbox and costs a round trip each
        // way, so it starts here and overlaps candidate gathering instead of following it.
        let media = match reuse {
            Some(mut media) if media.call() == id => match self.refresh_call_media(&mut media, now) {
                Ok(_) | Err(Error::Unprepared) => media,
                Err(_) => self.start_call_media(id, tracks, crate::conversations::now())?,
            },
            _ => self.start_call_media(id, tracks, crate::conversations::now())?,
        };
        let relay = self.call_relay_online(id, now)?;
        let mut config = RTCConfigurationBuilder::new();
        let mut runtime: Arc<dyn webrtc::runtime::Runtime> =
            Arc::new(webrtc::runtime::TokioRuntime);
        if let Some(relay) = relay {
            let mut urls = relay.urls;
            runtime = turn_stream::RelayRuntime::configure(self, &mut urls)?;
            config = config.with_ice_servers(vec![RTCIceServer {
                urls,
                username: relay.username,
                credential: relay.credential.to_string(),
            }]);
        }
        let mut engine = MediaEngine::default();
        engine
            .register_default_codecs()
            .map_err(|_| Error::Unprepared)?;
        // A lost fragment discards its whole frame, so video asks for retransmission of
        // the packets it misses and answers the same request from the forwarder.
        for parameter in ["", "pli"] {
            engine.register_feedback(rtc::rtp_transceiver::rtp_sender::RTCPFeedback {
                typ: "nack".into(), parameter: parameter.into(),
            }, RtpCodecKind::Video);
        }
        let encoder_requests = Arc::new(AtomicU8::new(0));
        let uplink = Arc::new(encoder_feedback::Uplink::default());
        let (feedback, reports) = (encoder_requests.clone(), uplink.clone());
        let interceptors = Registry::new()
            .with(move |inner| encoder_feedback::Feedback::new(inner, feedback.clone(), reports.clone()))
            .with(NackGeneratorBuilder::new().with_interval(Duration::from_millis(10)).build())
            .with(NackResponderBuilder::new().build());
        let mut settings = SettingEngine::default();
        settings.set_multicast_dns_mode(rtc::ice::mdns::MulticastDnsMode::Disabled);
        let (gathered, gather_rx) = mpsc::channel(1);
        let (packets, packet_rx) = mpsc::channel(512);
        let connection = Arc::new(AtomicU8::new(0));
        let key_requests = KeyRequests::default();
        let pc = PeerConnectionBuilder::new()
            .with_configuration(config.build())
            .with_media_engine(engine)
            .with_interceptor_registry(interceptors)
            .with_setting_engine(settings)
            .with_runtime(runtime)
            .with_handler(Arc::new(Handler {
                gathered,
                packets,
                connection: connection.clone(),
                key_requests: key_requests.clone(),
            }))
            .with_udp_addrs(vec!["0.0.0.0:0"])
            .build()
            .await
            .map_err(|_| Error::Unprepared)?;
        let mut transport = Transport {
            pc: Arc::new(pc),
            gathered: gather_rx,
            connection,
            runtime: tokio::runtime::Handle::current(),
            encoder_requests,
            uplink,
        };
        let mut uploads = Vec::new();
        let mut senders = Vec::new();
        for index in 0..3 {
            let audio = index == 0;
            let track = Arc::new(TrackLocalStaticRTP::new(MediaStreamTrack::new(
                "sigil".into(),
                format!("track{index}"),
                format!("track{index}"),
                if audio {
                    RtpCodecKind::Audio
                } else {
                    RtpCodecKind::Video
                },
                vec![RTCRtpEncodingParameters {
                    rtp_coding_parameters: RTCRtpCodingParameters {
                        ssrc: Some(index + 1),
                        ..Default::default()
                    },
                    codec: RTCRtpCodec {
                        mime_type: if audio { "audio/opus" } else { "video/AV1" }.into(),
                        clock_rate: if audio { 48000 } else { 90000 },
                        channels: if audio { 2 } else { 0 },
                        sdp_fmtp_line: String::new(),
                        rtcp_feedback: vec![],
                    },
                    ..Default::default()
                }],
            )));
            let transceiver = transport
                .pc
                .add_transceiver_from_track(
                    track.clone() as Arc<dyn TrackLocal>,
                    Some(RTCRtpTransceiverInit {
                        direction: RTCRtpTransceiverDirection::Sendonly,
                        ..Default::default()
                    }),
                )
                .await
                .map_err(|_| Error::Unprepared)?;
            // The forwarder fixes each codec's payload types from the first section it negotiates,
            // which is this upload: without RTX here, no lost video packet is ever resent.
            if !audio {
                transceiver.set_codec_preferences(video_codecs()).await.map_err(|_| Error::Unprepared)?;
            }
            senders.push(transceiver);
            uploads.push(track);
        }
        let mut downloads = Vec::new();
        for member in &roster.members {
            if member.id == own {
                continue;
            }
            for kind in [MediaKind::Audio, MediaKind::Camera, MediaKind::Screen] {
                let transceiver = transport
                    .pc
                    .add_transceiver_from_kind(
                        if kind == MediaKind::Audio {
                            RtpCodecKind::Audio
                        } else {
                            RtpCodecKind::Video
                        },
                        Some(RTCRtpTransceiverInit {
                            direction: RTCRtpTransceiverDirection::Recvonly,
                            ..Default::default()
                        }),
                    )
                    .await
                    .map_err(|_| Error::Unprepared)?;
                downloads.push((
                    Track {
                        sender: member.id,
                        kind,
                        mid: String::new(),
                    },
                    transceiver,
                ));
            }
        }
        let offer = transport
            .pc
            .create_offer(None)
            .await
            .map_err(|_| Error::Unprepared)?;
        transport
            .pc
            .set_local_description(offer)
            .await
            .map_err(|_| Error::Unprepared)?;
        let gathering_from = crate::clock::Instant::now();
        tokio::time::timeout(Duration::from_secs(15), transport.gathered.recv())
            .await
            .map_err(|_| Error::Unprepared)?
            .ok_or(Error::Unprepared)?;
        crate::perf::mark("rtc gather", gathering_from);
        let sdp = transport
            .pc
            .local_description()
            .await
            .ok_or(Error::Unprepared)?
            .sdp;
        let mut mids = Vec::new();
        for sender in senders {
            mids.push(
                sender
                    .mid()
                    .await
                    .map_err(|_| Error::Unprepared)?
                    .ok_or(Error::Unprepared)?,
            );
        }
        let mut layout = Layout {
            uploads: mids.try_into().map_err(|_| Error::Unprepared)?,
            downloads: Vec::new(),
        };
        for (mut track, transceiver) in downloads {
            track.mid = transceiver
                .mid()
                .await
                .map_err(|_| Error::Unprepared)?
                .ok_or(Error::Unprepared)?;
            layout.downloads.push(track);
        }
        let proof = self.prepare_call_connection(id, sdp, layout, crate::conversations::now())?;
        let connecting_from = crate::clock::Instant::now();
        let answer = self.connect_call_online(&proof, crate::conversations::now())?;
        crate::perf::mark("rtc connect", connecting_from);
        transport
            .pc
            .set_remote_description(
                RTCSessionDescription::answer(answer.sdp).map_err(|_| Error::InvalidEvent)?,
            )
            .await
            .map_err(|_| Error::Unprepared)?;
        let receive = RtcReceive {
            packets: packet_rx,
            held: None,
            runtime: transport.runtime.clone(),
            key_requests,
            video_gaps: answer.streams.iter().filter(|s| s.track.kind != MediaKind::Audio).map(|s| s.ssrc).collect(),
            streams: answer.streams,
            incoming: Default::default(),
            ready: Default::default(),
            camera_assembly: Default::default(),
            assembly: Default::default(),
            ready_cursor: 0,
            incoming_cursor: 0,
            tally: Tally::default(),
        };
        let send = RtcSend { uploads, sequence: [0; 3], connection: transport.connection.clone() };
        Ok(RtcCall { transport, media, roster: answer.roster, send: Some(send), receive: Some(receive) })
    }
    pub fn rtc_connection_state(
        &mut self,
        call: &RtcCall,
        now: u64,
    ) -> Result<&'static str, Error> {
        self.rtc_connection_check(&call.media, call.roster, &call.transport.connection, now)
    }
    fn rtc_connection_check(
        &mut self,
        media: &Media,
        roster: Id,
        connection: &AtomicU8,
        now: u64,
    ) -> Result<&'static str, Error> {
        // data_version observes commits from other connections; total_changes observes this
        // connection. A cached proof never masks a peer block, new lease or roster update.
        let stage = crate::clock::Instant::now();
        let version = self.db.query_row("PRAGMA data_version", [], |row| row.get::<_, i64>(0))?;
        if stage.elapsed().as_millis() >= 10 { crate::perf::mark("call.revision", stage); }
        let changes = self.db.total_changes();
        let current = self.db.is_autocommit() && self.rtc_authority.as_ref().is_some_and(|checked|
            checked.call == media.call && checked.lease == media.lease
                && checked.version == version && checked.changes == changes
                && checked.checked.elapsed() < Duration::from_millis(200)
                && now.saturating_add(sigil_calls::CLOCK_SKEW) >= checked.created && now < checked.expires
        );
        let checked_roster = if current {
            self.rtc_authority.as_ref().ok_or(Error::Unprepared)?.roster
        } else {
            self.rtc_authority = None;
            let stage = crate::clock::Instant::now();
            // Read the call and its peer proofs from one snapshot. Releasing the reader
            // between queries repeatedly waits behind unrelated durable commits.
            let snapshot = self.db.is_autocommit().then(|| self.db.unchecked_transaction()).transpose()?;
            let record = load(&self.db, &self.key, &media.call)?;
            record.authorize(&self.db, &self.key, now)?;
            if stage.elapsed().as_millis() >= 10 { crate::perf::mark("call.connection_authority", stage); }
            if record.lease != media.lease { return Err(Error::Obsolete); }
            let roster = record.state.roster.roster.digest().map_err(failure)?;
            if let Some(snapshot) = snapshot { snapshot.commit()?; }
            if self.db.is_autocommit() {
                self.rtc_authority = Some(RtcAuthority {
                    call: media.call, lease: media.lease, roster,
                    created: record.state.roster.roster.created,
                    expires: record.state.roster.roster.expires,
                    version, changes, checked: stage,
                });
            }
            roster
        };
        if checked_roster != roster {
            return Ok("reconnect");
        }
        Ok(match connection.load(Ordering::Relaxed) {
            1 => "connected",
            2 => "disconnected",
            3 => "failed",
            _ => "connecting",
        })
    }
    pub fn rtc_media_state(&mut self, call: &mut RtcCall, now: u64) -> Result<&'static str, Error> {
        let state = self.rtc_connection_state(call, now)?;
        if state != "connected" {
            return Ok(state);
        }
        match self.checked_call_media(&mut call.media, now) {
            Ok(count) if count > 0 => Ok("connected"),
            Ok(_) | Err(Error::Unprepared) => Ok("securing"),
            Err(error) => Err(error),
        }
    }
    /// Publishes this owner's authority to detached frame paths and reports the call state.
    /// Anything but a connected, keyed call revokes the gate at once rather than at its deadline.
    pub fn rtc_publish(
        &mut self,
        call: &mut RtcCall,
        gate: &std::sync::Mutex<MediaGate>,
        lifetime: Duration,
        now: u64,
    ) -> Result<&'static str, Error> {
        let result = self.rtc_publish_with(call, gate, lifetime, now);
        if !matches!(result, Ok("connected")) {
            if let Ok(mut gate) = gate.lock() { gate.revoke(); }
        }
        result
    }
    fn rtc_publish_with(
        &mut self,
        call: &mut RtcCall,
        gate: &std::sync::Mutex<MediaGate>,
        lifetime: Duration,
        now: u64,
    ) -> Result<&'static str, Error> {
        let state = self.rtc_connection_state(call, now)?;
        if state != "connected" {
            return Ok(state);
        }
        let renew = gate.lock().map_err(|_| Error::Unprepared)?.renew.take();
        if let Some(context) = renew {
            self.renew_detached_media(&mut call.media, context, now)?;
        }
        let update = match self.detach_call_media(&mut call.media, now) {
            Ok(update) => update,
            Err(Error::Unprepared) => return Ok("securing"),
            Err(error) => return Err(error),
        };
        let keyed = update.receivers() > 0;
        gate.lock().map_err(|_| Error::Unprepared)?.publish(update, lifetime)?;
        Ok(if keyed { "connected" } else { "securing" })
    }
    pub fn rtc_set_tracks(
        &mut self,
        call: &mut RtcCall,
        tracks: Tracks,
        now: u64,
    ) -> Result<(), Error> {
        self.set_call_tracks(&mut call.media, tracks, now)
    }
    pub async fn rtc_send(
        &mut self,
        call: &mut RtcCall,
        kind: MediaKind,
        timestamp: u64,
        keyframe: bool,
        encoded: &[u8],
        now: u64,
    ) -> Result<(), Error> {
        self.rtc_prepare_send(call, kind, timestamp, keyframe, encoded, now)?
            .send()
            .await
    }
    pub fn rtc_prepare_send(
        &mut self,
        call: &mut RtcCall,
        kind: MediaKind,
        timestamp: u64,
        keyframe: bool,
        encoded: &[u8],
        now: u64,
    ) -> Result<RtcTransmission, Error> {
        let RtcCall { transport, media, roster, send, .. } = call;
        let send = send.as_mut().ok_or(Error::Unprepared)?;
        let mut stored = Stored { store: self, media, connection: &transport.connection, roster: *roster, now };
        if stored.store.rtc_connection_check(stored.media, stored.roster, stored.connection, now)? != "connected" {
            return Err(Error::Unprepared);
        }
        send.prepare(&mut stored, kind, timestamp, keyframe, encoded)
    }
    pub fn rtc_receive(
        &mut self,
        call: &mut RtcCall,
        now: u64,
    ) -> Result<Vec<ReceivedFrame>, Error> {
        let RtcCall { transport, media, roster, receive, .. } = call;
        let receive = receive.as_mut().ok_or(Error::Unprepared)?;
        receive.receive(&mut Stored { store: self, media, connection: &transport.connection, roster: *roster, now })
    }
}
