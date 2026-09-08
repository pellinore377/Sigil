#![forbid(unsafe_code)]
mod common;
use common::{proxy::proxy, Packet, Peer};
use rtc::peer_connection::sdp::RTCSessionDescription;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use webrtc::media_stream::track_local::TrackLocal;
async fn str_sink(offer: &str) -> (String, mpsc::Receiver<Packet>, tokio::task::JoinHandle<()>) {
    str0m::crypto::from_feature_flags().install_process_default();
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let address = socket.local_addr().unwrap();
    let mut rtc = str0m::Rtc::builder()
        .set_rtp_mode(true)
        .set_ice_lite(true)
        .build(Instant::now());
    rtc.add_local_candidate(str0m::Candidate::host(address, "udp").unwrap())
        .unwrap();
    let answer = rtc
        .sdp_api()
        .accept_offer(str0m::change::SdpOffer::from_sdp_string(offer).unwrap())
        .unwrap()
        .to_sdp_string();
    let (tx, rx) = mpsc::channel(512);
    let task = tokio::spawn(async move {
        let mut buf = [0; 2048];
        loop {
            let timeout = loop {
                match rtc.poll_output() {
                    Err(str0m::RtcError::RemoteSdp(_)) => return,
                    Err(e) => panic!("{e}"),
                    Ok(value) => match value {
                        str0m::Output::Timeout(t) => break t,
                        str0m::Output::Transmit(t) => {
                            socket.send_to(&t.contents, t.destination).await.unwrap();
                        }
                        str0m::Output::Event(str0m::Event::RtpPacket(p)) => {
                            let _ = tx.try_send(Packet {
                                pt: *p.header.payload_type,
                                seq: *p.seq_no as u16,
                                ssrc: *p.header.ssrc,
                                received: Instant::now(),
                                payload: p.payload.to_vec(),
                            });
                        }
                        _ => (),
                    },
                }
            };
            tokio::select! {
                r=socket.recv_from(&mut buf)=>{
                    let(n,source)=r.unwrap();
                    if let Ok(received)=str0m::net::Receive::new(str0m::net::Protocol::Udp,source,address,&buf[..n]){
                        let input=str0m::Input::Receive(Instant::now(),received);
                        if rtc.accepts(&input) && rtc.handle_input(input).is_err(){return;}
                    }
                },
                _=tokio::time::sleep_until(timeout.into())=>{rtc.handle_input(str0m::Input::Timeout(Instant::now())).unwrap();}
            }
        }
    });
    (answer, rx, task)
}
async fn case(str0m: bool, lossy: bool, bad_fingerprint: bool, reconnect: bool, relay: bool) {
    for round in 0..if reconnect { 2 } else { 1 } {
        let mut source = Peer::new(false, relay).await;
        let mut tracks = Vec::new();
        for n in 1..=3 {
            tracks.push(source.track(n).await.0);
        }
        let offer = source
            .pc
            .create_offer(Some(rtc::peer_connection::configuration::RTCOfferOptions {
                ice_restart: false,
            }))
            .await
            .unwrap();
        let mut offer = source.local(offer).await;
        if relay {
            let candidates: Vec<_> = offer
                .lines()
                .filter(|l| l.starts_with("a=candidate:"))
                .collect();
            assert!(!candidates.is_empty());
            assert!(
                candidates.iter().all(|line| line.contains(" typ relay")),
                "relay-only gathering disclosed a host candidate"
            );
        }
        if bad_fingerprint {
            offer = corrupt_fingerprint(&offer);
        }
        let mut other = None;
        let mut task = None;
        let (answer, mut packets) = if str0m {
            let (a, p, t) = str_sink(&offer).await;
            task = Some(t);
            (a, p)
        } else {
            let mut peer = Peer::new(true, false).await;
            peer.pc
                .set_remote_description(RTCSessionDescription::offer(offer).unwrap())
                .await
                .unwrap();
            let answer = peer.pc.create_answer(None).await.unwrap();
            let answer = peer.local(answer).await;
            let (_, empty) = mpsc::channel(1);
            let packets = std::mem::replace(&mut peer.packets, empty);
            other = Some(peer);
            (answer, packets)
        };
        let (answer, proxy, altered) = proxy(answer, lossy).await;
        source
            .pc
            .set_remote_description(RTCSessionDescription::answer(answer).unwrap())
            .await
            .unwrap();
        let start = Instant::now();
        let mut seen = std::collections::BTreeSet::new();
        let mut first = None;
        let mut ssrcs = std::collections::BTreeMap::new();
        for seq in (round * 150)..((round + 1) * 150u16) {
            for (i, track) in tracks.iter().enumerate() {
                let pt = if i == 0 { 111 } else { 96 };
                let mut payload = vec![0u8; 400];
                payload[0] = 0x10;
                payload[1] = i as u8;
                payload[2..4].copy_from_slice(&seq.to_be_bytes());
                let result = track
                    .write_rtp(rtc::rtp::packet::Packet {
                        header: rtc::rtp::header::Header {
                            version: 2,
                            payload_type: pt,
                            sequence_number: seq,
                            timestamp: u32::from(seq) * 960,
                            ssrc: i as u32 + 1,
                            marker: true,
                            ..Default::default()
                        },
                        payload: payload.into(),
                    })
                    .await;
                if !bad_fingerprint {
                    result.unwrap();
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
            while let Ok(p) = packets.try_recv() {
                assert_eq!(p.payload.len(), 400);
                assert_eq!(
                    u16::from_be_bytes(p.payload[2..4].try_into().unwrap()),
                    p.seq
                );
                assert_eq!(p.pt, if p.payload[1] == 0 { 111 } else { 96 });
                assert_ne!(p.ssrc, 0);
                assert_eq!(*ssrcs.entry(p.payload[1]).or_insert(p.ssrc), p.ssrc);
                assert!(
                    seen.insert((p.payload[1], p.seq)),
                    "duplicate media delivered"
                );
                first.get_or_insert(p.received.duration_since(start));
            }
        }
        if bad_fingerprint {
            assert!(seen.is_empty());
        } else {
            let tracks: std::collections::BTreeSet<_> = seen.iter().map(|p| p.0).collect();
            assert_eq!(
                tracks.len(),
                3,
                "str0m={str0m}, round={round}; received {tracks:?}"
            );
            assert!(first.unwrap() < Duration::from_secs(2));
            assert!(seen.len() > 100);
            assert_eq!(
                ssrcs
                    .values()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len(),
                3
            );
            if lossy {
                assert!(altered.load(std::sync::atomic::Ordering::Relaxed) > 0);
            }
            eprintln!(
                "str0m={str0m}, loss={lossy}, round={round}: first RTP={:?}, packets={}",
                first.unwrap(),
                seen.len()
            );
        }
        proxy.abort();
        if let Some(peer) = other {
            peer.pc.close().await.unwrap();
        }
        if let Some(t) = task {
            if t.is_finished() {
                t.await.unwrap();
            } else {
                t.abort();
            }
        }
        source.pc.close().await.unwrap();
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn same_audio_video_screen_rtp_interoperability() {
    for str0m in [false, true] {
        case(str0m, false, false, false, false).await;
        case(str0m, true, false, true, false).await;
        case(str0m, false, true, false, false).await;
    }
}
fn corrupt_fingerprint(sdp: &str) -> String {
    sdp.lines()
        .map(|line| {
            if line.starts_with("a=fingerprint:") {
                let mut value = line.to_owned();
                let end = value.len();
                value.replace_range(
                    end - 2..,
                    if &line[end - 2..] == "00" { "01" } else { "00" },
                );
                value
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\r\n")
        + "\r\n"
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires local synthetic coturn fixture"]
async fn same_turn_relay_interoperability() {
    for candidate in [false, true] {
        case(candidate, false, false, false, true).await;
    }
}
