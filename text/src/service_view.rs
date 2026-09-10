use crate::{
    Error, Text,
    service::{Conditions, ResultData, Snapshot},
};
use serde_json::{Value, json};

fn copy(text: &Text) -> Option<&str> {
    text.spans()
        .iter()
        .all(|s| s.effects.reveal.is_none())
        .then(|| text.body())
}
fn temperature(value: i32) -> Value {
    json!([
        format!("{:.1} °C", f64::from(value) / 1000.0),
        format!("{:.1} °F", f64::from(value) * 0.0018 + 32.0)
    ])
}
fn date(at: u64, zone: &jiff::tz::TimeZone, pattern: &str) -> Result<String, Error> {
    Ok(
        jiff::Timestamp::from_second(at.try_into().map_err(|_| Error::Invalid)?)
            .map_err(|_| Error::Invalid)?
            .to_zoned(zone.clone())
            .strftime(pattern)
            .to_string(),
    )
}
fn conditions(c: &Conditions, zone: &jiff::tz::TimeZone) -> Result<Value, Error> {
    let direction = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"]
        [((u32::from(c.wind_degrees) + 22) / 45 % 8) as usize];
    Ok(
        json!({"at":c.at,"date":date(c.at,zone,"%a, %b %-d · %-I:%M %p %Z")?,"temperature":temperature(c.temperature_mc),
        "feels_like":c.feels_like_mc.map(temperature),"description":c.description.presentation(),"icon":icon(c.wmo_code),
        "rain":c.precipitation_um.map(|v|format!("{:.1} mm",f64::from(v)/1000.0)),"chance":c.precipitation_chance.map(|v|format!("{v}%")),
        "wind":[format!("{:.1} km/h {direction}",f64::from(c.wind_mms)*0.0036),format!("{:.1} mph {direction}",f64::from(c.wind_mms)*0.0022369362920544)],
        "humidity":c.humidity.map(|v|format!("{v}%")),"uv":c.uv_tenths.map(|v|format!("{:.1}",f64::from(v)/10.0))}),
    )
}
fn icon(code: u8) -> &'static str {
    match code {
        0 => "sunny",
        1..=3 => "partly_cloudy_day",
        45 | 48 => "foggy",
        51..=67 | 80..=82 => "rainy",
        71..=77 | 85 | 86 => "weather_snowy",
        95..=99 => "thunderstorm",
        _ => "cloud",
    }
}
impl Snapshot {
    pub fn presentation(&self, now: u64) -> Result<Value, Error> {
        self.validate(Default::default())?;
        let zone = match &self.content {
            ResultData::Weather(w) => crate::recurrence::zone(&w.timezone)?,
            _ => jiff::tz::TimeZone::UTC,
        };
        let mut value = json!({"attribution":self.provider.attribution.presentation(),"source":self.provider.source_url,
            "stamp":date(self.resolved_at,&zone,"%b %-d, %Y · %-I:%M %p %Z")?});
        match &self.content {
            ResultData::Translation {
                original,
                source_language,
                target_language,
                detected,
                translated,
            } => {
                value["kind"] = json!("translation");
                value["title"] = json!(translated.presentation());
                value["original"] = json!(original.presentation());
                value["language"] = json!(format!(
                    "{source_language}{} → {target_language}",
                    if *detected { " (detected)" } else { "" }
                ));
                value["copy"] = json!(copy(translated));
            }
            ResultData::Definition {
                word,
                language,
                pronunciation,
                audio_url,
                senses,
            } => {
                value["kind"] = json!("definition");
                value["title"] = json!(word.presentation());
                value["language"] = json!(language);
                value["pronunciation"] = json!(pronunciation.as_ref().map(Text::presentation));
                value["audio"] = json!(audio_url);
                value["senses"]=json!(senses.iter().map(|s|json!({"part":s.part_of_speech.presentation(),"definition":s.definition.presentation(),
                    "example":s.example.as_ref().map(Text::presentation),"etymology":s.etymology.as_ref().map(Text::presentation),
                    "synonyms":s.synonyms.iter().map(Text::presentation).collect::<Vec<_>>(),"antonyms":s.antonyms.iter().map(Text::presentation).collect::<Vec<_>>(),"copy":copy(&s.definition)})).collect::<Vec<_>>());
            }
            ResultData::Weather(w) => {
                value["kind"] = json!("weather");
                value["title"] = json!(w.place.presentation());
                value["current"] = conditions(&w.current, &zone)?;
                value["historical"] = json!(now.saturating_sub(w.current.at) > 3600);
                value["days"]=json!(w.days.iter().map(|d|Ok(json!({"date":date(d.at,&zone,"%a, %b %-d")?,"key":date(d.at,&zone,"%F")?,
                    "low":temperature(d.low_mc),"high":temperature(d.high_mc),"icon":icon(d.wmo_code),"chance":format!("{}%",d.precipitation_chance),"description":d.description.presentation()}))).collect::<Result<Vec<_>,Error>>()?);
                value["hours"] = json!(
                    w.hours
                        .iter()
                        .map(|h| {
                            let mut v = conditions(h, &zone)?;
                            v["key"] = json!(date(h.at, &zone, "%F")?);
                            Ok(v)
                        })
                        .collect::<Result<Vec<_>, Error>>()?
                );
                if let Some(days) = value["days"].as_array_mut() {
                    for day in days {
                        let key = day["key"].as_str().ok_or(Error::Invalid)?.to_owned();
                        let hours = w
                            .hours
                            .iter()
                            .filter(|h| date(h.at, &zone, "%F").ok().as_deref() == Some(&key))
                            .collect::<Vec<_>>();
                        if hours.is_empty() {
                            continue;
                        }
                        let mut charts = Vec::new();
                        for imperial in [false, true] {
                            let chart = crate::data::Chart {
                                kind: crate::data::ChartKind::Line,
                                title: Text::plain(
                                    if imperial {
                                        "Hourly temperature · °F"
                                    } else {
                                        "Hourly temperature · °C"
                                    },
                                    Default::default(),
                                )?,
                                points: hours
                                    .iter()
                                    .map(|h| {
                                        Ok(crate::data::Point {
                                            label: Text::plain(
                                                &date(h.at, &zone, "%-I:%M %p %Z")?,
                                                Default::default(),
                                            )?,
                                            x: None,
                                            percent: false,
                                            y: crate::numeric::Number::new(if imperial {
                                                f64::from(h.temperature_mc) * 0.0018 + 32.0
                                            } else {
                                                f64::from(h.temperature_mc) / 1000.0
                                            })?,
                                        })
                                    })
                                    .collect::<Result<Vec<_>, Error>>()?,
                            };
                            charts.push(chart.presentation()?);
                        }
                        day["charts"] = json!(charts);
                    }
                }
                value["today"] = json!(date(w.current.at, &zone, "%F")?);
            }
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn weather_units_and_dates_are_resolved_locally_and_translation_copy_respects_reveal() {
        let t = |s| Text::plain(s, Default::default()).unwrap();
        let mut snapshot = Snapshot {
            provider: crate::service::Provider {
                id: "local".into(),
                version: None,
                attribution: t("Synthetic"),
                source_url: None,
            },
            resolved_at: 1_800_000_000,
            content: ResultData::Translation {
                original: t("Hello"),
                source_language: "en".into(),
                target_language: "es".into(),
                detected: true,
                translated: crate::parse("spoiler::Hola;", Default::default()).unwrap(),
            },
        };
        assert!(snapshot.presentation(1_800_000_000).unwrap()["copy"].is_null());
        snapshot.content = ResultData::Weather(crate::service::Weather {
            place: t("Synthetic place"),
            coordinates: crate::service::Coordinates {
                latitude_e6: 0,
                longitude_e6: 0,
            },
            timezone: "America/Chicago".into(),
            current: Conditions {
                at: 1_800_000_000,
                temperature_mc: 0,
                feels_like_mc: Some(-5000),
                description: t("Snow"),
                wmo_code: 73,
                precipitation_um: Some(1500),
                precipitation_chance: Some(80),
                humidity: Some(50),
                wind_mms: 1000,
                wind_degrees: 360,
                uv_tenths: Some(15),
            },
            days: vec![],
            hours: vec![],
        });
        if let ResultData::Weather(w) = &mut snapshot.content {
            w.days.push(crate::service::ForecastDay {
                at: w.current.at,
                low_mc: -5000,
                high_mc: 5000,
                wmo_code: 73,
                precipitation_chance: 80,
                description: t("Snow"),
            });
            w.hours.push(w.current.clone());
            let mut next = w.current.clone();
            next.at += 3600;
            next.temperature_mc = 1000;
            w.hours.push(next);
        }
        let view = snapshot.presentation(1_800_003_601).unwrap();
        assert_eq!(view["current"]["temperature"], json!(["0.0 °C", "32.0 °F"]));
        assert_eq!(view["current"]["wind"][0], "3.6 km/h N");
        assert_eq!(view["current"]["rain"], "1.5 mm");
        assert_eq!(view["historical"], true);
        assert_eq!(view["current"]["icon"], "weather_snowy");
        assert_eq!(view["days"][0]["charts"][1]["points"][1]["value"], "33.8");
        assert_eq!(view["days"][0]["charts"][0]["points"][1]["value"], "1");
        assert!(view["current"]["date"].as_str().unwrap().contains("CST"));
    }
}
