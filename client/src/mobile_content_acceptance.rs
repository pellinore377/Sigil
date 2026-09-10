use super::*;
use crate::map_fixture;

#[test]
#[ignore = "Physical Android maps/media acceptance; run app/tests/content.sh"]
fn android_content_acceptance() {
    let export = std::path::PathBuf::from(std::env::var("SIGIL_ANDROID_CONTENT_EXPORT").unwrap());
    let key = Zeroizing::new(std::fs::read(export.join("key")).unwrap());
    let (dir, fixture, mut alice, mut bob, now) =
        crate::claims::tests::pair_with_bob_key(key.as_slice().try_into().unwrap());
    let maps = map_fixture::maps(dir.path());
    std::fs::write(std::path::Path::new(&maps.assets).join("style.json"), br##"{"version":8,"sources":{"local":{"type":"vector","url":"/client/v0/maps/tiles.json"}},"layers":[{"id":"paper","type":"background","paint":{"background-color":"#ece8e2"}},{"id":"point","type":"circle","source":"local","source-layer":"place","paint":{"circle-radius":24,"circle-color":"#e13e3e"}}]}"##).unwrap();
    let tile = [
        0x1a, 23, 0x0a, 5, b'p', b'l', b'a', b'c', b'e', 0x12, 9, 0x18, 1, 0x22, 5, 9, 0x80, 0x20,
        0x80, 0x20, 0x28, 0x80, 0x20, 0x78, 2,
    ];
    // One synthetic vector point at the middle of the sole z0 tile.
    let mut bytes = std::fs::read(&maps.archive).unwrap();
    bytes[64..72].copy_from_slice(&(tile.len() as u64).to_le_bytes());
    bytes[130] = tile.len() as u8;
    bytes.truncate(16386);
    bytes.extend_from_slice(&tile);
    std::fs::write(&maps.archive, bytes).unwrap();
    let mut server = sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap();
    let delivered = export.clone();
    let provider = crate::network::tests::Fixture::local_provider(axum::Router::new().route(
        "/push",
        axum::routing::post(move |body: axum::body::Bytes| {
            let delivered = delivered.clone();
            async move {
                std::fs::write(delivered.join("push-sealed.tmp"), &body).unwrap();
                std::fs::rename(
                    delivered.join("push-sealed.tmp"),
                    delivered.join("push-sealed"),
                )
                .unwrap();
                axum::http::StatusCode::CREATED
            }
        }),
    ));
    server
        .configure_push(sigil_server::push_config::Configure {
            expected_revision: 0,
            unified_push: true,
            contact: Some("mailto:acceptance@example.com".into()),
            rotate_vapid: false,
            fcm: sigil_server::push_config::FcmUpdate::Disable,
            exceptions: vec![sigil_server::egress::Exception {
                host: "127.0.0.1".into(),
                port: provider.port(),
                networks: vec!["127.0.0.1/32".into()],
                root_ca: Some(
                    include_bytes!("../../server/tests/fixtures/provider-ca.der").to_vec(),
                ),
            }],
        })
        .unwrap();
    server
        .configure_maps(sigil_server::maps::Configure {
            expected_revision: 0,
            settings: Some(maps),
        })
        .unwrap();
    let port = fixture.port();
    drop(fixture);
    let (router, maintenance) = sigil_server::router_with_maintenance(
        sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap(),
        sigil_server::auth::AdminToken::load_or_create(&dir.path().join("admin.token")).unwrap(),
    );
    let fixture = crate::network::tests::Fixture::maintained_at(router, maintenance, port);
    std::fs::write(
        export.join("push-endpoint"),
        format!("https://127.0.0.1:{}/push", provider.port()),
    )
    .unwrap();
    let (_, peer) = crate::incoming::tests::trust(&mut alice, &mut bob);
    let root = alice
        .conversation_operation(
            [88; 32],
            Action::Post {
                body: Body::Text("Attachment acceptance".into()),
                reply: None,
                thread: None,
                expires_at: None,
                view_once: false,
            },
        )
        .unwrap();
    alice.queue_peer_operation(peer, &root, now, now).unwrap();
    for item in alice.resume_send_intents_online(now).unwrap() {
        let session = item.result.unwrap();
        alice.send_pending_online(session, now).unwrap();
    }
    for item in bob.receive_mailbox_online(now).unwrap() {
        item.result.unwrap();
    }
    bob.acknowledge_incoming_online().unwrap();
    let conversation = alice.direct_conversation(peer).unwrap();
    let account = bob.connection_session().unwrap().unwrap().account_id;
    drop(bob);
    std::fs::copy(dir.path().join("bob.db"), export.join("client.db")).unwrap();
    std::fs::write(export.join("port"), fixture.port().to_string()).unwrap();
    std::fs::write(export.join("ready"), b"ready").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(360);
    let mut received = false;
    while std::time::Instant::now() < deadline {
        let incoming = match alice.receive_mailbox_online(conversations::now()) {
            Ok(incoming) => incoming,
            Err(Error::Network(network::Error::Status {
                code: 429,
                retry_after_seconds,
            })) => {
                std::thread::sleep(std::time::Duration::from_secs(
                    retry_after_seconds.unwrap_or(1),
                ));
                continue;
            }
            Err(error) => panic!("{error:?}"),
        };
        for item in incoming {
            item.result.unwrap();
        }
        alice.acknowledge_incoming_online().unwrap();
        for message in alice
            .recent_conversation_page(conversation, None, conversations::now())
            .unwrap()
            .messages
        {
            if matches!(message.body, Some(Body::File(_))) {
                assert_eq!(message.reply.as_ref().unwrap().message, root.id);
                let mut cache = alice.mobile_cache().unwrap();
                let file = alice
                    .prepare_conversation_file(
                        &mut cache,
                        conversation,
                        message.reference.clone(),
                        conversations::now(),
                    )
                    .unwrap();
                for _ in 0..4 {
                    alice
                        .download_attachment_step(&mut cache, file, conversations::now())
                        .unwrap();
                }
                let bytes = alice
                    .conversation_file_chunk(
                        &cache,
                        conversation,
                        message.reference,
                        0,
                        conversations::now(),
                    )
                    .unwrap();
                assert_eq!(bytes.len(), 44 + 48000 * 2 * 3);
                assert_eq!(&bytes[..4], b"RIFF");
                assert!(bytes[44..].iter().any(|v| *v != 0));
                received = true;
            }
        }
        if export.join("done").exists() {
            assert!(
                received,
                "The phone attachment never reached the other client"
            );
            let secret =
                Zeroizing::new(std::fs::read_to_string(export.join("recovery.key")).unwrap());
            let mut server =
                sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap();
            let invitation = server
                .invite_reauthorization(&account, 60, conversations::now())
                .unwrap();
            let mut replacement = ClientStore::open(
                &dir.path().join("replacement.db"),
                sigil_crypto::storage::StorageKey::new(sigil_crypto::Secret32::from_bytes(
                    [94; 32],
                ))
                .unwrap(),
            )
            .unwrap();
            replacement
                .prepare_enrollment(
                    "chat.example",
                    fixture.port(),
                    &[crate::network::tests::CA.to_vec()],
                    &invitation.secret,
                    "Replacement",
                    true,
                )
                .unwrap();
            replacement.enroll_online().unwrap();
            replacement.publish_device_binding_online().unwrap();
            replacement
                .begin_history_recovery_online(
                    sigil_crypto::Secret32::from_bytes(id(&secret).unwrap()),
                    true,
                )
                .unwrap();
            for _ in 0..100 {
                if replacement.download_recovery_step(false).unwrap().is_some() {
                    break;
                }
            }
            assert!(replacement.recovery_status().unwrap().pending.is_none());
            assert_eq!(
                replacement
                    .recent_search_conversations(
                        "Synthetic backup acceptance",
                        None,
                        conversations::now()
                    )
                    .unwrap()
                    .hits
                    .len(),
                1
            );
            assert_eq!(
                replacement
                    .db
                    .query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                0
            );
            return;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    panic!("Android content acceptance timed out");
}
