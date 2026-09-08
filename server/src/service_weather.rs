use super::*;
use sigil_protocol::text::service::{Conditions, Coordinates, ForecastDay, Weather};
fn scaled(value: &Value, scale: f64) -> Result<i32, Error> {
    let n = (value.as_f64().ok_or(Error::Invalid)? * scale).round();
    if !n.is_finite() || n < i32::MIN as f64 || n > i32::MAX as f64 {
        return Err(Error::Invalid);
    }
    Ok(n as i32)
}
pub(super) fn coordinates(lat: &Value, lon: &Value) -> Result<Coordinates, Error> {
    let point = Coordinates {
        latitude_e6: scaled(lat, 1_000_000.0)?,
        longitude_e6: scaled(lon, 1_000_000.0)?,
    };
    point.validate().map_err(|_| Error::Invalid)?;
    Ok(point)
}
fn unsigned(value: &Value) -> Result<u64, Error> {
    value.as_u64().ok_or(Error::Invalid)
}
fn code(value: &Value) -> Result<u8, Error> {
    u8::try_from(unsigned(value)?).map_err(|_| Error::Invalid)
}
fn description(code: u8) -> Result<Text, Error> {
    text(match code {
        0 => "Clear",
        1 => "Mainly clear",
        2 => "Partly cloudy",
        3 => "Overcast",
        45 | 48 => "Fog",
        51 | 53 | 55 => "Drizzle",
        56 | 57 => "Freezing drizzle",
        61 | 63 | 65 => "Rain",
        66 | 67 => "Freezing rain",
        71 | 73 | 75 => "Snow",
        77 => "Snow grains",
        80..=82 => "Rain showers",
        85 | 86 => "Snow showers",
        95 => "Thunderstorm",
        96 | 99 => "Thunderstorm with hail",
        _ => return Err(Error::Invalid),
    })
}
fn conditions(value: &Value) -> Result<Conditions, Error> {
    let weather = code(&value["weather_code"])?;
    Ok(Conditions {
        at: unsigned(&value["time"])?,
        temperature_mc: scaled(&value["temperature_2m"], 1000.0)?,
        feels_like_mc: value
            .get("apparent_temperature")
            .filter(|v| !v.is_null())
            .map(|v| scaled(v, 1000.0))
            .transpose()?,
        description: description(weather)?,
        wmo_code: weather,
        precipitation_um: value
            .get("precipitation")
            .filter(|v| !v.is_null())
            .map(|v| u32::try_from(scaled(v, 1000.0)?).map_err(|_| Error::Invalid))
            .transpose()?,
        precipitation_chance: value
            .get("precipitation_probability")
            .filter(|v| !v.is_null())
            .map(code)
            .transpose()?,
        humidity: value
            .get("relative_humidity_2m")
            .filter(|v| !v.is_null())
            .map(code)
            .transpose()?,
        wind_mms: u32::try_from(scaled(&value["wind_speed_10m"], 1000.0)?)
            .map_err(|_| Error::Invalid)?,
        wind_degrees: u16::try_from(scaled(&value["wind_direction_10m"], 1.0)?)
            .map_err(|_| Error::Invalid)?,
        uv_tenths: value
            .get("uv_index")
            .filter(|v| !v.is_null())
            .map(|v| u16::try_from(scaled(v, 10.0)?).map_err(|_| Error::Invalid))
            .transpose()?,
    })
}
fn rows(value: &Value, max: usize) -> Result<Vec<Value>, Error> {
    let count = value["time"]
        .as_array()
        .filter(|v| v.len() <= max)
        .ok_or(Error::Invalid)?
        .len();
    let mut rows = vec![json!({}); count];
    for (name, values) in value.as_object().ok_or(Error::Invalid)? {
        let values = values
            .as_array()
            .filter(|v| v.len() == count)
            .ok_or(Error::Invalid)?;
        for (row, value) in rows.iter_mut().zip(values) {
            row[name] = value.clone();
        }
    }
    Ok(rows)
}
pub(super) fn resolve(
    entry: &StoredEntry,
    place: &Place,
    forecast: bool,
) -> Result<Weather, Error> {
    let variables="temperature_2m,apparent_temperature,relative_humidity_2m,precipitation,weather_code,wind_speed_10m,wind_direction_10m";
    let url=format!("{}?latitude={:.6}&longitude={:.6}&timezone={}&timeformat=unixtime&temperature_unit=celsius&wind_speed_unit=ms&precipitation_unit=mm&forecast_days={}&current={variables}&hourly={variables},precipitation_probability,uv_index&daily=temperature_2m_max,temperature_2m_min,weather_code,precipitation_probability_max",entry.provider.endpoint,f64::from(place.coordinates.latitude_e6)/1_000_000.0,f64::from(place.coordinates.longitude_e6)/1_000_000.0,encode(&place.timezone),if forecast{7}else{1});
    let value = fetch(entry, &url, None)?;
    if string(&value["timezone"])? != place.timezone {
        return Err(Error::Invalid);
    }
    for (key, unit) in [
        ("temperature_2m", "°C"),
        ("wind_speed_10m", "m/s"),
        ("precipitation", "mm"),
        ("time", "unixtime"),
    ] {
        if string(&value["current_units"][key])? != unit {
            return Err(Error::Invalid);
        }
    }
    let current = conditions(&value["current"])?;
    for (key, unit) in [
        ("temperature_2m", "°C"),
        ("wind_speed_10m", "m/s"),
        ("precipitation", "mm"),
        ("time", "unixtime"),
    ] {
        if string(&value["hourly_units"][key])? != unit {
            return Err(Error::Invalid);
        }
    }
    for (key, unit) in [
        ("temperature_2m_min", "°C"),
        ("temperature_2m_max", "°C"),
        ("time", "unixtime"),
    ] {
        if string(&value["daily_units"][key])? != unit {
            return Err(Error::Invalid);
        }
    }
    let hours = rows(&value["hourly"], 168)?
        .iter()
        .map(conditions)
        .collect::<Result<_, _>>()?;
    let days = rows(&value["daily"], 7)?
        .iter()
        .map(|day| {
            let wmo = code(&day["weather_code"])?;
            Ok(ForecastDay {
                at: unsigned(&day["time"])?,
                low_mc: scaled(&day["temperature_2m_min"], 1000.0)?,
                high_mc: scaled(&day["temperature_2m_max"], 1000.0)?,
                wmo_code: wmo,
                precipitation_chance: code(&day["precipitation_probability_max"])?,
                description: description(wmo)?,
            })
        })
        .collect::<Result<_, Error>>()?;
    Ok(Weather {
        place: text(&place.name)?,
        coordinates: place.coordinates,
        timezone: place.timezone.clone(),
        current,
        days,
        hours,
    })
}
