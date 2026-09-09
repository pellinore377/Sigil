use sigil_protocol::{
    accounts::{Enrollment, InviteRequest},
    mailbox::Submit,
    Configure,
};
use sigil_server::{
    auth::random_secret,
    store::{Store, StoreError},
};
const NOW: u64 = 1000;
fn setup(path: &std::path::Path, now: u64) -> (Store, String, String, String) {
    let mut store = Store::open(path).unwrap();
    store
        .configure(Configure {
            expected_revision: 0,
            settings: serde_json::from_str(r#"{"server_name":"chat.example"}"#).unwrap(),
        })
        .unwrap();
    let mut tokens = Vec::new();
    let mut target = String::new();
    for name in ["alice", "bob"] {
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
        target = store
            .enroll(
                Enrollment {
                    invitation: invite.secret,
                    device_credential: token.clone(),
                    device_label: "Synthetic".into(),
                },
                now,
            )
            .unwrap()
            .device_id;
        tokens.push(token);
    }
    let sender = store.session(&tokens[0], now).unwrap().device_id;
    store.allow_sender(&tokens[1], &sender, now).unwrap();
    (store, tokens.remove(0), tokens.remove(0), target)
}
fn message(target: &str, id: &str, expiry: u64) -> Submit {
    Submit {
        recipient_device: target.into(),
        message_id: id.into(),
        payload: "ab".repeat(32),
        expires_at: expiry,
    }
}

#[test]
fn mailbox_turnover_crosses_4096_and_retains_exact_receipts_after_upgrade() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("turnover.db");
    let (mut store, alice, bob, target) = setup(&path, NOW);
    let first = store
        .submit_message(
            &alice,
            message(&target, &format!("{:064x}", 0), NOW + 60),
            NOW,
        )
        .unwrap();
    store
        .acknowledge_message(&bob, first.sequence, NOW)
        .unwrap();
    for n in 1..4097 {
        let receipt = store
            .submit_message(
                &alice,
                message(&target, &format!("{n:064x}"), NOW + 60),
                NOW,
            )
            .unwrap();
        store
            .acknowledge_message(&bob, receipt.sequence, NOW)
            .unwrap();
    }
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("DROP TABLE contact_requests; DROP TABLE contact_request_policy; DROP TABLE account_passwords; DROP TABLE password_policy; DROP TABLE web_oidc; DROP TABLE web_sessions; DROP TABLE oidc_fallback_ack; DROP TABLE oidc_transition; DROP TABLE account_profiles; DROP TABLE web_owner; DROP TABLE deleted_accounts; DROP TABLE operation_uploads; DROP TABLE operations; DROP TABLE operation_configuration; DROP TABLE oidc_grants; DROP TABLE oidc_bindings; DROP TABLE oidc_flows; DROP TABLE oidc_configuration; DROP TABLE registration_usage; DROP TABLE account_policy; DROP TABLE admin_policy; DROP TABLE call_connections; DROP TABLE calls; DROP TABLE call_configuration; DROP TABLE service_budgets; DROP TABLE service_configuration; DROP TABLE map_configuration; DROP TABLE private_group_invitations; DROP TABLE private_group_proposals; DROP TABLE private_group_nonces; DROP TABLE private_group_commits; DROP TABLE private_group_members; DROP TABLE private_groups; DROP TABLE group_credential_uids; DROP TABLE group_authority; DROP INDEX prekeys_available; DROP INDEX prekeys_remote_claim; ALTER TABLE prekeys DROP COLUMN remote_request; ALTER TABLE prekeys DROP COLUMN remote_device; ALTER TABLE prekeys DROP COLUMN remote_account; ALTER TABLE prekeys DROP COLUMN remote_server; CREATE INDEX prekeys_available ON prekeys(device_id,expires_at) WHERE bundle IS NOT NULL AND claimant IS NULL; DROP TABLE federation_outbox; DROP TABLE federation_revocations; DROP TABLE federation_senders; DROP TABLE federation_nonces; DROP TABLE federation_admission; DROP TABLE federation_usage; DROP TABLE federation_peers; DROP TABLE federation_configuration; DROP TABLE push_jobs; DROP TABLE push_channels; DROP TABLE push_configuration; DROP TABLE attachment_chunks; DROP TABLE attachments; DROP TABLE retained_storage; DROP INDEX mailbox_live_recipient; DROP INDEX prekeys_available; DROP INDEX devices_active_account; PRAGMA user_version=11;").unwrap();
    let mut store = Store::open(&path).unwrap();
    assert_eq!(
        store
            .submit_message(
                &alice,
                message(&target, &format!("{:064x}", 0), NOW + 60),
                NOW
            )
            .unwrap(),
        first
    );
    assert!(store.mailbox(&bob, NOW).unwrap().is_empty());
    let receipt = store
        .submit_message(
            &alice,
            message(&target, &format!("{:064x}", 4097), NOW + 60),
            NOW,
        )
        .unwrap();
    assert_eq!(
        store.mailbox(&bob, NOW).unwrap()[0].sequence,
        receipt.sequence
    );
    let before: i64 = db
        .query_row("SELECT sum(bytes) FROM retained_storage", [], |r| r.get(0))
        .unwrap();
    store.expire_batch(NOW + 60).unwrap();
    assert_eq!(
        db.query_row("SELECT sum(bytes) FROM retained_storage", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        before
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM mailbox", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        4098
    );
}

#[test]
fn encrypted_delivery_survives_restart_and_acknowledgement_is_retry_safe() {
    use sigil_crypto::{
        ratchet::{Packet, Session},
        DhKey, Secret32,
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let (mut store, alice, bob, target) = setup(&path, NOW);
    let dh = DhKey::generate().unwrap();
    let mut sending =
        Session::initiator(Secret32::from_bytes([7; 32]), dh.public_key(), [8; 32]).unwrap();
    let mut receiving = Session::responder(Secret32::from_bytes([7; 32]), dh, [8; 32]);
    let packet = sending
        .send(b"Synthetic private message")
        .unwrap()
        .to_bytes();
    let payload: String = packet.iter().map(|b| format!("{b:02x}")).collect();
    let id = random_secret().unwrap();
    let submit = || Submit {
        recipient_device: target.clone(),
        message_id: id.clone(),
        payload: payload.clone(),
        expires_at: NOW + 60,
    };
    let receipt = store.submit_message(&alice, submit(), NOW).unwrap();
    drop(store);
    let mut store = Store::open(&path).unwrap();
    assert!(store.mailbox(&alice, NOW).unwrap().is_empty());
    assert!(matches!(
        store.acknowledge_message(&alice, receipt.sequence, NOW),
        Err(StoreError::NotFound)
    ));
    let delivery = store.mailbox(&bob, NOW).unwrap().remove(0);
    assert_eq!(store.mailbox(&bob, NOW).unwrap().len(), 1);
    let bytes: Vec<_> = delivery
        .payload
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect();
    assert_eq!(
        receiving
            .receive(&Packet::from_bytes(&bytes).unwrap())
            .unwrap(),
        b"Synthetic private message"
    );
    store
        .acknowledge_message(&bob, receipt.sequence, NOW)
        .unwrap();
    store
        .acknowledge_message(&bob, receipt.sequence, NOW)
        .unwrap();
    assert_eq!(
        store.submit_message(&alice, submit(), NOW).unwrap(),
        receipt
    );
    assert!(store.mailbox(&bob, NOW).unwrap().is_empty());
    let mut altered = submit();
    altered.payload = "cd".repeat(32);
    assert!(matches!(
        store.submit_message(&alice, altered, NOW),
        Err(StoreError::AlreadyExists)
    ));
}

#[test]
fn polling_is_bounded_expiry_clears_payloads_and_revocation_blocks_access() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, alice, bob, target) = setup(&dir.path().join("sigil.db"), NOW);
    for _ in 0..17 {
        store
            .submit_message(
                &alice,
                message(&target, &random_secret().unwrap(), NOW + 60),
                NOW,
            )
            .unwrap();
    }
    let batch = store.mailbox(&bob, NOW).unwrap();
    assert_eq!(batch.len(), 16);
    let later = store.mailbox_after(&bob, batch[15].sequence, NOW).unwrap();
    assert_eq!(later.len(), 1);
    assert!(later[0].sequence > batch[15].sequence);
    assert_eq!(store.mailbox(&bob, NOW).unwrap().len(), 16);
    assert!(store
        .mailbox_after(&bob, later[0].sequence, NOW)
        .unwrap()
        .is_empty());
    assert!(store.mailbox_after(&alice, 0, NOW).unwrap().is_empty());
    assert!(store.mailbox_after(&bob, -1, NOW).is_err());
    for value in batch {
        store
            .acknowledge_message(&bob, value.sequence, NOW)
            .unwrap();
    }
    assert_eq!(store.mailbox(&bob, NOW).unwrap().len(), 1);
    assert!(store.mailbox(&bob, NOW + 60).unwrap().is_empty());
    store.revoke_device(&bob, &target, NOW).unwrap();
    assert!(matches!(
        store.mailbox(&bob, NOW),
        Err(StoreError::Unauthorized)
    ));
    assert!(matches!(
        store.submit_message(
            &alice,
            message(&target, &random_secret().unwrap(), NOW + 60),
            NOW
        ),
        Err(StoreError::NotFound)
    ));
}

#[test]
fn quota_failure_preserves_inventory_and_ack_frees_space() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, alice, bob, target) = setup(&dir.path().join("sigil.db"), NOW);
    store.configure(Configure {expected_revision:1,settings:serde_json::from_str(r#"{"server_name":"chat.example","default_quota_bytes":1048576,"max_attachment_bytes":1048576}"#).unwrap()}).unwrap();
    let make = || Submit {
        payload: "ab".repeat(67230),
        ..message(&target, &random_secret().unwrap(), NOW + 60)
    };
    for _ in 0..7 {
        store.submit_message(&alice, make(), NOW).unwrap();
    }
    assert!(matches!(
        store.submit_message(&alice, make(), NOW),
        Err(StoreError::Busy)
    ));
    let batch = store.mailbox(&bob, NOW).unwrap();
    assert_eq!(batch.len(), 7);
    store
        .acknowledge_message(&bob, batch[0].sequence, NOW)
        .unwrap();
    store.submit_message(&alice, make(), NOW).unwrap();
}

#[test]
fn concurrent_duplicate_submissions_have_one_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let (store, alice, bob, target) = setup(&path, NOW);
    let other = Store::open(&path).unwrap();
    let id = random_secret().unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let threads: Vec<_> = [store, other]
        .into_iter()
        .map(|mut store| {
            let barrier = barrier.clone();
            let alice = alice.clone();
            let target = target.clone();
            let id = id.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store
                    .submit_message(&alice, message(&target, &id, NOW + 60), NOW)
                    .unwrap()
            })
        })
        .collect();
    let receipts: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(receipts[0], receipts[1]);
    assert_eq!(
        Store::open(&path)
            .unwrap()
            .mailbox(&bob, NOW)
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn http_accepts_large_ciphertext_and_requires_device_auth() {
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
    let (store, alice, bob, target) = setup(&dir.path().join("sigil.db"), now);
    let admin =
        sigil_server::auth::AdminToken::load_or_create(&dir.path().join("admin.token")).unwrap();
    let app = sigil_server::router(store, admin);
    let payload = serde_json::to_vec(&Submit {
        payload: "ab".repeat(67230),
        ..message(&target, &random_secret().unwrap(), now + 60)
    })
    .unwrap();
    for (auth, origin, status) in [
        (false, false, StatusCode::UNAUTHORIZED),
        (true, true, StatusCode::FORBIDDEN),
        (true, false, StatusCode::ACCEPTED),
    ] {
        let mut req = Request::builder()
            .method("POST")
            .uri("/client/v0/messages")
            .header("content-type", "application/json");
        if auth {
            req = req.header("authorization", format!("Bearer {alice}"));
        }
        if origin {
            req = req.header("origin", "https://example.invalid");
        }
        assert_eq!(
            app.clone()
                .oneshot(req.body(Body::from(payload.clone())).unwrap())
                .await
                .unwrap()
                .status(),
            status
        );
    }
    for query in [
        "after=-1",
        "after=9223372036854775808",
        "after=1&after=2",
        "after=",
        "other=1",
        "after=%31",
    ] {
        assert_eq!(
            app.clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/client/v0/mailbox?{query}"))
                        .header("authorization", format!("Bearer {bob}"))
                        .body(Body::empty())
                        .unwrap()
                )
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    let response = app
        .oneshot(
            Request::builder()
                .uri("/client/v0/mailbox")
                .header("authorization", format!("Bearer {bob}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let values: Vec<sigil_protocol::mailbox::Delivery> =
        serde_json::from_slice(&to_bytes(response.into_body(), 200000).await.unwrap()).unwrap();
    assert_eq!(values.len(), 1);
}

#[test]
fn pending_limits_allow_retained_history_and_restore_clears_payloads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let (mut store, alice, bob, target) = setup(&path, NOW);
    let sender = store.session(&alice, NOW).unwrap().device_id;
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<64) INSERT INTO mailbox(sender,message_id,recipient,payload,payload_hash,expires_at) SELECT ?1,printf('%064x',x),?2,'abab',zeroblob(32),?3 FROM n", (&sender,&target,(NOW+60) as i64)).unwrap();
    assert!(matches!(
        store.submit_message(
            &alice,
            message(&target, &random_secret().unwrap(), NOW + 60),
            NOW
        ),
        Err(StoreError::Busy)
    ));
    let first = store.mailbox(&bob, NOW).unwrap().remove(0);
    store
        .acknowledge_message(&bob, first.sequence, NOW)
        .unwrap();
    store
        .submit_message(
            &alice,
            message(&target, &random_secret().unwrap(), NOW + 60),
            NOW,
        )
        .unwrap();
    db.execute("UPDATE mailbox SET payload=NULL", []).unwrap();
    db.execute("WITH RECURSIVE n(x) AS (VALUES(65) UNION ALL SELECT x+1 FROM n WHERE x<4095) INSERT INTO mailbox(sender,message_id,recipient,payload_hash,expires_at) SELECT ?1,printf('%064x',x),?2,zeroblob(32),0 FROM n", (&sender,&target)).unwrap();
    store
        .submit_message(
            &alice,
            message(&target, &random_secret().unwrap(), NOW + 60),
            NOW,
        )
        .unwrap();
    db.execute(
        "UPDATE mailbox SET payload='abab' WHERE sequence=?1",
        [first.sequence],
    )
    .unwrap();
    let backup = dir.path().join("backup.db");
    store.backup(&backup).unwrap();
    let restored = dir.path().join("restored.db");
    Store::restore(&backup, &restored).unwrap();
    let db = rusqlite::Connection::open(restored).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM mailbox WHERE payload IS NOT NULL",
            [],
            |r| r.get::<_, u32>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn schema_three_migration_creates_mailbox_without_resetting_accounts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let (store, alice, bob, target) = setup(&path, NOW);
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("DROP TABLE contact_requests; DROP TABLE contact_request_policy; DROP TABLE account_passwords; DROP TABLE password_policy; DROP TABLE web_oidc; DROP TABLE web_sessions; DROP TABLE oidc_fallback_ack; DROP TABLE oidc_transition; DROP TABLE account_profiles; DROP TABLE web_owner; DROP TABLE deleted_accounts; DROP TABLE operation_uploads; DROP TABLE operations; DROP TABLE operation_configuration; DROP TABLE oidc_grants; DROP TABLE oidc_bindings; DROP TABLE oidc_flows; DROP TABLE oidc_configuration; DROP TABLE registration_usage; DROP TABLE account_policy; DROP TABLE admin_policy; DROP TABLE call_connections; DROP TABLE calls; DROP TABLE call_configuration; DROP TABLE service_budgets; DROP TABLE service_configuration; DROP TABLE map_configuration; DROP TABLE private_group_invitations; DROP TABLE private_group_proposals; DROP TABLE private_group_nonces; DROP TABLE private_group_commits; DROP TABLE private_group_members; DROP TABLE private_groups; DROP TABLE group_credential_uids; DROP TABLE group_authority; DROP INDEX prekeys_available; DROP INDEX prekeys_remote_claim; ALTER TABLE prekeys DROP COLUMN remote_request; ALTER TABLE prekeys DROP COLUMN remote_device; ALTER TABLE prekeys DROP COLUMN remote_account; ALTER TABLE prekeys DROP COLUMN remote_server; CREATE INDEX prekeys_available ON prekeys(device_id,expires_at) WHERE bundle IS NOT NULL AND claimant IS NULL; DROP TABLE federation_outbox; DROP TABLE federation_revocations; DROP TABLE federation_senders; DROP TABLE federation_nonces; DROP TABLE federation_admission; DROP TABLE federation_usage; DROP TABLE federation_peers; DROP TABLE federation_configuration; DROP TABLE push_jobs; DROP TABLE push_channels; DROP TABLE push_configuration; DROP TABLE attachment_chunks; DROP TABLE attachments; DROP TABLE retained_storage; DROP INDEX mailbox_live_recipient; DROP INDEX prekeys_available; DROP INDEX devices_active_account; DROP TABLE cancelled_device_links; DROP TABLE device_links; DROP INDEX devices_account_id; DROP TABLE device_bindings; DROP TABLE recovery_heads; DROP TABLE recovery_objects; DROP TABLE contact_invitations; DROP TABLE allowed_senders; DROP TABLE mailbox; DROP INDEX IF EXISTS prekeys_expiry; DROP INDEX IF EXISTS invitations_expiry; PRAGMA user_version=3;")
        .unwrap();
    drop(db);
    let mut store = Store::open(&path).unwrap();
    let sender = store.session(&alice, NOW).unwrap().device_id;
    store.allow_sender(&bob, &sender, NOW).unwrap();
    store
        .submit_message(
            &alice,
            message(&target, &random_secret().unwrap(), NOW + 60),
            NOW,
        )
        .unwrap();
}

#[test]
fn recipient_permissions_gate_prekeys_delivery_and_retries_after_removal() {
    use sha2::{Digest, Sha256};
    use sigil_crypto::{handshake::Receiver, IdentityKey};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let (mut store, alice, bob, target) = setup(&path, NOW);
    let sender = store.session(&alice, NOW).unwrap().device_id;
    let receiver = Receiver::generate(&IdentityKey::generate().unwrap(), false).unwrap();
    let bundle = receiver.bundle().unwrap().to_bytes();
    let hex = |v: &[u8]| v.iter().map(|b| format!("{b:02x}")).collect::<String>();
    store
        .publish_prekey(
            &bob,
            &hex(&Sha256::digest(&bundle[139..1707])),
            sigil_protocol::prekeys::PublishPrekey {
                bundle: hex(&bundle),
                expires_in_seconds: 3600,
            },
            NOW,
        )
        .unwrap();
    store.remove_sender(&bob, &sender, NOW).unwrap();
    let id = random_secret().unwrap();
    let claim = random_secret().unwrap();
    assert!(matches!(
        store.submit_message(&alice, message(&target, &id, NOW + 60), NOW),
        Err(StoreError::Forbidden)
    ));
    assert!(matches!(
        store.claim_prekey(&alice, &target, &claim, NOW),
        Err(StoreError::Forbidden)
    ));
    store.allow_sender(&alice, &target, NOW).unwrap(); // Wrong direction does not authorize Alice to Bob.
    assert!(matches!(
        store.submit_message(&alice, message(&target, &id, NOW + 60), NOW),
        Err(StoreError::Forbidden)
    ));
    store.allow_sender(&bob, &sender, NOW).unwrap();
    store.allow_sender(&bob, &sender, NOW).unwrap();
    let receipt = store
        .submit_message(&alice, message(&target, &id, NOW + 60), NOW)
        .unwrap();
    store.claim_prekey(&alice, &target, &claim, NOW).unwrap();
    drop(store);
    let mut store = Store::open(&path).unwrap();
    assert_eq!(
        store.allowed_senders(&bob, NOW).unwrap(),
        vec![sender.clone()]
    );
    store.remove_sender(&bob, &sender, NOW).unwrap();
    store.remove_sender(&bob, &sender, NOW).unwrap();
    assert!(store.mailbox(&bob, NOW).unwrap().is_empty());
    assert!(matches!(
        store.claim_prekey(&alice, &target, &claim, NOW),
        Err(StoreError::Forbidden)
    ));
    store.allow_sender(&bob, &sender, NOW).unwrap();
    assert_eq!(
        store
            .submit_message(&alice, message(&target, &id, NOW + 60), NOW)
            .unwrap(),
        receipt
    );
    assert!(store.mailbox(&bob, NOW).unwrap().is_empty());
    assert!(store.remove_sender(&bob, &target, NOW).is_err());
}

#[test]
fn migration_clears_legacy_unapproved_external_payloads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let (mut store, alice, bob, target) = setup(&path, NOW);
    store
        .submit_message(
            &alice,
            message(&target, &random_secret().unwrap(), NOW + 60),
            NOW,
        )
        .unwrap();
    store
        .submit_message(
            &bob,
            message(&target, &random_secret().unwrap(), NOW + 60),
            NOW,
        )
        .unwrap();
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch(
        "DROP TABLE contact_requests; DROP TABLE contact_request_policy; DROP TABLE account_passwords; DROP TABLE password_policy; DROP TABLE web_oidc; DROP TABLE web_sessions; DROP TABLE oidc_fallback_ack; DROP TABLE oidc_transition; DROP TABLE account_profiles; DROP TABLE web_owner; DROP TABLE deleted_accounts; DROP TABLE operation_uploads; DROP TABLE operations; DROP TABLE operation_configuration; DROP TABLE oidc_grants; DROP TABLE oidc_bindings; DROP TABLE oidc_flows; DROP TABLE oidc_configuration; DROP TABLE registration_usage; DROP TABLE account_policy; DROP TABLE admin_policy; DROP TABLE call_connections; DROP TABLE calls; DROP TABLE call_configuration; DROP TABLE service_budgets; DROP TABLE service_configuration; DROP TABLE map_configuration; DROP TABLE private_group_invitations; DROP TABLE private_group_proposals; DROP TABLE private_group_nonces; DROP TABLE private_group_commits; DROP TABLE private_group_members; DROP TABLE private_groups; DROP TABLE group_credential_uids; DROP TABLE group_authority; DROP INDEX prekeys_available; DROP INDEX prekeys_remote_claim; ALTER TABLE prekeys DROP COLUMN remote_request; ALTER TABLE prekeys DROP COLUMN remote_device; ALTER TABLE prekeys DROP COLUMN remote_account; ALTER TABLE prekeys DROP COLUMN remote_server; CREATE INDEX prekeys_available ON prekeys(device_id,expires_at) WHERE bundle IS NOT NULL AND claimant IS NULL; DROP TABLE federation_outbox; DROP TABLE federation_revocations; DROP TABLE federation_senders; DROP TABLE federation_nonces; DROP TABLE federation_admission; DROP TABLE federation_usage; DROP TABLE federation_peers; DROP TABLE federation_configuration; DROP TABLE push_jobs; DROP TABLE push_channels; DROP TABLE push_configuration; DROP TABLE attachment_chunks; DROP TABLE attachments; DROP TABLE retained_storage; DROP INDEX mailbox_live_recipient; DROP INDEX prekeys_available; DROP INDEX devices_active_account; DROP TABLE cancelled_device_links; DROP TABLE device_links; DROP INDEX devices_account_id; DROP TABLE device_bindings; DROP TABLE recovery_heads; DROP TABLE recovery_objects; DROP TABLE contact_invitations; DROP TABLE allowed_senders; DROP INDEX IF EXISTS prekeys_expiry; DROP INDEX IF EXISTS invitations_expiry; PRAGMA user_version=4;",
    )
    .unwrap();
    drop(db);
    let mut store = Store::open(&path).unwrap();
    let values = store.mailbox(&bob, NOW).unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].sender_device, target);
    assert!(store.allowed_senders(&bob, NOW).unwrap().is_empty());
}

#[tokio::test]
async fn http_permissions_are_recipient_owned_and_native_only() {
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
    let (store, alice, bob, _) = setup(&dir.path().join("sigil.db"), now);
    let sender = store.session(&alice, now).unwrap().device_id;
    let admin =
        sigil_server::auth::AdminToken::load_or_create(&dir.path().join("admin.token")).unwrap();
    let app = sigil_server::router(store, admin);
    for (token, origin, status) in [
        (None, false, StatusCode::UNAUTHORIZED),
        (Some(&bob), true, StatusCode::FORBIDDEN),
        (Some(&bob), false, StatusCode::NO_CONTENT),
    ] {
        let mut req = Request::builder()
            .method("DELETE")
            .uri(format!("/client/v0/mailbox/senders/{sender}"));
        if let Some(token) = token {
            req = req.header("authorization", format!("Bearer {token}"));
        }
        if origin {
            req = req.header("origin", "https://example.invalid");
        }
        assert_eq!(
            app.clone()
                .oneshot(req.body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
            status
        );
    }
}

#[test]
fn permission_removal_serializes_with_submission() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let (mut store, alice, bob, target) = setup(&path, NOW);
    let sender = store.session(&alice, NOW).unwrap().device_id;
    let mut other = Store::open(&path).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let waiting = barrier.clone();
    let recipient = target.clone();
    let sending = std::thread::spawn(move || {
        waiting.wait();
        store.submit_message(
            &alice,
            message(&recipient, &random_secret().unwrap(), NOW + 60),
            NOW,
        )
    });
    barrier.wait();
    other.remove_sender(&bob, &sender, NOW).unwrap();
    let result = sending.join().unwrap();
    assert!(result.is_ok() || matches!(result, Err(StoreError::Forbidden)));
    assert!(other.mailbox(&bob, NOW).unwrap().is_empty());
}

#[test]
fn contact_invitation_bootstraps_mutual_delivery_and_retries_without_restoring_grants() {
    use sigil_protocol::contacts::CreateContactInvite;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let (mut store, alice, bob, target) = setup(&path, NOW);
    let sender = store.session(&alice, NOW).unwrap().device_id;
    store.remove_sender(&bob, &sender, NOW).unwrap();
    let secret = random_secret().unwrap();
    let request = || CreateContactInvite {
        secret: secret.clone(),
        expires_at: NOW + 60,
    };
    let created = store.create_contact_invite(&bob, request(), NOW).unwrap();
    assert_ne!(created.id, secret);
    assert_eq!(
        store.create_contact_invite(&bob, request(), NOW).unwrap(),
        created
    );
    let peer = store.redeem_contact_invite(&alice, &secret, NOW).unwrap();
    assert_eq!(peer.device_id, target);
    drop(store);
    let mut store = Store::open(&path).unwrap();
    assert_eq!(
        store.redeem_contact_invite(&alice, &secret, NOW).unwrap(),
        peer
    );
    store
        .submit_message(
            &alice,
            message(&target, &random_secret().unwrap(), NOW + 60),
            NOW,
        )
        .unwrap();
    store
        .submit_message(
            &bob,
            message(&sender, &random_secret().unwrap(), NOW + 60),
            NOW,
        )
        .unwrap();
    store.remove_sender(&bob, &sender, NOW).unwrap();
    assert!(matches!(
        store.redeem_contact_invite(&alice, &secret, NOW),
        Err(StoreError::Forbidden)
    ));
    assert!(store.allowed_senders(&bob, NOW).unwrap().is_empty());
    assert!(store
        .redeem_contact_invite(&alice, &created.id, NOW)
        .is_err());
    let db = rusqlite::Connection::open(&path).unwrap();
    let stored: String = db
        .query_row("SELECT id FROM contact_invitations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(stored, created.id);
}

#[test]
fn contact_invite_cancellation_expiry_and_capacity() {
    use sigil_protocol::contacts::CreateContactInvite;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let (mut store, alice, bob, _) = setup(&path, NOW);
    let mut values = Vec::new();
    for _ in 0..16 {
        let secret = random_secret().unwrap();
        let invite = store
            .create_contact_invite(
                &bob,
                CreateContactInvite {
                    secret: secret.clone(),
                    expires_at: NOW + 60,
                },
                NOW,
            )
            .unwrap();
        values.push((secret, invite));
    }
    assert!(matches!(
        store.create_contact_invite(
            &bob,
            CreateContactInvite {
                secret: random_secret().unwrap(),
                expires_at: NOW + 60
            },
            NOW
        ),
        Err(StoreError::Busy)
    ));
    assert!(matches!(
        store.revoke_contact_invite(&alice, &values[0].1.id, NOW),
        Err(StoreError::NotFound)
    ));
    store
        .revoke_contact_invite(&bob, &values[0].1.id, NOW)
        .unwrap();
    store
        .create_contact_invite(
            &bob,
            CreateContactInvite {
                secret: values[0].0.clone(),
                expires_at: NOW + 60,
            },
            NOW,
        )
        .unwrap();
    assert!(store
        .redeem_contact_invite(&alice, &values[0].0, NOW)
        .is_err());
    assert!(store
        .redeem_contact_invite(&alice, &values[1].0, NOW + 60)
        .is_err());
    assert!(store
        .redeem_contact_invite(&bob, &values[1].0, NOW)
        .is_err());
    store
        .create_contact_invite(
            &bob,
            CreateContactInvite {
                secret: random_secret().unwrap(),
                expires_at: NOW + 120,
            },
            NOW + 60,
        )
        .unwrap();
    let backup = dir.path().join("backup.db");
    store.backup(&backup).unwrap();
    let restored = dir.path().join("restored.db");
    Store::restore(&backup, &restored).unwrap();
    let db = rusqlite::Connection::open(restored).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM contact_invitations WHERE revoked=0",
            [],
            |r| r.get::<_, u32>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn failed_reciprocal_grant_does_not_consume_invite_or_leave_one_way_permission() {
    use sigil_protocol::contacts::CreateContactInvite;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let (mut store, alice, bob, _) = setup(&path, NOW);
    let claimant = store.session(&alice, NOW).unwrap().device_id;
    let owner = store.session(&bob, NOW).unwrap();
    store.remove_sender(&bob, &claimant, NOW).unwrap();
    let secret = random_secret().unwrap();
    store
        .create_contact_invite(
            &bob,
            CreateContactInvite {
                secret: secret.clone(),
                expires_at: NOW + 60,
            },
            NOW,
        )
        .unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<4096) INSERT INTO devices(id,account_id,label,expires_at) SELECT printf('%064x',x),?1,'Synthetic',999999 FROM n",[owner.account_id]).unwrap();
    db.execute("INSERT INTO allowed_senders SELECT ?1,id FROM devices WHERE label='Synthetic' AND id!=?1 AND id!=?2",(&claimant,&owner.device_id)).unwrap();
    assert!(matches!(
        store.redeem_contact_invite(&alice, &secret, NOW),
        Err(StoreError::Busy)
    ));
    assert!(store.allowed_senders(&bob, NOW).unwrap().is_empty());
    db.execute("DELETE FROM allowed_senders WHERE recipient=?1 AND sender=(SELECT min(sender) FROM allowed_senders WHERE recipient=?1)",[&claimant]).unwrap();
    store.redeem_contact_invite(&alice, &secret, NOW).unwrap();
    assert_eq!(store.allowed_senders(&bob, NOW).unwrap(), vec![claimant]);
}

#[test]
fn competing_devices_cannot_redeem_the_same_contact_invitation() {
    use sigil_protocol::contacts::CreateContactInvite;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let (mut store, alice, bob, _) = setup(&path, NOW);
    let invite = store
        .invite(
            InviteRequest {
                username: "charlie".into(),
                expires_in_seconds: 60,
            },
            NOW,
        )
        .unwrap();
    let charlie = random_secret().unwrap();
    store
        .enroll(
            Enrollment {
                invitation: invite.secret,
                device_credential: charlie.clone(),
                device_label: "Synthetic".into(),
            },
            NOW,
        )
        .unwrap();
    let secret = random_secret().unwrap();
    store
        .create_contact_invite(
            &bob,
            CreateContactInvite {
                secret: secret.clone(),
                expires_at: NOW + 60,
            },
            NOW,
        )
        .unwrap();
    let other = Store::open(&path).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let threads: Vec<_> = [(store, alice), (other, charlie)]
        .into_iter()
        .map(|(mut store, token)| {
            let barrier = barrier.clone();
            let secret = secret.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.redeem_contact_invite(&token, &secret, NOW)
            })
        })
        .collect();
    let values: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(values.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        values
            .iter()
            .filter(|r| matches!(r, Err(StoreError::NotFound)))
            .count(),
        1
    );
}

#[tokio::test]
async fn http_contact_invitation_create_and_redeem_require_native_device_auth() {
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
    let (store, alice, bob, target) = setup(&dir.path().join("sigil.db"), now);
    let admin =
        sigil_server::auth::AdminToken::load_or_create(&dir.path().join("admin.token")).unwrap();
    let app = sigil_server::router(store, admin);
    let secret = random_secret().unwrap();
    let payload = serde_json::json!({"secret":secret,"expires_at":now+60}).to_string();
    for (auth, origin, status) in [
        (false, false, StatusCode::UNAUTHORIZED),
        (true, true, StatusCode::FORBIDDEN),
        (true, false, StatusCode::CREATED),
    ] {
        let mut req = Request::builder()
            .method("POST")
            .uri("/client/v0/contact-invitations")
            .header("content-type", "application/json");
        if auth {
            req = req.header("authorization", format!("Bearer {bob}"));
        }
        if origin {
            req = req.header("origin", "https://example.invalid");
        }
        assert_eq!(
            app.clone()
                .oneshot(req.body(Body::from(payload.clone())).unwrap())
                .await
                .unwrap()
                .status(),
            status
        );
    }
    let result = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/client/v0/contact-invitations/redeem")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {alice}"))
                .body(Body::from(serde_json::json!({"secret":secret}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(result.status(), StatusCode::OK);
    let peer: sigil_protocol::contacts::ContactPeer =
        serde_json::from_slice(&to_bytes(result.into_body(), 8192).await.unwrap()).unwrap();
    assert_eq!(peer.device_id, target);
}

#[tokio::test]
async fn rate_limits_survive_credential_rotation_and_leave_reads_and_ack_available() {
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
    let (store, alice, bob, _) = setup(&dir.path().join("sigil.db"), now);
    let admin =
        sigil_server::auth::AdminToken::load_or_create(&dir.path().join("admin.token")).unwrap();
    let app = sigil_server::router(store, admin);
    // Even structurally invalid authenticated submissions spend this device's budget.
    for _ in 0..63 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/client/v0/contact-invitations")
                    .header("authorization", format!("Bearer {alice}"))
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
    let replacement = random_secret().unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/client/v0/session")
                .header("authorization", format!("Bearer {alice}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"device_credential":replacement}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    for (token, status) in [
        (&replacement, StatusCode::TOO_MANY_REQUESTS),
        (&bob, StatusCode::UNPROCESSABLE_ENTITY),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/client/v0/contact-invitations")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        if status == StatusCode::TOO_MANY_REQUESTS {
            assert!(response.headers().contains_key("retry-after"));
            assert_eq!(response.headers()["cache-control"], "no-store");
        }
    }
    for (method, path, status) in [
        ("GET", "/client/v0/session", StatusCode::OK),
        ("DELETE", "/client/v0/mailbox/1", StatusCode::NOT_FOUND),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("authorization", format!("Bearer {replacement}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status);
    }
}

#[tokio::test]
async fn enrollment_rate_is_bounded_without_blocking_health() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let dir = tempfile::tempdir().unwrap();
    let (store, _, _, _) = setup(&dir.path().join("sigil.db"), NOW);
    let admin =
        sigil_server::auth::AdminToken::load_or_create(&dir.path().join("admin.token")).unwrap();
    let app = sigil_server::router(store, admin);
    for _ in 0..5 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/client/v0/enroll")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/client/v0/enroll")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(response.headers().contains_key("retry-after"));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
