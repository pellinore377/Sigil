use galileo_mvt::{MvtGeometry, MvtTile, MvtValue};
use galileo_types::{cartesian::CartesianPoint2d, Contour, MultiContour, MultiPolygon, Polygon};
use js_sys::Uint8Array;
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    io::Read,
};
use wasm_bindgen::{prelude::*, JsCast};
use web_sys::{CanvasRenderingContext2d, CanvasWindingRule, HtmlCanvasElement};

const MAX_TILES: usize = 9;
const MAX_POINTS: usize = 64_000;
const TILE_SIZE: f64 = 512.0;
const LAYERS: &[&str] = &[
    "earth",
    "landcover",
    "landuse",
    "water",
    "natural",
    "physical_line",
    "physical_point",
    "roads",
    "transit",
    "buildings",
    "boundaries",
    "places",
    "pois",
];
thread_local! { static ACTIVE: Cell<bool> = const {Cell::new(false)}; }
fn fail(message: &str) -> JsValue {
    js_sys::Error::new(message).into()
}
fn valid(lat: f64, lon: f64) -> bool {
    lat.is_finite()
        && lon.is_finite()
        && (-85.051129..=85.051129).contains(&lat)
        && (-180.0..=180.0).contains(&lon)
}
use crate::bounds::{geographic, preflight, world, TILE_BYTES};
#[derive(Clone)]
struct Path {
    points: Vec<(f64, f64)>,
    closed: bool,
}
struct Feature {
    layer: usize,
    paths: Vec<Path>,
    label: Option<(String, f64, f64)>,
    major: bool,
}
struct Tile {
    features: Vec<Feature>,
}
type Key = (u32, u32, u32);

#[wasm_bindgen]
pub struct MapRenderer {
    canvas: HtmlCanvasElement,
    context: CanvasRenderingContext2d,
    dark: bool,
    sans: bool,
    center: (f64, f64),
    zoom: u32,
    width: f64,
    height: f64,
    dpr: f64,
    tiles: BTreeMap<Key, Tile>,
    wanted: BTreeSet<Key>,
    configured: bool,
    font: web_sys::FontFace,
}
impl Drop for MapRenderer {
    fn drop(&mut self) {
        if let Some(document) = web_sys::window().and_then(|w| w.document()) {
            let _ = document.fonts().delete(&self.font);
        }
        self.canvas.set_width(1);
        self.canvas.set_height(1);
        ACTIVE.with(|active| active.set(false));
    }
}
#[wasm_bindgen]
pub async fn create_map(
    canvas: HtmlCanvasElement,
    dark: bool,
    sans: bool,
) -> Result<MapRenderer, JsValue> {
    if ACTIVE.with(|active| active.replace(true)) {
        return Err(fail("Another map is already open"));
    }
    let result = async {
        let context = canvas
            .get_context("2d")?
            .ok_or_else(|| fail("Map drawing is unavailable"))?
            .dyn_into::<CanvasRenderingContext2d>()?;
        let name = if sans {
            "SigilMapSans"
        } else {
            "SigilMapSerif"
        };
        let bytes: &[u8] = if sans {
            include_bytes!("../../shared/src/commonMain/composeResources/font/google_sans_flex.ttf")
        } else {
            include_bytes!("../../shared/src/commonMain/composeResources/font/newsreader.ttf")
        };
        let face =
            web_sys::FontFace::new_with_array_buffer(name, &Uint8Array::from(bytes).buffer())?;
        wasm_bindgen_futures::JsFuture::from(face.load()?).await?;
        web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| fail("Map document unavailable"))?
            .fonts()
            .add(&face)?;
        Ok(MapRenderer {
            canvas,
            context,
            dark,
            sans,
            center: (0.5, 0.5),
            zoom: 1,
            width: 1.0,
            height: 1.0,
            dpr: 1.0,
            tiles: BTreeMap::new(),
            wanted: BTreeSet::new(),
            configured: false,
            font: face,
        })
    }
    .await;
    if result.is_err() {
        ACTIVE.with(|active| active.set(false));
    }
    result
}
#[wasm_bindgen]
impl MapRenderer {
    pub fn configure(&mut self, resource: &[u8]) -> Result<(), JsValue> {
        let style = std::str::from_utf8(resource_body(resource)?)
            .map_err(|_| fail("Invalid map style encoding"))?;
        if style.len() > 1024 * 1024 {
            return Err(fail("Map style is too large"));
        }
        let style: serde_json::Value =
            serde_json::from_str(style).map_err(|_| fail("Invalid map style"))?;
        let sources = style["sources"]
            .as_object()
            .ok_or_else(|| fail("Missing map sources"))?;
        if sources.len() != 1 || sources.values().any(|source| source["type"] != "vector") {
            return Err(fail("This map source is not supported in the browser"));
        }
        let layers = style["layers"]
            .as_array()
            .ok_or_else(|| fail("Missing map layers"))?;
        let names: Vec<_> = layers
            .iter()
            .filter_map(|layer| layer["source-layer"].as_str())
            .collect();
        if names.is_empty() || names.iter().any(|name| !LAYERS.contains(name)) {
            return Err(fail("This browser map supports the Protomaps profile"));
        }
        self.configured = true;
        Ok(())
    }
    pub fn metadata(&self, resource: &[u8]) -> Result<String, JsValue> {
        let value: serde_json::Value = serde_json::from_slice(resource_body(resource)?)
            .map_err(|_| fail("Invalid map metadata"))?;
        let minimum = value["minzoom"].as_u64().unwrap_or(0).min(18);
        let maximum = value["maxzoom"].as_u64().unwrap_or(18).min(18);
        if minimum > maximum {
            return Err(fail("Unsupported map zoom range"));
        }
        let center = value["center"]
            .as_array()
            .and_then(|v| Some((v.get(1)?.as_f64()?, v.first()?.as_f64()?)))
            .filter(|(a, b)| valid(*a, *b))
            .unwrap_or((0.0, 0.0));
        Ok(
            serde_json::json!({"lat":center.0,"lon":center.1,"min":minimum,"max":maximum})
                .to_string(),
        )
    }
    pub fn view(
        &mut self,
        lat: f64,
        lon: f64,
        zoom: u32,
        width: u32,
        height: u32,
        dpr: f64,
    ) -> Result<String, JsValue> {
        if !self.configured
            || !valid(lat, lon)
            || zoom > 18
            || !(1..=1024).contains(&width)
            || !(1..=768).contains(&height)
            || !dpr.is_finite()
            || !(1.0..=2.0).contains(&dpr)
        {
            return Err(fail("Invalid map viewport"));
        }
        if self.zoom != zoom {
            self.tiles.clear();
        }
        self.center = world(lat, lon);
        self.zoom = zoom;
        self.width = width as f64;
        self.height = height as f64;
        self.dpr = dpr;
        let pixel_width = (self.width * dpr).round() as u32;
        let pixel_height = (self.height * dpr).round() as u32;
        if self.canvas.width() != pixel_width {
            self.canvas.set_width(pixel_width);
        }
        if self.canvas.height() != pixel_height {
            self.canvas.set_height(pixel_height);
        }
        let side = 1u32 << zoom;
        let scale = TILE_SIZE * side as f64;
        let minx = ((self.center.0 * scale - self.width / 2.0) / TILE_SIZE).floor() as i64;
        let maxx = ((self.center.0 * scale + self.width / 2.0 - 0.001) / TILE_SIZE).floor() as i64;
        let miny = ((self.center.1 * scale - self.height / 2.0) / TILE_SIZE)
            .floor()
            .max(0.0) as i64;
        let maxy = ((self.center.1 * scale + self.height / 2.0 - 0.001) / TILE_SIZE)
            .floor()
            .min(side as f64 - 1.0) as i64;
        let mut wanted = BTreeSet::new();
        for x in minx..=maxx {
            for y in miny..=maxy {
                wanted.insert((zoom, x.rem_euclid(side as i64) as u32, y as u32));
            }
        }
        if wanted.len() > MAX_TILES {
            return Err(fail("Map viewport requests too many tiles"));
        }
        self.wanted = wanted;
        self.tiles.retain(|key, _| self.wanted.contains(key));
        self.draw()?;
        serde_json::to_string(
            &self
                .wanted
                .iter()
                .filter(|key| !self.tiles.contains_key(key))
                .map(|(z, x, y)| serde_json::json!({"z":z,"x":x,"y":y}))
                .collect::<Vec<_>>(),
        )
        .map_err(|_| fail("Invalid tile request"))
    }
    pub fn tile(&mut self, z: u32, x: u32, y: u32, bytes: &[u8]) -> Result<(), JsValue> {
        let key = (z, x, y);
        if !self.wanted.contains(&key) {
            return Ok(());
        }
        let bytes = resource_body(bytes)?;
        if bytes.len() > TILE_BYTES {
            return Err(fail("Map tile is too large"));
        }
        let mut raw = Vec::new();
        let data = if bytes.starts_with(&[0x1f, 0x8b]) {
            flate2::read::GzDecoder::new(bytes)
                .take(TILE_BYTES as u64 + 1)
                .read_to_end(&mut raw)
                .map_err(|_| fail("Invalid map compression"))?;
            if raw.len() > TILE_BYTES {
                return Err(fail("Map tile is too large"));
            }
            raw.as_slice()
        } else {
            bytes
        };
        let tile = if data.is_empty() {
            Tile { features: vec![] }
        } else {
            decode(data)?
        };
        self.tiles.insert(key, tile);
        self.draw()
    }
    pub fn point(&self, x: f64, y: f64) -> Result<String, JsValue> {
        if !x.is_finite() || !y.is_finite() {
            return Err(fail("Invalid map point"));
        }
        let scale = TILE_SIZE * (1u32 << self.zoom) as f64;
        let (lat, lon) = geographic(
            self.center.0 + (x - self.width / 2.0) / scale,
            (self.center.1 + (y - self.height / 2.0) / scale).clamp(0.0, 1.0),
        );
        Ok(serde_json::json!({"lat":lat,"lon":lon}).to_string())
    }
    pub fn project(&self, lat: f64, lon: f64) -> Result<String, JsValue> {
        if !valid(lat, lon) {
            return Err(fail("Invalid map point"));
        }
        let (x, y) = self.screen(world(lat, lon));
        Ok(serde_json::json!({"x":x,"y":y}).to_string())
    }
    pub fn snapshot(&self) -> Result<String, JsValue> {
        self.canvas.to_data_url_with_type("image/png")
    }
}
fn decode(data: &[u8]) -> Result<Tile, JsValue> {
    preflight(data).map_err(|_| fail("Map tile exceeds decoding limits"))?;
    let decoded = MvtTile::decode(data, false).map_err(|_| fail("Invalid vector tile"))?;
    if decoded.layers.len() > 32
        || decoded
            .layers
            .iter()
            .map(|l| l.features.len())
            .sum::<usize>()
            > 16000
    {
        return Err(fail("Map tile exceeds geometry limits"));
    }
    let mut features = Vec::new();
    let mut count = 0usize;
    for layer in decoded.layers {
        let Some(rank) = LAYERS.iter().position(|name| *name == layer.name) else {
            return Err(fail("Unsupported map tile layer"));
        };
        for feature in layer.features {
            let mut paths = Vec::new();
            let mut label = None;
            let mut add = |points: Vec<(f64, f64)>, closed: bool| -> Result<(), JsValue> {
                count += points.len();
                if count > MAX_POINTS
                    || points.iter().any(|(x, y)| {
                        !x.is_finite() || !y.is_finite() || x.abs() > 16.0 || y.abs() > 16.0
                    })
                {
                    return Err(fail("Map geometry is too complex"));
                }
                paths.push(Path { points, closed });
                Ok(())
            };
            match feature.geometry {
                MvtGeometry::Polygon(polygons) => {
                    for polygon in polygons.polygons() {
                        add(
                            polygon
                                .outer_contour()
                                .iter_points()
                                .map(|p| (p.x() as f64, p.y() as f64))
                                .collect(),
                            true,
                        )?;
                        for ring in polygon.inner_contours() {
                            add(
                                ring.iter_points()
                                    .map(|p| (p.x() as f64, p.y() as f64))
                                    .collect(),
                                true,
                            )?;
                        }
                    }
                }
                MvtGeometry::LineString(lines) => {
                    for line in lines.contours() {
                        add(
                            line.iter_points()
                                .map(|p| (p.x() as f64, p.y() as f64))
                                .collect(),
                            false,
                        )?;
                    }
                }
                MvtGeometry::Point(points) => {
                    count += points.len();
                    if count > MAX_POINTS
                        || points.iter().any(|p| {
                            !p.x().is_finite()
                                || !p.y().is_finite()
                                || p.x().abs() > 16.0
                                || p.y().abs() > 16.0
                        })
                    {
                        return Err(fail("Map geometry is too complex"));
                    }
                    if matches!(layer.name.as_str(), "places" | "pois" | "physical_point") {
                        if let (Some(MvtValue::String(name)), Some(point)) =
                            (feature.properties.get("name"), points.first())
                        {
                            if !name.is_empty() && name.len() <= 160 {
                                label = Some((name.clone(), point.x() as f64, point.y() as f64));
                            }
                        }
                    }
                }
            }
            let major = feature
                .properties
                .get("kind")
                .is_some_and(|value| value.eq_str("highway") || value.eq_str("major_road"));
            if !paths.is_empty() || label.is_some() {
                features.push(Feature {
                    layer: rank,
                    paths,
                    label,
                    major,
                });
            }
        }
    }
    features.sort_by_key(|feature| feature.layer);
    Ok(Tile { features })
}
impl MapRenderer {
    fn screen(&self, point: (f64, f64)) -> (f64, f64) {
        let scale = TILE_SIZE * (1u32 << self.zoom) as f64;
        let dx = (point.0 - self.center.0 + 0.5).rem_euclid(1.0) - 0.5;
        (
            self.width / 2.0 + dx * scale,
            self.height / 2.0 + (point.1 - self.center.1) * scale,
        )
    }
    fn draw(&self) -> Result<(), JsValue> {
        let c = &self.context;
        c.set_transform(self.dpr, 0.0, 0.0, self.dpr, 0.0, 0.0)?;
        let background = if self.dark { "#25272a" } else { "#eeece7" };
        c.set_fill_style_str(background);
        c.fill_rect(0.0, 0.0, self.width, self.height);
        c.set_line_join("round");
        c.set_line_cap("round");
        let side = (1u32 << self.zoom) as f64;
        for layer in 0..LAYERS.len() {
            for ((_, x, y), tile) in &self.tiles {
                let center = self.screen(((*x as f64 + 0.5) / side, (*y as f64 + 0.5) / side));
                for copy in -1..=1 {
                    let origin = (
                        center.0 - TILE_SIZE / 2.0 + copy as f64 * side * TILE_SIZE,
                        center.1 - TILE_SIZE / 2.0,
                    );
                    if origin.0 > self.width || origin.0 + TILE_SIZE < 0.0 {
                        continue;
                    }
                    c.save();
                    c.begin_path();
                    c.rect(origin.0, origin.1, TILE_SIZE, TILE_SIZE);
                    c.clip();
                    for feature in tile
                        .features
                        .iter()
                        .filter(|feature| feature.layer == layer)
                    {
                        let name = LAYERS[layer];
                        let fill = match name {
                            "water" => {
                                if self.dark {
                                    "#304653"
                                } else {
                                    "#bdd4dd"
                                }
                            }
                            "landcover" | "landuse" | "natural" => {
                                if self.dark {
                                    "#303b34"
                                } else {
                                    "#dde3d8"
                                }
                            }
                            "buildings" => {
                                if self.dark {
                                    "#484746"
                                } else {
                                    "#d4d1ca"
                                }
                            }
                            _ => background,
                        };
                        c.set_fill_style_str(fill);
                        c.set_stroke_style_str(match name {
                            "roads" => {
                                if self.dark {
                                    "#a29d92"
                                } else {
                                    "#ffffff"
                                }
                            }
                            "water" => {
                                if self.dark {
                                    "#456572"
                                } else {
                                    "#bdd4dd"
                                }
                            }
                            "boundaries" => {
                                if self.dark {
                                    "#88847c"
                                } else {
                                    "#aba396"
                                }
                            }
                            _ => {
                                if self.dark {
                                    "#676967"
                                } else {
                                    "#bbb8b0"
                                }
                            }
                        });
                        c.set_line_width(if name == "roads" {
                            if feature.major {
                                3.5
                            } else {
                                1.5
                            }
                        } else {
                            1.0
                        });
                        c.begin_path();
                        for path in &feature.paths {
                            for (i, (px, py)) in path.points.iter().enumerate() {
                                let p = (origin.0 + px * TILE_SIZE, origin.1 + py * TILE_SIZE);
                                if i == 0 {
                                    c.move_to(p.0, p.1)
                                } else {
                                    c.line_to(p.0, p.1)
                                }
                            }
                            if path.closed {
                                c.close_path();
                            }
                        }
                        if feature.paths.iter().any(|path| path.closed) {
                            c.fill_with_canvas_winding_rule(CanvasWindingRule::Evenodd);
                        } else {
                            c.stroke();
                        }
                    }
                    c.restore();
                }
            }
        }
        c.set_font(&format!(
            "14px {}",
            if self.sans {
                "SigilMapSans"
            } else {
                "SigilMapSerif"
            }
        ));
        c.set_text_align("center");
        c.set_text_baseline("middle");
        c.set_line_width(3.0);
        c.set_fill_style_str(if self.dark { "#f2eee5" } else { "#383832" });
        c.set_stroke_style_str(background);
        let mut occupied: Vec<(f64, f64, f64, f64)> = Vec::new();
        let mut labels = BTreeSet::new();
        for priority in ["places", "physical_point", "pois"] {
            for ((_, x, y), tile) in &self.tiles {
                for feature in tile
                    .features
                    .iter()
                    .filter(|feature| LAYERS[feature.layer] == priority)
                {
                    if occupied.len() >= 96 {
                        break;
                    }
                    if let Some((name, px, py)) = &feature.label {
                        let p = self.screen(((*x as f64 + px) / side, (*y as f64 + py) / side));
                        if p.0 < 0.0
                            || p.1 < 0.0
                            || p.0 > self.width
                            || p.1 > self.height
                            || labels.contains(name)
                        {
                            continue;
                        }
                        let w = c.measure_text(name)?.width().min(220.0);
                        let rect = (
                            p.0 - w / 2.0 - 5.0,
                            p.1 - 12.0,
                            p.0 + w / 2.0 + 5.0,
                            p.1 + 12.0,
                        );
                        if occupied
                            .iter()
                            .any(|r| r.0 < rect.2 && r.2 > rect.0 && r.1 < rect.3 && r.3 > rect.1)
                        {
                            continue;
                        }
                        occupied.push(rect);
                        labels.insert(name.clone());
                        c.stroke_text_with_max_width(name, p.0, p.1, 220.0)?;
                        c.fill_text_with_max_width(name, p.0, p.1, 220.0)?;
                    }
                }
            }
        }
        Ok(())
    }
}
fn resource_body(bytes: &[u8]) -> Result<&[u8], JsValue> {
    if bytes.len() > TILE_BYTES + 256 {
        return Err(fail("Map resource too large"));
    }
    let mut ends = bytes
        .iter()
        .enumerate()
        .filter_map(|(i, b)| (*b == b'\n').then_some(i));
    let a = ends.next().ok_or_else(|| fail("Invalid map response"))?;
    let b = ends.next().ok_or_else(|| fail("Invalid map response"))?;
    let c = ends.next().ok_or_else(|| fail("Invalid map response"))?;
    if c > 256 {
        return Err(fail("Invalid map response"));
    }
    if &bytes[..a] == b"204" {
        return Ok(&[]);
    }
    if &bytes[..a] != b"200" {
        return Err(fail("Map resource unavailable"));
    }
    if !matches!(&bytes[b + 1..c], b"" | b"gzip") {
        return Err(fail("Unsupported map compression"));
    }
    Ok(&bytes[c + 1..])
}
