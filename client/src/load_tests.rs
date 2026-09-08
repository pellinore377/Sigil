use super::*;
use crate::network::tests::{Fixture, CA};
use sigil_crypto::{DhKey, Secret32};
use sigil_protocol::{
    accounts::{Enrollment, InviteRequest},
    mailbox::Submit,
    Configure, Settings,
};
use sigil_server::{
    auth::{random_secret, AdminToken},
    store::Store,
};
use std::{
    sync::{Arc, Barrier},
    time::{Duration, Instant},
};

fn open(path: &Path) -> ClientStore {
    ClientStore::open(
        path,
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap()
}
fn p95(values: &mut [Duration]) -> Duration {
    values.sort();
    values[(values.len() * 95).div_ceil(100) - 1]
}

#[test]
#[ignore = "controlled resource benchmark; run client/tests/load.sh"]
fn encrypted_delivery_load() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let now = conversations::now();
    let path = dir.path().join("server.db");
    let mut server = Store::open(&path).unwrap();
    server
        .configure(Configure {
            expected_revision: 0,
            settings: Settings {
                server_name: "chat.example".into(),
                default_quota_bytes: 1024 * 1024 * 1024,
                max_attachment_bytes: 1024 * 1024 * 1024,
            },
        })
        .unwrap();
    let mut devices = Vec::new();
    for i in 0..50 {
        let invitation = server
            .invite(
                InviteRequest {
                    username: format!("user{i}"),
                    expires_in_seconds: 600,
                },
                now,
            )
            .unwrap();
        let credential = random_secret().unwrap();
        let session = server
            .enroll(
                Enrollment {
                    invitation: invitation.secret,
                    device_credential: credential.clone(),
                    device_label: "Synthetic benchmark".into(),
                },
                now,
            )
            .unwrap();
        devices.push((credential, session.device_id));
    }
    for i in (0..20).step_by(2) {
        server
            .allow_sender(&devices[i + 1].0, &devices[i].1, now)
            .unwrap();
    }
    let (app, maintenance) = sigil_server::router_with_maintenance(
        server,
        AdminToken::load_or_create(&dir.path().join("admin")).unwrap(),
    );
    let app = app.layer(axum::middleware::from_fn(
        |request: axum::extract::Request, next: axum::middleware::Next| async move {
            tokio::time::sleep(Duration::from_millis(25)).await;
            let response = next.run(request).await;
            tokio::time::sleep(Duration::from_millis(25)).await;
            response
        },
    ));
    let fixture = Fixture::maintained_at(app, maintenance, 0);
    let barrier = Arc::new(Barrier::new(14));
    let mut uploads = Vec::new();
    for i in (0..8).step_by(2) {
        let http = network::HttpsClient::new(
            "chat.example",
            fixture.port(),
            &devices[i].0,
            &[CA.to_vec()],
        )
        .unwrap();
        let barrier = barrier.clone();
        uploads.push(std::thread::spawn(move || {
            let key = sigil_crypto::attachment::FileKey::generate(40 * 1024 * 1024).unwrap();
            let shape = key.shape();
            let file = shape.file;
            barrier.wait();
            http.begin_attachment(
                file,
                &sigil_protocol::attachments::Begin {
                    plaintext_bytes: shape.length,
                    access_token: random_secret().unwrap(),
                    expires_at: None,
                },
            )
            .unwrap();
            let plaintext = vec![42; 1024 * 1024];
            for part in 0..40 {
                let start = Instant::now();
                let ciphertext = key.seal_chunk(part, &plaintext).unwrap();
                http.put_attachment_chunk(shape, part, &ciphertext).unwrap();
                std::thread::sleep(Duration::from_millis(500).saturating_sub(start.elapsed()));
            }
            http.remove_attachment(file).unwrap();
        }));
    }
    let mut workers = Vec::new();
    for i in 0..10 {
        let mut sender = open(&dir.path().join(format!("sender{i}.db")));
        let mut receiver = open(&dir.path().join(format!("receiver{i}.db")));
        let dh = DhKey::generate().unwrap();
        sender
            .insert_session(
                [1; 32],
                Session::initiator(Secret32::from_bytes([7; 32]), dh.public_key(), [8; 32])
                    .unwrap(),
            )
            .unwrap();
        receiver
            .insert_session(
                [1; 32],
                Session::responder(Secret32::from_bytes([7; 32]), dh, [8; 32]).unwrap(),
            )
            .unwrap();
        let a = network::HttpsClient::new(
            "chat.example",
            fixture.port(),
            &devices[i * 2].0,
            &[CA.to_vec()],
        )
        .unwrap();
        let b = network::HttpsClient::new(
            "chat.example",
            fixture.port(),
            &devices[i * 2 + 1].0,
            &[CA.to_vec()],
        )
        .unwrap();
        let recipient = devices[i * 2 + 1].1.clone();
        let barrier = barrier.clone();
        workers.push(std::thread::spawn(move || {
            let mut feedback = Vec::new();
            let mut delivered = Vec::new();
            barrier.wait();
            for n in 1u64..=20 {
                let start = Instant::now();
                let mut id = [0; 32];
                id[..8].copy_from_slice(&n.to_be_bytes());
                let body = format!("synthetic benchmark {i}/{n}");
                let packet = sender.send([1; 32], id, body.as_bytes()).unwrap();
                sender
                    .prepare_delivery(
                        [1; 32],
                        id,
                        connection::decode_id(&recipient).unwrap(),
                        now + 600,
                        now,
                    )
                    .unwrap();
                feedback.push(start.elapsed());
                let receipt = a
                    .submit(&Submit {
                        recipient_device: recipient.clone(),
                        message_id: transport::hex(&id),
                        payload: transport::hex(&packet),
                        expires_at: now + 600,
                    })
                    .unwrap();
                sender.acknowledge_sent([1; 32], id, &receipt).unwrap();
                let page = b.mailbox().unwrap();
                assert_eq!(page.len(), 1);
                let raw = page[0]
                    .payload
                    .as_bytes()
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
                    .collect::<Vec<_>>();
                assert_eq!(
                    receiver.receive([1; 32], id, &raw).unwrap().as_slice(),
                    body.as_bytes()
                );
                delivered.push(start.elapsed());
                b.acknowledge_delivery(page[0].sequence).unwrap();
                std::thread::sleep(Duration::from_secs(1).saturating_sub(start.elapsed()));
            }
            (feedback, delivered)
        }));
    }
    let mut feedback = Vec::new();
    let mut delivered = Vec::new();
    for worker in workers {
        let (a, b) = worker.join().unwrap();
        feedback.extend(a);
        delivered.extend(b);
    }
    for upload in uploads {
        upload.join().unwrap();
    }
    let feedback = p95(&mut feedback);
    let delivery = p95(&mut delivered);
    println!("50 accounts, 20 active devices, 200 encrypted messages alongside 160MiB uploads, HTTPS loopback + simulated 50ms/request RTT: durable feedback p95={feedback:?}, recipient decrypt p95={delivery:?}");
    assert!(feedback < Duration::from_millis(100));
    assert!(delivery < Duration::from_millis(500));
}
