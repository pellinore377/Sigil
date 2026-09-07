use sha2::{Digest, Sha256};
use sigil_protocol::{
    accounts::{Enrollment, InviteRequest},
    recovery::{DeleteObjects, Head, PublishHead, PutObject, MAX_OBJECT_BYTES},
    Configure,
};
use sigil_server::{
    auth::random_secret,
    store::{Store, StoreError},
};
const NOW: u64 = 1000;

fn setup(path: &std::path::Path) -> (Store, String, String) {
    let mut store = Store::open(path).unwrap();
    store.configure(Configure { expected_revision: 0, settings: serde_json::from_str(r#"{"server_name":"chat.example","default_quota_bytes":1048576,"max_attachment_bytes":1048576}"#).unwrap() }).unwrap();
    let mut tokens = Vec::new();
    for name in ["alice", "bob"] {
        let invite = store
            .invite(
                InviteRequest {
                    username: name.into(),
                    expires_in_seconds: 60,
                },
                NOW,
            )
            .unwrap();
        let token = random_secret().unwrap();
        store
            .enroll(
                Enrollment {
                    invitation: invite.secret,
                    device_credential: token.clone(),
                    device_label: "Synthetic".into(),
                },
                NOW,
            )
            .unwrap();
        tokens.push(token);
    }
    (store, tokens.remove(0), tokens.remove(0))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn object(byte: u8, length: usize) -> (String, PutObject) {
    let bytes = vec![byte; length];
    (
        hex(&Sha256::digest(&bytes)),
        PutObject {
            ciphertext: hex(&bytes),
        },
    )
}
fn put(store: &mut Store, token: &str, byte: u8, length: usize) -> String {
    let (id, body) = object(byte, length);
    store.put_recovery_object(token, &id, body, NOW).unwrap();
    id
}
fn publish(previous: &Head, manifest: &str) -> PublishHead {
    PublishHead {
        expected_generation: previous.generation,
        expected_manifest: previous.manifest.clone(),
        manifest: manifest.into(),
        acknowledge_restored_checkpoint: false,
        restore_generation: None,
    }
}
fn delete(head: &Head, ids: &[&str]) -> DeleteObjects {
    DeleteObjects {
        expected_generation: head.generation,
        expected_manifest: head.manifest.clone().unwrap(),
        objects: ids.iter().map(|id| (*id).into()).collect(),
    }
}

#[test]
fn immutable_objects_are_account_scoped_and_survive_reauthorization_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let (mut store, alice, bob) = setup(&path);
    let id = put(&mut store, &alice, 1, 36);
    assert_eq!(put(&mut store, &alice, 1, 36), id);
    assert!(matches!(
        store.recovery_object(&bob, &id, NOW),
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        store.publish_recovery_head(&bob, publish(&Head::default(), &id), NOW),
        Err(StoreError::NotFound)
    ));
    let head = store
        .publish_recovery_head(&alice, publish(&Head::default(), &id), NOW)
        .unwrap();
    assert_eq!(store.recovery_head(&bob, NOW).unwrap(), Head::default());
    let account = store.session(&alice, NOW).unwrap().account_id;
    let invite = store.invite_reauthorization(&account, 60, NOW).unwrap();
    let replacement = random_secret().unwrap();
    store
        .reauthorize(
            Enrollment {
                invitation: invite.secret,
                device_credential: replacement.clone(),
                device_label: "Synthetic replacement".into(),
            },
            NOW,
        )
        .unwrap();
    assert!(matches!(
        store.recovery_head(&alice, NOW),
        Err(StoreError::Unauthorized)
    ));
    drop(store);
    let mut store = Store::open(&path).unwrap();
    assert_eq!(store.recovery_head(&replacement, NOW).unwrap(), head);
    assert_eq!(
        store
            .recovery_object(&replacement, &id, NOW)
            .unwrap()
            .ciphertext,
        "01".repeat(36)
    );
    assert!(store.recovery_head(&replacement, u64::MAX).is_err());
    assert!(store
        .recovery_object(&replacement, &id, NOW + 31 * 86400)
        .is_err());
}

#[test]
fn competing_heads_have_one_winner_and_exact_retries_do_not_increment() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let (mut store, token, _) = setup(&path);
    let ids = [
        put(&mut store, &token, 1, 40),
        put(&mut store, &token, 2, 40),
    ];
    let barrier = std::sync::Barrier::new(2);
    let run = |id: &str| {
        let mut store = Store::open(&path).unwrap();
        barrier.wait();
        store.publish_recovery_head(&token, publish(&Head::default(), id), NOW)
    };
    let results = std::thread::scope(|scope| {
        let a = scope.spawn(|| run(&ids[0]));
        let b = scope.spawn(|| run(&ids[1]));
        [a.join().unwrap(), b.join().unwrap()]
    });
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Err(StoreError::Conflict)))
            .count(),
        1
    );
    let head = store.recovery_head(&token, NOW).unwrap();
    assert_eq!(head.generation, 1);
    let winner = head.manifest.as_ref().unwrap();
    assert_eq!(
        store
            .publish_recovery_head(&token, publish(&Head::default(), winner), NOW)
            .unwrap(),
        head
    );
    assert!(store
        .publish_recovery_head(&token, publish(&head, winner), NOW)
        .is_err());
    let next = put(&mut store, &token, 3, 40);
    let second = store
        .publish_recovery_head(&token, publish(&head, &next), NOW)
        .unwrap();
    assert_eq!(second.generation, 2);
    assert!(matches!(
        store.publish_recovery_head(&token, publish(&Head::default(), winner), NOW),
        Err(StoreError::Conflict)
    ));
    assert_eq!(
        store
            .publish_recovery_head(&token, publish(&head, &next), NOW)
            .unwrap(),
        second
    );
}

#[test]
fn deletion_is_atomic_head_guarded_and_prevents_resurrection() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, alice, bob) = setup(&dir.path().join("sigil.db"));
    let record = put(&mut store, &alice, 1, 40);
    let manifest = put(&mut store, &alice, 2, 40);
    let head = store
        .publish_recovery_head(&alice, publish(&Head::default(), &manifest), NOW)
        .unwrap();
    assert!(store
        .delete_recovery_objects(&bob, delete(&head, &[&record]), NOW)
        .is_err());
    assert!(store
        .delete_recovery_objects(&alice, delete(&head, &[&manifest]), NOW)
        .is_err());
    assert!(store
        .delete_recovery_objects(&alice, delete(&head, &[&record, &record]), NOW)
        .is_err());
    let absent = random_secret().unwrap();
    assert!(matches!(
        store.delete_recovery_objects(&alice, delete(&head, &[&record, &absent]), NOW),
        Err(StoreError::NotFound)
    ));
    assert!(store.recovery_object(&alice, &record, NOW).is_ok());
    let mut stale = delete(&head, &[&record]);
    stale.expected_generation += 1;
    assert!(matches!(
        store.delete_recovery_objects(&alice, stale, NOW),
        Err(StoreError::Conflict)
    ));
    for _ in 0..2 {
        store
            .delete_recovery_objects(&alice, delete(&head, &[&record]), NOW)
            .unwrap();
    }
    assert!(matches!(
        store.recovery_object(&alice, &record, NOW),
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        store.put_recovery_object(&alice, &record, object(1, 40).1, NOW),
        Err(StoreError::AlreadyExists)
    ));
    assert!(matches!(
        store.publish_recovery_head(&alice, publish(&head, &record), NOW),
        Err(StoreError::NotFound)
    ));
    // Identical ciphertext in another account has independent retention state.
    assert_eq!(put(&mut store, &bob, 1, 40), record);
}

#[test]
fn object_bounds_hashes_and_shared_mailbox_quota_are_enforced() {
    use sigil_protocol::mailbox::Submit;
    let dir = tempfile::tempdir().unwrap();
    let (mut store, alice, bob) = setup(&dir.path().join("sigil.db"));
    assert_eq!(MAX_OBJECT_BYTES, sigil_crypto::recovery::MAX_OBJECT_LEN);
    for length in [0, 35, MAX_OBJECT_BYTES + 1] {
        let (id, body) = object(1, length);
        assert!(store.put_recovery_object(&alice, &id, body, NOW).is_err());
    }
    let (id, _) = object(1, 36);
    for ciphertext in ["GG".repeat(36), "a".repeat(73), "02".repeat(36)] {
        assert!(store
            .put_recovery_object(&alice, &id, PutObject { ciphertext }, NOW)
            .is_err());
    }
    let first = put(&mut store, &alice, 0, MAX_OBJECT_BYTES);
    let mut ids = vec![first.clone()];
    for byte in 1..15 {
        ids.push(put(&mut store, &alice, byte, MAX_OBJECT_BYTES));
    }
    // 1,008,990 ciphertext bytes plus the 2,048-byte device reservation leave
    // 37,538 bytes in this account's shared 1 MiB quota.
    let head = store
        .publish_recovery_head(&alice, publish(&Head::default(), &first), NOW)
        .unwrap();
    let recipient = store.session(&alice, NOW).unwrap().device_id;
    let sender = store.session(&bob, NOW).unwrap().device_id;
    store.allow_sender(&alice, &sender, NOW).unwrap();
    let submit = |length| Submit {
        recipient_device: recipient.clone(),
        message_id: random_secret().unwrap(),
        payload: "ab".repeat(length),
        expires_at: NOW + 60,
    };
    assert!(matches!(
        store.submit_message(&bob, submit(20000), NOW),
        Err(StoreError::Busy)
    ));
    let receipt = store.submit_message(&bob, submit(18000), NOW).unwrap();
    let (extra, body) = object(16, 2000);
    assert!(matches!(
        store.put_recovery_object(&alice, &extra, body, NOW),
        Err(StoreError::Busy)
    ));
    assert_eq!(put(&mut store, &alice, 0, MAX_OBJECT_BYTES), first);
    store
        .acknowledge_message(&alice, receipt.sequence, NOW)
        .unwrap();
    assert_eq!(put(&mut store, &alice, 16, 2000), extra);
    store
        .delete_recovery_objects(&alice, delete(&head, &[&ids[1]]), NOW)
        .unwrap();
    put(&mut store, &alice, 17, MAX_OBJECT_BYTES);
}

#[test]
fn operator_restore_preserves_ciphertext_but_requires_explicit_checkpoint_acknowledgement() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, token, _) = setup(&dir.path().join("sigil.db"));
    let account = store.session(&token, NOW).unwrap().account_id;
    let first = put(&mut store, &token, 1, 40);
    let next = put(&mut store, &token, 2, 40);
    let head = store
        .publish_recovery_head(&token, publish(&Head::default(), &first), NOW)
        .unwrap();
    let backup = dir.path().join("backup.db");
    store.backup(&backup).unwrap();
    let restored = dir.path().join("restored.db");
    Store::restore(&backup, &restored).unwrap();
    let mut store = Store::open(&restored).unwrap();
    assert!(store.recovery_head(&token, NOW).is_err());
    let invitation = store.invite_reauthorization(&account, 60, NOW).unwrap();
    let fresh = random_secret().unwrap();
    store
        .reauthorize(
            Enrollment {
                invitation: invitation.secret,
                device_credential: fresh.clone(),
                device_label: "Synthetic restored".into(),
            },
            NOW,
        )
        .unwrap();
    let restored_head = store.recovery_head(&fresh, NOW).unwrap();
    assert!(restored_head.restored_checkpoint);
    assert_eq!(restored_head.generation, head.generation);
    assert!(store.recovery_object(&fresh, &first, NOW).is_ok());
    assert!(matches!(
        store.publish_recovery_head(&fresh, publish(&head, &next), NOW),
        Err(StoreError::Conflict)
    ));
    assert!(store
        .delete_recovery_objects(&fresh, delete(&head, &[&next]), NOW)
        .is_err());
    let mut request = publish(&head, &next);
    request.acknowledge_restored_checkpoint = true;
    let next_head = store.publish_recovery_head(&fresh, request, NOW).unwrap();
    assert!(!next_head.restored_checkpoint);
    assert_eq!(next_head.generation, 2);
    store
        .delete_recovery_objects(&fresh, delete(&next_head, &[&first]), NOW)
        .unwrap();
}

#[tokio::test]
async fn recovery_http_requires_native_device_auth_and_bounds_bodies() {
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let dir = tempfile::tempdir().unwrap();
    let (store, token, _) = setup(&dir.path().join("sigil.db"));
    // HTTP handlers use real time; extend only this synthetic fixture's credentials.
    let db = rusqlite::Connection::open(dir.path().join("sigil.db")).unwrap();
    db.execute("UPDATE devices SET expires_at=?1", [i64::MAX])
        .unwrap();
    let admin_path = dir.path().join("admin.token");
    let admin = sigil_server::auth::AdminToken::load_or_create(&admin_path).unwrap();
    let admin_token = std::fs::read_to_string(admin_path).unwrap();
    let app = sigil_server::router(store, admin);
    let (id, object) = object(1, 40);
    let path = format!("/client/v0/recovery/objects/{id}");
    for credential in [None, Some(admin_token.as_str())] {
        let mut request = Request::builder().uri("/client/v0/recovery/head");
        if let Some(value) = credential {
            request = request.header("authorization", format!("Bearer {value}"));
        }
        assert_eq!(
            app.clone()
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    let request = |method: &str, path: &str, body: String| {
        Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap()
    };
    let mut browser = request("GET", "/client/v0/recovery/head", String::new());
    browser
        .headers_mut()
        .insert("origin", "https://chat.example".parse().unwrap());
    assert_eq!(
        app.clone().oneshot(browser).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    let response = app
        .clone()
        .oneshot(request(
            "PUT",
            &path,
            serde_json::to_string(&object).unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let response = app
        .clone()
        .oneshot(request("GET", &path, String::new()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let returned: PutObject =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024).await.unwrap()).unwrap();
    assert_eq!(returned.ciphertext, object.ciphertext);
    let response = app
        .clone()
        .oneshot(request(
            "PUT",
            "/client/v0/recovery/head",
            serde_json::to_string(&publish(&Head::default(), &id)).unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let head: Head =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024).await.unwrap()).unwrap();
    assert_eq!(head.generation, 1);
    assert_eq!(
        app.clone()
            .oneshot(request(
                "PUT",
                &path,
                "x".repeat(sigil_protocol::recovery::MAX_BODY + 1)
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        app.clone()
            .oneshot(request(
                "PUT",
                "/client/v0/recovery/head",
                r#"{"unexpected":true}"#.into()
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // The static delete route must not be mistaken for an object identifier.
    assert_eq!(
        app.oneshot(request(
            "POST",
            "/client/v0/recovery/objects/delete",
            serde_json::to_string(&delete(&head, &[&id])).unwrap()
        ))
        .await
        .unwrap()
        .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[test]
fn restore_generation_jump_requires_flag_exact_cas_and_preserves_retry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let (mut store, alice, bob) = setup(&path);
    let first = put(&mut store, &alice, 1, 36);
    let second = put(&mut store, &alice, 2, 36);
    let head = store
        .publish_recovery_head(&alice, publish(&Head::default(), &first), NOW)
        .unwrap();
    let repair = || {
        let mut request = publish(&head, &second);
        request.acknowledge_restored_checkpoint = true;
        request.restore_generation = Some(5);
        request
    };
    assert!(store.publish_recovery_head(&alice, repair(), NOW).is_err());
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute("UPDATE recovery_heads SET restored_checkpoint=1", [])
        .unwrap();
    for generation in [0, 1, 2, i64::MAX as u64 + 1, u64::MAX] {
        let mut request = repair();
        request.restore_generation = Some(generation);
        assert!(store.publish_recovery_head(&alice, request, NOW).is_err());
    }
    let mut request = repair();
    request.acknowledge_restored_checkpoint = false;
    assert!(store.publish_recovery_head(&alice, request, NOW).is_err());
    let mut request = repair();
    request.expected_manifest = Some(second.clone());
    assert!(store.publish_recovery_head(&alice, request, NOW).is_err());
    assert!(store.publish_recovery_head(&bob, repair(), NOW).is_err());
    let result = store.publish_recovery_head(&alice, repair(), NOW).unwrap();
    assert_eq!(result.generation, 5);
    assert!(!result.restored_checkpoint);
    drop(store);
    let mut store = Store::open(&path).unwrap();
    assert_eq!(
        store.publish_recovery_head(&alice, repair(), NOW).unwrap(),
        result
    );
    let third = put(&mut store, &alice, 3, 36);
    let mut request = repair();
    request.manifest = third.clone();
    assert!(store.publish_recovery_head(&alice, request, NOW).is_err());
    let next = store
        .publish_recovery_head(&alice, publish(&result, &third), NOW)
        .unwrap();
    assert_eq!(next.generation, 6);
    assert!(store.publish_recovery_head(&alice, repair(), NOW).is_err());
}
