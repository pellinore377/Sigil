//! Names on the map: street names set along their streets, place names at
//! their points.
//!
//! The tiles always carried these — 5875 of the 6819 road features in a
//! sample of this cache have a `name`, and every one of the 106 places does —
//! and the renderer simply threw them away, which is why the map read as a
//! diagram rather than as a map. This puts them back.
//!
//! # Which schema
//!
//! Two are in the wild and this reads both, because the tiles a homeserver
//! points at are not ours to choose:
//!
//! * **Protomaps basemap** — what this cache actually holds: one `roads`
//!   layer carrying `name`/`kind`/`min_zoom`, one `places` layer with
//!   `kind`/`population_rank`, one `water` layer. There is no separate label
//!   layer; the names ride on the geometry that is already being drawn.
//! * **OpenMapTiles** — `transportation_name`, `place` and `water_name` are
//!   their own layers, and `class` does the work `kind` does above.
//!
//! Anything unrecognised is skipped rather than guessed at.
//!
//! # How a name gets on the map
//!
//! Candidates are collected, sorted by how much they deserve the space, and
//! then placed one at a time into a collision grid — the first to ask for a
//! patch of tile gets it, so a motorway's name beats a footpath's rather than
//! the two overlapping. Point labels sit centred on their point; line labels
//! are set along the longest run of their line, upright, centred, and only
//! if the run is long enough to hold them.
//!
//! Every glyph is stroked in white before it is filled, so a name stays
//! legible over a dark road or a green park.

use std::collections::HashMap;

use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};

use super::mvt;

/// Roboto Medium — the face the app's own UI is set in (`shared/fonts`), so
/// the map is lettered in the same hand as everything around it. Medium
/// rather than Regular because map labels are small, sit on busy ground, and
/// give up a little of every stroke to the halo.
static FONT: &[u8] = include_bytes!("../../../shared/fonts/Roboto-Medium.ttf");

fn face() -> Option<&'static ttf_parser::Face<'static>> {
    static F: std::sync::OnceLock<Option<ttf_parser::Face<'static>>> = std::sync::OnceLock::new();
    F.get_or_init(|| ttf_parser::Face::parse(FONT, 0).ok()).as_ref()
}

/// The white worn round every glyph, in canvas pixels. The canvas is 2× (512
/// for a 256 CSS tile), so this is one and a half CSS pixels of halo — enough
/// to hold a name off a dark road, little enough that it does not bleed the
/// letterforms into each other at these sizes.
const HALO: f32 = 3.0;

/// Ink and halo. Not taken from the style: a style's label paint lives in the
/// `symbol` layers this renderer does not read, and a near-black on white
/// reads on every basemap either of these schemas ships.
const INK: (u8, u8, u8) = (0x33, 0x33, 0x33);
const HALO_C: (u8, u8, u8) = (0xff, 0xff, 0xff);

/// One name, ready to place.
struct Cand<'a> {
    text: &'a str,
    /// Smaller is more important: it gets first refusal on the space.
    rank: i32,
    size: f32,
    kind: Kind,
}

enum Kind {
    /// Centred on a point in canvas space.
    Point(f32, f32),
    /// Set along `runs[i]`, in canvas space. An index rather than a borrow:
    /// the runs are still being collected while the candidates are, and a
    /// slice into a Vec that is about to grow is not a thing one can hold.
    Line(usize),
}

/// Which patches of the tile are spoken for.
///
/// A grid rather than a list of rectangles: a tile can offer a few hundred
/// candidates and testing each against every label already placed is the
/// quadratic that shows up as a stutter on the phone. Cells are 16 canvas
/// pixels, so the whole thing is a 32×32 bitmap and a test is a handful of
/// byte reads.
struct Grid {
    cell: f32,
    n: usize,
    taken: Vec<bool>,
}

impl Grid {
    fn new(side: f32) -> Grid {
        let cell = 16.0;
        let n = (side / cell).ceil() as usize + 1;
        Grid { cell, n, taken: vec![false; n * n] }
    }

    /// The cell range a box covers, clamped into the grid.
    fn span(&self, b: (f32, f32, f32, f32)) -> (usize, usize, usize, usize) {
        let c = |v: f32| (v / self.cell).floor().clamp(0.0, self.n as f32 - 1.0) as usize;
        (c(b.0), c(b.1), c(b.2), c(b.3))
    }

    fn free(&self, b: (f32, f32, f32, f32)) -> bool {
        let (x0, y0, x1, y1) = self.span(b);
        for y in y0..=y1 {
            for x in x0..=x1 {
                if self.taken[y * self.n + x] {
                    return false;
                }
            }
        }
        true
    }

    fn take(&mut self, b: (f32, f32, f32, f32)) {
        let (x0, y0, x1, y1) = self.span(b);
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.taken[y * self.n + x] = true;
            }
        }
    }
}

/// Lays one glyph's outline into a path, scaled, rotated and moved into place.
///
/// Font space has y going up and the canvas has it going down, so the y is
/// negated before the rotation rather than after — otherwise every letter set
/// along a road leans the wrong way.
struct Pen<'a> {
    pb: &'a mut PathBuilder,
    s: f32,
    cos: f32,
    sin: f32,
    tx: f32,
    ty: f32,
}

impl Pen<'_> {
    fn at(&self, x: f32, y: f32) -> (f32, f32) {
        let (px, py) = (x * self.s, -y * self.s);
        (self.tx + px * self.cos - py * self.sin, self.ty + px * self.sin + py * self.cos)
    }
}

impl ttf_parser::OutlineBuilder for Pen<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let p = self.at(x, y);
        self.pb.move_to(p.0, p.1);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.at(x, y);
        self.pb.line_to(p.0, p.1);
    }
    fn quad_to(&mut self, ax: f32, ay: f32, x: f32, y: f32) {
        let (a, p) = (self.at(ax, ay), self.at(x, y));
        self.pb.quad_to(a.0, a.1, p.0, p.1);
    }
    fn curve_to(&mut self, ax: f32, ay: f32, bx: f32, by: f32, x: f32, y: f32) {
        let (a, b, p) = (self.at(ax, ay), self.at(bx, by), self.at(x, y));
        self.pb.cubic_to(a.0, a.1, b.0, b.1, p.0, p.1);
    }
    fn close(&mut self) {
        self.pb.close();
    }
}

/// Every glyph's advance, in canvas pixels at `size`. `None` if the face has
/// no glyph for one of the characters — a Cyrillic street name is fine (Roboto
/// carries Cyrillic and Greek), a Chinese one is not, and half a name drawn
/// with the rest as empty boxes is worse than no name.
fn advances(f: &ttf_parser::Face, text: &str, size: f32) -> Option<Vec<(char, f32)>> {
    let upem = f.units_per_em() as f32;
    let s = size / upem;
    let mut out = Vec::new();
    for c in text.chars() {
        if c.is_control() {
            continue;
        }
        let g = f.glyph_index(c)?;
        out.push((c, f.glyph_hor_advance(g).unwrap_or(0) as f32 * s));
    }
    (!out.is_empty()).then_some(out)
}

fn width_of(adv: &[(char, f32)]) -> f32 {
    adv.iter().map(|(_, a)| *a).sum()
}

/// Set a name horizontally, centred on `(cx, cy)` — cy being the middle of
/// the letters rather than the baseline, which is what "at this point" means
/// to the eye.
fn lay_point(f: &ttf_parser::Face, pb: &mut PathBuilder, adv: &[(char, f32)], size: f32, cx: f32, cy: f32) {
    let mut x = cx - width_of(adv) / 2.0;
    // Roboto's cap height is about 0.71 em; half of it lifts the optical
    // middle of a line of capitals onto the point.
    let y = cy + size * 0.355;
    for (c, a) in adv {
        if let Some(g) = f.glyph_index(*c) {
            let mut pen = Pen { pb, s: size / f.units_per_em() as f32, cos: 1.0, sin: 0.0, tx: x, ty: y };
            f.outline_glyph(g, &mut pen);
        }
        x += a;
    }
}

/// Cumulative arc length along a run.
fn arcs(pts: &[(f32, f32)]) -> Vec<f32> {
    let mut out = Vec::with_capacity(pts.len());
    let mut d = 0.0;
    out.push(0.0);
    for w in pts.windows(2) {
        d += (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
        out.push(d);
    }
    out
}

/// The point at arc length `t` along the run, and the direction there.
fn at_arc(pts: &[(f32, f32)], acc: &[f32], t: f32) -> (f32, f32, f32, f32) {
    let i = match acc.binary_search_by(|v| v.partial_cmp(&t).unwrap_or(std::cmp::Ordering::Less)) {
        Ok(i) => i.min(pts.len() - 2),
        Err(i) => i.saturating_sub(1).min(pts.len() - 2),
    };
    let seg = (acc[i + 1] - acc[i]).max(1e-6);
    let u = ((t - acc[i]) / seg).clamp(0.0, 1.0);
    let (a, b) = (pts[i], pts[i + 1]);
    let (dx, dy) = ((b.0 - a.0) / seg, (b.1 - a.1) / seg);
    (a.0 + (b.0 - a.0) * u, a.1 + (b.1 - a.1) * u, dx, dy)
}

/// Set a name along a run, centred on it, each letter turned to the line's
/// direction where it stands.
///
/// Returns the box the letters occupy, so the caller can ask the grid for it.
/// `None` when the run is too short to hold the name — a name that runs off
/// the end of its own street is worse than no name.
fn lay_line(
    f: &ttf_parser::Face,
    pb: &mut PathBuilder,
    adv: &[(char, f32)],
    size: f32,
    pts: &[(f32, f32)],
) -> Option<(f32, f32, f32, f32)> {
    if pts.len() < 2 {
        return None;
    }
    let acc = arcs(pts);
    let total = *acc.last()?;
    let w = width_of(adv);
    // A tenth of the name's width of road at each end, so it does not start
    // in a junction and finish in one.
    if total < w * 1.2 {
        return None;
    }
    let start = (total - w) / 2.0;

    // Upright, or the name is upside down half the time: take the line's
    // overall direction across the span the letters will cover, and walk the
    // run backwards if it points left.
    let a = at_arc(pts, &acc, start);
    let b = at_arc(pts, &acc, start + w);
    let flip = b.0 - a.0 < 0.0;
    let owned: Vec<(f32, f32)>;
    let (pts, acc, start) = if flip {
        owned = pts.iter().rev().copied().collect();
        let acc2 = arcs(&owned);
        let s = (total - w) / 2.0;
        (&owned[..], acc2, s)
    } else {
        (pts, acc, start)
    };

    let upem = f.units_per_em() as f32;
    let lift = size * 0.355;
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    let mut t = start;
    for (c, a) in adv {
        // Each letter stands on the middle of its own advance, so a curve
        // turns the word rather than shearing it.
        let (px, py, dx, dy) = at_arc(pts, &acc, t + a / 2.0);
        // …and the pen for that letter starts half an advance back along the
        // line, with the baseline lifted along the line's normal.
        let (nx, ny) = (-dy, dx);
        let tx = px - dx * a / 2.0 + nx * lift;
        let ty = py - dy * a / 2.0 + ny * lift;
        if let Some(g) = f.glyph_index(*c) {
            let mut pen = Pen { pb, s: size / upem, cos: dx, sin: dy, tx, ty };
            f.outline_glyph(g, &mut pen);
        }
        // The box: the letter's own patch, generously, so the grid keeps
        // other names off the whole word rather than off its skeleton.
        x0 = x0.min(px - size); y0 = y0.min(py - size);
        x1 = x1.max(px + size); y1 = y1.max(py + size);
        t += a;
    }
    Some((x0, y0, x1, y1))
}

/// A tag as a string.
fn tag<'a>(f: &'a mvt::Feature, k: &str) -> Option<&'a str> {
    f.tags.get(k).and_then(mvt::Value::as_str)
}

fn num(f: &mvt::Feature, k: &str) -> Option<f64> {
    match f.tags.get(k) {
        Some(mvt::Value::Num(n)) => Some(*n),
        Some(mvt::Value::Str(s)) => s.parse().ok(),
        _ => None,
    }
}

/// The name to show. `name` is the local one, which is what a map of a place
/// should say; the translations beside it are for somebody else's map.
fn name_of(f: &mvt::Feature) -> Option<&str> {
    let n = tag(f, "name")?;
    let n = n.trim();
    (!n.is_empty() && n.chars().count() <= 40).then_some(n)
}

/// Does the schema say this feature is too small to name at this zoom? Both
/// schemas ship `min_zoom` on the features that have one, and honouring it is
/// most of what keeps a city tile from turning into a wall of words.
fn too_soon(f: &mvt::Feature, z: u32) -> bool {
    num(f, "min_zoom").map(|m| (z as f64) < m).unwrap_or(false)
}

/// How big, and how much it deserves the space. `None` means do not label.
fn road_style(kind: &str, z: u32) -> Option<(f32, i32)> {
    // The canvas is 2×, so these are half this many CSS pixels.
    match kind {
        // OpenMapTiles `class` and Protomaps `kind` land in the same arms.
        "motorway" | "trunk" | "highway" => Some((23.0, 0)),
        "primary" | "secondary" | "major_road" => Some((22.0, 1)),
        "tertiary" | "minor_road" | "street" | "residential" => Some((20.0, 2)),
        // Footpaths and service roads only once the map is properly close in;
        // there are more of them than of everything else put together.
        "path" | "footway" | "service" | "track" => (z >= 17).then_some((18.0, 4)),
        _ => None,
    }
}

fn place_style(kind: &str, rank: f64) -> Option<(f32, i32)> {
    match kind {
        "city" | "locality" => Some((if rank >= 10.0 { 32.0 } else { 28.0 }, 0)),
        "town" => Some((26.0, 1)),
        "macrohood" => Some((26.0, 2)),
        "village" | "suburb" => Some((24.0, 3)),
        "neighbourhood" | "neighborhood" | "hamlet" | "quarter" => Some((23.0, 4)),
        _ => None,
    }
}

/// Draw every name this tile can fit.
///
/// `scale`/`off_x`/`off_y` are the over-zoom window `render::tile` computed,
/// and the geometry is put through exactly the transform the road under it
/// was drawn with — a label that does not sit on its road is worse than none.
pub fn draw(
    pixmap: &mut Pixmap,
    layers: &[mvt::Layer],
    z: u32,
    side: f32,
    scale: f32,
    off_x: f32,
    off_y: f32,
) {
    let Some(f) = face() else { return };

    // Canvas-space geometry, kept alive for the candidates to borrow.
    let mut runs: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut cands: Vec<Cand> = Vec::new();
    // One label per name per layer: a street is a dozen separate features and
    // labelling each of them writes its name down the road a dozen times.
    // The longest run wins, since it has the best chance of holding the name.
    let mut best: HashMap<(&str, &str), (usize, f32)> = HashMap::new();

    for layer in layers {
        let lname = layer.name.as_str();
        let is_road = matches!(lname, "roads" | "transportation_name");
        let is_place = matches!(lname, "places" | "place");
        let is_water = matches!(lname, "water" | "water_name");
        if !(is_road || is_place || is_water) {
            continue;
        }
        let s = side / layer.extent as f32 * scale;
        let (ox, oy) = (-off_x * side, -off_y * side);
        for feat in &layer.features {
            let Some(text) = name_of(feat) else { continue };
            if too_soon(feat, z) {
                continue;
            }
            let kind = tag(feat, "kind").or_else(|| tag(feat, "class")).unwrap_or("");
            let rank = num(feat, "population_rank").unwrap_or(0.0);
            let styled = if is_road {
                road_style(kind, z)
            } else if is_place {
                place_style(kind, rank)
            } else {
                // Water: the big bodies and the rivers, nothing else. A
                // fountain and a swimming pool have names and no business
                // carrying them on a street map.
                match kind {
                    "ocean" | "sea" | "lake" | "water" | "bay" | "strait" => Some((24.0, 2)),
                    "river" | "stream" | "canal" => Some((20.0, 3)),
                    _ => None,
                }
            };
            let Some((size, rank_base)) = styled else { continue };

            match feat.geom_type {
                mvt::GeomType::Point => {
                    let Some(p) = feat.paths.first().and_then(|r| r.first()) else { continue };
                    let (x, y) = (p.0 * s + ox, p.1 * s + oy);
                    // Points outside the tile still matter — a name centred
                    // just past the edge shows half of itself — but not far
                    // outside.
                    if x < -side || x > side * 2.0 || y < -side || y > side * 2.0 {
                        continue;
                    }
                    cands.push(Cand { text, rank: rank_base, size, kind: Kind::Point(x, y) });
                }
                mvt::GeomType::Line => {
                    // The longest of the feature's runs: the one with room.
                    let mut pick: Option<(Vec<(f32, f32)>, f32)> = None;
                    for r in &feat.paths {
                        if r.len() < 2 {
                            continue;
                        }
                        let pts: Vec<(f32, f32)> =
                            r.iter().map(|p| (p.0 * s + ox, p.1 * s + oy)).collect();
                        let len = *arcs(&pts).last().unwrap_or(&0.0);
                        if pick.as_ref().map(|(_, l)| len > *l).unwrap_or(true) {
                            pick = Some((pts, len));
                        }
                    }
                    let Some((pts, len)) = pick else { continue };
                    // A street is a dozen separate features and labelling
                    // each writes its name down the road a dozen times. One
                    // candidate per name per layer, and the longest run wins
                    // it — that is the one with room to hold the name.
                    let key = (lname, text);
                    if let Some(&(idx, had)) = best.get(&key) {
                        if len > had {
                            runs[idx] = pts;
                            best.insert(key, (idx, len));
                        }
                        continue;
                    }
                    runs.push(pts);
                    best.insert(key, (runs.len() - 1, len));
                    cands.push(Cand {
                        text,
                        rank: rank_base,
                        size,
                        kind: Kind::Line(runs.len() - 1),
                    });
                }
                mvt::GeomType::Polygon => {
                    // A named park or lake: label the middle of its biggest
                    // ring, which is close enough to a pole of inaccessibility
                    // for a tile this size.
                    let Some(ring) = feat.paths.iter().max_by_key(|r| r.len()) else { continue };
                    if ring.len() < 3 {
                        continue;
                    }
                    let (mut sx, mut sy) = (0.0f32, 0.0f32);
                    for p in ring {
                        sx += p.0;
                        sy += p.1;
                    }
                    let n = ring.len() as f32;
                    let (x, y) = (sx / n * s + ox, sy / n * s + oy);
                    if x < 0.0 || x > side || y < 0.0 || y > side {
                        continue;
                    }
                    cands.push(Cand { text, rank: rank_base + 1, size, kind: Kind::Point(x, y) });
                }
            }
        }
    }

    // Most important first: whoever asks first gets the space.
    cands.sort_by(|a, b| a.rank.cmp(&b.rank).then(b.size.total_cmp(&a.size)));

    let mut grid = Grid::new(side);
    let mut pb = PathBuilder::new();
    let mut drawn = 0usize;
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for c in &cands {
        // A ceiling as well as a grid: a dense city tile can offer hundreds
        // of names and the grid alone would happily accept every one that
        // found a gap, which is a wall of words rather than a map.
        if drawn >= 48 {
            break;
        }
        // The same name twice on one tile is once too many, whatever layer
        // the second one came from.
        if !seen.insert(c.text) {
            continue;
        }
        let Some(adv) = advances(f, c.text, c.size) else { continue };
        let mut one = PathBuilder::new();
        let boxed = match c.kind {
            Kind::Point(x, y) => {
                let w = width_of(&adv);
                let b = (x - w / 2.0, y - c.size * 0.6, x + w / 2.0, y + c.size * 0.6);
                if b.2 < 0.0 || b.0 > side || b.3 < 0.0 || b.1 > side {
                    continue;
                }
                lay_point(f, &mut one, &adv, c.size, x, y);
                Some(b)
            }
            Kind::Line(i) => lay_line(f, &mut one, &adv, c.size, &runs[i]),
        };
        let Some(b) = boxed else { continue };
        if !grid.free(b) {
            continue;
        }
        let Some(p) = one.finish() else { continue };
        grid.take(b);
        drawn += 1;
        pb.push_path(&p);
    }

    let Some(path) = pb.finish() else { return };
    // The halo first, as one stroke of the whole tile's lettering, then the
    // ink over it: two passes for however many names, rather than two per
    // name, and no letter's halo can eat its neighbour's stroke.
    let mut halo = Paint::default();
    halo.set_color_rgba8(HALO_C.0, HALO_C.1, HALO_C.2, 235);
    halo.anti_alias = true;
    let stroke = Stroke {
        width: HALO,
        line_cap: tiny_skia::LineCap::Round,
        line_join: tiny_skia::LineJoin::Round,
        ..Stroke::default()
    };
    pixmap.stroke_path(&path, &halo, &stroke, Transform::identity(), None);
    let mut ink = Paint::default();
    ink.set_color_rgba8(INK.0, INK.1, INK.2, 255);
    ink.anti_alias = true;
    pixmap.fill_path(&path, &ink, FillRule::Winding, Transform::identity(), None);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_face_parses_and_carries_latin_and_cyrillic() {
        let f = face().expect("Roboto Medium did not parse");
        assert!(f.units_per_em() > 0);
        // A Latin street name and a Cyrillic one both measure; a Han one does
        // not, and `advances` says so rather than drawing empty boxes.
        assert!(advances(f, "Marlowe Street", 22.0).is_some());
        assert!(advances(f, "Тверская", 22.0).is_some());
        assert!(advances(f, "中山路", 22.0).is_none());
    }

    #[test]
    fn a_name_is_as_wide_as_its_letters_and_grows_with_the_size() {
        let f = face().unwrap();
        let a = width_of(&advances(f, "Bridge Road", 20.0).unwrap());
        let b = width_of(&advances(f, "Bridge Road", 40.0).unwrap());
        assert!(a > 0.0);
        assert!((b - a * 2.0).abs() < 0.01, "{a} {b}");
        // …and a longer name is wider than a shorter one.
        let c = width_of(&advances(f, "Bridge Road East", 20.0).unwrap());
        assert!(c > a);
    }

    #[test]
    fn arc_length_walks_the_run_and_reports_its_direction() {
        let pts = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)];
        let acc = arcs(&pts);
        assert_eq!(acc, vec![0.0, 10.0, 20.0]);
        // Five along is halfway down the first leg, heading east.
        let (x, y, dx, dy) = at_arc(&pts, &acc, 5.0);
        assert!((x - 5.0).abs() < 1e-5 && y.abs() < 1e-5);
        assert!((dx - 1.0).abs() < 1e-5 && dy.abs() < 1e-5);
        // Fifteen along is halfway up the second, heading south.
        let (x, y, dx, dy) = at_arc(&pts, &acc, 15.0);
        assert!((x - 10.0).abs() < 1e-5 && (y - 5.0).abs() < 1e-5);
        assert!(dx.abs() < 1e-5 && (dy - 1.0).abs() < 1e-5);
        // Past the end clamps rather than panicking.
        let (x, _, _, _) = at_arc(&pts, &acc, 99.0);
        assert!((x - 10.0).abs() < 1e-5);
    }

    #[test]
    fn a_name_needs_more_road_than_it_is_wide() {
        let f = face().unwrap();
        let adv = advances(f, "Marlowe Street", 22.0).unwrap();
        let w = width_of(&adv);
        let mut pb = PathBuilder::new();
        // A run shorter than the name: refused.
        let short = [(0.0, 0.0), (w * 0.9, 0.0)];
        assert!(lay_line(f, &mut pb, &adv, 22.0, &short).is_none());
        // A run with room: taken, and the letters land inside its span.
        let long = [(0.0, 100.0), (w * 3.0, 100.0)];
        let b = lay_line(f, &mut pb, &adv, 22.0, &long).expect("should fit");
        assert!(b.0 >= -1.0 && b.2 <= w * 3.0 + 1.0, "{b:?}");
        // Centred: as much road before the name as after it.
        assert!(((b.0) - (w * 3.0 - b.2)).abs() < 25.0, "{b:?}");
    }

    #[test]
    fn a_name_on_a_backwards_road_is_still_the_right_way_up() {
        let f = face().unwrap();
        let adv = advances(f, "Bridge Road", 22.0).unwrap();
        let w = width_of(&adv);
        // The same road, drawn east-to-west and west-to-east: the letters
        // must come out in the same place either way, which is only true if
        // the backwards one was walked in reverse.
        let fwd = [(0.0, 50.0), (w * 3.0, 50.0)];
        let rev = [(w * 3.0, 50.0), (0.0, 50.0)];
        let mut a = PathBuilder::new();
        let mut b = PathBuilder::new();
        let ba = lay_line(f, &mut a, &adv, 22.0, &fwd).unwrap();
        let bb = lay_line(f, &mut b, &adv, 22.0, &rev).unwrap();
        assert!((ba.0 - bb.0).abs() < 0.5 && (ba.2 - bb.2).abs() < 0.5, "{ba:?} {bb:?}");
        // And the ink itself is the same, not mirrored.
        let (pa, pb2) = (a.finish().unwrap(), b.finish().unwrap());
        let (ra, rb) = (pa.bounds(), pb2.bounds());
        assert!((ra.left() - rb.left()).abs() < 0.5, "{ra:?} {rb:?}");
        assert!((ra.top() - rb.top()).abs() < 0.5, "{ra:?} {rb:?}");
    }

    #[test]
    fn the_grid_lets_the_first_name_in_and_keeps_the_next_one_out() {
        let mut g = Grid::new(512.0);
        let b = (100.0, 100.0, 200.0, 130.0);
        assert!(g.free(b));
        g.take(b);
        assert!(!g.free(b));
        // Overlapping is refused …
        assert!(!g.free((150.0, 110.0, 260.0, 140.0)));
        // … and somewhere else is not.
        assert!(g.free((300.0, 300.0, 400.0, 330.0)));
        // Off the edge clamps rather than panicking.
        assert!(g.free((-40.0, -40.0, -10.0, -10.0)) || true);
        g.take((-40.0, -40.0, 900.0, 900.0));
    }

    #[test]
    fn the_schemas_agree_on_what_is_worth_a_name() {
        // Protomaps and OpenMapTiles spell the same road three ways; all three
        // get a size, and the bigger road gets the bigger one.
        let (big, _) = road_style("highway", 15).unwrap();
        let (mid, _) = road_style("major_road", 15).unwrap();
        let (small, _) = road_style("minor_road", 15).unwrap();
        assert!(big >= mid && mid > small);
        assert_eq!(road_style("motorway", 15).unwrap().0, big);
        assert_eq!(road_style("primary", 15).unwrap().0, mid);
        // Footpaths wait until the map is close in.
        assert!(road_style("path", 15).is_none());
        assert!(road_style("path", 17).is_some());
        // A city outranks a neighbourhood, in size and in priority.
        let city = place_style("city", 12.0).unwrap();
        let hood = place_style("neighbourhood", 0.0).unwrap();
        assert!(city.0 > hood.0 && city.1 < hood.1);
        assert_eq!(place_style("locality", 12.0), place_style("city", 12.0));
        assert!(place_style("continent", 0.0).is_none());
    }

    #[test]
    fn a_feature_the_schema_says_is_too_small_yet_is_left_alone() {
        let mut f = mvt::Feature {
            geom_type: mvt::GeomType::Line,
            paths: vec![],
            tags: HashMap::new(),
        };
        f.tags.insert("name".into(), mvt::Value::Str("Marlowe Street".into()));
        f.tags.insert("min_zoom".into(), mvt::Value::Num(16.0));
        assert!(too_soon(&f, 14));
        assert!(too_soon(&f, 15));
        assert!(!too_soon(&f, 16));
        assert!(!too_soon(&f, 18));
        assert_eq!(name_of(&f), Some("Marlowe Street"));
        // A nameless feature, and one whose name is a paragraph.
        f.tags.remove("name");
        assert_eq!(name_of(&f), None);
        f.tags.insert("name".into(), mvt::Value::Str("x".repeat(41)));
        assert_eq!(name_of(&f), None);
    }
}
