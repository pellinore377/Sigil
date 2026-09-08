//! Standalone structured messages. Labels use the same canonical inline parser.
use crate::{
    recurrence::{Interval, Recurrence},
    structured::*,
    Error, Text,
};
use pulldown_cmark::{Event, Parser};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy)]
pub struct Origin<'a> {
    pub message: Id,
    pub creator: Id,
    pub created_at: u64,
    pub timezone: Option<&'a str>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hint {
    Incomplete,
    InvalidStructure,
    TimezoneRequired,
    RepeatedOption,
    ConfirmTime,
    IdentityRequired,
    AmbiguousIdentity,
}
pub enum Parsed {
    Text(Text),
    Card(Box<Card>),
}
pub struct Draft {
    pub content: Parsed,
    pub hints: Vec<Hint>,
}

/// IDs are stable across retries and shared by syntax and graphical builders.
pub fn item_id(message: &Id, ordinal: u32) -> Id {
    Sha256::digest(
        [
            b"Sigil/structured-item/v1".as_slice(),
            message,
            &ordinal.to_be_bytes(),
        ]
        .concat(),
    )
    .into()
}
fn escaped(source: &str, at: usize) -> bool {
    source.as_bytes()[..at]
        .iter()
        .rev()
        .take_while(|&&b| b == b'\\')
        .count()
        % 2
        == 1
}
// CommonMark text events exclude code, entity punctuation and link destinations.
fn terminators(source: &str) -> Vec<usize> {
    let mut result = Vec::new();
    for (event, range) in Parser::new(source).into_offset_iter() {
        if let Event::Text(text) = event {
            if &source[range.clone()] == text.as_ref() {
                result
                    .extend(range.filter(|&i| source.as_bytes()[i] == b';' && !escaped(source, i)));
            }
        }
    }
    result
}
fn literal(source: &str, limits: CardLimits, hint: Option<Hint>) -> Result<Draft, Error> {
    Ok(Draft {
        content: Parsed::Text(crate::parse(source, limits.text)?),
        hints: hint.into_iter().collect(),
    })
}
/// Incomplete/invalid syntax stays literal, with redaction still applied.
pub fn parse_card(source: &str, origin: Origin<'_>, limits: CardLimits) -> Result<Draft, Error> {
    parse_card_with_dates(source, origin, limits, None)
}
pub fn parse_card_with_dates(
    source: &str,
    origin: Origin<'_>,
    limits: CardLimits,
    order: Option<crate::time::DateOrder>,
) -> Result<Draft, Error> {
    limits.validate()?;
    if source.len() > limits.text.source_bytes {
        return Err(Error::Limit);
    }
    let trimmed = source.trim_end();
    if let Some(draft) = crate::contact::parse_card(source, origin, limits, &[])? {
        return Ok(draft);
    }
    if let Some(query) = trimmed
        .strip_prefix("help::")
        .and_then(|s| s.strip_suffix(';'))
    {
        return match crate::help::sheet(query, limits.text) {
            Ok(text) => Ok(Draft {
                content: Parsed::Text(text),
                hints: Vec::new(),
            }),
            Err(Error::Invalid) => literal(source, limits, Some(Hint::InvalidStructure)),
            Err(error) => Err(error),
        };
    }
    let recognized = crate::help::structured_prefix(trimmed);
    if !recognized {
        return literal(source, limits, None);
    }
    parse_recognized(source, trimmed, origin, limits, order)
}
fn parse_recognized(
    source: &str,
    trimmed: &str,
    origin: Origin<'_>,
    limits: CardLimits,
    order: Option<crate::time::DateOrder>,
) -> Result<Draft, Error> {
    let mut lines: Vec<&str> = trimmed.lines().collect();
    if lines.is_empty() {
        return literal(source, limits, None);
    }
    let mut terminated = false;
    for (i, line) in lines.iter().enumerate() {
        if trimmed.starts_with("art::") && i + 1 < lines.len() {
            continue;
        }
        let positions = terminators(line);
        for position in positions {
            if i + 1 == lines.len() && position + 1 == line.len() && !terminated {
                terminated = true;
            } else {
                return literal(source, limits, Some(Hint::InvalidStructure));
            }
        }
    }
    if !terminated {
        return literal(source, limits, Some(Hint::Incomplete));
    }
    let last = lines.last_mut().ok_or(Error::Invalid)?;
    *last = &last[..last.len() - 1];
    let mut hints = Vec::new();
    let label = |text: &str| crate::parse(text, limits.text);
    let content = if let Some(text) = lines[0].strip_prefix("note::") {
        if lines.len() != 1 {
            return literal(source, limits, Some(Hint::InvalidStructure));
        }
        Construct::Note(Note { text: label(text)? })
    } else if [
        "calc::",
        "convert::",
        "math::",
        "art::",
        "qr::",
        "roll::",
        "pick::",
        "kbd::",
        "rate::",
        "progress::",
        "quote::",
        "swatch::",
    ]
    .iter()
    .any(|p| lines[0].starts_with(p))
    {
        match crate::utility::parse(&lines, limits) {
            Ok(value) => Construct::Utility(value),
            Err(Error::Limit) => return Err(Error::Limit),
            Err(_) => return literal(source, limits, Some(Hint::InvalidStructure)),
        }
    } else if ["chart::", "diagram::", "table::", "recipe::"]
        .iter()
        .any(|p| lines[0].starts_with(p))
    {
        match crate::data::parse(&lines, limits) {
            Ok(value) => Construct::Data(value),
            Err(Error::Limit) => return Err(Error::Limit),
            Err(_) => return literal(source, limits, Some(Hint::InvalidStructure)),
        }
    } else if ["remind::", "timer::", "countdown::", "ago::"]
        .iter()
        .any(|p| lines[0].starts_with(p))
    {
        if lines.len() != 1 {
            return literal(source, limits, Some(Hint::InvalidStructure));
        }
        let (kind, rest) = lines[0].split_once("::").ok_or(Error::Invalid)?;
        let result = if kind == "timer" {
            crate::time::duration(rest)
                .and_then(|duration| crate::time::Timer::new(origin.created_at, duration))
                .map(Construct::Timer)
        } else {
            let Some(zone) = origin.timezone else {
                return literal(source, limits, Some(Hint::TimezoneRequired));
            };
            let Some((date, text)) = rest.split_once("::") else {
                return literal(source, limits, Some(Hint::InvalidStructure));
            };
            crate::time::resolve_date(date, origin.created_at, zone, order)
                .and_then(|at| crate::time::Dated::new(label(text)?, at, zone))
                .map(|value| match kind {
                    "remind" => Construct::Reminder(value),
                    "countdown" => Construct::Countdown(value),
                    _ => Construct::Ago(value),
                })
        };
        match result {
            Ok(content) => {
                hints.push(Hint::ConfirmTime);
                content
            }
            Err(Error::Limit) => return Err(Error::Limit),
            Err(_) => return literal(source, limits, Some(Hint::InvalidStructure)),
        }
    } else if let Some(mut title) = lines[0].strip_prefix("checklist::") {
        let mode = if let Some(rest) = title.strip_prefix("task::") {
            title = rest;
            ListMode::Task
        } else if let Some(rest) = title.strip_prefix("recurr::") {
            let Some((interval, rest)) = rest.split_once("::") else {
                return literal(source, limits, Some(Hint::InvalidStructure));
            };
            let interval = match interval {
                "weekly" => Interval::Weekly,
                "monthly" => Interval::Monthly,
                "yearly" => Interval::Yearly,
                _ => return literal(source, limits, Some(Hint::InvalidStructure)),
            };
            let Some(timezone) = origin.timezone else {
                return literal(source, limits, Some(Hint::TimezoneRequired));
            };
            title = rest;
            ListMode::Recurring(Recurrence::new(interval, timezone, origin.created_at)?)
        } else {
            ListMode::Standard
        };
        if lines.len().saturating_sub(1) > limits.items {
            return Err(Error::Limit);
        }
        let mut items = Vec::new();
        for (i, row) in lines[1..].iter().enumerate() {
            let (text, checked, persistent) = if let Some(text) = row.strip_prefix("-x- ") {
                (text, true, false)
            } else if let Some(text) = row.strip_prefix("-r- ") {
                (text, false, true)
            } else if let Some(text) = row.strip_prefix("- ") {
                (text, false, false)
            } else {
                return literal(source, limits, Some(Hint::InvalidStructure));
            };
            items.push(ListItem {
                id: item_id(&origin.message, i as u32),
                text: label(text)?,
                checked,
                persistent,
            });
        }
        Construct::Checklist(Checklist {
            title: label(title)?,
            mode,
            items,
        })
    } else {
        let mut question = lines[0].strip_prefix("poll::").ok_or(Error::Invalid)?;
        let mut disclosure = Disclosure::Open;
        let mut selection = Selection::Single;
        let (mut seen_disclosure, mut seen_selection) = (false, false);
        while let Some((option, rest)) = question.split_once("::") {
            let repeated = match option {
                "open" | "closed" => {
                    disclosure = if option == "open" {
                        Disclosure::Open
                    } else {
                        Disclosure::Closed
                    };
                    std::mem::replace(&mut seen_disclosure, true)
                }
                "multi" => {
                    selection = Selection::Unlimited;
                    std::mem::replace(&mut seen_selection, true)
                }
                value
                    if value.strip_prefix("multi").is_some_and(|n| {
                        !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())
                    }) =>
                {
                    let digits = &value[5..];
                    let cap = digits.parse::<u16>().map_err(|_| Error::Limit)?;
                    if cap < 2 || digits.starts_with('0') {
                        return literal(source, limits, Some(Hint::InvalidStructure));
                    }
                    selection = Selection::Capped(cap);
                    std::mem::replace(&mut seen_selection, true)
                }
                _ => break,
            };
            if repeated && !hints.contains(&Hint::RepeatedOption) {
                hints.push(Hint::RepeatedOption);
            }
            question = rest;
        }
        if lines.len().saturating_sub(1) > limits.options {
            return Err(Error::Limit);
        }
        let mut options = Vec::new();
        for (i, row) in lines[1..].iter().enumerate() {
            let Some(text) = row.strip_prefix("- ") else {
                return literal(source, limits, Some(Hint::InvalidStructure));
            };
            options.push(PollOption {
                id: item_id(&origin.message, i as u32),
                text: label(text)?,
            });
        }
        Construct::Poll(Poll {
            question: label(question)?,
            disclosure,
            selection,
            options,
        })
    };
    let card = Card {
        id: origin.message,
        creator: origin.creator,
        created_at: origin.created_at,
        content,
    };
    match card.validate(limits) {
        Ok(()) => Ok(Draft {
            content: Parsed::Card(Box::new(card)),
            hints,
        }),
        Err(Error::Invalid) => literal(source, limits, Some(Hint::InvalidStructure)),
        Err(error) => Err(error),
    }
}
