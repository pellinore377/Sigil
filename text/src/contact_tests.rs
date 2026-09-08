use crate::{
    contact::{self, Contact},
    parse, Limits, Text,
};

fn contact(id: u8, address: &str) -> Contact {
    Contact {
        user_id: [id; 32],
        address: address.into(),
        display_name: Text::plain("Zoë, Example; \\", Limits::default()).unwrap(),
        avatar_url: None,
    }
}

#[test]
fn mentions_bind_only_selected_identities_and_redaction_removes_metadata() {
    let known = [
        contact(1, "@user:one.example"),
        contact(2, "@user:two.example"),
    ];
    assert_eq!(contact::resolve("@user", &known).unwrap().len(), 2);
    assert_eq!(contact::resolve("@missing", &known).unwrap().len(), 0);
    assert_eq!(
        contact::resolve("@user:one.example", &known).unwrap()[0].user_id,
        [1; 32]
    );
    let text = parse("Hi @user and @user", Limits::default())
        .unwrap()
        .bind_mention(3..8, &known[0])
        .unwrap()
        .bind_mention(13..18, &known[1])
        .unwrap();
    let encoded = text.to_bytes().unwrap();
    let restored = Text::from_bytes(&encoded).unwrap();
    assert_eq!(restored.mentions()[0].user_id, [1; 32]);
    assert_eq!(restored.mentions()[1].user_id, [2; 32]);
    let redacted = restored.redact_range(4..6, Limits::default()).unwrap();
    assert_eq!(redacted.mentions().len(), 1);
    assert_eq!(redacted.mentions()[0].user_id, [2; 32]);
    assert!(!String::from_utf8(redacted.to_bytes().unwrap())
        .unwrap()
        .contains("one.example"));
    assert!(Text::from_bytes(&redacted.to_bytes().unwrap()).is_ok());
    assert!(parse("`@user`", Limits::default())
        .unwrap()
        .bind_mention(0..5, &known[0])
        .is_err());
    let mut forged: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    forged["sigil"]["mentions"][0]["display"] = "@else".into();
    assert!(Text::from_bytes(&serde_json::to_vec(&forged).unwrap()).is_err());
}

#[test]
fn vcard_folding_escaping_and_identity_claims_round_trip_without_network() {
    let mut value = contact(3, "@user:example.org");
    value.display_name = Text::plain(&"名,;\\".repeat(35), Limits::default()).unwrap();
    let vcard = value.vcard().unwrap();
    assert!(vcard.split("\r\n").all(|line| line.len() <= 75));
    let restored = Contact::from_vcard(vcard.as_bytes()).unwrap();
    assert_eq!(restored.user_id, value.user_id);
    assert_eq!(restored.address, value.address);
    assert_eq!(restored.display_name.body(), value.display_name.body());
    assert!(Contact::from_vcard(
        vcard
            .replace(
                "END:VCARD",
                "X-SIGIL-ADDRESS:@evil:example.org\r\nEND:VCARD"
            )
            .as_bytes()
    )
    .is_err());
    assert!(
        Contact::from_vcard(vcard.replace("X-SIGIL-IDENTITY:", "X-UNKNOWN:").as_bytes()).is_err()
    );
    assert!(Contact::from_vcard(format!("{vcard}{vcard}").as_bytes()).is_err());
    assert!(!contact::valid_address("@user:example.org/path"));
    assert!(!contact::valid_address("@user:example.org\nFN:evil"));
}
#[test]
fn contact_and_contact_qr_authoring_require_an_unambiguous_local_resolution() {
    let origin = crate::Origin {
        message: [8; 32],
        creator: [9; 32],
        created_at: 1800000000,
        timezone: None,
    };
    let known = [
        contact(1, "@user:one.example"),
        contact(2, "@user:two.example"),
    ];
    for source in ["@::@user;", "qr::contact::@user;"] {
        let ambiguous = contact::parse_card(source, origin, Default::default(), &known)
            .unwrap()
            .unwrap();
        assert_eq!(ambiguous.hints, vec![crate::Hint::AmbiguousIdentity]);
        assert!(matches!(ambiguous.content, crate::Parsed::Text(_)));
        let resolved = contact::parse_card(source, origin, Default::default(), &known[..1])
            .unwrap()
            .unwrap();
        assert!(matches!(resolved.content, crate::Parsed::Card(_)));
        let missing = crate::parse_card(source, origin, Default::default()).unwrap();
        assert_eq!(missing.hints, vec![crate::Hint::IdentityRequired]);
    }
    let draft = crate::composition::parse_with_contacts(
        "Meet @::@user:one.example; or scan qr::contact::@user:two.example;",
        origin,
        Default::default(),
        None,
        &known,
    )
    .unwrap();
    let crate::Document::Composition(value) = draft.content else {
        panic!("missing contact cards");
    };
    assert_eq!(value.cards().count(), 2);
    assert!(
        crate::composition::Composition::from_bytes(&value.to_bytes().unwrap()).unwrap() == value
    );
}
