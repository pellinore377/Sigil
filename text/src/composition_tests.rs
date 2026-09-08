use crate::{
    composition::{self, Composition, Part},
    structured::CardLimits,
    Document, Origin,
};
fn origin() -> Origin<'static> {
    Origin {
        message: [1; 32],
        creator: [2; 32],
        created_at: 1800000000,
        timezone: Some("UTC"),
    }
}
#[test]
fn invalid_structured_headers_cannot_restore_redacted_source() {
    for source in [
        "poll::redact::SECRET;",
        "checklist::bold::redact::SECRET;",
        "Before poll::redact::SECRET; after",
        "help::redact::SECRET;",
    ] {
        let draft = composition::parse(source, origin(), CardLimits::default(), None).unwrap();
        assert!(
            !String::from_utf8(draft.content.to_bytes().unwrap())
                .unwrap()
                .contains("SECRET"),
            "{source}"
        );
    }
    for source in [
        "`poll::redact::literal;`",
        "```\npoll::redact::literal;\n```",
        "\\redact::literal;",
    ] {
        let draft = composition::parse(source, origin(), CardLimits::default(), None).unwrap();
        assert!(String::from_utf8(draft.content.to_bytes().unwrap())
            .unwrap()
            .contains("literal"));
    }
}
#[test]
fn mixed_messages_preserve_card_origins_and_safe_fallbacks() {
    let draft = composition::parse(
        "# Plans\n\nnote::bring **water**;\nThen timer::5m; done.",
        origin(),
        CardLimits::default(),
        None,
    )
    .unwrap();
    let Document::Composition(value) = draft.content else {
        panic!("missing composition");
    };
    assert_eq!(value.cards().count(), 2);
    let cards: Vec<_> = value.cards().collect();
    assert_eq!(cards[0].id, composition::card_id(&[1; 32], 0));
    assert_eq!(cards[1].id, composition::card_id(&[1; 32], 1));
    assert!(value.html().unwrap().contains("<h1>Plans</h1>"));
    let bytes = value.to_bytes().unwrap();
    assert!(Composition::from_bytes(&bytes).unwrap() == value);
    assert!(value
        .authorize_origin(&[1; 32], &[3; 32], origin().created_at)
        .is_err());
    let mut forged = value.clone();
    if let Part::Card(card) = &mut forged.parts[1] {
        card.id = [9; 32];
    }
    assert!(forged.to_bytes().is_err());
}
#[test]
fn code_escapes_redaction_and_malformed_blocks_do_not_create_interactive_content() {
    for source in [
        "`note::hidden;`",
        "redact::note::SECRET;;",
        "\\note::literal;",
        "poll::invalid note::nested;",
        "note::incomplete",
    ] {
        let draft = composition::parse(source, origin(), CardLimits::default(), None).unwrap();
        assert!(matches!(draft.content, Document::Text(_)), "{source}");
        if source.contains("SECRET") {
            assert!(!String::from_utf8(draft.content.to_bytes().unwrap())
                .unwrap()
                .contains("SECRET"));
        }
    }
    let draft = composition::parse(
        "redact::SECRET;\nnote::public;\n```\ntimer::5m;\n```",
        origin(),
        CardLimits::default(),
        None,
    )
    .unwrap();
    let Document::Composition(value) = draft.content else {
        panic!("missing public card");
    };
    assert_eq!(value.cards().count(), 1);
    assert!(!String::from_utf8(value.to_bytes().unwrap())
        .unwrap()
        .contains("SECRET"));
    let draft = composition::parse(
        "Before note::redact::SECRET; after",
        origin(),
        CardLimits::default(),
        None,
    )
    .unwrap();
    let Document::Composition(value) = draft.content else {
        panic!("missing redacted note");
    };
    assert_eq!(value.cards().count(), 1);
    assert!(!String::from_utf8(value.to_bytes().unwrap())
        .unwrap()
        .contains("SECRET"));
}
