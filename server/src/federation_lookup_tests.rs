use super::*;
use crate::federation_outbox::tests::Pair;
use sigil_crypto::{handshake::Receiver, IdentityKey};
#[test]
fn exact_account_discovery_is_authenticated_and_respects_opt_out() {
    let mut p = Pair::new(1000, "chat.example", "remote.example");
    let request = ProxyLookup {
        destination: p.request.destination.clone(),
        operation: Lookup::Account {
            username: "synthetic".into(),
        },
    };
    let proxy = p
        .source
        .prepare_federation_lookup(&p.alice, request.clone(), 1000)
        .unwrap();
    let result = call(&mut p.sink, &proxy, 1000);
    assert!(
        matches!(result.value,LookupValue::Account(ref a) if a.valid_for("synthetic","remote.example"))
    );
    let pref = p.sink.discovery_preference(&p.bob, None, 1000).unwrap();
    p.sink
        .discovery_preference(
            &p.bob,
            Some(sigil_protocol::admin::DiscoveryPreference {
                revision: pref.revision,
                discoverable: false,
            }),
            1000,
        )
        .unwrap();
    let proxy = p
        .source
        .prepare_federation_lookup(&p.alice, request, 1000)
        .unwrap();
    let result = proxy.send_with(
        || Ok(1000),
        |request| {
            assert!(matches!(
                p.sink
                    .receive_federation_lookup(request.body(), request.headers(), 1000),
                Err(StoreError::NotFound)
            ));
            Ok(crate::egress::Response {
                status: 404,
                retry_after: None,
                content_type: None,
                body: Zeroizing::new(Vec::new()),
            })
        },
    );
    assert!(result.is_err());
    assert!(p
        .source
        .prepare_federation_lookup(
            &"ab".repeat(32),
            ProxyLookup {
                destination: p.request.destination.clone(),
                operation: Lookup::Account {
                    username: "bob".into()
                }
            },
            1000
        )
        .is_err());
}
fn publish(p: &mut Pair, identity: &IdentityKey, now: u64) -> String {
    let receiver = Receiver::generate(identity, true).unwrap();
    let bundle = receiver.bundle().unwrap();
    let id = auth::hex(&bundle.prekey_id());
    p.sink
        .publish_prekey(
            &p.bob,
            &id,
            sigil_protocol::prekeys::PublishPrekey {
                bundle: auth::hex(&bundle.to_bytes()),
                expires_in_seconds: 3600,
            },
            now,
        )
        .unwrap();
    id
}
fn operation(p: &Pair, n: u8) -> ProxyLookup {
    ProxyLookup {
        destination: p.request.destination.clone(),
        operation: Lookup::Claim {
            device: p.request.recipient_device.clone(),
            request_id: auth::hex(&[n; 32]),
        },
    }
}
fn response(value: &LookupReply) -> crate::egress::Response {
    crate::egress::Response {
        status: 200,
        retry_after: None,
        content_type: Some("application/json".into()),
        body: Zeroizing::new(serde_json::to_vec(value).unwrap()),
    }
}
fn call(sink: &mut Store, proxy: &Proxy, now: u64) -> LookupReply {
    proxy
        .send_with(
            || Ok(now),
            |request| {
                let value = sink
                    .receive_federation_lookup(request.body(), request.headers(), now)
                    .unwrap();
                Ok(response(&value))
            },
        )
        .unwrap_or_else(|_| panic!("synthetic lookup failed"))
}
fn statement(p: &mut Pair, identity: &IdentityKey, now: u64) -> String {
    let own = p.sink.session(&p.bob, now).unwrap();
    let (username, server) = own.address[1..].split_once(':').unwrap();
    let binding = sigil_protocol::device::Binding {
        server: server.into(),
        username: username.into(),
        account: auth::bytes32(&own.account_id).unwrap(),
        device: auth::bytes32(&own.device_id).unwrap(),
        identity: identity.public_key(),
    };
    let signature = identity.sign(&binding.signing_bytes().unwrap()).unwrap();
    let signed = sigil_protocol::device::SignedBinding { binding, signature };
    let value = auth::hex(&signed.to_bytes().unwrap());
    p.sink
        .publish_device_binding(
            &p.bob,
            sigil_protocol::device::Statement {
                statement: value.clone(),
            },
            now,
        )
        .unwrap();
    value
}
#[test]
fn lost_lookup_response_retries_the_same_assignment_and_preserves_signed_binding() {
    let mut p = Pair::new(1000, "chat.example", "remote.example");
    let identity = IdentityKey::generate().unwrap();
    let signed = statement(&mut p, &identity, 1000);
    publish(&mut p, &identity, 1000);
    publish(&mut p, &identity, 1000);
    let request = operation(&p, 1);
    let proxy = p
        .source
        .prepare_federation_lookup(&p.alice, request.clone(), 1000)
        .unwrap();
    let lost = proxy.send_with(
        || Ok(1000),
        |request| {
            p.sink
                .receive_federation_lookup(request.body(), request.headers(), 1000)
                .unwrap();
            Err(crate::egress::Error::Transport)
        },
    );
    assert!(matches!(lost, Err(Failure::Transport)));
    assert_eq!(p.sink.prekey_inventory(&p.bob, 1000).unwrap().available, 1);
    let path = p.dir.path().join("sink.db");
    drop(p.sink);
    p.sink = Store::open(&path).unwrap();
    let again = p
        .source
        .prepare_federation_lookup(&p.alice, request, 1001)
        .unwrap();
    assert_eq!(proxy.body.as_slice(), again.body.as_slice());
    let result = call(&mut p.sink, &again, 1001);
    let accepted = p
        .source
        .finish_federation_lookup(&again, result, 1001)
        .unwrap();
    let LookupValue::Prekey(claimed) = accepted.value else {
        panic!("expected prekey")
    };
    let bytes = claimed
        .bundle
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect::<Vec<_>>();
    let bundle =
        sigil_crypto::handshake::Bundle::from_bytes(&bytes, &identity.public_key()).unwrap();
    assert_eq!(auth::hex(&bundle.prekey_id()), claimed.prekey_id);
    assert_eq!(p.sink.prekey_inventory(&p.bob, 1001).unwrap().available, 1);
    let proxy = p
        .source
        .prepare_federation_lookup(
            &p.alice,
            ProxyLookup {
                destination: p.request.destination.clone(),
                operation: Lookup::Binding {
                    device: p.request.recipient_device.clone(),
                },
            },
            1001,
        )
        .unwrap();
    let result = call(&mut p.sink, &proxy, 1001);
    assert_eq!(result.value, LookupValue::Binding(signed));
}
#[test]
fn remote_claim_rollback_does_not_consume_nonce_inventory_or_reservations() {
    let mut p = Pair::new(1000, "chat.example", "remote.example");
    let identity = IdentityKey::generate().unwrap();
    let id = publish(&mut p, &identity, 1000);
    let proxy = p
        .source
        .prepare_federation_lookup(&p.alice, operation(&p, 1), 1000)
        .unwrap();
    let before: i64 = p
        .sink
        .0
        .query_row("SELECT sum(bytes) FROM retained_storage", [], |r| r.get(0))
        .unwrap();
    p.sink.0.execute_batch("CREATE TRIGGER synthetic_failure BEFORE UPDATE ON prekeys BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    let result = proxy.send_with(
        || Ok(1000),
        |request| {
            assert!(p
                .sink
                .receive_federation_lookup(request.body(), request.headers(), 1000)
                .is_err());
            Err(crate::egress::Error::Transport)
        },
    );
    assert!(result.is_err());
    assert_eq!(p.sink.prekey_inventory(&p.bob, 1000).unwrap().available, 1);
    assert_eq!(
        p.sink
            .0
            .query_row("SELECT sum(bytes) FROM retained_storage", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        before
    );
    assert_eq!(
        p.sink
            .0
            .query_row("SELECT count(*) FROM federation_nonces", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    p.sink
        .0
        .execute_batch("DROP TRIGGER synthetic_failure")
        .unwrap();
    let result = call(&mut p.sink, &proxy, 1000);
    let LookupValue::Prekey(value) = result.value else {
        panic!()
    };
    assert_eq!(value.prekey_id, id);
    assert_eq!(p.sink.prekey_inventory(&p.bob, 1000).unwrap().available, 0);
    assert!(matches!(
        p.sink
            .claim_prekey(&p.bob, &p.request.recipient_device, &"aa".repeat(32), 1000),
        Err(StoreError::NotFound)
    ));
    let backup = p.dir.path().join("backup.db");
    p.sink.backup(&backup).unwrap();
    let restored = p.dir.path().join("restored.db");
    Store::restore(&backup, &restored).unwrap();
    let restored = Store::open(&restored).unwrap();
    let used: i64 = restored
        .0
        .query_row("SELECT ingress_bytes FROM federation_usage", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(used, crate::federation_mailbox::METADATA as i64);
    let bundle: Option<Vec<u8>> = restored
        .0
        .query_row("SELECT bundle FROM prekeys WHERE id=?1", [id], |r| r.get(0))
        .unwrap();
    assert!(bundle.is_none());
}
#[test]
fn local_and_remote_claims_cannot_assign_the_same_one_time_prekey() {
    let mut p = Pair::new(1000, "chat.example", "remote.example");
    let identity = IdentityKey::generate().unwrap();
    publish(&mut p, &identity, 1000);
    let proxy = p
        .source
        .prepare_federation_lookup(&p.alice, operation(&p, 1), 1000)
        .unwrap();
    let key = proxy.config.key.as_ref().unwrap();
    let headers = auth::sign(
        key,
        &auth::Request {
            origin: "chat.example",
            destination: "remote.example",
            path: LOOKUP_PATH,
            body: proxy.body.as_slice(),
        },
        1000,
        [1; 32],
    )
    .unwrap();
    let body = proxy.body.to_vec();
    let path = p.dir.path().join("sink.db");
    let local_path = path.clone();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let remote_barrier = barrier.clone();
    let token = p.bob.clone();
    let device = p.request.recipient_device.clone();
    let local = std::thread::spawn(move || {
        let mut s = Store::open(&local_path).unwrap();
        barrier.wait();
        s.claim_prekey(&token, &device, &"bb".repeat(32), 1000)
            .is_ok()
    });
    let remote = std::thread::spawn(move || {
        let mut s = Store::open(&path).unwrap();
        remote_barrier.wait();
        s.receive_federation_lookup(&body, &headers, 1000).is_ok()
    });
    assert_eq!(
        usize::from(local.join().unwrap()) + usize::from(remote.join().unwrap()),
        1
    );
    assert_eq!(p.sink.prekey_inventory(&p.bob, 1000).unwrap().available, 0);
    let assigned:i64=p.sink.0.query_row("SELECT count(*) FROM prekeys WHERE (claimant IS NOT NULL AND remote_server IS NULL) OR (claimant IS NULL AND remote_server IS NOT NULL)",[],|r|r.get(0)).unwrap();
    assert_eq!(assigned, 1);
}
#[test]
fn revocation_policy_and_altered_lookup_receipts_fail_closed() {
    let mut p = Pair::new(1000, "chat.example", "remote.example");
    let identity = IdentityKey::generate().unwrap();
    publish(&mut p, &identity, 1000);
    let proxy = p
        .source
        .prepare_federation_lookup(&p.alice, operation(&p, 1), 1000)
        .unwrap();
    let reply = call(&mut p.sink, &proxy, 1000);
    let mut bad = reply.clone();
    bad.request_hash = "00".repeat(32);
    assert!(p
        .source
        .finish_federation_lookup(&proxy, bad, 1000)
        .is_err());
    let mut bad = reply.clone();
    if let LookupValue::Prekey(value) = &mut bad.value {
        value.device_id = "ff".repeat(32);
    }
    assert!(p
        .source
        .finish_federation_lookup(&proxy, bad, 1000)
        .is_err());
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
    assert!(matches!(
        p.source.finish_federation_lookup(&proxy, reply, 1001),
        Err(StoreError::Conflict)
    ));
    let own = p.source.session(&p.alice, 1000).unwrap();
    p.sink
        .configure_federation_sender(
            &p.bob,
            sigil_protocol::federation::ConfigureSender {
                expected_revision: 1,
                sender: sigil_protocol::federation::RemoteSender {
                    server: "chat.example".into(),
                    account: own.account_id,
                    device: own.device_id,
                },
                allowed: false,
            },
            1001,
        )
        .unwrap();
    let result = proxy.send_with(
        || Ok(1001),
        |request| {
            assert!(matches!(
                p.sink
                    .receive_federation_lookup(request.body(), request.headers(), 1001),
                Err(StoreError::Forbidden)
            ));
            Err(crate::egress::Error::Transport)
        },
    );
    assert!(result.is_err());
}

#[test]
fn signed_lookup_crosses_https_and_rejects_query_or_unsigned_requests() {
    use axum::http::Request;
    let _network = crate::egress::tests::NETWORK.lock().unwrap();
    let now = crate::enrollment::now().unwrap();
    let mut p = Pair::new(now, "remote.example", "chat.example");
    let identity = IdentityKey::generate().unwrap();
    let id = publish(&mut p, &identity, now);
    let request = operation(&p, 1);
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
    let (config, peer) = p
        .source
        .begin_federation_refresh("chat.example", now)
        .unwrap();
    p.source
        .finish_federation_refresh(config.revision, &peer, Some(discovery), now)
        .unwrap();
    let proxy = p
        .source
        .prepare_federation_lookup(&p.alice, request, now)
        .unwrap();
    let result = proxy
        .send_with(|| Ok(now), |request| fixture.federation(request))
        .unwrap_or_else(|_| panic!("synthetic TLS lookup failed"));
    let reply = p
        .source
        .finish_federation_lookup(&proxy, result, now)
        .unwrap();
    assert!(matches!(reply.value, LookupValue::Prekey(ref key) if key.prekey_id == id));
    let rejected = proxy.send_with(
        || Ok(now),
        |request| {
            let (mut parts, body) = request.into_parts();
            parts.uri = format!("{}?extra=1", parts.uri).parse().unwrap();
            fixture.federation(Request::from_parts(parts, body))
        },
    );
    assert!(matches!(rejected, Err(Failure::Remote { status: 422, .. })));
    let unsigned = fixture
        .federation(
            Request::post(fixture.uri("chat.example", LOOKUP_PATH))
                .header("content-type", "application/json")
                .body(proxy.body.as_slice())
                .unwrap(),
        )
        .unwrap();
    assert_eq!(unsigned.status, 401);
}

#[tokio::test]
async fn lookup_proxy_rejects_missing_credentials_browser_origin_and_spoofed_identity() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let now = crate::enrollment::now().unwrap();
    let p = Pair::new(now, "chat.example", "remote.example");
    let request = operation(&p, 1);
    let admin = crate::auth::AdminToken::load_or_create(&p.dir.path().join("admin")).unwrap();
    let app = crate::router(p.source, admin);
    let body = serde_json::to_vec(&request).unwrap();
    for (token, origin, expected) in [
        (None, false, StatusCode::UNAUTHORIZED),
        (Some(p.alice.as_str()), true, StatusCode::FORBIDDEN),
    ] {
        let mut request = Request::post("/client/v0/federation/lookup")
            .header("content-type", "application/json");
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        if origin {
            request = request.header("origin", "https://untrusted.example");
        }
        let result = app
            .clone()
            .oneshot(request.body(Body::from(body.clone())).unwrap())
            .await
            .unwrap();
        assert_eq!(result.status(), expected);
    }
    let mut spoof = serde_json::to_value(request).unwrap();
    spoof["sender_account"] = serde_json::Value::String("00".repeat(32));
    let result = app
        .oneshot(
            Request::post("/client/v0/federation/lookup")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", p.alice))
                .body(Body::from(serde_json::to_vec(&spoof).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(result.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[test]
fn federated_services_bind_device_ownership_and_rollback_nonce_with_issuance() {
    let mut p = Pair::new(1000, "chat.example", "remote.example");
    let identity = IdentityKey::generate().unwrap();
    let binding = statement(&mut p, &identity, 1000);
    p.source
        .configure_groups(sigil_protocol::groups::Configure {
            expected_revision: 0,
            enabled: true,
            storage_limit_bytes: 1024 * 1024,
        })
        .unwrap();
    let profile = crate::group_authority::read(&p.source.0)
        .unwrap()
        .authority
        .unwrap();
    let request = ProxyLookup {
        destination: "chat.example".into(),
        operation: Lookup::Service {
            service: Service::GroupCredential {
                authority: auth::hex(&profile.id()),
                day: 0,
                binding: binding.clone(),
            },
        },
    };
    let proxy = p
        .sink
        .prepare_federation_lookup(&p.bob, request.clone(), 1000)
        .unwrap();
    let wire: ServerLookup = serde_json::from_slice(&proxy.body).unwrap();
    assert_eq!(wire.sender_device, p.request.recipient_device);
    p.source.0.execute_batch("CREATE TRIGGER fail_uid BEFORE INSERT ON group_credential_uids BEGIN SELECT RAISE(ABORT,'synthetic uid failure'); END;").unwrap();
    let before: u32 = p
        .source
        .0
        .query_row("SELECT count(*) FROM federation_nonces", [], |r| r.get(0))
        .unwrap();
    let headers = auth::sign(
        proxy.config.key.as_ref().unwrap(),
        &auth::Request {
            origin: "remote.example",
            destination: "chat.example",
            path: LOOKUP_PATH,
            body: &proxy.body,
        },
        1000,
        [71; 32],
    )
    .unwrap();
    assert!(p
        .source
        .receive_federation_lookup(&proxy.body, &headers, 1000)
        .is_err());
    assert_eq!(
        p.source
            .0
            .query_row("SELECT count(*) FROM federation_nonces", [], |r| r
                .get::<_, u32>(0))
            .unwrap(),
        before
    );
    p.source.0.execute_batch("DROP TRIGGER fail_uid").unwrap();
    let reply = p
        .source
        .receive_federation_lookup(&proxy.body, &headers, 1000)
        .unwrap();
    let LookupValue::Service(value) = reply.value else {
        panic!("credential expected")
    };
    let value: sigil_protocol::groups::CredentialResponse = serde_json::from_str(&value).unwrap();
    let raw = crate::group_authority::decode(&binding, 512).unwrap();
    let issuance = profile.issuance(&raw, 0).unwrap();
    assert_eq!(value.uid, auth::hex(&issuance.uid));
    sigil_crypto::private_credentials::Credential::accept(
        profile.issuer(),
        issuance.attributes().unwrap(),
        0,
        issuance.context(),
        &crate::group_authority::decode(&value.response, 352).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        p.source
            .receive_federation_lookup(&proxy.body, &headers, 1000),
        Err(StoreError::Conflict)
    ));
    let mut forged = request;
    let Lookup::Service {
        service: Service::GroupCredential { binding, .. },
    } = &mut forged.operation
    else {
        unreachable!()
    };
    let changed = if binding.ends_with('0') { "1" } else { "0" };
    binding.replace_range(binding.len() - 1.., changed);
    assert!(p
        .sink
        .prepare_federation_lookup(&p.bob, forged, 1000)
        .is_err());
    let proxy = p
        .sink
        .prepare_federation_lookup(
            &p.bob,
            ProxyLookup {
                destination: "chat.example".into(),
                operation: Lookup::Service {
                    service: Service::GroupAuthority,
                },
            },
            1001,
        )
        .unwrap();
    let wire: ServerLookup = serde_json::from_slice(&proxy.body).unwrap();
    assert!(wire.sender_account.is_empty() && wire.sender_device.is_empty());
    let result = call(&mut p.source, &proxy, 1001);
    assert_eq!(
        result.value,
        LookupValue::Service(serde_json::to_string(&auth::hex(&profile.to_bytes())).unwrap())
    );
    let invalid = p
        .sink
        .prepare_federation_lookup(
            &p.bob,
            ProxyLookup {
                destination: "chat.example".into(),
                operation: Lookup::Service {
                    service: Service::GroupRequest {
                        request: "{}".into(),
                    },
                },
            },
            1001,
        )
        .unwrap();
    let before: u32 = p
        .source
        .0
        .query_row("SELECT count(*) FROM federation_nonces", [], |r| r.get(0))
        .unwrap();
    let headers = auth::sign(
        invalid.config.key.as_ref().unwrap(),
        &auth::Request {
            origin: "remote.example",
            destination: "chat.example",
            path: LOOKUP_PATH,
            body: &invalid.body,
        },
        1001,
        [72; 32],
    )
    .unwrap();
    assert!(p
        .source
        .receive_federation_lookup(&invalid.body, &headers, 1001)
        .is_err());
    assert_eq!(
        p.source
            .0
            .query_row("SELECT count(*) FROM federation_nonces", [], |r| r
                .get::<_, u32>(0))
            .unwrap(),
        before
    );
}

#[tokio::test]
async fn proxy_lookup_keeps_its_read_budget_when_native_writes_are_exhausted() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let now = crate::enrollment::now().unwrap();
    let mut p = Pair::new(now, "chat.example", "remote.example");
    p.source
        .configure_federation(
            federation_config::Configure {
                expected_revision: 1,
                enabled: false,
                exceptions: vec![],
                peer_quota_bytes: 1024 * 1024,
                rotate_key: false,
            },
            now,
        )
        .unwrap();
    let token = crate::auth::AdminToken::load_or_create(&p.dir.path().join("admin")).unwrap();
    let app = crate::router(p.source, token);
    let request = |path: &str, body: Vec<u8>| {
        Request::post(path)
            .header("authorization", format!("Bearer {}", p.alice))
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap()
    };
    for _ in 0..64 {
        let response = app
            .clone()
            .oneshot(request(
                "/client/v0/federation/messages",
                serde_json::to_vec(&p.request).unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    assert_eq!(
        app.clone()
            .oneshot(request(
                "/client/v0/federation/messages",
                serde_json::to_vec(&p.request).unwrap()
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    let lookup = ProxyLookup {
        destination: "remote.example".into(),
        operation: Lookup::Service {
            service: Service::GroupAuthority,
        },
    };
    assert_eq!(
        app.oneshot(request(
            "/client/v0/federation/lookup",
            serde_json::to_vec(&lookup).unwrap()
        ))
        .await
        .unwrap()
        .status(),
        StatusCode::FORBIDDEN
    );
}
