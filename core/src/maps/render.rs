//! Vector tile rasterisation: the discovered server's own cartography, drawn
//! with tiny-skia. Backs both the static location composites and `map.tile`
//! for the interactive page. Tiles render at 2× (512px) so the cards stay
//! crisp on the phone.

use std::sync::Arc;

use serde_json::Value;
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};
use tracing::{debug, warn};

use super::{mvt, style};
use crate::engine::SharedEngine;

pub const TILE_PX: u32 = 512;

/// Everything the renderer needs, resolved once per style refresh.
pub struct VectorSource {
    pub template: String, // …/{z}/{x}/{y}.mvt
    pub maxzoom: u32,
    pub style: style::MapStyle,
}

pub fn resolve(style_doc: &Value, tilejson: &Value) -> Option<Arc<VectorSource>> {
    let template = tilejson
        .get("tiles")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(Value::as_str)?
        .to_string();
    if !template.contains("{z}") {
        return None;
    }
    let maxzoom = tilejson.get("maxzoom").and_then(Value::as_u64).unwrap_or(15) as u32;
    Some(Arc::new(VectorSource { template, maxzoom, style: style::parse(style_doc) }))
}

/// Where a rendered tile PNG lands (also the map.tile reply path).
pub fn png_path(z: u32, x: i64, y: i64) -> std::path::PathBuf {
    cache_path(z, x, y)
}

fn cache_path(z: u32, x: i64, y: i64) -> std::path::PathBuf {
    let d = crate::paths::cache_dir().join("tiles");
    let _ = crate::paths::ensure_private_dir(&d);
    d.join(format!("v-{z}-{x}-{y}.png"))
}

fn mvt_cache_path(z: u32, x: i64, y: i64) -> std::path::PathBuf {
    let d = crate::paths::cache_dir().join("tiles");
    let _ = crate::paths::ensure_private_dir(&d);
    d.join(format!("v-{z}-{x}-{y}.mvt"))
}

async fn fetch_mvt(src: &VectorSource, z: u32, x: i64, y: i64) -> Option<Vec<u8>> {
    let p = mvt_cache_path(z, x, y);
    if let Ok(data) = std::fs::read(&p) {
        return Some(data);
    }
    let url = src
        .template
        .replace("{z}", &z.to_string())
        .replace("{x}", &x.to_string())
        .replace("{y}", &y.to_string());
    let client = crate::net::http_builder()
        .user_agent(format!("Sigil/{} (Matrix client; maps)", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .ok()?;
    let resp = client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        debug!("map tile {url}: {}", resp.status());
        return None;
    }
    let data = resp.bytes().await.ok()?.to_vec();
    let _ = std::fs::write(&p, &data);
    Some(data)
}

/// Render one raster tile at `TILE_PX`. Beyond the source's maxzoom the parent
/// tile is over-zoomed (crop + scale), which is how every slippy map extends
/// its deepest data level.
pub async fn tile(
    engine: &SharedEngine,
    src: &Arc<VectorSource>,
    z: u32,
    x: i64,
    y: i64,
) -> Option<image::RgbaImage> {
    let _ = engine;
    let n = 1i64 << z;
    if y < 0 || y >= n {
        return None;
    }
    let x = x.rem_euclid(n);

    let cache = cache_path(z, x, y);
    if let Ok(data) = std::fs::read(&cache) {
        if let Ok(img) = image::load_from_memory(&data) {
            return Some(img.to_rgba8());
        }
    }

    // Over-zoom: fetch the deepest available ancestor and window into it.
    let (fz, fx, fy, scale, off_x, off_y) = if z > src.maxzoom {
        let d = z - src.maxzoom;
        (src.maxzoom, x >> d, y >> d, (1u32 << d) as f32, (x & ((1 << d) - 1)) as f32, (y & ((1 << d) - 1)) as f32)
    } else {
        (z, x, y, 1.0, 0.0, 0.0)
    };

    let data = fetch_mvt(src, fz, fx, fy).await?;
    let layers = mvt::decode(&data);
    if layers.is_empty() {
        return None;
    }

    let mut pixmap = Pixmap::new(TILE_PX, TILE_PX)?;
    let bg = src.style.background;
    pixmap.fill(tiny_skia::Color::from_rgba8(bg.0, bg.1, bg.2, bg.3));

    // The zoom the *style* evaluates at (line widths), in CSS px at 1×; we
    // render at 2× so widths double.
    let style_zoom = z as f64;

    for draw in &src.style.layers {
        let Some(layer) = layers.iter().find(|l| l.name == draw.source_layer) else { continue };
        // extent units → 512px canvas, windowed for over-zoom.
        let s = TILE_PX as f32 / layer.extent as f32 * scale;
        let tx = -(off_x / scale.max(1.0)) * TILE_PX as f32 * scale / 1.0;
        let _ = tx;
        let offset_x = -off_x * TILE_PX as f32;
        let offset_y = -off_y * TILE_PX as f32;
        let ts = Transform::from_row(s, 0.0, 0.0, s, offset_x, offset_y);

        for f in &layer.features {
            if !draw.matches(f) {
                continue;
            }
            match draw.kind {
                style::DrawKind::Fill => {
                    if f.geom_type != mvt::GeomType::Polygon {
                        continue;
                    }
                    let c = draw.color(f);
                    if c.3 == 0 {
                        continue;
                    }
                    let mut pb = PathBuilder::new();
                    for ring in &f.paths {
                        if ring.len() < 3 {
                            continue;
                        }
                        pb.move_to(ring[0].0, ring[0].1);
                        for p in &ring[1..] {
                            pb.line_to(p.0, p.1);
                        }
                        pb.close();
                    }
                    let Some(path) = pb.finish() else { continue };
                    let mut paint = Paint::default();
                    paint.set_color_rgba8(c.0, c.1, c.2, c.3);
                    paint.anti_alias = true;
                    pixmap.fill_path(&path, &paint, FillRule::EvenOdd, ts, None);
                }
                style::DrawKind::Line => {
                    if f.geom_type == mvt::GeomType::Point {
                        continue;
                    }
                    let c = draw.color(f);
                    if c.3 == 0 {
                        continue;
                    }
                    // Style widths are CSS px at 1×; the canvas is 2×, and the
                    // transform must not scale the stroke — build the path in
                    // canvas space instead.
                    let w = (draw.width(style_zoom, f) * 2.0) as f32;
                    if w <= 0.05 {
                        continue;
                    }
                    let mut pb = PathBuilder::new();
                    for line in &f.paths {
                        if line.len() < 2 {
                            continue;
                        }
                        pb.move_to(line[0].0 * s + offset_x, line[0].1 * s + offset_y);
                        for p in &line[1..] {
                            pb.line_to(p.0 * s + offset_x, p.1 * s + offset_y);
                        }
                    }
                    let Some(path) = pb.finish() else { continue };
                    let mut paint = Paint::default();
                    paint.set_color_rgba8(c.0, c.1, c.2, c.3);
                    paint.anti_alias = true;
                    let stroke = Stroke {
                        width: w,
                        line_cap: tiny_skia::LineCap::Round,
                        line_join: tiny_skia::LineJoin::Round,
                        ..Stroke::default()
                    };
                    pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
                }
            }
        }
    }

    let img = image::RgbaImage::from_raw(TILE_PX, TILE_PX, demultiply(pixmap.data()))?;
    let mut png = Vec::new();
    if image::DynamicImage::ImageRgba8(img.clone())
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .is_ok()
    {
        let _ = std::fs::write(&cache, &png);
    } else {
        warn!("map tile: png encode failed for {z}/{x}/{y}");
    }
    Some(img)
}

/// tiny-skia stores premultiplied RGBA; `image` wants straight alpha.
fn demultiply(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    for px in data.chunks_exact(4) {
        let a = px[3] as u32;
        if a == 0 || a == 255 {
            out.extend_from_slice(px);
        } else {
            out.push(((px[0] as u32 * 255) / a).min(255) as u8);
            out.push(((px[1] as u32 * 255) / a).min(255) as u8);
            out.push(((px[2] as u32 * 255) / a).min(255) as u8);
            out.push(px[3]);
        }
    }
    out
}
