use super::*;
use serde::Deserialize;
use sigil_protocol::services::{Catalog, Resolve, Resolved, MAX_BODY, MAX_RESPONSE};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapAvailability {
    pub revision: u64,
    pub enabled: bool,
}
pub struct MapTile {
    pub bytes: Zeroizing<Vec<u8>>,
    pub content_type: String,
    pub encoding: Option<String>,
}
impl HttpsClient {
    pub fn service_catalog(&self) -> Result<Catalog, Error> {
        let result: Catalog = self.json(
            self.request(Method::GET, "/client/v0/services", None::<&()>)?,
            200,
            MAX_BODY,
        )?;
        let mut ids = std::collections::BTreeSet::new();
        if result.revision > i64::MAX as u64 || result.providers.len() > 8 {
            return Err(Error::InvalidResponse);
        }
        for provider in &result.providers {
            provider.validate().map_err(|_| Error::InvalidResponse)?;
            if !ids.insert(&provider.id) {
                return Err(Error::InvalidResponse);
            }
        }
        Ok(result)
    }
    pub fn resolve_service(&self, request: &Resolve) -> Result<Resolved, Error> {
        request.query.validate().map_err(|_| Error::Configuration)?;
        if !request.query.compatible(request.provider.kind) {
            return Err(Error::Configuration);
        }
        let result: Resolved = self.json(
            self.request(Method::POST, "/client/v0/services/resolve", Some(request))?,
            200,
            MAX_RESPONSE,
        )?;
        result
            .validate_for(&request.provider, &request.query)
            .map_err(|_| Error::InvalidResponse)?;
        Ok(result)
    }
    pub fn map_availability(&self) -> Result<MapAvailability, Error> {
        self.json(
            self.request(Method::GET, "/client/v0/maps", None::<&()>)?,
            200,
            SMALL,
        )
    }
    pub fn map_style(&self) -> Result<Zeroizing<Vec<u8>>, Error> {
        self.response_bytes(
            self.request(Method::GET, "/client/v0/maps/style.json", None::<&()>)?,
            200,
            1024 * 1024,
            "application/json",
        )
    }
    pub fn map_metadata(&self) -> Result<serde_json::Value, Error> {
        self.json(
            self.request(Method::GET, "/client/v0/maps/tiles.json", None::<&()>)?,
            200,
            1024 * 1024,
        )
    }
    pub fn map_asset(&self, name: &str) -> Result<Zeroizing<Vec<u8>>, Error> {
        if name.is_empty()
            || name.len() > 512
            || name.contains(['\\', '\0'])
            || name
                .split('/')
                .any(|s| s.is_empty() || s == "." || s == "..")
        {
            return Err(Error::Configuration);
        }
        let kind = match name.rsplit_once('.').map(|(_, ext)| ext) {
            Some("json") => "application/json",
            Some("pbf") => "application/x-protobuf",
            Some("png") => "image/png",
            _ => return Err(Error::Configuration),
        };
        let mut escaped = String::new();
        for b in name.bytes() {
            if b.is_ascii_alphanumeric() || b"-_.~/".contains(&b) {
                escaped.push(char::from(b));
            } else {
                use std::fmt::Write;
                write!(escaped, "%{b:02X}").map_err(|_| Error::Configuration)?;
            }
        }
        self.response_bytes(
            self.request(
                Method::GET,
                &format!("/client/v0/maps/assets/{escaped}"),
                None::<&()>,
            )?,
            200,
            4 * 1024 * 1024,
            kind,
        )
    }
    pub fn map_tile(&self, z: u8, x: u32, y: u32) -> Result<Option<MapTile>, Error> {
        if z > 26 || x >= 1 << z || y >= 1 << z {
            return Err(Error::Configuration);
        }
        let mut response = self.request(
            Method::GET,
            &format!("/client/v0/maps/tiles/{z}/{x}/{y}"),
            None::<&()>,
        )?;
        if response.status().as_u16() == 204 {
            self.empty(response)?;
            return Ok(None);
        }
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .filter(|v| {
                matches!(
                    *v,
                    "application/x-protobuf"
                        | "application/vnd.mapbox-vector-tile"
                        | "application/vnd.maplibre-tile"
                        | "image/png"
                        | "image/jpeg"
                        | "image/webp"
                        | "image/avif"
                )
            })
            .ok_or(Error::InvalidResponse)?
            .to_owned();
        if response
            .headers()
            .get_all(header::CONTENT_ENCODING)
            .iter()
            .count()
            > 1
        {
            return Err(Error::InvalidResponse);
        }
        let encoding = response
            .headers_mut()
            .remove(header::CONTENT_ENCODING)
            .map(|v| {
                v.to_str()
                    .map(str::to_owned)
                    .map_err(|_| Error::InvalidResponse)
            })
            .transpose()?;
        if encoding
            .as_ref()
            .is_some_and(|v| !matches!(v.as_str(), "identity" | "gzip" | "br" | "zstd"))
        {
            return Err(Error::InvalidResponse);
        }
        let bytes = self.response_bytes(response, 200, 4 * 1024 * 1024, &content_type)?;
        Ok(Some(MapTile {
            bytes,
            content_type,
            encoding,
        }))
    }
}
