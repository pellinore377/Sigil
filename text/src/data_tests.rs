use crate::{structured::Card, *};
fn origin() -> Origin<'static> {
    Origin {
        message: [1; 32],
        creator: [2; 32],
        created_at: 1_767_225_600,
        timezone: Some("UTC"),
    }
}
#[test]
fn data_cards_round_trip_with_safe_fallbacks_and_redacted_labels() {
    let cases=[
        "chart::pie::Parts\n- A = 20%\n- redact::secret = 80%;",
        "chart::donut::Parts\n- A = 20\n- B = 80;",
        "chart::bar::Values\n- red::A = -2\n- B = 4;",
        "chart::line::Values\n- A = 1\n- B = 4;",
        "chart::area::Values\n- A = 1\n- B = 4;",
        "chart::scatter::Values\n- -1 = 2\n- 3 = -4;",
        "diagram::flow::Flow\n- (Start) -> [Build]\n- Build -> {Test}\n- Test -> Build [retry];",
        "diagram::sequence::Flow\n- Client -> Server: send\n- Server --> Client: reply;",
        "diagram::timeline::Project\n- 2026-03 = Start\n- 2026-09 = Finish;",
        "diagram::mindmap::Root\n- Root -> A\n- Root -> B;",
        "diagram::org::Team\n- Lead -> A\n- Lead -> B;",
        "diagram::state::Call\n- Idle -> Ringing [invite]\n- Ringing -> Idle [decline];",
        "table::Name | Role\n- **A** | Designer\n- B;",
        "recipe::Dinner\nserves::4\ntime::25 min\ningredients:\n- 200g pasta\nsteps:\n- Cook pasta;",
        "remind::tomorrow 9:30am::redact::secret;",
        "timer::1h45m;",
        "countdown::2027-07-05::Event;",
        "ago::2019-03-14::Start;",
    ];
    for source in cases {
        let draft = parse_card(source, origin(), Default::default()).unwrap();
        let Parsed::Card(card) = draft.content else {
            panic!("literal: {source}");
        };
        let bytes = card.to_bytes().unwrap();
        assert!(*card == Card::from_bytes(&bytes).unwrap(), "{source}");
        assert!(!String::from_utf8_lossy(&bytes).contains("secret"));
        if source.starts_with("table") {
            assert!(card.html().unwrap().contains("<table>"));
        }
    }
}
#[test]
fn hostile_and_ambiguous_structures_stay_noninteractive() {
    for source in [
        "chart::pie::Empty\n- A = 0;",
        "chart::pie::Negative\n- A = -1;",
        "chart::bar::NaN\n- A = NaN;",
        "chart::scatter::Invalid\n- word = 1;",
        "chart::bar::Invalid\n- A\\=1;",
        "diagram::org::Cycle\n- A -> B\n- B -> A;",
        "diagram::org::Parents\n- A -> C\n- B -> C;",
        "diagram::flow::Empty\n- A -> ;",
        "table::A | B\n- 1 | 2 | 3;",
        "table:: | \n- 1 | 2;",
        "recipe::Invalid\nsteps:\n- Start;",
        "recipe::Invalid\ningredients:\n- A\ningredients:\n- B\nsteps:\n- Cook;",
        "timer::0h;",
        "remind::07/05/27::Ambiguous;",
    ] {
        let parsed = parse_card(source, origin(), Default::default()).unwrap();
        assert!(matches!(parsed.content, Parsed::Text(_)), "{source}");
        assert!(!parsed.hints.is_empty(), "{source}");
    }
}

#[test]
fn utility_results_are_fixed_validated_and_never_executed() {
    for source in [
        "calc::17 * 34;",
        "convert::5 miles;",
        "math::E = mc^2;",
        "math::block\n\\frac{a}{b};",
        "art::\n  /\\\n <  >\n;",
        "qr::https://example.org;",
        "qr::wifi::SyntheticNetwork::redact::secret;",
        "qr::text::redact::secret;",
        "roll::2d6, 1d20;",
        "pick::food;",
        "pick::flip;",
        "pick::number::-5--1;",
        "pick::a, a, b;",
        "swatch::#ff5733;",
        "swatch::rgb(255,87,51);",
        "swatch::rgba(255,87,51,0.5);",
        "swatch::hsl(9,100%,60%);",
        "kbd::Ctrl+Shift+P;",
        "rate::4/5;",
        "progress::120;",
        "quote::Author::Source::redact::secret;",
    ] {
        let draft = parse_card(source, origin(), Default::default()).unwrap();
        let Parsed::Card(card) = draft.content else {
            panic!("literal: {source}");
        };
        let bytes = card.to_bytes().unwrap();
        for _ in 0..3 {
            assert_eq!(Card::from_bytes(&bytes).unwrap().to_bytes().unwrap(), bytes);
        }
        assert!(!String::from_utf8_lossy(&bytes).contains("secret"));
        if source.starts_with("art") {
            assert!(card.html().unwrap().contains("<pre>"));
        }
        if let structured::Construct::Utility(utility::Utility::Qr(qr)) = &card.content {
            let (width, modules) = qr.modules().unwrap();
            assert_eq!(modules.len(), width * width);
            assert!(modules.iter().any(|v| *v));
            for i in 0..4 * width {
                assert!(!modules[i]);
                assert!(!modules[modules.len() - 1 - i]);
            }
        }
    }
    for source in [
        "calc::1/0;",
        "math::\\input{file};",
        "rate::6/5;",
        "rate::1/0;",
        "progress::NaN;",
        "qr::javascript:alert(1);",
        "kbd::Ctrl++;",
        "swatch::rgba(255,1,1,2);",
    ] {
        assert!(
            matches!(
                parse_card(source, origin(), Default::default())
                    .unwrap()
                    .content,
                Parsed::Text(_)
            ),
            "{source}"
        );
    }
}
