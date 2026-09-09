use super::*;
use crate::push_config::{Configure, FcmUpdate};
use sigil_protocol::{
    accounts::{Enrollment, InviteRequest},
    push::{Confirm, Disable, Register},
    Configure as ConfigureServer, Settings,
};
const NOW: u64 = 1000;
fn setup(path: &std::path::Path) -> (Store, String, String) {
    let mut store = Store::open(path).unwrap();
    store
        .configure(ConfigureServer {
            expected_revision: 0,
            settings: Settings {
                server_name: "chat.example".into(),
                default_quota_bytes: 16 * 1024 * 1024,
                max_attachment_bytes: 1024 * 1024,
            },
        })
        .unwrap();
    store.configure_push(configuration(0)).unwrap();
    let mut tokens = Vec::new();
    for username in ["alice", "bob"] {
        let invitation = store
            .invite(
                InviteRequest {
                    username: username.into(),
                    expires_in_seconds: 60,
                },
                NOW,
            )
            .unwrap();
        let credential = random_secret().unwrap();
        store
            .enroll(
                Enrollment {
                    invitation: invitation.secret,
                    device_credential: credential.clone(),
                    device_label: "Synthetic".into(),
                },
                NOW,
            )
            .unwrap();
        tokens.push(credential);
    }
    (store, tokens.remove(0), tokens.remove(0))
}
fn configuration(expected_revision: u64) -> Configure {
    Configure {
        expected_revision,
        unified_push: true,
        contact: Some("mailto:operator@example.com".into()),
        exceptions: Vec::new(),
        rotate_vapid: false,
        fcm: FcmUpdate::Configure(push_provider::FcmCredentials {
            project_id: "sigil-synthetic".into(),
            client_email: "push@sigil-synthetic.iam.gserviceaccount.com".into(),
            private_key: Zeroizing::new(
                include_str!("../tests/fixtures/synthetic-fcm-key.pem").into(),
            ),
        }),
    }
}
fn registration(expected_revision: u64) -> Register {
    Register {
        expected_revision,
        target: Target::Fcm {
            token: "synthetic-registration".into(),
        },
    }
}
fn confirm(job: &Job) -> Confirm {
    let Hint::Challenge { channel, proof } = &job.hint else {
        panic!("expected challenge")
    };
    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    Confirm {
        revision: job.revision,
        channel: hex(channel),
        proof: hex(proof.as_slice()),
    }
}
fn accepted() -> Delivery {
    Delivery {
        outcome: Outcome::Accepted,
        global_backoff: false,
    }
}
fn message(target: &str, n: u64, expires: u64) -> sigil_protocol::mailbox::Submit {
    sigil_protocol::mailbox::Submit {
        recipient_device: target.into(),
        message_id: format!("{n:064x}"),
        payload: "ab".repeat(32),
        expires_at: expires,
    }
}
fn activate(store: &mut Store, credential: &str, now: u64) {
    store
        .register_push(credential, registration(0), now)
        .unwrap();
    let job = store.claim_push(now).unwrap().unwrap();
    store.confirm_push(credential, confirm(&job), now).unwrap();
}
pub(crate) fn enable_test_push(store: &mut Store, credential: &str, now: u64) {
    store.configure_push(configuration(0)).unwrap();
    activate(store, credential, now);
}
#[test]
fn mailbox_enqueue_and_completion_are_atomic_across_arrivals_restart_and_ack() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    let (mut store, alice, bob) = setup(&path);
    let sender = store.session(&alice, NOW).unwrap().device_id;
    let target = store.session(&bob, NOW).unwrap().device_id;
    store.allow_sender(&bob, &sender, NOW).unwrap();
    activate(&mut store, &bob, NOW);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON push_jobs BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(store
        .submit_message(&alice, message(&target, 1, NOW + 1000), NOW)
        .is_err());
    assert!(store.mailbox(&bob, NOW).unwrap().is_empty());
    db.execute_batch("DROP TRIGGER fail").unwrap();
    let first = store
        .submit_message(&alice, message(&target, 1, NOW + 1000), NOW)
        .unwrap();
    let leased = store.claim_push(NOW).unwrap().unwrap();
    assert!(matches!(leased.hint, Hint::Wake));
    drop(store);
    let mut store = Store::open(&path).unwrap();
    assert!(store.claim_push(NOW + 44).unwrap().is_none());
    let renewed = store.claim_push(NOW + 45).unwrap().unwrap();
    assert_ne!(renewed.lease, leased.lease);
    assert!(!store.finish_push(&leased, accepted(), NOW + 45).unwrap());
    let second = store
        .submit_message(&alice, message(&target, 2, NOW + 2000), NOW + 46)
        .unwrap();
    assert!(store.finish_push(&renewed, accepted(), NOW + 47).unwrap());
    let remaining = store.claim_push(NOW + 48).unwrap().unwrap();
    assert_eq!(remaining.through, second.sequence);
    assert!(store.finish_push(&remaining, accepted(), NOW + 48).unwrap());
    assert!(store.claim_push(NOW + 49).unwrap().is_none());
    // Provider acceptance never consumes mailbox ciphertext.
    assert_eq!(store.mailbox(&bob, NOW + 49).unwrap().len(), 2);
    store
        .acknowledge_message(&bob, first.sequence, NOW + 49)
        .unwrap();
    store
        .acknowledge_message(&bob, second.sequence, NOW + 49)
        .unwrap();
    let third = store
        .submit_message(&alice, message(&target, 3, NOW + 2000), NOW + 50)
        .unwrap();
    store
        .acknowledge_message(&bob, third.sequence, NOW + 50)
        .unwrap();
    assert!(store.claim_push(NOW + 50).unwrap().is_none());
    store
        .submit_message(&alice, message(&target, 4, NOW + 60), NOW + 51)
        .unwrap();
    assert!(store.claim_push(NOW + 60).unwrap().is_none());
}

#[test]
fn retry_after_survives_expired_jobs_new_mail_and_configuration_races() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    let (mut store, alice, bob) = setup(&path);
    let sender = store.session(&alice, NOW).unwrap().device_id;
    let target = store.session(&bob, NOW).unwrap().device_id;
    store.allow_sender(&bob, &sender, NOW).unwrap();
    activate(&mut store, &bob, NOW);
    store
        .submit_message(&alice, message(&target, 1, NOW + 30), NOW)
        .unwrap();
    let job = store.claim_push(NOW).unwrap().unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON push_jobs BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    let retry = Delivery {
        outcome: Outcome::Retry {
            not_before: NOW + 300,
            refresh_auth: false,
        },
        global_backoff: true,
    };
    assert!(store.finish_push(&job, retry, NOW + 1).is_err());
    assert_eq!(push_config::read(&store.0).unwrap().fcm_not_before, 0);
    db.execute_batch("DROP TRIGGER fail").unwrap();
    assert!(store.finish_push(&job, retry, NOW + 1).unwrap());
    store.expire_batch(NOW + 31).unwrap();
    store
        .submit_message(&alice, message(&target, 2, NOW + 1000), NOW + 32)
        .unwrap();
    drop(store);
    let mut store = Store::open(&path).unwrap();
    assert!(store.claim_push(NOW + 299).unwrap().is_none());
    let job = store.claim_push(NOW + 300).unwrap().unwrap();
    // A credential/configuration change makes an old provider failure stale.
    let mut changed = configuration(1);
    changed.fcm = FcmUpdate::Keep;
    store.configure_push(changed).unwrap();
    assert!(!store
        .finish_push(
            &job,
            Delivery {
                outcome: Outcome::InvalidRegistration,
                global_backoff: false
            },
            NOW + 301
        )
        .unwrap());
    assert_eq!(
        store.push_status(&bob, NOW + 301).unwrap().state,
        State::Active
    );
    let current = store.claim_push(NOW + 301).unwrap().unwrap();
    assert!(store
        .finish_push(
            &current,
            Delivery {
                outcome: Outcome::InvalidRegistration,
                global_backoff: false
            },
            NOW + 302
        )
        .unwrap());
    assert_eq!(
        store.push_status(&bob, NOW + 302).unwrap().state,
        State::Invalid
    );
    assert!(store.claim_push(NOW + 400).unwrap().is_none());
}

#[test]
fn repeated_fcm_throttling_backs_off_exponentially_across_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    let (mut store, alice, bob) = setup(&path);
    let sender = store.session(&alice, NOW).unwrap().device_id;
    let target = store.session(&bob, NOW).unwrap().device_id;
    store.allow_sender(&bob, &sender, NOW).unwrap();
    activate(&mut store, &bob, NOW);
    store
        .submit_message(&alice, message(&target, 1, NOW + 86400), NOW)
        .unwrap();
    let mut now = NOW;
    for minimum in [60, 120, 240, 480, 960, 1920, 3600, 3600] {
        let job = store.claim_push(now).unwrap().unwrap();
        store
            .finish_push(&job, Delivery::retry(now, 60, true), now)
            .unwrap();
        let due = push_config::read(&store.0).unwrap().fcm_not_before;
        assert!((now + minimum..=now + minimum + minimum / 4).contains(&due));
        drop(store);
        store = Store::open(&path).unwrap();
        assert!(store.claim_push(due - 1).unwrap().is_none());
        now = due;
    }
    assert_eq!(store.mailbox(&bob, now).unwrap().len(), 1);
}

#[test]
fn provider_confirmation_requires_device_echo_and_rollback_retains_slot_budget() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    let (mut store, alice, _) = setup(&path);
    let db = rusqlite::Connection::open(&path).unwrap();
    let before: i64 = db
        .query_row("SELECT sum(bytes) FROM retained_storage", [], |r| r.get(0))
        .unwrap();
    db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON push_jobs BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(store.register_push(&alice, registration(0), NOW).is_err());
    assert_eq!(store.push_status(&alice, NOW).unwrap().revision, 0);
    assert_eq!(
        db.query_row("SELECT sum(bytes) FROM retained_storage", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        before
    );
    db.execute_batch("DROP TRIGGER fail").unwrap();
    store.register_push(&alice, registration(0), NOW).unwrap();
    let job = store.claim_push(NOW).unwrap().unwrap();
    store.finish_push(&job, accepted(), NOW + 1).unwrap();
    assert_eq!(
        store.push_status(&alice, NOW + 1).unwrap().state,
        State::Pending
    );
    assert!(store.claim_push(NOW + 30).unwrap().is_none());
    let again = store.claim_push(NOW + 31).unwrap().unwrap();
    assert_eq!(confirm(&again).proof, confirm(&job).proof);
    let mut wrong = confirm(&again);
    wrong.proof = "00".repeat(32);
    assert!(store.confirm_push(&alice, wrong, NOW + 31).is_err());
    assert_eq!(
        store.push_status(&alice, NOW + 600).unwrap().state,
        State::Expired
    );
    assert!(store
        .confirm_push(&alice, confirm(&again), NOW + 600)
        .is_err());
    store
        .register_push(&alice, registration(1), NOW + 600)
        .unwrap();
    let device = store.session(&alice, NOW + 600).unwrap().device_id;
    store.revoke_device(&alice, &device, NOW + 600).unwrap();
    assert!(store.claim_push(NOW + 600).unwrap().is_none());
    assert_eq!(
        db.query_row("SELECT sum(bytes) FROM retained_storage", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        before + push::SLOT_BYTES as i64
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM push_channels WHERE target IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn configuration_rotation_migration_and_restore_preserve_safe_retry_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    let (mut store, alice, _) = setup(&path);
    let first = store.push_configuration().unwrap();
    assert!(store.configure_push(configuration(0)).unwrap() == first);
    let public = serde_json::to_string(&first).unwrap();
    assert!(!public.contains("PRIVATE KEY"));
    assert!(!public.contains("private_key"));
    let vapid = Vapid::from_pkcs8(
        push_config::read(&store.0)
            .unwrap()
            .settings
            .vapid
            .as_ref()
            .unwrap(),
    )
    .unwrap();
    use base64ct::{Base64UrlUnpadded as B64, Encoding};
    use web_push_native::p256::{elliptic_curve::sec1::ToEncodedPoint, SecretKey};
    let secret = SecretKey::from_slice(&[8; 32]).unwrap();
    let request = Register {
        expected_revision: 0,
        target: Target::UnifiedPush {
            endpoint: "https://push.example/synthetic".into(),
            public_key: B64::encode_string(secret.public_key().to_encoded_point(false).as_bytes()),
            auth_secret: B64::encode_string(&[9; 16]),
            vapid_key: vapid.public_key(),
        },
    };
    store.register_push(&alice, request.clone(), NOW).unwrap();
    let job = store.claim_push(NOW).unwrap().unwrap();
    store.confirm_push(&alice, confirm(&job), NOW).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON push_configuration BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    let mut rotate = configuration(1);
    rotate.rotate_vapid = true;
    rotate.fcm = FcmUpdate::Keep;
    assert!(store.configure_push(rotate.clone()).is_err());
    assert!(store.push_configuration().unwrap() == first);
    assert_eq!(store.push_status(&alice, NOW).unwrap().state, State::Active);
    db.execute_batch("DROP TRIGGER fail").unwrap();
    let rotated = store.configure_push(rotate.clone()).unwrap();
    assert!(rotated.vapid_public_key != first.vapid_public_key);
    assert!(store.configure_push(rotate).unwrap() == rotated);
    assert_eq!(
        store.push_status(&alice, NOW + 1).unwrap().state,
        State::Invalid
    );
    assert_eq!(
        store.register_push(&alice, request, NOW + 1).unwrap().state,
        State::Invalid
    );
    let backup = dir.path().join("backup.db");
    store.backup(&backup).unwrap();
    let restored = dir.path().join("restored.db");
    Store::restore(&backup, &restored).unwrap();
    let mut restored = Store::open(&restored).unwrap();
    let config = restored.push_configuration().unwrap();
    assert!(!config.unified_push);
    assert!(config.fcm_project_id.is_none());
    assert!(config.vapid_public_key.is_none());
    assert!(restored.claim_push(NOW + 2).unwrap().is_none());
    assert!(restored.push_status(&alice, NOW + 2).is_err());
    assert_eq!(restored.0.query_row("SELECT count(*) FROM push_channels WHERE target IS NOT NULL OR proof IS NOT NULL OR proof_hash IS NOT NULL",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(store);
    db.execute_batch("ALTER TABLE private_groups DROP COLUMN blocked; DROP TABLE profile_shares; DROP TABLE profile_photos; DROP TABLE contact_requests; DROP TABLE contact_request_policy; DROP TABLE account_passwords; DROP TABLE password_policy; DROP TABLE web_oidc; DROP TABLE web_sessions; DROP TABLE oidc_fallback_ack; DROP TABLE oidc_transition; DROP TABLE account_profiles; DROP TABLE web_owner; DROP TABLE deleted_accounts; DROP TABLE operation_uploads; DROP TABLE operations; DROP TABLE operation_configuration; DROP TABLE oidc_grants; DROP TABLE oidc_bindings; DROP TABLE oidc_flows; DROP TABLE oidc_configuration; DROP TABLE registration_usage; DROP TABLE account_policy; DROP TABLE admin_policy; DROP TABLE call_connections; DROP TABLE calls; DROP TABLE call_configuration; DROP TABLE service_budgets; DROP TABLE service_configuration; DROP TABLE map_configuration; DROP TABLE private_group_invitations; DROP TABLE private_group_proposals; DROP TABLE private_group_nonces; DROP TABLE private_group_commits; DROP TABLE private_group_members; DROP TABLE private_groups; DROP TABLE group_credential_uids; DROP TABLE group_authority; DROP INDEX prekeys_available; DROP INDEX prekeys_remote_claim; ALTER TABLE prekeys DROP COLUMN remote_request; ALTER TABLE prekeys DROP COLUMN remote_device; ALTER TABLE prekeys DROP COLUMN remote_account; ALTER TABLE prekeys DROP COLUMN remote_server; CREATE INDEX prekeys_available ON prekeys(device_id,expires_at) WHERE bundle IS NOT NULL AND claimant IS NULL; DROP TABLE federation_outbox; DROP TABLE federation_revocations; DROP TABLE federation_senders; DROP TABLE federation_nonces; DROP TABLE federation_admission; DROP TABLE federation_usage; DROP TABLE federation_peers; DROP TABLE federation_configuration; DROP TABLE push_jobs;DROP TABLE push_channels;DROP TABLE push_configuration;PRAGMA user_version=13;CREATE TABLE push_jobs(synthetic INTEGER);").unwrap();
    assert!(Store::open(&path).is_err());
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        13
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name='push_configuration'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    db.execute_batch("DROP TABLE push_jobs").unwrap();
    let store = Store::open(&path).unwrap();
    assert_eq!(store.push_configuration().unwrap().revision, 0);
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        crate::store::SCHEMA_VERSION
    );
}

#[test]
fn dispatcher_uses_fixed_google_destinations_refreshes_tokens_and_honors_completion_time() {
    use axum::{
        body::Bytes,
        http::{HeaderMap, StatusCode},
        routing::post,
        Json, Router,
    };
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };
    let _guard = crate::egress::tests::NETWORK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    let (mut store, alice, _) = setup(&path);
    store.register_push(&alice, registration(0), NOW).unwrap();
    let job = store.claim_push(NOW).unwrap().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let copied = seen.clone();
    let sends = Arc::new(AtomicUsize::new(0));
    let count = sends.clone();
    let fixture=crate::egress::tests::Fixture::new(Router::new().route("/token",post(move|headers:HeaderMap,bytes:Bytes|{let seen=copied.clone();async move{
        assert!(!headers.contains_key("authorization"));assert!(bytes.starts_with(b"grant_type=urn%3A"));seen.lock().unwrap().push("token");
        Json(serde_json::json!({"access_token":"synthetic-access","expires_in":3600,"token_type":"Bearer"}))
    }})).route("/send",post(move|headers:HeaderMap,bytes:Bytes|{let count=count.clone();async move{
        assert_eq!(headers["authorization"],"Bearer synthetic-access");let body:serde_json::Value=serde_json::from_slice(&bytes).unwrap();assert!(body["message"].get("notification").is_none());
        let status=match count.fetch_add(1,Ordering::SeqCst){0=>StatusCode::OK,1=>StatusCode::UNAUTHORIZED,_=>StatusCode::TOO_MANY_REQUESTS};
        (status,[("retry-after","120")],Json(serde_json::json!({"name":"projects/synthetic/messages/1"})))
    }})));
    let mut provider = Provider::default();
    let perform = |provider: &mut Provider| {
        let ticks = std::cell::Cell::new(NOW);
        provider.deliver_with(
            &job,
            || {
                let now = ticks.get();
                ticks.set(now + 1);
                Ok(now)
            },
            |google, request| {
                assert!(google);
                let path = match request.uri().to_string().as_str() {
                    "https://oauth2.googleapis.com/token" => "/token",
                    "https://fcm.googleapis.com/v1/projects/sigil-synthetic/messages:send" => {
                        "/send"
                    }
                    _ => panic!("unexpected provider destination"),
                };
                let (mut parts, body) = request.into_parts();
                parts.uri = fixture.uri("chat.example", path).parse().unwrap();
                fixture.send(ureq::http::Request::from_parts(parts, body))
            },
        )
    };
    assert_eq!(perform(&mut provider).outcome, Outcome::Accepted);
    assert_eq!(seen.lock().unwrap().len(), 1);
    let unauthorized = perform(&mut provider);
    assert!(matches!(
        unauthorized.outcome,
        Outcome::Retry {
            refresh_auth: true,
            ..
        }
    ));
    assert!(provider.token.is_none());
    assert_eq!(seen.lock().unwrap().len(), 1);
    let limited = perform(&mut provider);
    assert!(limited.global_backoff);
    assert_eq!(
        limited.outcome,
        Outcome::Retry {
            not_before: NOW + 122,
            refresh_auth: false
        }
    );
    assert_eq!(seen.lock().unwrap().len(), 2);
    store.finish_push(&job, limited, NOW + 3).unwrap();
    assert!(store.claim_push(NOW + 121).unwrap().is_none());
}

#[tokio::test]
async fn push_routes_enforce_admin_device_origin_and_body_boundaries() {
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    let (store, alice, _) = setup(&path);
    let now = crate::enrollment::now().unwrap();
    store
        .0
        .execute(
            "UPDATE devices SET expires_at=?1",
            [sql(now + 86400).unwrap()],
        )
        .unwrap();
    let token_path = dir.path().join("admin.token");
    let admin = crate::auth::AdminToken::load_or_create(&token_path).unwrap();
    let admin_text = std::fs::read_to_string(token_path).unwrap();
    let app = crate::router(store, admin);
    let request =
        |method: &str, path: &str, credential: Option<&str>, body: String, origin: bool| {
            let mut builder = Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json");
            if let Some(token) = credential {
                builder = builder.header("authorization", format!("Bearer {token}"));
            }
            if origin {
                builder = builder.header("origin", "https://untrusted.example");
            }
            builder.body(Body::from(body)).unwrap()
        };
    for method in ["GET", "PUT"] {
        assert_eq!(
            app.clone()
                .oneshot(request(method, "/admin/v0/push", None, "{}".into(), false))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        app.clone()
            .oneshot(request(
                "GET",
                "/admin/v0/push",
                Some(&alice),
                String::new(),
                false
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let public = app
        .clone()
        .oneshot(request(
            "GET",
            "/admin/v0/push",
            Some(&admin_text),
            String::new(),
            false,
        ))
        .await
        .unwrap();
    assert_eq!(public.status(), StatusCode::OK);
    assert_eq!(public.headers()["cache-control"], "no-store");
    let body = to_bytes(public.into_body(), 65536).await.unwrap();
    let text = std::str::from_utf8(&body).unwrap();
    assert!(!text.contains("PRIVATE KEY"));
    assert!(!text.contains("private_key"));
    assert_eq!(
        app.clone()
            .oneshot(request(
                "GET",
                "/client/v0/push",
                None,
                String::new(),
                false
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.clone()
            .oneshot(request(
                "PUT",
                "/client/v0/push",
                Some(&alice),
                serde_json::to_string(&registration(0)).unwrap(),
                true
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let response = app
        .clone()
        .oneshot(request(
            "PUT",
            "/client/v0/push",
            Some(&alice),
            serde_json::to_string(&registration(0)).unwrap(),
            false,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let status: sigil_protocol::push::Status =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    assert_eq!(status.state, State::Pending);
    assert_eq!(
        app.clone()
            .oneshot(request(
                "PUT",
                "/client/v0/push",
                Some(&alice),
                "x".repeat(8193),
                false
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
                "/admin/v0/push",
                Some(&admin_text),
                "x".repeat(push_config::MAX_BODY + 1),
                false
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
}
#[test]
fn registration_confirmation_replacement_and_cancel_are_revisioned_and_durable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    let (mut store, alice, bob) = setup(&path);
    let pending = store.register_push(&alice, registration(0), NOW).unwrap();
    assert_eq!(pending.state, State::Pending);
    assert_eq!(
        store.register_push(&alice, registration(0), NOW).unwrap(),
        pending
    );
    assert!(matches!(
        store.register_push(&alice, registration(1), NOW + 1),
        Err(StoreError::Busy)
    ));
    let job = store.claim_push(NOW).unwrap().unwrap();
    assert!(store.claim_push(NOW).unwrap().is_none());
    assert!(store.confirm_push(&bob, confirm(&job), NOW).is_err());
    let active = store.confirm_push(&alice, confirm(&job), NOW).unwrap();
    assert_eq!(active.state, State::Active);
    assert_eq!(
        store.confirm_push(&alice, confirm(&job), NOW + 1).unwrap(),
        active
    );
    assert!(!store
        .finish_push(
            &job,
            Delivery {
                outcome: Outcome::InvalidRegistration,
                global_backoff: false
            },
            NOW + 1
        )
        .unwrap());
    drop(store);
    let mut store = Store::open(&path).unwrap();
    assert_eq!(store.push_status(&alice, NOW + 2).unwrap(), active);
    let replacement = store
        .register_push(&alice, registration(1), NOW + 60)
        .unwrap();
    assert_ne!(replacement.channel, pending.channel);
    assert!(store.confirm_push(&alice, confirm(&job), NOW + 60).is_err());
    let disable = Disable {
        expected_revision: 2,
    };
    let disabled = store
        .disable_push(&alice, disable.clone(), NOW + 61)
        .unwrap();
    assert_eq!(
        store.disable_push(&alice, disable, NOW + 62).unwrap(),
        disabled
    );
    assert!(store
        .register_push(&alice, registration(1), NOW + 120)
        .is_err());
    assert!(store.claim_push(NOW + 120).unwrap().is_none());
    let secret_rows:i64=store.0.query_row("SELECT count(*) FROM push_channels WHERE target IS NOT NULL OR proof IS NOT NULL OR proof_hash IS NOT NULL",[],|r|r.get(0)).unwrap();
    assert_eq!(secret_rows, 0);
}
