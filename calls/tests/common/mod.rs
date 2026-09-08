#![forbid(unsafe_code)]

pub mod proxy;
mod turn;
use rtc::{
    media_stream::MediaStreamTrack,
    peer_connection::{
        configuration::{media_engine::MediaEngine, setting_engine::SettingEngine},
        sdp::RTCSessionDescription,
    },
    rtp_transceiver::rtp_sender::{
        RTCRtpCodec, RTCRtpCodingParameters, RTCRtpEncodingParameters, RtpCodecKind,
    },
};
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;
use webrtc::{
    media_stream::{
        track_local::{static_rtp::TrackLocalStaticRTP, TrackLocal},
        track_remote::{TrackRemote, TrackRemoteEvent},
    },
    peer_connection::{
        PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCIceGatheringState,
    },
};
#[derive(Debug)]
pub struct Packet {
    pub pt: u8,
    pub seq: u16,
    pub ssrc: u32,
    pub received: std::time::Instant,
    pub payload: Vec<u8>,
}
struct Handler {
    pub gather: mpsc::Sender<()>,
    pub packets: mpsc::Sender<Packet>,
}
#[async_trait::async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        if state == RTCIceGatheringState::Complete {
            let _ = self.gather.try_send(());
        }
    }
    async fn on_track(&self, track: Arc<dyn TrackRemote>) {
        let packets = self.packets.clone();
        tokio::spawn(async move {
            while let Some(event) = track.poll().await {
                if let TrackRemoteEvent::OnRtpPacket(p) = event {
                    let _ = packets.try_send(Packet {
                        pt: p.header.payload_type,
                        seq: p.header.sequence_number,
                        ssrc: p.header.ssrc,
                        received: std::time::Instant::now(),
                        payload: p.payload.to_vec(),
                    });
                }
            }
        });
    }
}
pub struct Peer {
    pub pc: Arc<dyn PeerConnection>,
    pub gather: mpsc::Receiver<()>,
    pub packets: mpsc::Receiver<Packet>,
    bridge: Option<tokio::task::JoinHandle<()>>,
}
impl Drop for Peer {
    fn drop(&mut self) {
        if let Some(task) = &self.bridge {
            task.abort();
        }
    }
}
impl Peer {
    #[allow(dead_code)]
    pub async fn new(lite: bool, relay: bool) -> Self {
        let server = relay.then(|| rtc::peer_connection::configuration::RTCIceServer {
            urls: vec![std::env::var("SIGIL_TEST_TURN_URL")
                .unwrap_or_else(|_| "turn:127.0.0.1:39781?transport=udp".into())],
            username: "synthetic".into(),
            credential: "synthetic-test-secret".into(),
        });
        Self::configured(lite, server).await
    }
    pub async fn configured(
        lite: bool,
        server: Option<rtc::peer_connection::configuration::RTCIceServer>,
    ) -> Self {
        let relay = server.is_some();
        let (gather, gather_rx) = mpsc::channel(8);
        let (packets, packets_rx) = mpsc::channel(512);
        let mut engine = MediaEngine::default();
        engine.register_default_codecs().unwrap();
        let mut settings = SettingEngine::default();
        settings.set_multicast_dns_mode(rtc::ice::mdns::MulticastDnsMode::Disabled);
        settings.set_lite(lite);
        if relay {
            settings.set_relay_acceptance_min_wait(Some(Duration::ZERO));
        }
        use rtc::peer_connection::configuration::{RTCConfigurationBuilder, RTCIceTransportPolicy};
        let mut config = RTCConfigurationBuilder::new();
        let mut bridge = None;
        if let Some(mut server) = server {
            assert_eq!(server.urls.len(), 1);
            let (url, task) = turn::bridge(&server.urls[0]).await;
            bridge = task;
            server.urls[0] = url;
            config = config
                .with_ice_servers(vec![server])
                .with_ice_transport_policy(RTCIceTransportPolicy::Relay);
        }
        let pc = PeerConnectionBuilder::new()
            .with_configuration(config.build())
            .with_media_engine(engine)
            .with_setting_engine(settings)
            .with_runtime(Arc::new(webrtc::runtime::TokioRuntime))
            .with_handler(Arc::new(Handler { gather, packets }))
            .with_udp_addrs(vec!["127.0.0.1:0"])
            .build()
            .await
            .unwrap();
        Self {
            pc: Arc::new(pc),
            gather: gather_rx,
            packets: packets_rx,
            bridge,
        }
    }
    pub async fn local(&mut self, description: RTCSessionDescription) -> String {
        self.pc.set_local_description(description).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), self.gather.recv())
            .await
            .unwrap()
            .unwrap();
        self.pc.local_description().await.unwrap().sdp
    }
    pub async fn track(
        &self,
        n: u32,
    ) -> (
        Arc<TrackLocalStaticRTP>,
        Arc<dyn webrtc::rtp_transceiver::RtpTransceiver>,
    ) {
        let audio = n == 1;
        let track = Arc::new(TrackLocalStaticRTP::new(MediaStreamTrack::new(
            "synthetic".into(),
            format!("track{n}"),
            format!("track{n}"),
            if audio {
                RtpCodecKind::Audio
            } else {
                RtpCodecKind::Video
            },
            vec![RTCRtpEncodingParameters {
                rtp_coding_parameters: RTCRtpCodingParameters {
                    ssrc: Some(n),
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
        let transceiver = self
            .pc
            .add_transceiver_from_track(
                track.clone() as Arc<dyn TrackLocal>,
                Some(rtc::rtp_transceiver::RTCRtpTransceiverInit {
                    direction: rtc::rtp_transceiver::RTCRtpTransceiverDirection::Sendonly,
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
        (track, transceiver)
    }
}

use rtc::rtp_transceiver::{RTCRtpTransceiverDirection, RTCRtpTransceiverInit};
use sigil_calls::{Id, Layout, MediaKind, Roster, Track};
#[allow(dead_code)]
pub async fn offer(
    peer: &mut Peer,
    roster: &Roster,
    own: Id,
    uploads: Vec<Arc<dyn webrtc::rtp_transceiver::RtpTransceiver>>,
) -> (String, Layout) {
    let mut downloads = Vec::new();
    let mut download_transceivers = Vec::new();
    for member in &roster.members {
        if member.id == own {
            continue;
        }
        for kind in [MediaKind::Audio, MediaKind::Camera, MediaKind::Screen] {
            let transceiver = peer
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
                .unwrap();
            download_transceivers.push(transceiver);
            downloads.push(Track {
                sender: member.id,
                kind,
                mid: String::new(),
            });
        }
    }
    let value = peer.pc.create_offer(None).await.unwrap();
    let sdp = peer.local(value).await;
    let mut mids = Vec::new();
    for t in uploads {
        mids.push(t.mid().await.unwrap().unwrap());
    }
    let uploads = mids.try_into().unwrap();
    for (track, transceiver) in downloads.iter_mut().zip(download_transceivers) {
        track.mid = transceiver.mid().await.unwrap().unwrap();
    }
    (sdp, Layout { uploads, downloads })
}
