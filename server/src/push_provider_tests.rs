use super::*;
use axum::{
    body::Bytes,
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use p256::elliptic_curve::sec1::ToEncodedPoint;
use std::{
    process::Command,
    sync::{Arc, Mutex},
};

const PEM: &str = include_str!("../tests/fixtures/synthetic-fcm-key.pem");
fn credentials() -> FcmCredentials {
    FcmCredentials {
        project_id: "sigil-synthetic".into(),
        client_email: "push@sigil-synthetic.iam.gserviceaccount.com".into(),
        private_key: Zeroizing::new(PEM.into()),
    }
}
fn response(status: u16, body: &str) -> egress::Response {
    egress::Response {
        status,
        retry_after: None,
        content_type: Some("application/json".into()),
        body: Zeroizing::new(body.as_bytes().to_vec()),
    }
}
fn token(now: u64) -> AccessToken {
    AccessToken::from_response(
        &response(
            200,
            r#"{"access_token":"synthetic-access","token_type":"Bearer","expires_in":3600}"#,
        ),
        now,
    )
    .unwrap()
}
fn subscription(vapid: &Vapid, endpoint: &str) -> (Target, p256::SecretKey, Auth) {
    let secret = p256::SecretKey::from_slice(&[7; 32]).unwrap();
    let auth = Auth::from([9; 16]);
    (
        Target::UnifiedPush {
            endpoint: endpoint.into(),
            public_key: B64::encode_string(secret.public_key().to_encoded_point(false).as_bytes()),
            auth_secret: B64::encode_string(&auth),
            vapid_key: vapid.public_key(),
        },
        secret,
        auth,
    )
}
fn jwt_parts(jwt: &str) -> (serde_json::Value, serde_json::Value, Vec<u8>, String) {
    let pieces: Vec<_> = jwt.split('.').collect();
    assert_eq!(pieces.len(), 3);
    (
        serde_json::from_slice(&B64::decode_vec(pieces[0]).unwrap()).unwrap(),
        serde_json::from_slice(&B64::decode_vec(pieces[1]).unwrap()).unwrap(),
        B64::decode_vec(pieces[2]).unwrap(),
        format!("{}.{}", pieces[0], pieces[1]),
    )
}

#[test]
fn vapid_and_webpush_bind_origin_key_and_exact_generic_payload() {
    let bytes = Vapid::generate().unwrap();
    let vapid = Vapid::from_pkcs8(&bytes).unwrap();
    let (target, secret, auth) = subscription(
        &vapid,
        "https://push.example:443/secret?capability=synthetic",
    );
    let challenge = Payload::Challenge {
        channel: &[1; 32],
        proof: &[2; 32],
    };
    for payload in [Payload::Wake, challenge] {
        let request = vapid
            .request(&target, "mailto:operator@example.com", &payload, 1000, 600)
            .unwrap();
        assert_eq!(request.headers()["ttl"], "600");
        assert_eq!(request.headers()[header::CONTENT_ENCODING], "aes128gcm");
        assert_eq!(
            request.headers().contains_key("topic"),
            matches!(payload, Payload::Wake)
        );
        assert!(request.body().len() <= 4096);
        assert!(
            u32::from_be_bytes(request.body()[16..20].try_into().unwrap()) as usize
                > request.body().len() - 86
        );
        let decoded = web_push_native::decrypt(request.body().to_vec(), &secret, &auth).unwrap();
        assert!(Payload::from_bytes(&decoded).unwrap() == payload);
        assert!(
            web_push_native::decrypt(request.body().to_vec(), &secret, &Auth::from([8; 16]))
                .is_err()
        );
        let next = vapid
            .request(&target, "mailto:operator@example.com", &payload, 1000, 600)
            .unwrap();
        assert_ne!(request.body(), next.body());
        let authorization = &request.headers()[header::AUTHORIZATION];
        assert!(authorization.is_sensitive());
        let (jwt, key) = authorization
            .to_str()
            .unwrap()
            .strip_prefix("vapid t=")
            .unwrap()
            .split_once(", k=")
            .unwrap();
        assert_eq!(key, vapid.public_key());
        let (header, claims, sig, input) = jwt_parts(jwt);
        assert_eq!(header, serde_json::json!({"alg":"ES256","typ":"JWT"}));
        assert_eq!(
            claims,
            serde_json::json!({"aud":"https://push.example","exp":4600,"sub":"mailto:operator@example.com"})
        );
        // Independent RustCrypto verification of ring's fixed-width ES256 signature.
        use p256::ecdsa::signature::Verifier;
        let verifier =
            p256::ecdsa::VerifyingKey::from_sec1_bytes(&B64::decode_vec(key).unwrap()).unwrap();
        let signature = p256::ecdsa::Signature::from_slice(&sig).unwrap();
        verifier.verify(input.as_bytes(), &signature).unwrap();
        assert!(verifier.verify(b"changed origin", &signature).is_err());
        assert_ne!(&request.body()[21..86], B64::decode_vec(key).unwrap());
    }
    let mut bad = target.clone();
    if let Target::UnifiedPush { vapid_key, .. } = &mut bad {
        *vapid_key = Vapid::from_pkcs8(&Vapid::generate().unwrap())
            .unwrap()
            .public_key();
    }
    assert!(vapid
        .request(
            &bad,
            "mailto:operator@example.com",
            &Payload::Wake,
            1000,
            600
        )
        .is_err());
    for contact in [
        "",
        "mailto:x@localhost",
        "mailto:x@example.com?body=secret",
        "https://user:secret@example.com",
        "https://example.com/#secret",
    ] {
        assert!(validate_contact(contact).is_err());
    }
    for ttl in [0, MAX_TTL + 1] {
        assert!(vapid
            .request(
                &target,
                "https://example.com/contact",
                &Payload::Wake,
                1000,
                ttl
            )
            .is_err());
    }
    assert!(vapid
        .request(
            &target,
            "https://example.com/contact",
            &Payload::Wake,
            u64::MAX,
            1
        )
        .is_err());
}

#[test]
fn webpush_dependency_decrypts_rfc8291_published_vector() {
    // RFC8291 section 5; public synthetic example, not a generated round trip.
    let ciphertext = B64::decode_vec("DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN").unwrap();
    let secret = p256::SecretKey::from_slice(
        &B64::decode_vec("q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94").unwrap(),
    )
    .unwrap();
    let auth = Auth::from(decode::<16>("BTBZMqHH6r4Tts7J_aSIgg").unwrap());
    assert_eq!(
        web_push_native::decrypt(ciphertext.clone(), &secret, &auth).unwrap(),
        b"When I grow up, I want to be a watermelon"
    );
    for end in [0, 20, 21, 85, 86, ciphertext.len() - 1] {
        assert!(web_push_native::decrypt(ciphertext[..end].to_vec(), &secret, &auth).is_err());
    }
    let mut zero_size = ciphertext.clone();
    zero_size[16..20].fill(0);
    assert!(web_push_native::decrypt(zero_size, &secret, &auth).is_err());
    let mut changed = ciphertext;
    *changed.last_mut().unwrap() ^= 1;
    assert!(web_push_native::decrypt(changed, &secret, &auth).is_err());
}

#[test]
fn oauth_assertion_has_fixed_scope_and_independent_rsa_verification() {
    let fcm = Fcm::new(&credentials()).unwrap();
    let request = fcm.token_request(1000).unwrap();
    assert_eq!(request.uri(), OAUTH);
    assert_eq!(
        request.headers()[header::CONTENT_TYPE],
        "application/x-www-form-urlencoded"
    );
    let text = std::str::from_utf8(request.body()).unwrap();
    let jwt = text
        .strip_prefix("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer&assertion=")
        .unwrap();
    let (header, claims, signature, input) = jwt_parts(jwt);
    assert_eq!(header, serde_json::json!({"alg":"RS256","typ":"JWT"}));
    assert_eq!(
        claims,
        serde_json::json!({"iss":credentials().client_email,"scope":SCOPE,"aud":OAUTH,"iat":1000,"exp":4600})
    );
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("key.pem"), PEM).unwrap();
    std::fs::write(temp.path().join("input"), input.as_bytes()).unwrap();
    std::fs::write(temp.path().join("signature"), signature).unwrap();
    let output = Command::new("openssl")
        .args(["pkey", "-in", "key.pem", "-pubout", "-out", "public.pem"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let verify = || {
        Command::new("openssl")
            .args([
                "dgst",
                "-sha256",
                "-verify",
                "public.pem",
                "-signature",
                "signature",
                "input",
            ])
            .current_dir(temp.path())
            .output()
            .unwrap()
    };
    assert!(verify().status.success());
    std::fs::write(temp.path().join("input"), b"changed scope").unwrap();
    assert!(!verify().status.success());
    assert!(fcm.token_request(u64::MAX).is_err());
    for project in [
        "short",
        "-invalid",
        "valid-project/other",
        "valid-project?x",
        "UPPER-project",
        "valid-project-",
    ] {
        let mut bad = credentials();
        bad.project_id = project.into();
        assert!(Fcm::new(&bad).is_err());
    }
    for email in [
        "push@example.com",
        "push@evilgserviceaccount.com",
        "push@service.gserviceaccount.com.evil.example",
        "push\n@service.gserviceaccount.com",
    ] {
        let mut bad = credentials();
        bad.client_email = email.into();
        assert!(Fcm::new(&bad).is_err());
    }
    let mut bad = credentials();
    bad.private_key = Zeroizing::new(format!("{PEM}{PEM}"));
    assert!(Fcm::new(&bad).is_err());
}

#[test]
fn token_lifetimes_and_provider_failures_do_not_discard_valid_channels() {
    let good = token(1000);
    assert!(!good.valid_at(999));
    assert!(good.valid_at(1000));
    assert!(good.valid_at(4539));
    assert!(!good.valid_at(4540));
    for body in [
        r#"{"access_token":"a","token_type":"Basic","expires_in":3600}"#,
        r#"{"access_token":"a\r\nb","token_type":"Bearer","expires_in":3600}"#,
        r#"{"access_token":"a","token_type":"Bearer","expires_in":60}"#,
        r#"{"access_token":"a","token_type":"Bearer","expires_in":3601}"#,
        r#"{"access_token":"a","token_type":"Bearer","expires_in":3600,"expires_in":1}"#,
    ] {
        assert!(AccessToken::from_response(&response(200, body), 1000).is_err());
    }
    let mut r = response(429, "{}");
    r.retry_after = Some("99999999999999999999999999".into());
    assert_eq!(
        classify(&r, true, 1000),
        Outcome::Retry {
            not_before: u64::MAX,
            refresh_auth: false
        }
    );
    r.retry_after = Some("Thu, 01 Jan 1970 00:33:20 GMT".into());
    assert_eq!(
        classify(&r, true, 1000),
        Outcome::Retry {
            not_before: 2000,
            refresh_auth: false
        }
    );
    r.retry_after = Some("0".into());
    assert_eq!(
        classify(&r, true, 1000),
        Outcome::Retry {
            not_before: 1060,
            refresh_auth: false
        }
    );
    for status in [200, 201, 202, 400, 401, 403, 404, 410, 429, 500, 503] {
        assert!(matches!(
            classify(&response(status, "{}"), true, 1000),
            Outcome::Retry { .. }
        ));
    }
    for (status, code) in [(404, "UNREGISTERED"), (403, "SENDER_ID_MISMATCH")] {
        let r = response(status, &serde_json::json!({"error":{"details":[{"@type":"type.googleapis.com/google.firebase.fcm.v1.FcmError","errorCode":code}]}}).to_string());
        assert_eq!(classify(&r, true, 1000), Outcome::InvalidRegistration);
        let forged = response(
            status,
            &serde_json::json!({"error":{"details":[{"@type":"other","errorCode":code}]}})
                .to_string(),
        );
        assert!(matches!(
            classify(&forged, true, 1000),
            Outcome::Retry { .. }
        ));
    }
    assert_eq!(
        classify(
            &response(200, r#"{"name":"projects/synthetic/messages/1"}"#),
            true,
            1000
        ),
        Outcome::Accepted
    );
    for status in [201, 202] {
        assert_eq!(
            classify(&response(status, ""), false, 1000),
            Outcome::Accepted
        );
    }
    for status in [404, 410] {
        assert_eq!(
            classify(&response(status, ""), false, 1000),
            Outcome::InvalidRegistration
        );
    }
}

#[test]
fn actual_https_carries_only_fcm_hint_or_encrypted_unifiedpush_payload() {
    let _guard = crate::egress::tests::NETWORK.lock().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let copy = seen.clone();
    let fixture = crate::egress::tests::Fixture::new(Router::new().route(
        "/send",
        post(move |headers: HeaderMap, bytes: Bytes| {
            let copy = copy.clone();
            async move {
                copy.lock().unwrap().push((headers.clone(), bytes.to_vec()));
                if headers.contains_key(header::CONTENT_ENCODING) {
                    (StatusCode::CREATED, Json(serde_json::json!({})))
                } else {
                    (
                        StatusCode::OK,
                        Json(serde_json::json!({"name":"projects/sigil-synthetic/messages/1"})),
                    )
                }
            }
        }),
    ));
    let vapid = Vapid::from_pkcs8(&Vapid::generate().unwrap()).unwrap();
    let (target, secret, auth) = subscription(&vapid, &fixture.uri("chat.example", "/send"));
    let request = vapid
        .request(
            &target,
            "mailto:operator@example.com",
            &Payload::Wake,
            1000,
            60,
        )
        .unwrap();
    let (parts, body) = request.into_parts();
    assert_eq!(
        classify(
            &fixture
                .send(Request::from_parts(parts, body.as_slice()))
                .unwrap(),
            false,
            1000
        ),
        Outcome::Accepted
    );
    let fcm = Fcm::new(&credentials()).unwrap();
    let target = Target::Fcm {
        token: "synthetic-registration".into(),
    };
    let request = fcm
        .request(&token(1000), &target, &Payload::Wake, 1000, 60)
        .unwrap();
    assert_eq!(
        request.uri().to_string(),
        "https://fcm.googleapis.com/v1/projects/sigil-synthetic/messages:send"
    );
    assert!(request.headers()[header::AUTHORIZATION].is_sensitive());
    let (mut parts, body) = request.into_parts();
    parts.uri = fixture.uri("chat.example", "/send").parse().unwrap();
    assert_eq!(
        classify(
            &fixture
                .send(Request::from_parts(parts, body.as_slice()))
                .unwrap(),
            true,
            1000
        ),
        Outcome::Accepted
    );
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    let decrypted = web_push_native::decrypt(seen[0].1.clone(), &secret, &auth).unwrap();
    assert!(Payload::from_bytes(&decrypted).unwrap() == Payload::Wake);
    let fcm_body: serde_json::Value = serde_json::from_slice(&seen[1].1).unwrap();
    assert_eq!(
        fcm_body,
        serde_json::json!({"message":{"token":"synthetic-registration","data":{"sigil":B64::encode_string(&Payload::Wake.to_bytes())},"android":{"priority":"normal","ttl":"60s","collapse_key":"sigil-wake-v0"}}})
    );
    assert!(fcm
        .request(&token(1000), &target, &Payload::Wake, 4540, 60)
        .is_err());
}
