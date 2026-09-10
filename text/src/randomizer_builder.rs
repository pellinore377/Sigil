use crate::{structured::CardLimits, utility::dice_plan, Error};
use std::collections::BTreeSet;

pub fn source(input: &str) -> Result<String, Error> {
    let limits = CardLimits::default();
    if input.len() > limits.text.source_bytes / 2
        || input.chars().any(|c| c.is_control() && c != '\n')
    {
        return Err(Error::Limit);
    }
    let mut fields = input.split('\n');
    let kind = fields.next().ok_or(Error::Invalid)?;
    let values = fields.collect::<Vec<_>>();
    let source = match kind {
        "Dice" => {
            if values.is_empty()
                || !values.len().is_multiple_of(2)
                || values.len() > limits.items * 2
            {
                return Err(Error::Invalid);
            }
            let groups = values
                .as_chunks::<2>()
                .0
                .iter()
                .map(|v| {
                    let count = v[0].trim().parse::<usize>().map_err(|_| Error::Invalid)?;
                    let sides = v[1].trim().parse::<u32>().map_err(|_| Error::Invalid)?;
                    Ok(format!("{count}d{sides}"))
                })
                .collect::<Result<Vec<_>, Error>>()?
                .join(", ");
            dice_plan(&groups, limits)?;
            format!("roll::{groups};")
        }
        "Choice" => {
            if values.len() > limits.items {
                return Err(Error::Limit);
            }
            let mut seen = BTreeSet::new();
            let choices = values
                .into_iter()
                .map(str::trim)
                .filter(|v| !v.is_empty() && seen.insert(*v))
                .map(|v| {
                    let mut literal = String::new();
                    for ch in v.chars() {
                        if ch.is_ascii_punctuation() {
                            literal.push('\\');
                        }
                        literal.push(ch);
                    }
                    literal
                })
                .collect::<Vec<_>>();
            if choices.len() < 2 {
                return Err(Error::Invalid);
            }
            format!("pick::{};", choices.join(", "))
        }
        "Number" => {
            if values.len() != 2 {
                return Err(Error::Invalid);
            }
            let min = values[0]
                .trim()
                .parse::<i64>()
                .map_err(|_| Error::Invalid)?;
            let max = values[1]
                .trim()
                .parse::<i64>()
                .map_err(|_| Error::Invalid)?;
            if !(1..=u64::MAX as i128).contains(&(i128::from(max) - i128::from(min) + 1)) {
                return Err(Error::Invalid);
            }
            format!("pick::number::{min}-{max};")
        }
        "Coin" if values.is_empty() => "pick::flip;".into(),
        _ => return Err(Error::Invalid),
    };
    if source.len() > limits.text.source_bytes {
        return Err(Error::Limit);
    }
    Ok(source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        parse_card,
        structured::Construct,
        utility::{Randomizer, Utility},
        Origin, Parsed,
    };
    fn resolve(source: &str) -> Randomizer {
        let Parsed::Card(card) = parse_card(
            source,
            Origin {
                message: [1; 32],
                creator: [2; 32],
                created_at: 1767225600,
                timezone: Some("UTC"),
            },
            Default::default(),
        )
        .unwrap()
        .content
        else {
            panic!("Expected a card")
        };
        let Construct::Utility(Utility::Random(value)) = card.content else {
            panic!("Expected a randomizer")
        };
        value
    }
    #[test]
    fn builders_resolve_through_canonical_syntax_without_executing_choice_text() {
        let inputs = [
            "Fish, chips",
            "redact::keep this;",
            "**literal**",
            "C:\\notes",
            "👩🏽‍💻 שלום",
            "[link](https://example.test)",
        ];
        let draft = format!("Choice\n{}\nFish, chips", inputs.join("\n"));
        let output = source(&draft).unwrap();
        assert_eq!(source(&draft).unwrap(), output);
        let Randomizer::Pick {
            options, selected, ..
        } = resolve(&output)
        else {
            panic!()
        };
        assert_eq!(options.iter().map(|v| v.body()).collect::<Vec<_>>(), inputs);
        assert!(usize::from(selected) < inputs.len());
        assert!(options
            .iter()
            .all(|v| v.spans().iter().all(|s| s.effects == Default::default())));
        let Randomizer::Dice { groups } = resolve(&source("Dice\n2\n6\n1\n20").unwrap()) else {
            panic!()
        };
        assert_eq!(
            groups
                .iter()
                .map(|g| (g.faces.len(), g.sides))
                .collect::<Vec<_>>(),
            vec![(2, 6), (1, 20)]
        );
        let Randomizer::Number { min, max, selected } = resolve(&source("Number\n-5\n-1").unwrap())
        else {
            panic!()
        };
        assert_eq!((min, max), (-5, -1));
        assert!((-5..=-1).contains(&selected));
        let Randomizer::Pick {
            category, options, ..
        } = resolve(&source("Coin").unwrap())
        else {
            panic!()
        };
        assert_eq!(category.as_deref(), Some("flip"));
        assert_eq!(options.len(), 2);
    }
    #[test]
    fn invalid_or_excessive_parameters_do_not_produce_sendable_source() {
        for input in [
            "Dice",
            "Dice\n0\n6",
            "Dice\n257\n6",
            "Dice\n1\n1000001",
            "Dice\n1\n6\n1",
            "Choice\nSame\nSame",
            "Choice\nA\nB\r",
            "Coin\nextra",
            "Number\n5\n1",
            "Number\n-9223372036854775808\n9223372036854775807",
        ] {
            assert!(source(input).is_err(), "{input}");
        }
        assert!(source(&"x".repeat(17000)).is_err());
    }
}
