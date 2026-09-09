#![cfg(feature = "forwarder")]
mod common;
use common::{offer, Peer};
use rtc::peer_connection::sdp::RTCSessionDescription;
use sigil_calls::forwarder::Forwarder;
use sigil_calls::*;
use sigil_crypto::IdentityKey;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use webrtc::media_stream::track_local::TrackLocal;
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn encrypted_forwarding_authenticates_tracks_and_removal_revokes_all_old_transports() {
    let count = 8usize;
    let owners: Vec<_> = (0..count)
        .map(|_| IdentityKey::generate().unwrap())
        .collect();
    let members: Vec<_> = owners.iter().map(|k| Member::new(k.public_key())).collect();
    let mut sorted = members.clone();
    sorted.sort_by_key(|m| m.id);
    let roster = Roster {
        controller: None,
        version: 1,
        call: [21; 32],
        server: "chat.example".into(),
        owner: owners[0].public_key(),
        created: 1000,
        expires: 2000,
        revision: 0,
        previous: None,
        members: sorted,
        closed: false,
    }
    .sign(&owners[0])
    .unwrap();
    let head = roster.roster.digest().unwrap();
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let address = socket.local_addr().unwrap();
    let mut forwarder = Forwarder::new(address, 1).unwrap();
    forwarder.install(roster.clone(), 1000).unwrap();
    let forwarder = Arc::new(Mutex::new(forwarder));
    let running = forwarder.clone();
    let task = tokio::spawn(async move {
        let mut buf = [0; 2048];
        loop {
            let tick = running.lock().unwrap().tick(1000, Instant::now());
            for packet in tick.datagrams {
                socket
                    .send_to(&packet.contents, packet.destination)
                    .await
                    .unwrap();
            }
            tokio::select! {r=socket.recv_from(&mut buf)=>{let(n,source)=r.unwrap();running.lock().unwrap().receive(source,&buf[..n],Instant::now());},_=tokio::time::sleep_until(tick.next.into())=>()}
        }
    });
    let mut clients = Vec::new();
    let mut media = Vec::new();
    let mut keys = Vec::new();
    let mut mappings = Vec::new();
    let mut joins = Vec::new();
    let mut proxies = Vec::new();
    for i in 0..count {
        let mut peer = Peer::new(false, false).await;
        let mut tracks = Vec::new();
        let mut uploads = Vec::new();
        for n in 1..=3 {
            let (track, transceiver) = peer.track(n).await;
            tracks.push(track);
            uploads.push(transceiver);
        }
        let (sdp, layout) = offer(&mut peer, &roster.roster, members[i].id, uploads).await;
        let signed = Connect {
            call: roster.roster.call,
            roster: head,
            participant: members[i].id,
            sequence: 1,
            sdp,
            layout,
        }
        .sign(&owners[i])
        .unwrap();
        let answer = forwarder
            .lock()
            .unwrap()
            .connect(&signed, 1000, Instant::now())
            .unwrap();
        let repeated = forwarder
            .lock()
            .unwrap()
            .connect(&signed, 1000, Instant::now())
            .unwrap();
        assert_eq!(answer.sdp, repeated.sdp);
        assert_eq!(answer.streams.len(), 3 * (count - 1));
        let (sdp, task, altered) = common::proxy::proxy(answer.sdp, true).await;
        proxies.push((task, altered));
        peer.pc
            .set_remote_description(RTCSessionDescription::answer(sdp).unwrap())
            .await
            .unwrap();
        let (sender, share) = Sender::generate(Context {
            call: roster.roster.call,
            roster: head,
            sender: members[i].id,
            incarnation: [i as u8 + 1; 32],
        })
        .unwrap();
        media.push((sender, tracks));
        keys.push(share);
        mappings.push(answer.streams);
        clients.push(peer);
        joins.push(signed);
    }
    let mut receivers: Vec<BTreeMap<Id, Receiver>> = (0..count)
        .map(|_| {
            keys.iter()
                .map(|key| (key.context.sender, Receiver::new(key).unwrap()))
                .collect()
        })
        .collect();
    let mut seen: Vec<BTreeSet<(Id, u8)>> = (0..count).map(|_| BTreeSet::new()).collect();
    let start = Instant::now();
    let mut first = vec![None; count];
    for seq in 0..65u16 {
        for (i, (sender, tracks)) in media.iter_mut().enumerate() {
            for (kind, track) in [MediaKind::Audio, MediaKind::Camera, MediaKind::Screen]
                .into_iter()
                .zip(tracks)
            {
                let body = [i as u8, kind as u8, (seq >> 8) as u8, seq as u8];
                let sealed = sender
                    .seal(kind, u64::from(seq), kind != MediaKind::Audio, &body)
                    .unwrap();
                let payload = if kind == MediaKind::Audio {
                    sealed
                } else {
                    [vec![0x10], sealed].concat()
                };
                track
                    .write_rtp(rtc::rtp::packet::Packet {
                        header: rtc::rtp::header::Header {
                            version: 2,
                            payload_type: if kind == MediaKind::Audio { 111 } else { 96 },
                            sequence_number: seq,
                            timestamp: u32::from(seq) * 960,
                            ssrc: kind as u32 + 1,
                            marker: true,
                            ..Default::default()
                        },
                        payload: payload.into(),
                    })
                    .await
                    .unwrap();
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        for (i, peer) in clients.iter_mut().enumerate() {
            while let Ok(packet) = peer.packets.try_recv() {
                let route = &mappings[i]
                    .iter()
                    .find(|m| m.ssrc == packet.ssrc)
                    .expect("unmapped RTP SSRC")
                    .track;
                let payload = if route.kind == MediaKind::Audio {
                    &packet.payload[..]
                } else {
                    assert_eq!(packet.payload[0], 0x10);
                    &packet.payload[1..]
                };
                let frame = receivers[i]
                    .get_mut(&route.sender)
                    .unwrap()
                    .open(payload)
                    .unwrap();
                assert_eq!(frame.kind, route.kind);
                let source = members.iter().position(|m| m.id == route.sender).unwrap();
                assert_eq!(
                    &*frame.data,
                    &[
                        source as u8,
                        route.kind as u8,
                        (packet.seq >> 8) as u8,
                        packet.seq as u8
                    ]
                );
                assert_eq!(
                    packet.pt,
                    if route.kind == MediaKind::Audio {
                        111
                    } else {
                        96
                    }
                );
                first[i].get_or_insert(packet.received.duration_since(start));
                seen[i].insert((route.sender, route.kind as u8));
            }
        }
    }
    for i in 0..count {
        assert_eq!(seen[i].len(), 3 * (count - 1), "recipient {i}");
        assert!(first[i].unwrap() < Duration::from_secs(2));
    }
    let mut replacement = Peer::new(false, false).await;
    let mut tracks = Vec::new();
    let mut uploads = Vec::new();
    for n in 1..=3 {
        let (track, transceiver) = replacement.track(n).await;
        tracks.push(track);
        uploads.push(transceiver);
    }
    let (sdp, layout) = offer(&mut replacement, &roster.roster, members[0].id, uploads).await;
    let rejoin = Connect {
        call: roster.roster.call,
        roster: head,
        participant: members[0].id,
        sequence: 2,
        sdp,
        layout,
    }
    .sign(&owners[0])
    .unwrap();
    let answer = forwarder
        .lock()
        .unwrap()
        .connect(&rejoin, 1000, Instant::now())
        .unwrap();
    replacement
        .pc
        .set_remote_description(RTCSessionDescription::answer(answer.sdp).unwrap())
        .await
        .unwrap();
    clients[0].pc.close().await.unwrap();
    clients[0] = replacement;
    media[0].1 = tracks;
    mappings[0] = answer.streams;
    tokio::time::sleep(Duration::from_millis(100)).await;
    for peer in &mut clients {
        while peer.packets.try_recv().is_ok() {}
    }
    let mut resumed = vec![false; count];
    for seq in 0..45u16 {
        let sealed = media[0]
            .0
            .seal(
                MediaKind::Audio,
                u64::from(seq),
                false,
                b"reconnected source",
            )
            .unwrap();
        media[0].1[0]
            .write_rtp(rtc::rtp::packet::Packet {
                header: rtc::rtp::header::Header {
                    version: 2,
                    payload_type: 111,
                    sequence_number: seq,
                    timestamp: u32::from(seq) * 960,
                    ssrc: 1,
                    marker: true,
                    ..Default::default()
                },
                payload: sealed.into(),
            })
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        for i in 1..count {
            while let Ok(packet) = clients[i].packets.try_recv() {
                let route = &mappings[i]
                    .iter()
                    .find(|m| m.ssrc == packet.ssrc)
                    .unwrap()
                    .track;
                if route.sender == members[0].id && route.kind == MediaKind::Audio {
                    assert!(
                        packet.seq >= 65,
                        "forwarded RTP sequence reused after source restart: {}",
                        packet.seq
                    );
                    let frame = receivers[i]
                        .get_mut(&route.sender)
                        .unwrap()
                        .open(&packet.payload)
                        .unwrap();
                    assert_eq!(&*frame.data, b"reconnected source");
                    resumed[i] = true;
                }
            }
        }
    }
    assert!(
        resumed[1..].iter().all(|v| *v),
        "existing receivers did not resume after source restart: {resumed:?}"
    );
    let mut next = roster.roster.clone();
    next.revision = 1;
    next.previous = Some(head);
    next.members.retain(|m| m.id != members[count - 1].id);
    forwarder
        .lock()
        .unwrap()
        .install(next.clone().sign(&owners[0]).unwrap(), 1000)
        .unwrap();
    assert_eq!(forwarder.lock().unwrap().counts(), (1, 0));
    assert!(matches!(
        forwarder
            .lock()
            .unwrap()
            .connect(&joins[0], 1000, Instant::now()),
        Err(Error::Conflict)
    ));
    next.previous = Some(next.digest().unwrap());
    next.closed = true;
    next.revision += 1;
    forwarder
        .lock()
        .unwrap()
        .install(next.sign(&owners[0]).unwrap(), 1000)
        .unwrap();
    assert_eq!(forwarder.lock().unwrap().counts(), (0, 0));
    for peer in clients {
        peer.pc.close().await.unwrap();
    }
    task.abort();
    for (task, altered) in proxies {
        assert!(altered.load(std::sync::atomic::Ordering::Relaxed) > 0);
        task.abort();
    }
}
