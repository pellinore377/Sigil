use super::*;
use crate::{claims::tests::pair, incoming::tests::trust, network::tests::Fixture};
#[path = "../../../calls/tests/common/mod.rs"]
mod common;
use common::{offer, Peer};
use sigil_calls::{Downstream, MediaKind, SignedConnect};
use std::sync::Arc;
use std::time::{Duration, Instant};
use webrtc::media_stream::track_local::{static_rtp::TrackLocalStaticRTP, TrackLocal};

fn restart(dir: &std::path::Path, port: u16) -> Fixture {
    let (app, maintenance) = sigil_server::router_with_maintenance(
        sigil_server::store::Store::open(&dir.join("server.db")).unwrap(),
        sigil_server::auth::AdminToken::load_or_create(&dir.join("admin.token")).unwrap(),
    );
    Fixture::maintained_at(app, maintenance, port)
}
struct Endpoint {
    peer: Peer,
    tracks: Vec<Arc<TrackLocalStaticRTP>>,
    streams: Vec<Downstream>,
    proof: SignedConnect,
}
async fn connect(store: &mut ClientStore, id: Id, relay: bool, now: u64) -> Endpoint {
    let credentials = store.call_relay_online(id, now).unwrap();
    assert_eq!(credentials.is_some(), relay);
    let server = credentials.map(|value| rtc::peer_connection::configuration::RTCIceServer {
        urls: value.urls,
        username: value.username,
        credential: value.credential.to_string(),
    });
    let mut peer = Peer::configured(false, server).await;
    let mut tracks = Vec::new();
    let mut uploads = Vec::new();
    for n in 1..=3 {
        let (track, transceiver) = peer.track(n).await;
        tracks.push(track);
        uploads.push(transceiver);
    }
    let record = load(&store.db, &store.key, &id).unwrap();
    let (sdp, layout) = offer(
        &mut peer,
        &record.state.roster.roster,
        record.own_id().unwrap(),
        uploads,
    )
    .await;
    if relay {
        let candidates: Vec<_> = sdp
            .lines()
            .filter(|s| s.starts_with("a=candidate:"))
            .collect();
        assert!(!candidates.is_empty());
        assert!(candidates.iter().all(|s| s.contains(" typ relay")));
    }
    let proof = store.prepare_call_connection(id, sdp, layout, now).unwrap();
    let answer = store.connect_call_online(&proof, now).unwrap();
    assert_eq!(
        answer.sdp,
        store.connect_call_online(&proof, now).unwrap().sdp
    );
    peer.pc
        .set_remote_description(
            rtc::peer_connection::sdp::RTCSessionDescription::answer(answer.sdp).unwrap(),
        )
        .await
        .unwrap();
    Endpoint {
        peer,
        tracks,
        streams: answer.streams,
        proof,
    }
}
async fn media(
    stores: &mut [&mut ClientStore; 2],
    handles: &mut [Media; 2],
    endpoints: &mut [Endpoint; 2],
    now: u64,
) {
    let mut received = [[false; 3]; 2];
    let mut sequence = [[0u16; 3]; 2];
    let started = Instant::now();
    for frame in 0..60u64 {
        for i in 0..2 {
            for kind in [MediaKind::Audio, MediaKind::Camera, MediaKind::Screen] {
                let mut bytes = vec![
                    i as u8;
                    match kind {
                        MediaKind::Audio => 160,
                        MediaKind::Camera => 8192,
                        MediaKind::Screen => 24576,
                    }
                ];
                bytes[..8].copy_from_slice(&frame.to_be_bytes());
                let packets = stores[i]
                    .seal_call_packets(
                        &mut handles[i],
                        kind,
                        frame,
                        kind != MediaKind::Audio,
                        &bytes,
                        now,
                    )
                    .unwrap();
                let count = packets.len();
                for (n, packet) in packets.into_iter().enumerate() {
                    let seq = &mut sequence[i][kind as usize];
                    endpoints[i].tracks[kind as usize]
                        .write_rtp(rtc::rtp::packet::Packet {
                            header: rtc::rtp::header::Header {
                                version: 2,
                                payload_type: if kind == MediaKind::Audio { 111 } else { 96 },
                                sequence_number: *seq,
                                timestamp: frame as u32
                                    * if kind == MediaKind::Audio { 960 } else { 3000 },
                                ssrc: kind as u32 + 1,
                                marker: n + 1 == count,
                                ..Default::default()
                            },
                            payload: packet.into(),
                        })
                        .await
                        .unwrap();
                    *seq = seq.wrapping_add(1);
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
        for i in 0..2 {
            while let Ok(packet) = endpoints[i].peer.packets.try_recv() {
                assert!(packet.seq < 3000 && packet.received >= started);
                let route = &endpoints[i]
                    .streams
                    .iter()
                    .find(|s| s.ssrc == packet.ssrc)
                    .unwrap()
                    .track;
                assert_eq!(
                    packet.pt,
                    if route.kind == MediaKind::Audio {
                        111
                    } else {
                        96
                    }
                );
                if let Some(frame) = stores[i]
                    .open_call_packet(
                        &mut handles[i],
                        route.sender,
                        route.kind,
                        &packet.payload,
                        now,
                    )
                    .unwrap()
                {
                    assert!(frame.data[8..].iter().all(|b| *b == (1 - i) as u8));
                    assert_eq!(
                        u64::from_be_bytes(frame.data[..8].try_into().unwrap()),
                        frame.timestamp
                    );
                    received[i][route.kind as usize] = true;
                }
            }
        }
        if received.iter().all(|v| v.iter().all(|v| *v)) {
            return;
        }
    }
    panic!("media not established: {received:?}");
}
fn run(relay: bool) {
    let (dir, fixture, mut alice, mut bob, now) = pair();
    super::tests::configure(dir.path());
    if relay {
        let mut server = sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap();
        let mut settings = server.call_configuration().unwrap().settings.unwrap();
        settings.turn_urls =
            vec![std::env::var("SIGIL_TEST_TURN_URL").expect("synthetic TURN URL")];
        server
            .configure_calls(sigil_server::call_config::Configure {
                expected_revision: 1,
                settings: Some(settings),
                turn_secret: sigil_server::service_config::SecretUpdate::Set(Zeroizing::new(
                    "synthetic-rest-secret-for-call-tests".into(),
                )),
            })
            .unwrap();
    }
    let port = fixture.port();
    drop(fixture);
    let fixture = restart(dir.path(), port);
    let (_, peer) = trust(&mut alice, &mut bob);
    let id = [78; 32];
    alice.create_call(id, now, 3600).unwrap();
    alice.invite_to_call(id, peer, now).unwrap();
    super::tests::pump(&mut alice, &mut bob, now);
    bob.answer_call(id, true, now).unwrap();
    super::tests::pump(&mut bob, &mut alice, now);
    let tracks = Tracks {
        audio: true,
        camera: true,
        screen: true,
    };
    let mut handles = [
        alice.start_call_media(id, tracks, now).unwrap(),
        bob.start_call_media(id, tracks, now).unwrap(),
    ];
    super::tests::pump(&mut alice, &mut bob, now);
    alice.refresh_call_media(&mut handles[0], now).unwrap();
    bob.refresh_call_media(&mut handles[1], now).unwrap();
    super::tests::pump(&mut alice, &mut bob, now);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut endpoints = [
            connect(&mut alice, id, relay, now).await,
            connect(&mut bob, id, relay, now).await,
        ];
        media(
            &mut [&mut alice, &mut bob],
            &mut handles,
            &mut endpoints,
            now,
        )
        .await;
        drop(fixture);
        let _fixture = restart(dir.path(), port);
        assert!(matches!(
            alice.connect_call_online(&endpoints[0].proof, now),
            Err(Error::Network(crate::network::Error::Status {
                code: 409,
                ..
            }))
        ));
        tokio::time::sleep(Duration::from_secs(1)).await;
        for endpoint in &endpoints {
            endpoint.peer.pc.close().await.unwrap();
        }
        endpoints = [
            connect(&mut alice, id, relay, now + 1).await,
            connect(&mut bob, id, relay, now + 1).await,
        ];
        media(
            &mut [&mut alice, &mut bob],
            &mut handles,
            &mut endpoints,
            now + 1,
        )
        .await;
        alice.leave_call(id, now + 1).unwrap();
        super::tests::pump(&mut alice, &mut bob, now + 1);
        assert!(bob.call(id, now + 1).unwrap().phase == Phase::Ended);
        assert!(bob.call_relay_online(id, now + 1).is_err());
        for endpoint in endpoints {
            endpoint.peer.pc.close().await.unwrap();
        }
    });
}
pub(crate) fn exchange(
    alice: &mut ClientStore,
    bob: &mut ClientStore,
    id: Id,
    handles: &mut [Media; 2],
    now: u64,
) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut endpoints = [
            connect(alice, id, false, now).await,
            connect(bob, id, false, now).await,
        ];
        media(&mut [alice, bob], handles, &mut endpoints, now).await;
        for endpoint in endpoints {
            endpoint.peer.pc.close().await.unwrap();
        }
    });
}
#[test]
fn native_https_forwarded_media_and_server_restart() {
    run(false);
}
#[test]
#[ignore = "requires synthetic Coturn REST fixture; run calls/tests/relay.sh"]
fn native_rest_turn_media_and_server_restart() {
    run(true);
}
