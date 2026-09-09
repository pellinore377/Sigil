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
    server
        .configure_maps(sigil_server::maps::Configure {
            expected_revision: 0,
            settings: Some(maps),
        })
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
    drop(bob);
    std::fs::copy(dir.path().join("bob.db"), export.join("client.db")).unwrap();
    std::fs::write(export.join("port"), fixture.port().to_string()).unwrap();
    std::fs::write(export.join("ready"), b"ready").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    let mut received = false;
    while std::time::Instant::now() < deadline {
        for item in alice.receive_mailbox_online(conversations::now()).unwrap() {
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
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    panic!("Android content acceptance timed out");
}
