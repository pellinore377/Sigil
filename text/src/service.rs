use crate::{structured::CardLimits, Error, Text};
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    pub id: String,
    pub version: Option<String>,
    pub attribution: Text,
    pub source_url: Option<String>,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub provider: Provider,
    pub resolved_at: u64,
    pub content: ResultData,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ResultData {
    Translation {
        original: Text,
        source_language: String,
        target_language: String,
        detected: bool,
        translated: Text,
    },
    Definition {
        word: Text,
        language: String,
        pronunciation: Option<Text>,
        audio_url: Option<String>,
        senses: Vec<Sense>,
    },
    Weather(Weather),
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sense {
    pub part_of_speech: Text,
    pub definition: Text,
    pub example: Option<Text>,
    pub etymology: Option<Text>,
    pub synonyms: Vec<Text>,
    pub antonyms: Vec<Text>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Coordinates {
    pub latitude_e6: i32,
    pub longitude_e6: i32,
}
impl Coordinates {
    pub fn validate(self) -> Result<(), Error> {
        if !(-90_000_000..=90_000_000).contains(&self.latitude_e6)
            || !(-180_000_000..=180_000_000).contains(&self.longitude_e6)
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Weather {
    pub place: Text,
    pub coordinates: Coordinates,
    pub timezone: String,
    pub current: Conditions,
    pub days: Vec<ForecastDay>,
    pub hours: Vec<Conditions>,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Conditions {
    pub at: u64,
    pub temperature_mc: i32,
    pub feels_like_mc: Option<i32>,
    pub description: Text,
    pub wmo_code: u8,
    pub precipitation_um: Option<u32>,
    pub precipitation_chance: Option<u8>,
    pub humidity: Option<u8>,
    pub wind_mms: u32,
    pub wind_degrees: u16,
    pub uv_tenths: Option<u16>,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForecastDay {
    pub at: u64,
    pub low_mc: i32,
    pub high_mc: i32,
    pub wmo_code: u8,
    pub precipitation_chance: u8,
    pub description: Text,
}
fn label(text: &Text, max: usize) -> Result<(), Error> {
    if text.body().is_empty() || text.body().len() > max {
        return Err(Error::Invalid);
    }
    Ok(())
}
pub fn language(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 35
        && value
            .split('-')
            .next()
            .is_some_and(|part| part.bytes().all(|b| b.is_ascii_alphabetic()))
        && value.split('-').all(|part| {
            !part.is_empty() && part.len() <= 8 && part.bytes().all(|b| b.is_ascii_alphanumeric())
        })
}
fn timestamp(at: u64) -> Result<(), Error> {
    jiff::Timestamp::from_second(i64::try_from(at).map_err(|_| Error::Invalid)?)
        .map_err(|_| Error::Invalid)?;
    if at == 0 {
        return Err(Error::Invalid);
    }
    Ok(())
}
fn temperature(value: i32) -> bool {
    (-150_000..=100_000).contains(&value)
}
fn url(value: &str) -> Result<(), Error> {
    if !value.starts_with("https://") {
        return Err(Error::Invalid);
    }
    crate::Effects {
        link: Some(value.into()),
        ..Default::default()
    }
    .validate()
}
impl Conditions {
    fn validate(&self) -> Result<(), Error> {
        timestamp(self.at)?;
        label(&self.description, 512)?;
        if !temperature(self.temperature_mc)
            || self.feels_like_mc.is_some_and(|v| !temperature(v))
            || self.wmo_code > 99
            || self.precipitation_um.is_some_and(|v| v > 10_000_000)
            || self.precipitation_chance.is_some_and(|v| v > 100)
            || self.humidity.is_some_and(|v| v > 100)
            || self.wind_mms > 200_000
            || self.wind_degrees > 360
            || self.uv_tenths.is_some_and(|v| v > 1000)
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
impl Snapshot {
    pub fn validate(&self, limits: CardLimits) -> Result<(), Error> {
        limits.validate()?;
        timestamp(self.resolved_at)?;
        if self.provider.id.is_empty()
            || self.provider.id.len() > 64
            || !self
                .provider
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            || self
                .provider
                .version
                .as_ref()
                .is_some_and(|v| v.is_empty() || v.len() > 128 || v.chars().any(char::is_control))
        {
            return Err(Error::Invalid);
        }
        label(&self.provider.attribution, 2048)?;
        if let Some(source) = &self.provider.source_url {
            url(source)?;
        }
        match &self.content {
            ResultData::Translation {
                original,
                source_language,
                target_language,
                translated,
                ..
            } => {
                label(original, limits.text.body_bytes)?;
                label(translated, limits.text.body_bytes)?;
                if !language(source_language) || !language(target_language) {
                    return Err(Error::Invalid);
                }
            }
            ResultData::Definition {
                word,
                language: lang,
                pronunciation,
                audio_url,
                senses,
            } => {
                label(word, 512)?;
                if !language(lang) || senses.is_empty() || senses.len() > 32 {
                    return Err(Error::Invalid);
                }
                if let Some(value) = pronunciation {
                    label(value, 512)?;
                }
                if let Some(value) = audio_url {
                    url(value)?;
                }
                for sense in senses {
                    label(&sense.part_of_speech, 64)?;
                    label(&sense.definition, 4096)?;
                    if sense.synonyms.len() > 32 || sense.antonyms.len() > 32 {
                        return Err(Error::Limit);
                    }
                    for value in [&sense.example, &sense.etymology].into_iter().flatten() {
                        label(value, 2048)?;
                    }
                    for value in sense.synonyms.iter().chain(&sense.antonyms) {
                        label(value, 256)?;
                    }
                }
            }
            ResultData::Weather(weather) => {
                label(&weather.place, 512)?;
                weather.coordinates.validate()?;
                crate::recurrence::zone(&weather.timezone)?;
                weather.current.validate()?;
                if weather.days.len() > 14 || weather.hours.len() > 168 {
                    return Err(Error::Limit);
                }
                let mut previous = 0;
                for day in &weather.days {
                    timestamp(day.at)?;
                    label(&day.description, 512)?;
                    if day.at <= previous
                        || !temperature(day.low_mc)
                        || !temperature(day.high_mc)
                        || day.low_mc > day.high_mc
                        || day.wmo_code > 99
                        || day.precipitation_chance > 100
                    {
                        return Err(Error::Invalid);
                    }
                    previous = day.at;
                }
                previous = 0;
                for hour in &weather.hours {
                    hour.validate()?;
                    if hour.at <= previous {
                        return Err(Error::Invalid);
                    }
                    previous = hour.at;
                }
            }
        }
        let texts = self.texts();
        if texts.iter().map(|t| t.body().len()).sum::<usize>() > limits.text.body_bytes
            || texts.iter().map(|t| t.spans().len()).sum::<usize>() > limits.text.spans
        {
            return Err(Error::Limit);
        }
        Ok(())
    }
    pub fn texts(&self) -> Vec<&Text> {
        let mut texts = vec![&self.provider.attribution];
        match &self.content {
            ResultData::Translation {
                original,
                translated,
                ..
            } => texts.extend([original, translated]),
            ResultData::Definition {
                word,
                pronunciation,
                senses,
                ..
            } => {
                texts.push(word);
                texts.extend(pronunciation);
                for sense in senses {
                    texts.extend([&sense.part_of_speech, &sense.definition]);
                    texts.extend(&sense.example);
                    texts.extend(&sense.etymology);
                    texts.extend(&sense.synonyms);
                    texts.extend(&sense.antonyms);
                }
            }
            ResultData::Weather(weather) => {
                texts.extend([&weather.place, &weather.current.description]);
                texts.extend(weather.days.iter().map(|v| &v.description));
                texts.extend(weather.hours.iter().map(|v| &v.description));
            }
        }
        texts
    }
    pub fn body(&self) -> Result<String, Error> {
        self.validate(Default::default())?;
        let body = match &self.content {
            ResultData::Translation {
                original,
                source_language,
                target_language,
                translated,
                ..
            } => format!(
                "{}\nOriginal ({source_language} → {target_language}): {}",
                translated.body(),
                original.body()
            ),
            ResultData::Definition { word, senses, .. } => format!(
                "{}\n{}",
                word.body(),
                senses
                    .iter()
                    .map(|sense| format!(
                        "{}: {}",
                        sense.part_of_speech.body(),
                        sense.definition.body()
                    ))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
            ResultData::Weather(weather) => format!(
                "{}: {:.1} °C, {}",
                weather.place.body(),
                f64::from(weather.current.temperature_mc) / 1000.0,
                weather.current.description.body()
            ),
        };
        Ok(format!(
            "{body}\n{} · Snapshot {}",
            self.provider.attribution.body(),
            self.resolved_at
        ))
    }
    pub fn html(&self) -> Result<String, Error> {
        Text::plain(&self.body()?, Default::default()).map(|v| v.html())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn text(value: &str) -> Text {
        Text::plain(value, Default::default()).unwrap()
    }
    fn provider() -> Provider {
        Provider {
            id: "synthetic".into(),
            version: Some("1".into()),
            attribution: text("Example provider"),
            source_url: Some("https://example.org/source".into()),
        }
    }
    #[test]
    fn snapshots_are_canonical_bounded_and_render_without_provider_requests() {
        let snapshot = Snapshot {
            provider: provider(),
            resolved_at: 1800000000,
            content: ResultData::Translation {
                original: crate::parse("Hello redact::SECRET;", Default::default()).unwrap(),
                source_language: "en".into(),
                target_language: "es".into(),
                detected: true,
                translated: text("Hola <script>alert(1)</script>"),
            },
        };
        let card = crate::structured::Card {
            id: [1; 32],
            creator: [2; 32],
            created_at: 1800000001,
            content: crate::structured::Construct::Service(Box::new(snapshot.clone())),
        };
        let bytes = card.to_bytes().unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("SECRET"));
        assert!(!card.html().unwrap().contains("<script>"));
        assert!(crate::structured::Card::from_bytes(&bytes).unwrap() == card);
        let mut invalid = snapshot;
        invalid.provider.source_url = Some("file:///etc/passwd".into());
        assert!(invalid.validate(Default::default()).is_err());
        assert!(Coordinates {
            latitude_e6: 90_000_001,
            longitude_e6: 0
        }
        .validate()
        .is_err());
        let current = Conditions {
            at: 1800000000,
            temperature_mc: 20100,
            feels_like_mc: Some(19000),
            description: text("Clear"),
            wmo_code: 0,
            precipitation_um: Some(0),
            precipitation_chance: Some(10),
            humidity: Some(50),
            wind_mms: 2500,
            wind_degrees: 90,
            uv_tenths: Some(20),
        };
        let mut weather = Snapshot {
            provider: provider(),
            resolved_at: 1800000000,
            content: ResultData::Weather(Weather {
                place: text("Example City"),
                coordinates: Coordinates {
                    latitude_e6: 47000000,
                    longitude_e6: -122000000,
                },
                timezone: "UTC".into(),
                current: current.clone(),
                days: vec![],
                hours: vec![current.clone()],
            }),
        };
        assert!(weather.body().unwrap().contains("20.1 °C"));
        if let ResultData::Weather(value) = &mut weather.content {
            value.hours.push(current);
        }
        assert!(weather.validate(Default::default()).is_err());
    }
}
