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
        Self::with_metadata(body_auth, None)
    }
    fn with_metadata(body_auth: bool, jwks_override: Option<String>) -> Self {
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
        let app=Router::new().route("/.well-known/openid-configuration",get(move || {let issuer=issuer.lock().unwrap().clone();let jwks_uri=jwks_override.clone().unwrap_or_else(||format!("{issuer}/jwks"));async move {Json(serde_json::json!({"issuer":issuer,"authorization_endpoint":format!("{issuer}/authorize"),"token_endpoint":format!("{issuer}/token"),"jwks_uri":jwks_uri,"response_types_supported":["code"],"subject_types_supported":["public"],"id_token_signing_alg_values_supported":["RS256"],"code_challenge_methods_supported":["S256"],"token_endpoint_auth_methods_supported":[if body_auth {"client_secret_post"} else {"client_secret_basic"}]}))}}))
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
fn loopback_provider_requires_a_scoped_exception_and_returns_it_without_the_secret() {
    let _guard = crate::egress::tests::NETWORK.lock().unwrap();
    let idp = Idp::new();
    let mut config = idp.configuration();
    let allowed = config.provider.as_ref().unwrap().exceptions.clone();
    config.provider.as_mut().unwrap().exceptions.clear();
    assert!(check(&config).is_err());
    config.provider.as_mut().unwrap().exceptions = allowed.clone();
    config.provider.as_mut().unwrap().exceptions[0].networks = vec!["127.0.0.2/32".into()];
    assert!(check(&config).is_err());
    config.provider.as_mut().unwrap().exceptions = allowed.clone();
    let metadata = check(&config).unwrap();
    let (_dir, mut store, _, _, _) = crate::admin::tests::setup();
    let mut policy = store.administration_policy().unwrap();
    policy.public_origin = Some("https://sigil.example".into());
    store.configure_administration(policy).unwrap();
    let saved = store.oidc_install(config, metadata).unwrap();
    assert_eq!(saved.exceptions, allowed);
    let public = serde_json::to_string(&store.oidc_configuration().unwrap()).unwrap();
    assert!(!public.contains("synthetic-secret"));
    assert!(saved.secret_configured);
}

#[test]
fn provider_requests_cannot_leave_the_issuer_origin_even_with_an_exception() {
    let _guard = crate::egress::tests::NETWORK.lock().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let other = crate::egress::tests::Fixture::local(Router::new().route(
        "/jwks",
        get(move || {
            count.fetch_add(1, Ordering::SeqCst);
            async { "{}" }
        }),
    ));
    let idp = Idp::with_metadata(false, Some(other.uri("127.0.0.1", "/jwks")));
    let mut provider = idp.configuration().provider.unwrap();
    provider.exceptions.push(other.exception("127.0.0.1"));
    assert!(metadata(&provider).is_err());
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    let request = ureq::http::Request::get(other.uri("127.0.0.1", "/jwks"))
        .body(vec![])
        .unwrap();
    assert!(http(&provider, request).is_err());
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    provider.exceptions.clear();
    let request = ureq::http::Request::get(idp.fixture.uri("127.0.0.1", "/jwks"))
        .body(vec![])
        .unwrap();
    assert!(http(&provider, request).is_err());
    let serialized = serde_json::json!({"issuer":"https://idp.example", "client_id":"sigil-synthetic", "client_secret":null});
    assert!(serde_json::from_value::<Provider>(serialized)
        .unwrap()
        .exceptions
        .is_empty());
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

#[test]
fn administrator_oidc_binds_verified_subject_to_the_initiating_browser() {
    let _guard = crate::egress::tests::NETWORK.lock().unwrap();
    let idp = Idp::new();
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("sigil.db")).unwrap();
    let now = crate::enrollment::now().unwrap();
    let code = store.web_setup_code().unwrap().unwrap();
    let browser = store
        .web_claim(
            crate::web_admin::Claim {
                code: Zeroizing::new(code),
                password: Zeroizing::new("a synthetic administrator passphrase".into()),
                server_name: "chat.example".into(),
                public_origin: "https://chat.example".into(),
            },
            now,
        )
        .unwrap();
    enable(&mut store, &idp);
    let (binding, started) = store.web_oidc_start(Some(&browser), now).unwrap();
    assert_eq!(binding, browser);
    let url = openidconnect::url::Url::parse(&started.authorization_url).unwrap();
    let params = url
        .query_pairs()
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(params["scope"], "openid profile");
    let state = params["state"].to_string();
    assert!(store.web_oidc_browser(&state, None, now).is_err());
    assert!(store
        .web_oidc_browser(&state, Some(&random_secret().unwrap()), now)
        .is_err());
    assert!(store.web_oidc_browser(&state, Some(&browser), now).unwrap());
    *idp.claims.lock().unwrap() = serde_json::json!({"iss":idp.fixture.uri("127.0.0.1",""),"sub":"admin-subject","aud":"sigil-synthetic","iat":now,"exp":now+600,"nonce":params["nonce"],"challenge":params["code_challenge"],"preferred_username":"admin","name":"Suggested name","picture":"https://images.example/avatar.png"});
    let callback = store.oidc_claim(&state, now).unwrap().unwrap();
    let identity = callback.verify_profile("synthetic-code").unwrap();
    assert_eq!(identity.username.as_deref(), Some("admin"));
    assert_eq!(identity.name.as_deref(), Some("Suggested name"));
    let session = store
        .web_oidc_verified(callback, Ok(identity), &browser, now)
        .unwrap();
    assert!(store.web_session(&browser, now).is_err());
    assert!(store.oidc_claim(&state, now).is_err());
    assert_eq!(
        store
            .web_status(Some(&session), now)
            .unwrap()
            .suggested_display_name
            .as_deref(),
        Some("Suggested name")
    );
    store
        .web_finish_setup(&session, "admin", Some("Chosen name"), now)
        .unwrap();
    assert_eq!(
        store.web_profile(&session, now).unwrap().display_name,
        "Chosen name"
    );
    store.web_password_policy(&session, false, now).unwrap();
    let status = store.web_status(Some(&session), now).unwrap();
    assert!(status.complete && status.oidc_linked && !status.password_login);
    store.web_logout(&session).unwrap();
    let (cookie, started) = store.web_oidc_start(None, now).unwrap();
    assert!(store.web_session(&cookie, now).is_err());
    let url = openidconnect::url::Url::parse(&started.authorization_url).unwrap();
    let params = url
        .query_pairs()
        .collect::<std::collections::BTreeMap<_, _>>();
    *idp.claims.lock().unwrap() = serde_json::json!({"iss":idp.fixture.uri("127.0.0.1",""),"sub":"different-subject","aud":"sigil-synthetic","iat":now,"exp":now+600,"nonce":params["nonce"],"challenge":params["code_challenge"]});
    let callback = store.oidc_claim(&params["state"], now).unwrap().unwrap();
    let identity = callback.verify_profile("synthetic-code");
    assert!(matches!(
        store.web_oidc_verified(callback, identity, &cookie, now),
        Err(StoreError::Unauthorized)
    ));
    assert!(store.web_session(&cookie, now).is_err());
}

#[test]
fn browser_callback_requires_its_cookie_before_consuming_the_authorization_code() {
    use tower::ServiceExt;
    let _guard = crate::egress::tests::NETWORK.lock().unwrap();
    let idp = Idp::new();
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("sigil.db")).unwrap();
    let now = crate::enrollment::now().unwrap();
    let code = store.web_setup_code().unwrap().unwrap();
    let token = store
        .web_claim(
            crate::web_admin::Claim {
                code: Zeroizing::new(code),
                password: Zeroizing::new("another synthetic admin passphrase".into()),
                server_name: "chat.example".into(),
                public_origin: "https://chat.example".into(),
            },
            now,
        )
        .unwrap();
    enable(&mut store, &idp);
    let (_, started) = store.web_oidc_start(Some(&token), now).unwrap();
    let url = openidconnect::url::Url::parse(&started.authorization_url).unwrap();
    let params = url
        .query_pairs()
        .collect::<std::collections::BTreeMap<_, _>>();
    *idp.claims.lock().unwrap() = serde_json::json!({"iss":idp.fixture.uri("127.0.0.1",""),"sub":"admin-subject","aud":"sigil-synthetic","iat":now,"exp":now+600,"nonce":params["nonce"],"challenge":params["code_challenge"],"preferred_username":"admin"});
    let path = format!(
        "/auth/v0/oidc/callback?code=synthetic-code&scope=openid+profile&session_state=provider-session&state={}",
        params["state"]
    );
    let app = crate::router(
        store,
        crate::auth::AdminToken::load_or_create(&dir.path().join("admin.token")).unwrap(),
    );
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let request = || axum::http::Request::get(&path);
        for extra in [
            "&state=other",
            "&code=other",
            "&iss=https%3A%2F%2Fone.example&iss=https%3A%2F%2Ftwo.example",
        ] {
            let response = app
                .clone()
                .oneshot(
                    axum::http::Request::get(format!("{path}{extra}"))
                        .header("cookie", format!("__Host-sigil-admin={token}"))
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
            assert_eq!(idp.requests.load(Ordering::SeqCst), 0);
        }
        let missing = app
            .clone()
            .oneshot(request().body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(missing.status(), axum::http::StatusCode::UNAUTHORIZED);
        assert_eq!(idp.requests.load(Ordering::SeqCst), 0);
        let response = app
            .clone()
            .oneshot(
                request()
                    .header("cookie", format!("__Host-sigil-admin={token}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
        assert_eq!(response.headers()["location"], "/");
        let cookie = response.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        assert!(!cookie.ends_with(&token));
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::get("/setup/v0/status")
                    .header("cookie", cookie)
                    .header("x-sigil-admin", "1")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(status["authenticated"], true);
        assert_eq!(status["oidc_linked"], true);
        let replay = app
            .oneshot(
                request()
                    .header("cookie", format!("__Host-sigil-admin={token}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(replay.status(), axum::http::StatusCode::UNAUTHORIZED);
        assert_eq!(idp.requests.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn changed_provider_requires_new_verified_login_before_passwords_can_be_disabled() {
    let _guard = crate::egress::tests::NETWORK.lock().unwrap();
    let idp = Idp::new();
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("sigil.db")).unwrap();
    let now = crate::enrollment::now().unwrap();
    let code = store.web_setup_code().unwrap().unwrap();
    let token = store
        .web_claim(
            crate::web_admin::Claim {
                code: Zeroizing::new(code),
                password: Zeroizing::new("synthetic administrator passphrase".into()),
                server_name: "chat.example".into(),
                public_origin: "https://chat.example".into(),
            },
            now,
        )
        .unwrap();
    enable(&mut store, &idp);
    let authenticate = |store: &mut Store, cookie: Option<&str>| {
        let (cookie, started) = store.web_oidc_start(cookie, now).unwrap();
        let url = openidconnect::url::Url::parse(&started.authorization_url).unwrap();
        let params = url
            .query_pairs()
            .collect::<std::collections::BTreeMap<_, _>>();
        *idp.claims.lock().unwrap() = serde_json::json!({"iss":idp.fixture.uri("127.0.0.1",""),"sub":"admin-subject","aud":"sigil-synthetic","iat":now,"exp":now+600,"nonce":params["nonce"],"challenge":params["code_challenge"]});
        let callback = store.oidc_claim(&params["state"], now).unwrap().unwrap();
        let result = callback.verify_profile("synthetic-code");
        store
            .web_oidc_verified(callback, result, &cookie, now)
            .unwrap()
    };
    let token = authenticate(&mut store, Some(&token));
    store.web_finish_setup(&token, "admin", None, now).unwrap();
    store.web_password_policy(&token, false, now).unwrap();
    assert!(store
        .web_unlink_oidc(&token, "synthetic administrator passphrase", now)
        .is_err());
    store.web_password_policy(&token, true, now).unwrap();
    let mut config = idp.configuration();
    config.expected_revision = store.oidc_configuration().unwrap().revision;
    let metadata = check(&config).unwrap();
    store.oidc_install(config, metadata).unwrap();
    assert!(!store.web_status(None, now).unwrap().oidc_login);
    assert!(store.web_password_policy(&token, false, now).is_err());
    assert!(store.web_oidc_start(None, now).is_err());
    let token = authenticate(&mut store, Some(&token));
    store.web_password_policy(&token, false, now).unwrap();
    store.web_logout(&token).unwrap();
    let token = authenticate(&mut store, None);
    assert!(store.web_status(Some(&token), now).unwrap().authenticated);
    store.web_password_policy(&token, true, now).unwrap();
    assert!(store
        .web_unlink_oidc(&token, "wrong password", now)
        .is_err());
    store
        .web_unlink_oidc(&token, "synthetic administrator passphrase", now)
        .unwrap();
    assert!(!store.web_status(None, now).unwrap().oidc_login);
    assert!(store.web_password_policy(&token, false, now).is_err());
}

fn finish_flow(
    store: &mut Store,
    idp: &Idp,
    token: Option<&str>,
    name: Option<&str>,
    subject: &str,
    replace: bool,
    now: u64,
) -> Result<Progress, StoreError> {
    let (request, state) = idp.start(store, token, name, replace, now);
    idp.claims.lock().unwrap()["sub"] = serde_json::json!(subject);
    let completion = authenticate(store, &state, now);
    store.oidc_finish(
        Finish {
            completion,
            request_id: request.request_id,
            secret: request.secret,
        },
        now,
    )
}
fn prepare_retirement(
    store: &mut Store,
    retiring: bool,
    now: u64,
) -> crate::oidc_transition::Transition {
    let current = store.oidc_transition(None, now).unwrap();
    store
        .prepare_oidc_retirement(
            crate::oidc_transition::Prepare {
                configuration_revision: current.configuration_revision,
                revision: current.revision,
                retiring,
                confirm: true,
            },
            now,
        )
        .unwrap()
}
fn acknowledge(
    store: &mut Store,
    token: &str,
    now: u64,
) -> sigil_protocol::oidc::AcknowledgeFallback {
    let current = store.oidc_access(token, now).unwrap();
    let request = sigil_protocol::oidc::AcknowledgeFallback {
        configuration_revision: current.configuration_revision,
        transition_revision: current.transition_revision,
        confirm_invitation_fallback: true,
    };
    store
        .acknowledge_oidc_fallback(token, request.clone(), now)
        .unwrap();
    request
}
#[test]
fn oidc_retirement_preserves_accounts_and_requires_current_user_acknowledgements() {
    let _guard = crate::egress::tests::NETWORK.lock().unwrap();
    let idp = Idp::new();
    let (dir, mut store, alice, bob, now) = crate::admin::tests::setup();
    enable(&mut store, &idp);
    let a = store.session(&alice, now).unwrap();
    assert!(matches!(
        finish_flow(
            &mut store,
            &idp,
            Some(&alice),
            None,
            "alice-sub",
            false,
            now
        )
        .unwrap(),
        Progress::Linked
    ));
    let disable = || Configure {
        expected_revision: 1,
        provider: None,
        confirm: true,
    };
    assert!(store.oidc_install(disable(), None).is_err());
    let transition = prepare_retirement(&mut store, true, now);
    assert_eq!(transition.awaiting_acknowledgement, 1);
    assert_eq!(transition.pending[0].id, a.account_id);
    assert!(finish_flow(
        &mut store,
        &idp,
        None,
        Some("new_user"),
        "new-sub",
        false,
        now
    )
    .is_err());
    assert!(matches!(
        finish_flow(&mut store, &idp, None, None, "alice-sub", true, now).unwrap(),
        Progress::Ready {
            reauthorize: true,
            ..
        }
    ));
    let request = sigil_protocol::oidc::AcknowledgeFallback {
        configuration_revision: transition.configuration_revision,
        transition_revision: transition.revision,
        confirm_invitation_fallback: true,
    };
    assert!(store
        .acknowledge_oidc_fallback(&bob, request.clone(), now)
        .is_err());
    let mut denied = request.clone();
    denied.confirm_invitation_fallback = false;
    assert!(store
        .acknowledge_oidc_fallback(&alice, denied, now)
        .is_err());
    acknowledge(&mut store, &alice, now);
    assert_eq!(
        store
            .oidc_transition(None, now)
            .unwrap()
            .awaiting_acknowledgement,
        0
    );
    prepare_retirement(&mut store, false, now);
    prepare_retirement(&mut store, true, now);
    assert!(matches!(
        store.acknowledge_oidc_fallback(&alice, request, now),
        Err(StoreError::Conflict)
    ));
    acknowledge(&mut store, &alice, now);
    assert!(matches!(
        finish_flow(&mut store, &idp, Some(&bob), None, "bob-sub", false, now).unwrap(),
        Progress::Linked
    ));
    assert!(store.oidc_install(disable(), None).is_err());
    acknowledge(&mut store, &bob, now);
    drop(store);
    let mut store = Store::open(&dir.path().join("sigil.db")).unwrap();
    assert!(
        store
            .oidc_access(&alice, now)
            .unwrap()
            .invitation_fallback_acknowledged
    );
    store.oidc_install(disable(), None).unwrap();
    assert_eq!(store.session(&alice, now).unwrap().account_id, a.account_id);
    assert!(!store.oidc_access(&alice, now).unwrap().retiring);
    let invitation = store
        .invite_reauthorization(&a.account_id, 600, now)
        .unwrap();
    let replacement = random_secret().unwrap();
    let new = store
        .reauthorize(
            sigil_protocol::accounts::Enrollment {
                invitation: invitation.secret,
                device_credential: replacement.clone(),
                device_label: "Fallback".into(),
            },
            now,
        )
        .unwrap();
    assert_eq!(new.account_id, a.account_id);
    assert_eq!(new.address, a.address);
    assert_ne!(new.device_id, a.device_id);
    assert!(store.session(&alice, now).is_err());
    assert_eq!(
        store
            .0
            .query_row("SELECT count(*) FROM accounts", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        store
            .0
            .query_row(
                "SELECT count(*) FROM device_bindings WHERE device=?1",
                [&new.device_id],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
        0
    );
}
#[test]
fn replacing_provider_identity_requires_retirement_but_secret_rotation_does_not() {
    let _guard = crate::egress::tests::NETWORK.lock().unwrap();
    let idp = Idp::new();
    let (_dir, mut store, alice, _, now) = crate::admin::tests::setup();
    enable(&mut store, &idp);
    finish_flow(
        &mut store,
        &idp,
        Some(&alice),
        None,
        "alice-sub",
        false,
        now,
    )
    .unwrap();
    let mut config = idp.configuration();
    config.expected_revision = 1;
    let metadata = check(&config).unwrap();
    config.provider.as_mut().unwrap().client_id = "replacement-client".into();
    assert!(store
        .oidc_install(config.clone(), metadata.clone())
        .is_err());
    config.provider.as_mut().unwrap().client_id = "sigil-synthetic".into();
    config.provider.as_mut().unwrap().issuer = "https://new.example".into();
    assert!(store.oidc_install(config, metadata).is_err());
    prepare_retirement(&mut store, true, now);
    let stale = acknowledge(&mut store, &alice, now);
    let mut config = idp.configuration();
    config.expected_revision = 1;
    let metadata = check(&config).unwrap();
    store.oidc_install(config, metadata).unwrap();
    assert!(!store.oidc_access(&alice, now).unwrap().retiring);
    prepare_retirement(&mut store, true, now);
    assert!(matches!(
        store.acknowledge_oidc_fallback(&alice, stale, now),
        Err(StoreError::Conflict)
    ));
    acknowledge(&mut store, &alice, now);
    let mut config = idp.configuration();
    config.expected_revision = 2;
    config.provider.as_mut().unwrap().client_id = "replacement-client".into();
    let metadata = check(&config).unwrap();
    store.oidc_install(config, metadata).unwrap();
    assert_eq!(
        store.session(&alice, now).unwrap().address,
        "@alice:chat.example"
    );
}
#[test]
fn retirement_review_is_paginated_and_excludes_disabled_users_and_password_owner() {
    let _guard = crate::egress::tests::NETWORK.lock().unwrap();
    let idp = Idp::new();
    let (dir, mut store, alice, bob, now) = crate::admin::tests::setup();
    enable(&mut store, &idp);
    let issuer = idp.configuration().provider.unwrap().issuer;
    for token in [&alice, &bob] {
        let account = store.session(token, now).unwrap().account_id;
        bind(&store.0, &issuer, token, &account).unwrap();
    }
    let owner = store.session(&alice, now).unwrap().account_id;
    store
        .0
        .execute("UPDATE web_owner SET account=?1", [&owner])
        .unwrap();
    for i in 0..51 {
        let account = format!("{i:064x}");
        store
            .0
            .execute(
                "INSERT INTO accounts VALUES(?1,?2,0)",
                (&account, format!("fixture_{i}")),
            )
            .unwrap();
        bind(&store.0, &issuer, &account, &account).unwrap();
    }
    let current = prepare_retirement(&mut store, true, now);
    assert_eq!(current.linked_accounts, 52);
    assert_eq!(current.pending.len(), 50);
    assert!(!current.pending.iter().any(|p| p.id == owner));
    let page = store
        .oidc_transition(current.next_after.as_deref(), now)
        .unwrap();
    assert_eq!(page.pending.len(), 2);
    assert!(page.next_after.is_none());
    assert!(page
        .pending
        .iter()
        .all(|p| p.id > current.next_after.clone().unwrap()));
    let stale = acknowledge(&mut store, &bob, now);
    store
        .0
        .execute("UPDATE web_owner SET password_login=0", [])
        .unwrap();
    assert!(store
        .prepare_oidc_retirement(
            crate::oidc_transition::Prepare {
                configuration_revision: current.configuration_revision,
                revision: current.revision,
                retiring: false,
                confirm: true
            },
            now
        )
        .is_err());
    let b = store.session(&bob, now).unwrap();
    store
        .0
        .execute(
            "UPDATE accounts SET disabled=1 WHERE id=?1",
            [&b.account_id],
        )
        .unwrap();
    assert!(store.acknowledge_oidc_fallback(&bob, stale, now).is_err());
    assert_eq!(
        store.oidc_transition(None, now).unwrap().linked_accounts,
        51
    );
    let backup = dir.path().join("backup.db");
    store.backup(&backup).unwrap();
    let restored = dir.path().join("restored.db");
    Store::restore(&backup, &restored).unwrap();
    let restored = Store::open(&restored).unwrap();
    assert!(!restored.oidc_transition(None, now).unwrap().retiring);
    assert_eq!(
        restored
            .0
            .query_row("SELECT count(*) FROM oidc_fallback_ack", [], |r| r
                .get::<_, u32>(0))
            .unwrap(),
        0
    );
}
