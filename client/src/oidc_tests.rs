use super::*;
use crate::network::tests::{Fixture, CA};
use axum::{
    body::Bytes,
    routing::{get, post},
    Json, Router,
};
use base64ct::{Base64UrlUnpadded as B64, Encoding};
use ring::signature::KeyPair;
use serde_json::json;
use std::sync::{Arc, Mutex};
fn parameter<'a>(text: &'a str, key: &str) -> &'a str {
    text.split('&')
        .filter_map(|v| v.split_once('='))
        .find(|(k, _)| *k == key)
        .unwrap()
        .1
}
fn mobile(client: &mut ClientStore, request: serde_json::Value) -> serde_json::Value {
    let result: serde_json::Value =
        serde_json::from_str(&client.mobile_command(&request.to_string())).unwrap();
    assert_eq!(result["ok"], true, "{result}");
    result["value"].clone()
}
#[test]
fn native_oidc_https_enrollment_survives_restarts_and_local_commit_failure() {
    let claims = Arc::new(Mutex::new(json!({})));
    let values = claims.clone();
    let origin = Arc::new(Mutex::new(String::new()));
    let issuer = origin.clone();
    let key = Arc::new(ring::signature::Ed25519KeyPair::from_seed_unchecked(&[37; 32]).unwrap());
    let public = B64::encode_string(key.public_key().as_ref());
    let idp=Fixture::local_provider(Router::new().route("/.well-known/openid-configuration",get(move||{let origin=issuer.lock().unwrap().clone();async move {Json(json!({"issuer":origin,"authorization_endpoint":format!("{origin}/authorize"),"token_endpoint":format!("{origin}/token"),"jwks_uri":format!("{origin}/jwks"),"response_types_supported":["code"],"subject_types_supported":["public"],"id_token_signing_alg_values_supported":["EdDSA"],"code_challenge_methods_supported":["S256"]}))}}))
        .route("/jwks",get(move||{let key=public.clone();async move {Json(json!({"keys":[{"kty":"OKP","crv":"Ed25519","kid":"test","use":"sig","alg":"EdDSA","x":key}]}))}}))
        .route("/token",post(move|body:Bytes|{let mut claims=values.lock().unwrap().clone();let text=std::str::from_utf8(&body).unwrap();assert_eq!(parameter(text,"code"),"synthetic-code");let expected=claims.as_object_mut().unwrap().remove("challenge").unwrap();assert_eq!(B64::encode_string(&sha2::Sha256::digest(parameter(text,"code_verifier").as_bytes())),expected.as_str().unwrap());
            let input=format!("{}.{}",B64::encode_string(br#"{"alg":"EdDSA","kid":"test"}"#),B64::encode_string(&serde_json::to_vec(&claims).unwrap()));let token=format!("{input}.{}",B64::encode_string(key.sign(input.as_bytes()).as_ref()));async move {Json(json!({"access_token":"synthetic","token_type":"Bearer","id_token":token}))}})));
    let provider_origin = format!("https://127.0.0.1:{}", idp.port());
    *origin.lock().unwrap() = provider_origin.clone();
    let (dir, fixture, _, now) = tests::setup();
    let server_origin = format!("https://chat.example:{}", fixture.port());
    let mut server = sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap();
    let mut policy = server.administration_policy().unwrap();
    policy.registration = sigil_protocol::admin::Registration::Oidc;
    policy.public_origin = Some(server_origin.clone());
    server.configure_administration(policy).unwrap();
    drop(server);
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::Specific(Arc::new(vec![
            ureq::tls::Certificate::from_der(CA),
        ])))
        .build();
    let agent = crate::network::tests::agent(
        ureq::Agent::config_builder()
            .tls_config(tls)
            .proxy(None)
            .http_status_as_error(false)
            .max_redirects(0)
            .build(),
    );
    let admin = std::fs::read_to_string(dir.path().join("admin.token")).unwrap();
    let config = json!({"expected_revision":0,"confirm":true,"provider":{"issuer":provider_origin,"client_id":"synthetic","client_secret":null,"exceptions":[{"host":"127.0.0.1","port":idp.port(),"networks":["127.0.0.1/32"],"root_ca":include_bytes!("../../server/tests/fixtures/provider-ca.der").to_vec()}]}});
    assert_eq!(
        agent
            .put(format!("{server_origin}/admin/v0/oidc"))
            .header("authorization", format!("Bearer {}", admin.trim()))
            .header("content-type", "application/json")
            .send(serde_json::to_vec(&config).unwrap())
            .unwrap()
            .status(),
        200
    );
    let path = dir.path().join("oidc-client.db");
    let open = || {
        ClientStore::open(
            &path,
            StorageKey::new(sigil_crypto::Secret32::from_bytes([9; 32])).unwrap(),
        )
        .unwrap()
    };
    let mut client = open();
    client
        .prepare_oidc_enrollment(
            "chat.example",
            fixture.port(),
            &[CA.to_vec()],
            Some("carol"),
            "Synthetic",
            false,
        )
        .unwrap();
    let previous = load(&client.db, &client.key).unwrap().0;
    client.restart_oidc_enrollment(Some("carol")).unwrap();
    let active = load(&client.db, &client.key).unwrap().0;
    assert_ne!(previous.credential, active.credential);
    assert_ne!(previous.invitation, active.invitation);
    let started = client.start_oidc_online().unwrap();
    let query = started.authorization_url.split_once('?').unwrap().1;
    let csrf = parameter(query, "state").to_owned();
    *claims.lock().unwrap() = json!({"iss":provider_origin,"sub":"synthetic-subject","aud":"synthetic","iat":now,"exp":now+600,"nonce":parameter(query,"nonce"),"challenge":parameter(query,"code_challenge")});
    assert!(client.finish_oidc_online().unwrap().is_none());
    drop(client);
    drop(fixture);
    let server = sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap();
    let token =
        sigil_server::auth::AdminToken::load_or_create(&dir.path().join("admin.token")).unwrap();
    let port = server_origin.rsplit(':').next().unwrap().parse().unwrap();
    let _fixture = Fixture::maintained_at(
        sigil_server::router(server, token),
        std::future::pending(),
        port,
    );
    let response = agent
        .get(format!(
            "{server_origin}/auth/v0/oidc/callback?state={csrf}&code=synthetic-code"
        ))
        .call()
        .unwrap();
    assert_eq!(response.status(), 303);
    let (request_id, completion) = response.headers()["location"]
        .to_str()
        .unwrap()
        .strip_prefix("sigil://oidc/")
        .unwrap()
        .split_once('/')
        .unwrap();
    let mut client = open();
    assert!(client.finish_oidc_online().unwrap().is_none());
    assert!(client
        .accept_oidc_callback(&"cd".repeat(32), completion)
        .is_err());
    client.accept_oidc_callback(request_id, completion).unwrap();
    client.db.execute_batch("CREATE TRIGGER synthetic BEFORE UPDATE ON connection BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(client.finish_oidc_online().is_err());
    client.db.execute_batch("DROP TRIGGER synthetic;").unwrap();
    drop(client);
    let mut client = open();
    let session = client.finish_oidc_online().unwrap().unwrap();
    assert_eq!(session.address, "@carol:chat.example");
    assert_eq!(client.connection_session().unwrap(), Some(session));
    assert!(client
        .connected_client()
        .unwrap()
        .oidc_bindings()
        .unwrap()
        .contains(&provider_origin));
    client
        .connected_client()
        .unwrap()
        .unlink_oidc(&provider_origin)
        .unwrap();
    assert!(client
        .connected_client()
        .unwrap()
        .oidc_bindings()
        .unwrap()
        .is_empty());
    client.publish_device_binding_online().unwrap();
    let identity = client.own_device_binding().unwrap();
    let session = client.connection_session().unwrap();
    assert_eq!(
        mobile(&mut client, json!({"command":"account_access"}))["access"]["linked"],
        false
    );
    let started = mobile(
        &mut client,
        json!({"command":"oidc_account","action":"start"}),
    );
    assert_eq!(
        mobile(&mut client, json!({"command":"account_access"}))["link_pending"],
        true
    );
    let query = started["authorization_url"]
        .as_str()
        .unwrap()
        .split_once('?')
        .unwrap()
        .1;
    *claims.lock().unwrap() = json!({"iss":provider_origin,"sub":"synthetic-subject","aud":"synthetic","iat":now,"exp":now+600,"nonce":parameter(query,"nonce"),"challenge":parameter(query,"code_challenge")});
    let response = agent
        .get(format!(
            "{server_origin}/auth/v0/oidc/callback?state={}&code=synthetic-code",
            parameter(query, "state")
        ))
        .call()
        .unwrap();
    assert_eq!(response.status(), 303);
    let (request_id, completion) = response.headers()["location"]
        .to_str()
        .unwrap()
        .strip_prefix("sigil://oidc/")
        .unwrap()
        .split_once('/')
        .unwrap();
    assert!(!client.finish_oidc_link_online().unwrap());
    drop(client);
    let mut client = open();
    let linked = mobile(
        &mut client,
        json!({"command":"callback","request_id":request_id,"completion":completion}),
    );
    assert_eq!(linked["access"]["linked"], true);
    assert_eq!(linked["link_pending"], false);
    assert_eq!(client.connection_session().unwrap(), session);
    assert_eq!(client.own_device_binding().unwrap(), identity);
    assert!(client
        .connected_client()
        .unwrap()
        .oidc_bindings()
        .unwrap()
        .contains(&provider_origin));
    client.prepare_oidc_link().unwrap();
    client.cancel_oidc_link().unwrap();
    assert!(client.finish_oidc_link_online().is_err());
    let network = client.connected_client().unwrap();
    assert_eq!(
        network.discover_account("carol").unwrap().address,
        "@carol:chat.example"
    );
    let preference = network.discovery_preference().unwrap();
    network
        .set_discovery_preference(&sigil_protocol::admin::DiscoveryPreference {
            revision: preference.revision,
            discoverable: false,
        })
        .unwrap();
    assert!(network.discover_account("carol").is_err());
    assert!(network.discover_account("car").is_err());
    assert!(client.restart_oidc_enrollment(None).is_err());
    let access = mobile(&mut client, json!({"command":"account_access"}))["access"].clone();
    let mut revision = access["transition_revision"].as_u64().unwrap();
    let mut first = None;
    for retiring in [true, false, true] {
        let body = json!({"configuration_revision":access["configuration_revision"],"revision":revision,"retiring":retiring,"confirm":true});
        let response = agent
            .put(format!("{server_origin}/admin/v0/oidc/transition"))
            .header("authorization", format!("Bearer {}", admin.trim()))
            .header("content-type", "application/json")
            .send(serde_json::to_vec(&body).unwrap())
            .unwrap();
        assert_eq!(response.status(), 200);
        let current = mobile(&mut client, json!({"command":"account_access"}))["access"].clone();
        revision = current["transition_revision"].as_u64().unwrap();
        assert_eq!(current["retiring"], retiring);
        assert_eq!(current["invitation_fallback_acknowledged"], false);
        if retiring {
            if let Some(prior) = first.as_ref() {
                let denied: serde_json::Value = serde_json::from_str(
                    &client.mobile_command(&serde_json::to_string(prior).unwrap()),
                )
                .unwrap();
                assert_eq!(denied["ok"], false);
            }
            let ack = json!({"command":"acknowledge_access","configuration_revision":current["configuration_revision"],"transition_revision":revision});
            let acknowledged = mobile(&mut client, ack.clone());
            assert_eq!(
                acknowledged["access"]["invitation_fallback_acknowledged"],
                true
            );
            first = Some(ack);
        }
    }
    assert_eq!(client.connection_session().unwrap(), session);
    assert_eq!(client.own_device_binding().unwrap(), identity);
}
