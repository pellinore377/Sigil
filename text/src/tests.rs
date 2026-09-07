use super::*;
use crate::model::Run;
fn parsed(s: &str) -> Text {
    parse(s, Limits::default()).unwrap()
}
fn check_wire(t: &Text) {
    if !t.body().is_empty() {
        let b = t.to_bytes().unwrap();
        assert!(Text::from_bytes(&b).unwrap() == *t);
    }
}
#[test]
fn redaction_precedes_all_transmitted_representations() {
    for source in [
        "redact::SYNTHETIC_SECRET;",
        "bold::redact::**SYNTHETIC_SECRET**;",
        "redact::[label](https://SYNTHETIC_SECRET.example);",
        "redact::`SYNTHETIC_SECRET`;",
        "red::before redact::SYNTHETIC_SECRET;",
        "redact::SYNTHETIC_SECRET",
        "redact::SYNTHETIC_SECRET\\;still secret;",
        "checklist::List\n- redact::SYNTHETIC_SECRET\n- public;",
    ] {
        let text = parsed(source);
        assert!(!text.body().contains("SYNTHETIC_SECRET"), "{source}");
        let bytes = text.to_bytes().unwrap();
        assert!(!std::str::from_utf8(&bytes)
            .unwrap()
            .contains("SYNTHETIC_SECRET"));
        check_wire(&text);
    }
    assert_eq!(
        parsed("redact::secret\npublic").body(),
        "[REDACTED]\npublic"
    );
    assert_eq!(parsed("redact::;public").body(), "public");
    let text = parsed("[redact::secret;](https://private.example)");
    assert_eq!(text.body(), "[REDACTED]");
    assert!(!text.html().contains("private.example"));
    assert_eq!(
        parsed("redact::secret;\n\n[REDACTED]: https://private.example").body(),
        "[REDACTED]"
    );
}
#[test]
fn modifiers_escapes_and_code_have_distinct_grammars() {
    for (source, body) in [
        ("red::A;blue::B;", "AB"),
        ("red::text", "text"),
        ("red::;", ""),
        (r"red::a\;b;", "a;b"),
        (r"\red::text;", "red::text;"),
        (r"\\red::text;", r"\text"),
        ("std::vector", "std::vector"),
        ("small4::text;", "small4::text;"),
        ("`redact::secret;`", "redact::secret;"),
        ("```\nredact::secret;\n```", "redact::secret;\n"),
        ("red::`code`;", "code"),
        ("red&#58;&#58;text;", "red::text;"),
    ] {
        let t = parsed(source);
        assert_eq!(t.body(), body, "{source}");
        check_wire(&t);
    }
    assert!(parsed(r"\red::text;").spans().is_empty());
    assert_eq!(parsed("||text||").spans(), parsed("spoiler::text;").spans());
    assert_eq!(parsed("||redact::secret;||").body(), "[REDACTED]");
    assert!(parsed("red::`code`;").spans()[0].effects.paint.is_none());
    assert!(parsed("red::**bold** and *italic*;")
        .spans()
        .iter()
        .all(|s| s.effects.paint.is_some()));
    assert_eq!(
        parsed("shake::bold::red::text;").spans(),
        parsed("red::shake::bold::text;").spans()
    );
    let t = parsed("shake::wave::big3::small1::red1-blue3::mark::x;");
    let e = &t.spans()[0].effects;
    assert_eq!(e.animation, Some(Animation::Wave));
    assert_eq!(e.size, Some(-1));
    assert!(e.mark);
    assert!(
        matches!(&e.paint,Some(Paint::Gradient {stops:s}) if s.len()==2 && s[0].shade==1 && s[1].shade==3)
    );
}
#[test]
fn unicode_17_normalization_and_style_boundaries_are_grapheme_safe() {
    assert_eq!(unicode_normalization::UNICODE_VERSION, (17, 0, 0));
    assert_eq!(unicode_segmentation::UNICODE_VERSION, (17, 0, 0));
    let a = parsed("red::e\u{301} 👩🏽‍💻 🇺🇸;blue::Z;");
    let b = parsed("red::é 👩🏽‍💻 🇺🇸;blue::Z;");
    assert_eq!(a.body(), b.body());
    assert_eq!(a.spans(), b.spans());
    assert_eq!(a.spans()[0].end, 5);
    assert_eq!(a.spans()[1].start, 5);
    check_wire(&a);
    let t = Text::from_runs(
        &[
            Run {
                text: "e",
                effects: Effects::default(),
            },
            Run {
                text: "\u{301}",
                effects: Effects {
                    bold: true,
                    ..Default::default()
                },
            },
        ],
        Limits::default(),
    )
    .unwrap();
    assert_eq!(t.body(), "é");
    assert_eq!(t.spans()[0].start, 0);
    assert_eq!(t.spans()[0].end, 1);
    check_wire(&t);
    let t = parsed("redact::secret;blue::👩🏽‍💻;");
    assert_eq!(t.spans()[0].start, 10);
    assert_eq!(t.spans()[0].end, 11);
}
#[test]
fn canonical_decode_rejects_forged_fallbacks_ranges_and_effects() {
    let t = parsed("bold::A;blue::B;");
    let bytes = t.to_bytes().unwrap();
    let original = std::str::from_utf8(&bytes).unwrap();
    for (from, to) in [
        ("<p><strong>A</strong>B</p>", "<script>alert(1)</script>"),
        ("\"end\":1", "\"end\":100"),
        (
            "\"kind\":\"emphasis\",\"value\":\"bold\"",
            "\"kind\":\"size\",\"value\":4",
        ),
        (
            "\"kind\":\"emphasis\",\"value\":\"bold\"",
            "\"kind\":\"link\",\"value\":\"javascript:alert(1)\"",
        ),
        ("\"body\":\"AB\"", "\"body\":\"e\\u0301\""),
        ("17.0.0", "16.0.0"),
        ("\"version\":1", "\"version\":1,\"original\":\"secret\""),
        ("\"start\":1", "\"start\":0"),
        (
            "\"kind\":\"emphasis\",\"value\":\"bold\"",
            "\"kind\":\"redact\",\"value\":\"secret\"",
        ),
    ] {
        let altered = original.replace(from, to);
        assert_ne!(altered, original);
        assert!(Text::from_bytes(altered.as_bytes()).is_err());
    }
    let mut altered = bytes.clone();
    altered.push(b' ');
    assert!(Text::from_bytes(&altered).is_err());
    let html=parsed("<script>alert(1)</script>\n\n[x](javascript:alert(1)) ![alt](https://image.example/x) [web](https://web.example/?a=1&b=2)").html();
    assert!(!html.contains("<script>"));
    assert!(!html.contains("javascript:"));
    assert!(!html.contains("<img"));
    assert!(html.contains("&lt;script&gt;"));
    assert!(html.contains("noreferrer noopener"));
    assert!(html.contains("&amp;b=2"));
}
#[test]
fn limits_and_deterministic_malformed_input_do_not_produce_noncanonical_data() {
    assert_eq!(
        parse(&"a".repeat(32769), Limits::default()).err(),
        Some(Error::Limit)
    );
    assert_eq!(
        parse(
            "red::one; blue::two;",
            Limits {
                spans: 1,
                ..Default::default()
            }
        )
        .err(),
        Some(Error::Limit)
    );
    assert_eq!(
        parse(
            "x",
            Limits {
                source_bytes: 0,
                ..Default::default()
            }
        )
        .err(),
        Some(Error::Limit)
    );
    let deep = format!("{}text", "> ".repeat(40));
    assert_eq!(parse(&deep, Limits::default()).err(), Some(Error::Limit));
    let mut state = 0x1a07_7781u64;
    let tokens = [
        "red::",
        "redact::",
        ";",
        "\\;",
        "\\",
        "`",
        "**",
        "_",
        "[",
        "](",
        "<",
        ">",
        "\n",
        "é",
        "\u{301}",
        "👩🏽‍💻",
        "x",
        "::",
    ];
    for _ in 0..4096 {
        let mut source = String::new();
        for _ in 0..32 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            source.push_str(tokens[(state >> 32) as usize % tokens.len()]);
        }
        if let Ok(t) = parse(&source, Limits::default()) {
            check_wire(&t);
        }
    }
}
#[test]
fn removing_modifiers_cannot_change_markdown_structure() {
    assert_eq!(
        parsed("red::# literal heading;").body(),
        "# literal heading"
    );
    assert_eq!(parsed("red::> literal quote;").body(), "> literal quote");
    assert_eq!(parsed("red::- literal list;").body(), "- literal list");
    assert_eq!(
        parsed("redact::secret;[REDACTED]: https://private.example").body(),
        "[REDACTED][REDACTED]: https://private.example"
    );
}

#[test]
fn graphical_redaction_uses_the_same_canonical_body_and_discards_empty_styles() {
    let original = Text::plain("A👩🏽‍💻B", Limits::default()).unwrap();
    let redacted = original.redact_range(1..2, Limits::default()).unwrap();
    assert!(
        Text::plain("👩🏽‍💻", Limits::default())
            .unwrap()
            .redact_range(0..1, Limits::default())
            .unwrap()
            == parsed("redact::👩🏽‍💻;")
    );
    assert_eq!(redacted.body(), "A[REDACTED]B");
    check_wire(&redacted);
    let styled = parsed("bold::A👩🏽‍💻B;")
        .redact_range(1..2, Limits::default())
        .unwrap();
    assert_eq!(styled.spans()[0].end, 12);
    assert!(!styled
        .to_bytes()
        .unwrap()
        .windows("👩🏽‍💻".len())
        .any(|w| w == "👩🏽‍💻".as_bytes()));
    assert!(original.redact_range(1..4, Limits::default()).is_err());
    let linked = parsed("[SYNTHETIC_SECRET](https://SYNTHETIC_SECRET.example)");
    let redacted = linked.redact_range(0..16, Limits::default()).unwrap();
    assert!(!std::str::from_utf8(&redacted.to_bytes().unwrap())
        .unwrap()
        .contains("SYNTHETIC_SECRET"));
    let t = Text::from_runs(
        &[
            Run {
                text: "e",
                effects: Effects::default(),
            },
            Run {
                text: "",
                effects: Effects {
                    bold: true,
                    ..Default::default()
                },
            },
            Run {
                text: "\u{301}",
                effects: Effects::default(),
            },
        ],
        Limits::default(),
    )
    .unwrap();
    assert_eq!(t.body(), "é");
    assert!(t.spans().is_empty());
}

#[test]
fn entity_and_link_metadata_punctuation_cannot_terminate_text_modifiers() {
    let text = parsed("red::a &amp; b;");
    assert_eq!(text.body(), "a & b");
    assert_eq!(text.spans()[0].end, 5);
    let text = parsed("redact::[link](https://example.org/;metadata) SYNTHETIC_SECRET;");
    assert_eq!(text.body(), "[REDACTED]");
    let text = parsed("||[link](https://example.org/||metadata) visible||");
    assert_eq!(text.body(), "link visible");
    assert!(text
        .spans()
        .iter()
        .all(|s| s.effects.reveal == Some(Reveal::Spoiler)));
}

#[test]
fn wire_effects_are_resolved_ordered_descriptors_with_theme_color_names() {
    let a = parsed("shake::red1-blue3::bold::x;");
    let b = parsed("bold::red1-blue3::shake::x;");
    assert_eq!(a.to_bytes().unwrap(), b.to_bytes().unwrap());
    let wire: serde_json::Value = serde_json::from_slice(&a.to_bytes().unwrap()).unwrap();
    assert_eq!(
        wire["sigil"]["text_spans"][0]["effects"],
        serde_json::json!([
            {"kind":"color","value":{"type":"gradient","stops":["red1","blue3"]}},
            {"kind":"emphasis","value":"bold"},
            {"kind":"animation","value":"shake"}
        ])
    );
    for source in [
        "mark::x;",
        "mark::yellow::x;",
        "bold::italic::strike::underline::mono::mark::big3::wave::scratch::x;",
    ] {
        check_wire(&parsed(source));
    }
    let full = Text::from_runs(
        &[Run {
            text: "x",
            effects: Effects {
                bold: true,
                italic: true,
                strike: true,
                underline: true,
                mono: true,
                code: true,
                mark: true,
                paint: Some(Paint::Rainbow),
                size: Some(3),
                animation: Some(Animation::Wave),
                reveal: Some(Reveal::Scratch),
                link: Some("https://example.org".into()),
            },
        }],
        Limits::default(),
    )
    .unwrap();
    check_wire(&full);
    assert!(serde_json::from_str::<Effects>(
        r#"[{"kind":"color","value":{"type":"solid","color":"red"}}]"#
    )
    .is_err());
    assert!(serde_json::from_str::<Effects>(
        r#"[{"kind":"emphasis","value":"bold"},{"kind":"emphasis","value":"bold"}]"#
    )
    .is_err());
}
