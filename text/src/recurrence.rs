use crate::Error;
use jiff::{
    civil::Date,
    tz::{TimeZone, TimeZoneDatabase},
    Span, Timestamp,
};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

pub const TZDB_VERSION: &str = "2026c";
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interval {
    Weekly,
    Monthly,
    Yearly,
}
impl Interval {
    pub fn name(self) -> &'static str {
        match self {
            Self::Weekly => "Weekly",
            Self::Monthly => "Monthly",
            Self::Yearly => "Yearly",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recurrence {
    pub interval: Interval,
    pub timezone: String,
    pub tzdb: String,
    pub anchor_at: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Period {
    pub start: u64,
    pub end: u64,
}
fn timestamp(value: u64) -> Result<Timestamp, Error> {
    if value == 0 {
        return Err(Error::Invalid);
    }
    Timestamp::from_second(i64::try_from(value).map_err(|_| Error::Invalid)?)
        .map_err(|_| Error::Invalid)
}
pub(crate) fn zone(name: &str) -> Result<TimeZone, Error> {
    if jiff_tzdb::VERSION != Some(TZDB_VERSION) {
        return Err(Error::Version);
    }
    if name.len() > 64 || name.is_empty() {
        return Err(Error::Invalid);
    }
    static DB: OnceLock<TimeZoneDatabase> = OnceLock::new();
    let zone = DB
        .get_or_init(TimeZoneDatabase::bundled)
        .get(name)
        .map_err(|_| Error::Invalid)?;
    if zone.is_unknown() || zone.iana_name() != Some(name) {
        return Err(Error::Invalid);
    }
    Ok(zone)
}
impl Recurrence {
    pub fn new(interval: Interval, timezone: &str, anchor_at: u64) -> Result<Self, Error> {
        let rule = Self {
            interval,
            timezone: timezone.into(),
            tzdb: TZDB_VERSION.into(),
            anchor_at,
        };
        rule.validate()?;
        Ok(rule)
    }
    pub fn validate(&self) -> Result<(), Error> {
        if self.tzdb != TZDB_VERSION {
            return Err(Error::Version);
        }
        zone(&self.timezone)?;
        timestamp(self.anchor_at)?;
        Ok(())
    }
    fn candidate(&self, anchor: Date, cycle: i64, zone: &TimeZone) -> Result<u64, Error> {
        let span = match self.interval {
            Interval::Weekly => Span::new().try_weeks(cycle),
            Interval::Monthly => Span::new().try_months(cycle),
            Interval::Yearly => Span::new().try_years(cycle),
        }
        .map_err(|_| Error::Invalid)?;
        let date = anchor.checked_add(span).map_err(|_| Error::Invalid)?;
        // Compatible: earlier fold, shift forward by the gap when 00:01 does not exist.
        let instant = zone
            .to_ambiguous_zoned(date.at(0, 1, 0, 0))
            .compatible()
            .map_err(|_| Error::Invalid)?
            .timestamp()
            .as_second();
        u64::try_from(instant).map_err(|_| Error::Invalid)
    }
    /// First reset strictly after `after`, always derived from the original anchor.
    pub fn next_reset(&self, after: u64) -> Result<u64, Error> {
        Ok(self.period_at(after.max(self.anchor_at))?.end)
    }
    /// Active interval [start,end), including the initial partial interval.
    /// Calculates at most three calendar candidates; missed cycles are not replayed.
    pub fn period_at(&self, at: u64) -> Result<Period, Error> {
        self.validate()?;
        if at < self.anchor_at {
            return Err(Error::Invalid);
        }
        let zone = zone(&self.timezone)?;
        let anchor = zone.to_datetime(timestamp(self.anchor_at)?).date();
        let current = zone.to_datetime(timestamp(at)?).date();
        let cycle = match self.interval {
            Interval::Weekly => current.duration_since(anchor).as_secs() / 604800,
            Interval::Monthly => {
                i64::from(current.year() - anchor.year()) * 12
                    + i64::from(current.month() - anchor.month())
            }
            Interval::Yearly => i64::from(current.year() - anchor.year()),
        }
        .max(1);
        let mut start = self.anchor_at;
        for cycle in cycle.saturating_sub(1).max(1)..=cycle.checked_add(1).ok_or(Error::Invalid)? {
            let candidate = self.candidate(anchor, cycle, &zone)?;
            if candidate > at {
                return Ok(Period {
                    start,
                    end: candidate,
                });
            }
            start = start.max(candidate);
        }
        Err(Error::Invalid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at(zone_name: &str, date: &str) -> u64 {
        zone(zone_name)
            .unwrap()
            .to_zoned(date.parse().unwrap())
            .unwrap()
            .timestamp()
            .as_second() as u64
    }
    #[test]
    fn month_end_and_leap_recurrences_clamp_and_snap_back() {
        for (year, february) in [(2023, 28), (2024, 29)] {
            let start = at("America/Chicago", &format!("{year}-01-31T12:00:00"));
            let rule = Recurrence::new(Interval::Monthly, "America/Chicago", start).unwrap();
            let first = rule.next_reset(start).unwrap();
            assert_eq!(
                first,
                at("America/Chicago", &format!("{year}-02-{february}T00:01:00"))
            );
            assert_eq!(
                rule.next_reset(first).unwrap(),
                at("America/Chicago", &format!("{year}-03-31T00:01:00"))
            );
        }
        let start = at("UTC", "2024-02-29T12:00:00");
        let rule = Recurrence::new(Interval::Yearly, "UTC", start).unwrap();
        let mut after = start;
        for year in 2025..=2028 {
            after = rule.next_reset(after).unwrap();
            let day = if year == 2028 { 29 } else { 28 };
            assert_eq!(after, at("UTC", &format!("{year}-02-{day}T00:01:00")));
        }
    }
    #[test]
    fn weekly_dates_use_bundled_rules_and_preserve_wall_time_through_dst() {
        let start = at("America/New_York", "2026-03-06T12:00:00");
        let rule = Recurrence::new(Interval::Weekly, "America/New_York", start).unwrap();
        let next = rule.next_reset(start).unwrap();
        assert_eq!(next, at("America/New_York", "2026-03-13T00:01:00"));
        assert_eq!(rule.next_reset(next - 1).unwrap(), next);
        assert_eq!(
            rule.next_reset(next).unwrap(),
            at("America/New_York", "2026-03-20T00:01:00")
        );
        // 2026c includes Alberta's change; no host timezone database participates.
        let alberta = zone("America/Edmonton").unwrap();
        let instant = timestamp(at("UTC", "2026-12-01T12:00:00")).unwrap();
        assert_eq!(alberta.to_datetime(instant).hour(), 6);
        assert!(Recurrence::new(Interval::Weekly, "america/new_york", start).is_err());
        assert!(Recurrence::new(Interval::Weekly, "/etc/localtime", start).is_err());
        let mut stale = rule;
        stale.tzdb = "2025b".into();
        assert_eq!(stale.next_reset(start).err(), Some(Error::Version));
    }
    #[test]
    fn midnight_gaps_folds_catchup_and_bounds_have_explicit_results() {
        let start = at("America/Sao_Paulo", "2018-10-28T12:00:00");
        let rule = Recurrence::new(Interval::Weekly, "America/Sao_Paulo", start).unwrap();
        assert_eq!(
            rule.next_reset(start).unwrap(),
            at("UTC", "2018-11-04T03:01:00")
        );
        let start = at("America/Havana", "2020-10-25T12:00:00");
        let rule = Recurrence::new(Interval::Weekly, "America/Havana", start).unwrap();
        assert_eq!(
            rule.next_reset(start).unwrap(),
            at("UTC", "2020-11-01T04:01:00")
        );
        let start = at("UTC", "2020-01-31T12:00:00");
        let rule = Recurrence::new(Interval::Monthly, "UTC", start).unwrap();
        assert_eq!(
            rule.next_reset(at("UTC", "2026-09-07T12:00:00")).unwrap(),
            at("UTC", "2026-09-30T00:01:00")
        );
        assert_eq!(rule.next_reset(0).unwrap(), rule.next_reset(start).unwrap());
        assert!(rule.next_reset(u64::MAX).is_err());
        assert!(Recurrence::new(Interval::Yearly, "UTC", u64::MAX).is_err());
        assert!(Recurrence::new(Interval::Yearly, "Etc/Unknown", start).is_err());
    }
    #[test]
    fn periods_change_at_exact_calendar_boundaries_and_catch_up_without_replay() {
        let start = at("America/New_York", "2026-03-06T12:00:00");
        let first = at("America/New_York", "2026-03-13T00:01:00");
        let second = at("America/New_York", "2026-03-20T00:01:00");
        let rule = Recurrence::new(Interval::Weekly, "America/New_York", start).unwrap();
        assert_eq!(rule.period_at(start).unwrap(), Period { start, end: first });
        assert_eq!(
            rule.period_at(first - 1).unwrap(),
            Period { start, end: first }
        );
        assert_eq!(
            rule.period_at(first).unwrap(),
            Period {
                start: first,
                end: second
            }
        );
        assert!(rule.period_at(start - 1).is_err());
        let start = at("UTC", "2020-01-31T12:00:00");
        let rule = Recurrence::new(Interval::Monthly, "UTC", start).unwrap();
        assert_eq!(
            rule.period_at(at("UTC", "2026-09-07T12:00:00")).unwrap(),
            Period {
                start: at("UTC", "2026-08-31T00:01:00"),
                end: at("UTC", "2026-09-30T00:01:00")
            }
        );
        let start = at("America/Sao_Paulo", "2018-10-28T12:00:00");
        let rule = Recurrence::new(Interval::Weekly, "America/Sao_Paulo", start).unwrap();
        let gap = at("UTC", "2018-11-04T03:01:00");
        assert_eq!(rule.period_at(gap - 1).unwrap().start, start);
        assert_eq!(rule.period_at(gap).unwrap().start, gap);
    }
}
