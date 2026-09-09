use super::*;
use sigil_crypto::IdentityKey;
fn id(text: &str) -> [u8; 32] {
    std::array::from_fn(|i| u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).unwrap())
}
pub(crate) fn request(
    store: &mut Store,
    token: &str,
    recipient: &str,
    now: u64,
) -> (IdentityKey, RequestContact) {
    let session = store.session(token, now).unwrap();
    let identity = IdentityKey::generate().unwrap();
    let binding = sigil_protocol::device::Binding {
        server: "chat.example".into(),
        username: session.address.split_once(':').unwrap().0[1..].into(),
        account: id(&session.account_id),
        device: id(&session.device_id),
        identity: identity.public_key(),
    };
    let signature = identity.sign(&binding.signing_bytes().unwrap()).unwrap();
    let binding = hex(&SignedBinding { binding, signature }.to_bytes().unwrap());
    store
        .publish_device_binding(
            token,
            Statement {
                statement: binding.clone(),
            },
            now,
        )
        .unwrap();
    let mut request = RequestContact {
        invitation: None,
        server: "chat.example".into(),
        recipient: recipient.into(),
        expires_at: now + 600,
        binding,
        signature: String::new(),
    };
    request.signature = hex(&identity.sign(&request.signing_bytes().unwrap()).unwrap());
    (identity, request)
}
#[test]
fn requests_authenticate_intent_without_granting_mailbox_or_key_access() {
    let (dir, mut store, alice, bob, now) = crate::admin::tests::setup();
    let a = store.session(&alice, now).unwrap();
    let b = store.session(&bob, now).unwrap();
    let (_, request) = request(&mut store, &alice, &b.account_id, now);
    let before: u64 = store
        .0
        .query_row(
            "SELECT bytes FROM retained_storage WHERE account_id=?1",
            [&b.account_id],
            |r| unsigned(r, 0),
        )
        .unwrap();
    let queued = store.request_contact(&alice, request.clone(), now).unwrap();
    assert_eq!(queued.state, RequestState::Pending);
    assert_eq!(
        store.request_contact(&alice, request.clone(), now).unwrap(),
        queued
    );
    assert_eq!(
        store
            .contact_request_status(&alice, &b.account_id, now)
            .unwrap(),
        queued
    );
    assert!(store
        .contact_request_status(&bob, &b.account_id, now)
        .is_err());
    assert!(store
        .resolve_contact_request(
            &alice,
            &queued.id,
            RequestState::Accepted,
            &request.signature,
            now
        )
        .is_err());
    let page = store.contact_requests(&bob, None, now).unwrap();
    assert_eq!(page.requests.len(), 1);
    assert_eq!(page.requests[0].binding, request.binding);
    assert_eq!(page.requests[0].signature, request.signature);
    assert_eq!(
        store
            .0
            .query_row(
                "SELECT bytes FROM retained_storage WHERE account_id=?1",
                [&b.account_id],
                |r| unsigned(r, 0)
            )
            .unwrap(),
        before + BYTES
    );
    crate::storage_budget::rebuild(&store.0).unwrap();
    assert_eq!(
        store
            .0
            .query_row(
                "SELECT bytes FROM retained_storage WHERE account_id=?1",
                [&b.account_id],
                |r| unsigned(r, 0)
            )
            .unwrap(),
        before + BYTES
    );
    assert_eq!(
        store
            .resolve_contact_request(
                &bob,
                &queued.id,
                RequestState::Accepted,
                &request.signature,
                now
            )
            .unwrap()
            .state,
        RequestState::Accepted
    );
    assert!(crate::admission::check(&store.0, &a.device_id, &b.device_id).is_err());
    assert!(store
        .claim_prekey(&alice, &b.device_id, &"01".repeat(32), now)
        .is_err());
    drop(store);
    let mut store = Store::open(&dir.path().join("sigil.db")).unwrap();
    assert_eq!(
        store
            .contact_request_status(&alice, &b.account_id, now)
            .unwrap()
            .state,
        RequestState::Accepted
    );
    assert!(store
        .contact_requests(&bob, None, now)
        .unwrap()
        .requests
        .is_empty());
    store.expire_batch(now + 601).unwrap();
    assert!(store
        .contact_request_status(&alice, &b.account_id, now + 601)
        .is_err());
    assert_eq!(
        store
            .0
            .query_row(
                "SELECT bytes FROM retained_storage WHERE account_id=?1",
                [&b.account_id],
                |r| unsigned(r, 0)
            )
            .unwrap(),
        before
    );
}
#[test]
fn recipient_expiry_and_identity_substitution_fail_before_storage() {
    let (_dir, mut store, alice, bob, now) = crate::admin::tests::setup();
    let a = store.session(&alice, now).unwrap();
    let b = store.session(&bob, now).unwrap();
    let (_, original) = request(&mut store, &alice, &b.account_id, now);
    let mut changed = original.clone();
    changed.recipient = a.account_id;
    assert!(store.request_contact(&alice, changed, now).is_err());
    let mut changed = original.clone();
    changed.expires_at += 1;
    assert!(store.request_contact(&alice, changed, now).is_err());
    let mut changed = original.clone();
    changed.server = "other.example".into();
    assert!(store.request_contact(&alice, changed, now).is_err());
    assert!(store.request_contact(&bob, original.clone(), now).is_err());
    let mut changed = original.clone();
    changed.signature = "00".repeat(64);
    assert!(store.request_contact(&alice, changed, now).is_err());
    assert!(store.request_contact(&alice, original, now + 601).is_err());
    assert_eq!(
        store
            .0
            .query_row("SELECT count(*) FROM contact_requests", [], |r| unsigned(
                r, 0
            ))
            .unwrap(),
        0
    );
}
#[test]
fn blocking_and_opt_out_survive_retries_without_authorizing_messages() {
    let (_dir, mut store, alice, bob, now) = crate::admin::tests::setup();
    let a = store.session(&alice, now).unwrap();
    let b = store.session(&bob, now).unwrap();
    let (key, mut request) = request(&mut store, &alice, &b.account_id, now);
    store
        .set_contact_request_policy(&bob, RequestPolicy { enabled: false }, now)
        .unwrap();
    assert!(store.request_contact(&alice, request.clone(), now).is_err());
    store
        .set_contact_request_policy(&bob, RequestPolicy { enabled: true }, now)
        .unwrap();
    let receipt = store.request_contact(&alice, request.clone(), now).unwrap();
    store
        .resolve_contact_request(
            &bob,
            &receipt.id,
            RequestState::Blocked,
            &request.signature,
            now,
        )
        .unwrap();
    request.expires_at = now + 700;
    request.signature = hex(&key.sign(&request.signing_bytes().unwrap()).unwrap());
    assert_eq!(
        store
            .request_contact(&alice, request.clone(), now)
            .unwrap()
            .state,
        RequestState::Declined
    );
    assert!(store
        .contact_requests(&bob, None, now)
        .unwrap()
        .requests
        .is_empty());
    assert!(store
        .resolve_contact_request(
            &bob,
            &receipt.id,
            RequestState::Accepted,
            &request.signature,
            now
        )
        .is_err());
    store.expire_batch(now + 601).unwrap();
    assert_eq!(
        store
            .contact_request_status(&alice, &b.account_id, now + 601)
            .unwrap()
            .state,
        RequestState::Declined
    );
    store
        .block_contact_requests(
            &bob,
            BlockContact {
                server: "chat.example".into(),
                account: a.account_id,
                blocked: false,
            },
            now + 601,
        )
        .unwrap();
    assert_eq!(
        store
            .request_contact(&alice, request, now + 601)
            .unwrap()
            .state,
        RequestState::Pending
    );
}
#[test]
fn failed_reservation_and_deletion_leave_no_partial_rows_or_charges() {
    let (_dir, mut store, alice, bob, now) = crate::admin::tests::setup();
    let a = store.session(&alice, now).unwrap();
    let b = store.session(&bob, now).unwrap();
    let (_, request) = request(&mut store, &alice, &b.account_id, now);
    let before: u64 = store
        .0
        .query_row(
            "SELECT bytes FROM retained_storage WHERE account_id=?1",
            [&b.account_id],
            |r| unsigned(r, 0),
        )
        .unwrap();
    store.0.execute_batch("CREATE TRIGGER fail_request BEFORE INSERT ON contact_requests BEGIN SELECT RAISE(ABORT,'synthetic'); END").unwrap();
    assert!(store.request_contact(&alice, request.clone(), now).is_err());
    assert_eq!(
        store
            .0
            .query_row(
                "SELECT bytes FROM retained_storage WHERE account_id=?1",
                [&b.account_id],
                |r| unsigned(r, 0)
            )
            .unwrap(),
        before
    );
    store.0.execute_batch("DROP TRIGGER fail_request").unwrap();
    store.request_contact(&alice, request, now).unwrap();
    store
        .set_contact_request_policy(&alice, RequestPolicy { enabled: false }, now)
        .unwrap();
    let tx = store.0.transaction().unwrap();
    delete_account(&tx, &a.account_id).unwrap();
    tx.commit().unwrap();
    assert_eq!(
        store
            .0
            .query_row("SELECT count(*) FROM contact_requests", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .0
            .query_row("SELECT count(*) FROM contact_request_policy", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .0
            .query_row(
                "SELECT bytes FROM retained_storage WHERE account_id=?1",
                [&b.account_id],
                |r| unsigned(r, 0)
            )
            .unwrap(),
        before
    );
}
#[test]
fn restore_preserves_blocks_and_opt_out_but_cannot_replay_pending_requests() {
    let (dir, mut store, alice, bob, now) = crate::admin::tests::setup();
    let a = store.session(&alice, now).unwrap();
    let b = store.session(&bob, now).unwrap();
    let (_, request) = request(&mut store, &alice, &b.account_id, now);
    store.request_contact(&alice, request, now).unwrap();
    store
        .block_contact_requests(
            &alice,
            BlockContact {
                server: "chat.example".into(),
                account: b.account_id,
                blocked: true,
            },
            now,
        )
        .unwrap();
    store
        .set_contact_request_policy(&alice, RequestPolicy { enabled: false }, now)
        .unwrap();
    let backup = dir.path().join("backup.db");
    let restored = dir.path().join("restored.db");
    store.backup(&backup).unwrap();
    Store::restore(&backup, &restored).unwrap();
    let mut restored = Store::open(&restored).unwrap();
    assert_eq!(
        restored
            .0
            .query_row(
                "SELECT count(*) FROM contact_requests WHERE state!=3",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        restored
            .0
            .query_row(
                "SELECT count(*) FROM contact_requests WHERE state=3",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert!(!enabled(&restored.0, &a.account_id).unwrap());
    let before: u64 = restored
        .0
        .query_row("SELECT sum(bytes) FROM retained_storage", [], |r| {
            unsigned(r, 0)
        })
        .unwrap();
    let tx = restored.0.transaction().unwrap();
    crate::storage_budget::rebuild(&tx).unwrap();
    tx.commit().unwrap();
    assert_eq!(
        restored
            .0
            .query_row("SELECT sum(bytes) FROM retained_storage", [], |r| unsigned(
                r, 0
            ))
            .unwrap(),
        before
    );
}
#[test]
fn recipient_capacity_pagination_and_expiry_are_bounded() {
    let (_dir, mut store, alice, bob, now) = crate::admin::tests::setup();
    let b = store.session(&bob, now).unwrap();
    let (_, request) = request(&mut store, &alice, &b.account_id, now);
    let first = store.request_contact(&alice, request.clone(), now).unwrap();
    // Occupy the remaining recipient slots; all rows retain their storage charge.
    for index in 1..64 {
        let id = format!("{index:064x}");
        store.0.execute("INSERT INTO contact_requests SELECT ?1,recipient,origin,?1,device,binding,signature,created_at,expires_at,state,invitation FROM contact_requests WHERE id=?2",(&id,&first.id)).unwrap();
        store
            .0
            .execute(
                "UPDATE retained_storage SET bytes=bytes+?2 WHERE account_id=?1",
                (&b.account_id, BYTES as i64),
            )
            .unwrap();
    }
    let page = store.contact_requests(&bob, None, now).unwrap();
    assert_eq!(page.requests.len(), 32);
    let second = store
        .contact_requests(&bob, page.next.as_deref(), now)
        .unwrap();
    assert_eq!(second.requests.len(), 32);
    assert!(second.next.is_none());
    assert!(page
        .requests
        .iter()
        .all(|a| second.requests.iter().all(|b| a.receipt.id != b.receipt.id)));
    store
        .0
        .execute(
            "UPDATE contact_requests SET account=?1 WHERE id=?2",
            ("ab".repeat(32), &first.id),
        )
        .unwrap();
    // A new authenticated request must not consume a 65th slot.
    store
        .0
        .execute(
            "UPDATE contact_requests SET id=?1 WHERE id=?2",
            ("fe".repeat(32), &first.id),
        )
        .unwrap();
    assert!(matches!(
        store.request_contact(&alice, request, now),
        Err(StoreError::Busy)
    ));
    assert!(store.expire_batch(now + 601).unwrap() >= 64);
    assert!(store
        .contact_requests(&bob, None, now + 601)
        .unwrap()
        .requests
        .is_empty());
}
#[test]
fn a_delayed_decision_cannot_accept_a_replacement_request() {
    let (_dir, mut store, alice, bob, now) = crate::admin::tests::setup();
    let b = store.session(&bob, now).unwrap();
    let (key, mut request) = request(&mut store, &alice, &b.account_id, now);
    let first = store.request_contact(&alice, request.clone(), now).unwrap();
    let old_signature = request.signature.clone();
    store.expire_batch(now + 601).unwrap();
    request.expires_at = now + 1200;
    request.signature = hex(&key.sign(&request.signing_bytes().unwrap()).unwrap());
    let renewed = store
        .request_contact(&alice, request.clone(), now + 601)
        .unwrap();
    assert_eq!(renewed.id, first.id);
    assert!(store
        .resolve_contact_request(
            &bob,
            &first.id,
            RequestState::Accepted,
            &old_signature,
            now + 601
        )
        .is_err());
    assert_eq!(
        store
            .contact_request_status(&alice, &b.account_id, now + 601)
            .unwrap()
            .state,
        RequestState::Pending
    );
    assert_eq!(
        store
            .resolve_contact_request(
                &bob,
                &renewed.id,
                RequestState::Accepted,
                &request.signature,
                now + 601
            )
            .unwrap()
            .state,
        RequestState::Accepted
    );
}

#[test]
fn directory_is_authenticated_and_schema_32_requests_remain_valid() {
    let (dir, mut store, alice, bob, now) = crate::admin::tests::setup();
    let account = store.session(&bob, now).unwrap().account_id;
    let (_, request) = request(&mut store, &alice, &account, now);
    let queued = store.request_contact(&alice, request.clone(), now).unwrap();
    assert!(store
        .contact_directory(&"00".repeat(32), "alice", now)
        .is_err());
    let directory = store.contact_directory(&bob, "alice", now).unwrap();
    assert!(directory.account.valid_for("alice", "chat.example"));
    assert_eq!(directory.bindings, vec![request.binding.clone()]);
    assert!(directory.links.is_empty());
    store
        .discovery_preference(
            &alice,
            Some(sigil_protocol::admin::DiscoveryPreference {
                revision: 0,
                discoverable: false,
            }),
            now,
        )
        .unwrap();
    assert!(matches!(
        store.contact_directory(&bob, "alice", now),
        Err(StoreError::NotFound)
    ));
    store
        .0
        .execute_batch(
            "ALTER TABLE contact_requests DROP COLUMN invitation; PRAGMA user_version=32;",
        )
        .unwrap();
    drop(store);
    let mut store = Store::open(&dir.path().join("sigil.db")).unwrap();
    assert_eq!(
        store.contact_request_status(&alice, &account, now).unwrap(),
        queued
    );
    assert!(store.contact_requests(&bob, None, now).unwrap().requests[0]
        .invitation
        .is_none());
    let json = serde_json::to_string(&request).unwrap();
    assert!(!json.contains("invitation"));
}
