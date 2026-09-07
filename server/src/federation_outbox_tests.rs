use super::*;
use sigil_protocol::federation::{ConfigureSender, Discovery, RemoteSender};
fn store(path: &std::path::Path, name: &str, now: u64) -> Store {
    let mut s = Store::open(path).unwrap();
    s.configure(sigil_protocol::Configure {
        expected_revision: 0,
        settings: sigil_protocol::Settings {
            server_name: name.into(),
            default_quota_bytes: sigil_protocol::DEFAULT_QUOTA,
            max_attachment_bytes: sigil_protocol::DEFAULT_ATTACHMENT_LIMIT,
        },
    })
    .unwrap();
    s.configure_federation(
        federation_config::Configure {
            expected_revision: 0,
            enabled: true,
            exceptions: vec![],
            peer_quota_bytes: 1024 * 1024,
            rotate_key: false,
        },
        now,
    )
    .unwrap();
    s
}
fn enroll(s: &mut Store, now: u64) -> String {
    let invitation = s
        .invite(
            sigil_protocol::accounts::InviteRequest {
                username: "synthetic".into(),
                expires_in_seconds: 60,
            },
            now,
        )
        .unwrap();
    let token = crate::auth::random_secret().unwrap();
    s.enroll(
        sigil_protocol::accounts::Enrollment {
            invitation: invitation.secret,
            device_credential: token.clone(),
            device_label: "Synthetic".into(),
        },
        now,
    )
    .unwrap();
    token
}
fn pin(s: &mut Store, other: Discovery, now: u64) {
    s.configure_federation_peer(
        &other.server,
        federation_config::ConfigurePeer {
            expected_revision: 0,
            allowed: true,
            port: 443,
            approve_key: None,
        },
    )
    .unwrap();
    let (c, p) = s.begin_federation_refresh(&other.server, now).unwrap();
    s.finish_federation_refresh(c.revision, &p, Some(other), now)
        .unwrap();
}
pub(crate) struct Pair {
    pub(crate) dir: tempfile::TempDir,
    pub(crate) source: Store,
    pub(crate) sink: Store,
    pub(crate) alice: String,
    pub(crate) bob: String,
    pub(crate) request: Queue,
}
impl Pair {
    pub(crate) fn new(now: u64, origin: &str, destination: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut source = store(&dir.path().join("source.db"), origin, now);
        let mut sink = store(&dir.path().join("sink.db"), destination, now);
        pin(&mut source, sink.federation_discovery().unwrap(), now);
        pin(&mut sink, source.federation_discovery().unwrap(), now);
        let alice = enroll(&mut source, now);
        let bob = enroll(&mut sink, now);
        let identity = source.session(&alice, now).unwrap();
        let recipient = sink.session(&bob, now).unwrap().device_id;
        sink.configure_federation_sender(
            &bob,
            ConfigureSender {
                expected_revision: 0,
                sender: RemoteSender {
                    server: origin.into(),
                    account: identity.account_id,
                    device: identity.device_id,
                },
                allowed: true,
            },
            now,
        )
        .unwrap();
        let request = Queue {
            destination: destination.into(),
            recipient_device: recipient,
            message_id: format!("{:064x}", 1),
            payload: "ab".repeat(32),
            expires_at: now + 2000,
        };
        Self {
            dir,
            source,
            sink,
            alice,
            bob,
            request,
        }
    }
    fn enqueue(&mut self, now: u64) -> Outbound {
        self.source
            .queue_federated_message(&self.alice, self.request.clone(), now)
            .unwrap()
    }
}
fn response(status: u16, body: Vec<u8>) -> crate::egress::Response {
    crate::egress::Response {
        status,
        retry_after: None,
        content_type: Some("application/json".into()),
        body: Zeroizing::new(body),
    }
}
fn transmit(sink: &mut Store, job: &Job, now: u64) -> Outcome {
    job.deliver_with(
        || Ok(now),
        |request| {
            let receipt = sink
                .receive_federated_message(request.body(), request.headers(), now)
                .unwrap();
            Ok(response(202, serde_json::to_vec(&receipt).unwrap()))
        },
    )
}
fn accepted(job: &Job) -> Outcome {
    Outcome::Accepted(Receipt {
        request_hash: job.message.request_hash.clone(),
        sequence: 1,
        expires_at: job.message.expires_at,
    })
}
fn usage(s: &Store) -> i64 {
    s.0.query_row("SELECT egress_bytes FROM federation_usage", [], |r| {
        r.get(0)
    })
    .unwrap()
}
#[test]
fn ambiguous_delivery_restarts_without_reencrypting_and_receipt_failure_rolls_back() {
    let mut p = Pair::new(1000, "chat.example", "remote.example");
    let queued = p.enqueue(1000);
    assert_eq!(queued.state, OutboundState::Pending);
    let job = p.source.claim_federation_delivery(1000).unwrap().unwrap();
    let body = job.body.clone();
    let lost = job.deliver_with(
        || Ok(1000),
        |request| {
            p.sink
                .receive_federated_message(request.body(), request.headers(), 1000)
                .unwrap();
            Err(crate::egress::Error::Transport)
        },
    );
    assert!(matches!(lost, Outcome::Retry { .. }));
    p.source
        .finish_federation_delivery(&job, lost, 1000)
        .unwrap();
    let status = p
        .source
        .federated_outbound(&p.alice, &p.request.message_id, 1000)
        .unwrap();
    assert!(status.not_before >= 1005);
    let path = p.dir.path().join("source.db");
    drop(p.source);
    p.source = Store::open(&path).unwrap();
    assert!(p
        .source
        .claim_federation_delivery(status.not_before - 1)
        .unwrap()
        .is_none());
    let job = p
        .source
        .claim_federation_delivery(status.not_before)
        .unwrap()
        .unwrap();
    assert_eq!(job.body.as_str(), body.as_str());
    let outcome = transmit(&mut p.sink, &job, status.not_before);
    p.source.0.execute_batch("CREATE TRIGGER synthetic_failure BEFORE UPDATE ON federation_outbox BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(p
        .source
        .finish_federation_delivery(&job, outcome, status.not_before)
        .is_err());
    assert_eq!(usage(&p.source), (METADATA + body.len() as u64) as i64);
    p.source
        .0
        .execute_batch("DROP TRIGGER synthetic_failure")
        .unwrap();
    let outcome = transmit(&mut p.sink, &job, status.not_before);
    p.source
        .finish_federation_delivery(&job, outcome, status.not_before)
        .unwrap();
    let delivered = p
        .sink
        .federated_mailbox(&p.bob, 0, status.not_before)
        .unwrap();
    assert_eq!(delivered.len(), 1);
    let status = p
        .source
        .federated_outbound(&p.alice, &p.request.message_id, status.not_before)
        .unwrap();
    assert_eq!(status.state, OutboundState::Accepted);
    assert_eq!(status.receipt.unwrap().sequence, delivered[0].sequence);
    assert_eq!(usage(&p.source), METADATA as i64);
    assert_eq!(p.enqueue(status.not_before).state, OutboundState::Accepted);
    assert!(p
        .source
        .claim_federation_delivery(status.not_before)
        .unwrap()
        .is_none());
}
#[test]
fn leases_policy_changes_and_device_revocation_reject_stale_completions() {
    let mut p = Pair::new(1000, "chat.example", "remote.example");
    p.enqueue(1000);
    let first = p.source.claim_federation_delivery(1000).unwrap().unwrap();
    assert!(p.source.claim_federation_delivery(1044).unwrap().is_none());
    let second = p.source.claim_federation_delivery(1045).unwrap().unwrap();
    assert_ne!(first.lease, second.lease);
    assert!(!p
        .source
        .finish_federation_delivery(&first, accepted(&first), 1045)
        .unwrap());
    p.source
        .configure_federation_peer(
            "remote.example",
            federation_config::ConfigurePeer {
                expected_revision: 1,
                allowed: false,
                port: 443,
                approve_key: None,
            },
        )
        .unwrap();
    assert!(!p
        .source
        .finish_federation_delivery(&second, accepted(&second), 1046)
        .unwrap());
    assert!(p.source.claim_federation_delivery(1046).unwrap().is_none());
    p.source
        .configure_federation_peer(
            "remote.example",
            federation_config::ConfigurePeer {
                expected_revision: 2,
                allowed: true,
                port: 443,
                approve_key: None,
            },
        )
        .unwrap();
    let (c, peer) = p
        .source
        .begin_federation_refresh("remote.example", 1046)
        .unwrap();
    p.source
        .finish_federation_refresh(
            c.revision,
            &peer,
            Some(p.sink.federation_discovery().unwrap()),
            1046,
        )
        .unwrap();
    let third = p.source.claim_federation_delivery(1046).unwrap().unwrap();
    let device = p.source.session(&p.alice, 1046).unwrap().device_id;
    p.source.revoke_device(&p.alice, &device, 1046).unwrap();
    assert!(p
        .source
        .finish_federation_delivery(&third, accepted(&third), 1046)
        .unwrap());
    assert_eq!(
        read(&p.source.0, &device, &p.request.message_id)
            .unwrap()
            .unwrap()
            .state,
        OutboundState::Revoked
    );
    assert_eq!(usage(&p.source), METADATA as i64);
}
#[test]
fn retry_after_uses_completion_time_and_new_jobs_cannot_bypass_peer_backoff() {
    use std::sync::atomic::{AtomicU64, Ordering};
    let mut p = Pair::new(1000, "chat.example", "remote.example");
    p.enqueue(1000);
    let job = p.source.claim_federation_delivery(1000).unwrap().unwrap();
    let clock = AtomicU64::new(1000);
    let outcome = job.deliver_with(
        || Ok(clock.load(Ordering::Relaxed)),
        |_| {
            clock.store(1007, Ordering::Relaxed);
            let mut r = response(429, vec![]);
            r.retry_after = Some("120".into());
            Ok(r)
        },
    );
    assert!(matches!(
        outcome,
        Outcome::Retry {
            not_before: 1127,
            peer_backoff: true,
            ..
        }
    ));
    p.source
        .finish_federation_delivery(&job, outcome, 1007)
        .unwrap();
    p.request.message_id = format!("{:064x}", 2);
    p.enqueue(1008);
    assert!(p.source.claim_federation_delivery(1126).unwrap().is_none());
    let key = auth::SigningKey::generate(1, 1000).unwrap();
    pin(
        &mut p.source,
        Discovery {
            version: 0,
            server: "other.example".into(),
            current: key.descriptor().clone(),
            rotation: None,
        },
        1008,
    );
    p.request.destination = "other.example".into();
    p.request.message_id = format!("{:064x}", 3);
    p.enqueue(1008);
    let other = p.source.claim_federation_delivery(1008).unwrap().unwrap();
    assert_eq!(other.peer.server, "other.example");
    let mut invalid = Receipt {
        request_hash: "00".repeat(32),
        sequence: 1,
        expires_at: other.message.expires_at,
    };
    for n in 0..3 {
        if n == 1 {
            invalid.request_hash = other.message.request_hash.clone();
            invalid.sequence = 0;
        }
        if n == 2 {
            invalid.sequence = 1;
            invalid.expires_at += 1;
        }
        let outcome = other.deliver_with(
            || Ok(1008),
            |_| Ok(response(202, serde_json::to_vec(&invalid).unwrap())),
        );
        assert!(matches!(
            outcome,
            Outcome::Retry {
                error: "invalid_receipt",
                ..
            }
        ));
    }
    assert!(matches!(
        other.deliver_with(
            || Ok(other.message.expires_at),
            |_| panic!("expired work must not reach HTTPS")
        ),
        Outcome::Retry { .. }
    ));
}
#[test]
fn queue_identity_budget_and_restore_do_not_replay_old_work() {
    let mut p = Pair::new(1000, "chat.example", "remote.example");
    p.enqueue(1000);
    let device = p.source.session(&p.alice, 1000).unwrap().device_id;
    let local = sigil_protocol::mailbox::Submit {
        recipient_device: device.clone(),
        message_id: p.request.message_id.clone(),
        payload: p.request.payload.clone(),
        expires_at: p.request.expires_at,
    };
    assert!(matches!(
        p.source.submit_message(&p.alice, local, 1000),
        Err(StoreError::AlreadyExists)
    ));
    let mut conflict = p.request.clone();
    conflict.expires_at += 1;
    assert!(matches!(
        p.source.queue_federated_message(&p.alice, conflict, 1000),
        Err(StoreError::AlreadyExists)
    ));
    let local = sigil_protocol::mailbox::Submit {
        recipient_device: device.clone(),
        message_id: format!("{:064x}", 2),
        payload: "ab".repeat(32),
        expires_at: 3000,
    };
    p.source.submit_message(&p.alice, local, 1000).unwrap();
    p.request.message_id = format!("{:064x}", 2);
    assert!(matches!(
        p.source
            .queue_federated_message(&p.alice, p.request.clone(), 1000),
        Err(StoreError::AlreadyExists)
    ));
    for n in 3..=9 {
        p.request.message_id = format!("{n:064x}");
        p.request.payload = "ab".repeat(sigil_protocol::mailbox::MAX_PAYLOAD_HEX / 2);
        p.enqueue(1000);
    }
    let before = usage(&p.source);
    p.request.message_id = format!("{:064x}", 10);
    assert!(matches!(
        p.source
            .queue_federated_message(&p.alice, p.request.clone(), 1000),
        Err(StoreError::Busy)
    ));
    assert_eq!(usage(&p.source), before);
    let backup = p.dir.path().join("backup.db");
    p.source.backup(&backup).unwrap();
    let restored = p.dir.path().join("restored.db");
    Store::restore(&backup, &restored).unwrap();
    let mut restored = Store::open(&restored).unwrap();
    assert!(restored.claim_federation_delivery(1001).unwrap().is_none());
    assert_eq!(usage(&restored), 8 * METADATA as i64);
    let states:i64=restored.0.query_row("SELECT count(*) FROM federation_outbox WHERE state=5 AND body IS NULL AND lease IS NULL",[],|r|r.get(0)).unwrap();
    assert_eq!(states, 8);
}

#[test]
fn queued_signed_delivery_crosses_real_https_and_returns_the_bound_receipt() {
    use axum::http::Request;
    let _network = crate::egress::tests::NETWORK.lock().unwrap();
    let now = crate::enrollment::now().unwrap();
    let mut p = Pair::new(now, "remote.example", "chat.example");
    let discovery = p.sink.federation_discovery().unwrap();
    let admin = crate::auth::AdminToken::load_or_create(&p.dir.path().join("admin")).unwrap();
    let fixture = crate::egress::tests::Fixture::new(crate::router(p.sink, admin));
    let exception = fixture.exception("chat.example");
    let port = exception.port;
    p.source
        .configure_federation(
            federation_config::Configure {
                expected_revision: 1,
                enabled: true,
                exceptions: vec![exception],
                peer_quota_bytes: 1024 * 1024,
                rotate_key: false,
            },
            now,
        )
        .unwrap();
    p.source
        .configure_federation_peer(
            "chat.example",
            federation_config::ConfigurePeer {
                expected_revision: 1,
                allowed: true,
                port,
                approve_key: None,
            },
        )
        .unwrap();
    let (c, peer) = p
        .source
        .begin_federation_refresh("chat.example", now)
        .unwrap();
    p.source
        .finish_federation_refresh(c.revision, &peer, Some(discovery), now)
        .unwrap();
    p.source
        .queue_federated_message(&p.alice, p.request.clone(), now)
        .unwrap();
    let job = p.source.claim_federation_delivery(now).unwrap().unwrap();
    let outcome = job.deliver_with(|| Ok(now), |request| fixture.federation(request));
    assert!(matches!(outcome, Outcome::Accepted(_)));
    p.source
        .finish_federation_delivery(&job, outcome, now)
        .unwrap();
    let status = p
        .source
        .federated_outbound(&p.alice, &p.request.message_id, now)
        .unwrap();
    assert_eq!(status.state, OutboundState::Accepted);
    let response = fixture
        .federation(
            Request::get(fixture.uri("chat.example", "/client/v0/federation/mailbox"))
                .header("authorization", format!("Bearer {}", p.bob))
                .body(&[][..])
                .unwrap(),
        )
        .unwrap();
    assert_eq!(response.status, 200);
    let deliveries: Vec<sigil_protocol::federation::Delivery> =
        serde_json::from_slice(&response.body).unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].sequence, status.receipt.unwrap().sequence);
    assert_eq!(deliveries[0].sender.server, "remote.example");
    assert_eq!(deliveries[0].payload, p.request.payload);
}
#[tokio::test]
async fn queue_routes_require_local_credentials_and_reject_body_identity_spoofing() {
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let now = crate::enrollment::now().unwrap();
    let p = Pair::new(now, "chat.example", "remote.example");
    let admin = crate::auth::AdminToken::load_or_create(&p.dir.path().join("admin")).unwrap();
    let app = crate::router(p.source, admin);
    let body = serde_json::to_vec(&p.request).unwrap();
    for (credential, origin, expected) in [
        (None, false, StatusCode::UNAUTHORIZED),
        (Some(p.alice.as_str()), true, StatusCode::FORBIDDEN),
        (Some(p.alice.as_str()), false, StatusCode::ACCEPTED),
    ] {
        let mut request = Request::post("/client/v0/federation/messages")
            .header("content-type", "application/json");
        if let Some(token) = credential {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        if origin {
            request = request.header("origin", "https://untrusted.example");
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::from(body.clone())).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    let path = format!("/client/v0/federation/messages/{}", p.request.message_id);
    let response = app
        .clone()
        .oneshot(
            Request::get(&path)
                .header("authorization", format!("Bearer {}", p.alice))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let status: Outbound =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(status.state, OutboundState::Pending);
    let response = app
        .clone()
        .oneshot(
            Request::get(path)
                .header("authorization", format!("Bearer {}", p.bob))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    let mut spoof = serde_json::to_value(&p.request).unwrap();
    spoof["sender_account"] = serde_json::Value::String("00".repeat(32));
    let response = app
        .oneshot(
            Request::post("/client/v0/federation/messages")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", p.alice))
                .body(Body::from(serde_json::to_vec(&spoof).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 422);
}
