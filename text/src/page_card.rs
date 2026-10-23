//! Page card content and the page grant it carries.
use crate::Error;
use serde::{Deserialize, Serialize};

pub type Id = [u8; 32];
pub const GRANT_VERSION: u8 = 1;
pub const MAX_GRANT_BYTES: usize = 256;
pub const MAX_TITLE_SCALARS: usize = 200;
pub const MAX_PREVIEW_SCALARS: usize = 280;
const FIXED: usize = 3 + 32 + 8 + 32 + 32;
pub const MAX_HOST_BYTES: usize = MAX_GRANT_BYTES - FIXED - 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageKind {
    Page,
    List,
}

/// Capability to read and append to one page epoch; `manage` also rotates and deletes.
#[derive(Clone, PartialEq, Eq)]
pub struct Grant {
    pub host: String,
    pub id: Id,
    pub epoch: u64,
    pub key: [u8; 32],
    pub access: [u8; 32],
    pub manage: Option<[u8; 32]>,
}
impl std::fmt::Debug for Grant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Grant")
            .field("host", &self.host)
            .field("epoch", &self.epoch)
            .finish_non_exhaustive()
    }
}
impl Grant {
    pub fn validate(&self) -> Result<(), Error> {
        if !valid_host(&self.host)
            || self.host.len() > MAX_HOST_BYTES
            || self.id == [0; 32]
            || self.epoch == 0
            || self.epoch > i64::MAX as u64
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    /// Version, flags, host length, id, epoch, key, access, optional manage, host.
    pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        self.validate()?;
        let mut bytes = Vec::with_capacity(MAX_GRANT_BYTES);
        bytes.push(GRANT_VERSION);
        bytes.push(u8::from(self.manage.is_some()));
        bytes.push(self.host.len() as u8);
        bytes.extend_from_slice(&self.id);
        bytes.extend_from_slice(&self.epoch.to_be_bytes());
        bytes.extend_from_slice(&self.key);
        bytes.extend_from_slice(&self.access);
        if let Some(manage) = &self.manage {
            bytes.extend_from_slice(manage);
        }
        bytes.extend_from_slice(self.host.as_bytes());
        Ok(bytes)
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_GRANT_BYTES || bytes.len() < FIXED || bytes[0] != GRANT_VERSION {
            return Err(Error::Invalid);
        }
        let take = |at: usize| -> Result<[u8; 32], Error> {
            bytes
                .get(at..at + 32)
                .and_then(|v| v.try_into().ok())
                .ok_or(Error::Invalid)
        };
        let (manage, host) = match bytes[1] {
            0 => (None, FIXED),
            1 => (Some(take(FIXED)?), FIXED + 32),
            _ => return Err(Error::Invalid),
        };
        let grant = Self {
            host: std::str::from_utf8(bytes.get(host..).ok_or(Error::Invalid)?)
                .map_err(|_| Error::Invalid)?
                .into(),
            id: take(3)?,
            epoch: u64::from_be_bytes(bytes[35..43].try_into().map_err(|_| Error::Invalid)?),
            key: take(43)?,
            access: take(75)?,
            manage,
        };
        if usize::from(bytes[2]) != grant.host.len() {
            return Err(Error::Invalid);
        }
        grant.validate()?;
        Ok(grant)
    }
    /// Same grant without the manage token.
    pub fn reader(&self) -> Self {
        Self {
            manage: None,
            ..self.clone()
        }
    }
}
impl Serialize for Grant {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let bytes = self.to_bytes().map_err(serde::ser::Error::custom)?;
        serializer.serialize_str(&bytes.iter().map(|b| format!("{b:02x}")).collect::<String>())
    }
}
impl<'de> Deserialize<'de> for Grant {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text.len() > MAX_GRANT_BYTES * 2
            || !text.len().is_multiple_of(2)
            || !text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(serde::de::Error::custom("invalid grant"));
        }
        let bytes = text
            .as_bytes()
            .chunks(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap_or("x"), 16))
            .collect::<Result<Vec<_>, _>>()
            .map_err(serde::de::Error::custom)?;
        Self::from_bytes(&bytes).map_err(serde::de::Error::custom)
    }
}

/// Timeline card for a page; summary fields follow its latest edit notice.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageCard {
    pub grant: Grant,
    pub title: String,
    pub preview: String,
    pub kind: PageKind,
    pub done: u32,
    pub total: u32,
}
impl PageCard {
    pub fn validate(&self) -> Result<(), Error> {
        self.grant.validate()?;
        validate_summary(&self.title, &self.preview, self.done, self.total)
    }
}

/// Title is one non-blank line; preview is at most two lines.
pub fn validate_summary(title: &str, preview: &str, done: u32, total: u32) -> Result<(), Error> {
    let plain = |text: &str, scalars: usize, lines: usize| {
        text.chars().count() <= scalars
            && text.split('\n').count() <= lines
            && !text.chars().any(|c| c != '\n' && c.is_control())
    };
    if title.trim().is_empty()
        || !plain(title, MAX_TITLE_SCALARS, 1)
        || !plain(preview, MAX_PREVIEW_SCALARS, 2)
        || done > total
    {
        return Err(Error::Invalid);
    }
    Ok(())
}

fn valid_host(name: &str) -> bool {
    name.contains('.')
        && name.parse::<std::net::IpAddr>().is_err()
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(crate) fn grant() -> Grant {
        Grant {
            host: "chat.example".into(),
            id: [1; 32],
            epoch: 1,
            key: [2; 32],
            access: [3; 32],
            manage: Some([4; 32]),
        }
    }
    #[test]
    fn grants_round_trip_strictly_within_256_bytes() {
        for value in [grant(), grant().reader()] {
            let bytes = value.to_bytes().unwrap();
            assert_eq!(Grant::from_bytes(&bytes).unwrap(), value);
            for n in 0..bytes.len() {
                assert!(Grant::from_bytes(&bytes[..n]).is_err(), "{n}");
            }
            let mut extra = bytes.clone();
            extra.push(b'a');
            assert!(Grant::from_bytes(&extra).is_err());
            let json = serde_json::to_string(&value).unwrap();
            assert_eq!(serde_json::from_str::<Grant>(&json).unwrap(), value);
            assert!(serde_json::from_str::<Grant>(&json.to_uppercase()).is_err());
        }
        let mut long = grant();
        long.host = (0..MAX_HOST_BYTES - 8)
            .map(|i| if i % 40 == 39 { '.' } else { 'a' })
            .chain(".example".chars())
            .collect();
        assert_eq!(long.to_bytes().unwrap().len(), MAX_GRANT_BYTES);
        long.host.insert(0, 'a');
        assert!(long.to_bytes().is_err());
        for host in [
            "",
            "localhost",
            "Chat.example",
            "chat.example:443",
            "127.0.0.1",
            "a..b",
        ] {
            let mut bad = grant();
            bad.host = host.into();
            assert!(bad.to_bytes().is_err(), "{host}");
        }
        let mut bad = grant();
        bad.epoch = 0;
        assert!(bad.to_bytes().is_err());
        bad.epoch = 1;
        bad.id = [0; 32];
        assert!(bad.to_bytes().is_err());
        let mut bytes = grant().to_bytes().unwrap();
        bytes[1] = 2;
        assert!(Grant::from_bytes(&bytes).is_err());
        bytes[1] = 1;
        bytes[0] = 2;
        assert!(Grant::from_bytes(&bytes).is_err());
        assert!(!format!("{:?}", grant()).contains("02, 2"));
    }
    #[test]
    fn page_summaries_bound_title_preview_and_progress() {
        assert!(validate_summary("Trip plan", "Saturday\nTrain at 9:10", 0, 0).is_ok());
        assert!(validate_summary("Groceries", "", 1, 2).is_ok());
        assert!(validate_summary(&"t".repeat(200), &"p".repeat(280), 2, 2).is_ok());
        for (title, preview, done, total) in [
            (" ", "", 0, 0),
            ("a\nb", "", 0, 0),
            (&"t".repeat(201) as &str, "", 0, 0),
            ("t", "a\nb\nc", 0, 0),
            ("t", &"p".repeat(281) as &str, 0, 0),
            ("t\u{7}", "", 0, 0),
            ("t", "\r", 0, 0),
            ("t", "", 3, 2),
        ] {
            assert!(validate_summary(title, preview, done, total).is_err());
        }
    }
    #[test]
    fn page_cards_round_trip_and_reject_bad_grants() {
        use crate::structured::{Card, Construct};
        let card = Card {
            id: [5; 32],
            creator: [6; 32],
            created_at: 10,
            content: Construct::Page(PageCard {
                grant: grant().reader(),
                title: "Trip plan".into(),
                preview: "Saturday\nTrain at 9:10".into(),
                kind: PageKind::Page,
                done: 0,
                total: 0,
            }),
        };
        let bytes = card.to_bytes().unwrap();
        assert!(Card::from_bytes(&bytes).unwrap() == card);
        assert_eq!(
            card.body().unwrap(),
            "Page: Trip plan\nSaturday\nTrain at 9:10"
        );
        let text = String::from_utf8(bytes).unwrap();
        assert!(!text.contains(&"04".repeat(32)));
        let hex: String = grant()
            .reader()
            .to_bytes()
            .unwrap()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert!(text.contains(&hex));
        assert!(Card::from_bytes(text.replace(&hex, &hex[..hex.len() - 2]).as_bytes()).is_err());
        let mut invalid = card.clone();
        if let Construct::Page(value) = &mut invalid.content {
            value.done = 1;
        }
        assert!(invalid.to_bytes().is_err());
        assert!(crate::editing::validate_shape(&card.content, 10).is_err());
    }
}
