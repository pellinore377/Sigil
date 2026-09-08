use crate::text::{
    service::{language, Coordinates, Snapshot},
    Text,
};
use serde::{Deserialize, Serialize};
pub const MAX_BODY: usize = 64 * 1024;
pub const MAX_RESPONSE: usize = 256 * 1024;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    GoogleTranslate,
    LibreTranslate,
    Wiktionary,
    DictionaryIndex,
    OpenMeteo,
    Geocoder,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    pub id: String,
    pub kind: Kind,
    pub endpoint: String,
    pub attribution: Text,
    pub version: Option<String>,
    pub source_url: Option<String>,
}
pub fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
impl Provider {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !identifier(&self.id)
            || !self.endpoint.starts_with("https://")
            || self.endpoint.len() > 1000
            || self.attribution.body().is_empty()
            || self.attribution.body().len() > 512
            || self.version.as_ref().is_some_and(|v| v.len() > 64)
            || self
                .source_url
                .as_ref()
                .is_some_and(|v| !v.starts_with("https://") || v.len() > 1000)
        {
            return Err("invalid service provider");
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub revision: u64,
    pub providers: Vec<Provider>,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Place {
    pub name: String,
    pub coordinates: Coordinates,
    pub timezone: String,
    pub country: Option<String>,
    pub region: Option<String>,
}
impl Place {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.coordinates
            .validate()
            .map_err(|_| "invalid coordinates")?;
        if self.name.is_empty()
            || self.name.len() > 256
            || self.timezone.len() > 100
            || self.country.as_ref().is_some_and(|v| v.len() > 128)
            || self.region.as_ref().is_some_and(|v| v.len() > 256)
        {
            return Err("invalid place");
        }
        crate::text::time::Dated::new(
            Text::plain("place", Default::default()).map_err(|_| "invalid place")?,
            1,
            &self.timezone,
        )
        .map_err(|_| "invalid timezone")?;
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Query {
    Translate {
        text: Text,
        source: Option<String>,
        target: String,
    },
    Define {
        word: String,
        language: String,
    },
    Locate {
        name: String,
        language: String,
    },
    Weather {
        place: Place,
        forecast: bool,
    },
}
impl Query {
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Translate {
                text,
                source,
                target,
            } if text.body().is_empty()
                || text.body().len() > 8000
                || !language(target)
                || source.as_ref().is_some_and(|s| !language(s)) =>
            {
                Err("invalid translation query")
            }
            Self::Define {
                word,
                language: lang,
            }
            | Self::Locate {
                name: word,
                language: lang,
            } if word.trim().is_empty()
                || word.len() > 256
                || word.chars().any(char::is_control)
                || !language(lang) =>
            {
                Err("invalid service query")
            }
            Self::Weather { place, .. } => place.validate(),
            _ => Ok(()),
        }
    }
    pub fn compatible(&self, kind: Kind) -> bool {
        matches!(
            (self, kind),
            (
                Self::Translate { .. },
                Kind::GoogleTranslate | Kind::LibreTranslate
            ) | (
                Self::Define { .. },
                Kind::Wiktionary | Kind::DictionaryIndex
            ) | (Self::Locate { .. }, Kind::Geocoder)
                | (Self::Weather { .. }, Kind::OpenMeteo)
        )
    }
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Resolve {
    pub revision: u64,
    pub provider: Provider,
    pub query: Query,
    #[serde(default)]
    pub refresh: bool,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Resolved {
    Snapshot(Box<Snapshot>),
    Places(Vec<Place>),
}
impl Resolved {
    pub fn validate_for(&self, provider: &Provider, query: &Query) -> Result<(), &'static str> {
        self.validate()?;
        match (self, query) {
            (Self::Snapshot(snapshot), query) => {
                if snapshot.provider.id != provider.id
                    || snapshot.provider.version != provider.version
                    || snapshot.provider.attribution != provider.attribution
                    || snapshot.provider.source_url != provider.source_url
                {
                    return Err("provider changed");
                }
                match (&snapshot.content, query) {
                    (
                        crate::text::service::ResultData::Translation {
                            original,
                            source_language,
                            target_language,
                            ..
                        },
                        Query::Translate {
                            text,
                            source,
                            target,
                        },
                    ) if original == text
                        && target_language == target
                        && source.as_ref().is_none_or(|s| s == source_language) =>
                    {
                        Ok(())
                    }
                    (
                        crate::text::service::ResultData::Definition { word, language, .. },
                        Query::Define {
                            word: w,
                            language: l,
                        },
                    ) if word.body() == w && language == l => Ok(()),
                    (
                        crate::text::service::ResultData::Weather(weather),
                        Query::Weather { place, .. },
                    ) if weather.coordinates == place.coordinates
                        && weather.timezone == place.timezone
                        && weather.place.body() == place.name =>
                    {
                        Ok(())
                    }
                    _ => Err("result does not match query"),
                }
            }
            (Self::Places(_), Query::Locate { .. }) => Ok(()),
            _ => Err("result does not match query"),
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Snapshot(value) => value
                .validate(Default::default())
                .map_err(|_| "invalid service snapshot"),
            Self::Places(places) => {
                if places.len() > 10 {
                    return Err("too many places");
                }
                for place in places {
                    place.validate()?;
                }
                Ok(())
            }
        }
    }
}
