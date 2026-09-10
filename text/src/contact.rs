use crate::{structured::Id, Error, Limits, Text};
use serde::{Deserialize, Serialize};
use unicode_normalization::is_nfc;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contact {
    #[serde(with = "crate::structured::id")]
    pub user_id: Id,
    pub address: String,
    pub display_name: Text,
    pub avatar_url: Option<String>,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mention {
    pub start: u32,
    pub end: u32,
    #[serde(with = "crate::structured::id")]
    pub user_id: Id,
    pub display: String,
    pub address: String,
}

pub fn valid_address(address: &str) -> bool {
    let Some((user, server)) = address.strip_prefix('@').and_then(|v| v.split_once(':')) else {
        return false;
    };
    !user.is_empty()
        && user.len() <= 128
        && !server.is_empty()
        && server.len() <= 253
        && user
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        && server.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

impl Contact {
    pub fn validate(&self) -> Result<(), Error> {
        if self.user_id == [0; 32]
            || !valid_address(&self.address)
            || self.display_name.body().is_empty()
            || self.display_name.body().len() > 512
            || self.display_name.body().chars().any(char::is_control)
            || !self.display_name.mentions().is_empty()
        {
            return Err(Error::Invalid);
        }
        if let Some(url) = &self.avatar_url {
            crate::Effects {
                link: Some(url.clone()),
                ..Default::default()
            }
            .validate()?;
            if !url.starts_with("https://") {
                return Err(Error::Invalid);
            }
        }
        Ok(())
    }
    pub fn handle(&self) -> &str {
        self.address.split(':').next().unwrap_or(&self.address)
    }
    pub fn presentation(&self) -> Result<serde_json::Value, Error> {
        self.validate()?;
        let concealed = self.display_name.spans().iter().any(|span| span.effects.reveal.is_some());
        Ok(serde_json::json!({"address":self.address,"name":self.display_name.presentation(),
            "identity":self.user_id.iter().map(|b|format!("{b:02x}")).collect::<String>(),
            "vcard":if concealed {None} else {Some(self.vcard()?)}}))
    }
    pub fn body(&self) -> Result<String, Error> {
        self.validate()?;
        Ok(format!(
            "Contact: {} ({})",
            self.display_name.body(),
            self.address
        ))
    }
    /// RFC 6350 text properties; identity extensions are claims, never trust proofs.
    pub fn vcard(&self) -> Result<String, Error> {
        self.validate()?;
        let identity = self
            .user_id
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let lines = [
            "BEGIN:VCARD".into(),
            "VERSION:4.0".into(),
            format!("FN:{}", escape(self.display_name.body())),
            format!("X-SIGIL-ADDRESS:{}", escape(&self.address)),
            format!("X-SIGIL-IDENTITY:{identity}"),
            format!("NOTE:Sigil {} {identity}", escape(&self.address)),
            "END:VCARD".into(),
        ];
        let mut out = String::new();
        for line in lines {
            let mut width = 0;
            for ch in line.chars() {
                if width + ch.len_utf8() > 75 {
                    out.push_str("\r\n ");
                    width = 1;
                }
                out.push(ch);
                width += ch.len_utf8();
            }
            out.push_str("\r\n");
        }
        Ok(out)
    }
    pub fn from_vcard(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > 16384 {
            return Err(Error::Limit);
        }
        let mut unfolded = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i..].starts_with(b"\r\n ") || bytes[i..].starts_with(b"\r\n\t") {
                i += 3;
            } else {
                unfolded.push(bytes[i]);
                i += 1;
            }
        }
        let source = std::str::from_utf8(&unfolded).map_err(|_| Error::Invalid)?;
        let mut lines = source.split_terminator("\r\n");
        if lines.next() != Some("BEGIN:VCARD") || lines.next() != Some("VERSION:4.0") {
            return Err(Error::Invalid);
        }
        let mut name = None;
        let mut address = None;
        let mut identity = None;
        let mut ended = false;
        for line in lines {
            if ended {
                return Err(Error::Invalid);
            }
            if line == "END:VCARD" {
                ended = true;
                continue;
            }
            let (key, value) = line.split_once(':').ok_or(Error::Invalid)?;
            let key = key
                .split(';')
                .next()
                .ok_or(Error::Invalid)?
                .rsplit('.')
                .next()
                .ok_or(Error::Invalid)?;
            let slot = match key.to_ascii_uppercase().as_str() {
                "FN" => &mut name,
                "X-SIGIL-ADDRESS" => &mut address,
                "X-SIGIL-IDENTITY" => &mut identity,
                "BEGIN" | "VERSION" | "END" => return Err(Error::Invalid),
                _ => continue,
            };
            if slot.replace(unescape(value)?).is_some() {
                return Err(Error::Invalid);
            }
        }
        if !ended {
            return Err(Error::Invalid);
        }
        let identity = identity.ok_or(Error::Invalid)?;
        if identity.len() != 64 || !identity.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Invalid);
        }
        let mut user_id = [0; 32];
        for (index, byte) in user_id.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&identity[index * 2..index * 2 + 2], 16)
                .map_err(|_| Error::Invalid)?;
        }
        let contact = Self {
            user_id,
            address: address.ok_or(Error::Invalid)?,
            display_name: Text::plain(&name.ok_or(Error::Invalid)?, Limits::default())?,
            avatar_url: None,
        };
        contact.validate()?;
        Ok(contact)
    }
}
fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace(';', "\\;")
        .replace(',', "\\,")
}
fn unescape(value: &str) -> Result<String, Error> {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        out.push(if c == '\\' {
            match chars.next() {
                Some('n' | 'N') => '\n',
                Some(c @ ('\\' | ';' | ',')) => c,
                _ => return Err(Error::Invalid),
            }
        } else {
            c
        });
    }
    Ok(out)
}

/// Only the caller's known contacts/current participants are searched. No network lookup.
pub fn resolve<'a>(query: &str, known: &'a [Contact]) -> Result<Vec<&'a Contact>, Error> {
    if query.len() > 384 || known.len() > 16384 {
        return Err(Error::Limit);
    }
    let mut matches = Vec::new();
    for contact in known {
        contact.validate()?;
        if (query == contact.address || query == contact.handle())
            && !matches
                .iter()
                .any(|other: &&Contact| other.user_id == contact.user_id)
        {
            matches.push(contact);
        }
    }
    Ok(matches)
}
pub fn parse_card(
    source: &str,
    origin: crate::Origin<'_>,
    limits: crate::structured::CardLimits,
    known: &[Contact],
) -> Result<Option<crate::Draft>, Error> {
    limits.validate()?;
    if source.len() > limits.text.source_bytes {
        return Err(Error::Limit);
    }
    let source = source.trim_end();
    let (query, qr) = if let Some(query) = source.strip_prefix("@::") {
        (query, false)
    } else if let Some(query) = source.strip_prefix("qr::contact::") {
        (query, true)
    } else {
        return Ok(None);
    };
    let Some(query) = query.strip_suffix(';') else {
        return Ok(Some(crate::Draft {
            content: crate::Parsed::Text(crate::parse(source, limits.text)?),
            hints: vec![crate::Hint::IdentityRequired],
        }));
    };
    let matches = resolve(query, known)?;
    if matches.len() != 1 {
        return Ok(Some(crate::Draft {
            content: crate::Parsed::Text(crate::parse(source, limits.text)?),
            hints: vec![if matches.len() > 1 {
                crate::Hint::AmbiguousIdentity
            } else {
                crate::Hint::IdentityRequired
            }],
        }));
    }
    let contact = matches[0];
    let content = if qr {
        crate::structured::Construct::Utility(crate::utility::Utility::Qr(
            crate::utility::Qr::Contact {
                address: contact.address.clone(),
                identity: contact.user_id,
            },
        ))
    } else {
        crate::structured::Construct::Contact(contact.clone())
    };
    let card = crate::structured::Card {
        id: origin.message,
        creator: origin.creator,
        created_at: origin.created_at,
        content,
    };
    card.validate(limits)?;
    Ok(Some(crate::Draft {
        content: crate::Parsed::Card(Box::new(card)),
        hints: Vec::new(),
    }))
}
pub(crate) fn validate_mentions(text: &Text, mentions: &[Mention]) -> Result<(), Error> {
    if mentions.len() > 256 {
        return Err(Error::Limit);
    }
    let offsets: Vec<_> = text
        .body()
        .grapheme_indices(true)
        .map(|(i, _)| i)
        .chain(std::iter::once(text.body().len()))
        .collect();
    let mut end = 0;
    for mention in mentions {
        if mention.start < end
            || mention.start >= mention.end
            || mention.end as usize >= offsets.len()
            || mention.user_id == [0; 32]
            || !valid_address(&mention.address)
            || !is_nfc(&mention.display)
            || !mention.display.starts_with('@')
            || mention.display.chars().any(char::is_control)
            || text.body()[offsets[mention.start as usize]..offsets[mention.end as usize]]
                != mention.display
            || text.spans().iter().any(|s| {
                s.start < mention.end
                    && mention.start < s.end
                    && (s.effects.code || s.effects.link.is_some())
            })
        {
            return Err(Error::Invalid);
        }
        end = mention.end;
    }
    Ok(())
}
