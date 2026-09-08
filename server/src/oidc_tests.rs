use super::*;
use axum::{
    body::Bytes,
    routing::{get, post},
    Json, Router,
};
use base64ct::{Base64, Base64UrlUnpadded as B64, Encoding};
use ring::{rand::SystemRandom, signature};
use sha2::Digest;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

struct Idp {
    fixture: crate::egress::tests::Fixture,
    claims: Arc<Mutex<serde_json::Value>>,
    requests: Arc<AtomicUsize>,
}
impl Idp {
    fn new() -> Self {
        Self::with_post(false)
    }
    fn with_post(body_auth: bool) -> Self {
        let pem = include_str!("../tests/fixtures/synthetic-fcm-key.pem");
        let encoded = pem
            .lines()
            .filter(|s| !s.starts_with("-----"))
            .collect::<String>();
        let key = Arc::new(
            signature::RsaKeyPair::from_pkcs8(&Base64::decode_vec(&encoded).unwrap()).unwrap(),
        );
        let modulus = std::process::Command::new("openssl")
            .args([
                "rsa",
                "-in",
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/fixtures/synthetic-fcm-key.pem"
                ),
                "-noout",
                "-modulus",
            ])
            .output()
            .unwrap();
        assert!(modulus.status.success());
        let modulus = String::from_utf8(modulus.stdout).unwrap();
        let modulus = crate::group_authority::decode(
            &modulus
                .trim()
                .strip_prefix("Modulus=")
                .unwrap()
                .to_ascii_lowercase(),
            256,
        )
        .unwrap();
        let jwks = serde_json::json!({"keys":[{"kty":"RSA","kid":"synthetic","use":"sig","alg":"RS256","n":B64::encode_string(&modulus),"e":"AQAB"}]});
        let origin = Arc::new(Mutex::new(String::new()));
        let issuer = origin.clone();
        let claims = Arc::new(Mutex::new(serde_json::json!({})));
        let values = claims.clone();
        let requests = Arc::new(AtomicUsize::new(0));
        let count = requests.clone();
        let app=Router::new().route("/.well-known/openid-configuration",get(move || {let issuer=issuer.lock().unwrap().clone();async move {Json(serde_json::json!({"issuer":issuer,"authorization_endpoint":format!("{issuer}/authorize"),"token_endpoint":format!("{issuer}/token"),"jwks_uri":format!("{issuer}/jwks"),"response_types_supported":["code"],"subject_types_supported":["public"],"id_token_signing_alg_values_supported":["RS256"],"code_challenge_methods_supported":["S256"],"token_endpoint_auth_methods_supported":[if body_auth {"client_secret_post"} else {"client_secret_basic"}]}))}}))
            .route("/jwks",get(move ||{let jwks=jwks.clone();async move {Json(jwks)}}))
            .route("/token",post(move |headers:axum::http::HeaderMap,body:Bytes|{let values=values.clone();let key=key.clone();let count=count.clone();async move {
                count.fetch_add(1,Ordering::SeqCst);
                let form=openidconnect::url::form_urlencoded::parse(&body).collect::<std::collections::BTreeMap<_,_>>();
                assert_eq!(form["grant_type"],"authorization_code");assert_eq!(form["code"],"synthetic-code");if body_auth {assert!(headers.get("authorization").is_none());assert_eq!(form["client_id"],"sigil-synthetic");assert_eq!(form["client_secret"],"synthetic-secret");} else {assert_eq!(headers["authorization"],format!("Basic {}",Base64::encode_string(b"sigil-synthetic:synthetic-secret")));}
                let mut claims=values.lock().unwrap().clone();let challenge=claims.as_object_mut().unwrap().remove("challenge").unwrap();
                assert_eq!(B64::encode_string(&sha2::Sha256::digest(form["code_verifier"].as_bytes())),challenge.as_str().unwrap());
                let tamper=claims.as_object_mut().unwrap().remove("tamper").is_some();
                let header=B64::encode_string(br#"{"alg":"RS256","typ":"JWT","kid":"synthetic"}"#);
                let payload=B64::encode_string(&serde_json::to_vec(&claims).unwrap());let input=format!("{header}.{payload}");
                let mut signature=vec![0;key.public().modulus_len()];key.sign(&signature::RSA_PKCS1_SHA256,&SystemRandom::new(),input.as_bytes(),&mut signature).unwrap();if tamper {signature[0]^=1;}
                Json(serde_json::json!({"access_token":"synthetic-access","token_type":"Bearer","expires_in":600,"id_token":format!("{input}.{}",B64::encode_string(&signature))}))
            }}));
        let fixture = crate::egress::tests::Fixture::local(app);
        *origin.lock().unwrap() = fixture.uri("127.0.0.1", "");
        Self {
            fixture,
            claims,
            requests,
        }
    }
    fn configuration(&self) -> Configure {
        Configure {
            expected_revision: 0,
            confirm: true,
            provider: Some(Provider {
                issuer: self.fixture.uri("127.0.0.1", ""),
                client_id: "sigil-synthetic".into(),
                client_secret: Some(Zeroizing::new("synthetic-secret".into())),
                exceptions: vec![self.fixture.exception("127.0.0.1")],
            }),
        }
    }
    fn start(
        &self,
        store: &mut Store,
        link: Option<&str>,
        username: Option<&str>,
        replace: bool,
        now: u64,
    ) -> (Start, String) {
        let request = Start {
            request_id: random_secret().unwrap(),
            secret: random_secret().unwrap(),
            username: username.map(str::to_owned),
            replace_devices: replace,
        };
        let started = store.oidc_start(request.clone(), link, now).unwrap();
        let url = openidconnect::url::Url::parse(&started.authorization_url).unwrap();
        let params = url
            .query_pairs()
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(params["code_challenge_method"], "S256");
        assert_eq!(params["scope"], "openid");
        *self.claims.lock().unwrap() = serde_json::json!({"iss":self.fixture.uri("127.0.0.1",""),"sub":"synthetic-subject","aud":"sigil-synthetic","iat":now,"exp":now+600,"nonce":params["nonce"],"challenge":params["code_challenge"]});
        (request, params["state"].to_string())
    }
}
fn enable(store: &mut Store, idp: &Idp) {
    let mut policy = store.administration_policy().unwrap();
    policy.public_origin = Some("https://chat.example".into());
    policy.registration = sigil_protocol::admin::Registration::Oidc;
    store.configure_administration(policy).unwrap();
    let config = idp.configuration();
    let metadata = check(&config).unwrap();
    store.oidc_install(config, metadata).unwrap();
}
fn authenticate(store: &mut Store, state: &str, now: u64) -> Option<String> {
    let callback = store.oidc_claim(state, now).unwrap().unwrap();
    let result = callback.verify("synthetic-code");
    store
        .oidc_verified(callback, result, now)
        .unwrap()
        .map(|c| c.secret)
}
#[test]
fn browser_authorization_cannot_be_polled_by_a_different_client() {
    let _guard = crate::egress::tests::NETWORK.lock().unwrap();
    let idp = Idp::new();
    let (_dir, mut store, _, _, now) = crate::admin::tests::setup();
    enable(&mut store, &idp);
    let (request, state) = idp.start(&mut store, None, Some("carol"), false, now);
    let completion = authenticate(&mut store, &state, now);
    let finish = |completion| Finish {
        completion,
        request_id: request.request_id.clone(),
        secret: request.secret.clone(),
    };
    assert!(matches!(
        store.oidc_finish(finish(None), now).unwrap(),
        Progress::Pending
    ));
    assert!(store
        .oidc_finish(finish(Some("ab".repeat(32))), now)
        .is_err());
    assert!(store.oidc_claim(&state, now).unwrap().is_none());
    assert!(matches!(
        store.oidc_finish(finish(completion), now).unwrap(),
        Progress::Ready { .. }
    ));
}
#[test]
fn unlink_revokes_grants_and_pending_links_without_replay_rebinding() {
    let _guard = crate::egress::tests::NETWORK.lock().unwrap();
    let idp = Idp::with_post(true);
    let (_dir, mut store, alice, _, now) = crate::admin::tests::setup();
    enable(&mut store, &idp);
    let (link, state) = idp.start(&mut store, Some(&alice), None, false, now);
    let completion = authenticate(&mut store, &state, now);
    store
        .oidc_finish(
            Finish {
                completion: completion.clone(),
                request_id: link.request_id.clone(),
                secret: link.secret.clone(),
            },
            now,
        )
        .unwrap();
    let (pending, state) = idp.start(&mut store, Some(&alice), None, false, now);
    authenticate(&mut store, &state, now);
    let (login, state) = idp.start(&mut store, None, None, true, now);
    let completion = authenticate(&mut store, &state, now);
    store
        .oidc_finish(
            Finish {
                completion: completion.clone(),
                request_id: login.request_id,
                secret: login.secret.clone(),
            },
            now,
        )
        .unwrap();
    store
        .unlink_oidc(&alice, &idp.configuration().provider.unwrap().issuer, now)
        .unwrap();
    assert!(store.oidc_bindings(&alice, now).unwrap().is_empty());
    for request in [link, pending] {
        assert!(store
            .oidc_finish(
                Finish {
                    completion: completion.clone(),
                    request_id: request.request_id,
                    secret: request.secret
                },
                now
            )
            .is_err());
    }
    assert!(store
        .reauthorize(
            sigil_protocol::accounts::Enrollment {
                invitation: login.secret,
                device_credential: "cd".repeat(32),
                device_label: "Synthetic".into()
            },
            now
        )
        .is_err());
    assert!(store.session(&alice, now).is_ok());
}
#[test]
fn oidc_proof_link_login_and_recovery_never_inherit_device_identity() {
    let _guard = crate::egress::tests::NETWORK.lock().unwrap();
    let idp = Idp::new();
    let (dir, mut store, alice, _, now) = crate::admin::tests::setup();
    enable(&mut store, &idp);
    let before = store.session(&alice, now).unwrap();
    let (link, state) = idp.start(&mut store, Some(&alice), None, false, now);
    assert!(matches!(
        store
            .oidc_finish(
                Finish {
                    completion: None,
                    request_id: link.request_id.clone(),
                    secret: link.secret.clone()
                },
                now
            )
            .unwrap(),
        Progress::Pending
    ));
    let completion = authenticate(&mut store, &state, now);
    assert!(store
        .oidc_finish(
            Finish {
                completion: completion.clone(),
                request_id: link.request_id.clone(),
                secret: random_secret().unwrap()
            },
            now
        )
        .is_err());
    assert!(matches!(
        store
            .oidc_finish(
                Finish {
                    completion: completion.clone(),
                    request_id: link.request_id,
                    secret: link.secret
                },
                now
            )
            .unwrap(),
        Progress::Linked
    ));
    assert!(store.oidc_claim(&state, now).unwrap().is_none());
    assert_eq!(idp.requests.load(Ordering::SeqCst), 1);
    let (request, state) = idp.start(&mut store, None, None, false, now);
    let completion = authenticate(&mut store, &state, now);
    assert!(store
        .oidc_finish(
            Finish {
                completion: completion.clone(),
                request_id: request.request_id,
                secret: request.secret
            },
            now
        )
        .is_err());
    let (request, state) = idp.start(&mut store, None, None, true, now);
    let completion = authenticate(&mut store, &state, now);
    assert!(matches!(
        store
            .oidc_finish(
                Finish {
                    completion: completion.clone(),
                    request_id: request.request_id.clone(),
                    secret: request.secret.clone()
                },
                now
            )
            .unwrap(),
        Progress::Ready {
            reauthorize: true,
            ..
        }
    ));
    drop(store);
    let mut store = Store::open(&dir.path().join("sigil.db")).unwrap();
    let replacement = random_secret().unwrap();
    let session = store
        .reauthorize(
            sigil_protocol::accounts::Enrollment {
                invitation: request.secret,
                device_credential: replacement,
                device_label: "Recovered".into(),
            },
            now,
        )
        .unwrap();
    assert_eq!(before.account_id, session.account_id);
    assert_ne!(before.device_id, session.device_id);
    assert!(store.session(&alice, now).is_err());
    assert_eq!(
        store
            .0
            .query_row(
                "SELECT count(*) FROM device_bindings WHERE device=?1",
                [session.device_id],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
        0
    );
    assert!(!serde_json::to_string(&store.oidc_configuration().unwrap())
        .unwrap()
        .contains("synthetic-secret"));
}
#[test]
fn issuer_audience_nonce_signature_time_and_username_substitution_are_rejected() {
    let _guard = crate::egress::tests::NETWORK.lock().unwrap();
    let idp = Idp::new();
    let (_dir, mut store, _alice, _bob, now) = crate::admin::tests::setup();
    enable(&mut store, &idp);
    for (field, value) in [
        ("iss", serde_json::json!("https://wrong.example")),
        ("aud", serde_json::json!("wrong")),
        ("aud", serde_json::json!(["sigil-synthetic", "wrong"])),
        ("nonce", serde_json::json!("wrong")),
        ("exp", serde_json::json!(now - 1)),
        ("iat", serde_json::json!(now + 120)),
        ("tamper", serde_json::json!(true)),
        ("at_hash", serde_json::json!("incorrect")),
    ] {
        let (request, state) = idp.start(&mut store, None, Some("new_user"), false, now);
        idp.claims.lock().unwrap()[field] = value;
        let completion = authenticate(&mut store, &state, now);
        assert!(
            matches!(
                store
                    .oidc_finish(
                        Finish {
                            completion: completion.clone(),
                            request_id: request.request_id,
                            secret: request.secret
                        },
                        now
                    )
                    .unwrap(),
                Progress::Failed
            ),
            "{field}"
        );
    }
    let (request, state) = idp.start(&mut store, None, Some("alice"), false, now);
    let completion = authenticate(&mut store, &state, now);
    assert!(store
        .oidc_finish(
            Finish {
                completion: completion.clone(),
                request_id: request.request_id,
                secret: request.secret
            },
            now
        )
        .is_err());
    let (request, state) = idp.start(&mut store, None, Some("carol"), false, now);
    let completion = authenticate(&mut store, &state, now);
    store.0.execute_batch("CREATE TRIGGER synthetic BEFORE INSERT ON oidc_grants BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    let finish = || Finish {
        completion: completion.clone(),
        request_id: request.request_id.clone(),
        secret: request.secret.clone(),
    };
    assert!(store.oidc_finish(finish(), now).is_err());
    store.0.execute_batch("DROP TRIGGER synthetic").unwrap();
    assert!(matches!(
        store.oidc_finish(finish(), now).unwrap(),
        Progress::Ready {
            reauthorize: false,
            ..
        }
    ));
    let enrollment = || sigil_protocol::accounts::Enrollment {
        invitation: request.secret.clone(),
        device_credential: "ab".repeat(32),
        device_label: "Synthetic".into(),
    };
    store.0.execute_batch("CREATE TRIGGER synthetic BEFORE INSERT ON oidc_bindings BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(store.enroll(enrollment(), now).is_err());
    assert_eq!(
        store
            .0
            .query_row(
                "SELECT count(*) FROM accounts WHERE username='carol'",
                [],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
        0
    );
    store.0.execute_batch("DROP TRIGGER synthetic").unwrap();
    let session = store.enroll(enrollment(), now).unwrap();
    assert_eq!(
        store
            .0
            .query_row(
                "SELECT account FROM oidc_bindings WHERE subject='synthetic-subject'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        session.account_id
    );
}
#[test]
fn configuration_changes_expiry_revocation_and_restore_invalidate_pending_logins() {
    let _guard = crate::egress::tests::NETWORK.lock().unwrap();
    let idp = Idp::new();
    let (dir, mut store, alice, _, now) = crate::admin::tests::setup();
    enable(&mut store, &idp);
    let mut changed_origin = store.administration_policy().unwrap();
    changed_origin.public_origin = Some("https://other.example".into());
    assert!(store.configure_administration(changed_origin).is_err());
    let (request, state) = idp.start(&mut store, Some(&alice), None, false, now);
    let completion = authenticate(&mut store, &state, now);
    let device = store.session(&alice, now).unwrap().device_id;
    store.revoke_device(&alice, &device, now).unwrap();
    assert!(store
        .oidc_finish(
            Finish {
                completion: completion.clone(),
                request_id: request.request_id,
                secret: request.secret
            },
            now
        )
        .is_err());
    let (_, state) = idp.start(&mut store, None, Some("carol"), false, now);
    assert!(store.oidc_claim(&state, now + 600).is_err());
    let (_, state) = idp.start(&mut store, None, Some("dave"), false, now);
    let callback = store.oidc_claim(&state, now).unwrap().unwrap();
    store
        .oidc_install(
            Configure {
                expected_revision: 1,
                provider: None,
                confirm: true,
            },
            None,
        )
        .unwrap();
    assert!(store
        .oidc_verified(callback, Ok("synthetic-subject".into()), now)
        .is_err());
    let backup = dir.path().join("backup.db");
    store.backup(&backup).unwrap();
    let restored = dir.path().join("restored.db");
    Store::restore(&backup, &restored).unwrap();
    let store = Store::open(&restored).unwrap();
    assert!(!store.oidc_configuration().unwrap().enabled);
    assert_eq!(
        store
            .0
            .query_row("SELECT count(*) FROM oidc_flows", [], |r| r
                .get::<_, u32>(0))
            .unwrap(),
        0
    );
}
