//! `location.map` and `map.tile`: what the frontends actually ask for. A
//! composite is a crop of rendered tiles centred on a point — nothing is
//! drawn on it, the UI places its own marker. Tiles come from the resolved
//! style's vector source through the rasteriser next door.

use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::{json, Value};

use super::render::{self, VectorSource, TILE_PX};
use crate::engine::SharedEngine;
use crate::ipc::wire::Reply;

/// The resolved source for the current style URL; dropped when it changes.
static SOURCE: Mutex<Option<(String, Arc<VectorSource>)>> = Mutex::new(None);

async fn fetch_json(url: &str) -> Option<Value> {
    let client = crate::net::http_builder()
        .user_agent(format!("Sigil/{} (Matrix client; maps)", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .ok()?;
    let resp = client.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json().await.ok()
}

/// Style URL → style document → its first vector source, tilejson and all.
async fn source_for(engine: &SharedEngine) -> Result<Arc<VectorSource>, Reply> {
    let style_url = engine.state.lock().map_style_url.clone();
    if style_url.is_empty() {
        return Err(Reply::err(
            "unavailable",
            "no map style configured: set one with map.setStyle, or the homeserver can publish m.tile_server",
        ));
    }
    if let Some((url, src)) = &*SOURCE.lock() {
        if *url == style_url {
            return Ok(src.clone());
        }
    }
    let style_doc = fetch_json(&style_url)
        .await
        .ok_or_else(|| Reply::err("network", "could not fetch the map style"))?;
    let vector = style_doc
        .get("sources")
        .and_then(Value::as_object)
        .and_then(|m| m.values().find(|s| s.get("type").and_then(Value::as_str) == Some("vector")))
        .cloned()
        .ok_or_else(|| Reply::err("unavailable", "the map style has no vector source"))?;
    // Inline `tiles` is already tilejson-shaped; a `url` points at one.
    let tilejson = if vector.get("tiles").is_some() {
        vector
    } else {
        let url = vector.get("url").and_then(Value::as_str).unwrap_or("");
        if !super::sane(url) {
            return Err(Reply::err("unavailable", "the style's tile index is not http(s)"));
        }
        fetch_json(url)
            .await
            .ok_or_else(|| Reply::err("network", "could not fetch the tile index"))?
    };
    let src = render::resolve(&style_url, &style_doc, &tilejson)
        .ok_or_else(|| Reply::err("unavailable", "the tile index names no tiles"))?;
    *SOURCE.lock() = Some((style_url, src.clone()));
    Ok(src)
}

/// `geo:lat,lon[,alt][;params]` → (lat, lon), Mercator-representable only.
fn parse_geo(uri: &str) -> Option<(f64, f64)> {
    let coords = uri.trim().strip_prefix("geo:")?.split(';').next()?;
    let mut it = coords.split(',');
    let lat: f64 = it.next()?.trim().parse().ok()?;
    let lon: f64 = it.next()?.trim().parse().ok()?;
    if !lat.is_finite() || !lon.is_finite() || lat.abs() > 85.06 || lon.abs() > 180.0 {
        return None;
    }
    Some((lat, lon))
}

/// Web-Mercator world-pixel position at `z`, with `TILE_PX` tiles.
fn world_px(lat: f64, lon: f64, z: u32) -> (f64, f64) {
    let scale = f64::from(TILE_PX) * (1u64 << z) as f64;
    let x = (lon + 180.0) / 360.0 * scale;
    let lr = lat.to_radians();
    let y = (1.0 - ((lr.tan() + 1.0 / lr.cos()).ln()) / std::f64::consts::PI) / 2.0 * scale;
    (x, y)
}

/// `location.map {geoUri, width?, height?, zoom?}` → `{path, width, height}`:
/// a crop of rendered tiles centred on the point, cached by geo+zoom+size.
pub async fn location_map(engine: &SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    let geo = p.get("geoUri").and_then(Value::as_str).unwrap_or("");
    let Some((lat, lon)) = parse_geo(geo) else {
        return Reply::err("bad_request", "geoUri must look like geo:lat,lon");
    };
    let w = p.get("width").and_then(Value::as_u64).unwrap_or(640).clamp(64, 2048) as u32;
    let h = p.get("height").and_then(Value::as_u64).unwrap_or(400).clamp(64, 2048) as u32;
    let z = p.get("zoom").and_then(Value::as_u64).unwrap_or(15).clamp(3, 19) as u32;

    let dir = crate::paths::cache_dir().join("maps");
    let _ = crate::paths::ensure_private_dir(&dir);
    let path = dir.join(format!("loc{lat:.5}_{lon:.5}-z{z}-{w}x{h}.png"));
    if path.is_file() {
        return Reply::ok(json!({"path": path.to_string_lossy(), "width": w, "height": h}));
    }

    let src = match source_for(engine).await {
        Ok(s) => s,
        Err(e) => return e,
    };

    let (cx, cy) = world_px(lat, lon, z);
    let left = (cx - f64::from(w) / 2.0).round() as i64;
    let top = (cy - f64::from(h) / 2.0).round() as i64;
    let bg = src.style.background;
    let mut out = image::RgbaImage::from_pixel(w, h, image::Rgba([bg.0, bg.1, bg.2, bg.3]));
    let tp = i64::from(TILE_PX);
    for ty in top.div_euclid(tp)..=(top + i64::from(h) - 1).div_euclid(tp) {
        for tx in left.div_euclid(tp)..=(left + i64::from(w) - 1).div_euclid(tp) {
            // A missing tile (fetch failure, off the top of the world) stays
            // the style's background colour.
            let Some(tile) = render::tile(engine, &src, z, tx, ty).await else { continue };
            image::imageops::overlay(&mut out, &tile, tx * tp - left, ty * tp - top);
        }
    }
    if out.save(&path).is_err() {
        return Reply::err("internal", "could not write the map composite");
    }
    Reply::ok(json!({"path": path.to_string_lossy(), "width": w, "height": h}))
}

/// `map.tile {z,x,y}` → `{path}`: one rendered 512px tile from the cache,
/// for the interactive page. x wraps around the antimeridian like the renderer.
pub async fn map_tile(engine: &SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    let Some(z) = p.get("z").and_then(Value::as_u64).filter(|z| *z <= 22).map(|z| z as u32) else {
        return Reply::err("bad_request", "z must be 0..=22");
    };
    let x = p.get("x").and_then(Value::as_i64).unwrap_or(-1);
    let y = p.get("y").and_then(Value::as_i64).unwrap_or(-1);
    let n = 1i64 << z;
    if y < 0 || y >= n {
        return Reply::err("bad_request", "y is outside the tile grid");
    }
    let x = x.rem_euclid(n);
    // The style comes first now, even for a tile already on disk: the cache
    // key names the style the tile was drawn from (`VectorSource::raster_key`)
    // and there is no way to look for a file without knowing it. Resolving is
    // a lock and a clone once the first tile has done it.
    let src = match source_for(engine).await {
        Ok(s) => s,
        Err(e) => return e,
    };
    // Already rendered: the reply is a path, and the path is already good.
    // This is the whole of a warm hit, and it is what the page's prefetching
    // leans on — asking for a tile a second time must cost a `stat` and not a
    // render. It used to call `tile` for the boolean, which read the PNG back,
    // decoded every one of its 512 squares, un-premultiplied a megabyte of it
    // and then dropped the picture on the floor and rebuilt this same path.
    if render::have(&src, z, x, y) {
        return Reply::ok(json!({"path": render::png_path(&src, z, x, y).to_string_lossy()}));
    }
    if render::tile(engine, &src, z, x, y).await.is_none() {
        return Reply::err("network", "the tile could not be fetched or rendered");
    }
    Reply::ok(json!({"path": render::png_path(&src, z, x, y).to_string_lossy()}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geo_uris_parse_with_altitude_and_params() {
        assert_eq!(parse_geo("geo:51.5,-0.12"), Some((51.5, -0.12)));
        assert_eq!(parse_geo("geo:51.5,-0.12,30;u=10"), Some((51.5, -0.12)));
        assert_eq!(parse_geo(" geo:-33.9,151.2 "), Some((-33.9, 151.2)));
    }

    #[test]
    fn nonsense_geo_uris_are_refused() {
        assert_eq!(parse_geo("51.5,-0.12"), None); // no scheme
        assert_eq!(parse_geo("geo:north,west"), None);
        assert_eq!(parse_geo("geo:91.0,0"), None); // off the Mercator sheet
        assert_eq!(parse_geo("geo:0,181.0"), None);
        assert_eq!(parse_geo("geo:0"), None); // no longitude
    }

    #[test]
    fn the_origin_sits_at_the_centre_of_the_world() {
        // At z1 the world is 2×2 tiles of 512px: (0,0) lands at (512,512).
        let (x, y) = world_px(0.0, 0.0, 1);
        assert!((x - 512.0).abs() < 1e-6);
        assert!((y - 512.0).abs() < 1e-6);
    }

    #[test]
    fn west_is_left_and_north_is_up() {
        let (x0, y0) = world_px(0.0, 0.0, 3);
        let (xw, _) = world_px(0.0, -90.0, 3);
        let (_, yn) = world_px(45.0, 0.0, 3);
        assert!(xw < x0);
        assert!(yn < y0);
    }

    #[test]
    fn zooming_in_doubles_the_world() {
        let (x1, y1) = world_px(37.0, -122.0, 10);
        let (x2, y2) = world_px(37.0, -122.0, 11);
        assert!((x2 - 2.0 * x1).abs() < 1e-6);
        assert!((y2 - 2.0 * y1).abs() < 1e-6);
    }
}
