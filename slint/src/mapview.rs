//! The location page's map, as a grid rather than a photograph.
//!
//! The page used to show one composite the engine rendered around the point,
//! which is why it could not be dragged: there was nothing to drag, and a
//! zoom step meant asking for a whole new picture. This keeps the view's own
//! centre and zoom and places single rendered tiles (`map.tile`) under it, so
//! a drag moves the centre and the grid follows in the same frame.
//!
//! Units. Tiles are rendered 512 px square and drawn at 256 logical pixels,
//! the usual two-times arrangement, so all the arithmetic here is in those
//! 256-pixel "CSS" units at the current zoom: the world is `256 · 2^z` across.

use std::collections::{HashMap, HashSet};

/// A tile's side where it is drawn, in logical pixels.
pub const TILE: f64 = 256.0;
/// The zoom range the engine's renderer accepts (composite.rs clamps to it).
pub const Z_MIN: u32 = 3;
pub const Z_MAX: u32 = 19;

/// How far the drawn zoom may drift from the level the tiles come from before
/// the level is swapped, in whole levels.
///
/// Half a level is the "nearest level" rule, and it is what made a pinch feel
/// finicky: a pinch that hovers around the halfway point swaps the entire grid
/// back and forth every frame, and each swap is a set of tiles that has not
/// been fetched yet, so the map blinks. Three quarters of a level keeps a
/// level until the gesture has clearly left it — the classic hysteresis — at
/// the price of drawing tiles at up to 2^0.75 ≈ 1.68 of their size, which is
/// soft but never blank.
const LEVEL_KEEP: f64 = 0.75;

/// The whole level a fractional zoom belongs to.
fn level_of(zf: f64) -> u32 {
    (zf.round() as i64).clamp(Z_MIN as i64, Z_MAX as i64) as u32
}

/// How far across the world is at this zoom, in drawn pixels.
fn world(z: u32) -> f64 {
    TILE * f64::from(1u32 << z)
}

/// Web Mercator: a position to a place on the world sheet at `z`.
pub fn px_of(lat: f64, lon: f64, z: u32) -> (f64, f64) {
    let w = world(z);
    let x = (lon + 180.0) / 360.0 * w;
    let s = lat.to_radians().sin().clamp(-0.9999, 0.9999);
    let y = (0.5 - ((1.0 + s) / (1.0 - s)).ln() / (4.0 * std::f64::consts::PI)) * w;
    (x, y)
}

/// Where the view stood when two fingers went down, so every step of a pinch
/// is measured from there rather than accumulating one step's rounding into
/// the next.
#[derive(Clone, Copy)]
struct Grip {
    z: u32,
    cx: f64,
    cy: f64,
    scale: f64,
    /// The level the grid is being drawn from, carried through the gesture so
    /// the hysteresis has a memory: without it every step would re-decide from
    /// the grip and a finger sitting on the boundary would still thrash.
    lvl: u32,
    /// Where the fingers' midpoint was when the gesture began, in the map
    /// area. The spot of world under it is the one the pinch is about, and it
    /// is kept under the midpoint wherever that travels — so two fingers drag
    /// the map as well as magnify it, the way every map does. It is taken from
    /// the first `pinch_to` rather than from `pinch_begin`, which the runtime
    /// reports without a position.
    anchor: Option<(f64, f64)>,
}

/// The page's view of the map: where it is looking and what it has drawn.
pub struct MapView {
    /// The level the tiles are fetched from: always a whole one, since that
    /// is all the renderer serves.
    pub z: u32,
    /// How much those tiles are magnified as they are drawn. A pinch moves
    /// this continuously, and it is *left* wherever the fingers put it: the
    /// view's real zoom is `z + log2(scale)` and nothing rounds that off, so
    /// letting go of a pinch does not move the map. What settles on the lift
    /// is `z` — the level the tiles are fetched from — which becomes the whole
    /// level nearest the zoom without the zoom itself changing. Between
    /// gestures `scale` is therefore within [1/√2, √2] of 1, and during one
    /// within [2^-0.75, 2^0.75] (`LEVEL_KEEP`).
    pub scale: f64,
    /// The centre of the view on the world sheet at `z`, in *unmagnified*
    /// drawn pixels — so panning and clamping stay in one space whatever the
    /// magnification.
    pub cx: f64,
    pub cy: f64,
    /// The shared point the page was opened on.
    pub lat: f64,
    pub lon: f64,
    /// The map area, in logical pixels.
    pub w: f64,
    pub h: f64,
    /// Tiles already rendered, by zoom and tile position.
    pub have: HashMap<(u32, i64, i64), slint::Image>,
    /// Tiles asked for and not yet answered, so nothing is asked for twice.
    pub asked: HashSet<(u32, i64, i64)>,
    /// The page is open: a reply for another point is stale.
    pub epoch: u64,
    /// A pinch in progress.
    grip: Option<Grip>,
}

impl Default for MapView {
    fn default() -> Self {
        MapView {
            z: 15,
            scale: 1.0,
            cx: 0.0,
            cy: 0.0,
            lat: 0.0,
            lon: 0.0,
            w: 0.0,
            h: 0.0,
            have: HashMap::new(),
            asked: HashSet::new(),
            epoch: 0,
            grip: None,
        }
    }
}

impl MapView {
    /// Open on a point: centre there, forget the last page's grid.
    pub fn open(&mut self, lat: f64, lon: f64) {
        self.epoch = self.epoch.wrapping_add(1);
        self.z = 15;
        self.scale = 1.0;
        self.grip = None;
        self.lat = lat;
        self.lon = lon;
        let (x, y) = px_of(lat, lon, self.z);
        self.cx = x;
        self.cy = y;
        self.asked.clear();
        // The rendered tiles are worth keeping across points — they are the
        // same world — but a run of pages would grow without a bound.
        if self.have.len() > 400 {
            self.have.clear();
        }
    }

    pub fn resize(&mut self, w: f64, h: f64) {
        self.w = w.max(0.0);
        self.h = h.max(0.0);
        self.clamp();
    }

    /// Put the view back where the page opened it: the shared point, at the
    /// level the page opens on. What the recentre disc does (MapPage.qml:351-366).
    pub fn recentre(&mut self) {
        self.z = 15;
        self.scale = 1.0;
        self.grip = None;
        let (x, y) = px_of(self.lat, self.lon, self.z);
        self.cx = x;
        self.cy = y;
        self.clamp();
    }

    /// Dragged by this much: the map goes with the finger, so the centre goes
    /// against it. The drag is in drawn pixels, the centre in unmagnified
    /// ones, so it divides through the magnification.
    ///
    /// A drag reported while two fingers are down is dropped: the pinch is
    /// already moving the map by its own midpoint, and letting the single
    /// pointer's drag through as well moves it twice, which is most of what
    /// made the gesture feel finicky. The page disarms its drag area when the
    /// pinch starts too; this is the belt to that pair of braces, and it is
    /// the half that can be tested.
    pub fn pan(&mut self, dx: f64, dy: f64) {
        if self.pinching() {
            return;
        }
        self.cx -= dx / self.scale;
        self.cy -= dy / self.scale;
        self.clamp();
    }

    /// The zoom the view is actually showing, whole level and magnification
    /// together.
    pub fn zoom_f(&self) -> f64 {
        self.z as f64 + self.scale.log2()
    }

    /// A step in or out, holding the point at `(ax, ay)` in the view still —
    /// what a double tap on a map does. Pass the view's centre for the chips.
    /// It steps off the whole level nearest the current zoom, so a step taken
    /// mid-pinch lands somewhere sensible.
    pub fn zoom(&mut self, step: i32, ax: f64, ay: f64) {
        let z = (self.zoom_f().round() as i32 + step).clamp(Z_MIN as i32, Z_MAX as i32) as f64;
        self.zoom_to(z, ax, ay);
    }

    /// Look at this zoom — a fraction of a level is allowed — holding the
    /// point at `(ax, ay)` in the view still. The level the tiles come from is
    /// kept where it is until the zoom has drifted `LEVEL_KEEP` away from it.
    pub fn zoom_to(&mut self, zf: f64, ax: f64, ay: f64) {
        let zf = zf.clamp(Z_MIN as f64, Z_MAX as f64);
        let lvl = self.level_for(zf, self.z);
        self.look(zf, lvl, ax, ay, ax, ay);
    }

    /// The level to draw from at `zf`, given the one already in use: `from`
    /// while the zoom is within `LEVEL_KEEP` of it, the nearest level once it
    /// is not.
    fn level_for(&self, zf: f64, from: u32) -> u32 {
        if (zf - f64::from(from)).abs() <= LEVEL_KEEP {
            from
        } else {
            level_of(zf)
        }
    }

    /// Look at `zf`, drawing level `lvl`, so that whatever is under `(sx, sy)`
    /// now ends up under `(dx, dy)`. The two points are the same for a zoom
    /// about a spot; a pinch passes the midpoint it started from and the one
    /// the fingers are at now, which is what makes two fingers pan.
    fn look(&mut self, zf: f64, lvl: u32, sx: f64, sy: f64, dx: f64, dy: f64) {
        let zf = zf.clamp(Z_MIN as f64, Z_MAX as f64);
        // Where the source point is on the world sheet, as a fraction of the
        // whole of it — the one description of the spot that survives a
        // change of level.
        let (ox, oy) = self.origin();
        let w0 = world(self.z);
        let (fx, fy) = ((ox + sx / self.scale) / w0, (oy + sy / self.scale) / w0);
        self.z = lvl.clamp(Z_MIN, Z_MAX);
        self.scale = 2f64.powf(zf - f64::from(self.z));
        let w1 = world(self.z);
        // Put it back under the destination place on the screen.
        self.cx = fx * w1 - dx / self.scale + self.w / (2.0 * self.scale);
        self.cy = fy * w1 - dy / self.scale + self.h / (2.0 * self.scale);
        self.clamp();
    }

    /// Fingers are on the map right now — a settle still running from the
    /// last pinch must let go rather than fight them.
    pub fn pinching(&self) -> bool {
        self.grip.is_some()
    }

    /// Two fingers went down: remember the view, so the whole gesture is one
    /// move from here rather than a chain of small ones.
    pub fn pinch_begin(&mut self) {
        self.grip = Some(Grip {
            z: self.z,
            cx: self.cx,
            cy: self.cy,
            scale: self.scale,
            lvl: self.z,
            anchor: None,
        });
    }

    /// The fingers have spread by `factor` since they went down, their
    /// midpoint at `(ax, ay)` in the map area: magnify about the spot the
    /// gesture began on by that much, and carry that spot to where the
    /// midpoint is now.
    pub fn pinch_to(&mut self, factor: f64, ax: f64, ay: f64) {
        let Some(mut g) = self.grip else { return };
        let factor = if factor.is_finite() && factor > 1e-12 {
            factor
        } else {
            return;
        };
        // The first report of the gesture fixes the spot it is about.
        let (sx, sy) = match g.anchor {
            Some(p) => p,
            None => (ax, ay),
        };
        let from = f64::from(g.z) + g.scale.log2();
        let zf = (from + factor.log2()).clamp(Z_MIN as f64, Z_MAX as f64);
        // The level is decided against the one the gesture is already drawing,
        // not against the grip, so a finger resting on a boundary cannot make
        // it flip on alternate frames.
        let lvl = if (zf - f64::from(g.lvl)).abs() > LEVEL_KEEP {
            level_of(zf)
        } else {
            g.lvl
        };
        g.lvl = lvl;
        g.anchor = Some((sx, sy));
        self.grip = Some(g);
        // Back to where the gesture began, then the whole of it in one move:
        // every step of the pinch is measured from the grip, so a report that
        // arrives out of order or is dropped costs nothing.
        self.z = g.z;
        self.scale = g.scale;
        self.cx = g.cx;
        self.cy = g.cy;
        self.look(zf, lvl, sx, sy, ax, ay);
    }

    /// The fingers lifted.
    ///
    /// Nothing about the view changes: the zoom stays exactly where the
    /// fingers left it, because a map that jumps half a level the moment you
    /// let go of it is the single worst thing a pinch can do. What settles is
    /// which level the tiles are *fetched* from — the whole level nearest the
    /// zoom — and since the view is re-anchored on its own centre at the same
    /// fractional zoom, that is a change no one can see except as the imagery
    /// arriving at its proper size.
    pub fn pinch_end(&mut self) {
        self.grip = None;
        let zf = self.zoom_f();
        let (ax, ay) = (self.w / 2.0, self.h / 2.0);
        self.look(zf, level_of(zf), ax, ay, ax, ay);
    }

    /// The map area in unmagnified pixels: what the view covers of the sheet.
    fn span(&self) -> (f64, f64) {
        (self.w / self.scale, self.h / self.scale)
    }

    /// East and west wrap; north and south stop at the sheet's edges.
    fn clamp(&mut self) {
        let world = world(self.z);
        let (_, vh) = self.span();
        self.cx = self.cx.rem_euclid(world);
        if world > vh {
            self.cy = self.cy.clamp(vh / 2.0, world - vh / 2.0);
        } else {
            self.cy = world / 2.0;
        }
    }

    /// The view's top-left on the world sheet, in unmagnified pixels.
    fn origin(&self) -> (f64, f64) {
        let (vw, vh) = self.span();
        (self.cx - vw / 2.0, self.cy - vh / 2.0)
    }

    /// Every tile the view touches, plus a ring around it so a drag has
    /// something to pull in. Tile x may run outside the world: the engine
    /// wraps it, and the placement wants the unwrapped one.
    pub fn wanted(&self) -> Vec<(i64, i64)> {
        if self.w <= 0.0 || self.h <= 0.0 {
            return Vec::new();
        }
        let (ox, oy) = self.origin();
        let (vw, vh) = self.span();
        let n = 1i64 << self.z;
        let (x0, x1) = ((ox / TILE).floor() as i64 - 1, ((ox + vw) / TILE).floor() as i64 + 1);
        let (y0, y1) = ((oy / TILE).floor() as i64 - 1, ((oy + vh) / TILE).floor() as i64 + 1);
        let mut out = Vec::new();
        for ty in y0.max(0)..=y1.min(n - 1) {
            for tx in x0..=x1 {
                out.push((tx, ty));
            }
        }
        out
    }

    /// Where a tile sits inside the map area and how big it is drawn, in
    /// logical pixels, given how many device pixels there are to a logical one.
    ///
    /// The seams come from here. A tile used to be placed at a fractional
    /// position and given one fractional side, and a renderer resolves that by
    /// rounding the position and the side *separately*: tile n ends at
    /// round(x·d) + round(s·d) and tile n+1 begins at round((x + s)·d), which
    /// is one device pixel further along about as often as not. What shows in
    /// the gap is the page's ground, and on a light map that is a hard dark
    /// hairline down the join — exactly what a screenshot of the phone has in
    /// it, one device pixel of #27272b, not a blend.
    ///
    /// So both edges are put on whole device pixels here and the size is the
    /// difference between them, and a tile's far edge is computed by the very
    /// same expression as its neighbour's near edge: the two are one number by
    /// construction, whatever the magnification. Widths then differ by a pixel
    /// here and there, which is what a fractional grid of whole pixels is.
    ///
    /// `BLEED` is the belt to that pair of braces: the tile is drawn one
    /// device pixel wider and taller than the ground it owns, so a renderer
    /// that truncates where this assumes it rounds still has no crack to show.
    /// The cost is one duplicated column of pixels per join — a third of a
    /// logical pixel of imagery drawn twice, out of continuous map artwork,
    /// which is not visible; the cost of getting it wrong the other way is a
    /// black line across the map, which is all too visible.
    pub fn place(&self, tx: i64, ty: i64, dpr: f64) -> (f32, f32, f32, f32) {
        /// Device pixels of overlap between one tile and the next.
        const BLEED: f64 = 1.0;
        let (ox, oy) = self.origin();
        let d = if dpr.is_finite() && dpr > 0.01 { dpr } else { 1.0 };
        let edge = |t: i64, o: f64| ((t as f64 * TILE - o) * self.scale * d).round();
        let (x0, x1) = (edge(tx, ox), edge(tx + 1, ox));
        let (y0, y1) = (edge(ty, oy), edge(ty + 1, oy));
        (
            (x0 / d) as f32,
            (y0 / d) as f32,
            ((x1 - x0 + BLEED) / d) as f32,
            ((y1 - y0 + BLEED) / d) as f32,
        )
    }

    /// Where the shared point sits inside the map area. It can be off the
    /// view once dragged, and the marker simply goes with it.
    pub fn pin(&self) -> (f32, f32) {
        let (px, py) = px_of(self.lat, self.lon, self.z);
        let (ox, oy) = self.origin();
        let world = world(self.z);
        // The view wraps, so show the point on whichever side of the
        // antimeridian it is nearer to.
        let mut x = px - ox;
        if x < -world / 2.0 {
            x += world;
        } else if x > world / 2.0 {
            x -= world;
        }
        ((x * self.scale) as f32, ((py - oy) * self.scale) as f32)
    }

    /// The tile key the engine answers to: x wrapped into the world.
    pub fn key(&self, tx: i64, ty: i64) -> (u32, i64, i64) {
        let n = 1i64 << self.z;
        (self.z, tx.rem_euclid(n), ty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prime_meridian_at_the_equator_is_the_middle_of_the_sheet() {
        let (x, y) = px_of(0.0, 0.0, 0);
        assert!((x - TILE / 2.0).abs() < 0.001);
        assert!((y - TILE / 2.0).abs() < 0.001);
    }

    #[test]
    fn a_drag_moves_the_view_the_other_way() {
        let mut v = MapView::default();
        v.resize(400.0, 600.0);
        v.open(10.0, 20.0);
        let before = v.cx;
        v.pan(50.0, 0.0);
        assert!((v.cx - (before - 50.0)).abs() < 0.001);
    }

    #[test]
    fn zooming_holds_the_tapped_point_still() {
        let mut v = MapView::default();
        v.resize(400.0, 600.0);
        v.open(10.0, 20.0);
        // the point 100 logical px right of centre, before and after
        let anchor = (300.0, 300.0);
        let world_before = (v.cx - v.w / 2.0 + anchor.0, v.cy - v.h / 2.0 + anchor.1);
        v.zoom(1, anchor.0, anchor.1);
        let world_after = (v.cx - v.w / 2.0 + anchor.0, v.cy - v.h / 2.0 + anchor.1);
        assert!((world_after.0 - world_before.0 * 2.0).abs() < 0.001);
        assert!((world_after.1 - world_before.1 * 2.0).abs() < 0.001);
    }

    #[test]
    fn the_zoom_stays_inside_what_the_renderer_serves() {
        let mut v = MapView::default();
        v.resize(400.0, 600.0);
        v.open(10.0, 20.0);
        for _ in 0..40 {
            v.zoom(1, 200.0, 300.0);
        }
        assert_eq!(v.z, Z_MAX);
        for _ in 0..40 {
            v.zoom(-1, 200.0, 300.0);
        }
        assert_eq!(v.z, Z_MIN);
    }

    #[test]
    fn a_pinch_holds_the_spot_between_the_fingers_still() {
        let mut v = MapView::default();
        v.resize(400.0, 600.0);
        v.open(10.0, 20.0);
        // The midpoint, off centre so a mistake in the anchor shows.
        let (ax, ay) = (120.0, 500.0);
        // Where that spot is on the sheet, as a fraction of the whole of it.
        let spot = |v: &MapView| {
            let (ox, oy) = v.origin();
            let w = world(v.z);
            ((ox + ax / v.scale) / w, (oy + ay / v.scale) / w)
        };
        let before = spot(&v);
        v.pinch_begin();
        for f in [1.1, 1.4, 1.9, 2.6] {
            v.pinch_to(f, ax, ay);
            let now = spot(&v);
            assert!((now.0 - before.0).abs() < 1e-9, "{f}: {now:?} {before:?}");
            assert!((now.1 - before.1).abs() < 1e-9, "{f}: {now:?} {before:?}");
        }
    }

    #[test]
    fn a_pinch_is_measured_from_where_it_began_not_from_the_last_step() {
        let mut v = MapView::default();
        v.resize(400.0, 600.0);
        v.open(10.0, 20.0);
        // What must match is what is on the screen — the zoom, and where the
        // view is looking. Not `z`: which level the tiles are fetched from
        // depends on the road taken, since it is held until the gesture has
        // clearly left it, and that is the whole point of holding it.
        let looking = |v: &MapView| {
            let (ox, oy) = v.origin();
            let w = world(v.z);
            (v.zoom_f(), ox / w, oy / w)
        };
        v.pinch_begin();
        v.pinch_to(3.0, 200.0, 300.0);
        let straight = looking(&v);
        // The same gesture reported in steps must land in the same place.
        v.open(10.0, 20.0);
        v.pinch_begin();
        for f in [1.2, 1.7, 2.4, 3.0] {
            v.pinch_to(f, 200.0, 300.0);
        }
        let stepped = looking(&v);
        assert!((stepped.0 - straight.0).abs() < 1e-9, "{stepped:?} {straight:?}");
        assert!((stepped.1 - straight.1).abs() < 1e-12, "{stepped:?} {straight:?}");
        assert!((stepped.2 - straight.2).abs() < 1e-12, "{stepped:?} {straight:?}");
    }

    #[test]
    fn the_level_drawn_never_strays_far_from_the_zoom() {
        let mut v = MapView::default();
        v.resize(400.0, 600.0);
        v.open(10.0, 20.0);
        v.pinch_begin();
        // A pinch right across the range: the tiles fetched stay within
        // LEVEL_KEEP of what is shown, so they are never stretched past 1.68.
        let cap = 2f64.powf(LEVEL_KEEP);
        for i in 1..=60 {
            v.pinch_to(1.0 + i as f64 * 0.4, 200.0, 300.0);
            assert!(v.scale >= 1.0 / cap - 1e-9 && v.scale <= cap + 1e-9, "{}", v.scale);
            assert!((v.zoom_f() - f64::from(v.z)).abs() <= LEVEL_KEEP + 1e-9);
        }
    }

    #[test]
    fn a_finger_resting_on_a_level_boundary_does_not_swap_the_grid() {
        let mut v = MapView::default();
        v.resize(400.0, 600.0);
        v.open(10.0, 20.0);
        v.pinch_begin();
        // Halfway between two levels is where the old nearest-level rule
        // flipped the whole grid on alternate frames. A hand shaking about
        // that point must keep drawing one level.
        let mut levels = std::collections::HashSet::new();
        for i in 0..40 {
            let jitter = if i % 2 == 0 { 1.0e-3 } else { -1.0e-3 };
            v.pinch_to(2f64.powf(0.5 + jitter), 200.0, 300.0);
            levels.insert(v.z);
        }
        assert_eq!(levels.len(), 1, "the grid swapped levels: {levels:?}");
    }

    #[test]
    fn letting_go_of_a_pinch_does_not_move_the_map() {
        let mut v = MapView::default();
        v.resize(400.0, 600.0);
        v.open(10.0, 20.0);
        v.pinch_begin();
        v.pinch_to(1.3, 200.0, 300.0);
        // Where a spot on the screen is looking, as a fraction of the sheet.
        let spot = |v: &MapView, sx: f64, sy: f64| {
            let (ox, oy) = v.origin();
            let w = world(v.z);
            ((ox + sx / v.scale) / w, (oy + sy / v.scale) / w)
        };
        let zoom = v.zoom_f();
        let corners = [(0.0, 0.0), (400.0, 600.0), (137.0, 421.0)];
        let before: Vec<_> = corners.iter().map(|&(x, y)| spot(&v, x, y)).collect();
        v.pinch_end();
        // The zoom is left exactly where the fingers put it …
        assert!((v.zoom_f() - zoom).abs() < 1e-9, "{} {}", v.zoom_f(), zoom);
        // … and every part of the screen is still looking at the same place.
        for (i, &(x, y)) in corners.iter().enumerate() {
            let now = spot(&v, x, y);
            assert!((now.0 - before[i].0).abs() < 1e-12, "{now:?} {:?}", before[i]);
            assert!((now.1 - before[i].1).abs() < 1e-12, "{now:?} {:?}", before[i]);
        }
        // What it does settle is the level the tiles come from.
        assert_eq!(v.z, level_of(zoom));
    }

    #[test]
    fn two_fingers_carry_the_map_with_them() {
        let mut v = MapView::default();
        v.resize(400.0, 600.0);
        v.open(10.0, 20.0);
        let spot = |v: &MapView, sx: f64, sy: f64| {
            let (ox, oy) = v.origin();
            let w = world(v.z);
            ((ox + sx / v.scale) / w, (oy + sy / v.scale) / w)
        };
        v.pinch_begin();
        // The gesture is about the spot the midpoint started on …
        v.pinch_to(1.0, 100.0, 200.0);
        let held = spot(&v, 100.0, 200.0);
        // … and that spot follows the midpoint across the screen, whatever
        // the fingers do to the zoom on the way.
        for (f, x, y) in [(1.2, 160.0, 260.0), (1.9, 300.0, 180.0), (1.4, 90.0, 500.0)] {
            v.pinch_to(f, x, y);
            let now = spot(&v, x, y);
            assert!((now.0 - held.0).abs() < 1e-12, "{f}: {now:?} {held:?}");
            assert!((now.1 - held.1).abs() < 1e-12, "{f}: {now:?} {held:?}");
        }
    }

    #[test]
    fn a_drag_reported_under_a_pinch_is_ignored() {
        let mut v = MapView::default();
        v.resize(400.0, 600.0);
        v.open(10.0, 20.0);
        v.pinch_begin();
        v.pinch_to(1.2, 200.0, 300.0);
        let (cx, cy) = (v.cx, v.cy);
        v.pan(80.0, -40.0);
        assert!((v.cx - cx).abs() < 1e-12 && (v.cy - cy).abs() < 1e-12);
        // …and it is heard again the moment the fingers are up.
        v.pinch_end();
        let after_lift = v.cx;
        v.pan(80.0, 0.0);
        assert!((v.cx - (after_lift - 80.0 / v.scale)).abs() < 1e-9);
    }

    #[test]
    fn a_pinch_stops_where_the_renderer_does() {
        let mut v = MapView::default();
        v.resize(400.0, 600.0);
        v.open(10.0, 20.0);
        v.pinch_begin();
        v.pinch_to(1e6, 200.0, 300.0);
        assert_eq!(v.z, Z_MAX);
        assert!((v.scale - 1.0).abs() < 1e-9);
        v.pinch_to(1e-6, 200.0, 300.0);
        assert_eq!(v.z, Z_MIN);
        assert!((v.scale - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_drag_after_a_pinch_moves_by_what_the_finger_covered_on_screen() {
        let mut v = MapView::default();
        v.resize(400.0, 600.0);
        v.open(10.0, 20.0);
        v.pinch_begin();
        v.pinch_to(1.3, 200.0, 300.0);
        v.pinch_end();
        let before = v.cx;
        v.pan(50.0, 0.0);
        // 50 drawn pixels, not 50 sheet pixels.
        assert!((v.cx - (before - 50.0 / v.scale)).abs() < 1e-9);
    }

    #[test]
    fn the_recentre_disc_puts_the_point_back_in_the_middle() {
        let mut v = MapView::default();
        v.resize(400.0, 600.0);
        v.open(10.0, 20.0);
        v.pan(500.0, -300.0);
        v.zoom(2, 10.0, 10.0);
        v.recentre();
        let (px, py) = v.pin();
        assert!((f64::from(px) - 200.0).abs() < 0.5, "{px}");
        assert!((f64::from(py) - 300.0).abs() < 0.5, "{py}");
        assert!((v.scale - 1.0).abs() < 1e-9);
    }

    #[test]
    fn the_grid_covers_the_view_with_a_ring_to_spare() {
        let mut v = MapView::default();
        v.resize(TILE as f64, TILE as f64);
        v.open(0.0, 0.0);
        let want = v.wanted();
        // one tile of view plus a ring is three by three at most
        assert!(want.len() >= 9 && want.len() <= 16, "{}", want.len());
    }

    #[test]
    fn neighbouring_tiles_overlap_by_exactly_one_device_pixel() {
        // The seam test, as arithmetic: whatever the magnification and
        // whatever the device pixel ratio, every edge sits on a whole device
        // pixel and a tile reaches one device pixel past where its neighbour
        // begins. A gap of any size would be a hairline of ground; more than a
        // pixel of overlap would be imagery drawn where it does not belong.
        for dpr in [1.0, 1.5, 2.0, 2.625, 3.0] {
            for mag in [0.6, 0.7071, 0.83, 1.0, 1.0001, 1.13, 1.4142, 1.618, 1.68] {
                let mut v = MapView::default();
                v.resize(400.0, 600.0);
                v.open(51.5, -0.12);
                v.scale = mag;
                v.clamp();
                let mut seen: Vec<(i64, i64, f32, f32, f32, f32)> = Vec::new();
                for (tx, ty) in v.wanted() {
                    let (x, y, w, h) = v.place(tx, ty, dpr);
                    // Whole device pixels, both corners.
                    for e in [x, y, x + w, y + h] {
                        let d = f64::from(e) * dpr;
                        assert!(
                            (d - d.round()).abs() < 1e-3,
                            "dpr {dpr} mag {mag}: edge {e} is not a device pixel"
                        );
                    }
                    assert!(w > 0.0 && h > 0.0, "dpr {dpr} mag {mag}: empty tile");
                    seen.push((tx, ty, x, y, w, h));
                }
                let over = 1.0 / dpr; // one device pixel, in logical ones
                for &(tx, ty, x, y, w, h) in &seen {
                    if let Some(&(_, _, rx, _, _, _)) =
                        seen.iter().find(|t| t.0 == tx + 1 && t.1 == ty)
                    {
                        let gap = f64::from(rx) - f64::from(x + w);
                        assert!(
                            (gap + over).abs() < 1e-3,
                            "dpr {dpr} mag {mag}: {gap} beside {tx},{ty}, wanted -{over}"
                        );
                    }
                    if let Some(&(_, _, _, by, _, _)) =
                        seen.iter().find(|t| t.0 == tx && t.1 == ty + 1)
                    {
                        let gap = f64::from(by) - f64::from(y + h);
                        assert!(
                            (gap + over).abs() < 1e-3,
                            "dpr {dpr} mag {mag}: {gap} under {tx},{ty}, wanted -{over}"
                        );
                    }
                }
            }
        }
    }
}
