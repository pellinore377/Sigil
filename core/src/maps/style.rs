//! The slice of the MapLibre style spec the discovered basemap style actually
//! uses: literal paints, `["match",["get","kind"],…]` colour tables,
//! `["interpolate",["exponential"|"linear",…],["zoom"],…]` widths, and
//! equality filters. Enough to draw the server's own cartography instead of a
//! stand-in.

use serde_json::Value;

use super::mvt;

#[derive(Debug, Clone, Copy)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

fn parse_color(s: &str) -> Option<Rgba> {
    let s = s.trim();
    let hex = s.strip_prefix('#')?;
    let (r, g, b, a) = match hex.len() {
        6 => (
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
            255,
        ),
        8 => (
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
            u8::from_str_radix(&hex[6..8], 16).ok()?,
        ),
        3 => (
            u8::from_str_radix(&hex[0..1], 16).ok()? * 17,
            u8::from_str_radix(&hex[1..2], 16).ok()? * 17,
            u8::from_str_radix(&hex[2..3], 16).ok()? * 17,
            255,
        ),
        _ => return None,
    };
    Some(Rgba(r, g, b, a))
}

/// `["match", ["get", key], case, out, case, out, …, fallback]` or a literal.
#[derive(Debug, Clone)]
enum Matcher<T> {
    Literal(T),
    ByKey { key: String, cases: Vec<(String, T)>, fallback: T },
}

impl<T: Clone> Matcher<T> {
    fn eval(&self, tags: &std::collections::HashMap<String, mvt::Value>) -> T {
        match self {
            Matcher::Literal(v) => v.clone(),
            Matcher::ByKey { key, cases, fallback } => {
                let got = tags.get(key).and_then(|v| v.as_str()).unwrap_or("");
                cases
                    .iter()
                    .find(|(c, _)| c == got)
                    .map(|(_, v)| v.clone())
                    .unwrap_or_else(|| fallback.clone())
            }
        }
    }
}

fn parse_matcher<T: Clone>(v: &Value, leaf: &impl Fn(&Value) -> Option<T>) -> Option<Matcher<T>> {
    if let Some(t) = leaf(v) {
        return Some(Matcher::Literal(t));
    }
    let arr = v.as_array()?;
    if arr.first()?.as_str()? != "match" {
        return None;
    }
    let key = arr.get(1)?.as_array()?.get(1)?.as_str()?.to_string();
    let body = &arr[2..];
    let fallback = leaf(body.last()?)?;
    let mut cases = Vec::new();
    for pair in body[..body.len() - 1].chunks_exact(2) {
        let out = leaf(&pair[1])?;
        match &pair[0] {
            Value::String(s) => cases.push((s.clone(), out)),
            Value::Array(alts) => {
                for a in alts {
                    if let Some(s) = a.as_str() {
                        cases.push((s.to_string(), out.clone()));
                    }
                }
            }
            _ => return None,
        }
    }
    Some(Matcher::ByKey { key, cases, fallback })
}

/// `["interpolate", ["exponential", base]|["linear"], ["zoom"], stop, out, …]`
/// where each `out` may itself be a match on a tag. Constants work too.
#[derive(Debug, Clone)]
struct WidthCurve {
    base: f64,
    stops: Vec<(f64, Matcher<f64>)>,
}

impl WidthCurve {
    fn eval(&self, zoom: f64, tags: &std::collections::HashMap<String, mvt::Value>) -> f64 {
        if self.stops.is_empty() {
            return 1.0;
        }
        let vals: Vec<(f64, f64)> =
            self.stops.iter().map(|(z, m)| (*z, m.eval(tags))).collect();
        if zoom <= vals[0].0 {
            return vals[0].1;
        }
        if zoom >= vals[vals.len() - 1].0 {
            return vals[vals.len() - 1].1;
        }
        for w in vals.windows(2) {
            let (z0, v0) = w[0];
            let (z1, v1) = w[1];
            if zoom >= z0 && zoom <= z1 {
                // MapLibre's exponential interpolation.
                let t = if (self.base - 1.0).abs() < 1e-9 {
                    (zoom - z0) / (z1 - z0)
                } else {
                    (self.base.powf(zoom - z0) - 1.0) / (self.base.powf(z1 - z0) - 1.0)
                };
                return v0 + (v1 - v0) * t;
            }
        }
        vals[vals.len() - 1].1
    }
}

fn parse_width(v: &Value) -> Option<WidthCurve> {
    if let Some(n) = v.as_f64() {
        return Some(WidthCurve { base: 1.0, stops: vec![(0.0, Matcher::Literal(n))] });
    }
    let arr = v.as_array()?;
    if arr.first()?.as_str()? != "interpolate" {
        return None;
    }
    let curve = arr.get(1)?.as_array()?;
    let base = match curve.first()?.as_str()? {
        "exponential" => curve.get(1)?.as_f64()?,
        _ => 1.0,
    };
    let mut stops = Vec::new();
    for pair in arr[3..].chunks_exact(2) {
        let z = pair[0].as_f64()?;
        let m = parse_matcher(&pair[1], &|v: &Value| v.as_f64())?;
        stops.push((z, m));
    }
    Some(WidthCurve { base, stops })
}

/// The filters this style uses: geometry-type equality and kind (in)equality.
#[derive(Debug, Clone)]
enum Filter {
    Any,
    GeomIs(mvt::GeomType),
    KindIs(String, bool), // (value, negated)
}

impl Filter {
    fn pass(&self, f: &mvt::Feature) -> bool {
        match self {
            Filter::Any => true,
            Filter::GeomIs(t) => f.geom_type == *t,
            Filter::KindIs(v, negated) => {
                let got = f.tags.get("kind").and_then(|k| k.as_str()).unwrap_or("");
                (got == v) != *negated
            }
        }
    }
}

fn parse_filter(v: Option<&Value>) -> Filter {
    let Some(arr) = v.and_then(Value::as_array) else { return Filter::Any };
    let op = arr.first().and_then(Value::as_str).unwrap_or("");
    let lhs = arr.get(1).and_then(Value::as_array);
    let rhs = arr.get(2).and_then(Value::as_str);
    match (op, lhs, rhs) {
        ("==", Some(l), Some(r)) if l.first().and_then(Value::as_str) == Some("geometry-type") => {
            match r {
                "Polygon" => Filter::GeomIs(mvt::GeomType::Polygon),
                "LineString" => Filter::GeomIs(mvt::GeomType::Line),
                "Point" => Filter::GeomIs(mvt::GeomType::Point),
                _ => Filter::Any,
            }
        }
        (op @ ("==" | "!="), Some(l), Some(r))
            if l.first().and_then(Value::as_str) == Some("get")
                && l.get(1).and_then(Value::as_str) == Some("kind") =>
        {
            Filter::KindIs(r.to_string(), op == "!=")
        }
        _ => Filter::Any,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DrawKind {
    Fill,
    Line,
}

pub struct DrawLayer {
    pub source_layer: String,
    pub kind: DrawKind,
    filter: Filter,
    color: Matcher<Rgba>,
    width: Option<WidthCurve>,
}

impl DrawLayer {
    pub fn matches(&self, f: &mvt::Feature) -> bool {
        self.filter.pass(f)
    }
    pub fn color(&self, f: &mvt::Feature) -> Rgba {
        self.color.eval(&f.tags)
    }
    pub fn width(&self, zoom: f64, f: &mvt::Feature) -> f64 {
        self.width.as_ref().map(|w| w.eval(zoom, &f.tags)).unwrap_or(1.0)
    }
}

pub struct MapStyle {
    pub background: Rgba,
    pub layers: Vec<DrawLayer>,
}

/// Digest a style document into draw layers, in document (paint) order.
/// Unsupported layers (symbols/labels) are skipped, as are paints the subset
/// cannot evaluate — the map still draws, just without that layer.
pub fn parse(style: &Value) -> MapStyle {
    let mut background = Rgba(0xf6, 0xf3, 0xee, 0xff);
    let mut layers = Vec::new();
    let leaf_color = |v: &Value| v.as_str().and_then(parse_color);
    for l in style.get("layers").and_then(Value::as_array).into_iter().flatten() {
        let ty = l.get("type").and_then(Value::as_str).unwrap_or("");
        let paint = l.get("paint").cloned().unwrap_or(Value::Null);
        match ty {
            "background" => {
                if let Some(c) =
                    paint.get("background-color").and_then(Value::as_str).and_then(parse_color)
                {
                    background = c;
                }
            }
            "fill" | "line" => {
                let source_layer =
                    l.get("source-layer").and_then(Value::as_str).unwrap_or("").to_string();
                let color_v = paint
                    .get(if ty == "fill" { "fill-color" } else { "line-color" })
                    .cloned()
                    .unwrap_or(Value::Null);
                let Some(color) = parse_matcher(&color_v, &leaf_color) else { continue };
                let width = paint.get("line-width").and_then(parse_width);
                layers.push(DrawLayer {
                    source_layer,
                    kind: if ty == "fill" { DrawKind::Fill } else { DrawKind::Line },
                    filter: parse_filter(l.get("filter")),
                    color,
                    width,
                });
            }
            _ => {}
        }
    }
    MapStyle { background, layers }
}
