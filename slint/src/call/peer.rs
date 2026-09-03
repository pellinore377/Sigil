//! The WebRTC peer for a call, on its own thread the way `str0m` is driven:
//! poll outputs, feed inputs, drive time. It owns the socket, the sound
//! devices, the Opus codec and the frame cipher; the session manager on the
//! UI thread talks to it with commands and hears back through events.
//!
//! One audio track goes out; every other participant's track comes in as a
//! media the forwarding unit offers by renegotiation, on the data channel
//! once it is open (or through `call.poll` before that). The unit names the
//! sender of each incoming media in the SDP's `msid`, which is how frames
//! are attributed to people and their cipher nonces.

use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use str0m::change::{SdpAnswer, SdpOffer, SdpPendingOffer};
use str0m::channel::ChannelId;
use str0m::media::{Direction, Frequency, MediaKind, MediaTime, Mid};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc};

use super::audio::{self, Capture, Playback, FRAME};
use super::crypt::FrameCrypt;

pub enum PeerCmd {
    /// The unit's answer to our first offer, and the peer id it gave us.
    TakeAnswer {
        answer: String,
        peer: [u8; 16],
    },
    /// A renegotiation offer fetched with `call.poll`; the answer goes back
    /// through the callback (to `call.answer`).
    TakeOffer(String, Box<dyn FnOnce(String) + Send>),
    SetMuted(bool),
    /// A text for everyone in the room, through the unit's channel relay.
    Send(String),
    AddKey(u8, [u8; 32]),
    SetDevice {
        kind: String,
        id: String,
    },
    Stop,
}

pub enum PeerEvent {
    /// Our first offer, to hand to `call.join`.
    Offer(String),
    Connected,
    ChannelOpen,
    /// A text from another participant, relayed by the unit with its origin.
    Message {
        from: String,
        text: String,
    },
    /// Loudness this window: ours, and each origin's.
    Levels {
        local: f32,
        remotes: Vec<(String, f32)>,
    },
    /// A frame arrived under a key we do not hold.
    NeedKey(u8),
    Disconnected(String),
}

pub struct PeerHandle {
    pub cmd: Sender<PeerCmd>,
}

/// Start the peer; `on_event` is called from the peer's thread.
pub fn spawn(on_event: Box<dyn Fn(PeerEvent) + Send>) -> PeerHandle {
    let (cmd_tx, cmd_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("sigil-call".into())
        .spawn(move || {
            if let Err(e) = run(cmd_rx, &on_event) {
                on_event(PeerEvent::Disconnected(format!("{e:#}")));
            }
        })
        .expect("call thread");
    PeerHandle { cmd: cmd_tx }
}

/// Where a packet to the wider world would leave from, if anywhere.
fn guess_local_ip() -> Option<IpAddr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:80").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

/// `a=mid:` → the `a=msid:` stream id of each media section: the unit
/// names the originating peer there.
fn msid_map(sdp: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let (mut mid, mut msid) = (None::<String>, None::<String>);
    let flush =
        |mid: &mut Option<String>, msid: &mut Option<String>, out: &mut Vec<(String, String)>| {
            if let (Some(m), Some(s)) = (mid.take(), msid.take()) {
                out.push((m, s));
            }
            *mid = None;
            *msid = None;
        };
    for line in sdp.lines() {
        let line = line.trim();
        if line.starts_with("m=") {
            flush(&mut mid, &mut msid, &mut out);
        } else if let Some(v) = line.strip_prefix("a=mid:") {
            mid = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("a=msid:") {
            msid = Some(v.split_whitespace().next().unwrap_or("").to_string());
        }
    }
    flush(&mut mid, &mut msid, &mut out);
    out
}

fn peer_bytes(hex_id: &str) -> Option<[u8; 16]> {
    let v = hex::decode(hex_id).ok()?;
    v.try_into().ok()
}

struct Remote {
    origin: String,
    decoder: opus::Decoder,
    level: f32,
}

fn run(cmd_rx: Receiver<PeerCmd>, on_event: &dyn Fn(PeerEvent)) -> anyhow::Result<()> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    let port = socket.local_addr()?.port();
    let mut rtc = Rtc::builder().build();
    let mut ips: Vec<IpAddr> = vec![IpAddr::V4(Ipv4Addr::LOCALHOST)];
    if let Some(ip) = guess_local_ip() {
        ips.push(ip);
    }
    for ip in &ips {
        if let Ok(c) = Candidate::host(SocketAddr::new(*ip, port), "udp") {
            rtc.add_local_candidate(c);
        }
    }

    // our offer: one audio track out, and the channel the unit talks on
    let mut change = rtc.sdp_api();
    let out_mid: Mid = change.add_media(MediaKind::Audio, Direction::SendOnly, None, None, None);
    let cid: ChannelId = change.add_channel("sigil".into());
    let (offer, pending) = change
        .apply()
        .ok_or_else(|| anyhow::anyhow!("nothing to offer"))?;
    let mut pending: Option<SdpPendingOffer> = Some(pending);
    on_event(PeerEvent::Offer(offer.to_sdp_string()));

    // sound
    let (cap_tx, cap_rx) = mpsc::channel::<Vec<f32>>();
    let mut capture = Capture::start(None, cap_tx.clone()).ok();
    let mut playback = Playback::start(None).ok();
    let mut encoder =
        opus::Encoder::new(audio::RATE, opus::Channels::Mono, opus::Application::Voip)?;
    let mut enc_buf = vec![0u8; 1500];
    let mut crypt: Option<FrameCrypt> = None;
    let mut pending_keys: Vec<(u8, [u8; 32])> = Vec::new();
    let mut muted = false;
    let mut rtp_ts: u64 = 0;
    let mut local_level = 0.0f32;

    let mut remotes: HashMap<Mid, Remote> = HashMap::new();
    let mut origins: BTreeMap<String, String> = BTreeMap::new(); // mid -> origin hex
    let mut channel_open = false;
    let mut asked_keys: Vec<u8> = Vec::new();
    let mut last_levels = Instant::now();
    let mut buf = vec![0u8; 2000];

    loop {
        // commands from the manager
        loop {
            match cmd_rx.try_recv() {
                Ok(PeerCmd::TakeAnswer { answer, peer }) => {
                    let mut c = FrameCrypt::new(peer);
                    for (kid, key) in pending_keys.drain(..) {
                        c.add_key(kid, &key);
                    }
                    crypt = Some(c);
                    if let (Some(p), Ok(a)) = (pending.take(), SdpAnswer::from_sdp_string(&answer))
                    {
                        if let Err(e) = rtc.sdp_api().accept_answer(p, a) {
                            anyhow::bail!("the unit's answer was refused: {e:?}");
                        }
                    }
                }
                Ok(PeerCmd::TakeOffer(sdp, reply)) => {
                    for (mid, origin) in msid_map(&sdp) {
                        origins.insert(mid, origin);
                    }
                    if let Ok(offer) = SdpOffer::from_sdp_string(&sdp) {
                        if let Ok(answer) = rtc.sdp_api().accept_offer(offer) {
                            reply(answer.to_sdp_string());
                        }
                    }
                }
                Ok(PeerCmd::SetMuted(m)) => muted = m,
                Ok(PeerCmd::Send(text)) => {
                    if let Some(mut ch) = rtc.channel(cid) {
                        let _ = ch.write(false, text.as_bytes());
                    }
                }
                Ok(PeerCmd::AddKey(kid, key)) => {
                    asked_keys.retain(|k| *k != kid);
                    match crypt.as_mut() {
                        Some(c) => c.add_key(kid, &key),
                        None => pending_keys.push((kid, key)),
                    }
                }
                Ok(PeerCmd::SetDevice { kind, id }) => {
                    let name = (id != "default" && !id.is_empty()).then_some(id.as_str());
                    if kind == "mic" {
                        capture = Capture::start(name, cap_tx.clone()).ok();
                    } else {
                        playback = Playback::start(name).ok();
                    }
                }
                Ok(PeerCmd::Stop) => {
                    rtc.disconnect();
                    return Ok(());
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
            }
        }

        // the microphone
        while let Ok(frame) = cap_rx.try_recv() {
            if muted {
                local_level = 0.0;
                continue;
            }
            local_level = local_level.max(audio::rms(&frame));
            let Some(c) = crypt.as_mut() else { continue };
            if !c.ready() {
                continue;
            }
            let Some(writer) = rtc.writer(out_mid) else {
                continue;
            };
            let Some(pt) = writer.payload_params().next().map(|p| p.pt()) else {
                continue;
            };
            let n = match encoder.encode_float(&frame[..FRAME.min(frame.len())], &mut enc_buf) {
                Ok(n) => n,
                Err(_) => continue,
            };
            let Some(sealed) = c.seal(&enc_buf[..n]) else {
                continue;
            };
            let _ = writer.write(
                pt,
                Instant::now(),
                MediaTime::new(rtp_ts, Frequency::FORTY_EIGHT_KHZ),
                sealed,
            );
            rtp_ts = rtp_ts.wrapping_add(FRAME as u64);
        }

        // the peer's outputs
        let timeout = loop {
            match rtc.poll_output()? {
                Output::Transmit(t) => {
                    let _ = socket.send_to(&t.contents, t.destination);
                }
                Output::Timeout(t) => break t,
                Output::Event(ev) => match ev {
                    Event::Connected => {
                        tracing::debug!("call: connected to the unit");
                        on_event(PeerEvent::Connected);
                    }
                    Event::IceConnectionStateChange(IceConnectionState::Disconnected) => {
                        rtc.disconnect();
                    }
                    Event::ChannelOpen(_, _) => {
                        channel_open = true;
                        on_event(PeerEvent::ChannelOpen);
                    }
                    Event::ChannelData(d) => {
                        let Ok(text) = std::str::from_utf8(&d.data) else {
                            continue;
                        };
                        if let Ok(offer) = SdpOffer::from_sdp_string(text) {
                            for (mid, origin) in msid_map(text) {
                                origins.insert(mid, origin);
                            }
                            if let Ok(answer) = rtc.sdp_api().accept_offer(offer) {
                                if let Some(mut ch) = rtc.channel(cid) {
                                    let _ = ch.write(false, answer.to_sdp_string().as_bytes());
                                }
                            }
                        } else if SdpAnswer::from_sdp_string(text).is_ok() {
                            // we never offer on the channel; nothing pending
                        } else if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
                            let from = v["from"].as_str().unwrap_or("").to_string();
                            let inner = v["data"].as_str().unwrap_or("").to_string();
                            if !inner.is_empty() {
                                on_event(PeerEvent::Message { from, text: inner });
                            }
                        }
                    }
                    Event::MediaAdded(m) => {
                        tracing::debug!("call: media {} {:?} {:?}", m.mid, m.kind, m.direction);
                        // ours goes out; only what comes in gets a decoder
                        if m.mid != out_mid
                            && m.kind == MediaKind::Audio
                            && matches!(m.direction, Direction::RecvOnly | Direction::SendRecv)
                        {
                            if let Ok(d) = opus::Decoder::new(audio::RATE, opus::Channels::Mono) {
                                let origin =
                                    origins.get(&m.mid.to_string()).cloned().unwrap_or_default();
                                remotes.insert(
                                    m.mid,
                                    Remote {
                                        origin,
                                        decoder: d,
                                        level: 0.0,
                                    },
                                );
                            }
                        }
                    }
                    Event::MediaData(data) => {
                        let Some(r) = remotes.get_mut(&data.mid) else {
                            tracing::debug!("media on unknown mid {}", data.mid);
                            continue;
                        };
                        if r.origin.is_empty() {
                            if let Some(o) = origins.get(&data.mid.to_string()) {
                                r.origin = o.clone();
                            }
                        }
                        let Some(c) = crypt.as_ref() else { continue };
                        let Some(sender) = peer_bytes(&r.origin) else {
                            tracing::debug!("media on mid {} with no known origin", data.mid);
                            continue;
                        };
                        let plain = match c.open(&sender, &data.data) {
                            Ok(Some(p)) => p,
                            Ok(None) => {
                                tracing::debug!("a frame from {} would not open", r.origin);
                                continue;
                            }
                            Err(kid) => {
                                if !asked_keys.contains(&kid) {
                                    asked_keys.push(kid);
                                    on_event(PeerEvent::NeedKey(kid));
                                }
                                continue;
                            }
                        };
                        let mut pcm = vec![0f32; FRAME * 3];
                        if let Ok(n) = r.decoder.decode_float(&plain, &mut pcm, false) {
                            pcm.truncate(n);
                            r.level = r.level.max(audio::rms(&pcm));
                            if let Some(p) = playback.as_ref() {
                                p.push(&pcm);
                            }
                        }
                    }
                    _ => {}
                },
            }
        };

        if !rtc.is_alive() {
            on_event(PeerEvent::Disconnected("the connection dropped".into()));
            return Ok(());
        }

        // levels, ten times a second
        if last_levels.elapsed() >= Duration::from_millis(100) {
            last_levels = Instant::now();
            let mut by_origin: BTreeMap<String, f32> = BTreeMap::new();
            for r in remotes.values_mut() {
                if !r.origin.is_empty() {
                    let e = by_origin.entry(r.origin.clone()).or_insert(0.0);
                    *e = e.max(r.level);
                }
                r.level = 0.0;
            }
            if let Some(p) = playback.as_ref() {
                let _ = p.take_level();
            }
            on_event(PeerEvent::Levels {
                local: local_level,
                remotes: by_origin.into_iter().collect(),
            });
            local_level = 0.0;
        }
        let _ = (&capture, channel_open);

        // the socket, for as long as the peer can wait (never long: the
        // microphone and the manager want attention too)
        let now = Instant::now();
        let wait = timeout
            .saturating_duration_since(now)
            .min(Duration::from_millis(5))
            .max(Duration::from_millis(1));
        socket.set_read_timeout(Some(wait))?;
        buf.resize(2000, 0);
        match socket.recv_from(&mut buf) {
            Ok((n, source)) => {
                buf.truncate(n);
                if let Ok(contents) = buf.as_slice().try_into() {
                    let _ = rtc.handle_input(Input::Receive(
                        Instant::now(),
                        Receive {
                            proto: Protocol::Udp,
                            source,
                            destination: SocketAddr::new(local_for(source, &ips), port),
                            contents,
                        },
                    ));
                }
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => tracing::debug!("call socket: {e}"),
        }
        rtc.handle_input(Input::Timeout(Instant::now()))?;
    }
}

/// The local address a packet from `source` was received on: loopback for
/// loopback, otherwise the outward-facing one.
fn local_for(source: SocketAddr, ips: &[IpAddr]) -> IpAddr {
    if source.ip().is_loopback() {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    } else {
        ips.iter()
            .copied()
            .find(|ip| !ip.is_loopback())
            .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn msids_name_the_origin_of_each_media() {
        let sdp = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=mid:1\r\na=msid:aabbcc track1\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=mid:2\r\na=msid:ddeeff t2\r\n";
        let m = msid_map(sdp);
        assert_eq!(
            m,
            vec![
                ("1".to_string(), "aabbcc".to_string()),
                ("2".to_string(), "ddeeff".to_string())
            ]
        );
    }
}
