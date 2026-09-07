use super::*;
use crate::network::tests::{Fixture, CA};
use sigil_crypto::Secret32;
use sigil_server::{
    auth::AdminToken,
    push_config::{Configure, FcmUpdate},
    push_provider::{FcmCredentials, Vapid},
    store::Store,
};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
fn open(path: &Path) -> ClientStore {
    ClientStore::open(
        path,
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap()
}
fn setup() -> (
    tempfile::TempDir,
    Fixture,
    ClientStore,
    u64,
    Arc<AtomicBool>,
) {
    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let now = crate::schedule::clock().unwrap();
    let mut server = Store::open(&dir.path().join("server.db")).unwrap();
    server
        .configure(sigil_protocol::Configure {
            expected_revision: 0,
            settings: sigil_protocol::Settings {
                server_name: "chat.example".into(),
                default_quota_bytes: 16 * 1024 * 1024,
                max_attachment_bytes: 1024 * 1024,
            },
        })
        .unwrap();
    server
        .configure_push(Configure {
            expected_revision: 0,
            unified_push: true,
            contact: Some("mailto:operator@example.com".into()),
            exceptions: Vec::new(),
            rotate_vapid: false,
            fcm: FcmUpdate::Configure(FcmCredentials {
                project_id: "sigil-synthetic".into(),
                client_email: "push@sigil-synthetic.iam.gserviceaccount.com".into(),
                private_key: Zeroizing::new(
                    include_str!("../../server/tests/fixtures/synthetic-fcm-key.pem").into(),
                ),
            }),
        })
        .unwrap();
    let invitation = server
        .invite(
            sigil_protocol::accounts::InviteRequest {
                username: "alice".into(),
                expires_in_seconds: 60,
            },
            now,
        )
        .unwrap();
    let limited = Arc::new(AtomicBool::new(false));
    let flag = limited.clone();
    let app = sigil_server::router(
        server,
        AdminToken::load_or_create(&dir.path().join("admin.token")).unwrap(),
    )
    .layer(axum::middleware::from_fn(
        move |request: axum::extract::Request, next: axum::middleware::Next| {
            let flag = flag.clone();
            async move {
                if flag.load(Ordering::SeqCst) && request.uri().path() == "/client/v0/push" {
                    use axum::response::IntoResponse;
                    (
                        axum::http::StatusCode::TOO_MANY_REQUESTS,
                        [("retry-after", "99")],
                    )
                        .into_response()
                } else {
                    next.run(request).await
                }
            }
        },
    ));
    let fixture = Fixture::new(app);
    let mut client = open(&dir.path().join("client.db"));
    client
        .prepare_enrollment(
            "chat.example",
            fixture.port(),
            &[CA.to_vec()],
            &invitation.secret,
            "Synthetic",
            false,
        )
        .unwrap();
    client.enroll_online().unwrap();
    (dir, fixture, client, now, limited)
}
fn server_push(dir: &tempfile::TempDir, client: &ClientStore) -> (Vec<u8>, Target) {
    let db = Connection::open(dir.path().join("server.db")).unwrap();
    let device = client.connection_session().unwrap().unwrap().device_id;
    let (channel, proof, target): (String, String, String) = db
        .query_row(
            "SELECT channel,proof,target FROM push_channels WHERE device=?1",
            [device],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    (
        Payload::Challenge {
            channel: &decode_id(&channel).unwrap(),
            proof: &decode_id(&proof).unwrap(),
        }
        .to_bytes(),
        serde_json::from_str(&target).unwrap(),
    )
}
fn register(client: &mut ClientStore, now: u64) {
    assert!(matches!(
        client.push_step(now).unwrap(),
        Progress::Reconciled
    ));
    assert!(
        matches!(client.push_step(now).unwrap(),Progress::Updated(s) if s.state==RemoteState::Pending)
    );
}
fn activate(dir: &tempfile::TempDir, client: &mut ClientStore, now: u64) {
    client.set_fcm_push_token("synthetic-token", now).unwrap();
    register(client, now);
    let (bytes, _) = server_push(dir, client);
    assert_eq!(
        client
            .receive_fcm_push(&B64::encode_string(&bytes), now)
            .unwrap(),
        ReceivedHint::ConfirmationQueued
    );
    assert!(
        matches!(client.push_step(now).unwrap(),Progress::Updated(s) if s.state==RemoteState::Active)
    );
}

#[test]
fn own_channel_proof_is_durable_and_disabling_cannot_confirm_an_old_target() {
    let (dir, _fixture, mut client, now, _) = setup();
    client.set_fcm_push_token("synthetic-token", now).unwrap();
    register(&mut client, now);
    let (bytes, _) = server_push(&dir, &client);
    let mut wrong = bytes.clone();
    wrong[9] ^= 1;
    assert_eq!(
        client
            .receive_fcm_push(&B64::encode_string(&wrong), now)
            .unwrap(),
        ReceivedHint::Ignored
    );
    client.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON push_state BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(client
        .receive_fcm_push(&B64::encode_string(&bytes), now)
        .is_err());
    assert!(!client.push_state().unwrap().pending);
    client.db.execute_batch("DROP TRIGGER fail").unwrap();
    assert_eq!(
        client
            .receive_fcm_push(&B64::encode_string(&bytes), now)
            .unwrap(),
        ReceivedHint::ConfirmationQueued
    );
    let sealed: Vec<u8> = client
        .db
        .query_row("SELECT state FROM push_state", [], |r| r.get(0))
        .unwrap();
    assert!(!sealed
        .windows(b"synthetic-token".len())
        .any(|w| w == b"synthetic-token"));
    assert!(!sealed.windows(32).any(|w| w == &bytes[41..73]));
    drop(client);
    let mut client = open(&dir.path().join("client.db"));
    assert_eq!(
        client
            .receive_fcm_push(&B64::encode_string(&bytes), now)
            .unwrap(),
        ReceivedHint::ConfirmationQueued
    );
    client.disable_push(now).unwrap();
    assert_eq!(
        client
            .receive_fcm_push(&B64::encode_string(&bytes), now)
            .unwrap(),
        ReceivedHint::Ignored
    );
    assert!(
        matches!(client.push_step(now).unwrap(),Progress::Updated(s) if s.state==RemoteState::Disabled&&s.revision==2)
    );
    assert!(matches!(client.push_step(now).unwrap(), Progress::Idle));
    let db = Connection::open(dir.path().join("server.db")).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM push_channels WHERE target IS NOT NULL OR proof IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn ambiguous_registration_retries_exactly_and_early_challenges_grant_no_authority() {
    let (dir, _fixture, mut client, now, _) = setup();
    client.set_fcm_push_token("synthetic-old", now).unwrap();
    client.push_step(now).unwrap();
    client.db.execute_batch("CREATE TABLE synthetic_writes(n INTEGER);INSERT INTO synthetic_writes VALUES(0);CREATE TRIGGER count_write AFTER UPDATE ON push_state BEGIN UPDATE synthetic_writes SET n=n+1;END;CREATE TRIGGER fail BEFORE UPDATE ON push_state WHEN (SELECT n FROM synthetic_writes)>=1 BEGIN SELECT RAISE(ABORT,'synthetic');END;").unwrap();
    assert!(client.push_step(now).is_err());
    let (bytes, target) = server_push(&dir, &client);
    assert!(matches!(target,Target::Fcm{token} if token=="synthetic-old"));
    assert_eq!(
        client
            .receive_fcm_push(&B64::encode_string(&bytes), now)
            .unwrap(),
        ReceivedHint::Ignored
    );
    client
        .db
        .execute_batch("DROP TRIGGER fail;DROP TRIGGER count_write;DROP TABLE synthetic_writes;")
        .unwrap();
    drop(client);
    let mut client = open(&dir.path().join("client.db"));
    client.set_fcm_push_token("synthetic-new", now).unwrap();
    assert!(
        matches!(client.push_step(now).unwrap(),Progress::Updated(s) if s.revision==1&&s.state==RemoteState::Pending)
    );
    assert_eq!(server_push(&dir, &client).0, bytes);
    assert_eq!(
        client
            .receive_fcm_push(&B64::encode_string(&bytes), now)
            .unwrap(),
        ReceivedHint::Ignored
    );
    // Advance only the synthetic server's replacement cooldown for this race test.
    let db = Connection::open(dir.path().join("server.db")).unwrap();
    db.execute(
        "UPDATE push_channels SET last_change=?1",
        [(now - 60) as i64],
    )
    .unwrap();
    assert!(matches!(client.push_step(now).unwrap(),Progress::Updated(s) if s.revision==2));
    let (new, target) = server_push(&dir, &client);
    assert_ne!(new, bytes);
    assert!(matches!(target,Target::Fcm{token} if token=="synthetic-new"));
}

#[test]
fn unifiedpush_uses_persisted_rust_keys_and_ignores_obsolete_connectors() {
    let (dir, _fixture, mut client, now, _) = setup();
    let providers = client.connected_client().unwrap().push_providers().unwrap();
    let vapid_key = providers.vapid_public_key.unwrap();
    let connector = client.prepare_unified_push(&vapid_key, false, now).unwrap();
    let same = client.prepare_unified_push(&vapid_key, false, now).unwrap();
    assert_eq!(same.connection, connector.connection);
    client
        .set_unified_push_endpoint(&connector.connection, "https://push.example/synthetic", now)
        .unwrap();
    register(&mut client, now);
    let (bytes, target) = server_push(&dir, &client);
    let db = Connection::open(dir.path().join("server.db")).unwrap();
    let settings: String = db
        .query_row("SELECT settings FROM push_configuration", [], |r| r.get(0))
        .unwrap();
    let settings: serde_json::Value = serde_json::from_str(&settings).unwrap();
    let key: Vec<u8> = serde_json::from_value(settings["vapid"].clone()).unwrap();
    let vapid = Vapid::from_pkcs8(&key).unwrap();
    let request = vapid
        .request(
            &target,
            "mailto:operator@example.com",
            &Payload::from_bytes(&bytes).unwrap(),
            now,
            600,
        )
        .unwrap();
    let encrypted = request.into_body();
    for end in [0, 20, 21, 85, 86, 102, encrypted.len() - 1] {
        assert!(client
            .receive_unified_push(&connector.connection, &encrypted[..end], now)
            .is_err());
    }
    let mut malformed = encrypted.to_vec();
    malformed[16..20].fill(0);
    assert!(client
        .receive_unified_push(&connector.connection, &malformed, now)
        .is_err());
    drop(client);
    let mut client = open(&dir.path().join("client.db"));
    assert_eq!(
        client
            .receive_unified_push(&connector.connection, &encrypted, now)
            .unwrap(),
        ReceivedHint::ConfirmationQueued
    );
    assert!(
        matches!(client.push_step(now).unwrap(),Progress::Updated(s) if s.state==RemoteState::Active)
    );
    let wake = vapid
        .request(
            &target,
            "mailto:operator@example.com",
            &Payload::Wake,
            now,
            600,
        )
        .unwrap()
        .into_body();
    assert_eq!(
        client
            .receive_unified_push(&connector.connection, &wake, now)
            .unwrap(),
        ReceivedHint::Wake
    );
    let replacement = client.prepare_unified_push(&vapid_key, true, now).unwrap();
    assert_ne!(replacement.connection, connector.connection);
    assert_eq!(
        client
            .receive_unified_push(&connector.connection, &encrypted, now)
            .unwrap(),
        ReceivedHint::Ignored
    );
    assert!(matches!(
        client.set_unified_push_endpoint(&connector.connection, "https://push.example/old", now),
        Err(Error::Obsolete)
    ));
    assert!(client.push_state().unwrap().awaiting_endpoint);
}

#[test]
fn push_scheduler_preserves_retry_after_and_new_preferences_survive_old_completion() {
    let (dir, _fixture, mut client, now, limited) = setup();
    client.set_fcm_push_token("synthetic-token", now).unwrap();
    limited.store(true, Ordering::SeqCst);
    let mut times = [now, now + 7].into_iter();
    let failed = client
        .push_with_clock(|| Ok(times.next().unwrap()))
        .unwrap();
    assert!(matches!(
        failed.progress,
        Some(Err(Error::Network(network::Error::Status {
            code: 429,
            ..
        })))
    ));
    assert_eq!(failed.next_at, now + 106);
    client.set_fcm_push_token("synthetic-new", now + 8).unwrap();
    drop(client);
    let mut client = open(&dir.path().join("client.db"));
    let deferred = client.push_with_clock(|| Ok(now + 105)).unwrap();
    assert!(deferred.progress.is_none());
    assert_eq!(deferred.next_at, now + 106);
    limited.store(false, Ordering::SeqCst);
    let resumed = client.push_with_clock(|| Ok(now + 106)).unwrap();
    assert!(matches!(resumed.progress, Some(Ok(Progress::Reconciled))));
    // A separate store changes intent between successful work and schedule completion.
    let mut other = open(&dir.path().join("client.db"));
    let mut call = 0;
    let changed = client
        .push_with_clock(|| {
            call += 1;
            if call == 1 {
                Ok(now + 107)
            } else {
                other.disable_push(now + 108)?;
                Ok(now + 108)
            }
        })
        .unwrap();
    assert!(changed.scheduling_error.is_none());
    assert_eq!(changed.next_at, now + 109);
    assert_eq!(other.push_state().unwrap().choice, Choice::Disabled);
}

#[test]
fn scheduled_completion_failure_keeps_progress_and_scope_rebinding_fails() {
    let (dir, fixture, mut client, now, _) = setup();
    activate(&dir, &mut client, now);
    let db = Connection::open(dir.path().join("client.db")).unwrap();
    let mut calls = 0;
    let result=client.push_with_clock(||{calls+=1;if calls==2{db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON sync_schedule WHEN OLD.id=3 BEGIN SELECT RAISE(ABORT,'synthetic');END;")?;}Ok(now)}).unwrap();
    assert!(matches!(result.progress, Some(Ok(Progress::Idle))));
    assert!(result.scheduling_error.is_some());
    assert_eq!(result.next_at, now + 60);
    db.execute_batch("DROP TRIGGER fail").unwrap();
    let sealed: Vec<u8> = client
        .db
        .query_row("SELECT state FROM push_state", [], |r| r.get(0))
        .unwrap();
    let account = client.connection_session().unwrap().unwrap().account_id;
    let mut server = Store::open(&dir.path().join("server.db")).unwrap();
    let invite = server.invite_reauthorization(&account, 60, now).unwrap();
    let mut fresh = open(&dir.path().join("fresh.db"));
    fresh
        .prepare_enrollment(
            "chat.example",
            fixture.port(),
            &[CA.to_vec()],
            &invite.secret,
            "Fresh",
            true,
        )
        .unwrap();
    fresh.enroll_online().unwrap();
    assert!(!fresh.push_state().unwrap().configured);
    fresh
        .db
        .execute("INSERT INTO push_state VALUES(1,?1)", [sealed])
        .unwrap();
    assert!(matches!(fresh.push_state(), Err(Error::Crypto(_))));
    crate::test_schema::rewind(&client.db, 48);
    drop(client);
    let client = open(&dir.path().join("client.db"));
    assert!(!client.push_state().unwrap().configured);
    assert_eq!(
        client
            .db
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        55
    );
}
