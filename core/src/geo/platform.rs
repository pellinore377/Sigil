//! The OS location service, one backend per target: implement `fix()` and the ladder in
//! `super`, every caller and the view, stay unchanged.

use super::{now_ms, Fix, Source};

/// What this build can do, shown when nothing can place the machine. Never empty.
pub fn describe() -> &'static str {
    #[cfg(target_os = "linux")]
    { "Linux: GeoClue if present, otherwise a WiFi lookup" }
    #[cfg(target_os = "macos")]
    { "macOS: CoreLocation" }
    #[cfg(target_os = "windows")]
    { "Windows: Windows.Devices.Geolocation" }
    #[cfg(target_os = "android")]
    { "Android: FusedLocationProvider" }
    #[cfg(target_os = "ios")]
    { "iOS: CoreLocation" }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows",
                  target_os = "android", target_os = "ios")))]
    { "this platform has no location backend yet" }
}

/// The platform's answer, or `None` — not an error; the ladder moves to the next source.
pub async fn fix() -> Option<Fix> {
    #[cfg(target_os = "linux")]
    { linux::fix().await }

    // Stubs. Each is a real API returning a position directly: no WiFi scan, no key.
    #[cfg(target_os = "macos")]
    {
        // CLLocationManager: authorize, startUpdatingLocation, read CLLocation.
        None
    }
    #[cfg(target_os = "windows")]
    {
        // Geolocator::RequestAccessAsync, then GetGeopositionAsync.
        None
    }
    #[cfg(target_os = "android")]
    {
        // FusedLocationProviderClient.getCurrentLocation via JNI (ACCESS_FINE_LOCATION).
        None
    }
    #[cfg(target_os = "ios")]
    {
        // CLLocationManager, as macOS.
        None
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows",
                  target_os = "android", target_os = "ios")))]
    { None }
}

/// Web builds have no engine process: the view calls `navigator.geolocation`.
pub const WEB_NOTE: &str = "web builds use navigator.geolocation from the view";

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::time::Duration;
    use tracing::debug;
    use zbus::zvariant::OwnedObjectPath;

    /// GeoClue is optional: it refuses unrecognised apps, and with no authorisation
    /// agent running it never answers — hence the timeout, then the next source.
    pub async fn fix() -> Option<Fix> {
        match tokio::time::timeout(Duration::from_secs(6), talk_to_geoclue()).await {
            Ok(v) => v,
            Err(_) => {
                debug!("geo: GeoClue did not answer in time (no authorisation agent?)");
                None
            }
        }
    }

    async fn talk_to_geoclue() -> Option<Fix> {
        let conn = zbus::Connection::system().await.ok()?;
        let manager = zbus::Proxy::new(
            &conn, "org.freedesktop.GeoClue2", "/org/freedesktop/GeoClue2/Manager",
            "org.freedesktop.GeoClue2.Manager",
        ).await.ok()?;
        let client_path: OwnedObjectPath = manager.call("GetClient", &()).await.ok()?;

        let client = zbus::Proxy::new(
            &conn, "org.freedesktop.GeoClue2", client_path.as_str(),
            "org.freedesktop.GeoClue2.Client",
        ).await.ok()?;
        // GeoClue authorises on DesktopId and rejects everything until it is set.
        client.set_property("DesktopId", "sigil").await.ok()?;
        client.set_property("RequestedAccuracyLevel", 8u32).await.ok()?;
        client.call::<_, _, ()>("Start", &()).await.ok()?;

        let path: OwnedObjectPath = client.get_property("Location").await.ok()?;
        if path.as_str() == "/" { return None }
        let loc = zbus::Proxy::new(
            &conn, "org.freedesktop.GeoClue2", path.as_str(),
            "org.freedesktop.GeoClue2.Location",
        ).await.ok()?;
        let lat: f64 = loc.get_property("Latitude").await.ok()?;
        let lon: f64 = loc.get_property("Longitude").await.ok()?;
        let accuracy: f64 = loc.get_property("Accuracy").await.unwrap_or(0.0);
        if !super::super::valid_coords(lat, lon) { return None }
        Some(Fix { lat, lon, accuracy, at_ms: now_ms(), source: Source::Platform })
    }
}
