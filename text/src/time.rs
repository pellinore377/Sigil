use crate::{
    recurrence::{zone, TZDB_VERSION},
    Error, Text,
};
use jiff::{
    civil::{Date, Time},
    Span, Timestamp,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DateOrder {
    MonthFirst,
    DayFirst,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dated {
    #[serde(with = "crate::structured::inline")]
    pub text: Text,
    pub at: u64,
    pub timezone: String,
    pub tzdb: String,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Timer {
    pub started_at: u64,
    pub ends_at: u64,
}

pub(crate) fn timestamp(at: u64) -> Result<Timestamp, Error> {
    if at == 0 {
        return Err(Error::Invalid);
    }
    Timestamp::from_second(i64::try_from(at).map_err(|_| Error::Invalid)?)
        .map_err(|_| Error::Invalid)
}
impl Dated {
    pub fn new(text: Text, at: u64, timezone: &str) -> Result<Self, Error> {
        let value = Self {
            text,
            at,
            timezone: timezone.into(),
            tzdb: TZDB_VERSION.into(),
        };
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<(), Error> {
        if self.tzdb != TZDB_VERSION {
            return Err(Error::Version);
        }
        timestamp(self.at)?;
        zone(&self.timezone)?;
        if self.text.body().trim().is_empty() {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
impl Timer {
    pub fn new(started_at: u64, duration: u64) -> Result<Self, Error> {
        let value = Self {
            started_at,
            ends_at: started_at.checked_add(duration).ok_or(Error::Limit)?,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<(), Error> {
        timestamp(self.started_at)?;
        timestamp(self.ends_at)?;
        if self.started_at >= self.ends_at {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    pub fn remaining(&self, now: u64) -> u64 {
        self.ends_at
            .saturating_sub(now)
            .min(self.ends_at.saturating_sub(self.started_at))
    }
    pub fn progress_per_mille(&self, now: u64) -> Result<u16, Error> {
        self.validate()?;
        Ok((u128::from(now.saturating_sub(self.started_at)) * 1000
            / u128::from(self.ends_at - self.started_at))
        .min(1000) as u16)
    }
}

pub fn duration(source: &str) -> Result<u64, Error> {
    if source.len() > 256 {
        return Err(Error::Limit);
    }
    let mut tail = source.trim();
    let mut total = 0u128;
    while !tail.is_empty() {
        let n = tail
            .bytes()
            .take_while(|b| b.is_ascii_digit() || *b == b'.')
            .count();
        if n == 0 {
            return Err(Error::Invalid);
        }
        let value = &tail[..n];
        let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
        if whole.is_empty()
            || fraction.len() > 6
            || (value.contains('.') && fraction.is_empty())
            || !fraction.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(Error::Invalid);
        }
        let scale = 10u128.pow(fraction.len() as u32);
        let numerator = whole
            .parse::<u128>()
            .map_err(|_| Error::Limit)?
            .checked_mul(scale)
            .and_then(|v| v.checked_add(fraction.parse::<u128>().unwrap_or(0)))
            .ok_or(Error::Limit)?;
        tail = tail[n..].trim_start();
        let n = tail.bytes().take_while(u8::is_ascii_alphabetic).count();
        let factor = match &tail[..n] {
            "s" | "sec" | "second" | "seconds" => 1,
            "m" | "min" | "minute" | "minutes" => 60,
            "h" | "hr" | "hour" | "hours" => 3600,
            "d" | "day" | "days" => 86400,
            _ => return Err(Error::Invalid),
        };
        let scaled = numerator.checked_mul(factor).ok_or(Error::Limit)?;
        if scaled % scale != 0 {
            return Err(Error::Invalid);
        }
        total = total.checked_add(scaled / scale).ok_or(Error::Limit)?;
        tail = tail[n..].trim_start();
    }
    if total == 0 {
        return Err(Error::Invalid);
    }
    u64::try_from(total).map_err(|_| Error::Limit)
}

/// Resolve once using sender context. Numeric dates never guess an absent locale.
pub fn resolve_date(
    source: &str,
    now: u64,
    timezone: &str,
    order: Option<DateOrder>,
) -> Result<u64, Error> {
    if source.len() > 128 {
        return Err(Error::Limit);
    }
    let zone = zone(timezone)?;
    let today = zone.to_datetime(timestamp(now)?).date();
    let source = source.trim().to_ascii_lowercase();
    let mut date_text = source.as_str();
    let mut time = Time::new(9, 0, 0, 0).map_err(|_| Error::Invalid)?;
    if let Some((date, clock)) = source.rsplit_once(' ') {
        if clock.bytes().next().is_some_and(|b| b.is_ascii_digit()) {
            time = parse_clock(clock)?;
            date_text = date;
        }
    } else if let Some((date, clock)) = source.split_once('t') {
        time = parse_clock(clock)?;
        date_text = date;
    }
    let date = match date_text {
        "today" => today,
        "tomorrow" => today
            .checked_add(Span::new().days(1))
            .map_err(|_| Error::Invalid)?,
        "next week" => today
            .checked_add(Span::new().days(7))
            .map_err(|_| Error::Invalid)?,
        text => {
            let weekdays = [
                "monday",
                "tuesday",
                "wednesday",
                "thursday",
                "friday",
                "saturday",
                "sunday",
            ];
            let day = text.strip_prefix("next ").unwrap_or(text);
            if let Some(index) = weekdays.iter().position(|v| *v == day) {
                let offset = (index as i64 - i64::from(today.weekday().to_monday_zero_offset()))
                    .rem_euclid(7);
                today
                    .checked_add(Span::new().days(if offset == 0 { 7 } else { offset }))
                    .map_err(|_| Error::Invalid)?
            } else if text.contains('/') {
                let parts = text
                    .split('/')
                    .map(str::parse::<i16>)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| Error::Invalid)?;
                if parts.len() != 3 {
                    return Err(Error::Invalid);
                }
                let (month, day) = match order.ok_or(Error::Invalid)? {
                    DateOrder::MonthFirst => (parts[0], parts[1]),
                    DateOrder::DayFirst => (parts[1], parts[0]),
                };
                let year = if (0..100).contains(&parts[2]) {
                    2000 + parts[2]
                } else {
                    parts[2]
                };
                Date::new(
                    year,
                    month.try_into().map_err(|_| Error::Invalid)?,
                    day.try_into().map_err(|_| Error::Invalid)?,
                )
                .map_err(|_| Error::Invalid)?
            } else {
                if text.len() != 10 || text.as_bytes()[4] != b'-' || text.as_bytes()[7] != b'-' {
                    return Err(Error::Invalid);
                }
                text.parse::<Date>().map_err(|_| Error::Invalid)?
            }
        }
    };
    let value = zone
        .to_ambiguous_zoned(date.to_datetime(time))
        .compatible()
        .map_err(|_| Error::Invalid)?
        .timestamp()
        .as_second();
    let value = u64::try_from(value).map_err(|_| Error::Invalid)?;
    timestamp(value)?;
    Ok(value)
}
fn parse_clock(source: &str) -> Result<Time, Error> {
    let am = source.ends_with("am");
    let pm = source.ends_with("pm");
    let raw = if am || pm {
        &source[..source.len() - 2]
    } else {
        source
    };
    let parts = raw.split(':').collect::<Vec<_>>();
    if parts.is_empty() || parts.len() > 3 {
        return Err(Error::Invalid);
    }
    let mut hour = parts[0].parse::<i8>().map_err(|_| Error::Invalid)?;
    let minute = parts
        .get(1)
        .map_or(Ok(0), |v| v.parse::<i8>())
        .map_err(|_| Error::Invalid)?;
    let second = parts
        .get(2)
        .map_or(Ok(0), |v| v.parse::<i8>())
        .map_err(|_| Error::Invalid)?;
    if am || pm {
        if !(1..=12).contains(&hour) {
            return Err(Error::Invalid);
        }
        hour = hour % 12 + if pm { 12 } else { 0 };
    }
    Time::new(hour, minute, second, 0).map_err(|_| Error::Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn epoch(value: &str) -> u64 {
        value.parse::<Timestamp>().unwrap().as_second() as u64
    }
    #[test]
    fn sender_dates_resolve_once_across_locale_dst_and_midnight() {
        let now = epoch("2026-03-07T23:59:59-05:00");
        assert_eq!(
            resolve_date("tomorrow 2:30am", now, "America/New_York", None).unwrap(),
            epoch("2026-03-08T03:30:00-04:00")
        );
        assert_eq!(
            resolve_date("2026-11-01 1:30am", now, "America/New_York", None).unwrap(),
            epoch("2026-11-01T01:30:00-04:00")
        );
        assert_eq!(
            resolve_date("07/05/27 9:30am", now, "UTC", Some(DateOrder::MonthFirst)).unwrap(),
            epoch("2027-07-05T09:30:00Z")
        );
        assert_eq!(
            resolve_date("07/05/27 9:30am", now, "UTC", Some(DateOrder::DayFirst)).unwrap(),
            epoch("2027-05-07T09:30:00Z")
        );
        assert!(resolve_date("07/05/27", now, "UTC", None).is_err());
        assert!(resolve_date("2026-02-30", now, "UTC", None).is_err());
        assert!(resolve_date("2026-03-08 24:00", now, "UTC", None).is_err());
    }
    #[test]
    fn timers_reject_invalid_arithmetic_and_use_absolute_endpoints() {
        for (input, seconds) in [
            ("1h45m", 6300),
            ("1 hour 45 min", 6300),
            ("1.5 hours", 5400),
            ("45s", 45),
        ] {
            assert_eq!(duration(input).unwrap(), seconds);
        }
        for invalid in [
            "0s",
            "-1h",
            "NaN h",
            "1.1.1h",
            "0.1s",
            "1.",
            "1hour code()",
            "999999999999999999999999999999999999999999999s",
        ] {
            assert!(duration(invalid).is_err(), "{invalid}");
        }
        let timer = Timer::new(100, 50).unwrap();
        assert_eq!(timer.remaining(90), 50);
        assert_eq!(timer.progress_per_mille(125).unwrap(), 500);
        assert_eq!(timer.progress_per_mille(500).unwrap(), 1000);
        assert!(Timer::new(u64::MAX, 1).is_err());
    }
}
