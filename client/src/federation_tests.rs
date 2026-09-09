use super::*;
#[path = "federation_call_tests.rs"]
mod calls;
use crate::network::tests::Fixture;
use sigil_crypto::Secret32;
use sigil_server::{
    auth::AdminToken,
    federation_config::{Configure, ConfigurePeer},
    store::Store,
};
use std::time::{Duration, Instant};
const CA: &[u8] = include_bytes!("../tests/fixtures/federation-ca.der");
fn now() -> u64 {
    crate::schedule::clock().unwrap()
}
#[track_caller]
fn open(path: &Path) -> ClientStore {
    ClientStore::open(
        path,
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap()
}
#[track_caller]
fn retry<T>(mut action: impl FnMut() -> Result<T, Error>) -> T {
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut delay = 1;
    loop {
        match action() {
            Ok(value) => return value,
            Err(Error::Network(network::Error::Status {
                code: 429 | 503 | 502,
                retry_after_seconds,
            })) if Instant::now() < deadline => {
                delay = delay.max(retry_after_seconds.unwrap_or(1)).min(60);
                std::thread::sleep(Duration::from_secs(delay));
                delay = (delay * 2).min(60);
            }
            Err(error) => panic!("{error:?}"),
        }
    }
}
fn server(path: &Path, name: &str) -> Store {
    let mut server = Store::open(path).unwrap();
    server
        .configure(sigil_protocol::Configure {
            expected_revision: 0,
            settings: sigil_protocol::Settings {
                server_name: name.into(),
                default_quota_bytes: sigil_protocol::DEFAULT_QUOTA,
                max_attachment_bytes: sigil_protocol::DEFAULT_ATTACHMENT_LIMIT,
            },
        })
        .unwrap();
    server
        .configure_federation(
            Configure {
                expected_revision: 0,
                enabled: true,
                exceptions: vec![],
                peer_quota_bytes: 64 * 1024 * 1024,
                rotate_key: false,
            },
            now(),
        )
        .unwrap();
    server
        .configure_groups(sigil_protocol::groups::Configure {
            expected_revision: 0,
            enabled: true,
            storage_limit_bytes: 64 * 1024 * 1024,
        })
        .unwrap();
    server
}
fn link(server: &mut Store, other: &Store, port: u16) {
    let discovery = other.federation_discovery().unwrap();
    server
        .configure_federation(
            Configure {
                expected_revision: 1,
                enabled: true,
                exceptions: vec![sigil_server::egress::Exception {
                    host: discovery.server.clone(),
                    port,
                    networks: vec!["127.0.0.1/32".into()],
                    root_ca: Some(CA.to_vec()),
                }],
                peer_quota_bytes: 64 * 1024 * 1024,
                rotate_key: false,
            },
            now(),
        )
        .unwrap();
    server
        .configure_federation_peer(
            &discovery.server,
            ConfigurePeer {
                expected_revision: 0,
                allowed: true,
                port,
                approve_key: Some(discovery.current.id),
            },
        )
        .unwrap();
}
fn enroll(server: &mut Store, path: &Path, name: &str, fixture: &Fixture) -> ClientStore {
    let invitation = server
        .invite(
            sigil_protocol::accounts::InviteRequest {
                username: "synthetic".into(),
                expires_in_seconds: 600,
            },
            now(),
        )
        .unwrap();
    let mut store = open(path);
    store
        .prepare_enrollment(
            name,
            fixture.port(),
            &[CA.to_vec()],
            &invitation.secret,
            "Synthetic",
            false,
        )
        .unwrap();
    store.enroll_online().unwrap();
    store.publish_device_binding_online().unwrap();
    let own = peers::parse(&store.own_device_binding().unwrap())
        .unwrap()
        .binding;
    store
        .configure_recovery(name, own.account, Secret32::from_bytes([7; 32]))
        .unwrap();
    for n in 1..=12 {
        store
            .prepare_prekey_publication([n; 32], true, 3600)
            .unwrap();
        retry(|| store.publish_prekey_online([n; 32]));
    }
    store
}
#[track_caller]
fn receive(target: &mut ClientStore) -> Vec<IncomingAttempt> {
    let mut accepted = std::collections::BTreeMap::new();
    let mut pending = std::collections::BTreeSet::new();
    retry(|| {
        for _ in 0..3 {
            let mut failure = None;
            for attempt in target.receive_mailbox_online(now())? {
                if attempt.result.is_ok() {
                    pending.remove(&attempt.sequence);
                    accepted.insert(attempt.sequence, attempt);
                } else {
                    pending.insert(attempt.sequence);
                    failure = attempt.result.err();
                }
            }
            if let Some(error) = failure {
                return Err(error);
            }
            if pending.is_empty() {
                return Ok(());
            }
        }
        panic!("failed deliveries disappeared before acceptance");
    });
    accepted.into_values().collect()
}
#[track_caller]
fn pump(source: &mut ClientStore, target: &mut ClientStore) -> Vec<IncomingAttempt> {
    retry(|| {
        for attempt in source.resume_outbound_online(now())? {
            attempt.result?;
        }
        Ok(())
    });
    let received = receive(target);
    retry(|| target.acknowledge_incoming_online());
    received
}
#[track_caller]
fn send(source: &mut ClientStore, target: &mut ClientStore) -> Vec<IncomingAttempt> {
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut received = Vec::new();
    loop {
        received.extend(pump(source, target));
        let pending: bool = source
            .db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM outbox WHERE packet IS NOT NULL)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        if !received.is_empty() && !pending {
            return received;
        }
        if Instant::now() >= deadline {
            let pending: u32 = source
                .db
                .query_row(
                    "SELECT count(*) FROM outbox WHERE packet IS NOT NULL",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            eprintln!("Pending native packets: {pending}");
            let root = source.db.path().unwrap();
            let name = if root.ends_with("alice.db") {
                "a-server.db"
            } else {
                "b-server.db"
            };
            let db = Connection::open(Path::new(root).parent().unwrap().join(name)).unwrap();
            let statuses = db
                .prepare("SELECT state,attempts,error FROM federation_outbox")
                .unwrap()
                .query_map([], |r| {
                    Ok((
                        r.get::<_, u32>(0)?,
                        r.get::<_, u32>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            let eligibility = db.prepare("SELECT p.allowed,p.error,p.checked_at,j.due_at,j.lease_until,j.expires_at,length(j.body),a.delivery_not_before FROM federation_outbox j JOIN federation_peers p ON p.server=j.destination JOIN federation_admission a ON a.server=p.server").unwrap().query_map([],|r|Ok((r.get::<_,bool>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?,r.get::<_,i64>(4)?,r.get::<_,i64>(5)?,r.get::<_,Option<i64>>(6)?,r.get::<_,i64>(7)?))).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
            panic!(
                "no delivery at {}; server states: {statuses:?}; eligibility: {eligibility:?}",
                now()
            );
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}
#[track_caller]
fn send_group(source: &mut ClientStore, target: &mut ClientStore) -> Vec<IncomingAttempt> {
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut received = Vec::new();
    loop {
        retry(|| {
            for attempt in source.resume_group_outbound_online(now())? {
                attempt.result?;
            }
            Ok(())
        });
        received.extend(receive(target));
        retry(|| target.acknowledge_incoming_online());
        let pending: bool = source
            .db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM group_delivery WHERE status=0)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        if !pending && !received.is_empty() {
            return received;
        }
        assert!(Instant::now() < deadline, "group delivery timed out");
        std::thread::sleep(Duration::from_millis(250));
    }
}
#[test]
fn remote_origin_is_bound_to_the_known_account_and_cannot_alias_a_local_device() {
    let (_dir, _fixture, mut alice, mut bob, time) = crate::claims::tests::pair();
    let (_, peer) = crate::incoming::tests::trust(&mut alice, &mut bob);
    crate::incoming::tests::start(&mut alice, peer, time);
    let mut packet = crate::incoming::tests::next(&bob);
    packet.origin = Some(wire::RemoteSender {
        server: "remote.example".into(),
        account: alice.connection_session().unwrap().unwrap().account_id,
        device: packet.sender_device.clone(),
    });
    assert!(bob.accept_delivery(&packet).is_err());
    packet.origin.as_mut().unwrap().server = "chat.example".into();
    assert!(bob.accept_delivery(&packet).is_err());
    let mut foreign = peers::parse(&alice.own_device_binding().unwrap()).unwrap();
    foreign.binding.server = "remote.example".into();
    let tx = alice.db.transaction().unwrap();
    foreign.signature = handshake::identity(&tx, &alice.key)
        .unwrap()
        .sign(&foreign.binding.signing_bytes().unwrap())
        .unwrap();
    tx.commit().unwrap();
    let known = bob
        .observe_peer_binding(&foreign.to_bytes().unwrap())
        .unwrap();
    packet.origin.as_mut().unwrap().server = "remote.example".into();
    assert_eq!(
        delivery_peer(&bob.db, &bob.key, "chat.example", &packet).unwrap(),
        known.id
    );
    packet.origin.as_mut().unwrap().account = "ff".repeat(32);
    assert!(delivery_peer(&bob.db, &bob.key, "chat.example", &packet).is_err());
    packet.origin = None;
    assert!(bob.accept_delivery(&packet).is_ok());
}
#[test]
#[ignore = "isolated two-server HTTPS acceptance; run client/tests/federation.sh"]
fn two_servers_exchange_messages_groups_and_files_across_restart_and_outage() {
    assert_eq!(
        std::env::var("SIGIL_FEDERATION_ACCEPTANCE").as_deref(),
        Ok("1")
    );
    let dir = tempfile::tempdir().unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let apath = dir.path().join("a-server.db");
    let bpath = dir.path().join("b-server.db");
    let a = server(&apath, "chat.example");
    let b = server(&bpath, "federated.example");
    let latency = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let delayed = |app: axum::Router| {
        let latency = latency.clone();
        app.layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let latency = latency.clone();
                async move {
                    let enabled = latency.load(std::sync::atomic::Ordering::Relaxed);
                    if enabled {
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                    let response = next.run(request).await;
                    if enabled {
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                    response
                }
            },
        ))
    };
    let (app, maintenance) = sigil_server::router_with_maintenance(
        a,
        AdminToken::load_or_create(&dir.path().join("a-admin")).unwrap(),
    );
    let af = Fixture::federation(delayed(app), maintenance);
    let (app, maintenance) = sigil_server::router_with_maintenance(
        b,
        AdminToken::load_or_create(&dir.path().join("b-admin")).unwrap(),
    );
    let bf = Fixture::federation(delayed(app), maintenance);
    let mut a = Store::open(&apath).unwrap();
    let mut b = Store::open(&bpath).unwrap();
    link(&mut a, &b, bf.port());
    link(&mut b, &a, af.port());
    let deadline = Instant::now() + Duration::from_secs(15);
    while a.federation_peer("federated.example").unwrap().checked_at == 0
        || b.federation_peer("chat.example").unwrap().checked_at == 0
    {
        assert!(Instant::now() < deadline, "discovery did not finish");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut alice = enroll(&mut a, &dir.path().join("alice.db"), "chat.example", &af);
    let mut bob = enroll(&mut b, &dir.path().join("bob.db"), "federated.example", &bf);
    let command = |client: &mut ClientStore, request: serde_json::Value| {
        let result: serde_json::Value =
            serde_json::from_str(&client.mobile_command(&request.to_string())).unwrap();
        assert_eq!(result["ok"], true, "{result}");
        result["value"].clone()
    };
    let found = command(
        &mut alice,
        serde_json::json!({"command":"find","address":"@synthetic:federated.example"}),
    );
    let destination = found["chats"][0]["id"].as_str().unwrap().to_owned();
    assert!(found["chats"][0]["devices"].as_array().unwrap().is_empty());
    command(
        &mut alice,
        serde_json::json!({"command":"contact_request","peer":destination,"action":"send"}),
    );
    let incoming = command(&mut bob, serde_json::json!({"command":"contact_refresh"}));
    assert_eq!(incoming["chats"][0]["request"], "incoming");
    let source = incoming["chats"][0]["id"].as_str().unwrap().to_owned();
    command(
        &mut bob,
        serde_json::json!({"command":"contact_request","peer":source,"action":"accept"}),
    );
    assert_eq!(
        Connection::open(&apath)
            .unwrap()
            .query_row("SELECT count(*) FROM federation_senders", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        Connection::open(&bpath)
            .unwrap()
            .query_row("SELECT count(*) FROM federation_senders", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let (pa, pb) = crate::incoming::tests::trust(&mut alice, &mut bob);
    retry(|| alice.allow_peer_sender_online(pb));
    retry(|| bob.allow_peer_sender_online(pa));
    let remote = retry(|| {
        alice.fetch_remote_peer_online(
            "federated.example",
            peers::parse(&bob.own_device_binding().unwrap())
                .unwrap()
                .binding
                .device,
        )
    });
    assert_eq!(remote.id, pb);
    alice.prepare_peer_claim([21; 32], pb).unwrap();
    retry(|| alice.claim_prekey_online([21; 32], now()));
    alice
        .start_claimed_text([21; 32], [22; 32], [23; 32], "cross-server", now(), now())
        .unwrap();
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    eprintln!("Sending Alice to Bob");
    let received = send(&mut alice, &mut bob);
    assert!(received.iter().any(
        |r| matches!(&r.result,Ok(MailboxEvent::Text(m)) if m.text().unwrap().body=="cross-server")
    ));
    let (session, _) = bob
        .send_peer_text(pa, [24; 32], "reply", now(), now())
        .unwrap();
    let _ = session;
    eprintln!("Sending Bob to Alice");
    send(&mut bob, &mut alice);
    latency.store(true, std::sync::atomic::Ordering::Relaxed);
    let mut samples = Vec::new();
    for n in 0u8..20 {
        let start = Instant::now();
        let mut id = [192; 32];
        id[0] = n;
        let (session, _) = alice
            .send_peer_text(pb, id, "federated latency", now(), now())
            .unwrap();
        alice.send_pending_online(session, now()).unwrap();
        loop {
            let messages = receive(&mut bob);
            if messages.iter().any(|r| matches!(&r.result, Ok(MailboxEvent::Text(m)) if m.text().unwrap().body == "federated latency")) { break; }
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(5));
        }
        samples.push(start.elapsed());
        retry(|| bob.acknowledge_incoming_online());
        retry(|| alice.send_pending_online(session, now()));
        std::thread::sleep(Duration::from_secs(1).saturating_sub(start.elapsed()));
    }
    latency.store(false, std::sync::atomic::Ordering::Relaxed);
    samples.sort();
    eprintln!("Two-server HTTPS, simulated 50ms/client and inter-server RTT, 20 encrypted messages: recipient decrypt p95={:?}", samples[18]);
    assert!(samples[18] < Duration::from_millis(500));
    let profile = a.group_authority().unwrap();
    let signing = a.group_configuration().unwrap().authority.unwrap();
    assert_eq!(transport::hex(&profile), signing);
    let raw = &profile[profile.len() - 96..profile.len() - 64];
    let authority = sigil_crypto::private_group::authority_fingerprint(raw.try_into().unwrap());
    let group = alice.create_group(authority).unwrap();
    alice
        .pin_group_service(group, &profile, Zeroizing::new([30; 32]))
        .unwrap();
    alice.prepare_group_service_request(group, None).unwrap();
    retry(|| alice.submit_group_service_online(group, now()));
    alice
        .queue_group_text(group, [31; 32], "earlier history", now() - 10, now())
        .unwrap();
    retry(|| {
        alice.prepare_group_invitation(
            group,
            pb,
            [32; 32],
            groups::Role::Member,
            now() + 600,
            now(),
        )
    });
    retry(|| alice.advance_group_invitation_online([32; 32], now()));
    eprintln!("Sending Alice control to Bob");
    send(&mut alice, &mut bob);
    eprintln!("Accepting remote invitation");
    bob.accept_group_invitation([32; 32], now()).unwrap();
    retry(|| bob.advance_group_invitation_online([32; 32], now()));
    send(&mut bob, &mut alice);
    retry(|| alice.advance_group_invitation_online([32; 32], now()));
    retry(|| bob.advance_group_invitation_online([32; 32], now()));
    assert_eq!(
        bob.group_invitation([32; 32]).unwrap().status,
        groups::InvitationStatus::Joined
    );
    eprintln!("Distributing remote group keys");
    retry(|| alice.prepare_group_distribution_online(group, pb, now()));
    send(&mut alice, &mut bob);
    retry(|| bob.prepare_group_distribution_online(group, pa, now()));
    send(&mut bob, &mut alice);
    alice
        .queue_group_text(group, [33; 32], "group over federation", now(), now())
        .unwrap();
    let received = send_group(&mut alice, &mut bob);
    assert!(received.iter().any(|r| matches!(&r.result,Ok(MailboxEvent::GroupText(m)) if m.text().unwrap().body=="group over federation")));
    eprintln!("Resuming delivery after server outage");
    let port = bf.port();
    drop(bf);
    alice
        .send_peer_text(pb, [39; 32], "outage", now(), now())
        .unwrap();
    retry(|| alice.resume_outbound_online(now()));
    let frozen: Vec<Vec<u8>> = alice
        .db
        .prepare("SELECT packet FROM outbox WHERE packet IS NOT NULL ORDER BY rowid")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(!frozen.is_empty());
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        retry(|| {
            for attempt in alice.resume_outbound_online(now())? {
                attempt.result?;
            }
            Ok(())
        });
        let db = Connection::open(&apath).unwrap();
        let attempted: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM federation_outbox WHERE state=0 AND attempts>0 AND error='transport_failed')", [], |r| r.get(0)).unwrap();
        if attempted {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "offline delivery was not retried"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    drop(alice);
    let mut alice = open(&dir.path().join("alice.db"));
    let retained: Vec<Vec<u8>> = alice
        .db
        .prepare("SELECT packet FROM outbox WHERE packet IS NOT NULL ORDER BY rowid")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(frozen, retained);
    let port_a = af.port();
    drop(af);
    let (app, maintenance) = sigil_server::router_with_maintenance(
        Store::open(&apath).unwrap(),
        AdminToken::load_or_create(&dir.path().join("a-admin")).unwrap(),
    );
    let _af = Fixture::federation_at(app, maintenance, port_a);
    let (app, maintenance) = sigil_server::router_with_maintenance(
        Store::open(&bpath).unwrap(),
        AdminToken::load_or_create(&dir.path().join("b-admin")).unwrap(),
    );
    let _bf = Fixture::federation_at(app, maintenance, port);
    assert!(send(&mut alice, &mut bob).iter().any(
        |r| matches!(&r.result,Ok(MailboxEvent::Text(m)) if m.text().unwrap().body=="outage")
    ));
    eprintln!("Resuming encrypted file download");
    let mut cache = alice
        .open_attachment_cache(&dir.path().join("upload.db"), 8 * 1024 * 1024)
        .unwrap();
    let file = cache
        .prepare_upload(
            1024 * 1024 + 5,
            crate::attachments::Metadata {
                name: "synthetic.bin".into(),
                media_type: "application/octet-stream".into(),
            },
            None,
        )
        .unwrap();
    let first = vec![42; 1024 * 1024];
    cache.stage_chunk(file, 0, &first).unwrap();
    cache.stage_chunk(file, 1, b"hello").unwrap();
    cache.finish_staging(file).unwrap();
    for _ in 0..4 {
        retry(|| alice.upload_attachment_step(&mut cache, file));
    }
    alice
        .queue_group_file(group, [40; 32], (&mut cache, file), now(), now())
        .unwrap();
    assert!(send_group(&mut alice, &mut bob)
        .iter()
        .any(|r| matches!(&r.result, Ok(MailboxEvent::GroupFile(_)))));
    alice
        .queue_peer_file(pb, [34; 32], (&mut cache, file), now(), now())
        .unwrap();
    for _ in 0..2 {
        for attempt in retry(|| alice.resume_send_intents_online(now())) {
            attempt.result.unwrap();
        }
    }
    let received = send(&mut alice, &mut bob);
    let sequence = received
        .iter()
        .find(|r| matches!(&r.result, Ok(MailboxEvent::File(_))))
        .unwrap()
        .sequence;
    let cache_path = dir.path().join("download.db");
    let mut download = bob
        .open_attachment_cache(&cache_path, 8 * 1024 * 1024)
        .unwrap();
    assert_eq!(
        bob.prepare_received_file(&mut download, sequence, now())
            .unwrap(),
        file
    );
    assert_eq!(
        retry(|| bob.download_attachment_step(&mut download, file, now())),
        crate::attachments::DownloadStep::Chunk(0)
    );
    drop(download);
    drop(bob);
    let mut bob = open(&dir.path().join("bob.db"));
    let mut download = bob
        .open_attachment_cache(&cache_path, 8 * 1024 * 1024)
        .unwrap();
    retry(|| bob.download_attachment_step(&mut download, file, now()));
    retry(|| bob.download_attachment_step(&mut download, file, now()));
    assert_eq!(
        download.completed_chunk(file, 1, now()).unwrap().as_slice(),
        b"hello"
    );
    assert_eq!(
        download.completed_chunk(file, 0, now()).unwrap().as_slice(),
        first
    );
    let afp = device_fingerprint(&alice.own_device_binding().unwrap()).unwrap();
    eprintln!("Sending a conversation file and opening its recovery copy");
    let conv = alice.direct_conversation(pb).unwrap();
    let author = alice.account_reference().unwrap();
    alice
        .queue_conversation_file(
            crate::conversations::Destination::Peer(pb),
            &mut cache,
            file,
            crate::conversations::FilePost {
                id: [150; 32],
                timestamp: now(),
                reply: None,
                thread: None,
                expires_at: None,
                view_once: false,
            },
            now(),
        )
        .unwrap();
    for _ in 0..2 {
        for attempt in retry(|| alice.resume_send_intents_online(now())) {
            attempt.result.unwrap();
        }
    }
    assert!(send(&mut alice, &mut bob)
        .iter()
        .any(|r| matches!(r.result, Ok(MailboxEvent::Conversation))));
    let target = crate::conversations::Reference {
        author,
        message: [150; 32],
    };
    assert_eq!(
        bob.prepare_conversation_file(&mut download, conv, target.clone(), now())
            .unwrap(),
        file
    );
    assert_eq!(
        bob.conversation_file_chunk(&download, conv, target, 1, now())
            .unwrap()
            .as_slice(),
        b"hello"
    );
    let mut after = None;
    let record = loop {
        let page = bob.recovery_records(after).unwrap();
        assert!(!page.is_empty(), "conversation file absent from recovery");
        after = page.last().map(|r| r.id);
        if let Some(record) = page.into_iter().find(|r| matches!(&r.content, sigil_crypto::recovery::Content::Conversation(raw) if sigil_protocol::conversation::Snapshot::from_bytes(raw).unwrap().operation.id == [150;32])) {
            break record;
        }
    };
    assert_eq!(
        bob.recovered_file_chunk(&download, record.id, 1, now())
            .unwrap()
            .as_slice(),
        b"hello"
    );
    let bfp = device_fingerprint(&bob.own_device_binding().unwrap()).unwrap();
    assert_eq!(
        bob.prepare_received_group_file(&mut download, group, afp, [40; 32], now())
            .unwrap(),
        file
    );
    eprintln!("Sharing authorized history across servers");
    let change = alice
        .prepare_group_change(group, groups::Change::EarlierHistory(true))
        .unwrap();
    alice
        .prepare_group_service_request(group, Some(&change))
        .unwrap();
    retry(|| alice.submit_group_service_online(group, now()));
    retry(|| bob.sync_group_service_online(group, now()));
    alice
        .prepare_group_history_share(
            group,
            [41; 32],
            groups::HistoryRange {
                source_device: afp,
                recipient_device: bfp,
                from_timestamp: 1,
                until_timestamp: now(),
                expires_at: now() + 1200,
            },
            now(),
        )
        .unwrap();
    retry(|| alice.advance_group_history_online([41; 32], now()));
    send(&mut alice, &mut bob);
    let deadline = Instant::now() + Duration::from_secs(300);
    let mut previous = String::new();
    loop {
        retry(|| alice.advance_group_history_online([41; 32], now()));
        pump(&mut alice, &mut bob);
        retry(|| bob.advance_group_history_online([41; 32], now()));
        pump(&mut bob, &mut alice);
        let progress = format!(
            "{:?} / {:?}",
            alice.group_history_share_status([41; 32]).unwrap(),
            bob.group_history_share_status([41; 32]).unwrap()
        );
        if progress != previous {
            eprintln!("History {progress}");
            previous = progress;
        }
        if alice.group_history_share_status([41; 32]).unwrap().0
            == groups::HistoryShareStatus::Complete
        {
            break;
        }
        assert!(Instant::now() < deadline, "history transfer timed out");
    }
    assert_eq!(
        bob.group_history_share_status([41; 32]).unwrap(),
        (groups::HistoryShareStatus::Complete, 3)
    );
    let history = bob.shared_group_history(group, None).unwrap();
    assert!(history
        .iter()
        .any(|r| sigil_protocol::event::Group::from_bytes(&r.plaintext)
            .unwrap()
            .message
            == [31; 32]));
    let record = history
        .iter()
        .find(|r| {
            sigil_protocol::event::Group::from_bytes(&r.plaintext)
                .unwrap()
                .message
                == [40; 32]
        })
        .unwrap()
        .id;
    assert_eq!(
        bob.prepare_shared_group_file(&mut download, group, record, now())
            .unwrap(),
        file
    );
    eprintln!("Removing remote member");
    let member = alice
        .group_status(group)
        .unwrap()
        .state
        .members()
        .iter()
        .find(|m| m.device_fingerprints().unwrap().contains(&bfp))
        .unwrap()
        .id();
    let change = alice
        .prepare_group_change(group, groups::Change::Remove(member))
        .unwrap();
    alice
        .prepare_group_service_request(group, Some(&change))
        .unwrap();
    retry(|| alice.submit_group_service_online(group, now()));
    retry(|| match bob.sync_group_service_online(group, now()) {
        Err(Error::Network(network::Error::Status { code: 403, .. })) => Ok(()),
        Err(error) => Err(error),
        Ok(_) => panic!("removed member retained group access"),
    });
    retry(
        || match alice.prepare_group_distribution_online(group, pb, now()) {
            Err(Error::Unprepared) => Ok(()),
            Err(error) => Err(error),
            Ok(_) => panic!("removed member received new group keys"),
        },
    );
    assert!(alice.peer(pb).unwrap().verified && bob.peer(pa).unwrap().verified);
    eprintln!("Exchanging encrypted call media across servers");
    calls::run(&mut alice, &mut bob, &mut a, pb);
}

#[test]
fn queued_receipt_polling_needs_no_new_send_authorization_but_new_submissions_do() {
    use axum::{
        http::StatusCode,
        response::IntoResponse,
        routing::{get, post},
        Json, Router,
    };
    use std::sync::{Arc, Mutex};
    let own = sigil_protocol::accounts::Session {
        account_id: "01".repeat(32),
        device_id: "02".repeat(32),
        address: "@alice:chat.example".into(),
        device_label: "Synthetic".into(),
        expires_at: 3000,
    };
    let request = mailbox::Submit {
        recipient_device: "03".repeat(32),
        message_id: "04".repeat(32),
        payload: "ab".repeat(32),
        expires_at: 2000,
    };
    let remote = wire::Submit {
        sender_account: own.account_id.clone(),
        sender_device: own.device_id.clone(),
        recipient_device: request.recipient_device.clone(),
        message_id: request.message_id.clone(),
        payload: request.payload.clone(),
        expires_at: request.expires_at,
    };
    let hash = transport::hex(&sha2::Sha256::digest(serde_json::to_vec(&remote).unwrap()));
    let response: Arc<Mutex<Option<wire::Outbound>>> = Arc::new(Mutex::new(None));
    let handler = response.clone();
    let app = Router::new()
        .route(
            "/client/v0/federation/messages/{id}",
            get(move || {
                let reply = handler.lock().unwrap().clone();
                async move {
                    match reply {
                        Some(value) => Json(value).into_response(),
                        None => StatusCode::NOT_FOUND.into_response(),
                    }
                }
            }),
        )
        .route(
            "/client/v0/federation/messages",
            post(|| async { StatusCode::FORBIDDEN }),
        );
    let fixture = Fixture::new(app);
    let network = network::HttpsClient::new(
        "chat.example",
        fixture.port(),
        &"ab".repeat(32),
        &[crate::network::tests::CA.to_vec()],
    )
    .unwrap();
    assert!(matches!(
        submit(&network, &own, "remote.example", &request, || Err(
            Error::Cancelled
        )),
        Err(Error::Cancelled)
    ));
    let mut value = wire::Outbound {
        message_id: request.message_id.clone(),
        destination: "remote.example".into(),
        request_hash: hash.clone(),
        expires_at: request.expires_at,
        state: wire::OutboundState::Pending,
        receipt: None,
        not_before: 1000,
        attempts: 0,
        error: None,
    };
    *response.lock().unwrap() = Some(value.clone());
    assert!(
        submit(&network, &own, "remote.example", &request, || panic!(
            "receipt polling attempted a send"
        ))
        .unwrap()
        .is_none()
    );
    value.state = wire::OutboundState::Accepted;
    value.receipt = Some(wire::Receipt {
        request_hash: hash,
        sequence: 9,
        expires_at: 2000,
    });
    *response.lock().unwrap() = Some(value);
    assert_eq!(
        submit(&network, &own, "remote.example", &request, || panic!(
            "receipt polling attempted a send"
        ))
        .unwrap()
        .unwrap()
        .sequence,
        9
    );
}
