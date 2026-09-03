//! The call forwarding unit. One UDP socket carries every call; each
//! participant is a `str0m` peer; media is forwarded between the peers of a
//! room and nowhere else. The room is a 32-byte value the participants
//! derived among themselves, so the server sees "N peers in random room X",
//! and with SFrame on the clients the media it forwards is opaque too.
//!
//! Signalling arrives as `call.signal` operations in sealed bags (wire spec
//! 3.8): `join` carries the participant's SDP offer and returns the answer
//! and a peer id; whenever another participant's track has to be added the
//! unit produces a renegotiation offer, handed over on `poll` (or on the
//! participant's data channel if it opened one) and completed by `answer`;
//! `leave` drops the peer. The unit itself runs on its own thread, the way
//! `str0m` is designed to be driven: poll outputs, feed inputs, drive time.

use std::collections::VecDeque;
use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use str0m::change::{SdpAnswer, SdpOffer, SdpPendingOffer};
use str0m::channel::{ChannelData, ChannelId};
use str0m::media::{
    Direction, KeyframeRequest, KeyframeRequestKind, MediaData, MediaKind, Mid, Rid,
};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc};
use tokio::sync::oneshot;

pub type PeerId = [u8; 16];
pub type Room = [u8; 32];

/// A peer that never finished connecting is dropped after this long.
const CONNECT_GRACE: Duration = Duration::from_secs(30);
/// A connected peer that neither polls nor sends anything for this long is
/// dropped; ICE normally notices sooner.
const IDLE_LIMIT: Duration = Duration::from_secs(60);

enum Cmd {
    Join {
        room: Room,
        offer: String,
        reply: oneshot::Sender<Result<(String, PeerId), String>>,
    },
    Poll {
        peer: PeerId,
        reply: oneshot::Sender<Result<(Option<String>, usize), String>>,
    },
    Answer {
        peer: PeerId,
        answer: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Leave {
        peer: PeerId,
        reply: oneshot::Sender<Result<(), String>>,
    },
}

/// Handle to the running unit.
pub struct Sfu {
    tx: Mutex<Sender<Cmd>>,
    /// The address participants send media to.
    pub public: SocketAddr,
}

impl Sfu {
    /// Bind `bind` and start the forwarding thread. `public` is the address
    /// to advertise when it differs from the bound one.
    pub fn start(bind: &str, public: Option<&str>) -> anyhow::Result<Arc<Sfu>> {
        let socket = UdpSocket::bind(bind)?;
        let local = socket.local_addr()?;
        let public = match public {
            Some(p) => std::net::ToSocketAddrs::to_socket_addrs(&p)?
                .next()
                .ok_or_else(|| anyhow::anyhow!("media_public does not resolve"))?,
            None if local.ip().is_unspecified() => SocketAddr::new(guess_local_ip(), local.port()),
            None => local,
        };
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("sfu".into())
            .spawn(move || run(socket, public, rx))?;
        tracing::info!("forwarding unit listening on udp {local}");
        Ok(Arc::new(Sfu {
            tx: Mutex::new(tx),
            public,
        }))
    }

    fn send(&self, cmd: Cmd) -> Result<(), String> {
        self.tx
            .lock()
            .map_err(|_| "forwarding unit lock".to_string())?
            .send(cmd)
            .map_err(|_| "forwarding unit stopped".to_string())
    }

    /// Handle one JSON signalling message and return the JSON reply.
    pub async fn signal(&self, room: Room, body: &[u8]) -> Result<Value, String> {
        let msg: Value = serde_json::from_slice(body).map_err(|_| "not json".to_string())?;
        let field = |k: &str| {
            msg.get(k)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| format!("missing {k}"))
        };
        match kind_of(body).as_deref() {
            Some("join") => {
                let (s, r) = oneshot::channel();
                self.send(Cmd::Join {
                    room,
                    offer: field("offer")?,
                    reply: s,
                })?;
                let (answer, peer) = r.await.map_err(|_| "no reply".to_string())??;
                Ok(json!({"answer": answer, "peer": hex::encode(peer)}))
            }
            Some("poll") => {
                let (s, r) = oneshot::channel();
                self.send(Cmd::Poll {
                    peer: parse_peer(&field("peer")?)?,
                    reply: s,
                })?;
                let (offer, peers) = r.await.map_err(|_| "no reply".to_string())??;
                Ok(json!({"offer": offer, "peers": peers}))
            }
            Some("answer") => {
                let (s, r) = oneshot::channel();
                self.send(Cmd::Answer {
                    peer: parse_peer(&field("peer")?)?,
                    answer: field("answer")?,
                    reply: s,
                })?;
                r.await.map_err(|_| "no reply".to_string())??;
                Ok(json!({}))
            }
            Some("leave") => {
                let (s, r) = oneshot::channel();
                self.send(Cmd::Leave {
                    peer: parse_peer(&field("peer")?)?,
                    reply: s,
                })?;
                r.await.map_err(|_| "no reply".to_string())??;
                Ok(json!({}))
            }
            _ => Err("unknown kind".into()),
        }
    }
}

/// The `kind` of a signalling message, so the caller can decide what to
/// charge before handing it over.
pub fn kind_of(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<Value>(body)
        .ok()?
        .get("kind")?
        .as_str()
        .map(str::to_string)
}

fn parse_peer(s: &str) -> Result<PeerId, String> {
    let b = hex::decode(s).map_err(|_| "bad peer id".to_string())?;
    b.try_into().map_err(|_| "bad peer id".to_string())
}

/// The address a packet to the outside world would leave from. No packet
/// is sent: a UDP `connect` only picks the route.
fn guess_local_ip() -> IpAddr {
    UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| s.connect("192.0.2.1:9").and_then(|_| s.local_addr()))
        .map(|a| a.ip())
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
}

// ---------------------------------------------------------------- the loop

fn run(socket: UdpSocket, public: SocketAddr, rx: Receiver<Cmd>) {
    let mut peers: Vec<Peer> = Vec::new();
    let mut queue: VecDeque<Propagated> = VecDeque::new();
    let mut buf = vec![0u8; 2000];
    loop {
        peers.retain(|p| p.rtc.is_alive() && !p.expired());
        loop {
            match rx.try_recv() {
                Ok(cmd) => handle_cmd(cmd, &mut peers, public),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        let mut timeout = Instant::now() + Duration::from_millis(100);
        for p in peers.iter_mut() {
            let t = poll_until_timeout(p, &mut queue, &socket);
            timeout = timeout.min(t);
        }
        if let Some(p) = queue.pop_front() {
            propagate(&p, &mut peers);
            continue;
        }
        let wait = timeout
            .saturating_duration_since(Instant::now())
            .max(Duration::from_millis(1));
        let _ = socket.set_read_timeout(Some(wait));
        if let Some(input) = read_socket_input(&socket, &mut buf) {
            if let Some(p) = peers.iter_mut().find(|p| p.rtc.accepts(&input)) {
                p.last_seen = Instant::now();
                p.handle_input(input);
            }
        }
        let now = Instant::now();
        for p in peers.iter_mut() {
            p.handle_input(Input::Timeout(now));
        }
    }
}

fn handle_cmd(cmd: Cmd, peers: &mut Vec<Peer>, public: SocketAddr) {
    match cmd {
        Cmd::Join { room, offer, reply } => {
            let _ = reply.send(join(room, &offer, peers, public));
        }
        Cmd::Poll { peer, reply } => {
            let r = match peers.iter_mut().find(|p| p.id == peer) {
                Some(p) => {
                    p.last_seen = Instant::now();
                    let room = p.room;
                    let offer = p.offer_out.take();
                    let n = peers.iter().filter(|q| q.room == room).count();
                    Ok((offer, n))
                }
                None => Err("unknown peer".to_string()),
            };
            let _ = reply.send(r);
        }
        Cmd::Answer {
            peer,
            answer,
            reply,
        } => {
            let r = match peers.iter_mut().find(|p| p.id == peer) {
                Some(p) => SdpAnswer::from_sdp_string(&answer)
                    .map_err(|e| format!("bad answer: {e:?}"))
                    .and_then(|a| p.handle_answer(a)),
                None => Err("unknown peer".to_string()),
            };
            let _ = reply.send(r);
        }
        Cmd::Leave { peer, reply } => {
            let r = match peers.iter_mut().find(|p| p.id == peer) {
                Some(p) => {
                    p.rtc.disconnect();
                    Ok(())
                }
                None => Err("unknown peer".to_string()),
            };
            peers.retain(|p| p.id != peer);
            let _ = reply.send(r);
        }
    }
}

fn join(
    room: Room,
    offer: &str,
    peers: &mut Vec<Peer>,
    public: SocketAddr,
) -> Result<(String, PeerId), String> {
    let offer = SdpOffer::from_sdp_string(offer).map_err(|e| format!("bad offer: {e:?}"))?;
    let mut rtc = Rtc::builder().build();
    let candidate = Candidate::host(public, "udp").map_err(|e| format!("candidate: {e:?}"))?;
    rtc.add_local_candidate(candidate);
    let answer = rtc
        .sdp_api()
        .accept_offer(offer)
        .map_err(|e| format!("offer refused: {e:?}"))?;
    let mut peer = Peer::new(room, rtc);
    for track in peers
        .iter()
        .filter(|p| p.room == room)
        .flat_map(|p| p.tracks_in.iter())
    {
        peer.handle_track_open(Arc::downgrade(&track.id));
    }
    let id = peer.id;
    peers.push(peer);
    Ok((answer.to_sdp_string(), id))
}

fn poll_until_timeout(
    peer: &mut Peer,
    queue: &mut VecDeque<Propagated>,
    socket: &UdpSocket,
) -> Instant {
    loop {
        if !peer.rtc.is_alive() {
            return Instant::now();
        }
        let propagated = peer.poll_output(socket);
        if let Propagated::Timeout(t) = propagated {
            return t;
        }
        queue.push_back(propagated);
    }
}

fn propagate(propagated: &Propagated, peers: &mut [Peer]) {
    let Some(origin) = propagated.origin() else {
        return;
    };
    let Some(room) = peers.iter().find(|p| p.id == origin).map(|p| p.room) else {
        return;
    };
    for peer in peers.iter_mut() {
        if peer.id == origin || peer.room != room {
            continue;
        }
        match propagated {
            Propagated::TrackOpen(_, track_in) => peer.handle_track_open(track_in.clone()),
            Propagated::MediaData(_, data) => peer.handle_media_data_out(origin, data),
            Propagated::Channel(_, text) => peer.handle_channel_relay(text),
            Propagated::KeyframeRequest(_, req, target, mid_in) => {
                if *target == peer.id {
                    peer.handle_keyframe_request(*req, *mid_in);
                }
            }
            Propagated::Noop | Propagated::Timeout(_) => {}
        }
    }
}

fn read_socket_input<'a>(socket: &UdpSocket, buf: &'a mut Vec<u8>) -> Option<Input<'a>> {
    buf.resize(2000, 0);
    match socket.recv_from(buf) {
        Ok((n, source)) => {
            buf.truncate(n);
            let Ok(contents) = buf.as_slice().try_into() else {
                return None;
            };
            Some(Input::Receive(
                Instant::now(),
                Receive {
                    proto: Protocol::Udp,
                    source,
                    destination: socket.local_addr().ok()?,
                    contents,
                },
            ))
        }
        Err(e) => match e.kind() {
            ErrorKind::WouldBlock | ErrorKind::TimedOut => None,
            _ => {
                tracing::warn!("media socket read failed: {e}");
                None
            }
        },
    }
}

// ---------------------------------------------------------------- one peer

struct TrackIn {
    origin: PeerId,
    mid: Mid,
    kind: MediaKind,
}

struct TrackInEntry {
    id: Arc<TrackIn>,
    last_keyframe_request: Option<Instant>,
}

struct TrackOut {
    track_in: Weak<TrackIn>,
    state: TrackOutState,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TrackOutState {
    ToOpen,
    Negotiating(Mid),
    Open(Mid),
}

impl TrackOut {
    fn mid(&self) -> Option<Mid> {
        match self.state {
            TrackOutState::ToOpen => None,
            TrackOutState::Negotiating(m) | TrackOutState::Open(m) => Some(m),
        }
    }
}

struct Peer {
    id: PeerId,
    room: Room,
    rtc: Rtc,
    pending: Option<SdpPendingOffer>,
    /// A renegotiation offer waiting for the participant's next poll.
    offer_out: Option<String>,
    cid: Option<ChannelId>,
    tracks_in: Vec<TrackInEntry>,
    tracks_out: Vec<TrackOut>,
    chosen_rid: Option<Rid>,
    joined: Instant,
    connected: bool,
    last_seen: Instant,
}

#[allow(clippy::large_enum_variant)]
enum Propagated {
    Noop,
    Timeout(Instant),
    TrackOpen(PeerId, Weak<TrackIn>),
    MediaData(PeerId, MediaData),
    /// (from, request, track origin, origin's mid)
    KeyframeRequest(PeerId, KeyframeRequest, PeerId, Mid),
    /// A text from one participant for the others in the room.
    Channel(PeerId, String),
}

impl Propagated {
    fn origin(&self) -> Option<PeerId> {
        match self {
            Propagated::TrackOpen(c, _)
            | Propagated::MediaData(c, _)
            | Propagated::Channel(c, _)
            | Propagated::KeyframeRequest(c, _, _, _) => Some(*c),
            _ => None,
        }
    }
}

impl Peer {
    fn new(room: Room, rtc: Rtc) -> Peer {
        Peer {
            id: rand::random(),
            room,
            rtc,
            pending: None,
            offer_out: None,
            cid: None,
            tracks_in: Vec::new(),
            tracks_out: Vec::new(),
            chosen_rid: None,
            joined: Instant::now(),
            connected: false,
            last_seen: Instant::now(),
        }
    }

    fn expired(&self) -> bool {
        if self.connected {
            self.last_seen.elapsed() > IDLE_LIMIT
        } else {
            self.joined.elapsed() > CONNECT_GRACE
        }
    }

    fn handle_input(&mut self, input: Input) {
        if !self.rtc.is_alive() {
            return;
        }
        if let Err(e) = self.rtc.handle_input(input) {
            tracing::debug!("peer disconnected: {e:?}");
            self.rtc.disconnect();
        }
    }

    fn poll_output(&mut self, socket: &UdpSocket) -> Propagated {
        if !self.rtc.is_alive() {
            return Propagated::Noop;
        }
        if self.negotiate_if_needed() {
            return Propagated::Noop;
        }
        match self.rtc.poll_output() {
            Ok(output) => self.handle_output(output, socket),
            Err(e) => {
                tracing::debug!("peer poll_output failed: {e:?}");
                self.rtc.disconnect();
                Propagated::Noop
            }
        }
    }

    fn handle_output(&mut self, output: Output, socket: &UdpSocket) -> Propagated {
        match output {
            Output::Transmit(t) => {
                if let Err(e) = socket.send_to(&t.contents, t.destination) {
                    tracing::debug!("media send failed: {e}");
                }
                Propagated::Noop
            }
            Output::Timeout(t) => Propagated::Timeout(t),
            Output::Event(e) => match e {
                Event::Connected => {
                    self.connected = true;
                    self.last_seen = Instant::now();
                    Propagated::Noop
                }
                Event::IceConnectionStateChange(v) => {
                    if v == IceConnectionState::Disconnected {
                        self.rtc.disconnect();
                    }
                    Propagated::Noop
                }
                // Only media the participant sends is a track for the others;
                // the ones we add to send them are not, or every renegotiation
                // would breed tracks for everyone.
                Event::MediaAdded(m) => {
                    let ours = self.tracks_out.iter().any(|t| t.mid() == Some(m.mid));
                    if ours || !matches!(m.direction, Direction::RecvOnly | Direction::SendRecv) {
                        Propagated::Noop
                    } else {
                        self.handle_media_added(m.mid, m.kind)
                    }
                }
                Event::MediaData(data) => self.handle_media_data_in(data),
                Event::KeyframeRequest(req) => self.handle_incoming_keyframe_req(req),
                Event::ChannelOpen(cid, _) => {
                    self.cid = Some(cid);
                    Propagated::Noop
                }
                Event::ChannelData(data) => self.handle_channel_data(data),
                _ => Propagated::Noop,
            },
        }
    }

    fn handle_media_added(&mut self, mid: Mid, kind: MediaKind) -> Propagated {
        tracing::debug!(
            "unit: peer {} sends {:?} on {}",
            hex::encode(&self.id[..4]),
            kind,
            mid
        );
        let track_in = TrackInEntry {
            id: Arc::new(TrackIn {
                origin: self.id,
                mid,
                kind,
            }),
            last_keyframe_request: None,
        };
        let weak = Arc::downgrade(&track_in.id);
        self.tracks_in.push(track_in);
        Propagated::TrackOpen(self.id, weak)
    }

    fn handle_media_data_in(&mut self, data: MediaData) -> Propagated {
        self.last_seen = Instant::now();
        if !data.contiguous {
            self.request_keyframe_throttled(data.mid, data.rid, KeyframeRequestKind::Fir);
        }
        Propagated::MediaData(self.id, data)
    }

    fn request_keyframe_throttled(
        &mut self,
        mid: Mid,
        rid: Option<Rid>,
        kind: KeyframeRequestKind,
    ) {
        let Some(mut writer) = self.rtc.writer(mid) else {
            return;
        };
        let Some(entry) = self.tracks_in.iter_mut().find(|t| t.id.mid == mid) else {
            return;
        };
        if entry
            .last_keyframe_request
            .map(|t| t.elapsed() < Duration::from_secs(1))
            .unwrap_or(false)
        {
            return;
        }
        let _ = writer.request_keyframe(rid, kind);
        entry.last_keyframe_request = Some(Instant::now());
    }

    fn handle_incoming_keyframe_req(&self, mut req: KeyframeRequest) -> Propagated {
        let Some(track_out) = self.tracks_out.iter().find(|t| t.mid() == Some(req.mid)) else {
            return Propagated::Noop;
        };
        let Some(track_in) = track_out.track_in.upgrade() else {
            return Propagated::Noop;
        };
        req.rid = self.chosen_rid;
        Propagated::KeyframeRequest(self.id, req, track_in.origin, track_in.mid)
    }

    /// Offer the tracks that are waiting to be added. Returns true when an
    /// offer went out this round.
    fn negotiate_if_needed(&mut self) -> bool {
        if self.pending.is_some() || self.offer_out.is_some() {
            return false;
        }
        let mut change = self.rtc.sdp_api();
        for track in &mut self.tracks_out {
            if let TrackOutState::ToOpen = track.state {
                if let Some(track_in) = track.track_in.upgrade() {
                    let stream_id = hex::encode(track_in.origin);
                    let mid = change.add_media(
                        track_in.kind,
                        Direction::SendOnly,
                        Some(stream_id),
                        None,
                        None,
                    );
                    track.state = TrackOutState::Negotiating(mid);
                }
            }
        }
        if !change.has_changes() {
            return false;
        }
        let Some((offer, pending)) = change.apply() else {
            return false;
        };
        let sdp = offer.to_sdp_string();
        self.pending = Some(pending);
        match self.cid.and_then(|id| self.rtc.channel(id)) {
            Some(mut channel) => {
                if channel.write(false, sdp.as_bytes()).is_err() {
                    self.offer_out = Some(sdp);
                }
            }
            None => self.offer_out = Some(sdp),
        }
        true
    }

    /// SDP on the channel is for the unit; anything else (a hello, a
    /// reaction) goes to everyone else in the room, marked with its origin.
    fn handle_channel_data(&mut self, d: ChannelData) -> Propagated {
        if let Ok(text) = std::str::from_utf8(&d.data) {
            // An offer and an answer read the same; what tells them apart
            // is whether we are waiting for an answer.
            if text.starts_with('{') {
                if text.len() <= 4096 {
                    self.last_seen = Instant::now();
                    let wrapped = json!({"from": hex::encode(self.id), "data": text}).to_string();
                    return Propagated::Channel(self.id, wrapped);
                }
            } else if self.pending.is_some() {
                if let Ok(answer) = SdpAnswer::from_sdp_string(text) {
                    let _ = self.handle_answer(answer);
                }
            } else if let Ok(offer) = SdpOffer::from_sdp_string(text) {
                self.handle_offer(offer);
            }
        }
        Propagated::Noop
    }

    fn handle_channel_relay(&mut self, text: &str) {
        if let Some(mut channel) = self.cid.and_then(|id| self.rtc.channel(id)) {
            let _ = channel.write(false, text.as_bytes());
        }
    }

    fn handle_offer(&mut self, offer: SdpOffer) {
        let answer = match self.rtc.sdp_api().accept_offer(offer) {
            Ok(a) => a,
            Err(e) => {
                tracing::debug!("renegotiation offer refused: {e:?}");
                return;
            }
        };
        for track in &mut self.tracks_out {
            if let TrackOutState::Negotiating(_) = track.state {
                track.state = TrackOutState::ToOpen;
            }
        }
        self.pending = None;
        if let Some(mut channel) = self.cid.and_then(|id| self.rtc.channel(id)) {
            let _ = channel.write(false, answer.to_sdp_string().as_bytes());
        }
    }

    fn handle_answer(&mut self, answer: SdpAnswer) -> Result<(), String> {
        let Some(pending) = self.pending.take() else {
            return Err("no offer pending".into());
        };
        self.rtc
            .sdp_api()
            .accept_answer(pending, answer)
            .map_err(|e| format!("answer refused: {e:?}"))?;
        for track in &mut self.tracks_out {
            if let TrackOutState::Negotiating(m) = track.state {
                track.state = TrackOutState::Open(m);
            }
        }
        Ok(())
    }

    fn handle_track_open(&mut self, track_in: Weak<TrackIn>) {
        self.tracks_out.push(TrackOut {
            track_in,
            state: TrackOutState::ToOpen,
        });
    }

    fn handle_media_data_out(&mut self, origin: PeerId, data: &MediaData) {
        let Some(mid) = self
            .tracks_out
            .iter()
            .find(|o| {
                o.track_in
                    .upgrade()
                    .filter(|i| i.origin == origin && i.mid == data.mid)
                    .is_some()
            })
            .and_then(|o| o.mid())
        else {
            return;
        };
        // Simulcast: forward the highest layer only, for now.
        if data.rid.is_some() && data.rid != Some("h".into()) {
            return;
        }
        if self.chosen_rid != data.rid {
            self.chosen_rid = data.rid;
        }
        let Some(writer) = self.rtc.writer(mid) else {
            return;
        };
        let Some(pt) = writer.match_params(data.params) else {
            return;
        };
        if let Err(e) = writer.write(pt, data.network_time, data.time, data.data.clone()) {
            tracing::debug!("forward failed: {e:?}");
            self.rtc.disconnect();
        }
    }

    fn handle_keyframe_request(&mut self, req: KeyframeRequest, mid_in: Mid) {
        if !self.tracks_in.iter().any(|i| i.id.mid == mid_in) {
            return;
        }
        let Some(mut writer) = self.rtc.writer(mid_in) else {
            return;
        };
        if let Err(e) = writer.request_keyframe(req.rid, req.kind) {
            tracing::debug!("request_keyframe failed: {e:?}");
        }
    }
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use str0m::media::{Frequency, MediaTime};

    /// An in-process participant: its own socket and `Rtc`, driven by hand.
    struct Participant {
        rtc: Rtc,
        socket: UdpSocket,
        pending: Option<SdpPendingOffer>,
        mid: Option<Mid>,
        connected: bool,
        got_media: usize,
    }

    impl Participant {
        fn new() -> Participant {
            let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
            let mut rtc = Rtc::builder().build();
            rtc.add_local_candidate(Candidate::host(socket.local_addr().unwrap(), "udp").unwrap());
            Participant {
                rtc,
                socket,
                pending: None,
                mid: None,
                connected: false,
                got_media: 0,
            }
        }

        fn offer(&mut self) -> String {
            let mut change = self.rtc.sdp_api();
            self.mid =
                Some(change.add_media(MediaKind::Audio, Direction::SendOnly, None, None, None));
            let (offer, pending) = change.apply().unwrap();
            self.pending = Some(pending);
            offer.to_sdp_string()
        }

        fn take_answer(&mut self, sdp: &str) {
            let answer = SdpAnswer::from_sdp_string(sdp).unwrap();
            self.rtc
                .sdp_api()
                .accept_answer(self.pending.take().unwrap(), answer)
                .unwrap();
        }

        fn take_offer(&mut self, sdp: &str) -> String {
            let offer = SdpOffer::from_sdp_string(sdp).unwrap();
            self.rtc
                .sdp_api()
                .accept_offer(offer)
                .unwrap()
                .to_sdp_string()
        }

        fn pump(&mut self, budget: Duration) {
            let end = Instant::now() + budget;
            let mut buf = vec![0u8; 2000];
            loop {
                let timeout = loop {
                    match self.rtc.poll_output().unwrap() {
                        Output::Transmit(t) => {
                            self.socket.send_to(&t.contents, t.destination).unwrap();
                        }
                        Output::Timeout(t) => break t,
                        Output::Event(Event::Connected) => self.connected = true,
                        Output::Event(Event::MediaData(_)) => self.got_media += 1,
                        Output::Event(_) => {}
                    }
                };
                let now = Instant::now();
                if now >= end {
                    break;
                }
                let wait = timeout
                    .saturating_duration_since(now)
                    .min(end - now)
                    .max(Duration::from_millis(1));
                self.socket.set_read_timeout(Some(wait)).unwrap();
                buf.resize(2000, 0);
                if let Ok((n, source)) = self.socket.recv_from(&mut buf) {
                    buf.truncate(n);
                    if let Ok(contents) = buf.as_slice().try_into() {
                        self.rtc
                            .handle_input(Input::Receive(
                                Instant::now(),
                                Receive {
                                    proto: Protocol::Udp,
                                    source,
                                    destination: self.socket.local_addr().unwrap(),
                                    contents,
                                },
                            ))
                            .unwrap();
                    }
                }
                self.rtc
                    .handle_input(Input::Timeout(Instant::now()))
                    .unwrap();
            }
        }

        fn write_audio(&mut self, ts: u64) {
            let Some(mid) = self.mid else { return };
            let Some(writer) = self.rtc.writer(mid) else {
                return;
            };
            let pt = writer.payload_params().next().unwrap().pt();
            let _ = writer.write(
                pt,
                Instant::now(),
                MediaTime::new(ts, Frequency::FORTY_EIGHT_KHZ),
                vec![1u8; 160],
            );
        }
    }

    #[test]
    fn two_participants_hear_each_other() {
        let sfu = Sfu::start("127.0.0.1:0", None).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let room: Room = [7u8; 32];
        let other: Room = [8u8; 32];
        let sig = |room: Room, v: Value| rt.block_on(sfu.signal(room, v.to_string().as_bytes()));

        let mut a = Participant::new();
        let mut b = Participant::new();
        let ja = sig(room, json!({"kind": "join", "offer": a.offer()})).unwrap();
        a.take_answer(ja["answer"].as_str().unwrap());
        let pa = ja["peer"].as_str().unwrap().to_string();
        let jb = sig(room, json!({"kind": "join", "offer": b.offer()})).unwrap();
        b.take_answer(jb["answer"].as_str().unwrap());
        let pb = jb["peer"].as_str().unwrap().to_string();
        // a third participant in another room must never hear these two
        let mut c = Participant::new();
        let jc = sig(other, json!({"kind": "join", "offer": c.offer()})).unwrap();
        c.take_answer(jc["answer"].as_str().unwrap());
        let pc = jc["peer"].as_str().unwrap().to_string();

        let start = Instant::now();
        let mut ts = 0u64;
        while start.elapsed() < Duration::from_secs(30) && (a.got_media == 0 || b.got_media == 0) {
            for p in [&mut a, &mut b, &mut c] {
                p.pump(Duration::from_millis(20));
            }
            for (p, id) in [(&mut a, &pa), (&mut b, &pb), (&mut c, &pc)] {
                let r = sig(room, json!({"kind": "poll", "peer": id})).unwrap();
                if let Some(o) = r["offer"].as_str() {
                    let answer = p.take_offer(o);
                    sig(
                        room,
                        json!({"kind": "answer", "peer": id, "answer": answer}),
                    )
                    .unwrap();
                }
            }
            for p in [&mut a, &mut b, &mut c] {
                if p.connected {
                    p.write_audio(ts);
                }
            }
            ts += 960;
        }
        assert!(a.got_media > 0, "a heard nothing");
        assert!(b.got_media > 0, "b heard nothing");
        assert_eq!(c.got_media, 0, "c is in another room");
        let r = sig(room, json!({"kind": "poll", "peer": pa})).unwrap();
        assert_eq!(r["peers"], 2);
        sig(room, json!({"kind": "leave", "peer": pa})).unwrap();
        assert!(sig(room, json!({"kind": "poll", "peer": pa})).is_err());
        let r = sig(room, json!({"kind": "poll", "peer": pb})).unwrap();
        assert_eq!(r["peers"], 1);
        assert!(sig(room, json!({"kind": "dance"})).is_err());
    }
}
