use super::*;
use crate::network::tests::{Fixture, CA};
use axum::{
    routing::{get, post},
    Json, Router,
};
use std::sync::{Arc, Mutex};
fn own() -> accounts::Session {
    accounts::Session {
        account_id: "01".repeat(32),
        address: "@alice:chat.example".into(),
        device_id: "02".repeat(32),
        device_label: "Synthetic".into(),
        expires_at: 3000,
    }
}
fn queue() -> Queue {
    Queue {
        destination: "remote.example".into(),
        recipient_device: "03".repeat(32),
        message_id: "04".repeat(32),
        payload: "ab".repeat(32),
        expires_at: 2000,
    }
}
fn client(fixture: &Fixture, token: &str) -> HttpsClient {
    HttpsClient::new("chat.example", fixture.port(), token, &[CA.to_vec()]).unwrap()
}
#[test]
fn real_home_server_preserves_pending_status_and_revisioned_permissions() {
    use sigil_server::{auth::AdminToken, federation_config, store::Store};
    let dir = tempfile::tempdir().unwrap();
    let now = crate::schedule::clock().unwrap();
    let mut server = Store::open(&dir.path().join("s.db")).unwrap();
    server
        .configure(sigil_protocol::Configure {
            expected_revision: 0,
            settings: sigil_protocol::Settings {
                server_name: "chat.example".into(),
                default_quota_bytes: sigil_protocol::DEFAULT_QUOTA,
                max_attachment_bytes: sigil_protocol::DEFAULT_ATTACHMENT_LIMIT,
            },
        })
        .unwrap();
    server
        .configure_federation(
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
    server
        .configure_federation_peer(
            "remote.example",
            federation_config::ConfigurePeer {
                expected_revision: 0,
                allowed: true,
                port: 443,
                approve_key: None,
            },
        )
        .unwrap();
    let invitation = server
        .invite(
            accounts::InviteRequest {
                username: "alice".into(),
                expires_in_seconds: 60,
            },
            now,
        )
        .unwrap();
    let token = "ab".repeat(32);
    let own = server
        .enroll(
            accounts::Enrollment {
                invitation: invitation.secret,
                device_credential: token.clone(),
                device_label: "Synthetic".into(),
            },
            now,
        )
        .unwrap();
    let app = sigil_server::router(
        server,
        AdminToken::load_or_create(&dir.path().join("admin")).unwrap(),
    );
    let fixture = Fixture::new(app);
    let client = client(&fixture, &token);
    let mut request = queue();
    request.expires_at = now + 1000;
    let pending = client.queue_federated_message(&own, &request).unwrap();
    assert_eq!(pending.state, OutboundState::Pending);
    assert!(pending.receipt.is_none());
    assert_eq!(
        client.queue_federated_message(&own, &request).unwrap(),
        pending
    );
    assert_eq!(client.federated_outbound(&own, &request).unwrap(), pending);
    let mut altered = request.clone();
    altered.expires_at += 1;
    assert_eq!(
        client.federated_outbound(&own, &altered),
        Err(Error::InvalidResponse)
    );
    let allow = ConfigureSender {
        expected_revision: 0,
        sender: RemoteSender {
            server: "remote.example".into(),
            account: "05".repeat(32),
            device: "06".repeat(32),
        },
        allowed: true,
    };
    let permission = client.configure_federation_sender(&allow).unwrap();
    assert_eq!(permission.revision, 1);
    assert_eq!(
        client.configure_federation_sender(&allow).unwrap(),
        permission
    );
    assert_eq!(
        client.federated_senders().unwrap(),
        vec![allow.sender.clone()]
    );
    let mut remove = allow.clone();
    remove.expected_revision = 1;
    remove.allowed = false;
    let removed = client.configure_federation_sender(&remove).unwrap();
    assert_eq!(removed.revision, 2);
    assert_eq!(
        client
            .federation_sender_permission(&allow.sender.server, &allow.sender.device)
            .unwrap(),
        removed
    );
    assert!(client.federated_senders().unwrap().is_empty());
    assert!(matches!(
        client.configure_federation_sender(&allow),
        Err(Error::Status { code: 409, .. })
    ));
    assert!(client.mailbox_after(0).unwrap().is_empty());
}
#[test]
fn status_is_bound_to_the_exact_request_owner_and_remote_receipt() {
    let own = own();
    let request = queue();
    // Independently spell the canonical server body, including derived ownership.
    let raw=format!("{{\"sender_account\":\"{}\",\"sender_device\":\"{}\",\"recipient_device\":\"{}\",\"message_id\":\"{}\",\"payload\":\"{}\",\"expires_at\":{}}}",own.account_id,own.device_id,request.recipient_device,request.message_id,request.payload,request.expires_at);
    let hash = crate::transport::hex(&Sha256::digest(raw.as_bytes()));
    let good = Outbound {
        message_id: request.message_id.clone(),
        destination: request.destination.clone(),
        request_hash: hash.clone(),
        expires_at: request.expires_at,
        state: OutboundState::Accepted,
        receipt: Some(sigil_protocol::federation::Receipt {
            request_hash: hash,
            sequence: 1,
            expires_at: request.expires_at,
        }),
        not_before: 1000,
        attempts: 1,
        error: None,
    };
    let value = Arc::new(Mutex::new(good.clone()));
    let getter = value.clone();
    let setter = value.clone();
    let app = Router::new()
        .route(
            "/client/v0/federation/messages/{id}",
            get(move || {
                let value = getter.lock().unwrap().clone();
                async move { Json(value) }
            }),
        )
        .route(
            "/client/v0/federation/messages",
            post(move |Json(_request): Json<Queue>| {
                let value = setter.lock().unwrap().clone();
                async move { (axum::http::StatusCode::ACCEPTED, Json(value)) }
            }),
        );
    let fixture = Fixture::new(app);
    let client = client(&fixture, &"ab".repeat(32));
    assert_eq!(
        client.queue_federated_message(&own, &request).unwrap(),
        good
    );
    assert_eq!(client.federated_outbound(&own, &request).unwrap(), good);
    for n in 0..12 {
        let mut bad = good.clone();
        match n {
            0 => bad.message_id = "aa".repeat(32),
            1 => bad.destination = "wrong.example".into(),
            2 => bad.request_hash = "bb".repeat(32),
            3 => bad.expires_at += 1,
            4 => bad.state = OutboundState::Pending,
            5 => bad.receipt.as_mut().unwrap().sequence = 0,
            6 => bad.receipt.as_mut().unwrap().request_hash = "cc".repeat(32),
            7 => bad.receipt.as_mut().unwrap().expires_at += 1,
            8 => bad.attempts = 32,
            9 => bad.not_before = 0,
            10 => bad.error = Some("unexpected".into()),
            _ => bad.receipt = None,
        }
        *value.lock().unwrap() = bad;
        assert_eq!(
            client.federated_outbound(&own, &request),
            Err(Error::InvalidResponse), "GET case {n}"
        );
        assert_eq!(
            client.queue_federated_message(&own, &request),
            Err(Error::InvalidResponse), "POST case {n}"
        );
    }
    *value.lock().unwrap() = good;
    let mut wrong = own.clone();
    wrong.account_id = "dd".repeat(32);
    assert_eq!(
        client.federated_outbound(&wrong, &request),
        Err(Error::InvalidResponse)
    );
    wrong = own.clone();
    wrong.address = "@alice:wrong.example".into();
    assert_eq!(
        client.federated_outbound(&wrong, &request),
        Err(Error::Configuration)
    );
    let mut bad = request.clone();
    bad.destination = "remote.example/../../admin".into();
    assert_eq!(
        client.queue_federated_message(&own, &bad),
        Err(Error::Configuration)
    );
}
#[test]
fn mailbox_origin_order_and_maximum_envelopes_are_validated() {
    let entry = mailbox::Delivery {
        sequence: 1,
        sender_device: "02".repeat(32),
        origin: Some(RemoteSender {
            server: "remote.example".into(),
            account: "01".repeat(32),
            device: "02".repeat(32),
        }),
        message_id: "03".repeat(32),
        payload: "ab".repeat(32),
        expires_at: 2000,
    };
    let values = Arc::new(Mutex::new(vec![entry.clone()]));
    let captured = values.clone();
    let fixture = Fixture::new(Router::new().route(
        "/client/v0/mailbox",
        get(move || {
            let values = captured.lock().unwrap().clone();
            async move { Json(values) }
        }),
    ));
    let client = client(&fixture, &"ab".repeat(32));
    assert_eq!(client.mailbox_after(0).unwrap(), vec![entry.clone()]);
    assert_eq!(client.mailbox_after(1), Err(Error::InvalidResponse));
    for n in 0..7 {
        let mut bad = entry.clone();
        match n {
            0 => bad.origin.as_mut().unwrap().server = "chat.example".into(),
            1 => bad.origin.as_mut().unwrap().server = "remote.example/path".into(),
            2 => bad.origin.as_mut().unwrap().account = "zz".repeat(32),
            3 => bad.origin.as_mut().unwrap().device = "00".repeat(31),
            4 => bad.expires_at = 0,
            5 => bad.payload = "ab".into(),
            _ => bad.message_id = "ff".repeat(33),
        }
        *values.lock().unwrap() = vec![bad];
        assert_eq!(client.mailbox_after(0), Err(Error::InvalidResponse));
    }
    *values.lock().unwrap() = vec![entry.clone(), entry.clone()];
    assert_eq!(client.mailbox_after(0), Err(Error::InvalidResponse));
    let server = [
        "a".repeat(63),
        "b".repeat(63),
        "c".repeat(63),
        "d".repeat(61),
    ]
    .join(".");
    let mut maximum = Vec::new();
    for sequence in 1..=16 {
        let mut value = entry.clone();
        value.sequence = sequence;
        value.origin.as_mut().unwrap().server = server.clone();
        value.payload = "ab".repeat(mailbox::MAX_PAYLOAD_HEX / 2);
        maximum.push(value);
    }
    *values.lock().unwrap() = maximum.clone();
    assert_eq!(client.mailbox_after(0).unwrap(), maximum);
    maximum.push(entry);
    *values.lock().unwrap() = maximum;
    assert!(matches!(
        client.mailbox_after(0),
        Err(Error::Limit | Error::InvalidResponse)
    ));
}

#[test]
fn lookup_binds_response_to_owner_request_and_remote_device() {
    use sigil_protocol::federation::Lookup;
    let own = own();
    let request = ProxyLookup {
        destination: "remote.example".into(),
        operation: Lookup::Claim {
            device: "03".repeat(32),
            request_id: "04".repeat(32),
        },
    };
    let raw = format!(
        "{{\"sender_account\":\"{}\",\"sender_device\":\"{}\",\"operation\":{{\"kind\":\"claim\",\"device\":\"{}\",\"request_id\":\"{}\"}}}}",
        own.account_id, own.device_id, "03".repeat(32), "04".repeat(32)
    );
    let identity = sigil_crypto::IdentityKey::generate().unwrap();
    let receiver = sigil_crypto::handshake::Receiver::generate(&identity, true).unwrap();
    let bundle = receiver.bundle().unwrap();
    let prekey = prekeys::ClaimedPrekey {
        device_id: "03".repeat(32),
        prekey_id: crate::transport::hex(&bundle.prekey_id()),
        bundle: crate::transport::hex(&bundle.to_bytes()),
        expires_at: 2000,
    };
    let good = LookupReply {
        request_hash: crate::transport::hex(&Sha256::digest(raw.as_bytes())),
        value: LookupValue::Prekey(prekey.clone()),
    };
    let response = Arc::new(Mutex::new(good.clone()));
    let handler = response.clone();
    let expected = request.clone();
    let app = Router::new().route(
        "/client/v0/federation/lookup",
        post(move |Json(value): Json<ProxyLookup>| {
            assert_eq!(value, expected);
            let reply = handler.lock().unwrap().clone();
            async move { Json(reply) }
        }),
    );
    let fixture = Fixture::new(app);
    let client = client(&fixture, &"ab".repeat(32));
    assert_eq!(client.federated_lookup(&own, &request).unwrap(), good.value);
    for n in 0..7 {
        let mut bad = good.clone();
        let LookupValue::Prekey(ref mut key) = bad.value else {
            unreachable!()
        };
        match n {
            0 => bad.request_hash = "ff".repeat(32),
            1 => key.device_id = "ff".repeat(32),
            2 => key.prekey_id = "invalid".into(),
            3 => key.bundle.truncate(3546),
            4 => key.expires_at = 0,
            5 => key.bundle = "GG".repeat(1772),
            _ => bad.value = LookupValue::Binding("00".repeat(100)),
        }
        *response.lock().unwrap() = bad;
        assert_eq!(
            client.federated_lookup(&own, &request),
            Err(Error::InvalidResponse)
        );
    }
    *response.lock().unwrap() = good;
    let mut wrong = own.clone();
    wrong.account_id = "ff".repeat(32);
    assert_eq!(
        client.federated_lookup(&wrong, &request),
        Err(Error::InvalidResponse)
    );
    wrong.address = "@alice:wrong.example".into();
    assert_eq!(
        client.federated_lookup(&wrong, &request),
        Err(Error::Configuration)
    );
}

#[test]
fn anonymous_service_response_hash_omits_device_identifiers() {
    use sigil_protocol::federation::{Lookup, Service};
    let request = ProxyLookup {
        destination: "remote.example".into(),
        operation: Lookup::Service {
            service: Service::GroupAuthority,
        },
    };
    let raw = br#"{"sender_account":"","sender_device":"","operation":{"kind":"service","service":{"kind":"group_authority"}}}"#;
    let reply = LookupReply {
        request_hash: crate::transport::hex(&Sha256::digest(raw)),
        value: LookupValue::Service("synthetic".into()),
    };
    let app = Router::new().route(
        "/client/v0/federation/lookup",
        post(move || {
            let reply = reply.clone();
            async move { Json(reply) }
        }),
    );
    let fixture = Fixture::new(app);
    assert_eq!(
        client(&fixture, &"ab".repeat(32))
            .federated_lookup(&own(), &request)
            .unwrap(),
        LookupValue::Service("synthetic".into())
    );
}
