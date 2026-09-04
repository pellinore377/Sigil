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
}

/// The page's view of the map: where it is looking and what it has drawn.
pub struct MapView {
    /// The level the tiles are fetched from: always a whole one, since that
    /// is all the renderer serves.
    pub z: u32,
    /// How much those tiles are magnified as they are drawn. A pinch moves
    /// this continuously; between gestures it is 1. The view's real zoom is
    /// `z + log2(scale)`, and `z` is kept as the level nearest it, so `scale`
    /// stays inside [1/√2, √2] and the tiles under the fingers are always the
    /// right ones.
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

    /// Dragged by this much: the map goes with the finger, so the centre goes
    /// against it. The drag is in drawn pixels, the centre in unmagnified
    /// ones, so it divides through the magnification.
    pub fn pan(&mut self, dx: f64, dy: f64) {
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
    /// point at `(ax, ay)` in the view still. The whole level nearest it
    /// becomes the one the tiles come from and the remainder becomes the
    /// magnification, so the grid is never more than √2 off its drawn size.
    pub fn zoom_to(&mut self, zf: f64, ax: f64, ay: f64) {
        let zf = zf.clamp(Z_MIN as f64, Z_MAX as f64);
        // Where the anchor is on the world sheet, as a fraction of the whole
        // of it — the one description of the spot that survives a change of
        // level.
        let (ox, oy) = self.origin();
        let w0 = world(self.z);
        let (fx, fy) = ((ox + ax / self.scale) / w0, (oy + ay / self.scale) / w0);
        self.z = (zf.round() as i64).clamp(Z_MIN as i64, Z_MAX as i64) as u32;
        self.scale = 2f64.powf(zf - self.z as f64);
        let w1 = world(self.z);
        // Put the anchor back under the same place on the screen.
        self.cx = fx * w1 - ax / self.scale + self.w / (2.0 * self.scale);
        self.cy = fy * w1 - ay / self.scale + self.h / (2.0 * self.scale);
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
        });
    }

    /// The fingers have spread by `factor` since they went down, their
    /// midpoint at `(ax, ay)` in the map area: magnify about it by that much.
    pub fn pinch_to(&mut self, factor: f64, ax: f64, ay: f64) {
        let Some(g) = self.grip else { return };
        let factor = if factor.is_finite() && factor > 1e-12 {
            factor
        } else {
            return;
        };
        // Back to where the gesture began, then the whole of it in one move.
        let from = g.z as f64 + g.scale.log2();
        self.z = g.z;
        self.scale = g.scale;
        self.cx = g.cx;
        self.cy = g.cy;
        self.zoom_to(from + factor.log2(), ax, ay);
    }

    /// The fingers lifted. Returns the zoom to ease to — the whole level
    /// nearest where the pinch left off — or `None` if the view is already on
    /// one.
    pub fn pinch_end(&mut self) -> Option<f64> {
        self.grip = None;
        let zf = self.zoom_f();
        let to = zf.round().clamp(Z_MIN as f64, Z_MAX as f64);
        if (zf - to).abs() < 1e-4 {
            self.scale = 1.0;
            None
        } else {
            Some(to)
        }
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

    /// Where a tile sits inside the map area, in logical pixels.
    pub fn place(&self, tx: i64, ty: i64) -> (f32, f32) {
        let (ox, oy) = self.origin();
        (
            ((tx as f64 * TILE - ox) * self.scale) as f32,
            ((ty as f64 * TILE - oy) * self.scale) as f32,
        )
    }

    /// A tile's side where it is drawn, magnification included.
    pub fn tile_size(&self) -> f32 {
        (TILE * self.scale) as f32
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
        v.pinch_begin();
        v.pinch_to(3.0, 200.0, 300.0);
        let straight = (v.z, v.scale, v.cx, v.cy);
        // The same gesture reported in steps must land in the same place.
        v.open(10.0, 20.0);
        v.pinch_begin();
        for f in [1.2, 1.7, 2.4, 3.0] {
            v.pinch_to(f, 200.0, 300.0);
        }
        assert_eq!(v.z, straight.0);
        assert!((v.scale - straight.1).abs() < 1e-9);
        assert!((v.cx - straight.2).abs() < 1e-6);
        assert!((v.cy - straight.3).abs() < 1e-6);
    }

    #[test]
    fn the_level_drawn_is_always_the_one_nearest_the_zoom() {
        let mut v = MapView::default();
        v.resize(400.0, 600.0);
        v.open(10.0, 20.0);
        v.pinch_begin();
        // A pinch right across the range: the tiles fetched stay within half
        // a level of what is shown, so they are never stretched past √2.
        for i in 1..=60 {
            v.pinch_to(1.0 + i as f64 * 0.4, 200.0, 300.0);
            assert!(v.scale >= 0.7071 - 1e-9 && v.scale <= 1.4143, "{}", v.scale);
            assert!((v.zoom_f() - v.z as f64).abs() <= 0.5 + 1e-9);
        }
    }

    #[test]
    fn the_fingers_lift_onto_a_whole_zoom() {
        let mut v = MapView::default();
        v.resize(400.0, 600.0);
        v.open(10.0, 20.0);
        v.pinch_begin();
        v.pinch_to(1.3, 200.0, 300.0);
        let to = v.pinch_end().expect("mid-level, so there is somewhere to go");
        assert_eq!(to, to.round());
        v.zoom_to(to, 200.0, 300.0);
        assert!((v.scale - 1.0).abs() < 1e-9);
        // A pinch that went nowhere has nothing to settle.
        v.pinch_begin();
        v.pinch_to(1.0, 200.0, 300.0);
        assert!(v.pinch_end().is_none());
        assert!((v.scale - 1.0).abs() < 1e-9);
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
    fn a_drag_mid_pinch_moves_by_what_the_finger_covered_on_screen() {
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
    fn the_grid_covers_the_view_with_a_ring_to_spare() {
        let mut v = MapView::default();
        v.resize(TILE as f64, TILE as f64);
        v.open(0.0, 0.0);
        let want = v.wanted();
        // one tile of view plus a ring is three by three at most
        assert!(want.len() >= 9 && want.len() <= 16, "{}", want.len());
    }
}
