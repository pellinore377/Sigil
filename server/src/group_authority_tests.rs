use super::*;
use sigil_crypto::{
    private_credentials::{Credential, GroupKey},
    private_group::{authority_fingerprint, Authority},
    IdentityKey, Secret32,
};
use sigil_protocol::{
    accounts::{Enrollment, InviteRequest},
    groups::{Configure, CredentialRequest},
};

const NOW: u64 = 1_800_000_000;
struct Actor {
    token: String,
    credential: Credential,
    member: Member,
}
struct Fixture {
    dir: tempfile::TempDir,
    store: Store,
    profile: Authority,
    fingerprint: [u8; 32],
    key: GroupKey,
    alice: Actor,
    bob: Actor,
}
fn actor(store: &mut Store, profile: &Authority, key: &GroupKey, name: &str, admin: bool) -> Actor {
    let token = crate::auth::random_secret().unwrap();
    let invite = store
        .invite(
            InviteRequest {
                username: name.into(),
                expires_in_seconds: 60,
            },
            NOW,
        )
        .unwrap();
    let own = store
        .enroll(
            Enrollment {
                invitation: invite.secret,
                device_credential: token.clone(),
                device_label: "Synthetic group test".into(),
            },
            NOW,
        )
        .unwrap();
    let identity = IdentityKey::generate().unwrap();
    let binding = sigil_protocol::device::Binding {
        server: "chat.example".into(),
        username: name.into(),
        account: id(&own.account_id).unwrap(),
        device: id(&own.device_id).unwrap(),
        identity: identity.public_key(),
    };
    let signature = identity.sign(&binding.signing_bytes().unwrap()).unwrap();
    let binding = sigil_protocol::device::SignedBinding { binding, signature }
        .to_bytes()
        .unwrap();
    store
        .publish_device_binding(
            &token,
            sigil_protocol::device::Statement {
                statement: hex(&binding),
            },
            NOW,
        )
        .unwrap();
    let issuance = profile.issuance(&binding, (NOW / 86400) as u32).unwrap();
    let response = store
        .issue_group_credential(
            &token,
            CredentialRequest {
                authority: hex(&profile.id()),
                day: issuance.day,
            },
            NOW,
        )
        .unwrap();
    assert_eq!(response.uid, hex(&issuance.uid));
    let credential = Credential::accept(
        profile.issuer(),
        issuance.attributes().unwrap(),
        issuance.day,
        issuance.context(),
        &decode(&response.response, 352).unwrap(),
    )
    .unwrap();
    let member = Member {
        ciphertext: hex(&key.ciphertext(&issuance.attributes().unwrap())),
        admin,
    };
    Actor {
        token,
        credential,
        member,
    }
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("groups.db")).unwrap();
        store
            .configure(sigil_protocol::Configure {
                expected_revision: 0,
                settings: serde_json::from_str(r#"{"server_name":"chat.example"}"#).unwrap(),
            })
            .unwrap();
        store
            .configure_groups(Configure {
                expected_revision: 0,
                enabled: true,
                storage_limit_bytes: 1024 * 1024,
            })
            .unwrap();
        let stored = authority::read(&store.0).unwrap();
        let fingerprint = authority_fingerprint(&stored.signing.unwrap().public_key());
        let profile = stored.authority.unwrap();
        let key = GroupKey::from_master(Secret32::from_bytes([1; 32])).unwrap();
        let alice = actor(&mut store, &profile, &key, "alice", true);
        let bob = actor(&mut store, &profile, &key, "bob", false);
        Self {
            dir,
            store,
            profile,
            fingerprint,
            key,
            alice,
            bob,
        }
    }
    fn request(&self, actor: &Actor, operation: Operation, nonce: u8) -> Request {
        let (kind, predecessor) = match &operation {
            Operation::Create { .. } => (Kind::Create, [0; 32]),
            Operation::Read { .. } => (Kind::Read, [0; 32]),
            Operation::Advance { predecessor, .. } => (Kind::Advance, id(predecessor).unwrap()),
        };
        let context = Context {
            operation: kind,
            group: [2; 32],
            predecessor,
            body_hash: Sha256::digest(serde_json::to_vec(&operation).unwrap()).into(),
            nonce: [nonce; 32],
            expires_at: NOW + 60,
        }
        .context(&self.profile, NOW)
        .unwrap();
        Request {
            authority: hex(&self.profile.id()),
            group: hex(&[2; 32]),
            day: (NOW / 86400) as u32,
            nonce: hex(&[nonce; 32]),
            expires_at: NOW + 60,
            operation,
            proof: hex(&actor.credential.present(&self.key, &context).unwrap()),
        }
    }
    fn create(&self) -> Operation {
        Operation::Create {
            public: hex(&self.key.public()),
            head: hex(&[3; 32]),
            control: hex(b"synthetic opaque genesis"),
            members: vec![self.alice.member.clone()],
        }
    }
    fn advance(&self, revision: u64, predecessor: u8, head: u8, include_bob: bool) -> Operation {
        let mut members = vec![self.alice.member.clone()];
        if include_bob {
            members.push(self.bob.member.clone());
        }
        members.sort_by(|a, b| a.ciphertext.cmp(&b.ciphertext));
        Operation::Advance {
            predecessor: hex(&[predecessor; 32]),
            head: hex(&[head; 32]),
            revision,
            control: hex(b"synthetic opaque control"),
            members,
        }
    }
    fn start(&mut self) {
        let request = self.request(&self.alice, self.create(), 1);
        self.store.group_request(request, NOW).unwrap();
    }
}

#[test]
fn group_authority_orders_updates_and_preserves_lost_receipts_after_removal_and_restart() {
    let mut f = Fixture::new();
    f.start();
    let request = f.request(&f.bob, Operation::Read { from_revision: 0 }, 2);
    assert!(matches!(
        f.store.group_request(request, NOW),
        Err(StoreError::Forbidden)
    ));
    let mut add = f.advance(1, 3, 4, true);
    if let Operation::Advance { members, .. } = &mut add {
        for member in members {
            member.admin = true;
        }
    }
    let request = f.request(&f.alice, add.clone(), 3);
    let receipt = f.store.group_request(request.clone(), NOW).unwrap().commits[0]
        .receipt
        .clone()
        .unwrap();
    let checked = Receipt::from_bytes(&decode(&receipt, 208).unwrap(), f.fingerprint).unwrap();
    assert_eq!(checked.head, [4; 32]);
    assert!(matches!(
        f.store.group_request(request, NOW),
        Err(StoreError::Conflict)
    ));
    let request = f.request(&f.alice, f.advance(1, 3, 5, true), 4);
    assert!(matches!(
        f.store.group_request(request, NOW),
        Err(StoreError::Conflict)
    ));
    let request = f.request(&f.bob, Operation::Read { from_revision: 0 }, 5);
    assert_eq!(
        f.store.group_request(request, NOW).unwrap().commits.len(),
        2
    );
    let leave = f.advance(2, 4, 5, false);
    let request = f.request(&f.bob, leave.clone(), 6);
    let lost = f.store.group_request(request, NOW).unwrap();
    drop(f.store);
    f.store = Store::open(&f.dir.path().join("groups.db")).unwrap();
    assert_eq!(f.store.group_authority().unwrap(), f.profile.to_bytes());
    let request = f.request(&f.bob, leave, 7);
    let retry = f.store.group_request(request, NOW).unwrap();
    assert_eq!(retry.commits[0].receipt, lost.commits[0].receipt);
    let request = f.request(&f.bob, Operation::Read { from_revision: 2 }, 8);
    assert!(matches!(
        f.store.group_request(request, NOW),
        Err(StoreError::Forbidden)
    ));
    let request = f.request(&f.alice, add, 9);
    assert_eq!(
        f.store.group_request(request, NOW).unwrap().commits[0]
            .receipt
            .as_ref(),
        Some(&receipt)
    );
}

#[test]
fn group_authority_rejects_escalation_tampering_and_stale_credentials() {
    let mut f = Fixture::new();
    f.start();
    let request = f.request(&f.alice, f.advance(1, 3, 4, true), 2);
    f.store.group_request(request, NOW).unwrap();
    let request = f.request(&f.bob, f.advance(2, 4, 5, true), 20);
    assert!(matches!(
        f.store.group_request(request, NOW),
        Err(StoreError::Forbidden)
    ));
    let request = f.request(&f.bob, f.advance(2, 4, 5, false), 21);
    assert!(matches!(
        f.store.group_request(request, NOW),
        Err(StoreError::Forbidden)
    ));
    let mut operation = f.advance(2, 4, 5, true);
    if let Operation::Advance { members, .. } = &mut operation {
        for member in members {
            member.admin = true;
        }
    }
    let request = f.request(&f.bob, operation, 3);
    assert!(matches!(
        f.store.group_request(request, NOW),
        Err(StoreError::Forbidden)
    ));
    let mut request = f.request(&f.alice, Operation::Read { from_revision: 0 }, 4);
    request.operation = Operation::Read { from_revision: 1 };
    assert!(matches!(
        f.store.group_request(request, NOW),
        Err(StoreError::Unauthorized)
    ));
    let request = f.request(&f.alice, Operation::Read { from_revision: 0 }, 5);
    assert!(f.store.group_request(request, NOW + 86400).is_err());
    assert!(f
        .store
        .issue_group_credential(
            &f.alice.token,
            CredentialRequest {
                authority: hex(&f.profile.id()),
                day: (NOW / 86400 + 1) as u32
            },
            NOW
        )
        .is_err());
    assert!(matches!(
        f.store.issue_group_credential(
            "invalid",
            CredentialRequest {
                authority: hex(&f.profile.id()),
                day: (NOW / 86400) as u32
            },
            NOW
        ),
        Err(StoreError::Unauthorized)
    ));
}

#[test]
fn group_authority_failed_writes_roll_back_head_roster_nonce_and_budget() {
    let mut f = Fixture::new();
    f.start();
    let before = f.store.group_configuration().unwrap().used_bytes;
    let request = f.request(&f.alice, f.advance(1, 3, 4, true), 2);
    f.store.0.execute_batch("CREATE TRIGGER synthetic_failure BEFORE INSERT ON private_group_nonces BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(f.store.group_request(request.clone(), NOW).is_err());
    assert_eq!(f.store.group_configuration().unwrap().used_bytes, before);
    assert_eq!(
        f.store
            .0
            .query_row("SELECT revision FROM private_groups", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        f.store
            .0
            .query_row("SELECT count(*) FROM private_group_members", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    f.store
        .0
        .execute_batch("DROP TRIGGER synthetic_failure; UPDATE group_authority SET quota=used;")
        .unwrap();
    assert!(matches!(
        f.store.group_request(request.clone(), NOW),
        Err(StoreError::Busy)
    ));
    assert_eq!(f.store.group_configuration().unwrap().used_bytes, before);
    f.store
        .0
        .execute_batch("UPDATE group_authority SET quota=1048576")
        .unwrap();
    assert_eq!(f.store.group_request(request, NOW).unwrap().revision, 1);
}

#[test]
fn group_authority_restore_disables_service_and_never_signs_a_rewound_head() {
    let mut f = Fixture::new();
    f.start();
    let backup = f.dir.path().join("backup.db");
    f.store.backup(&backup).unwrap();
    let target = f.dir.path().join("restored.db");
    Store::restore(&backup, &target).unwrap();
    let mut restored = Store::open(&target).unwrap();
    assert!(matches!(
        restored.group_authority(),
        Err(StoreError::Forbidden)
    ));
    let config = restored.group_configuration().unwrap();
    restored
        .configure_groups(Configure {
            expected_revision: config.revision,
            enabled: true,
            storage_limit_bytes: config.storage_limit_bytes,
        })
        .unwrap();
    let request = f.request(&f.alice, Operation::Read { from_revision: 0 }, 2);
    assert!(restored.group_request(request, NOW).unwrap().restored);
    let request = f.request(&f.alice, f.advance(1, 3, 4, true), 3);
    assert!(matches!(
        restored.group_request(request, NOW),
        Err(StoreError::Conflict)
    ));
}

#[test]
fn group_authority_competing_writers_receive_only_one_commit() {
    let mut f = Fixture::new();
    f.start();
    let a = f.request(&f.alice, f.advance(1, 3, 4, true), 2);
    let b = f.request(&f.alice, f.advance(1, 3, 5, false), 3);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let jobs = [a, b].map(|request| {
        let barrier = barrier.clone();
        let path = f.dir.path().join("groups.db");
        std::thread::spawn(move || {
            let mut store = Store::open(&path).unwrap();
            barrier.wait();
            store.group_request(request, NOW).is_ok()
        })
    });
    assert_eq!(
        jobs.into_iter()
            .map(|j| usize::from(j.join().unwrap()))
            .sum::<usize>(),
        1
    );
}

#[tokio::test]
async fn group_authority_routes_separate_admin_device_and_anonymous_access() {
    use axum::{
        body::Body,
        http::{Request as HttpRequest, StatusCode},
    };
    use tower::ServiceExt;
    let f = Fixture::new();
    let admin_path = f.dir.path().join("admin.token");
    let admin = crate::auth::AdminToken::load_or_create(&admin_path).unwrap();
    let token = std::fs::read_to_string(&admin_path).unwrap();
    let app = crate::router(f.store, admin);
    for (path, header, expected) in [
        ("/groups/v0/authority", None, StatusCode::OK),
        (
            "/groups/v0/authority",
            Some(("authorization", "Bearer synthetic")),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "/groups/v0/authority",
            Some(("cookie", "synthetic=1")),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "/groups/v0/authority",
            Some(("origin", "https://chat.example")),
            StatusCode::FORBIDDEN,
        ),
        (
            "/groups/v0/authority?account=synthetic",
            None,
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        ("/admin/v0/groups", None, StatusCode::UNAUTHORIZED),
    ] {
        let mut request = HttpRequest::get(path);
        if let Some((name, value)) = header {
            request = request.header(name, value);
        }
        assert_eq!(
            app.clone()
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
            expected
        );
    }
    let response = app
        .clone()
        .oneshot(
            HttpRequest::get("/admin/v0/groups")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = app
        .clone()
        .oneshot(
            HttpRequest::post("/client/v0/groups/credential")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&CredentialRequest {
                        authority: hex(&f.profile.id()),
                        day: (NOW / 86400) as u32,
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = app
        .oneshot(
            HttpRequest::post("/groups/v0/request")
                .header("content-type", "application/json")
                .body(Body::from(vec![b'a'; sigil_protocol::groups::MAX_BODY + 1]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[test]
fn group_authority_configuration_and_uid_collision_checks_are_durable() {
    let mut f = Fixture::new();
    let config = f.store.group_configuration().unwrap();
    let retry = f
        .store
        .configure_groups(Configure {
            expected_revision: 0,
            enabled: true,
            storage_limit_bytes: 1024 * 1024,
        })
        .unwrap();
    assert_eq!(retry.revision, config.revision);
    assert_eq!(retry.authority, config.authority);
    let credential = CredentialRequest {
        authority: hex(&f.profile.id()),
        day: (NOW / 86400) as u32,
    };
    f.store
        .issue_group_credential(&f.alice.token, credential.clone(), NOW)
        .unwrap();
    assert_eq!(
        f.store.group_configuration().unwrap().used_bytes,
        config.used_bytes
    );
    // Simulate the UID truncation collision without weakening cryptographic hashes.
    let own = f.store.session(&f.alice.token, NOW).unwrap();
    let binding: Vec<u8> = f
        .store
        .0
        .query_row(
            "SELECT statement FROM device_bindings WHERE device=?1",
            [own.device_id],
            |r| r.get(0),
        )
        .unwrap();
    let issuance = f.profile.issuance(&binding, credential.day).unwrap();
    f.store
        .0
        .execute(
            "UPDATE group_credential_uids SET fingerprint=?1 WHERE uid=?2",
            ([99_u8; 32].as_slice(), issuance.uid.as_slice()),
        )
        .unwrap();
    assert!(matches!(
        f.store
            .issue_group_credential(&f.alice.token, credential.clone(), NOW),
        Err(StoreError::Conflict)
    ));
    assert_eq!(
        f.store.group_configuration().unwrap().used_bytes,
        config.used_bytes
    );
    let disabled = f
        .store
        .configure_groups(Configure {
            expected_revision: 1,
            enabled: false,
            storage_limit_bytes: 1024 * 1024,
        })
        .unwrap();
    assert!(matches!(
        f.store.group_authority(),
        Err(StoreError::Forbidden)
    ));
    assert!(matches!(
        f.store
            .issue_group_credential(&f.bob.token, credential, NOW),
        Err(StoreError::Forbidden)
    ));
    let enabled = f
        .store
        .configure_groups(Configure {
            expected_revision: disabled.revision,
            enabled: true,
            storage_limit_bytes: 1024 * 1024,
        })
        .unwrap();
    assert_eq!(enabled.authority, config.authority);
}
