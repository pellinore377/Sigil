use crate::{
    structured::{Card, CardLimits, Id},
    Error, Origin, Text, MAX_WIRE_BYTES,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Part {
    Text(Text),
    Card(Box<Card>),
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Composition {
    #[serde(with = "crate::structured::id")]
    pub id: Id,
    #[serde(with = "crate::structured::id")]
    pub creator: Id,
    pub created_at: u64,
    pub parts: Vec<Part>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
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
    composition: Composition,
}

pub fn card_id(message: &Id, ordinal: u32) -> Id {
    Sha256::digest(
        [
            b"Sigil/composition-card/v1".as_slice(),
            message,
            &ordinal.to_be_bytes(),
        ]
        .concat(),
    )
    .into()
}
impl Composition {
    pub fn validate(&self, limits: CardLimits) -> Result<(), Error> {
        limits.validate()?;
        if self.id == [0; 32]
            || self.creator == [0; 32]
            || self.created_at == 0
            || self.created_at > i64::MAX as u64
        {
            return Err(Error::Invalid);
        }
        if self.parts.is_empty() || self.parts.len() > 64 {
            return Err(Error::Limit);
        }
        let mut cards = 0;
        let mut previous_text = false;
        let mut bytes = 0usize;
        for part in &self.parts {
            match part {
                Part::Text(text) => {
                    if previous_text || text.body().is_empty() {
                        return Err(Error::Invalid);
                    }
                    bytes = bytes.checked_add(text.body().len()).ok_or(Error::Limit)?;
                    previous_text = true;
                }
                Part::Card(card) => {
                    card.validate(limits)?;
                    card.authorize_origin(
                        &card_id(&self.id, cards),
                        &self.creator,
                        self.created_at,
                    )?;
                    bytes = bytes.checked_add(card.body()?.len()).ok_or(Error::Limit)?;
                    cards += 1;
                    previous_text = false;
                }
            }
            if bytes > limits.text.body_bytes {
                return Err(Error::Limit);
            }
        }
        if cards == 0 {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    pub fn cards(&self) -> impl Iterator<Item = &Card> {
        self.parts.iter().filter_map(|part| {
            if let Part::Card(card) = part {
                Some(card.as_ref())
            } else {
                None
            }
        })
    }
    pub fn body(&self) -> Result<String, Error> {
        self.validate(CardLimits::default())?;
        self.parts
            .iter()
            .map(|part| match part {
                Part::Text(t) => Ok(t.body().to_owned()),
                Part::Card(c) => c.body(),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|parts| parts.join("\n"))
    }
    pub fn html(&self) -> Result<String, Error> {
        self.validate(CardLimits::default())?;
        self.parts
            .iter()
            .map(|part| match part {
                Part::Text(t) => Ok(t.html()),
                Part::Card(c) => c.html(),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|parts| parts.join(""))
    }
    pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let wire = Wire {
            body: self.body()?,
            format: "text/html".into(),
            formatted_body: self.html()?,
            sigil: Metadata {
                version: 1,
                unicode: "17.0.0".into(),
                composition: self.clone(),
            },
        };
        let bytes = serde_json::to_vec(&wire).map_err(|_| Error::Invalid)?;
        if bytes.len() > MAX_WIRE_BYTES {
            return Err(Error::Limit);
        }
        Ok(bytes)
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_WIRE_BYTES {
            return Err(Error::Limit);
        }
        let wire: Wire = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
        if wire.sigil.version != 1 || wire.sigil.unicode != "17.0.0" {
            return Err(Error::Version);
        }
        let composition = wire.sigil.composition;
        if composition.to_bytes()?.as_slice() != bytes {
            return Err(Error::Invalid);
        }
        Ok(composition)
    }
    pub fn authorize_origin(
        &self,
        message: &Id,
        creator: &Id,
        created_at: u64,
    ) -> Result<(), Error> {
        self.validate(CardLimits::default())?;
        if self.id != *message || self.creator != *creator || self.created_at != created_at {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}

pub struct Draft {
    pub content: crate::Document,
    pub hints: Vec<crate::Hint>,
}
pub fn parse(
    source: &str,
    origin: Origin<'_>,
    limits: CardLimits,
    order: Option<crate::time::DateOrder>,
) -> Result<Draft, Error> {
    parse_with_contacts(source, origin, limits, order, &[])
}
pub fn parse_with_contacts(
    source: &str,
    origin: Origin<'_>,
    limits: CardLimits,
    order: Option<crate::time::DateOrder>,
    known: &[crate::contact::Contact],
) -> Result<Draft, Error> {
    limits.validate()?;
    if source.len() > limits.text.source_bytes {
        return Err(Error::Limit);
    }
    if source.starts_with("help::") && source.trim_end().ends_with(';') {
        let draft = crate::parse_card_with_dates(source, origin, limits, order)?;
        if let crate::Parsed::Text(text) = draft.content {
            return Ok(Draft {
                content: crate::Document::Text(text),
                hints: draft.hints,
            });
        }
    }
    let (eligible, terminators) = crate::parse::construct_positions(source)?;
    let mut parts = Vec::new();
    let mut hints = Vec::new();
    let mut consumed = 0;
    let mut cards = 0;
    let mut scanned = 0;
    for start in 0..source.len() {
        if start < scanned
            || !source.is_char_boundary(start)
            || !eligible[start]
            || !crate::help::structured_prefix(&source[start..])
            || source[..start]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '\\')
        {
            continue;
        }
        let art = source[start..].starts_with("art::");
        let mut stop = None;
        for end in start..source.len() {
            if source.as_bytes()[end] != b';' || (!art && !terminators[end]) {
                continue;
            }
            if art
                && (!source[..end].ends_with('\n')
                    || source[end + 1..]
                        .chars()
                        .next()
                        .is_some_and(|c| c != '\n' && c != '\r'))
            {
                continue;
            }
            stop = Some(end + 1);
            break;
        }
        let Some(stop) = stop else {
            hints.push(crate::Hint::Incomplete);
            break;
        };
        scanned = stop;
        let child = Origin {
            message: card_id(&origin.message, cards),
            ..origin
        };
        let draft = match crate::contact::parse_card(&source[start..stop], child, limits, known)? {
            Some(draft) => draft,
            None => crate::parse_card_with_dates(&source[start..stop], child, limits, order)?,
        };
        hints.extend(draft.hints);
        if let crate::Parsed::Card(card) = draft.content {
            if consumed < start {
                let text = crate::parse(&source[consumed..start], limits.text)?;
                if !text.body().is_empty() {
                    parts.push(Part::Text(text));
                }
            }
            parts.push(Part::Card(card));
            consumed = stop;
            cards += 1;
            if parts.len() > 64 {
                return Err(Error::Limit);
            }
        }
    }
    if cards == 0 {
        return Ok(Draft {
            content: crate::Document::Text(crate::parse(source, limits.text)?),
            hints,
        });
    }
    if consumed < source.len() {
        let text = crate::parse(&source[consumed..], limits.text)?;
        if !text.body().is_empty() {
            parts.push(Part::Text(text));
        }
    }
    let composition = Composition {
        id: origin.message,
        creator: origin.creator,
        created_at: origin.created_at,
        parts,
    };
    composition.validate(limits)?;
    composition.to_bytes()?;
    Ok(Draft {
        content: crate::Document::Composition(composition),
        hints,
    })
}
