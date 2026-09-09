use super::*;
use rtc::{
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
#[cfg(test)]
#[path = "rtc_tests.rs"]
mod tests;
#[path = "turn_stream.rs"]
mod turn_stream;

struct Packet {
    ssrc: u32,
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
            while let Some(event) = track.poll().await {
                if let TrackRemoteEvent::OnRtpPacket(packet) = event {
                    if packet.payload.len() <= 2048 {
                        let _ = packets.try_send(Packet {
                            ssrc: packet.header.ssrc,
                            payload: packet.payload.to_vec(),
                        });
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
pub struct RtcCall {
    transport: Transport,
    media: Media,
    roster: Id,
    streams: Vec<Downstream>,
    uploads: Vec<Arc<TrackLocalStaticRTP>>,
    sequence: [u16; 3],
}
impl ClientStore {
    pub async fn connect_rtc_call(
        &mut self,
        id: Id,
        tracks: Tracks,
        now: u64,
    ) -> Result<RtcCall, Error> {
        let record = load(&self.db, &self.key, &id)?;
        record.authorize(&self.db, &self.key, now)?;
        let roster = record.state.roster.roster;
        let own = record.own.as_ref().ok_or(Error::Unprepared)?.member.id;
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
        let mut settings = SettingEngine::default();
        settings.set_multicast_dns_mode(rtc::ice::mdns::MulticastDnsMode::Disabled);
        let (gathered, gather_rx) = mpsc::channel(1);
        let (packets, packet_rx) = mpsc::channel(512);
        let connection = Arc::new(AtomicU8::new(0));
        let pc = PeerConnectionBuilder::new()
            .with_configuration(config.build())
            .with_media_engine(engine)
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
                        mime_type: if audio { "audio/opus" } else { "video/VP8" }.into(),
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
        tokio::time::timeout(Duration::from_secs(15), transport.gathered.recv())
            .await
            .map_err(|_| Error::Unprepared)?
            .ok_or(Error::Unprepared)?;
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
        let answer = self.connect_call_online(&proof, crate::conversations::now())?;
        transport
            .pc
            .set_remote_description(
                RTCSessionDescription::answer(answer.sdp).map_err(|_| Error::InvalidEvent)?,
            )
            .await
            .map_err(|_| Error::Unprepared)?;
        let media = self.start_call_media(id, tracks, crate::conversations::now())?;
        Ok(RtcCall {
            transport,
            media,
            roster: answer.roster,
            streams: answer.streams,
            uploads,
            sequence: [0; 3],
        })
    }
    pub fn rtc_connection_state(
        &mut self,
        call: &RtcCall,
        now: u64,
    ) -> Result<&'static str, Error> {
        let record = load(&self.db, &self.key, &call.media.call)?;
        record.authorize(&self.db, &self.key, now)?;
        if record.lease != call.media.lease {
            return Err(Error::Obsolete);
        }
        if record.state.roster.roster.digest().map_err(failure)? != call.roster {
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
        match self.refresh_call_media(&mut call.media, now) {
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
        if self.rtc_connection_state(call, now)? != "connected" {
            return Err(Error::Unprepared);
        }
        let index = kind as usize;
        let packets =
            self.seal_call_packets(&mut call.media, kind, timestamp, keyframe, encoded, now)?;
        for (n, payload) in packets.iter().enumerate() {
            let packet = rtc::rtp::packet::Packet {
                header: rtc::rtp::header::Header {
                    version: 2,
                    marker: n + 1 == packets.len(),
                    payload_type: if index == 0 { 111 } else { 96 },
                    sequence_number: call.sequence[index],
                    timestamp: (timestamp.wrapping_mul(if index == 0 { 48 } else { 90 }) / 1000)
                        as u32,
                    ssrc: index as u32 + 1,
                    ..Default::default()
                },
                payload: payload.clone().into(),
            };
            call.sequence[index] = call.sequence[index].wrapping_add(1);
            call.uploads[index]
                .write_rtp(packet)
                .await
                .map_err(|_| Error::Unprepared)?;
        }
        Ok(())
    }
    pub fn rtc_receive(
        &mut self,
        call: &mut RtcCall,
        now: u64,
    ) -> Result<Vec<ReceivedFrame>, Error> {
        if self.rtc_connection_state(call, now)? != "connected" {
            return Err(Error::Unprepared);
        }
        self.refresh_call_media(&mut call.media, now)?;
        let mut frames = Vec::new();
        let mut bytes = 0;
        for _ in 0..128 {
            let Ok(packet) = call.transport.packets.try_recv() else {
                break;
            };
            let Some(stream) = call.streams.iter().find(|s| s.ssrc == packet.ssrc) else {
                continue;
            };
            match self.open_call_packet(
                &mut call.media,
                stream.track.sender,
                stream.track.kind,
                &packet.payload,
                now,
            ) {
                Ok(Some(frame)) => {
                    bytes += frame.data.len();
                    frames.push(ReceivedFrame {
                        sender: stream.track.sender,
                        frame,
                    });
                }
                Ok(None) | Err(Error::InvalidEvent | Error::Conflict | Error::Unprepared) => (),
                Err(error) => return Err(error),
            }
            if bytes >= 4 * 1024 * 1024 {
                break;
            }
        }
        Ok(frames)
    }
}
