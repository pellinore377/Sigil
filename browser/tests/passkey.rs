#[path = "../src/passkey_codec.rs"]
mod codec;

#[test]
fn base64url_round_trips_without_padding() {
    let bytes: Vec<u8> = (0..=255).collect();
    let text = codec::encode(&bytes);
    assert!(!text.contains(['=', '+', '/']));
    assert_eq!(codec::decode(&text).unwrap(), bytes);
    assert_eq!(codec::encode(&[0xfb, 0xff]), "-_8");
    assert!(codec::decode("-_8=").is_err());
    assert!(codec::decode("+/8").is_err());
}
#[test]
fn request_decodes_and_canonicalises_credential_keys() {
    let json = r#"{"rp_id":"chat.example.test","challenge":"AAECAwQFBgcICQoLDA0ODw","credentials":[{"id":"AQID","salt":"BAUG"}],"future":1}"#;
    let request = codec::get_request(json).unwrap();
    assert_eq!(request.rp_id, "chat.example.test");
    assert_eq!(request.challenge, (0..16).collect::<Vec<u8>>());
    assert_eq!(request.credentials[0].key, "AQID");
    assert_eq!(request.credentials[0].id, [1, 2, 3]);
    assert_eq!(request.credentials[0].salt, [4, 5, 6]);
}
#[test]
fn request_rejects_empty_or_malformed_fields() {
    for json in [
        r#"{"rp_id":"","challenge":"AQID","credentials":[{"id":"AQID","salt":"AQID"}]}"#,
        r#"{"rp_id":"a.test","challenge":"","credentials":[{"id":"AQID","salt":"AQID"}]}"#,
        r#"{"rp_id":"a.test","challenge":"AQID","credentials":[]}"#,
        r#"{"rp_id":"a.test","challenge":"AQID","credentials":[{"id":"AQ==","salt":"AQID"}]}"#,
        r#"{"rp_id":"a.test","challenge":"AQID","credentials":[{"id":"AQID"}]}"#,
        "not json",
    ] {
        assert!(codec::get_request(json).is_err(), "{json}");
    }
}
#[test]
fn create_options_decode_with_display_fallback() {
    let json = r#"{"rp_id":"a.test","rp_name":"Sigil","user_id":"AQID","user_name":"@alice:a.test","user_display":"","challenge":"BAUG","salt":"BwgJ","exclude":["CgsM"]}"#;
    let options = codec::create_options(json).unwrap();
    assert_eq!(
        (options.rp_id.as_str(), options.rp_name.as_str()),
        ("a.test", "Sigil")
    );
    assert_eq!(
        (options.user_name.as_str(), options.challenge.as_slice()),
        ("@alice:a.test", &[4u8, 5, 6][..])
    );
    assert_eq!(options.user_id, [1, 2, 3]);
    assert_eq!(options.user_display, "@alice:a.test");
    assert_eq!(options.salt, [7, 8, 9]);
    assert_eq!(options.exclude, [vec![10, 11, 12]]);
    let missing = r#"{"rp_id":"a.test","rp_name":"Sigil","user_id":"AQID","user_name":"u","user_display":"U","challenge":"BAUG","salt":"BwgJ"}"#;
    assert!(codec::create_options(missing).unwrap().exclude.is_empty());
    assert!(codec::create_options(&missing.replace("BwgJ", "")).is_err());
}
#[test]
fn prf_output_must_be_32_bytes_and_results_are_base64url() {
    assert_eq!(
        codec::prf_output(vec![0; 16]).unwrap_err(),
        codec::UNSUPPORTED
    );
    let prf = codec::prf_output(vec![0xff; 32]).unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&codec::create_result(&[0xfb], &[1, 2, 3], &prf)).unwrap();
    assert_eq!(value["credential"], "-w");
    assert_eq!(value["salt"], "AQID");
    assert_eq!(
        codec::decode(value["prf"].as_str().unwrap()).unwrap(),
        [0xff; 32]
    );
    let value: serde_json::Value = serde_json::from_str(&codec::get_result(&[1], &prf)).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 2);
    assert_eq!(value["credential"], "AQ");
}
