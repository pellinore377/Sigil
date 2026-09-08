use crate::{egress, service_config::StoredEntry};
use serde_json::{json, Value};
use sigil_protocol::{
    services::{Kind, Place, Query, Resolved},
    text::{
        service::{Provider, ResultData, Sense, Snapshot},
        Text,
    },
};
use zeroize::Zeroizing;
#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
#[path = "service_weather.rs"]
mod weather;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Error {
    Configuration,
    Unavailable,
    Invalid,
    Limit,
}
pub(super) fn text(value: &str) -> Result<Text, Error> {
    Text::plain(value, Default::default()).map_err(|_| Error::Limit)
}
pub(super) fn string(value: &Value) -> Result<&str, Error> {
    value.as_str().ok_or(Error::Invalid)
}
pub(super) fn encode(value: &str) -> String {
    let mut result = String::new();
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            result.push(char::from(b));
        } else {
            use std::fmt::Write;
            write!(result, "%{b:02X}").unwrap();
        }
    }
    result
}
fn html(value: &str) -> Result<Text, Error> {
    if value.len() > 8192 {
        return Err(Error::Limit);
    }
    let plain = html2text::config::plain_no_decorate()
        .link_footnotes(false)
        .string_from_read(value.as_bytes(), 4096)
        .map_err(|_| Error::Invalid)?;
    text(plain.trim())
}
pub(super) fn fetch(entry: &StoredEntry, url: &str, body: Option<&Value>) -> Result<Value, Error> {
    let bytes = Zeroizing::new(
        body.map(serde_json::to_vec)
            .transpose()
            .map_err(|_| Error::Invalid)?
            .unwrap_or_default(),
    );
    let mut request = ureq::http::Request::builder()
        .method(if body.is_some() { "POST" } else { "GET" })
        .uri(url)
        .header("accept", "application/json")
        .header("accept-encoding", "identity");
    if body.is_some() {
        request = request.header("content-type", "application/json");
    }
    if let Some(secret) = &entry.secret {
        match entry.provider.kind {
            Kind::GoogleTranslate => request = request.header("x-goog-api-key", secret.as_str()),
            Kind::LibreTranslate => (),
            _ => request = request.header("authorization", format!("Bearer {}", secret.as_str())),
        }
    }
    let request = request
        .body(bytes.as_slice())
        .map_err(|_| Error::Configuration)?;
    let response = egress::Policy::new(entry.exceptions.clone())
        .map_err(|_| Error::Configuration)?
        .service(request)
        .map_err(|e| match e {
            egress::Error::Limit => Error::Limit,
            egress::Error::InvalidEndpoint | egress::Error::Policy => Error::Configuration,
            _ => Error::Unavailable,
        })?;
    if response.status != 200 {
        return Err(Error::Unavailable);
    }
    if response
        .content_type
        .as_ref()
        .is_none_or(|v| v.split(';').next().map(str::trim) != Some("application/json"))
    {
        return Err(Error::Invalid);
    }
    serde_json::from_slice(&response.body).map_err(|_| Error::Invalid)
}
pub(crate) fn resolve(entry: &StoredEntry, query: &Query, now: u64) -> Result<Resolved, Error> {
    query.validate().map_err(|_| Error::Invalid)?;
    if !query.compatible(entry.provider.kind) {
        return Err(Error::Configuration);
    }
    let endpoint = entry.provider.endpoint.trim_end_matches('/');
    let content = match query {
        Query::Translate {
            text: original,
            source,
            target,
        } => {
            let mut body = json!({"q":original.body(),"target":target,"format":"text"});
            if let Some(source) = source {
                body["source"] = json!(source);
            } else if entry.provider.kind == Kind::LibreTranslate {
                body["source"] = json!("auto");
            }
            if entry.provider.kind == Kind::LibreTranslate {
                if let Some(secret) = &entry.secret {
                    body["api_key"] = json!(secret.as_str());
                }
            }
            let value = fetch(entry, endpoint, Some(&body))?;
            let (translated, detected) = if entry.provider.kind == Kind::GoogleTranslate {
                let translations = value["data"]["translations"]
                    .as_array()
                    .filter(|v| v.len() == 1)
                    .ok_or(Error::Invalid)?;
                (
                    html_escape::decode_html_entities(string(&translations[0]["translatedText"])?)
                        .into_owned(),
                    translations[0]["detectedSourceLanguage"].as_str(),
                )
            } else {
                (
                    string(&value["translatedText"])?.into(),
                    value["detectedLanguage"]["language"].as_str(),
                )
            };
            let source_language = source
                .as_deref()
                .or(detected)
                .ok_or(Error::Invalid)?
                .to_owned();
            ResultData::Translation {
                original: original.clone(),
                source_language,
                target_language: target.clone(),
                detected: source.is_none(),
                translated: text(&translated)?,
            }
        }
        Query::Define { word, language } => {
            if entry.provider.kind == Kind::DictionaryIndex {
                let value = fetch(
                    entry,
                    endpoint,
                    Some(&json!({"word":word,"language":language})),
                )?;
                let data: ResultData = serde_json::from_value(value).map_err(|_| Error::Invalid)?;
                if !matches!(&data,ResultData::Definition{word:w,language:l,..} if w.body()==word && l==language)
                {
                    return Err(Error::Invalid);
                }
                data
            } else {
                let value = fetch(entry, &format!("{endpoint}/{}", encode(word)), None)?;
                let entries = value[language].as_array().ok_or(Error::Invalid)?;
                let mut senses = Vec::new();
                for item in entries.iter().take(32) {
                    for definition in item["definitions"]
                        .as_array()
                        .ok_or(Error::Invalid)?
                        .iter()
                        .take(32 - senses.len())
                    {
                        senses.push(Sense {
                            part_of_speech: text(string(&item["partOfSpeech"])?)?,
                            definition: html(string(&definition["definition"])?)?,
                            example: definition["examples"]
                                .as_array()
                                .and_then(|v| v.first())
                                .map(|v| html(string(v)?))
                                .transpose()?,
                            etymology: None,
                            synonyms: Vec::new(),
                            antonyms: Vec::new(),
                        });
                    }
                    if senses.len() == 32 {
                        break;
                    }
                }
                ResultData::Definition {
                    word: text(word)?,
                    language: language.clone(),
                    pronunciation: None,
                    audio_url: None,
                    senses,
                }
            }
        }
        Query::Locate { name, language } => {
            let value = fetch(
                entry,
                &format!(
                    "{endpoint}?name={}&language={}&count=10&format=json",
                    encode(name),
                    encode(language)
                ),
                None,
            )?;
            let mut places = Vec::new();
            if let Some(results) = value.get("results") {
                for place in results
                    .as_array()
                    .filter(|v| v.len() <= 10)
                    .ok_or(Error::Invalid)?
                {
                    places.push(Place {
                        name: string(&place["name"])?.into(),
                        coordinates: weather::coordinates(&place["latitude"], &place["longitude"])?,
                        timezone: string(&place["timezone"])?.into(),
                        country: place["country"].as_str().map(str::to_owned),
                        region: place["admin1"].as_str().map(str::to_owned),
                    });
                }
            }
            let result = Resolved::Places(places);
            result.validate().map_err(|_| Error::Invalid)?;
            return Ok(result);
        }
        Query::Weather { place, forecast } => {
            ResultData::Weather(weather::resolve(entry, place, *forecast)?)
        }
    };
    let provider = Provider {
        id: entry.provider.id.clone(),
        version: entry.provider.version.clone(),
        attribution: entry.provider.attribution.clone(),
        source_url: entry.provider.source_url.clone(),
    };
    let result = Resolved::Snapshot(Box::new(Snapshot {
        provider,
        resolved_at: now,
        content,
    }));
    result.validate().map_err(|_| Error::Invalid)?;
    if serde_json::to_vec(&result)
        .map_err(|_| Error::Invalid)?
        .len()
        > sigil_protocol::services::MAX_RESPONSE
    {
        return Err(Error::Limit);
    }
    Ok(result)
}
