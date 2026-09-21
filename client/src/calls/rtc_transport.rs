use super::*;
use rtc::{
    interceptor::Registry,
    media_stream::MediaStreamTrack,
    peer_connection::{
        configuration::{
            media_engine::MediaEngine, setting_engine::SettingEngine, RTCConfigurationBuilder,
            RTCIceServer,
        },
        sdp::RTCSessionDescription,
    },
    rtp_transceiver::{
        rtp_sender::{RTCRtpCodec, RTCRtpCodingParameters, RTCRtpEncodingParameters, RtpCodecKind},
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
struct Handler {
    gathered: mpsc::Sender<()>,
    packets: mpsc::Sender<Packet>,
    connection: Arc<AtomicU8>,
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
        tokio::spawn(async move {
            let video = track.kind().await == RtpCodecKind::Video;
            let mut key_requested = None::<std::time::Instant>;
            let mut seen = 0u64;
            let mut refused = 0u64;
            while let Some(event) = track.poll().await {
                if let TrackRemoteEvent::OnRtpPacket(packet) = event {
                    let now = std::time::Instant::now();
                    if video && key_requested.is_none_or(|at| now.duration_since(at) >= Duration::from_secs(1)) {
                        key_requested = Some(now);
                        let request = rtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication {
                            sender_ssrc: 0, media_ssrc: packet.header.ssrc,
                        };
                        let _ = track.write_rtcp(vec![Box::new(request)]).await;
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
                        }
                    }
                }
            }
        });
    }
}
struct Transport {
    pc: Arc<dyn PeerConnection>,
    packets: mpsc::Receiver<Packet>,
    gathered: mpsc::Receiver<()>,
    connection: Arc<AtomicU8>,
    runtime: tokio::runtime::Handle,
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
    streams: Vec<Downstream>,
    uploads: Vec<Arc<TrackLocalStaticRTP>>,
    sequence: [u16; 3],
    incoming: std::collections::BTreeMap<u32, packet_order::PacketOrder<Packet>>,
    video_gaps: std::collections::BTreeSet<u32>,
    ready: ready_frames::ReadyFrames,
    camera_assembly: std::collections::BTreeMap<u32, sigil_calls::av1::Assembly>,
    ready_cursor: usize,
    incoming_cursor: usize,
    tally: Tally,
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
async fn send_packets<F, U>(
    packets: Vec<rtc::rtp::packet::Packet>,
    mut write: F,
) -> Result<(), Error>
where
    F: FnMut(rtc::rtp::packet::Packet) -> U,
    U: std::future::Future<Output = Result<(), Error>>,
{
    for packet in packets {
        write(packet).await?;
    }
    Ok(())
}
impl RtcCall {
    /// The media handle outlives the transport: a rebuild reuses it through `connect_rtc_call_with`.
    pub fn into_media(self) -> Media {
        self.media
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
        let interceptors = rtc::peer_connection::configuration::interceptor_registry::configure_nack(
            Registry::new(),
            &mut engine,
        );
        let mut settings = SettingEngine::default();
        settings.set_multicast_dns_mode(rtc::ice::mdns::MulticastDnsMode::Disabled);
        let (gathered, gather_rx) = mpsc::channel(1);
        let (packets, packet_rx) = mpsc::channel(512);
        let connection = Arc::new(AtomicU8::new(0));
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
            }))
            .with_udp_addrs(vec!["0.0.0.0:0"])
            .build()
            .await
            .map_err(|_| Error::Unprepared)?;
        let mut transport = Transport {
            pc: Arc::new(pc),
            packets: packet_rx,
            gathered: gather_rx,
            connection,
            runtime: tokio::runtime::Handle::current(),
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
            senders.push(
                transport
                    .pc
                    .add_transceiver_from_track(
                        track.clone() as Arc<dyn TrackLocal>,
                        Some(RTCRtpTransceiverInit {
                            direction: RTCRtpTransceiverDirection::Sendonly,
                            ..Default::default()
                        }),
                    )
                    .await
                    .map_err(|_| Error::Unprepared)?,
            );
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
        Ok(RtcCall {
            transport,
            media,
            roster: answer.roster,
            streams: answer.streams,
            uploads,
            sequence: [0; 3],
            incoming: Default::default(),
            video_gaps: Default::default(),
            ready: Default::default(),
            camera_assembly: Default::default(),
            ready_cursor: 0,
            incoming_cursor: 0,
            tally: Tally::default(),
        })
    }
    pub fn rtc_connection_state(
        &mut self,
        call: &RtcCall,
        now: u64,
    ) -> Result<&'static str, Error> {
        // data_version observes commits from other connections; total_changes observes this
        // connection. A cached proof never masks a peer block, new lease or roster update.
        let stage = crate::clock::Instant::now();
        let version = self.db.query_row("PRAGMA data_version", [], |row| row.get::<_, i64>(0))?;
        if stage.elapsed().as_millis() >= 10 { crate::perf::mark("call.revision", stage); }
        let changes = self.db.total_changes();
        let current = self.db.is_autocommit() && self.rtc_authority.as_ref().is_some_and(|checked|
            checked.call == call.media.call && checked.lease == call.media.lease
                && checked.version == version && checked.changes == changes
                && checked.checked.elapsed() < Duration::from_millis(200)
                && now.saturating_add(sigil_calls::CLOCK_SKEW) >= checked.created && now < checked.expires
        );
        let roster = if current {
            self.rtc_authority.as_ref().ok_or(Error::Unprepared)?.roster
        } else {
            self.rtc_authority = None;
            let stage = crate::clock::Instant::now();
            // Read the call and its peer proofs from one snapshot. Releasing the reader
            // between queries repeatedly waits behind unrelated durable commits.
            let snapshot = self.db.is_autocommit().then(|| self.db.unchecked_transaction()).transpose()?;
            let record = load(&self.db, &self.key, &call.media.call)?;
            record.authorize(&self.db, &self.key, now)?;
            if stage.elapsed().as_millis() >= 10 { crate::perf::mark("call.connection_authority", stage); }
            if record.lease != call.media.lease { return Err(Error::Obsolete); }
            let roster = record.state.roster.roster.digest().map_err(failure)?;
            if let Some(snapshot) = snapshot { snapshot.commit()?; }
            if self.db.is_autocommit() {
                self.rtc_authority = Some(RtcAuthority {
                    call: call.media.call, lease: call.media.lease, roster,
                    created: record.state.roster.roster.created,
                    expires: record.state.roster.roster.expires,
                    version, changes, checked: stage,
                });
            }
            roster
        };
        if roster != call.roster {
            return Ok("reconnect");
        }
        Ok(match call.transport.connection.load(Ordering::Relaxed) {
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
        if self.rtc_connection_state(call, now)? != "connected" {
            return Err(Error::Unprepared);
        }
        let index = kind as usize;
        let packets = if kind == MediaKind::Camera {
            let sealed = self.seal_call_frame(&mut call.media, kind, timestamp, keyframe, encoded, now)?;
            sigil_calls::av1::packetize(&sealed, keyframe).map_err(failure)?
        } else { self.seal_call_packets(&mut call.media, kind, timestamp, keyframe, encoded, now)? };
        let mut wire = Vec::with_capacity(packets.len());
        for (n, payload) in packets.iter().enumerate() {
            let packet = rtc::rtp::packet::Packet {
                header: rtc::rtp::header::Header {
                    version: 2,
                    marker: n + 1 == packets.len(),
                    payload_type: if index == 0 { 111 } else { 41 },
                    sequence_number: call.sequence[index],
                    timestamp: (timestamp.wrapping_mul(if index == 0 { 48 } else { 90 }) / 1000)
                        as u32,
                    ssrc: index as u32 + 1,
                    ..Default::default()
                },
                payload: payload.clone().into(),
            };
            call.sequence[index] = call.sequence[index].wrapping_add(1);
            wire.push(packet);
        }
        Ok(RtcTransmission {
            track: call.uploads[index].clone(),
            packets: wire,
        })
    }
    pub fn rtc_receive(
        &mut self,
        call: &mut RtcCall,
        now: u64,
    ) -> Result<Vec<ReceivedFrame>, Error> {
        if self.rtc_connection_state(call, now)? != "connected" {
            return Err(Error::Unprepared);
        }
        if let Err(error) = self.checked_call_media(&mut call.media, now) {
            call.tally.stalled += 1;
            if call.tally.stalled % 50 == 1 {
                crate::perf::note(format!("call rx stalled={} {error:?}", call.tally.stalled));
            }
            return Err(error);
        }
        let mut frames = Vec::new();
        let mut bytes = 0;
        let clock = crate::clock::Instant::now();
        for _ in 0..128 {
            let Ok(packet) = call.transport.packets.try_recv() else {
                break;
            };
            call.tally.packets += 1;
            if call.streams.iter().any(|s| s.ssrc == packet.ssrc) {
                call.incoming
                    .entry(packet.ssrc)
                    .or_default()
                    .push(packet.sequence, packet, clock);
            } else {
                call.tally.unknown += 1;
            }
        }
        for _ in 0..128 {
            let mut candidate = None;
            for _ in 0..call.streams.len() {
                let stream = &call.streams[call.incoming_cursor];
                call.incoming_cursor = (call.incoming_cursor + 1) % call.streams.len();
                // Drain complete frames before assembling more from a burst. Overflow here
                // used to discard reference frames before the decoder ever had a chance.
                if call.ready.full(stream.ssrc, stream.track.kind == MediaKind::Audio) {
                    continue;
                }
                let ssrc = stream.ssrc;
                if let Some(packet) = call.incoming.get_mut(&ssrc).and_then(|q| q.pop(clock)) {
                    candidate = Some(packet);
                    break;
                }
            }
            let Some((packet, gap)) = candidate else {
                break;
            };
            let stream = call
                .streams
                .iter()
                .find(|s| s.ssrc == packet.ssrc)
                .ok_or(Error::InvalidStore)?;
            if gap && stream.track.kind != MediaKind::Audio {
                call.ready.clear(packet.ssrc);
                call.video_gaps.insert(packet.ssrc);
            }
            let assembled = if stream.track.kind == MediaKind::Camera {
                call.camera_assembly.entry(packet.ssrc).or_default()
                    .push(packet.sequence, packet.timestamp, packet.marker, &packet.payload).map_err(failure)
            } else { call.media.assemble(stream.track.sender, stream.track.kind, &packet.payload) };
            match assembled {
                Ok(Some(encrypted)) => {
                    call.tally.assembled += 1;
                    if call.ready.push(
                        packet.ssrc,
                        stream.track.kind == MediaKind::Audio,
                        encrypted,
                        clock,
                    ) && stream.track.kind != MediaKind::Audio
                    {
                        call.video_gaps.insert(packet.ssrc);
                    }
                }
                Ok(None) => (),
                Err(Error::InvalidEvent | Error::Conflict | Error::Unprepared | Error::Limit) => {
                    call.tally.rejected += 1;
                    if call.tally.rejected % 200 == 1 {
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
            for _ in 0..call.streams.len() {
                let stream = &call.streams[call.ready_cursor];
                call.ready_cursor = (call.ready_cursor + 1) % call.streams.len();
                let (encrypted, dropped) = call.ready.pop(stream.ssrc, clock);
                if dropped && stream.track.kind != MediaKind::Audio {
                    call.video_gaps.insert(stream.ssrc);
                }
                if let Some(encrypted) = encrypted {
                    candidate = Some((stream, encrypted));
                    break;
                }
            }
            let Some((stream, encrypted)) = candidate else {
                break;
            };
            match self.open_call_frame(
                &mut call.media,
                stream.track.sender,
                stream.track.kind,
                &encrypted,
                now,
            ) {
                Ok(frame) => {
                    call.tally.opened += 1;
                    if !call.video_gaps.contains(&stream.ssrc) || frame.keyframe {
                        call.video_gaps.remove(&stream.ssrc);
                        bytes += frame.data.len();
                        frames.push(ReceivedFrame {
                            sender: stream.track.sender,
                            frame,
                        });
                        call.tally.delivered[stream.track.kind as usize] += 1;
                    } else {
                        call.tally.waiting_key += 1;
                    }
                }
                Err(Error::InvalidEvent | Error::Conflict | Error::Unprepared) => {
                    call.tally.refused += 1;
                    if stream.track.kind != MediaKind::Audio {
                        call.video_gaps.insert(stream.ssrc);
                    }
                }
                Err(error) => return Err(error),
            }
            if bytes >= 4 * 1024 * 1024 || clock.elapsed() >= Duration::from_millis(8) {
                break;
            }
        }
        let due = call
            .tally
            .reported
            .is_none_or(|at| at.elapsed() >= Duration::from_secs(1));
        if due && call.tally.packets > 0 {
            call.tally.reported = Some(crate::clock::Instant::now());
            crate::perf::note(format!(
                "call rx packets={} unknown={} assembled={} rejected={} opened={} refused={} delivered={:?} waiting_key={}",
                call.tally.packets,
                call.tally.unknown,
                call.tally.assembled,
                call.tally.rejected,
                call.tally.opened,
                call.tally.refused,
                call.tally.delivered,
                call.tally.waiting_key
            ));
        }
        Ok(frames)
    }
}
