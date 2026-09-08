use crate::{Answer, Error, Id, Layout, MediaKind, SignedConnect, SignedRoster};
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    time::{Duration, Instant},
};
use str0m::{
    format::Codec,
    media::{Direction, Mid},
    net::{Protocol, Receive},
    rtp::{RtpPacket, RtpWrite},
    Event, Input, Output, Rtc,
};
pub struct Datagram {
    pub destination: SocketAddr,
    pub contents: Vec<u8>,
}
pub struct Tick {
    pub datagrams: Vec<Datagram>,
    pub next: Instant,
}
struct Peer {
    rtc: Rtc,
    layout: Layout,
    sequence: u64,
    digest: Id,
    answer: Answer,
    window: Instant,
    bytes: usize,
    feedback: Instant,
    queues: BTreeMap<Mid, crate::forwarding::Queue>,
    rewrite: BTreeMap<Mid, crate::forwarding::Rewrite>,
}
struct Room {
    roster: SignedRoster,
    peers: BTreeMap<Id, Peer>,
}
pub struct Forwarder {
    address: SocketAddr,
    rooms: BTreeMap<Id, Room>,
    max_rooms: usize,
    dropped: u64,
}
fn mid(value: &str) -> Mid {
    value.into()
}
fn codec(kind: MediaKind) -> Codec {
    if kind == MediaKind::Audio {
        Codec::Opus
    } else {
        Codec::Vp8
    }
}
impl Forwarder {
    pub fn new(address: SocketAddr, max_rooms: usize) -> Result<Self, Error> {
        if address.port() == 0 || max_rooms == 0 || max_rooms > 8 {
            return Err(Error::Invalid);
        }
        str0m::crypto::from_feature_flags().install_process_default();
        Ok(Self {
            address,
            rooms: BTreeMap::new(),
            max_rooms,
            dropped: 0,
        })
    }
    /// The service must authorize the call's owner before installing its signed roster.
    pub fn install(&mut self, roster: SignedRoster, now: u64) -> Result<(), Error> {
        roster.verify()?;
        let call = roster.roster.call;
        if let Some(old) = self.rooms.get(&call) {
            if old.roster.roster == roster.roster {
                return Ok(());
            }
            old.roster.roster.successor(&roster.roster, false)?;
        }
        if roster.roster.closed || roster.roster.expires <= now {
            self.rooms.remove(&call);
            return Ok(());
        }
        roster.roster.active(now)?;
        if !self.rooms.contains_key(&call) && self.rooms.len() >= self.max_rooms {
            return Err(Error::Limit);
        }
        self.rooms.insert(
            call,
            Room {
                roster,
                peers: BTreeMap::new(),
            },
        );
        Ok(())
    }
    pub fn retain(&mut self, calls: &std::collections::BTreeSet<Id>) {
        self.rooms.retain(|id, _| calls.contains(id));
    }
    pub fn connected(&self, call: Id, participant: Id) -> bool {
        self.rooms
            .get(&call)
            .is_some_and(|room| room.peers.contains_key(&participant))
    }
    pub fn connect(
        &mut self,
        request: &SignedConnect,
        now: u64,
        clock: Instant,
    ) -> Result<Answer, Error> {
        let room = self
            .rooms
            .get_mut(&request.request.call)
            .ok_or(Error::Invalid)?;
        request.verify(&room.roster.roster, now)?;
        let value = &request.request;
        let digest = value.digest()?;
        if let Some(old) = room.peers.get(&value.participant) {
            if old.sequence == value.sequence && old.digest == digest {
                return Ok(old.answer.clone());
            }
            if old.sequence >= value.sequence {
                return Err(Error::Conflict);
            }
        }
        let mut rtc = Rtc::builder()
            .set_rtp_mode(true)
            .set_ice_lite(true)
            .clear_codecs()
            .enable_opus(true)
            .enable_vp8(true)
            .build(clock);
        rtc.add_local_candidate(
            str0m::Candidate::host(self.address, "udp").map_err(|_| Error::Invalid)?,
        )
        .ok_or(Error::Invalid)?;
        let offer =
            str0m::change::SdpOffer::from_sdp_string(&value.sdp).map_err(|_| Error::Invalid)?;
        let sdp = rtc
            .sdp_api()
            .accept_offer(offer)
            .map_err(|_| Error::Invalid)?
            .to_sdp_string();
        if value.sdp.lines().filter(|l| l.starts_with("m=")).count()
            != 3 + value.layout.downloads.len()
        {
            return Err(Error::Invalid);
        }
        for (kind, upload) in value.layout.uploads.iter().enumerate() {
            let media = rtc.media(mid(upload)).ok_or(Error::Invalid)?;
            if media.disabled()
                || media.stopped()
                || media.direction() != Direction::RecvOnly
                || media.kind().is_audio() != (kind == 0)
            {
                return Err(Error::Invalid);
            }
        }
        let mut streams = Vec::new();
        for track in &value.layout.downloads {
            let media = rtc.media(mid(&track.mid)).ok_or(Error::Invalid)?;
            if media.disabled()
                || media.stopped()
                || media.direction() != Direction::SendOnly
                || media.kind().is_audio() != (track.kind == MediaKind::Audio)
            {
                return Err(Error::Invalid);
            }
            let mut api = rtc.direct_api();
            let stream = api
                .stream_tx_by_mid(mid(&track.mid), None)
                .ok_or(Error::Invalid)?;
            stream.set_rtx_cache(128, Duration::from_secs(2), Some(0.15));
            streams.push(crate::Downstream {
                track: track.clone(),
                ssrc: *stream.ssrc(),
            });
        }
        let answer = Answer {
            sdp,
            sequence: value.sequence,
            roster: value.roster,
            participant: value.participant,
            streams,
        };
        room.peers.insert(
            value.participant,
            Peer {
                rtc,
                layout: value.layout.clone(),
                sequence: value.sequence,
                digest,
                answer: answer.clone(),
                window: clock,
                bytes: 0,
                feedback: clock,
                queues: BTreeMap::new(),
                rewrite: BTreeMap::new(),
            },
        );
        Ok(answer)
    }
    pub fn receive(&mut self, source: SocketAddr, bytes: &[u8], now: Instant) {
        if bytes.len() > 2048 {
            return;
        }
        let Ok(packet) = Receive::new(Protocol::Udp, source, self.address, bytes) else {
            return;
        };
        let input = Input::Receive(now, packet);
        for room in self.rooms.values_mut() {
            for peer in room.peers.values_mut() {
                if peer.rtc.accepts(&input) {
                    if now.duration_since(peer.window) >= Duration::from_secs(1) {
                        peer.bytes = 0;
                        peer.window = now;
                    }
                    peer.bytes = peer.bytes.saturating_add(bytes.len());
                    if peer.bytes <= 1024 * 1024 {
                        let _ = peer.rtc.handle_input(input);
                    } else {
                        self.dropped = self.dropped.saturating_add(1);
                    }
                    return;
                }
            }
        }
    }
    pub fn tick(&mut self, now: u64, clock: Instant) -> Tick {
        self.rooms
            .retain(|_, r| r.roster.roster.active(now).is_ok());
        let mut datagrams = Vec::new();
        let mut next = clock + Duration::from_millis(50);
        for room in self.rooms.values_mut() {
            let mut packets: Vec<(Id, u64, MediaKind, RtpPacket)> = Vec::new();
            let mut feedback = Vec::new();
            let mut dead = Vec::new();
            for (id, peer) in &mut room.peers {
                if peer.rtc.handle_input(Input::Timeout(clock)).is_err() {
                    dead.push(*id);
                    continue;
                }
                for _ in 0..64 {
                    match peer.rtc.poll_output() {
                        Ok(Output::Timeout(at)) => {
                            next = next.min(at);
                            break;
                        }
                        Ok(Output::Transmit(v)) => {
                            if v.contents.len() <= 2048 {
                                datagrams.push(Datagram {
                                    destination: v.destination,
                                    contents: v.contents.to_vec(),
                                });
                            }
                        }
                        Ok(Output::Event(Event::RtpPacket(packet))) => {
                            let source = peer
                                .rtc
                                .direct_api()
                                .stream_rx(&packet.header.ssrc)
                                .map(|s| s.mid());
                            if let Some(kind) = source.and_then(|source| {
                                peer.layout.uploads.iter().position(|v| mid(v) == source)
                            }) {
                                let kind = match kind {
                                    0 => MediaKind::Audio,
                                    1 => MediaKind::Camera,
                                    _ => MediaKind::Screen,
                                };
                                let valid = peer.rtc.codec_config().iter().any(|p| {
                                    p.pt() == packet.header.payload_type
                                        && p.spec().codec == codec(kind)
                                });
                                if valid && packet.payload.len() <= 1500 {
                                    packets.push((*id, peer.sequence, kind, packet));
                                }
                            }
                        }
                        Ok(Output::Event(Event::KeyframeRequest(request))) => {
                            if clock.duration_since(peer.feedback) >= Duration::from_millis(500) {
                                if let Some(track) = peer
                                    .layout
                                    .downloads
                                    .iter()
                                    .find(|v| mid(&v.mid) == request.mid)
                                {
                                    feedback.push((track.sender, track.kind, request.kind));
                                    peer.feedback = clock;
                                }
                            }
                        }
                        Ok(_) => (),
                        Err(_) => {
                            dead.push(*id);
                            break;
                        }
                    }
                }
                if !peer.rtc.is_alive() {
                    dead.push(*id);
                }
            }
            for id in dead {
                room.peers.remove(&id);
            }
            for (source, kind, request) in feedback {
                if let Some(peer) = room.peers.get_mut(&source) {
                    let upload = mid(&peer.layout.uploads[kind as usize]);
                    if let Some(stream) = peer.rtc.direct_api().stream_rx_by_mid(upload, None) {
                        stream.request_keyframe(request);
                    }
                }
            }
            if !packets.is_empty() {
                next = clock + Duration::from_millis(1);
            }
            for (source, generation, kind, packet) in packets {
                for (id, peer) in &mut room.peers {
                    if *id == source {
                        continue;
                    }
                    let Some(route) = peer
                        .layout
                        .downloads
                        .iter()
                        .find(|r| r.sender == source && r.kind == kind)
                    else {
                        continue;
                    };
                    let mid = mid(&route.mid);
                    let pt = peer.rtc.media(mid).and_then(|media| {
                        peer.rtc
                            .codec_config()
                            .iter()
                            .find(|p| {
                                p.spec().codec == codec(kind)
                                    && media.remote_pts().contains(&p.pt())
                            })
                            .map(|p| p.pt())
                    });
                    if let Some(pt) = pt {
                        if let Some(stream) = peer.rtc.direct_api().stream_tx_by_mid(mid, None) {
                            let sample = stream
                                .queue_info()
                                .map(|s| (s.created_at(), s.byte_size(), s.packet_count()));
                            if !peer
                                .queues
                                .entry(mid)
                                .or_default()
                                .admit(sample, packet.payload.len())
                            {
                                self.dropped = self.dropped.saturating_add(1);
                                continue;
                            }
                            let Some((sequence, timestamp)) =
                                peer.rewrite.entry(mid).or_default().packet(
                                    generation,
                                    *packet.header.ssrc,
                                    *packet.seq_no,
                                    packet.header.timestamp,
                                    if kind == MediaKind::Audio { 960 } else { 3000 },
                                )
                            else {
                                continue;
                            };
                            stream.write_rtp(
                                RtpWrite::new(
                                    pt,
                                    sequence.into(),
                                    timestamp,
                                    clock,
                                    packet.payload.clone(),
                                )
                                .marker(packet.header.marker)
                                .nackable(kind != MediaKind::Audio),
                            );
                        }
                    }
                }
            }
        }
        Tick {
            datagrams,
            next: next.max(clock + Duration::from_millis(1)),
        }
    }
    pub fn counts(&self) -> (usize, usize) {
        (
            self.rooms.len(),
            self.rooms.values().map(|r| r.peers.len()).sum(),
        )
    }
    pub fn dropped_packets(&self) -> u64 {
        self.dropped
    }
}
