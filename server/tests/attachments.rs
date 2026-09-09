use sigil_crypto::attachment::{CiphertextList, FileKey, CHUNK_SIZE};
use sigil_protocol::{
    accounts::{Enrollment, InviteRequest},
    attachments::{Begin, Publish, State},
    Configure, Settings,
};
use sigil_server::{
    auth::random_secret,
    store::{Store, StoreError},
};
const NOW: u64 = 1000;
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|v| format!("{v:02x}")).collect()
}
fn setup(path: &std::path::Path, quota: u64) -> (Store, String, String) {
    setup_at(path, quota, NOW)
}
fn setup_at(path: &std::path::Path, quota: u64, now: u64) -> (Store, String, String) {
    let mut store = Store::open(path).unwrap();
    store
        .configure(Configure {
            expected_revision: 0,
            settings: Settings {
                server_name: "chat.example".into(),
                default_quota_bytes: quota,
                max_attachment_bytes: quota,
            },
        })
        .unwrap();
    let mut tokens = Vec::new();
    for username in ["alice", "bob"] {
        let invitation = store
            .invite(
                InviteRequest {
                    username: username.into(),
                    expires_in_seconds: 60,
                },
                now,
            )
            .unwrap();
        let token = random_secret().unwrap();
        store
            .enroll(
                Enrollment {
                    invitation: invitation.secret,
                    device_credential: token.clone(),
                    device_label: "Synthetic".into(),
                },
                now,
            )
            .unwrap();
        tokens.push(token);
    }
    (store, tokens.remove(0), tokens.remove(0))
}
fn file(length: u64) -> (FileKey, String, Begin, Vec<Vec<u8>>, String) {
    let key = FileKey::generate(length).unwrap();
    let id = hex(&key.shape().file);
    let mut chunks = Vec::new();
    let mut list = CiphertextList::new(key.shape()).unwrap();
    for index in 0..key.shape().chunks().unwrap() {
        let bytes = key
            .seal_chunk(
                index,
                &vec![index as u8 + 1; key.shape().chunk_length(index).unwrap()],
            )
            .unwrap();
        list.push(index, &bytes).unwrap();
        chunks.push(bytes);
    }
    let root = hex(&list.finish().unwrap());
    let begin = Begin {
        plaintext_bytes: length,
        access_token: random_secret().unwrap(),
        expires_at: None,
    };
    (key, id, begin, chunks, root)
}
fn publish(root: &str) -> Publish {
    Publish {
        root: root.into(),
        acknowledge_restored_checkpoint: false,
    }
}
fn accounting(db: &rusqlite::Connection, id: &str) -> (i64, i64, i64) {
    db.query_row(
        "SELECT reserved_bytes,stored_bytes,received_chunks FROM attachments WHERE id=?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .unwrap()
}

#[test]
fn cancellation_before_creation_is_durable_charged_and_owner_scoped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    let (mut store, alice, bob) = setup(&path, 8 * CHUNK_SIZE as u64);
    let (_, id, begin, _, _) = file(0);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON attachments BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(store.remove_attachment(&alice, &id, NOW).is_err());
    assert_eq!(
        db.query_row("SELECT count(*) FROM attachments", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    db.execute_batch("DROP TRIGGER fail").unwrap();
    store.remove_attachment(&alice, &id, NOW).unwrap();
    let charge: i64 = db
        .query_row("SELECT sum(bytes) FROM retained_storage", [], |r| r.get(0))
        .unwrap();
    store.remove_attachment(&alice, &id, NOW + 1).unwrap();
    assert_eq!(
        db.query_row("SELECT sum(bytes) FROM retained_storage", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        charge
    );
    drop(store);
    let mut store = Store::open(&path).unwrap();
    assert!(matches!(
        store.begin_attachment(&alice, &id, begin.clone(), NOW + 2),
        Err(StoreError::Conflict)
    ));
    assert!(matches!(
        store.begin_attachment(&bob, &id, begin, NOW + 2),
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        store.remove_attachment(&bob, &id, NOW + 2),
        Err(StoreError::NotFound)
    ));
    assert_eq!(
        store.attachment_status(&alice, &id, NOW + 2).unwrap().state,
        State::Removed
    );
}

#[test]
fn encrypted_chunks_resume_publish_and_require_independent_read_authority() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    let (mut store, alice, bob) = setup(&path, 8 * CHUNK_SIZE as u64);
    let (key, id, begin, chunks, root) = file(CHUNK_SIZE as u64 + 19);
    let initial = store
        .begin_attachment(&alice, &id, begin.clone(), NOW)
        .unwrap();
    assert_eq!(initial.state, State::Uploading);
    assert_eq!(initial.chunks, 2);
    assert!(matches!(
        store.begin_attachment(&bob, &id, begin.clone(), NOW),
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        store.put_attachment_chunk(&bob, &id, 0, &chunks[0], NOW),
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        store.publish_attachment(&alice, &id, publish(&root), NOW),
        Err(StoreError::Conflict)
    ));
    store
        .put_attachment_chunk(&alice, &id, 1, &chunks[1], NOW)
        .unwrap();
    assert_eq!(
        store
            .attachment_parts(&alice, &id, None, NOW)
            .unwrap()
            .chunks[0]
            .index,
        1
    );
    assert!(store
        .attachment_chunk(&alice, &id, 1, &begin.access_token, NOW)
        .is_err());
    drop(store);
    let mut store = Store::open(&path).unwrap();
    assert_eq!(
        store
            .begin_attachment(&alice, &id, begin.clone(), NOW + 10)
            .unwrap()
            .upload_deadline,
        initial.upload_deadline
    );
    store
        .put_attachment_chunk(&alice, &id, 1, &chunks[1], NOW + 10)
        .unwrap();
    store
        .put_attachment_chunk(&alice, &id, 0, &chunks[0], NOW + 10)
        .unwrap();
    let mut altered = chunks[0].clone();
    altered[90] ^= 1;
    assert!(matches!(
        store.put_attachment_chunk(&alice, &id, 0, &altered, NOW),
        Err(StoreError::Conflict)
    ));
    assert!(matches!(
        store.publish_attachment(&alice, &id, publish(&"00".repeat(32)), NOW),
        Err(StoreError::Conflict)
    ));
    let published = store
        .publish_attachment(&alice, &id, publish(&root), NOW)
        .unwrap();
    assert_eq!(published.state, State::Published);
    assert_eq!(
        store
            .publish_attachment(&alice, &id, publish(&root), NOW + 20)
            .unwrap(),
        published
    );
    store
        .put_attachment_chunk(&alice, &id, 0, &chunks[0], NOW)
        .unwrap();
    assert!(matches!(
        store.attachment_chunk(&bob, &id, 0, &"00".repeat(32), NOW),
        Err(StoreError::NotFound)
    ));
    assert!(store
        .attachment_chunk(&"00".repeat(32), &id, 0, &begin.access_token, NOW)
        .is_err());
    let returned = store
        .attachment_chunk(&bob, &id, 0, &begin.access_token, NOW)
        .unwrap();
    assert_eq!(returned, chunks[0]);
    assert_eq!(
        key.open_chunk(0, &returned).unwrap().as_slice(),
        vec![1; CHUNK_SIZE]
    );
    assert_eq!(
        store
            .attachment_status(&alice, &id, NOW)
            .unwrap()
            .received_chunks,
        2
    );
    assert!(store.attachment_status(&bob, &id, NOW).is_err());
}

#[test]
fn quota_reservations_cancel_atomically_and_release_physical_bytes_only_during_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    let (mut store, alice, _) = setup(&path, 3 * CHUNK_SIZE as u64);
    let (_, id, begin, chunks, _) = file(2 * CHUNK_SIZE as u64);
    store
        .begin_attachment(&alice, &id, begin.clone(), NOW)
        .unwrap();
    let (_, other, second, _, _) = file(CHUNK_SIZE as u64);
    assert!(matches!(
        store.begin_attachment(&alice, &other, second.clone(), NOW),
        Err(StoreError::Busy)
    ));
    let db = rusqlite::Connection::open(&path).unwrap();
    let before = accounting(&db, &id);
    db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON attachments BEGIN SELECT RAISE(ABORT,'synthetic quota failure'); END;").unwrap();
    assert!(matches!(
        store.put_attachment_chunk(&alice, &id, 0, &chunks[0], NOW),
        Err(StoreError::Database(_))
    ));
    assert_eq!(accounting(&db, &id), before);
    assert_eq!(
        db.query_row("SELECT count(*) FROM attachment_chunks", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    db.execute_batch("DROP TRIGGER fail;").unwrap();
    store
        .put_attachment_chunk(&alice, &id, 0, &chunks[0], NOW)
        .unwrap();
    store.remove_attachment(&alice, &id, NOW).unwrap();
    store.remove_attachment(&alice, &id, NOW).unwrap();
    let retained = accounting(&db, &id);
    assert_eq!(retained.0, 0);
    assert!(retained.1 > CHUNK_SIZE as i64);
    let (_, large, large_begin, _, _) = file(2 * CHUNK_SIZE as u64);
    assert!(matches!(
        store.begin_attachment(&alice, &large, large_begin.clone(), NOW),
        Err(StoreError::Busy)
    ));
    assert_eq!(
        store
            .begin_attachment(&alice, &id, begin, NOW + 1)
            .unwrap()
            .state,
        State::Removed
    );
    assert!(store
        .put_attachment_chunk(&alice, &id, 0, &chunks[0], NOW)
        .is_err());
    db.execute_batch("CREATE TRIGGER fail BEFORE DELETE ON attachment_chunks BEGIN SELECT RAISE(ABORT,'synthetic cleanup failure'); END;").unwrap();
    assert!(store.expire_batch(NOW).is_err());
    assert_eq!(accounting(&db, &id), retained);
    db.execute_batch("DROP TRIGGER fail;").unwrap();
    store.expire_batch(NOW).unwrap();
    assert_eq!(accounting(&db, &id), (0, 0, 0));
    store
        .begin_attachment(&alice, &large, large_begin, NOW)
        .unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM attachments WHERE id=?1", [&id], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn expiry_disable_and_corruption_fail_before_serving_ciphertext() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    let (mut store, alice, bob) = setup(&path, 4 * CHUNK_SIZE as u64);
    let (_, id, mut begin, chunks, root) = file(0);
    begin.expires_at = Some(NOW + 30);
    store
        .begin_attachment(&alice, &id, begin.clone(), NOW)
        .unwrap();
    store
        .put_attachment_chunk(&alice, &id, 0, &chunks[0], NOW)
        .unwrap();
    store
        .publish_attachment(&alice, &id, publish(&root), NOW)
        .unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute(
        "UPDATE attachment_chunks SET data=zeroblob(length(data)) WHERE file=?1",
        [&id],
    )
    .unwrap();
    assert!(matches!(
        store.attachment_chunk(&bob, &id, 0, &begin.access_token, NOW),
        Err(StoreError::InvalidData)
    ));
    db.execute(
        "UPDATE attachment_chunks SET data=?2 WHERE file=?1",
        (&id, &chunks[0]),
    )
    .unwrap();
    assert_eq!(
        store
            .attachment_status(&alice, &id, NOW + 30)
            .unwrap()
            .state,
        State::Expired
    );
    assert!(store
        .attachment_chunk(&bob, &id, 0, &begin.access_token, NOW + 30)
        .is_err());
    assert_eq!(
        store
            .begin_attachment(&alice, &id, begin, NOW + 30)
            .unwrap()
            .state,
        State::Expired
    );
    let (_, unfinished, begin, _, _) = file(0);
    let initial = store
        .begin_attachment(&alice, &unfinished, begin.clone(), NOW)
        .unwrap();
    assert_eq!(
        store
            .begin_attachment(&alice, &unfinished, begin, initial.upload_deadline)
            .unwrap()
            .state,
        State::Expired
    );
    let (_, active, begin, chunks, root) = file(5);
    store
        .begin_attachment(&alice, &active, begin.clone(), NOW)
        .unwrap();
    store
        .put_attachment_chunk(&alice, &active, 0, &chunks[0], NOW)
        .unwrap();
    store
        .publish_attachment(&alice, &active, publish(&root), NOW)
        .unwrap();
    let account = store.session(&alice, NOW).unwrap().account_id;
    store.disable_account(&account).unwrap();
    assert!(store
        .attachment_chunk(&bob, &active, 0, &begin.access_token, NOW)
        .is_err());
    store.expire_batch(NOW + 86400).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM attachments WHERE state IN(0,1)",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn concurrent_chunk_writers_cannot_replace_a_committed_part() {
    use std::sync::{Arc, Barrier};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    let (mut store, alice, _) = setup(&path, 4 * CHUNK_SIZE as u64);
    let (_, id, begin, chunks, _) = file(17);
    store.begin_attachment(&alice, &id, begin, NOW).unwrap();
    drop(store);
    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for n in 0..2 {
        let path = path.clone();
        let token = alice.clone();
        let id = id.clone();
        let barrier = barrier.clone();
        let mut bytes = chunks[0].clone();
        bytes[90] ^= n;
        handles.push(std::thread::spawn(move || {
            let mut store = Store::open(&path).unwrap();
            barrier.wait();
            store.put_attachment_chunk(&token, &id, 0, &bytes, NOW)
        }));
    }
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Err(StoreError::Conflict)))
            .count(),
        1
    );
    let mut store = Store::open(&path).unwrap();
    assert_eq!(
        store
            .attachment_status(&alice, &id, NOW)
            .unwrap()
            .received_chunks,
        1
    );
}

#[test]
fn restore_revokes_incomplete_uploads_and_requires_explicit_owner_reconciliation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    let (mut store, alice, bob) = setup(&path, 4 * CHUNK_SIZE as u64);
    let (_, id, begin, chunks, root) = file(17);
    store
        .begin_attachment(&alice, &id, begin.clone(), NOW)
        .unwrap();
    store
        .put_attachment_chunk(&alice, &id, 0, &chunks[0], NOW)
        .unwrap();
    store
        .publish_attachment(&alice, &id, publish(&root), NOW)
        .unwrap();
    let (_, pending, pending_begin, pending_chunks, _) = file(0);
    store
        .begin_attachment(&alice, &pending, pending_begin, NOW)
        .unwrap();
    store
        .put_attachment_chunk(&alice, &pending, 0, &pending_chunks[0], NOW)
        .unwrap();
    let accounts = [
        store.session(&alice, NOW).unwrap().account_id,
        store.session(&bob, NOW).unwrap().account_id,
    ];
    let backup = dir.path().join("backup.db");
    store.backup(&backup).unwrap();
    let restored = dir.path().join("restored.db");
    Store::restore(&backup, &restored).unwrap();
    let mut store = Store::open(&restored).unwrap();
    assert!(store.attachment_status(&alice, &id, NOW).is_err());
    let mut tokens = Vec::new();
    for account in accounts {
        let invite = store.invite_reauthorization(&account, 60, NOW).unwrap();
        let token = random_secret().unwrap();
        store
            .reauthorize(
                Enrollment {
                    invitation: invite.secret,
                    device_credential: token.clone(),
                    device_label: "Synthetic recovery".into(),
                },
                NOW,
            )
            .unwrap();
        tokens.push(token);
    }
    assert_eq!(
        store
            .attachment_status(&tokens[0], &pending, NOW)
            .unwrap()
            .state,
        State::Removed
    );
    assert!(store
        .put_attachment_chunk(&tokens[0], &pending, 0, &pending_chunks[0], NOW)
        .is_err());
    assert!(
        store
            .attachment_status(&tokens[0], &id, NOW)
            .unwrap()
            .restored_checkpoint
    );
    assert!(store
        .attachment_chunk(&tokens[1], &id, 0, &begin.access_token, NOW)
        .is_err());
    assert!(matches!(
        store.publish_attachment(&tokens[0], &id, publish(&root), NOW),
        Err(StoreError::Conflict)
    ));
    let mut repair = publish(&root);
    repair.acknowledge_restored_checkpoint = true;
    assert!(matches!(
        store.publish_attachment(&tokens[1], &id, repair.clone(), NOW),
        Err(StoreError::NotFound)
    ));
    assert!(
        !store
            .publish_attachment(&tokens[0], &id, repair, NOW)
            .unwrap()
            .restored_checkpoint
    );
    assert_eq!(
        store
            .attachment_chunk(&tokens[1], &id, 0, &begin.access_token, NOW)
            .unwrap(),
        chunks[0]
    );
}

#[test]
fn chunk_listing_and_cleanup_are_bounded_and_schema_twelve_upgrades() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    let (store, alice, _) = setup(&path, 128 * CHUNK_SIZE as u64);
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch(
        "DROP TABLE profile_shares; DROP TABLE profile_photos; DROP TABLE contact_requests; DROP TABLE contact_request_policy; DROP TABLE account_passwords; DROP TABLE password_policy; DROP TABLE web_oidc; DROP TABLE web_sessions; DROP TABLE oidc_fallback_ack; DROP TABLE oidc_transition; DROP TABLE account_profiles; DROP TABLE web_owner; DROP TABLE deleted_accounts; DROP TABLE operation_uploads; DROP TABLE operations; DROP TABLE operation_configuration; DROP TABLE oidc_grants; DROP TABLE oidc_bindings; DROP TABLE oidc_flows; DROP TABLE oidc_configuration; DROP TABLE registration_usage; DROP TABLE account_policy; DROP TABLE admin_policy; DROP TABLE call_connections; DROP TABLE calls; DROP TABLE call_configuration; DROP TABLE service_budgets; DROP TABLE service_configuration; DROP TABLE map_configuration; DROP TABLE private_group_invitations; DROP TABLE private_group_proposals; DROP TABLE private_group_nonces; DROP TABLE private_group_commits; DROP TABLE private_group_members; DROP TABLE private_groups; DROP TABLE group_credential_uids; DROP TABLE group_authority; DROP INDEX prekeys_available; DROP INDEX prekeys_remote_claim; ALTER TABLE prekeys DROP COLUMN remote_request; ALTER TABLE prekeys DROP COLUMN remote_device; ALTER TABLE prekeys DROP COLUMN remote_account; ALTER TABLE prekeys DROP COLUMN remote_server; CREATE INDEX prekeys_available ON prekeys(device_id,expires_at) WHERE bundle IS NOT NULL AND claimant IS NULL; DROP TABLE federation_outbox; DROP TABLE federation_revocations; DROP TABLE federation_senders; DROP TABLE federation_nonces; DROP TABLE federation_admission; DROP TABLE federation_usage; DROP TABLE federation_peers; DROP TABLE federation_configuration; DROP TABLE push_jobs; DROP TABLE push_channels; DROP TABLE push_configuration; DROP TABLE attachment_chunks; DROP TABLE attachments; PRAGMA user_version=12;",
    )
    .unwrap();
    let mut store = Store::open(&path).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        sigil_server::store::SCHEMA_VERSION
    );
    let id = random_secret().unwrap();
    let request = Begin {
        plaintext_bytes: 65 * CHUNK_SIZE as u64,
        access_token: random_secret().unwrap(),
        expires_at: None,
    };
    store.begin_attachment(&alice, &id, request, NOW).unwrap();
    // The opaque server validates shape/ownership; AEAD validation is client work.
    let bytes = vec![1; CHUNK_SIZE + sigil_crypto::attachment::CHUNK_OVERHEAD];
    for index in 0..65 {
        store
            .put_attachment_chunk(&alice, &id, index, &bytes, NOW)
            .unwrap();
    }
    let first = store.attachment_parts(&alice, &id, None, NOW).unwrap();
    assert_eq!(first.chunks.len(), 64);
    assert_eq!(first.next_after, Some(63));
    let last = store
        .attachment_parts(&alice, &id, first.next_after, NOW)
        .unwrap();
    assert_eq!(last.chunks.len(), 1);
    assert_eq!(last.chunks[0].index, 64);
    assert_eq!(last.next_after, None);
    store.remove_attachment(&alice, &id, NOW).unwrap();
    assert_eq!(store.expire_batch(NOW).unwrap(), 64);
    assert_eq!(accounting(&db, &id).2, 1);
    assert_eq!(store.expire_batch(NOW).unwrap(), 1);
    assert_eq!(accounting(&db, &id), (0, 0, 0));
    assert_eq!(store.expire_batch(NOW).unwrap(), 0);
}

#[tokio::test]
async fn http_authentication_binary_limits_and_private_download_headers() {
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    async fn send(
        app: &axum::Router,
        method: &str,
        path: &str,
        token: Option<&str>,
        content_type: &str,
        body: Vec<u8>,
        headers: &[(&str, &str)],
    ) -> axum::response::Response {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", content_type);
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        for (key, value) in headers {
            request = request.header(*key, *value);
        }
        app.clone()
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap()
    }
    let dir = tempfile::tempdir().unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let (store, alice, bob) = setup_at(&dir.path().join("server.db"), 4 * CHUNK_SIZE as u64, now);
    let app = sigil_server::router(
        store,
        sigil_server::auth::AdminToken::load_or_create(&dir.path().join("admin.token")).unwrap(),
    );
    let (key, id, begin, chunks, root) = file(CHUNK_SIZE as u64);
    let path = format!("/client/v0/attachments/{id}");
    let chunk_path = format!("{path}/chunks/0");
    assert_eq!(
        send(&app, "PUT", &path, None, "application/json", vec![], &[])
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(
            &app,
            "PUT",
            &path,
            Some(&alice),
            "application/json",
            serde_json::to_vec(&begin).unwrap(),
            &[("origin", "https://untrusted.example")]
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(
            &app,
            "PUT",
            &path,
            Some(&alice),
            "application/json",
            serde_json::to_vec(&begin).unwrap(),
            &[]
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        send(
            &app,
            "PUT",
            &chunk_path,
            Some(&alice),
            "text/plain",
            chunks[0].clone(),
            &[]
        )
        .await
        .status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    assert_eq!(
        send(
            &app,
            "PUT",
            &chunk_path,
            Some(&alice),
            "application/octet-stream",
            vec![0; CHUNK_SIZE + 85],
            &[]
        )
        .await
        .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        send(
            &app,
            "PUT",
            &chunk_path,
            Some(&alice),
            "application/octet-stream",
            chunks[0].clone(),
            &[]
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(
            &app,
            "POST",
            &format!("{path}/publish"),
            Some(&alice),
            "application/json",
            serde_json::to_vec(&publish(&root)).unwrap(),
            &[]
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        send(
            &app,
            "GET",
            &chunk_path,
            Some(&bob),
            "application/octet-stream",
            vec![],
            &[]
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    let access = sigil_protocol::attachments::ACCESS_HEADER;
    assert_eq!(
        send(
            &app,
            "GET",
            &chunk_path,
            Some(&bob),
            "application/octet-stream",
            vec![],
            &[(access, &begin.access_token), (access, &begin.access_token)]
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    let response = send(
        &app,
        "GET",
        &chunk_path,
        Some(&bob),
        "application/octet-stream",
        vec![],
        &[(access, &begin.access_token)],
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(
        response.headers()["content-type"],
        "application/octet-stream"
    );
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    let received = to_bytes(response.into_body(), CHUNK_SIZE + 84)
        .await
        .unwrap();
    assert_eq!(
        key.open_chunk(0, &received).unwrap().as_slice(),
        vec![1; CHUNK_SIZE]
    );
    assert_eq!(
        send(
            &app,
            "GET",
            &format!("{path}/parts"),
            Some(&bob),
            "application/json",
            vec![],
            &[]
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(
            &app,
            "DELETE",
            &path,
            Some(&alice),
            "application/json",
            vec![],
            &[]
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(
            &app,
            "GET",
            &chunk_path,
            Some(&bob),
            "application/octet-stream",
            vec![],
            &[(access, &begin.access_token)]
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
}
