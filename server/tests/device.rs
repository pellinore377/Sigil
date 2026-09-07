use sigil_crypto::{handshake::Receiver, IdentityKey};
use sigil_protocol::{
    accounts::{Enrollment, InviteRequest, Session},
    device::{Binding, SignedBinding, Statement},
    prekeys::PublishPrekey,
    Configure,
};
use sigil_server::{
    auth::{random_secret, AdminToken},
    store::{Store, StoreError},
};
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn id(value: &str) -> [u8; 32] {
    let mut bytes = [0; 32];
    for (out, pair) in bytes.iter_mut().zip(value.as_bytes().as_chunks::<2>().0) {
        *out = u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap();
    }
    bytes
}
fn setup(path: &std::path::Path, now: u64) -> (Store, Vec<(String, Session, IdentityKey)>) {
    let mut store = Store::open(path).unwrap();
    store
        .configure(Configure {
            expected_revision: 0,
            settings: serde_json::from_str(r#"{"server_name":"chat.example"}"#).unwrap(),
        })
        .unwrap();
    let mut users = Vec::new();
    for name in ["alice", "bob"] {
        let invitation = store
            .invite(
                InviteRequest {
                    username: name.into(),
                    expires_in_seconds: 60,
                },
                now,
            )
            .unwrap();
        let token = random_secret().unwrap();
        let session = store
            .enroll(
                Enrollment {
                    invitation: invitation.secret,
                    device_credential: token.clone(),
                    device_label: "Synthetic".into(),
                },
                now,
            )
            .unwrap();
        users.push((token, session, IdentityKey::generate().unwrap()));
    }
    (store, users)
}
fn signed(session: &Session, identity: &IdentityKey) -> SignedBinding {
    let (username, server) = session.address[1..].split_once(':').unwrap();
    let binding = Binding {
        server: server.into(),
        username: username.into(),
        account: id(&session.account_id),
        device: id(&session.device_id),
        identity: identity.public_key(),
    };
    let signature = identity.sign(&binding.signing_bytes().unwrap()).unwrap();
    SignedBinding { binding, signature }
}
fn request(signed: &SignedBinding) -> Statement {
    Statement {
        statement: hex(&signed.to_bytes().unwrap()),
    }
}

#[test]
fn statements_require_account_binding_and_contact_access_and_cannot_replace_keys() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    let (mut store, users) = setup(&path, 1000);
    let (alice, a, ak) = &users[0];
    let (bob, b, bk) = &users[1];
    let proof = signed(a, ak);
    for field in 0..4 {
        let mut wrong = proof.clone();
        match field {
            0 => wrong.binding.server = "other.example".into(),
            1 => wrong.binding.username = "bob".into(),
            2 => wrong.binding.account = id(&b.account_id),
            _ => wrong.binding.device = id(&b.device_id),
        }
        wrong.signature = ak.sign(&wrong.binding.signing_bytes().unwrap()).unwrap();
        assert!(store
            .publish_device_binding(alice, request(&wrong), 1000)
            .is_err());
    }
    store
        .publish_device_binding(alice, request(&proof), 1000)
        .unwrap();
    store
        .publish_device_binding(alice, request(&proof), 1001)
        .unwrap();
    assert!(matches!(
        store.device_binding(bob, &a.device_id, 1000),
        Err(StoreError::Forbidden)
    ));
    store.allow_sender(alice, &b.device_id, 1000).unwrap();
    assert_eq!(
        store
            .device_binding(bob, &a.device_id, 1000)
            .unwrap()
            .statement,
        request(&proof).statement
    );
    store
        .publish_device_binding(bob, request(&signed(b, bk)), 1000)
        .unwrap();
    assert!(store.device_binding(alice, &b.device_id, 1000).is_ok());
    assert!(store
        .publish_device_binding(alice, request(&signed(a, bk)), 1000)
        .is_err());
    let receiver = Receiver::generate(bk, false).unwrap();
    let bundle = receiver.bundle().unwrap();
    assert!(store
        .publish_prekey(
            alice,
            &hex(&bundle.prekey_id()),
            PublishPrekey {
                bundle: hex(&bundle.to_bytes()),
                expires_in_seconds: 3600
            },
            1000
        )
        .is_err());
    drop(store);
    let mut store = Store::open(&path).unwrap();
    assert_eq!(
        store
            .device_binding(alice, &a.device_id, 1001)
            .unwrap()
            .statement,
        request(&proof).statement
    );
    store.revoke_device(alice, &a.device_id, 1001).unwrap();
    assert!(matches!(
        store.device_binding(bob, &a.device_id, 1001),
        Err(StoreError::NotFound)
    ));
    assert!(store
        .publish_device_binding(alice, request(&proof), 1001)
        .is_err());
}

#[test]
fn migration_does_not_invent_statements_and_restore_revokes_their_devices() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    let (store, users) = setup(&path, 1000);
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch(
        "DROP TABLE private_group_nonces; DROP TABLE private_group_commits; DROP TABLE private_group_members; DROP TABLE private_groups; DROP TABLE group_credential_uids; DROP TABLE group_authority; DROP INDEX prekeys_available; DROP INDEX prekeys_remote_claim; ALTER TABLE prekeys DROP COLUMN remote_request; ALTER TABLE prekeys DROP COLUMN remote_device; ALTER TABLE prekeys DROP COLUMN remote_account; ALTER TABLE prekeys DROP COLUMN remote_server; CREATE INDEX prekeys_available ON prekeys(device_id,expires_at) WHERE bundle IS NOT NULL AND claimant IS NULL; DROP TABLE federation_outbox; DROP TABLE federation_revocations; DROP TABLE federation_senders; DROP TABLE federation_nonces; DROP TABLE federation_admission; DROP TABLE federation_usage; DROP TABLE federation_peers; DROP TABLE federation_configuration; DROP TABLE push_jobs; DROP TABLE push_channels; DROP TABLE push_configuration; DROP TABLE attachment_chunks; DROP TABLE attachments; DROP TABLE retained_storage; DROP INDEX mailbox_live_recipient; DROP INDEX prekeys_available; DROP INDEX devices_active_account; DROP TABLE cancelled_device_links; DROP TABLE device_links; DROP INDEX devices_account_id; DROP TABLE device_bindings; PRAGMA user_version=8;",
    )
    .unwrap();
    let mut store = Store::open(&path).unwrap();
    let (token, session, key) = &users[0];
    assert!(matches!(
        store.device_binding(token, &session.device_id, 1000),
        Err(StoreError::NotFound)
    ));
    store
        .publish_device_binding(token, request(&signed(session, key)), 1000)
        .unwrap();
    let backup = dir.path().join("backup.db");
    let restored = dir.path().join("restored.db");
    store.backup(&backup).unwrap();
    Store::restore(&backup, &restored).unwrap();
    let mut store = Store::open(&restored).unwrap();
    assert!(matches!(
        store.device_binding(token, &session.device_id, 1000),
        Err(StoreError::Unauthorized)
    ));
}

#[tokio::test]
async fn binding_http_is_native_authenticated_and_size_bounded() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let dir = tempfile::tempdir().unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let (store, users) = setup(&dir.path().join("server.db"), now);
    let (token, session, key) = &users[0];
    let bytes = serde_json::to_vec(&request(&signed(session, key))).unwrap();
    let app = sigil_server::router(
        store,
        AdminToken::load_or_create(&dir.path().join("admin.token")).unwrap(),
    );
    for (auth, origin, body, expected) in [
        (false, false, bytes.clone(), StatusCode::UNAUTHORIZED),
        (true, true, bytes.clone(), StatusCode::FORBIDDEN),
        (true, false, vec![0; 2049], StatusCode::PAYLOAD_TOO_LARGE),
        (true, false, bytes, StatusCode::NO_CONTENT),
    ] {
        let mut request = Request::builder()
            .method("PUT")
            .uri("/client/v0/device-binding")
            .header("content-type", "application/json");
        if auth {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        if origin {
            request = request.header("origin", "https://chat.example");
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        assert_eq!(response.headers()["cache-control"], "no-store");
    }
}

fn link_proof(
    session: &Session,
    identity: &IdentityKey,
    token: &str,
    nonce: u8,
) -> sigil_protocol::link::Proof {
    use sha2::{Digest, Sha256};
    let sponsor = signed(session, identity);
    let target = IdentityKey::generate().unwrap();
    let mut binding = sponsor.binding.clone();
    binding.device = [nonce; 32];
    binding.identity = target.public_key();
    let signature = target.sign(&binding.signing_bytes().unwrap()).unwrap();
    let joining = SignedBinding { binding, signature };
    let transcript = sigil_protocol::link::Transcript {
        sponsor: sigil_crypto::link::fingerprint(&sponsor.binding).unwrap(),
        joining: sigil_crypto::link::fingerprint(&joining.binding).unwrap(),
        sponsor_challenge: [nonce; 32],
        joining_challenge: [nonce.wrapping_add(1); 32],
        provisioning_key: IdentityKey::generate().unwrap().public_key(),
        credential_commitment: Sha256::digest(token.as_bytes()).into(),
        created_at: 1000,
        expires_at: 1600,
    };
    let sponsor_signature = identity
        .sign(&sigil_crypto::link::signing_bytes(&transcript, true).unwrap())
        .unwrap();
    let joining_signature = target
        .sign(&sigil_crypto::link::signing_bytes(&transcript, false).unwrap())
        .unwrap();
    sigil_protocol::link::Proof {
        transcript,
        sponsor,
        joining,
        sponsor_signature,
        joining_signature,
    }
}
fn link_request(proof: &sigil_protocol::link::Proof) -> sigil_protocol::link::Authorization {
    sigil_protocol::link::Authorization {
        proof: hex(&proof.to_bytes().unwrap()),
    }
}
#[test]
fn linking_authorization_checks_signatures_scope_atomicity_exact_retries_and_restore() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    let (mut store, users) = setup(&path, 1000);
    let (alice, session, key) = &users[0];
    let target = random_secret().unwrap();
    let proof = link_proof(session, key, &target, 77);
    store
        .publish_device_binding(alice, request(&proof.sponsor), 1000)
        .unwrap();
    let encoded = proof.to_bytes().unwrap();
    assert_eq!(
        sigil_protocol::link::Proof::from_bytes(&encoded).unwrap(),
        proof
    );
    for length in 0..encoded.len() {
        assert!(sigil_protocol::link::Proof::from_bytes(&encoded[..length]).is_err());
    }
    assert!(sigil_protocol::link::Proof::from_bytes(&[encoded.as_slice(), &[0]].concat()).is_err());
    assert!(store
        .authorize_device_link(&users[1].0, link_request(&proof), 1000)
        .is_err());
    assert!(store
        .authorize_device_link(alice, link_request(&proof), 1600)
        .is_err());
    for n in 0..4 {
        let mut bad = proof.clone();
        match n {
            0 => bad.sponsor_signature[0] ^= 1,
            1 => bad.joining_signature[0] ^= 1,
            2 => bad.transcript.credential_commitment[0] ^= 1,
            _ => bad.joining.binding.account[0] ^= 1,
        }
        assert!(store
            .authorize_device_link(alice, link_request(&bad), 1000)
            .is_err());
    }
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_link BEFORE INSERT ON device_links BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(store
        .authorize_device_link(alice, link_request(&proof), 1000)
        .is_err());
    assert!(store.session(&target, 1000).is_err());
    db.execute_batch("DROP TRIGGER fail_link;").unwrap();
    // Historical revoked devices do not consume the active linking allowance.
    for n in 0..256 {
        db.execute("INSERT INTO devices(id,account_id,label,expires_at,revoked) VALUES(?1,?2,'Retired fixture',0,1)",(format!("{n:064x}"),&session.account_id)).unwrap();
    }
    // Exercise the previous schema without inventing device grants during migration.
    drop(store);
    db.execute_batch(
        "DROP TABLE private_group_nonces; DROP TABLE private_group_commits; DROP TABLE private_group_members; DROP TABLE private_groups; DROP TABLE group_credential_uids; DROP TABLE group_authority; DROP INDEX prekeys_available; DROP INDEX prekeys_remote_claim; ALTER TABLE prekeys DROP COLUMN remote_request; ALTER TABLE prekeys DROP COLUMN remote_device; ALTER TABLE prekeys DROP COLUMN remote_account; ALTER TABLE prekeys DROP COLUMN remote_server; CREATE INDEX prekeys_available ON prekeys(device_id,expires_at) WHERE bundle IS NOT NULL AND claimant IS NULL; DROP TABLE federation_outbox; DROP TABLE federation_revocations; DROP TABLE federation_senders; DROP TABLE federation_nonces; DROP TABLE federation_admission; DROP TABLE federation_usage; DROP TABLE federation_peers; DROP TABLE federation_configuration; DROP TABLE push_jobs; DROP TABLE push_channels; DROP TABLE push_configuration; DROP TABLE attachment_chunks; DROP TABLE attachments; DROP TABLE retained_storage; DROP INDEX mailbox_live_recipient; DROP INDEX prekeys_available; DROP INDEX devices_active_account; DROP TABLE cancelled_device_links; DROP TABLE device_links; PRAGMA user_version=10;",
    )
    .unwrap();
    drop(db);
    let mut store = Store::open(&path).unwrap();
    assert_eq!(store.session(alice, 1000).unwrap(), *session);
    let joined = store
        .authorize_device_link(alice, link_request(&proof), 1000)
        .unwrap();
    assert_eq!(store.session(&target, 1000).unwrap(), joined);
    assert_eq!(
        store
            .authorize_device_link(alice, link_request(&proof), 1700)
            .unwrap(),
        joined
    );
    assert_eq!(
        store
            .own_device_link(&target, 1000)
            .unwrap()
            .parse()
            .unwrap(),
        proof
    );
    let backup = dir.path().join("backup.db");
    store.backup(&backup).unwrap();
    let restored = dir.path().join("restored.db");
    Store::restore(&backup, &restored).unwrap();
    let mut restored = Store::open(&restored).unwrap();
    assert!(restored.session(&target, 1000).is_err());
    assert!(restored
        .authorize_device_link(alice, link_request(&proof), 1000)
        .is_err());
    store.revoke_device(alice, &joined.device_id, 1000).unwrap();
    assert!(store
        .authorize_device_link(alice, link_request(&proof), 1000)
        .is_err());
}
#[test]
fn cancellation_is_sponsor_scoped_and_wins_a_race_with_link_authorization() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    let (mut store, users) = setup(&path, 1000);
    let (alice, session, key) = &users[0];
    for n in 0..256 {
        store
            .cancel_device_link(alice, &format!("{n:064x}"), 1000)
            .unwrap();
    }
    let token = random_secret().unwrap();
    let proof = link_proof(session, key, &token, 79);
    store
        .publish_device_binding(alice, request(&proof.sponsor), 1000)
        .unwrap();
    store
        .cancel_device_link(&users[1].0, &hex(&proof.transcript.sponsor_challenge), 1000)
        .unwrap();
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let authorize = scope.spawn(|| {
            let mut db = Store::open(&path).unwrap();
            barrier.wait();
            db.authorize_device_link(alice, link_request(&proof), 1000)
        });
        let cancel = scope.spawn(|| {
            let mut db = Store::open(&path).unwrap();
            barrier.wait();
            db.cancel_device_link(alice, &hex(&proof.transcript.sponsor_challenge), 1000)
        });
        let result = authorize.join().unwrap();
        assert!(result.is_ok() || matches!(result, Err(StoreError::Unauthorized)));
        cancel.join().unwrap().unwrap();
    });
    assert!(store.session(&token, 1000).is_err());
    assert!(store
        .authorize_device_link(alice, link_request(&proof), 1000)
        .is_err());
    store
        .cancel_device_link(alice, &hex(&proof.transcript.sponsor_challenge), 1000)
        .unwrap();
    let proof = link_proof(session, key, &random_secret().unwrap(), 81);
    store
        .cancel_device_link(alice, &hex(&proof.transcript.sponsor_challenge), 1000)
        .unwrap();
    assert!(store
        .authorize_device_link(alice, link_request(&proof), 1000)
        .is_err());
}
