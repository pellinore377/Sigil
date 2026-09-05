//! Vector tile rasterisation: the discovered server's own cartography, drawn
//! with tiny-skia. Backs both the static location composites and `map.tile`
//! for the interactive page. Tiles render at 2× (512px) so the cards stay
//! crisp on the phone.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use serde_json::Value;
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};
use tracing::{debug, warn};

use super::{labels, mvt, style};
use crate::engine::SharedEngine;

pub const TILE_PX: u32 = 512;

/// What this renderer draws. It is half of the rendered tile's cache key, so
/// **bump it whenever the picture a tile would come out as changes** — a new
/// layer, different widths, labels. Old files are then simply a different key
/// and are never read again.
///
/// 2: street and place names.
/// 3: names placed across the tile's whole 3x3 neighbourhood, so one that
///    crosses a tile edge is drawn whole by both tiles instead of being cut in
///    half by each; near-vertical names read upward; street text at 11-12 dp.
const RENDER_VERSION: u32 = 3;

/// Everything the renderer needs, resolved once per style refresh.
pub struct VectorSource {
    pub template: String, // …/{z}/{x}/{y}.mvt
    pub maxzoom: u32,
    pub style: style::MapStyle,
    /// The rendered-tile cache key: this style and this renderer.
    ///
    /// The old key was `v-{z}-{x}-{y}` with nothing in it about where the
    /// cartography came from, so switching map styles kept serving the old
    /// one for ever — the tiles were already on disk under the only name the
    /// cache knew. Now a different style, or a different `RENDER_VERSION`, is
    /// a different set of files.
    pub raster_key: String,
    /// The vector cache key: the tile TEMPLATE alone, and deliberately not
    /// the style or the version. The vectors do not change when the drawing
    /// does, and re-fetching a city over the network to redraw it with names
    /// on would be minutes of waiting for something already on the disk.
    pub vector_key: String,
}

/// A short stable name for a string. FNV-1a — the cache wants a filename, not
/// a promise about adversaries.
fn short_hash(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    format!("{h:012x}")
}

pub fn resolve(style_url: &str, style_doc: &Value, tilejson: &Value) -> Option<Arc<VectorSource>> {
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
    Some(Arc::new(VectorSource {
        raster_key: format!("r{RENDER_VERSION}{}", short_hash(style_url)),
        vector_key: format!("v{}", short_hash(&template)),
        template,
        maxzoom,
        style: style::parse(style_doc),
    }))
}

/// Where a rendered tile PNG lands (also the map.tile reply path).
pub fn png_path(src: &VectorSource, z: u32, x: i64, y: i64) -> std::path::PathBuf {
    cache_path(src, z, x, y)
}

fn cache_path(src: &VectorSource, z: u32, x: i64, y: i64) -> std::path::PathBuf {
    let d = crate::paths::cache_dir().join("tiles");
    let _ = crate::paths::ensure_private_dir(&d);
    d.join(format!("{}-{z}-{x}-{y}.png", src.raster_key))
}

fn mvt_cache_path(src: &VectorSource, z: u32, x: i64, y: i64) -> std::path::PathBuf {
    let d = crate::paths::cache_dir().join("tiles");
    let _ = crate::paths::ensure_private_dir(&d);
    d.join(format!("{}-{z}-{x}-{y}.mvt", src.vector_key))
}

/// Beside itself, then renamed into place — never written over.
///
/// Nothing single-flights this cache, so the same tile can be fetched and
/// rendered by several requests at once, and `fs::write` truncates before it
/// writes: a reader arriving mid-write used to get a torn file, which is why
/// both readers here have to tolerate their content being nonsense. A rename
/// is atomic, so a reader sees the whole of the old file or the whole of the
/// new one. It is also what lets `have` trust that a PNG on disk is a PNG,
/// and that is what makes a prefetched tile cost nothing the second time.
fn write_atomic(path: &std::path::Path, data: &[u8]) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let tmp = path.with_extension(format!(
        "t{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    if std::fs::write(&tmp, data).is_ok() && std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Is this tile already rendered and on disk?
///
/// The whole of a warm `map.tile`: the reply is a path, and this says the path
/// is good. It used to go the long way round — render the tile, which read the
/// PNG, decoded all 512 squares of it and un-premultiplied a megabyte, purely
/// to learn that the file was there, and then threw the picture away and
/// rebuilt the path anyway. That is the cost a prefetch pays twice, once to
/// warm the tile and once when the view actually reaches it, and it is the
/// difference between prefetching being free and prefetching being the thing
/// that makes the map slow.
pub fn have(src: &VectorSource, z: u32, x: i64, y: i64) -> bool {
    std::fs::metadata(cache_path(src, z, x, y)).map(|m| m.len() > 0).unwrap_or(false)
}

async fn fetch_mvt(src: &VectorSource, z: u32, x: i64, y: i64) -> Option<Vec<u8>> {
    let p = mvt_cache_path(src, z, x, y);
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
    write_atomic(&p, &data);
    Some(data)
}

/// The named features of vector tiles already decoded, so the label pass can
/// read a tile's eight neighbours without decoding eight tiles.
///
/// Every tile's labels are placed across its 3×3 neighbourhood (see
/// `labels::draw`), which would be nine decodes per tile — except that the
/// nine overlap almost entirely with the next tile's nine. Keeping the
/// extracted names, which are a few hundred bytes where the tile is a
/// megabyte, turns nine decodes back into about one.
static NAMED: Mutex<Option<HashMap<(u32, i64, i64), Arc<Vec<labels::Named>>>>> = Mutex::new(None);

/// The names in one vector tile, from the cache or by decoding it.
///
/// Neighbours are fetched like any other tile, so they land in the vector
/// cache and cost nothing when their own turn to be drawn comes round. A
/// neighbour that cannot be had — off the top of the world, a 404, no network
/// — is simply absent, and its names go unwritten until it arrives.
async fn named_of(src: &Arc<VectorSource>, z: u32, x: i64, y: i64) -> Option<Arc<Vec<labels::Named>>> {
    let n = 1i64 << z;
    if y < 0 || y >= n {
        return None;
    }
    let key = (z, x.rem_euclid(n), y);
    if let Some(m) = NAMED.lock().as_ref() {
        if let Some(v) = m.get(&key) {
            return Some(v.clone());
        }
    }
    let data = fetch_mvt(src, key.0, key.1, key.2).await?;
    let got = Arc::new(labels::extract(&mvt::decode(&data)));
    let mut guard = NAMED.lock();
    let m = guard.get_or_insert_with(HashMap::new);
    // A plain bound, not an LRU: a map page walks a neighbourhood and then
    // leaves, and the cheapest correct thing is to start again.
    if m.len() > 512 {
        m.clear();
    }
    m.insert(key, got.clone());
    Some(got)
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

    let cache = cache_path(src, z, x, y);
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

    let began = std::time::Instant::now();
    let data = fetch_mvt(src, fz, fx, fy).await?;
    let fetched = began.elapsed();
    let layers = mvt::decode(&data);
    if layers.is_empty() {
        return None;
    }

    let mut pixmap = Pixmap::new(TILE_PX, TILE_PX)?;
    let bg = src.style.background;
    pixmap.fill(tiny_skia::Color::from_rgba8(bg.0, bg.1, bg.2, bg.3));
    rasterize(&mut pixmap, src, &layers, z, scale, off_x, off_y);
    // …and then the names, over everything the cartography drew. They are a
    // pass of their own rather than another style layer because they are not
    // one: a name has to know where its road went, has to be told what else
    // is already written on this tile, and has to be legible against whatever
    // it lands on. See `labels`.
    // …and then the names, over everything the cartography drew, placed across
    // this tile AND its eight neighbours so that a street crossing the seam is
    // written once, whole, and identically by both tiles (`labels::draw`).
    // The neighbourhood is of the SOURCE tile — at an over-zoom the drawn tile
    // is a window into it, and its neighbours are the source's.
    let ring: Vec<(i64, i64, Arc<Vec<labels::Named>>)> = {
        let mut out = Vec::with_capacity(9);
        let mut jobs = Vec::with_capacity(9);
        for dy in -1i64..=1 {
            for dx in -1i64..=1 {
                jobs.push(async move { (dx, dy, named_of(src, fz, fx + dx, fy + dy).await) });
            }
        }
        for (dx, dy, got) in futures_util::future::join_all(jobs).await {
            if let Some(named) = got {
                out.push((dx, dy, named));
            }
        }
        out
    };
    let ring: Vec<labels::Neighbour> = ring
        .iter()
        .map(|(dx, dy, named)| labels::Neighbour { dx: *dx, dy: *dy, named })
        .collect();
    labels::draw(
        &mut pixmap,
        &ring,
        z,
        fx,
        fy,
        TILE_PX as f32,
        scale,
        off_x,
        off_y,
    );
    // What a miss costs, split at the only line that matters: how much of it
    // was waiting for the network and how much was this thread drawing. The
    // drawing runs on a runtime worker, so it is also how long everything else
    // the app wanted to do was held up. Nothing times a hit — a hit never
    // reaches here, it is a `stat` in `map_tile`.
    debug!(
        "map tile {z}/{x}/{y}: {:.1}ms fetch + {:.1}ms draw, {} features",
        fetched.as_secs_f64() * 1e3,
        (began.elapsed() - fetched).as_secs_f64() * 1e3,
        layers.iter().map(|l| l.features.len()).sum::<usize>(),
    );

    let img = image::RgbaImage::from_raw(TILE_PX, TILE_PX, demultiply(pixmap.data()))?;
    let mut png = Vec::new();
    if image::DynamicImage::ImageRgba8(img.clone())
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .is_ok()
    {
        write_atomic(&cache, &png);
    } else {
        warn!("map tile: png encode failed for {z}/{x}/{y}");
    }
    Some(img)
}

/// Draw the style's layers onto the pixmap: the CPU half of a tile, held apart
/// from the fetching and the encoding around it so it can be timed on its own.
fn rasterize(
    pixmap: &mut Pixmap,
    src: &VectorSource,
    layers: &[mvt::Layer],
    z: u32,
    scale: f32,
    off_x: f32,
    off_y: f32,
) {
    // The zoom the *style* evaluates at (line widths), in CSS px at 1×; we
    // render at 2× so widths double.
    let style_zoom = z as f64;

    for draw in &src.style.layers {
        let Some(layer) = layers.iter().find(|l| l.name == draw.source_layer) else { continue };
        // extent units → 512px canvas, windowed for over-zoom.
        let s = TILE_PX as f32 / layer.extent as f32 * scale;
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


