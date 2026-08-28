//! Position, by a ladder: the OS location service, then (desktop Linux only, where GeoClue
//! is absent or refuses) our own WiFi lookup, then the coarse IP guess. No API key lives in
//! this source. There is deliberately no "type in your coordinates" fallback.

pub mod platform;
pub mod wifi;

use std::time::Duration;

use serde_json::{json, Value};
use tracing::debug;

use crate::engine::SharedEngine;

/// Re-ask this often while the app runs.
const REFRESH_EVERY: Duration = Duration::from_secs(15 * 60);

/// Metres. Looser than this is a guess at a city, not a position.
pub const COARSE_ACCURACY_M: f64 = 5_000.0;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    #[default]
    None,
    /// The operating system's own location service.
    Platform,
    /// Our own WiFi lookup — desktop Linux fallback only.
    Wifi,
    /// The coarse IP guess returned when nothing nearby is recognised.
    Ip,
}

impl Source {
    pub fn as_str(&self) -> &'static str {
        match self {
            Source::None => "none",
            Source::Platform => "platform",
            Source::Wifi => "wifi",
            Source::Ip => "ip",
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Fix {
    pub lat: f64,
    pub lon: f64,
    pub accuracy: f64,
    pub at_ms: u64,
    pub source: Source,
}

impl Fix {
    /// A guess at a city rather than a position.
    pub fn coarse(&self) -> bool { self.accuracy > COARSE_ACCURACY_M }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn valid_coords(lat: f64, lon: f64) -> bool {
    (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)
}


pub fn settings() -> Value {
    std::fs::read(crate::notify::settings_path())
        .ok()
        .and_then(|d| serde_json::from_slice(&d).ok())
        .unwrap_or_else(|| json!({}))
}


async fn resolve(engine: &SharedEngine) {
    // Prefer the better answer, not the higher rung: a coarse platform IP guess must not beat a WiFi fix.
    let platform_fix = platform::fix().await;
    if let Some(f) = platform_fix {
        if !f.coarse() {
            debug!("geo: platform fix {:.5},{:.5} +/-{:.0}m", f.lat, f.lon, f.accuracy);
            publish(engine, Some(f), "");
            return
        }
        debug!("geo: the platform only managed +/-{:.0} m; looking further", f.accuracy);
    }

    let best = better(wifi::fix().await, platform_fix);
    match best {
        Some(f) => {
            debug!("geo: {} fix {:.5},{:.5} +/-{:.0}m", f.source.as_str(), f.lat, f.lon, f.accuracy);
            publish(engine, Some(f), "");
        }
        None => {
            debug!("geo: nothing could place this machine");
            publish(engine, None, &format!(
                "this system has no location service Sigil can use ({})",
                platform::describe()
            ));
        }
    }
}

/// The tighter of two answers; zero accuracy is not an answer.
fn better(a: Option<Fix>, b: Option<Fix>) -> Option<Fix> {
    match (a, b) {
        (Some(x), Some(y)) => Some(if x.accuracy > 0.0 && x.accuracy <= y.accuracy { x } else { y }),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

fn publish(engine: &SharedEngine, fix: Option<Fix>, error: &str) {
    {
        let mut s = engine.state.lock();
        s.position = fix;
        s.position_error = error.to_string();
    }
    engine.hub.broadcast(position_json(engine));
}

pub fn start(engine: SharedEngine) {
    tokio::spawn(async move {
        loop {
            resolve(&engine).await;
            tokio::time::sleep(REFRESH_EVERY).await;
        }
    });
}

/// `position.refresh` — ask again now.
pub fn refresh(engine: &SharedEngine) {
    let engine = engine.clone();
    tokio::spawn(async move { resolve(&engine).await });
}

pub fn position_json(engine: &crate::engine::Engine) -> Value {
    let (fix, err) = {
        let s = engine.state.lock();
        (s.position, s.position_error.clone())
    };
    match fix {
        Some(f) => json!({
            "event": "position", "known": true,
            "lat": f.lat, "lon": f.lon, "accuracy": f.accuracy, "ts": f.at_ms,
            "source": f.source.as_str(), "coarse": f.coarse(),
            "platform": platform::describe(), "error": "",
        }),
        None => json!({
            "event": "position", "known": false, "source": "none", "coarse": false,
            "platform": platform::describe(), "error": err,
        }),
    }
}

/// Oldest fix a live-location beacon may still use.
pub const MAX_FIX_AGE: Duration = Duration::from_secs(16 * 60);

pub fn fresh_fix(engine: &SharedEngine) -> Option<Fix> {
    let fix = engine.state.lock().position?;
    if now_ms().saturating_sub(fix.at_ms) > MAX_FIX_AGE.as_millis() as u64 { return None }
    Some(fix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_city_sized_answer_is_coarse_and_a_street_sized_one_is_not() {
        let ip = Fix { lat: 51.5, lon: -0.12, accuracy: 25_000.0, at_ms: 0, source: Source::Ip };
        assert!(ip.coarse(), "25 km is a guess at a city");
        assert!(!Fix { accuracy: 34.0, source: Source::Wifi, ..ip }.coarse());
        assert!(!Fix { accuracy: 15.0, source: Source::Platform, ..ip }.coarse());
    }

    #[test]
    fn nonsense_coordinates_are_refused() {
        for (lat, lon) in [(91.0, 0.0), (-91.0, 0.0), (0.0, 181.0), (0.0, -181.0)] {
            assert!(!valid_coords(lat, lon), "{lat},{lon} should be rejected");
        }
        assert!(valid_coords(48.8583, 2.2944));
    }

    #[test]
    fn every_source_has_a_name_the_view_can_switch_on() {
        for s in [Source::None, Source::Platform, Source::Wifi, Source::Ip] {
            assert!(!s.as_str().is_empty());
        }
        assert_eq!(Source::Platform.as_str(), "platform");
    }

    #[test]
    fn the_tighter_answer_wins_whichever_rung_it_came_from() {
        let coarse = Fix { lat: 51.5, lon: -0.12, accuracy: 25_000.0, at_ms: 0, source: Source::Platform };
        let tight = Fix { lat: 48.8, lon: 2.2, accuracy: 23.0, at_ms: 0, source: Source::Wifi };
        // Regression: a coarse IP guess must not beat a tight WiFi fix.
        assert_eq!(better(Some(tight), Some(coarse)).unwrap().source, Source::Wifi);
        assert_eq!(better(Some(coarse), Some(tight)).unwrap().source, Source::Wifi);
        assert_eq!(better(Some(coarse), None).unwrap().source, Source::Platform);
        assert!(better(None, None).is_none());
    }

    #[test]
    fn the_build_can_say_what_it_is_capable_of() {
        // Shown in the error when nothing can place the machine; never empty.
        assert!(!platform::describe().is_empty());
    }
}
