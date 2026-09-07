use crate::{structured::*, *};
fn origin() -> Origin<'static> {
    Origin {
        message: [1; 32],
        creator: [2; 32],
        created_at: 1_767_225_600,
        timezone: Some("America/Chicago"),
    }
}
fn card(source: &str) -> Card {
    let Parsed::Card(card) = parse_card(source, origin(), Default::default())
        .unwrap()
        .content
    else {
        panic!("not a card")
    };
    card
}
#[test]
fn cards_have_shared_builder_bytes_redacted_labels_and_authenticated_origins() {
    let source =
        "checklist::task::Move-in\n-x- red::First &amp; second\n- redact::hidden material;";
    let parsed = card(source);
    let built = Card {
        id: origin().message,
        creator: origin().creator,
        created_at: origin().created_at,
        content: Construct::Checklist(Checklist {
            title: parse("Move-in", Default::default()).unwrap(),
            mode: ListMode::Task,
            items: vec![
                ListItem {
                    id: item_id(&origin().message, 0),
                    text: parse("red::First &amp; second", Default::default()).unwrap(),
                    checked: true,
                    persistent: false,
                },
                ListItem {
                    id: item_id(&origin().message, 1),
                    text: parse("redact::hidden material", Default::default()).unwrap(),
                    checked: false,
                    persistent: false,
                },
            ],
        }),
    };
    assert!(parsed == built);
    let bytes = parsed.to_bytes().unwrap();
    assert_eq!(bytes, built.to_bytes().unwrap());
    assert!(!String::from_utf8_lossy(&bytes).contains("hidden material"));
    assert!(Card::from_bytes(&bytes).unwrap() == parsed);
    assert!(Text::from_bytes(&bytes).is_err());
    parsed
        .authorize_origin(&origin().message, &origin().creator, origin().created_at)
        .unwrap();
    assert!(parsed
        .authorize_origin(&[3; 32], &origin().creator, origin().created_at)
        .is_err());
    assert!(parsed
        .authorize_origin(&origin().message, &[3; 32], origin().created_at)
        .is_err());
    assert!(parsed
        .authorize_origin(
            &origin().message,
            &origin().creator,
            origin().created_at + 1
        )
        .is_err());
    assert_ne!(item_id(&[1; 32], 0), item_id(&[1; 32], 1));
    assert_ne!(item_id(&[1; 32], 0), item_id(&[2; 32], 0));
}
#[test]
fn structured_grammar_preserves_code_escapes_and_line_scoped_effects() {
    let parsed = card("checklist::recurr::monthly::Groceries\n-r- bold::Milk\\; eggs\n- `red::literal;`\n- [link](https://example.invalid/a;b);");
    let Construct::Checklist(list) = &parsed.content else {
        panic!()
    };
    assert!(matches!(list.mode, ListMode::Recurring(_)));
    assert!(list.items[0].persistent);
    assert_eq!(list.items[0].text.body(), "Milk; eggs");
    assert!(list.items[0].text.spans()[0].effects.bold);
    assert_eq!(list.items[1].text.body(), "red::literal;");
    assert!(list.items[1].text.spans()[0].effects.code);
    assert!(Card::from_bytes(&parsed.to_bytes().unwrap()).unwrap() == parsed);
    assert!(parsed.body().unwrap().contains("Monthly, America/Chicago"));
    assert!(parsed.html().unwrap().contains("Monthly · America/Chicago"));
    for source in [
        "`note::literal;`",
        "```\nnote::literal;\n```",
        "\\note::literal;",
        "text\nnote::literal;",
    ] {
        assert!(matches!(
            parse_card(source, origin(), Default::default())
                .unwrap()
                .content,
            Parsed::Text(_)
        ));
    }
    for (source, hint) in [
        ("checklist::Title\n- redact::secret", Hint::Incomplete),
        ("checklist::Title\n- a;\n- b;", Hint::InvalidStructure),
        ("checklist::Title\n-r- a;", Hint::InvalidStructure),
    ] {
        let draft = parse_card(source, origin(), Default::default()).unwrap();
        assert_eq!(draft.hints, vec![hint]);
        let Parsed::Text(text) = draft.content else {
            panic!()
        };
        assert!(!text.body().contains("secret"));
    }
    let draft = parse_card(
        "checklist::recurr::weekly::Title\n-r- a;",
        Origin {
            timezone: None,
            ..origin()
        },
        Default::default(),
    )
    .unwrap();
    assert_eq!(draft.hints, vec![Hint::TimezoneRequired]);
    assert!(matches!(draft.content, Parsed::Text(_)));
}
#[test]
fn poll_options_are_order_independent_and_repeat_warnings_do_not_change_content() {
    let a = card("poll::closed::multi2::Choose\n- One\n- Two\n- Three;");
    let b = card("poll::multi2::closed::Choose\n- One\n- Two\n- Three;");
    assert_eq!(a.to_bytes().unwrap(), b.to_bytes().unwrap());
    let repeated = parse_card(
        "poll::open::multi::closed::multi2::Choose\n- One\n- Two\n- Three;",
        origin(),
        Default::default(),
    )
    .unwrap();
    assert_eq!(repeated.hints, vec![Hint::RepeatedOption]);
    let Parsed::Card(repeated) = repeated.content else {
        panic!()
    };
    assert!(repeated == a);
    let Construct::Poll(defaults) = card("poll::Choose\n- One\n- Two;").content else {
        panic!()
    };
    assert_eq!(defaults.disclosure, Disclosure::Open);
    assert_eq!(defaults.selection, Selection::Single);
    assert!(matches!(
        parse_card(
            "poll::multi1::Choose\n- One\n- Two;",
            origin(),
            Default::default()
        )
        .unwrap()
        .content,
        Parsed::Text(_)
    ));
}
#[test]
fn card_decoding_rejects_forged_fallback_unknown_fields_duplicate_ids_and_limits() {
    let parsed = card("poll::Choose\n- <script>\n- Two;");
    let bytes = parsed.to_bytes().unwrap();
    assert!(!parsed.html().unwrap().contains("<script>"));
    assert!(parsed.html().unwrap().contains("&lt;script&gt;"));
    let wire = String::from_utf8(bytes.clone()).unwrap();
    for forged in [
        wire.replacen("Poll: Choose", "Poll: forged", 1),
        wire.replacen("<strong>Poll</strong>", "<script>Poll</script>", 1),
        wire.replacen("\"version\":1", "\"version\":2", 1),
        wire.replacen("\"version\":1", "\"version\":1,\"extra\":true", 1),
        format!(" {wire}"),
    ] {
        assert!(Card::from_bytes(forged.as_bytes()).is_err());
    }
    let mut duplicate = parsed.clone();
    let Construct::Poll(poll) = &mut duplicate.content else {
        panic!()
    };
    poll.options[1].id = poll.options[0].id;
    assert!(duplicate.to_bytes().is_err());
    assert!(Card::from_bytes(&vec![b' '; MAX_WIRE_BYTES + 1]).is_err());
    assert!(parse_card(
        "checklist::Title\n- a\n- b;",
        origin(),
        CardLimits {
            items: 1,
            ..Default::default()
        }
    )
    .is_err());
    let note = card("note::bold::A note;");
    assert_eq!(note.body().unwrap(), "Note: A note");
    assert!(Card::from_bytes(&note.to_bytes().unwrap()).unwrap() == note);
}
