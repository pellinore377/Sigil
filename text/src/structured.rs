use crate::{recurrence::Recurrence, Error, Limits, Span, Text, MAX_WIRE_BYTES};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub type Id = [u8; 32];
#[derive(Clone, Copy)]
pub struct CardLimits {
    pub items: usize,
    pub options: usize,
    pub dice_sides: u32,
    pub text: Limits,
}
impl Default for CardLimits {
    fn default() -> Self {
        Self {
            items: 256,
            options: 64,
            dice_sides: 1_000_000,
            text: Limits::default(),
        }
    }
}
impl CardLimits {
    pub(crate) fn validate(self) -> Result<(), Error> {
        self.text.validate()?;
        if !(1..=256).contains(&self.items)
            || !(2..=64).contains(&self.options)
            || !(2..=1_000_000).contains(&self.dice_sides)
        {
            return Err(Error::Limit);
        }
        Ok(())
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Card {
    #[serde(with = "id")]
    pub id: Id,
    #[serde(with = "id")]
    pub creator: Id,
    pub created_at: u64,
    pub content: Construct,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Construct {
    Checklist(Checklist),
    Poll(Poll),
    Note(Note),
    Reminder(crate::time::Dated),
    Countdown(crate::time::Dated),
    Ago(crate::time::Dated),
    Timer(crate::time::Timer),
    Data(crate::data::Data),
    Utility(crate::utility::Utility),
    Contact(crate::contact::Contact),
    Service(Box<crate::service::Snapshot>),
    Location(crate::location::Share),
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Note {
    #[serde(with = "inline")]
    pub text: Text,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checklist {
    #[serde(with = "inline")]
    pub title: Text,
    pub mode: ListMode,
    pub items: Vec<ListItem>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "rule",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ListMode {
    Standard,
    Task,
    Recurring(Recurrence),
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListItem {
    #[serde(with = "id")]
    pub id: Id,
    #[serde(with = "inline")]
    pub text: Text,
    pub checked: bool,
    pub persistent: bool,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Poll {
    #[serde(with = "inline")]
    pub question: Text,
    pub disclosure: Disclosure,
    pub selection: Selection,
    pub options: Vec<PollOption>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disclosure {
    Open,
    Closed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "max",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Selection {
    Single,
    Unlimited,
    Capped(u16),
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PollOption {
    #[serde(with = "id")]
    pub id: Id,
    #[serde(with = "inline")]
    pub text: Text,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    body: String,
    format: String,
    formatted_body: String,
    sigil: Metadata,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    version: u8,
    unicode: String,
    card: Card,
}

fn label(text: &Text, bytes: usize) -> Result<(), Error> {
    if text.body().trim().is_empty() || text.body().contains('\n') || text.body().len() > bytes {
        return Err(Error::Invalid);
    }
    Ok(())
}
impl Card {
    pub fn validate(&self, limits: CardLimits) -> Result<(), Error> {
        limits.validate()?;
        if self.id == [0; 32]
            || self.creator == [0; 32]
            || self.created_at == 0
            || self.created_at > i64::MAX as u64
        {
            return Err(Error::Invalid);
        }
        let mut size = 0usize;
        let mut spans = 0usize;
        let mut add = |text: &Text| -> Result<(), Error> {
            size = size.checked_add(text.body().len()).ok_or(Error::Limit)?;
            spans = spans.checked_add(text.spans().len()).ok_or(Error::Limit)?;
            if size > limits.text.body_bytes || spans > limits.text.spans {
                return Err(Error::Limit);
            }
            Ok(())
        };
        match &self.content {
            Construct::Location(value) => {
                value.validate(self.created_at)?;
                add(&value.label)?;
            }
            Construct::Service(value) => {
                value.validate(limits)?;
                for text in value.texts() {
                    add(text)?;
                }
            }
            Construct::Contact(value) => {
                value.validate()?;
                add(&value.display_name)?;
            }
            Construct::Utility(value) => {
                value.validate(limits)?;
                for text in value.texts() {
                    add(text)?;
                }
            }
            Construct::Data(value) => {
                value.validate(limits)?;
                for text in value.texts() {
                    add(text)?;
                }
            }
            Construct::Reminder(value) | Construct::Countdown(value) | Construct::Ago(value) => {
                value.validate()?;
                add(&value.text)?;
            }
            Construct::Timer(value) => value.validate()?,
            Construct::Note(Note { text }) => {
                label(text, limits.text.body_bytes)?;
                add(text)?;
            }
            Construct::Checklist(list) => {
                label(&list.title, 512)?;
                add(&list.title)?;
                if list.items.is_empty() {
                    return Err(Error::Invalid);
                }
                if list.items.len() > limits.items {
                    return Err(Error::Limit);
                }
                if let ListMode::Recurring(rule) = &list.mode {
                    rule.validate()?;
                    if rule.anchor_at != self.created_at {
                        return Err(Error::Invalid);
                    }
                }
                let mut ids = BTreeSet::new();
                for item in &list.items {
                    if item.id == [0; 32]
                        || !ids.insert(item.id)
                        || (item.persistent && !matches!(list.mode, ListMode::Recurring(_)))
                    {
                        return Err(Error::Invalid);
                    }
                    label(&item.text, 2048)?;
                    add(&item.text)?;
                }
            }
            Construct::Poll(poll) => {
                label(&poll.question, 512)?;
                add(&poll.question)?;
                if poll.options.len() < 2 {
                    return Err(Error::Invalid);
                }
                if poll.options.len() > limits.options {
                    return Err(Error::Limit);
                }
                if matches!(poll.selection,Selection::Capped(n) if n<2 || n as usize>poll.options.len())
                {
                    return Err(Error::Invalid);
                }
                let mut ids = BTreeSet::new();
                for option in &poll.options {
                    if option.id == [0; 32] || !ids.insert(option.id) {
                        return Err(Error::Invalid);
                    }
                    label(&option.text, 2048)?;
                    add(&option.text)?;
                }
            }
        }
        Ok(())
    }
    pub fn body(&self) -> Result<String, Error> {
        self.validate(Default::default())?;
        let body = match &self.content {
            Construct::Service(value) => value.body()?,
            Construct::Location(value) => value.body(),
            Construct::Contact(value) => value.body()?,
            Construct::Utility(value) => value.body()?,
            Construct::Data(value) => value.body(),
            Construct::Reminder(value) | Construct::Countdown(value) | Construct::Ago(value) => {
                format!(
                    "{}: {} · {} ({})",
                    match self.content {
                        Construct::Reminder(_) => "Reminder",
                        Construct::Countdown(_) => "Countdown",
                        _ => "Since",
                    },
                    value.text.body(),
                    crate::time::timestamp(value.at)?,
                    value.timezone
                )
            }
            Construct::Timer(value) => format!(
                "Timer: {} seconds · {} — {}",
                value.ends_at - value.started_at,
                crate::time::timestamp(value.started_at)?,
                crate::time::timestamp(value.ends_at)?
            ),
            Construct::Note(Note { text }) => format!("Note: {}", text.body()),
            Construct::Checklist(list) => {
                let mut body = format!(
                    "{}: {}",
                    match list.mode {
                        ListMode::Standard => "Checklist",
                        ListMode::Task => "Tasks",
                        ListMode::Recurring(_) => "Recurring checklist",
                    },
                    list.title.body()
                );
                if let ListMode::Recurring(rule) = &list.mode {
                    body.push_str(&format!(" ({}, {})", rule.interval.name(), rule.timezone));
                }
                for item in &list.items {
                    body.push_str(if item.checked { "\n[x] " } else { "\n[ ] " });
                    body.push_str(item.text.body());
                    if item.persistent {
                        body.push_str(" (persistent)")
                    }
                }
                body
            }
            Construct::Poll(poll) => {
                let mut body = format!("Poll: {}", poll.question.body());
                for option in &poll.options {
                    body.push_str("\n• ");
                    body.push_str(option.text.body())
                }
                body
            }
        };
        if body.len() > Limits::default().body_bytes {
            return Err(Error::Limit);
        }
        Ok(body)
    }
    pub fn html(&self) -> Result<String, Error> {
        self.validate(Default::default())?;
        let mut html = String::from("<div>");
        match &self.content {
            Construct::Contact(value) => {
                html.push_str(&Text::plain(&value.body()?, Limits::default())?.html())
            }
            Construct::Service(value) => html.push_str(&value.html()?),
            Construct::Location(value) => html.push_str(&value.html()?),
            Construct::Utility(value) => html.push_str(&value.html()?),
            Construct::Data(value) => html.push_str(&value.html()?),
            Construct::Reminder(_)
            | Construct::Countdown(_)
            | Construct::Ago(_)
            | Construct::Timer(_) => {
                html.push_str(&Text::plain(&self.body()?, Default::default())?.html())
            }
            Construct::Note(Note { text }) => {
                html.push_str("<p><strong>Note</strong></p>");
                html.push_str(&text.html());
            }
            Construct::Checklist(list) => {
                html.push_str(match list.mode {
                    ListMode::Standard => "<p><strong>Checklist</strong></p>",
                    ListMode::Task => "<p><strong>Tasks</strong></p>",
                    ListMode::Recurring(_) => "<p><strong>Recurring checklist</strong></p>",
                });
                html.push_str(&list.title.html());
                if let ListMode::Recurring(rule) = &list.mode {
                    html.push_str(
                        &Text::plain(
                            &format!("{} · {}", rule.interval.name(), rule.timezone),
                            Default::default(),
                        )?
                        .html(),
                    );
                }
                html.push_str("<ul>");
                for item in &list.items {
                    html.push_str(if item.checked { "<li>[x] " } else { "<li>[ ] " });
                    html.push_str(&item.text.html());
                    if item.persistent {
                        html.push_str("<p>Persistent</p>");
                    }
                    html.push_str("</li>")
                }
                html.push_str("</ul>");
            }
            Construct::Poll(poll) => {
                html.push_str("<p><strong>Poll</strong></p>");
                html.push_str(&poll.question.html());
                html.push_str("<ul>");
                for option in &poll.options {
                    html.push_str("<li>");
                    html.push_str(&option.text.html());
                    html.push_str("</li>")
                }
                html.push_str("</ul>");
            }
        }
        html.push_str("</div>");
        Ok(html)
    }
    pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let bytes = serde_json::to_vec(&Envelope {
            body: self.body()?,
            format: "text/html".into(),
            formatted_body: self.html()?,
            sigil: Metadata {
                version: 1,
                unicode: "17.0.0".into(),
                card: self.clone(),
            },
        })
        .map_err(|_| Error::Invalid)?;
        if bytes.len() > MAX_WIRE_BYTES {
            return Err(Error::Limit);
        }
        Ok(bytes)
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_WIRE_BYTES {
            return Err(Error::Limit);
        }
        let wire: Envelope = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
        if wire.sigil.version != 1 || wire.sigil.unicode != "17.0.0" {
            return Err(Error::Version);
        }
        let card = wire.sigil.card;
        if wire.format != "text/html"
            || wire.body != card.body()?
            || wire.formatted_body != card.html()?
            || card.to_bytes()?.as_slice() != bytes
        {
            return Err(Error::Invalid);
        }
        Ok(card)
    }
    pub fn authorize_origin(
        &self,
        message: &Id,
        creator: &Id,
        created_at: u64,
    ) -> Result<(), Error> {
        self.validate(Default::default())?;
        if &self.id != message || &self.creator != creator || self.created_at != created_at {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
pub(crate) mod inline {
    use super::*;
    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Value {
        body: String,
        spans: Vec<Span>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        blocks: Vec<crate::Block>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        mentions: Vec<crate::contact::Mention>,
    }
    pub fn serialize<S: serde::Serializer>(text: &Text, serializer: S) -> Result<S::Ok, S::Error> {
        Value {
            body: text.body().into(),
            spans: text.spans().to_vec(),
            blocks: text.blocks().to_vec(),
            mentions: text.mentions().to_vec(),
        }
        .serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Text, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Text::from_parts(value.body, value.spans)
            .and_then(|v| v.with_blocks(value.blocks))
            .and_then(|v| v.with_mentions(value.mentions))
            .map_err(serde::de::Error::custom)
    }
}
pub(crate) mod id {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(id: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error> {
        const HEX: &[u8] = b"0123456789abcdef";
        let text: String = id
            .iter()
            .flat_map(|b| {
                [
                    HEX[(b >> 4) as usize] as char,
                    HEX[(b & 15) as usize] as char,
                ]
            })
            .collect();
        serializer.serialize_str(&text)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<[u8; 32], D::Error> {
        let text = String::deserialize(deserializer)?;
        if text.len() != 64
            || !text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(serde::de::Error::custom("invalid identifier"));
        }
        let mut id = [0; 32];
        for (i, byte) in id.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16)
                .map_err(serde::de::Error::custom)?;
        }
        Ok(id)
    }
}
