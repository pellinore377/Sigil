#![cfg(feature = "forwarder")]
mod common;
use common::{offer, Peer};
use sigil_calls::*;
use sigil_crypto::IdentityKey;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use webrtc::{
    data_channel::{DataChannelEvent, RTCDataChannelInit},
    media_stream::track_local::TrackLocal,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn data_channel_and_rtp_forward_the_same_authenticated_fragmented_media() {
    let keys = [
        IdentityKey::generate().unwrap(),
        IdentityKey::generate().unwrap(),
    ];
    let members = keys
        .iter()
        .map(|key| Member::new(key.public_key()))
        .collect::<Vec<_>>();
    let mut sorted = members.clone();
    sorted.sort_by_key(|member| member.id);
    let roster = Roster {
        controller: None,
        version: 1,
        call: [82; 32],
        server: "chat.example".into(),
        owner: keys[0].public_key(),
        created: 1000,
        expires: 2000,
        revision: 0,
        previous: None,
        members: sorted,
        closed: false,
    }
    .sign(&keys[0])
    .unwrap();
    let digest = roster.roster.digest().unwrap();
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let forwarder = Arc::new(Mutex::new(
        forwarder::Forwarder::new(socket.local_addr().unwrap(), 1).unwrap(),
    ));
    forwarder
        .lock()
        .unwrap()
        .install(roster.clone(), 1000)
        .unwrap();
    let running = forwarder.clone();
    let task = tokio::spawn(async move {
        let mut buffer = [0; 2048];
        loop {
            let tick = running.lock().unwrap().tick(1000, Instant::now());
            for packet in tick.datagrams {
                socket
                    .send_to(&packet.contents, packet.destination)
                    .await
                    .unwrap();
            }
            tokio::select! {value=socket.recv_from(&mut buffer)=>{let(length,source)=value.unwrap();running.lock().unwrap().receive(source,&buffer[..length],Instant::now());},_=tokio::time::sleep_until(tick.next.into())=>()}
        }
    });
    let mut peers = Vec::new();
    let mut uploads = Vec::new();
    let mut maps = Vec::new();
    let mut channel = None;
    for i in 0..2 {
        let mut peer = Peer::new(false, false).await;
        if i == 0 {
            channel = Some(
                peer.pc
                    .create_data_channel(
                        channel::LABEL,
                        Some(RTCDataChannelInit {
                            ordered: false,
                            max_retransmits: Some(0),
                            ..Default::default()
                        }),
                    )
                    .await
                    .unwrap(),
            );
        }
        let mut tracks = Vec::new();
        let mut transceivers = Vec::new();
        for n in 1..=3 {
            let (track, transceiver) = peer.track(n).await;
            tracks.push(track);
            transceivers.push(transceiver);
        }
        let (sdp, layout) = offer(&mut peer, &roster.roster, members[i].id, transceivers).await;
        let proof = Connect {
            call: roster.roster.call,
            roster: digest,
            participant: members[i].id,
            sequence: 1,
            sdp,
            layout,
        }
        .sign(&keys[i])
        .unwrap();
        let answer = forwarder
            .lock()
            .unwrap()
            .connect(&proof, 1000, Instant::now())
            .unwrap();
        maps.push(answer.streams);
        peer.pc
            .set_remote_description(
                rtc::peer_connection::sdp::RTCSessionDescription::answer(answer.sdp).unwrap(),
            )
            .await
            .unwrap();
        peers.push(peer);
        uploads.push(tracks);
    }
    let channel = channel.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(event) = channel.poll().await {
            if matches!(event, DataChannelEvent::OnOpen) {
                return;
            }
        }
        panic!("channel closed before opening")
    })
    .await
    .unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(128);
    let listener = channel.clone();
    let reader = tokio::spawn(async move {
        while let Some(event) = listener.poll().await {
            if let DataChannelEvent::OnMessage(message) = event {
                tx.send(message.data).await.unwrap();
            }
        }
    });
    let mut senders = Vec::new();
    let mut receivers = Vec::new();
    for member in &members {
        let (sender, key) = Sender::generate(Context {
            call: roster.roster.call,
            roster: digest,
            sender: member.id,
            incarnation: random_id().unwrap(),
        })
        .unwrap();
        senders.push(sender);
        receivers.push(Receiver::new(&key).unwrap());
    }
    let mut assemblies = [Assembly::default(), Assembly::default()];
    let mut sequences = [[0u16; 3]; 2];
    let mut seen = [[false; 3]; 2];
    for iteration in 0..100 {
        for source in 0..2 {
            for kind in [MediaKind::Audio, MediaKind::Camera, MediaKind::Screen] {
                let body = vec![kind as u8 + source as u8 + 17; 3500];
                let sealed = senders[source]
                    .seal(kind, iteration, kind != MediaKind::Audio, &body)
                    .unwrap();
                let fragments = packetize(kind, &sealed).unwrap();
                let count = fragments.len();
                for (index, payload) in fragments.into_iter().enumerate() {
                    let seq = &mut sequences[source][kind as usize];
                    *seq += 1;
                    if source == 0 {
                        channel
                            .send(bytes::BytesMut::from(
                                channel::Packet {
                                    sender: members[source].id,
                                    kind,
                                    sequence: u64::from(*seq),
                                    timestamp: iteration as u32 * 3000,
                                    marker: index + 1 == count,
                                    payload: payload.into(),
                                }
                                .encode()
                                .unwrap()
                                .as_slice(),
                            ))
                            .await
                            .unwrap();
                    } else {
                        uploads[source][kind as usize]
                            .write_rtp(rtc::rtp::packet::Packet {
                                header: rtc::rtp::header::Header {
                                    version: 2,
                                    payload_type: if kind == MediaKind::Audio { 111 } else { 96 },
                                    sequence_number: *seq,
                                    timestamp: iteration as u32 * 3000,
                                    ssrc: kind as u32 + 1,
                                    marker: index + 1 == count,
                                    ..Default::default()
                                },
                                payload: payload.into(),
                            })
                            .await
                            .unwrap();
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        while let Ok(bytes) = rx.try_recv() {
            let packet = channel::Packet::decode(&bytes).unwrap();
            assert_eq!(packet.sender, members[1].id);
            if let Some(encrypted) = assemblies[0]
                .push(packet.sender, packet.kind, &packet.payload, Instant::now())
                .unwrap()
            {
                let frame = receivers[1].open(&encrypted).unwrap();
                assert_eq!(frame.data.as_slice(), vec![packet.kind as u8 + 18; 3500]);
                seen[0][packet.kind as usize] = true;
            }
        }
        while let Ok(packet) = peers[1].packets.try_recv() {
            let route = &maps[1]
                .iter()
                .find(|entry| entry.ssrc == packet.ssrc)
                .unwrap()
                .track;
            assert_eq!(route.sender, members[0].id);
            if let Some(encrypted) = assemblies[1]
                .push(route.sender, route.kind, &packet.payload, Instant::now())
                .unwrap()
            {
                let frame = receivers[0].open(&encrypted).unwrap();
                assert_eq!(frame.data.as_slice(), vec![route.kind as u8 + 17; 3500]);
                seen[1][route.kind as usize] = true;
            }
        }
        if seen.iter().flatten().all(|value| *value) {
            break;
        }
    }
    assert!(
        seen.iter().flatten().all(|value| *value),
        "not all encrypted tracks crossed both transports: {seen:?}"
    );
    let mut closed = roster.roster;
    closed.revision += 1;
    closed.previous = Some(digest);
    closed.closed = true;
    forwarder
        .lock()
        .unwrap()
        .install(closed.sign(&keys[0]).unwrap(), 1001)
        .unwrap();
    assert_eq!(forwarder.lock().unwrap().counts(), (0, 0));
    for peer in peers {
        peer.pc.close().await.unwrap();
    }
    reader.abort();
    task.abort();
}
