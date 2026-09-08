use crate::{service::Coordinates, structured::Id, Error, Text};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Duration {
    FifteenMinutes,
    Hour,
    EightHours,
}
impl Duration {
    pub fn seconds(self) -> u64 {
        match self {
            Self::FifteenMinutes => 900,
            Self::Hour => 3600,
            Self::EightHours => 28800,
        }
    }
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Mode {
    Once,
    Pin,
    Live {
        duration: Duration,
        #[serde(with = "crate::structured::id")]
        device: Id,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub coordinates: Coordinates,
    pub accuracy_cm: Option<u32>,
    pub sampled_at: u64,
}
impl Point {
    pub fn validate(&self) -> Result<(), Error> {
        self.coordinates.validate()?;
        if self.sampled_at == 0
            || self.sampled_at > i64::MAX as u64
            || self.accuracy_cm.is_some_and(|v| v > 100_000_000)
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Share {
    pub mode: Mode,
    pub point: Point,
    pub label: Text,
}
impl Share {
    pub fn until(&self, created_at: u64) -> Result<Option<u64>, Error> {
        match self.mode {
            Mode::Live { duration, device } => {
                if device == [0; 32] {
                    return Err(Error::Invalid);
                }
                Ok(Some(
                    created_at
                        .checked_add(duration.seconds())
                        .filter(|v| *v <= i64::MAX as u64)
                        .ok_or(Error::Invalid)?,
                ))
            }
            _ => Ok(None),
        }
    }
    pub fn validate(&self, created_at: u64) -> Result<(), Error> {
        self.point.validate()?;
        if self.label.body().len() > 256
            || self.point.sampled_at < created_at.saturating_sub(300)
            || self.point.sampled_at > self.until(created_at)?.unwrap_or(created_at)
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    pub fn validate_update(
        &self,
        original: &Self,
        parent: &Self,
        created_at: u64,
        at: u64,
    ) -> Result<(), Error> {
        let until = original.until(created_at)?.ok_or(Error::Invalid)?;
        if self.mode != original.mode
            || self.label != original.label
            || at < created_at
            || at >= until
            || self.point.sampled_at > at
            || self.point.sampled_at <= parent.point.sampled_at
        {
            return Err(Error::Invalid);
        }
        self.validate(created_at)
    }
    pub fn body(&self) -> String {
        let kind = match self.mode {
            Mode::Once => "Location",
            Mode::Pin => "Dropped pin",
            Mode::Live { .. } => "Live location",
        };
        format!(
            "{kind}: {} ({:.6}, {:.6})",
            self.label.body(),
            f64::from(self.point.coordinates.latitude_e6) / 1_000_000.0,
            f64::from(self.point.coordinates.longitude_e6) / 1_000_000.0
        )
    }
    pub fn html(&self) -> Result<String, Error> {
        Ok(Text::plain(&self.body(), Default::default())?.html())
    }
}
