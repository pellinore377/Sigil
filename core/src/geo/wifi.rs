//! Desktop Linux fallback where GeoClue does not work: scan the access points and ask an
//! Ichnaea-compatible service where they are. No key ships here — the default is keyless
//! BeaconDB; `geolocationUrl` takes another provider, whose key belongs to the deployer.

use std::time::Duration;

use serde_json::{json, Value};
use tracing::{info, warn};

use super::{now_ms, settings, Fix, Source, COARSE_ACCURACY_M};

/// Keyless, so it can honestly be a default; coverage is patchy.
const DEFAULT_PROVIDER: &str = "https://api.beacondb.net/v1/geolocate";

/// Ichnaea's own advice: a lookup from one access point is a coincidence.
const MIN_ACCESS_POINTS: usize = 2;

fn choose_provider(configured: &str) -> String {
    let url = configured.trim();
    if url.starts_with("https://") || url.starts_with("http://") { url.to_string() }
    else { DEFAULT_PROVIDER.into() }
}

fn provider() -> String {
    choose_provider(settings().get("geolocationUrl").and_then(Value::as_str).unwrap_or(""))
}

#[derive(Debug, Clone)]
pub struct AccessPoint {
    pub bssid: String,
    pub dbm: i32,
    pub frequency: u32,
}

/// NetworkManager reports link quality 0..100; Ichnaea wants dBm.
pub fn signal_dbm(strength: u8) -> i32 { (strength as i32) / 2 - 100 }

/// An IP guess is signalled only by a loose accuracy radius; label it so the view can say so.
pub fn classify(ap_count: usize, accuracy: f64) -> Source {
    if ap_count >= MIN_ACCESS_POINTS && accuracy > 0.0 && accuracy <= COARSE_ACCURACY_M {
        Source::Wifi
    } else {
        Source::Ip
    }
}

pub async fn fix() -> Option<Fix> {
    let aps = access_points().await;
    info!("geo: wifi fallback has {} access points to work with", aps.len());
    let url = provider();
    match ask(&url, &aps).await {
        Some(f) => {
            if f.coarse() {
                info!("geo: {url} could only manage +/-{:.0} m — it does not know this area", f.accuracy);
            }
            Some(f)
        }
        None => { info!("geo: the geolocation service returned nothing usable"); None }
    }
}

async fn ask(url: &str, aps: &[AccessPoint]) -> Option<Fix> {
    let body = json!({
        "considerIp": true,
        "wifiAccessPoints": aps.iter().map(|a| json!({
            "macAddress": a.bssid,
            "signalStrength": a.dbm,
            "frequency": a.frequency,
        })).collect::<Vec<_>>(),
    });
    let client = crate::net::http_builder().timeout(Duration::from_secs(20)).build().ok()?;
    let resp = client.post(url).json(&body).send().await.ok()?;
    if !resp.status().is_success() {
        warn!("geo: geolocation service answered {}", resp.status());
        return None
    }
    let v: Value = resp.json().await.ok()?;
    let lat = v.pointer("/location/lat").and_then(Value::as_f64)?;
    let lon = v.pointer("/location/lng").and_then(Value::as_f64)?;
    let accuracy = v.get("accuracy").and_then(Value::as_f64).unwrap_or(0.0);
    if !super::valid_coords(lat, lon) { return None }
    Some(Fix { lat, lon, accuracy, at_ms: now_ms(), source: classify(aps.len(), accuracy) })
}

/// APs the system already knows. NetworkManager only: under iwd there are none and the ladder moves on.
#[cfg(target_os = "linux")]
async fn access_points() -> Vec<AccessPoint> {
    use zbus::zvariant::OwnedObjectPath;
    let Ok(conn) = zbus::Connection::system().await else { return Vec::new() };

    async fn prop<T>(conn: &zbus::Connection, path: &str, iface: &str, name: &str) -> Option<T>
    where
        T: TryFrom<zbus::zvariant::OwnedValue>,
        <T as TryFrom<zbus::zvariant::OwnedValue>>::Error: Into<zbus::Error>,
    {
        let p = zbus::Proxy::new(conn, "org.freedesktop.NetworkManager", path, iface).await.ok()?;
        p.get_property::<T>(name).await.ok()
    }

    let Some(devices) = prop::<Vec<OwnedObjectPath>>(
        &conn, "/org/freedesktop/NetworkManager", "org.freedesktop.NetworkManager", "Devices").await
    else { return Vec::new() };

    let mut out = Vec::new();
    for dev in devices {
        // 2 is NM_DEVICE_TYPE_WIFI.
        let kind: u32 = prop(&conn, dev.as_str(), "org.freedesktop.NetworkManager.Device", "DeviceType")
            .await.unwrap_or(0);
        if kind != 2 { continue }
        let Some(aps) = prop::<Vec<OwnedObjectPath>>(
            &conn, dev.as_str(), "org.freedesktop.NetworkManager.Device.Wireless", "AccessPoints").await
        else { continue };
        for ap in aps {
            let iface = "org.freedesktop.NetworkManager.AccessPoint";
            let Some(bssid) = prop::<String>(&conn, ap.as_str(), iface, "HwAddress").await else { continue };
            let strength: u8 = prop(&conn, ap.as_str(), iface, "Strength").await.unwrap_or(0);
            let frequency: u32 = prop(&conn, ap.as_str(), iface, "Frequency").await.unwrap_or(0);
            out.push(AccessPoint { bssid, dbm: signal_dbm(strength), frequency });
        }
    }
    out
}

#[cfg(not(target_os = "linux"))]
async fn access_points() -> Vec<AccessPoint> {
    // Other targets all have a platform location service.
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_ships_with_an_api_key() {
        // A key in the source tree is someone else's quota and terms.
        assert!(!DEFAULT_PROVIDER.contains("key="), "{DEFAULT_PROVIDER}");
        assert!(DEFAULT_PROVIDER.starts_with("https://"));
    }

    #[test]
    fn a_configured_provider_replaces_the_default() {
        // Pure, so no settings.json on the running machine can affect it.
        assert_eq!(choose_provider(""), DEFAULT_PROVIDER);
        assert_eq!(choose_provider("   "), DEFAULT_PROVIDER);
        assert_eq!(choose_provider("not a url"), DEFAULT_PROVIDER);
        assert_eq!(choose_provider("https://api.example/v1/geolocate?key=abc"),
                   "https://api.example/v1/geolocate?key=abc");
    }

    #[test]
    fn a_25km_answer_is_labelled_ip_however_many_access_points_were_sent() {
        // The accuracy radius alone decides the label; the count says nothing.
        assert_eq!(classify(41, 25_000.0), Source::Ip);
        assert_eq!(classify(41, 34.0), Source::Wifi);
        assert_eq!(classify(1, 34.0), Source::Ip, "one access point is never a fix");
        assert_eq!(classify(10, 0.0), Source::Ip, "no accuracy means no confidence");
    }

    #[test]
    fn signal_quality_becomes_plausible_dbm() {
        assert_eq!(signal_dbm(100), -50);
        assert_eq!(signal_dbm(20), -90);
        assert_eq!(signal_dbm(0), -100);
    }
}
