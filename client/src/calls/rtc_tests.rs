use super::*;
use crate::conversations;

#[test]
#[ignore = "Physical Android/TURN acceptance; run app/tests/calls.sh"]
fn android_codec_transport_acceptance() {
    let export = std::path::PathBuf::from(std::env::var("SIGIL_ANDROID_CALL_EXPORT").unwrap());
    let relay = std::env::var("SIGIL_ANDROID_CALL_RELAY").unwrap();
    let address: std::net::IpAddr = "127.0.0.1".parse().unwrap();
    let key = Zeroizing::new(std::fs::read(export.join("key")).unwrap());
    let ui = export.join("ui").exists();
    let group = export.join("group").exists();
    let (dir, fixture, mut alice, mut bob, now) =
        crate::claims::tests::pair_with_bob_key(key.as_slice().try_into().unwrap());
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port();
    drop(socket);
    let mut store = sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap();
    store
        .configure_calls(sigil_server::call_config::Configure {
            expected_revision: 0,
            settings: Some(sigil_server::call_config::Settings {
                bind: std::net::SocketAddr::new(address, port),
                advertised: std::net::SocketAddr::new(address, port),
                max_calls: 1,
                turn_urls: vec![relay],
            }),
            turn_secret: sigil_server::service_config::SecretUpdate::Set(Zeroizing::new(
                "synthetic-rest-secret-for-call-tests".into(),
            )),
        })
        .unwrap();
    let mut guest = if group {
        let invite = store
            .invite(
                sigil_protocol::accounts::InviteRequest {
                    username: "charlie".into(),
                    expires_in_seconds: 60,
                },
                now,
            )
            .unwrap();
        let mut guest = ClientStore::open(
            &dir.path().join("charlie.db"),
            StorageKey::new(sigil_crypto::Secret32::from_bytes([9; 32])).unwrap(),
        )
        .unwrap();
        crate::connection::tests::prepare(&mut guest, &fixture, &invite.secret);
        let session = guest.enroll_online().unwrap();
        store
            .allow_sender(
                &crate::connection::tests::credential(&alice),
                &session.device_id,
                now,
            )
            .unwrap();
        store
            .allow_sender(
                &crate::connection::tests::credential(&guest),
                &alice.connection_session().unwrap().unwrap().device_id,
                now,
            )
            .unwrap();
        guest.replenish_prekey_online().unwrap();
        Some(guest)
    } else {
        None
    };
    drop(store);
    let https = fixture.port();
    drop(fixture);
    let (app, maintenance) = sigil_server::router_with_maintenance(
        sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap(),
        sigil_server::auth::AdminToken::load_or_create(&dir.path().join("admin.token")).unwrap(),
    );
    let _fixture = crate::network::tests::Fixture::maintained_at(app, maintenance, https);
    let (_, peer) = crate::incoming::tests::trust(&mut alice, &mut bob);
    let id = [97; 32];
    if group {
        alice.create_group_call(id, now, 3600).unwrap();
    } else {
        alice.create_direct_call(id, now, 3600).unwrap();
    }
    alice.invite_to_call(id, peer, now).unwrap();
    super::super::tests::pump(&mut alice, &mut bob, now);
    if !ui {
        bob.answer_call(id, true, now).unwrap();
        super::super::tests::pump(&mut bob, &mut alice, now);
    }
    if let Some(guest) = &mut guest {
        let (_, recipient) = crate::incoming::tests::trust(&mut alice, guest);
        alice.invite_to_call(id, recipient, now).unwrap();
        super::super::tests::pump(&mut alice, guest, now);
        guest.answer_call(id, true, now).unwrap();
        super::super::tests::pump(guest, &mut alice, now);
    }
    let phone = crate::device_fingerprint(&bob.own_device_binding().unwrap()).unwrap();
    drop(bob);
    std::fs::create_dir_all(&export).unwrap();
    std::fs::copy(dir.path().join("bob.db"), export.join("client.db")).unwrap();
    std::fs::write(export.join("port"), https.to_string()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        if let Some(guest) = &mut guest {
            group_android_loop(&mut alice, guest, id, phone, &export).await;
            return;
        }
        let mut call = alice
            .connect_rtc_call(
                id,
                Tracks {
                    audio: true,
                    camera: true,
                    screen: true,
                },
                now,
            )
            .await
            .unwrap();
        std::fs::write(export.join("ready"), b"ready").unwrap();
        let start = std::time::Instant::now();
        let mut echoed = [0; 3];
        let mut sync_at = std::time::Instant::now();
        let mut connection = "connecting";
        let mut ended = false;
        while start.elapsed() < Duration::from_secs(180) {
            if alice.call(id, conversations::now()).unwrap().phase == Phase::Ended {
                ended = true;
                break;
            }
            connection = alice
                .rtc_connection_state(&call, conversations::now())
                .unwrap();
            if connection == "reconnect" {
                drop(call);
                call = alice
                    .connect_rtc_call(
                        id,
                        Tracks {
                            audio: true,
                            camera: true,
                            screen: true,
                        },
                        conversations::now(),
                    )
                    .await
                    .unwrap();
                continue;
            }
            if connection == "connected" {
                match alice.rtc_receive(&mut call, conversations::now()) {
                    Ok(frames) => {
                        for frame in frames {
                            assert!(
                                frame.frame.data.len()
                                    <= if frame.frame.kind == MediaKind::Audio {
                                        8192
                                    } else {
                                        1024 * 1024
                                    }
                            );
                            alice
                                .rtc_send(
                                    &mut call,
                                    frame.frame.kind,
                                    frame.frame.timestamp,
                                    frame.frame.keyframe,
                                    &frame.frame.data,
                                    conversations::now(),
                                )
                                .await
                                .unwrap();
                            echoed[frame.frame.kind as usize] += 1;
                        }
                    }
                    Err(Error::Unprepared) => (),
                    Err(error) => panic!("{error:?}"),
                }
            }
            if std::time::Instant::now() >= sync_at {
                let result = alice.sync_foreground_online().unwrap();
                sync_at = std::time::Instant::now()
                    + Duration::from_secs(
                        result.next_at.saturating_sub(conversations::now()).max(1),
                    );
                if let Some(step) = result.step {
                    assert!(step.incoming.iter().all(|v| v.result.is_ok()));
                }
            }
            if !ui && export.join("done").exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if ui {
            assert!(
                ended && echoed[0] > 0,
                "UI call ended: {ended}; authenticated frames: {echoed:?}"
            );
        } else {
            assert!(
                echoed[0] >= 50 && echoed[1] >= 10 && echoed[2] >= 10,
                "Authenticated frames returned: {echoed:?}; connection: {connection}"
            );
        }
    });
}

async fn group_android_loop(
    alice: &mut ClientStore,
    charlie: &mut ClientStore,
    id: Id,
    phone: Id,
    export: &std::path::Path,
) {
    let tracks = Tracks {
        audio: true,
        ..Default::default()
    };
    let mut a = Some(
        alice
            .connect_rtc_call(id, tracks, conversations::now())
            .await
            .unwrap(),
    );
    let mut c = Some(
        charlie
            .connect_rtc_call(id, tracks, conversations::now())
            .await
            .unwrap(),
    );
    std::fs::write(export.join("ready"), b"ready").unwrap();
    let start = std::time::Instant::now();
    let mut sync_at = [start; 2];
    let mut connect_at = [start; 2];
    let mut echoed = [0; 2];
    let mut left = false;
    let mut continued = 0;
    let mut phone_left = false;
    while start.elapsed() < Duration::from_secs(180) {
        let now = conversations::now();
        for (store, transport, index) in [(&mut *alice, &mut a, 0), (&mut *charlie, &mut c, 1)] {
            if std::time::Instant::now() >= sync_at[index] {
                let result = store.sync_foreground_online().unwrap();
                sync_at[index] = std::time::Instant::now()
                    + Duration::from_secs(result.next_at.saturating_sub(now).max(1));
                if let Some(step) = result.step {
                    assert!(step.incoming.iter().all(|v| v.result.is_ok()));
                }
            }
            if left && index == 0 {
                continue;
            }
            let connection = match transport
                .as_ref()
                .map(|call| store.rtc_connection_state(call, now))
            {
                Some(Ok(value)) => value,
                Some(Err(Error::Unprepared)) => continue,
                None | Some(Err(Error::Obsolete)) => "reconnect",
                Some(Err(error)) => panic!("group connection {index}: {error:?}"),
            };
            if connection == "reconnect" {
                if std::time::Instant::now() < connect_at[index] {
                    continue;
                }
                transport.take();
                match store.connect_rtc_call(id, tracks, now).await {
                    Ok(call) => *transport = Some(call),
                    Err(Error::Unprepared) => (),
                    Err(error) => panic!("group reconnect {index}: {error:?}"),
                }
                connect_at[index] = std::time::Instant::now() + Duration::from_secs(1);
                continue;
            }
            if connection != "connected" {
                continue;
            }
            let call = transport.as_mut().unwrap();
            let phone_member = store
                .call(id, now)
                .unwrap()
                .participants
                .iter()
                .find(|person| crate::device_fingerprint(&person.device).ok() == Some(phone))
                .map(|person| person.member);
            match store.rtc_receive(call, now) {
                Ok(frames) => {
                    for frame in frames {
                        if Some(frame.sender) != phone_member {
                            continue;
                        }
                        assert_eq!(frame.frame.kind, MediaKind::Audio);
                        store
                            .rtc_send(
                                call,
                                MediaKind::Audio,
                                frame.frame.timestamp,
                                false,
                                &frame.frame.data,
                                now,
                            )
                            .await
                            .unwrap();
                        echoed[index] += 1;
                        if left
                            && index == 1
                            && store.call(id, now).unwrap().participants.len() == 2
                        {
                            continued += 1;
                        }
                    }
                }
                Err(Error::Unprepared) => (),
                Err(error) => panic!("group media {index}: {error:?}"),
            }
        }
        if !left && echoed.iter().all(|count| *count >= 5) {
            alice.leave_call(id, now).unwrap();
            a.take();
            left = true;
        }
        if left && continued >= 20 {
            let call = charlie.call(id, now).unwrap();
            if call.participants.len() == 1 && call.phase == Phase::Active {
                phone_left = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(left && continued >= 20 && phone_left, "creator left: {left}, rekeyed phone audio: {continued}, phone left without ending the group: {phone_left}, frames: {echoed:?}");
    charlie.leave_call(id, conversations::now()).unwrap();
}

#[test]
fn adapter_transports_only_authenticated_frames_and_rejects_ended_handles() {
    let (dir, fixture, mut alice, mut bob, now) = crate::claims::tests::pair();
    super::super::tests::configure(dir.path());
    let port = fixture.port();
    drop(fixture);
    let (app, maintenance) = sigil_server::router_with_maintenance(
        sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap(),
        sigil_server::auth::AdminToken::load_or_create(&dir.path().join("admin.token")).unwrap(),
    );
    let _fixture = crate::network::tests::Fixture::maintained_at(app, maintenance, port);
    let (_, peer) = crate::incoming::tests::trust(&mut alice, &mut bob);
    let id = [96; 32];
    alice.create_call(id, now, 3600).unwrap();
    alice.invite_to_call(id, peer, now).unwrap();
    super::super::tests::pump(&mut alice, &mut bob, now);
    bob.answer_call(id, true, now).unwrap();
    super::super::tests::pump(&mut bob, &mut alice, now);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let tracks = Tracks {
            audio: true,
            camera: true,
            screen: true,
        };
        let mut a = alice.connect_rtc_call(id, tracks, now).await.unwrap();
        let mut b = bob.connect_rtc_call(id, tracks, now).await.unwrap();
        super::super::tests::pump(&mut alice, &mut bob, now);
        alice.refresh_call_media(&mut a.media, now).unwrap();
        bob.refresh_call_media(&mut b.media, now).unwrap();
        super::super::tests::pump(&mut alice, &mut bob, now);
        let mut received = [[false; 3]; 2];
        for attempt in 0..150 {
            for (store, call, index) in [(&mut alice, &mut a, 0), (&mut bob, &mut b, 1)] {
                if store.rtc_connection_state(call, now).unwrap() == "connected" {
                    for kind in [MediaKind::Audio, MediaKind::Camera, MediaKind::Screen] {
                        store
                            .rtc_send(
                                call,
                                kind,
                                attempt * 20_000,
                                kind != MediaKind::Audio,
                                &vec![
                                    index as u8;
                                    if kind == MediaKind::Audio { 160 } else { 8192 }
                                ],
                                now,
                            )
                            .await
                            .unwrap();
                    }
                    for value in store.rtc_receive(call, now).unwrap() {
                        assert!(value.frame.data.iter().all(|b| *b == (1 - index) as u8));
                        assert_eq!(
                            value.frame.data.len(),
                            if value.frame.kind == MediaKind::Audio {
                                160
                            } else {
                                8192
                            }
                        );
                        received[index][value.frame.kind as usize] = true;
                    }
                }
            }
            if received.iter().flatten().all(|v| *v) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(received, [[true; 3]; 2]);
        assert!(bob.rtc_receive(&mut a, now).is_err());
        alice.leave_call(id, now).unwrap();
        assert!(alice
            .rtc_send(&mut a, MediaKind::Audio, 0, false, b"forbidden", now)
            .await
            .is_err());
        assert!(alice.rtc_receive(&mut a, now).is_err());
    });
}
