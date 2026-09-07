use super::*;
use crate::federation_outbox::tests::Pair;

#[test]
fn reservations_and_pending_diagnostics_survive_restart_and_cleanup() {
    let mut p = Pair::new(1000, "chat.example", "remote.example");
    p.source
        .queue_federated_message(&p.alice, p.request.clone(), 1000)
        .unwrap();
    let initial = p
        .source
        .federation_peer_status("remote.example", 1000)
        .unwrap();
    assert_eq!(initial.block, None);
    assert_eq!(initial.queue.pending, 1);
    assert_eq!(initial.queue.ready, 1);
    assert_eq!(initial.queue.next_attempt, Some(1000));
    assert!(initial.egress_bytes > crate::federation_outbox::METADATA);
    let global = p.source.federation_status().unwrap();
    assert_eq!(global.peers, 1);
    assert_eq!(
        global.peer_metadata.reserved_bytes,
        federation_config::PEER_METADATA
    );
    assert_eq!(global.egress.reserved_bytes, initial.egress_bytes);
    let serialized = serde_json::to_string(&initial).unwrap();
    for forbidden in [
        &p.alice,
        &p.bob,
        &p.request.message_id,
        &p.request.payload,
        &p.request.recipient_device,
    ] {
        assert!(!serialized.contains(forbidden));
    }
    p.source.0.execute("UPDATE federation_outbox SET due_at=1030,lease_until=1020,attempts=2,error='transport_failed'", []).unwrap();
    drop(p.source);
    p.source = Store::open(&p.dir.path().join("source.db")).unwrap();
    let waiting = p
        .source
        .federation_peer_status("remote.example", 1001)
        .unwrap();
    assert_eq!(waiting.queue.ready, 0);
    assert_eq!(waiting.queue.leased, 1);
    assert_eq!(waiting.queue.retries, 2);
    assert_eq!(waiting.queue.errors.get("transport_failed"), Some(&1));
    assert_eq!(waiting.queue.next_attempt, Some(1030));
    let expired = p
        .source
        .federation_peer_status("remote.example", p.request.expires_at)
        .unwrap();
    assert_eq!(expired.queue.expired, 1);
    assert_eq!(expired.queue.ready, 0);
    assert_eq!(expired.queue.next_attempt, None);
    p.source.expire_batch(p.request.expires_at).unwrap();
    let cleared = p
        .source
        .federation_peer_status("remote.example", p.request.expires_at)
        .unwrap();
    assert_eq!(cleared.queue.pending, 0);
    assert_eq!(cleared.egress_bytes, crate::federation_outbox::METADATA);
    assert_eq!(
        p.source.federation_status().unwrap().egress.reserved_bytes,
        cleared.egress_bytes
    );
}

#[test]
fn policy_clock_and_cooldown_diagnostics_do_not_mutate_delivery_state() {
    let mut p = Pair::new(1000, "chat.example", "remote.example");
    p.source
        .queue_federated_message(&p.alice, p.request.clone(), 1000)
        .unwrap();
    assert_eq!(
        p.source
            .federation_peer_status("remote.example", 999)
            .unwrap()
            .block,
        Some(Block::ClockBeforeDiscovery)
    );
    assert_eq!(
        p.source
            .federation_peer_status("remote.example", 4601)
            .unwrap()
            .block,
        Some(Block::DiscoveryExpired)
    );
    p.source
        .0
        .execute(
            "UPDATE federation_admission SET delivery_not_before=1060",
            [],
        )
        .unwrap();
    let cooldown = p
        .source
        .federation_peer_status("remote.example", 1000)
        .unwrap();
    assert_eq!(cooldown.block, Some(Block::PeerCooldown));
    assert_eq!(cooldown.queue.ready, 0);
    assert_eq!(cooldown.queue.next_attempt, Some(1060));
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
    let blocked = p
        .source
        .federation_peer_status("remote.example", 1000)
        .unwrap();
    assert_eq!(blocked.block, Some(Block::PeerDisabled));
    assert_eq!(blocked.queue.next_attempt, None);
    assert_eq!(blocked.queue.pending, 1);
    p.source
        .configure_federation(
            federation_config::Configure {
                expected_revision: 1,
                enabled: false,
                exceptions: vec![],
                peer_quota_bytes: 1024 * 1024,
                rotate_key: false,
            },
            1000,
        )
        .unwrap();
    assert_eq!(
        p.source
            .federation_peer_status("remote.example", 1000)
            .unwrap()
            .block,
        Some(Block::FederationDisabled)
    );
    let unchanged = p
        .source
        .federated_outbound(&p.alice, &p.request.message_id, 1000)
        .unwrap();
    assert_eq!(unchanged.attempts, 0);
    assert_eq!(
        unchanged.state,
        sigil_protocol::federation::OutboundState::Pending
    );
    assert!(matches!(
        p.source.federation_peer_status("unknown.example", 1000),
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        p.source.federation_peer_status("bad/host", 1000),
        Err(StoreError::Invalid(_))
    ));
}

#[tokio::test]
async fn diagnostic_routes_require_admin_and_never_accept_device_or_browser_authority() {
    use axum::{
        body::{to_bytes, Body},
        http::Request,
    };
    use tower::ServiceExt;
    let p = Pair::new(1000, "chat.example", "remote.example");
    let path = p.dir.path().join("admin");
    let admin = crate::auth::AdminToken::load_or_create(&path).unwrap();
    let secret = std::fs::read_to_string(&path).unwrap();
    let app = crate::router(p.source, admin);
    for endpoint in [
        "/admin/v0/federation/status",
        "/admin/v0/federation/peers/remote.example/status",
    ] {
        for (token, origin, expected) in [
            (None, false, 401),
            (Some(p.alice.as_str()), false, 401),
            (Some(secret.trim()), true, 403),
            (Some(secret.trim()), false, 200),
        ] {
            let mut request = Request::get(endpoint);
            if let Some(token) = token {
                request = request.header("authorization", format!("Bearer {token}"));
            }
            if origin {
                request = request.header("origin", "https://untrusted.example");
            }
            let response = app
                .clone()
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status().as_u16(), expected);
            let bytes = to_bytes(response.into_body(), 8192).await.unwrap();
            assert!(!std::str::from_utf8(&bytes).unwrap().contains(secret.trim()));
        }
    }
}

#[test]
fn quota_reduction_reports_no_available_bytes_and_restore_retains_only_evidence() {
    let mut p = Pair::new(1000, "chat.example", "remote.example");
    p.source
        .configure_federation(
            federation_config::Configure {
                expected_revision: 1,
                enabled: true,
                exceptions: vec![],
                peer_quota_bytes: 2 * 1024 * 1024,
                rotate_key: false,
            },
            1000,
        )
        .unwrap();
    for n in 0..12u8 {
        let mut request = p.request.clone();
        request.message_id = crate::federation_auth::hex(&[n; 32]);
        request.payload = "ab".repeat(sigil_protocol::mailbox::MAX_PAYLOAD_HEX / 2);
        p.source
            .queue_federated_message(&p.alice, request, 1000)
            .unwrap();
    }
    p.source
        .configure_federation(
            federation_config::Configure {
                expected_revision: 2,
                enabled: true,
                exceptions: vec![],
                peer_quota_bytes: 1024 * 1024,
                rotate_key: false,
            },
            1000,
        )
        .unwrap();
    let full = p
        .source
        .federation_peer_status("remote.example", 1000)
        .unwrap();
    assert!(full.resources.reserved_bytes > full.resources.limit_bytes);
    assert_eq!(full.resources.available_bytes, 0);
    assert_eq!(full.queue.pending, 12);
    // Being over the admission budget does not stop delivery of already charged work.
    assert_eq!(full.queue.ready, 12);
    let backup = p.dir.path().join("backup.db");
    p.source.backup(&backup).unwrap();
    let restored = p.dir.path().join("restored.db");
    Store::restore(&backup, &restored).unwrap();
    let mut restored = Store::open(&restored).unwrap();
    let state = restored
        .federation_peer_status("remote.example", 1000)
        .unwrap();
    assert_eq!(state.block, Some(Block::FederationDisabled));
    assert_eq!(state.queue, Queue::default());
    assert_eq!(state.egress_bytes, 12 * crate::federation_outbox::METADATA);
    assert_eq!(
        state.resources.available_bytes,
        state.resources.limit_bytes - state.egress_bytes
    );
    assert_eq!(
        restored.federation_status().unwrap().egress.reserved_bytes,
        state.egress_bytes
    );
}
