use sha2::{Digest, Sha256};
use sigil_crypto::{
    handshake::{initiate_session, Bundle, Receiver},
    IdentityKey,
};
use sigil_protocol::{
    accounts::{Enrollment, InviteRequest},
    prekeys::PublishPrekey,
    Configure,
};
use sigil_server::{
    auth::random_secret,
    store::{Store, StoreError},
};

const NOW: u64 = 1000;

#[test]
fn inventory_is_device_scoped_and_excludes_claimed_expired_and_cleared_bundles() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let mut store = configured(&path);
    let (alice, device) = enroll(&mut store, "alice", NOW);
    let (bob, _) = enroll(&mut store, "bob", NOW);
    let identity = IdentityKey::generate().unwrap();
    for _ in 0..3 {
        let receiver = Receiver::generate(&identity, true).unwrap();
        let (id, request) = publication(&receiver);
        store.publish_prekey(&alice, &id, request, NOW).unwrap();
    }
    assert_eq!(store.prekey_inventory(&alice, NOW).unwrap().available, 3);
    assert_eq!(store.prekey_inventory(&bob, NOW).unwrap().available, 0);
    store
        .claim_prekey(&alice, &device, &random_secret().unwrap(), NOW)
        .unwrap();
    assert_eq!(store.prekey_inventory(&alice, NOW).unwrap().available, 2);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute("UPDATE prekeys SET bundle=NULL WHERE id=(SELECT id FROM prekeys WHERE claimant IS NULL LIMIT 1)", []).unwrap();
    assert_eq!(store.prekey_inventory(&alice, NOW).unwrap().available, 1);
    assert_eq!(
        store
            .prekey_inventory(&alice, NOW + 3600)
            .unwrap()
            .available,
        0
    );
    assert!(matches!(
        store.prekey_inventory(&random_secret().unwrap(), NOW),
        Err(StoreError::Unauthorized)
    ));
    store.revoke_device(&alice, &device, NOW).unwrap();
    assert!(matches!(
        store.prekey_inventory(&alice, NOW),
        Err(StoreError::Unauthorized)
    ));
}

#[tokio::test]
async fn http_prekeys_require_device_auth_and_reject_browser_access() {
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let dir = tempfile::tempdir().unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut store = configured(&dir.path().join("sigil.db"));
    let (token, target) = enroll(&mut store, "alice", now);
    let receiver = Receiver::generate(&IdentityKey::generate().unwrap(), false).unwrap();
    let (id, request) = publication(&receiver);
    let admin =
        sigil_server::auth::AdminToken::load_or_create(&dir.path().join("admin.token")).unwrap();
    let app = sigil_server::router(store, admin);
    for (credential, origin, expected) in [
        (false, false, StatusCode::UNAUTHORIZED),
        (true, true, StatusCode::FORBIDDEN),
        (true, false, StatusCode::OK),
    ] {
        let mut request = Request::builder().uri("/client/v0/prekeys");
        if credential {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        if origin {
            request = request.header("origin", "https://example.invalid");
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        if expected == StatusCode::OK {
            let inventory: sigil_protocol::prekeys::PrekeyInventory =
                serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap())
                    .unwrap();
            assert_eq!(inventory.available, 0);
        }
    }
    let payload = serde_json::to_vec(&request).unwrap();
    let lifetime = request.expires_in_seconds as u64;
    let path = format!("/client/v0/prekeys/{id}");
    for (credential, origin, expected) in [
        (false, false, StatusCode::UNAUTHORIZED),
        (true, true, StatusCode::FORBIDDEN),
        (true, false, StatusCode::OK),
    ] {
        let mut request = Request::builder()
            .method("PUT")
            .uri(&path)
            .header("content-type", "application/json");
        if credential {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        if origin {
            request = request.header("origin", "https://example.invalid");
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::from(payload.clone())).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        if expected == StatusCode::OK {
            let receipt: sigil_protocol::prekeys::PublishedPrekey =
                serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap())
                    .unwrap();
            assert_eq!(receipt.prekey_id, id);
            assert!(receipt.expires_at >= now + lifetime);
        }
    }
    let claim_id = random_secret().unwrap();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/client/v0/devices/{target}/prekeys/claim"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(
                    serde_json::json!({"request_id":claim_id}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let result: sigil_protocol::prekeys::ClaimedPrekey =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    assert_eq!(result.bundle, request.bundle);
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn decode(text: &str) -> Vec<u8> {
    text.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
fn configured(path: &std::path::Path) -> Store {
    let mut store = Store::open(path).unwrap();
    store
        .configure(Configure {
            expected_revision: 0,
            settings: serde_json::from_str(r#"{"server_name":"chat.example"}"#).unwrap(),
        })
        .unwrap();
    store
}
fn enroll(store: &mut Store, name: &str, now: u64) -> (String, String) {
    let invite = store
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
                invitation: invite.secret,
                device_credential: token.clone(),
                device_label: "Synthetic device".into(),
            },
            now,
        )
        .unwrap();
    (token, session.device_id)
}
fn publication(receiver: &Receiver) -> (String, PublishPrekey) {
    let bytes = receiver.bundle().unwrap().to_bytes();
    (
        hex(&Sha256::digest(&bytes[139..1707])),
        PublishPrekey {
            bundle: hex(&bytes),
            expires_in_seconds: 3600,
        },
    )
}

#[test]
fn claimed_bundle_starts_a_real_crypto_session_and_survives_retry_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let mut store = configured(&path);
    let (alice_token, _) = enroll(&mut store, "alice", NOW);
    let (bob_token, bob_device) = enroll(&mut store, "bob", NOW);
    let sender = store.session(&alice_token, NOW).unwrap().device_id;
    store.allow_sender(&bob_token, &sender, NOW).unwrap();
    let bob = IdentityKey::generate().unwrap();
    let alice = IdentityKey::generate().unwrap();
    let mut receiver = Receiver::generate(&bob, true).unwrap();
    let (id, request) = publication(&receiver);
    let published = store.publish_prekey(&bob_token, &id, request, NOW).unwrap();
    assert_eq!(published.prekey_id, id);
    assert_eq!(published.expires_at, NOW + 3600);
    let claim = random_secret().unwrap();
    let first = store
        .claim_prekey(&alice_token, &bob_device, &claim, NOW)
        .unwrap();
    drop(store);
    let mut store = Store::open(&path).unwrap();
    assert_eq!(
        store
            .claim_prekey(&alice_token, &bob_device, &claim, NOW + 1)
            .unwrap(),
        first
    );
    let (_, request) = publication(&receiver);
    assert_eq!(
        store
            .publish_prekey(&bob_token, &id, request, NOW + 1)
            .unwrap(),
        published
    );
    assert!(matches!(
        store.claim_prekey(
            &alice_token,
            &bob_device,
            &random_secret().unwrap(),
            NOW + 1
        ),
        Err(StoreError::NotFound)
    ));
    let bundle = Bundle::from_bytes(&decode(&first.bundle), &bob.public_key()).unwrap();
    let (mut sending, initial) =
        initiate_session(&alice, &bob.public_key(), &bundle, b"hello").unwrap();
    let (mut receiving, hello) = receiver
        .accept_session(&bob, &alice.public_key(), &initial)
        .unwrap();
    assert_eq!(hello, b"hello");
    assert_eq!(
        receiving
            .receive(&sending.send(b"synthetic").unwrap())
            .unwrap(),
        b"synthetic"
    );
}

#[test]
fn identity_is_pinned_and_keys_cannot_be_moved_between_devices() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = configured(&dir.path().join("sigil.db"));
    let (token, _) = enroll(&mut store, "alice", NOW);
    let (other, _) = enroll(&mut store, "bob", NOW);
    let first = Receiver::generate(&IdentityKey::generate().unwrap(), false).unwrap();
    let (id, request) = publication(&first);
    store.publish_prekey(&token, &id, request, NOW).unwrap();
    let (_, request) = publication(&first);
    assert!(matches!(
        store.publish_prekey(&other, &id, request, NOW),
        Err(StoreError::AlreadyExists)
    ));
    let second = Receiver::generate(&IdentityKey::generate().unwrap(), false).unwrap();
    let (id, request) = publication(&second);
    assert!(matches!(
        store.publish_prekey(&token, &id, request, NOW),
        Err(StoreError::Invalid(_))
    ));
    let (_, request) = publication(&second);
    assert!(store
        .publish_prekey(&other, &random_secret().unwrap(), request, NOW)
        .is_err());
}

#[test]
fn competing_connections_assign_a_bundle_only_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let mut store = configured(&path);
    let (one, _) = enroll(&mut store, "alice", NOW);
    let (two, _) = enroll(&mut store, "charlie", NOW);
    let (owner, target) = enroll(&mut store, "bob", NOW);
    for token in [&one, &two] {
        let sender = store.session(token, NOW).unwrap().device_id;
        store.allow_sender(&owner, &sender, NOW).unwrap();
    }
    let receiver = Receiver::generate(&IdentityKey::generate().unwrap(), true).unwrap();
    let (id, request) = publication(&receiver);
    store.publish_prekey(&owner, &id, request, NOW).unwrap();
    let second = Store::open(&path).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let threads: Vec<_> = [(store, one), (second, two)]
        .into_iter()
        .map(|(mut store, token)| {
            let barrier = barrier.clone();
            let target = target.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.claim_prekey(&token, &target, &random_secret().unwrap(), NOW)
            })
        })
        .collect();
    let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Err(StoreError::NotFound)))
            .count(),
        1
    );
}

#[test]
fn expired_claim_ids_do_not_consume_replacement_keys() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = configured(&dir.path().join("sigil.db"));
    let (token, target) = enroll(&mut store, "alice", NOW);
    let identity = IdentityKey::generate().unwrap();
    let receiver = Receiver::generate(&identity, false).unwrap();
    let (id, request) = publication(&receiver);
    store.publish_prekey(&token, &id, request, NOW).unwrap();
    let claim = random_secret().unwrap();
    store.claim_prekey(&token, &target, &claim, NOW).unwrap();
    let replacement = Receiver::generate(&identity, false).unwrap();
    let (id, request) = publication(&replacement);
    store
        .publish_prekey(&token, &id, request, NOW + 3600)
        .unwrap();
    assert_eq!(store.expire_batch(NOW + 3600).unwrap(), 1);
    assert!(matches!(
        store.claim_prekey(&token, &target, &claim, NOW + 3600),
        Err(StoreError::NotFound)
    ));
    assert_eq!(
        store
            .claim_prekey(&token, &target, &random_secret().unwrap(), NOW + 3600)
            .unwrap()
            .prekey_id,
        id
    );
}

#[test]
fn schema_two_migration_and_per_device_inventory_limits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let mut store = configured(&path);
    let (token, target) = enroll(&mut store, "alice", NOW);
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch(
        "DROP TABLE profile_shares; DROP TABLE profile_photos; DROP TABLE contact_requests; DROP TABLE contact_request_policy; DROP TABLE account_passwords; DROP TABLE password_policy; DROP TABLE web_oidc; DROP TABLE web_sessions; DROP TABLE oidc_fallback_ack; DROP TABLE oidc_transition; DROP TABLE account_profiles; DROP TABLE web_owner; DROP TABLE deleted_accounts; DROP TABLE operation_uploads; DROP TABLE operations; DROP TABLE operation_configuration; DROP TABLE oidc_grants; DROP TABLE oidc_bindings; DROP TABLE oidc_flows; DROP TABLE oidc_configuration; DROP TABLE registration_usage; DROP TABLE account_policy; DROP TABLE admin_policy; DROP TABLE call_connections; DROP TABLE calls; DROP TABLE call_configuration; DROP TABLE service_budgets; DROP TABLE service_configuration; DROP TABLE map_configuration; DROP TABLE private_group_invitations; DROP TABLE private_group_proposals; DROP TABLE private_group_nonces; DROP TABLE private_group_commits; DROP TABLE private_group_members; DROP TABLE private_groups; DROP TABLE group_credential_uids; DROP TABLE group_authority; DROP INDEX prekeys_available; DROP INDEX prekeys_remote_claim; ALTER TABLE prekeys DROP COLUMN remote_request; ALTER TABLE prekeys DROP COLUMN remote_device; ALTER TABLE prekeys DROP COLUMN remote_account; ALTER TABLE prekeys DROP COLUMN remote_server; CREATE INDEX prekeys_available ON prekeys(device_id,expires_at) WHERE bundle IS NOT NULL AND claimant IS NULL; DROP TABLE federation_outbox; DROP TABLE federation_revocations; DROP TABLE federation_senders; DROP TABLE federation_nonces; DROP TABLE federation_admission; DROP TABLE federation_usage; DROP TABLE federation_peers; DROP TABLE federation_configuration; DROP TABLE push_jobs; DROP TABLE push_channels; DROP TABLE push_configuration; DROP TABLE attachment_chunks; DROP TABLE attachments; DROP TABLE retained_storage; DROP INDEX mailbox_live_recipient; DROP INDEX prekeys_available; DROP INDEX devices_active_account; DROP TABLE cancelled_device_links; DROP TABLE device_links; DROP INDEX devices_account_id; DROP TABLE device_bindings; DROP TABLE recovery_heads; DROP TABLE recovery_objects; DROP TABLE contact_invitations; DROP TABLE allowed_senders; DROP TABLE mailbox; DROP TABLE prekeys; DROP TABLE encryption_identities; DROP INDEX IF EXISTS prekeys_expiry; DROP INDEX IF EXISTS invitations_expiry; DROP TABLE IF EXISTS push_android; PRAGMA user_version=2;",
    )
    .unwrap();
    let mut store = Store::open(&path).unwrap();
    let identity = IdentityKey::generate().unwrap();
    let receiver = Receiver::generate(&identity, false).unwrap();
    let (id, request) = publication(&receiver);
    db.execute("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<64) INSERT INTO prekeys(id,device_id,bundle,bundle_hash,expires_at) SELECT printf('%064x',x),?1,x'01',zeroblob(32),?2 FROM n", (&target, (NOW+3600) as i64)).unwrap();
    assert!(matches!(
        store.publish_prekey(&token, &id, request, NOW),
        Err(StoreError::Busy)
    ));
    let (_, request) = publication(&receiver);
    store
        .publish_prekey(&token, &id, request, NOW + 3600)
        .unwrap();
    assert_eq!(store.expire_batch(NOW + 3600).unwrap(), 64);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM prekeys WHERE bundle IS NOT NULL",
            [],
            |r| r.get::<_, u32>(0)
        )
        .unwrap(),
        1
    );
    db.execute("WITH RECURSIVE n(x) AS (VALUES(65) UNION ALL SELECT x+1 FROM n WHERE x<4095) INSERT INTO prekeys(id,device_id,bundle_hash,expires_at) SELECT printf('%064x',x),?1,zeroblob(32),0 FROM n", [&target]).unwrap();
    let next = Receiver::generate(&identity, false).unwrap();
    let (id, request) = publication(&next);
    store
        .publish_prekey(&token, &id, request, NOW + 3600)
        .unwrap();
    assert_eq!(
        store
            .prekey_inventory(&token, NOW + 3600)
            .unwrap()
            .available,
        2
    );
}

#[test]
fn revocation_and_restore_prevent_old_key_distribution() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let mut store = configured(&path);
    let (reader, _) = enroll(&mut store, "alice", NOW);
    let (owner, target) = enroll(&mut store, "bob", NOW);
    let receiver = Receiver::generate(&IdentityKey::generate().unwrap(), false).unwrap();
    let (id, request) = publication(&receiver);
    store.publish_prekey(&owner, &id, request, NOW).unwrap();
    let backup = dir.path().join("backup.db");
    store.backup(&backup).unwrap();
    store.revoke_device(&owner, &target, NOW).unwrap();
    assert!(matches!(
        store.claim_prekey(&reader, &target, &random_secret().unwrap(), NOW),
        Err(StoreError::NotFound)
    ));
    let restored = dir.path().join("restored.db");
    Store::restore(&backup, &restored).unwrap();
    let mut restored_store = Store::open(&restored).unwrap();
    assert!(matches!(
        restored_store.claim_prekey(&reader, &target, &random_secret().unwrap(), NOW),
        Err(StoreError::Unauthorized)
    ));
    let db = rusqlite::Connection::open(&restored).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM prekeys WHERE bundle IS NOT NULL",
            [],
            |r| r.get::<_, u32>(0)
        )
        .unwrap(),
        0
    );
}
