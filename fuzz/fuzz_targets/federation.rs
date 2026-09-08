#![no_main]
use libfuzzer_sys::fuzz_target;
use sigil_protocol::federation as p;
use sigil_server::federation_auth as auth;
use std::sync::OnceLock;
use zeroize::Zeroizing;
fuzz_target!(|bytes: &[u8]| {
    if bytes.is_empty() || bytes.len() > 16384 {
        return;
    }
    static KEY: OnceLock<auth::SigningKey> = OnceLock::new();
    let key = KEY.get_or_init(|| {
        auth::SigningKey::from_seed(Zeroizing::new(vec![11; 32]), 1, 1000).unwrap()
    });
    let body = &bytes[1..];
    let request = auth::Request {
        origin: "origin.example",
        destination: "target.example",
        path: p::DELIVER_PATH,
        body,
    };
    let mut headers = auth::sign(key, &request, 1001, [1; 32]).unwrap();
    let names = [
        "signature-input",
        "signature",
        "content-digest",
        "sigil-origin",
        "sigil-destination",
        "content-type",
        "content-encoding",
    ];
    let mode = usize::from(bytes[0]) % 8;
    if mode < 7 {
        if let Ok(value) = axum::http::HeaderValue::from_bytes(body) {
            headers.insert(names[mode], value);
        }
        let _ = auth::inspect(&headers);
        let _ = auth::verify(key.descriptor(), &request, &headers, 1001);
    } else {
        assert!(auth::verify(key.descriptor(), &request, &headers, 1001).is_ok());
    }
    if let Ok(value) = serde_json::from_slice::<p::Discovery>(body) {
        let _ = auth::validate_discovery(&value, "origin.example", 1001);
    }
    if let Ok(value) = serde_json::from_slice::<p::ProxyLookup>(body) {
        let _ = value.operation.valid();
    }
    let _ = serde_json::from_slice::<p::ServerLookup>(body);
    let _ = serde_json::from_slice::<p::Submit>(body);
    let _ = serde_json::from_slice::<p::ConfigureSender>(body);
});
