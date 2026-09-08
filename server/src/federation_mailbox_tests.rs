use super::*;
use crate::federation_config::tests::trusted;
use sigil_protocol::mailbox::Delivery;
impl Store {
    // Fixture helpers model fresh operator decisions. Retry tests below retain
    // and replay the actual revisioned wire request instead.
    fn test_permission(
        &mut self,
        credential: &str,
        sender: RemoteSender,
        allowed: bool,
        now: u64,
    ) -> Result<(), StoreError> {
        let revision = match self.federation_sender_permission(
            credential,
            &sender.server,
            &sender.device,
            now,
        ) {
            Ok(v) => v.revision,
            Err(StoreError::NotFound) => 0,
            Err(e) => return Err(e),
        };
        self.configure_federation_sender(
            credential,
            sigil_protocol::federation::ConfigureSender {
                expected_revision: revision,
                sender,
                allowed,
            },
            now,
        )
        .map(|_| ())
    }
    fn allow_federated_sender(
        &mut self,
        credential: &str,
        sender: RemoteSender,
        now: u64,
    ) -> Result<(), StoreError> {
        self.test_permission(credential, sender, true, now)
    }
    fn remove_federated_sender(
        &mut self,
        credential: &str,
        sender: RemoteSender,
        now: u64,
    ) -> Result<(), StoreError> {
        self.test_permission(credential, sender, false, now)
    }
}
fn enroll(store: &mut Store, username: &str, now: u64) -> String {
    let invitation = store
        .invite(
            sigil_protocol::accounts::InviteRequest {
                username: username.into(),
                expires_in_seconds: 60,
            },
            now,
        )
        .unwrap();
    let credential = crate::auth::random_secret().unwrap();
    store
        .enroll(
            sigil_protocol::accounts::Enrollment {
                invitation: invitation.secret,
                device_credential: credential.clone(),
                device_label: "Synthetic".into(),
            },
            now,
        )
        .unwrap();
    credential
}
fn sender(n: u8) -> RemoteSender {
    RemoteSender {
        server: "remote.example".into(),
        account: auth::hex(&[n; 32]),
        device: auth::hex(&[n.wrapping_add(1); 32]),
    }
}
fn message(recipient: &str, source: &RemoteSender, n: u64, expires_at: u64) -> Submit {
    Submit {
        sender_account: source.account.clone(),
        sender_device: source.device.clone(),
        recipient_device: recipient.into(),
        message_id: format!("{n:064x}"),
        payload: "ab".repeat(32),
        expires_at,
    }
}
fn signed(key: &auth::SigningKey, request: &Submit, now: u64, nonce: u64) -> (Vec<u8>, HeaderMap) {
    let body = serde_json::to_vec(request).unwrap();
    let mut bytes = [0; 32];
    bytes[..8].copy_from_slice(&nonce.to_le_bytes());
    let headers = auth::sign(
        key,
        &auth::Request {
            origin: "remote.example",
            destination: "chat.example",
            path: sigil_protocol::federation::DELIVER_PATH,
            body: &body,
        },
        now,
        bytes,
    )
    .unwrap();
    (body, headers)
}
fn receive(
    s: &mut Store,
    key: &auth::SigningKey,
    request: &Submit,
    now: u64,
    nonce: u64,
) -> Result<Receipt, StoreError> {
    let (body, headers) = signed(key, request, now, nonce);
    s.receive_federated_message(&body, &headers, now)
}
fn ingress(s: &Store) -> i64 {
    s.0.query_row("SELECT ingress_bytes FROM federation_usage", [], |r| {
        r.get(0)
    })
    .unwrap()
}
#[test]
fn ingress_permissions_retry_restart_ack_and_failed_commit_are_atomic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    let (mut s, key) = trusted(&path);
    let token = enroll(&mut s, "recipient", 1000);
    let target = s.session(&token, 1000).unwrap().device_id;
    let source = sender(1);
    let request = message(&target, &source, 1, 2000);
    assert!(matches!(
        receive(&mut s, &key, &request, 1000, 1),
        Err(StoreError::Forbidden)
    ));
    s.allow_federated_sender(&token, source.clone(), 1000)
        .unwrap();
    s.0.execute_batch("CREATE TRIGGER synthetic_failure BEFORE INSERT ON mailbox BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(receive(&mut s, &key, &request, 1000, 1).is_err());
    assert_eq!(ingress(&s), 0);
    assert_eq!(
        s.0.query_row("SELECT count(*) FROM federation_nonces", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    s.0.execute_batch("DROP TRIGGER synthetic_failure").unwrap();
    let first = receive(&mut s, &key, &request, 1000, 1).unwrap();
    assert_eq!(receive(&mut s, &key, &request, 1000, 2).unwrap(), first);
    assert_eq!(ingress(&s), (METADATA + 64) as i64);
    let mut changed = request.clone();
    changed.expires_at += 1;
    assert!(matches!(
        receive(&mut s, &key, &changed, 1000, 3),
        Err(StoreError::AlreadyExists)
    ));
    changed = request.clone();
    changed.payload = "cd".repeat(32);
    assert!(matches!(
        receive(&mut s, &key, &changed, 1000, 3),
        Err(StoreError::AlreadyExists)
    ));
    assert!(s.mailbox(&token, 1000).unwrap()[0].origin.is_some());
    let delivered = s.mailbox_after(&token, 0, 1000).unwrap();
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].origin.as_ref().unwrap(), &source);
    drop(s);
    let mut s = Store::open(&path).unwrap();
    assert_eq!(s.mailbox_after(&token, 0, 1000).unwrap(), delivered);
    s.acknowledge_message(&token, delivered[0].sequence, 1000)
        .unwrap();
    assert_eq!(ingress(&s), METADATA as i64);
    s.acknowledge_message(&token, delivered[0].sequence, 1000)
        .unwrap();
    assert_eq!(receive(&mut s, &key, &request, 1000, 3).unwrap(), first);
    assert!(s.mailbox_after(&token, 0, 1000).unwrap().is_empty());
    assert_eq!(ingress(&s), METADATA as i64);
}
#[test]
fn origin_limits_do_not_reset_when_the_origin_invents_more_accounts() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, key) = trusted(&dir.path().join("s.db"));
    let token = enroll(&mut s, "recipient", 1000);
    let target = s.session(&token, 1000).unwrap().device_id;
    let mut first = 0;
    for n in 1..=65u8 {
        let source = sender(n);
        s.allow_federated_sender(&token, source.clone(), 1000)
            .unwrap();
        let request = message(&target, &source, n as u64, 2000);
        let result = receive(&mut s, &key, &request, 1000 + n as u64, n as u64);
        if n == 1 {
            assert!(result.unwrap().sequence > 0);
            first = s.mailbox_after(&token, 0, 1001).unwrap()[0].sequence;
        } else if n == 65 {
            assert!(matches!(result, Err(StoreError::Busy)));
            s.acknowledge_message(&token, first, 1065).unwrap();
            receive(&mut s, &key, &request, 1065, 65).unwrap();
        } else {
            result.unwrap();
        }
    }
    let pending: i64 =
        s.0.query_row(
            "SELECT count(*) FROM mailbox WHERE payload IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pending, 64);
    assert_eq!(ingress(&s), 65 * METADATA as i64 + 64 * 64);
}
#[test]
fn revocation_hides_backlog_immediately_and_reallow_does_not_resurrect_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    let (mut s, key) = trusted(&path);
    let token = enroll(&mut s, "recipient", 1000);
    let target = s.session(&token, 1000).unwrap().device_id;
    let source = sender(1);
    s.allow_federated_sender(&token, source.clone(), 1000)
        .unwrap();
    for n in 0..128u64 {
        let now = if n < 64 { 1000 + n } else { 1200 + n };
        let expiry = if n < 64 { 1100 } else { 2000 };
        receive(&mut s, &key, &message(&target, &source, n, expiry), now, n).unwrap();
    }
    s.remove_federated_sender(&token, source.clone(), 1400)
        .unwrap();
    assert!(s.mailbox_after(&token, 0, 1400).unwrap().is_empty());
    s.allow_federated_sender(&token, source.clone(), 1400)
        .unwrap();
    assert!(s.mailbox_after(&token, 0, 1400).unwrap().is_empty());
    drop(s);
    let mut s = Store::open(&path).unwrap();
    for _ in 0..2 {
        let tx =
            s.0.transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
        assert_eq!(cleanup(&tx).unwrap(), 64);
        tx.commit().unwrap();
    }
    {
        let tx =
            s.0.transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
        assert_eq!(cleanup(&tx).unwrap(), 1);
        tx.commit().unwrap();
    }
    assert_eq!(ingress(&s), 128 * METADATA as i64);
    receive(
        &mut s,
        &key,
        &message(&target, &source, 200, 2000),
        1400,
        200,
    )
    .unwrap();
    assert_eq!(s.mailbox_after(&token, 0, 1400).unwrap().len(), 1);
    let backup = dir.path().join("backup.db");
    s.backup(&backup).unwrap();
    let restored = dir.path().join("restored.db");
    Store::restore(&backup, &restored).unwrap();
    let s = Store::open(&restored).unwrap();
    assert_eq!(ingress(&s), 129 * METADATA as i64);
    let bytes:i64=s.0.query_row("SELECT bytes FROM retained_storage WHERE account_id=(SELECT account_id FROM devices WHERE id=?1)",[&target],|r|r.get(0)).unwrap();
    assert_eq!(
        bytes,
        crate::storage_budget::DEVICE as i64 + 130 * METADATA as i64 + REVOCATION_BYTES as i64
    );
}

#[test]
fn remote_mail_uses_the_existing_push_sequence_and_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, key) = trusted(&dir.path().join("s.db"));
    let recipient = enroll(&mut s, "recipient", 1000);
    let local = enroll(&mut s, "local", 1000);
    let target = s.session(&recipient, 1000).unwrap().device_id;
    let local_device = s.session(&local, 1000).unwrap().device_id;
    s.allow_sender(&recipient, &local_device, 1000).unwrap();
    s.allow_federated_sender(&recipient, sender(1), 1000)
        .unwrap();
    crate::push_delivery::tests::enable_test_push(&mut s, &recipient, 1000);
    let first = s
        .submit_message(
            &local,
            sigil_protocol::mailbox::Submit {
                recipient_device: target.clone(),
                message_id: "aa".repeat(32),
                payload: "ab".repeat(32),
                expires_at: 2000,
            },
            1000,
        )
        .unwrap();
    let through: i64 =
        s.0.query_row(
            "SELECT through_sequence FROM push_jobs WHERE device=?1",
            [&target],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(through, first.sequence);
    let request = message(&target, &sender(1), 2, 2000);
    s.0.execute_batch("CREATE TRIGGER synthetic_failure BEFORE UPDATE ON push_jobs BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(receive(&mut s, &key, &request, 1000, 1).is_err());
    assert_eq!(ingress(&s), 0);
    let retained = s.mailbox_after(&recipient, 0, 1000).unwrap();
    assert_eq!(retained.len(), 1);
    assert_eq!(retained[0].sequence, first.sequence);
    assert!(retained[0].origin.is_none());
    s.0.execute_batch("DROP TRIGGER synthetic_failure").unwrap();
    let second = receive(&mut s, &key, &request, 1000, 1).unwrap();
    assert!(second.sequence > 0);
    let page = s.mailbox_after(&recipient, first.sequence, 1000).unwrap();
    assert_eq!(page.len(), 1);
    assert!(page[0].sequence > first.sequence);
    let through: i64 =
        s.0.query_row(
            "SELECT through_sequence FROM push_jobs WHERE device=?1",
            [&target],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(through, page[0].sequence);
    assert_eq!(s.mailbox(&recipient, 1000).unwrap().len(), 2);
    assert_eq!(page[0].payload, request.payload);
    assert_eq!(page[0].message_id, request.message_id);
    assert_eq!(page[0].origin.as_ref().unwrap(), &sender(1));
}
#[test]
fn schema_fifteen_upgrade_rolls_back_and_preserves_local_receipts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    let (mut s, _) = trusted(&path);
    let token = enroll(&mut s, "recipient", 1000);
    let target = s.session(&token, 1000).unwrap().device_id;
    let request = sigil_protocol::mailbox::Submit {
        recipient_device: target.clone(),
        message_id: "aa".repeat(32),
        payload: "ab".repeat(32),
        expires_at: 2000,
    };
    let frozen = serde_json::to_vec(&request).unwrap();
    let receipt = s
        .submit_message(&token, serde_json::from_slice(&frozen).unwrap(), 1000)
        .unwrap();
    drop(s);
    let db = Connection::open(&path).unwrap();
    db.execute_batch("ALTER TABLE mailbox RENAME TO mailbox_current;CREATE TABLE mailbox(sequence INTEGER PRIMARY KEY AUTOINCREMENT,sender TEXT NOT NULL REFERENCES devices(id),message_id TEXT NOT NULL,recipient TEXT NOT NULL REFERENCES devices(id),payload TEXT,payload_hash BLOB NOT NULL,expires_at INTEGER NOT NULL,UNIQUE(sender,message_id));INSERT INTO mailbox SELECT sequence,sender,message_id,recipient,payload,payload_hash,expires_at FROM mailbox_current;DROP TABLE mailbox_current;CREATE INDEX mailbox_recipient ON mailbox(recipient,sequence);CREATE INDEX mailbox_expiry ON mailbox(expires_at) WHERE payload IS NOT NULL;CREATE INDEX mailbox_live_recipient ON mailbox(recipient,sender,sequence) WHERE payload IS NOT NULL;DROP TABLE operation_uploads; DROP TABLE operations; DROP TABLE operation_configuration; DROP TABLE oidc_grants; DROP TABLE oidc_bindings; DROP TABLE oidc_flows; DROP TABLE oidc_configuration; DROP TABLE registration_usage; DROP TABLE account_policy; DROP TABLE admin_policy; DROP TABLE call_connections; DROP TABLE calls; DROP TABLE call_configuration; DROP TABLE service_budgets; DROP TABLE service_configuration; DROP TABLE map_configuration; DROP TABLE private_group_invitations; DROP TABLE private_group_proposals; DROP TABLE private_group_nonces; DROP TABLE private_group_commits; DROP TABLE private_group_members; DROP TABLE private_groups; DROP TABLE group_credential_uids; DROP TABLE group_authority; DROP INDEX prekeys_available; DROP INDEX prekeys_remote_claim; ALTER TABLE prekeys DROP COLUMN remote_request; ALTER TABLE prekeys DROP COLUMN remote_device; ALTER TABLE prekeys DROP COLUMN remote_account; ALTER TABLE prekeys DROP COLUMN remote_server; CREATE INDEX prekeys_available ON prekeys(device_id,expires_at) WHERE bundle IS NOT NULL AND claimant IS NULL; DROP TABLE federation_outbox;DROP TABLE federation_senders;DROP TABLE federation_revocations;ALTER TABLE federation_admission DROP COLUMN egress_bytes;ALTER TABLE federation_admission DROP COLUMN delivery_not_before;ALTER TABLE federation_usage DROP COLUMN egress_bytes;ALTER TABLE federation_admission DROP COLUMN ingress_bytes;ALTER TABLE federation_usage DROP COLUMN ingress_bytes;PRAGMA user_version=15;CREATE TABLE federation_senders(synthetic INTEGER);").unwrap();
    assert!(Store::open(&path).is_err());
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        15
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM pragma_table_info('mailbox')",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        7
    );
    db.execute_batch("DROP TABLE federation_senders").unwrap();
    let mut s = Store::open(&path).unwrap();
    assert_eq!(s.submit_message(&token, request, 1000).unwrap(), receipt);
    assert_eq!(s.mailbox(&token, 1000).unwrap().len(), 1);
    let mut next = message(&target, &sender(1), 2, 2000);
    s.allow_federated_sender(&token, sender(1), 1000).unwrap();
    // A migrated local message remains isolated from equal remote message IDs.
    next.message_id = "aa".repeat(32);
    let key = auth::SigningKey::generate(1, 900).unwrap();
    // Replace only this synthetic peer pin through its explicit approval path.
    let mut update = crate::federation_config::ConfigurePeer {
        expected_revision: 1,
        allowed: true,
        port: 443,
        approve_key: Some(key.descriptor().id.clone()),
    };
    s.configure_federation_peer("remote.example", update.clone())
        .unwrap();
    let (c, p) = s.begin_federation_refresh("remote.example", 1000).unwrap();
    s.finish_federation_refresh(
        c.revision,
        &p,
        Some(sigil_protocol::federation::Discovery {
            version: 0,
            server: "remote.example".into(),
            current: key.descriptor().clone(),
            rotation: None,
        }),
        1000,
    )
    .unwrap();
    assert!(receive(&mut s, &key, &next, 1000, 1).unwrap().sequence > 0);
    let page = s.mailbox_after(&token, receipt.sequence, 1000).unwrap();
    assert_eq!(page.len(), 1);
    assert!(page[0].sequence > receipt.sequence);
    assert_eq!(page[0].payload, next.payload);
    assert_eq!(page[0].origin.as_ref().unwrap(), &sender(1));
    update.expected_revision = 2;
    update.allowed = false;
    s.configure_federation_peer("remote.example", update)
        .unwrap();
}

#[test]
fn real_https_ingress_and_native_mailbox_enforce_authentication_and_body_limits() {
    use axum::http::Request;
    let _network = crate::egress::tests::NETWORK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let (mut s, key) = trusted(&dir.path().join("s.db"));
    let now = crate::enrollment::now().unwrap();
    let (c, p) = s.begin_federation_refresh("remote.example", now).unwrap();
    s.finish_federation_refresh(
        c.revision,
        &p,
        Some(sigil_protocol::federation::Discovery {
            version: 0,
            server: "remote.example".into(),
            current: key.descriptor().clone(),
            rotation: None,
        }),
        now,
    )
    .unwrap();
    let token = enroll(&mut s, "recipient", now);
    let target = s.session(&token, now).unwrap().device_id;
    s.allow_federated_sender(&token, sender(1), now).unwrap();
    let admin = crate::auth::AdminToken::load_or_create(&dir.path().join("admin")).unwrap();
    let fixture = crate::egress::tests::Fixture::new(crate::router(s, admin));
    let submit = |message: &Submit, nonce: u64| {
        let (body, headers) = signed(&key, message, now, nonce);
        let mut request =
            Request::post(fixture.uri("chat.example", sigil_protocol::federation::DELIVER_PATH))
                .body(body.as_slice())
                .unwrap();
        *request.headers_mut() = headers;
        fixture.federation(request).unwrap()
    };
    let first = message(&target, &sender(1), 1, now + 1000);
    let receipt = submit(&first, 1);
    assert_eq!(receipt.status, 202);
    let receipt: Receipt = serde_json::from_slice(&receipt.body).unwrap();
    assert_eq!(submit(&first, 1).status, 409);
    for (credential, origin, expected) in [
        (None, false, 401),
        (Some(token.as_str()), true, 403),
        (Some(token.as_str()), false, 200),
    ] {
        let mut request = Request::get(fixture.uri("chat.example", "/client/v0/mailbox"));
        if let Some(token) = credential {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        if origin {
            request = request.header("origin", "https://untrusted.example");
        }
        let response = fixture.federation(request.body(&[][..]).unwrap()).unwrap();
        assert_eq!(response.status, expected);
        if expected == 200 {
            let values: Vec<Delivery> = serde_json::from_slice(&response.body).unwrap();
            assert_eq!(values.len(), 1);
            assert!(values[0].sequence > 0 && receipt.sequence > 0);
            assert_eq!(values[0].message_id, first.message_id);
            assert_eq!(values[0].payload, first.payload);
            assert_eq!(receipt.expires_at, first.expires_at);
            assert_eq!(values[0].origin.as_ref().unwrap(), &sender(1));
        }
    }
    let mut maximum = message(&target, &sender(1), 2, now + 1000);
    maximum.payload = "ab".repeat(MAX_PAYLOAD_HEX / 2);
    assert_eq!(submit(&maximum, 2).status, 202);
    maximum.message_id = format!("{:064x}", 3);
    maximum.payload.push_str("ab");
    assert_eq!(submit(&maximum, 3).status, 422);
}

#[test]
fn delayed_permission_retry_cannot_undo_a_newer_decision() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _) = trusted(&dir.path().join("s.db"));
    let token = enroll(&mut s, "recipient", 1000);
    let source = sender(1);
    let allow = sigil_protocol::federation::ConfigureSender {
        expected_revision: 0,
        sender: source.clone(),
        allowed: true,
    };
    let first = s
        .configure_federation_sender(&token, allow.clone(), 1000)
        .unwrap();
    assert_eq!(first.revision, 1);
    assert_eq!(
        s.configure_federation_sender(&token, allow.clone(), 1001)
            .unwrap(),
        first
    );
    let remove = sigil_protocol::federation::ConfigureSender {
        expected_revision: 1,
        sender: source.clone(),
        allowed: false,
    };
    s.configure_federation_sender(&token, remove.clone(), 1001)
        .unwrap();
    // This is a delayed retry of the original allow, not a new decision.
    assert!(matches!(
        s.configure_federation_sender(&token, allow, 1002),
        Err(StoreError::Conflict)
    ));
    assert!(s.federated_senders(&token, 1002).unwrap().is_empty());
    let renewed = sigil_protocol::federation::ConfigureSender {
        expected_revision: 2,
        sender: source.clone(),
        allowed: true,
    };
    s.configure_federation_sender(&token, renewed, 1003)
        .unwrap();
    assert!(matches!(
        s.configure_federation_sender(&token, remove, 1004),
        Err(StoreError::Conflict)
    ));
    assert_eq!(
        s.federated_senders(&token, 1004).unwrap(),
        vec![source.clone()]
    );
    // Existing reservation pays for removal, even after quota exhaustion.
    let quota = s
        .configuration()
        .unwrap()
        .settings
        .unwrap()
        .default_quota_bytes;
    s.0.execute(
        "UPDATE retained_storage SET bytes=?1",
        [sql(quota).unwrap()],
    )
    .unwrap();
    let remove = sigil_protocol::federation::ConfigureSender {
        expected_revision: 3,
        sender: source.clone(),
        allowed: false,
    };
    s.0.execute_batch("CREATE TRIGGER synthetic_failure BEFORE UPDATE ON federation_senders BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(s
        .configure_federation_sender(&token, remove.clone(), 1005)
        .is_err());
    assert!(
        s.federation_sender_permission(&token, &source.server, &source.device, 1005)
            .unwrap()
            .allowed
    );
    s.0.execute_batch("DROP TRIGGER synthetic_failure").unwrap();
    let removed = s
        .configure_federation_sender(&token, remove.clone(), 1005)
        .unwrap();
    assert!(!removed.allowed);
    assert_eq!(removed.revision, 4);
    assert_eq!(
        s.configure_federation_sender(&token, remove, 1006).unwrap(),
        removed
    );
}

#[test]
fn remote_receipt_does_not_disclose_unrelated_local_mailbox_inserts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("quiet.db");
    let other_path = dir.path().join("active.db");
    let (mut quiet, key) = trusted(&path);
    let token = enroll(&mut quiet, "recipient", 1000);
    let target = quiet.session(&token, 1000).unwrap().device_id;
    let local = enroll(&mut quiet, "unrelated", 1000);
    let local_device = quiet.session(&local, 1000).unwrap().device_id;
    quiet
        .allow_federated_sender(&token, sender(1), 1000)
        .unwrap();
    quiet.backup(&other_path).unwrap();
    let mut active = Store::open(&other_path).unwrap();
    active
        .submit_message(
            &local,
            sigil_protocol::mailbox::Submit {
                recipient_device: local_device,
                message_id: "ee".repeat(32),
                payload: "cd".repeat(32),
                expires_at: 2000,
            },
            1000,
        )
        .unwrap();
    let request = message(&target, &sender(1), 1, 2000);
    let (body, headers) = signed(&key, &request, 1000, 1);
    let a = quiet
        .receive_federated_message(&body, &headers, 1000)
        .unwrap();
    let b = active
        .receive_federated_message(&body, &headers, 1000)
        .unwrap();
    let quiet_delivery = quiet.mailbox_after(&token, 0, 1000).unwrap();
    let active_delivery = active.mailbox_after(&token, 0, 1000).unwrap();
    assert_eq!(quiet_delivery.len(), 1);
    assert_eq!(active_delivery.len(), 1);
    assert_eq!(quiet_delivery[0].payload, request.payload);
    assert_eq!(active_delivery[0].payload, request.payload);
    assert_ne!(quiet_delivery[0].sequence, active_delivery[0].sequence);
    assert!(a.sequence > 0 && b.sequence > 0);
    assert_eq!(
        a, b,
        "remote receipt must not reveal unrelated local insertion count"
    );
    drop(active);
    let mut reopened = Store::open(&other_path).unwrap();
    assert_eq!(receive(&mut reopened, &key, &request, 1000, 2).unwrap(), a);
}
