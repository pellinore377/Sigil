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

/// The page's view of the map: where it is looking and what it has drawn.
pub struct MapView {
    pub z: u32,
    /// The centre of the view on the world sheet at `z`, in drawn pixels.
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
}

impl Default for MapView {
    fn default() -> Self {
        MapView {
            z: 15,
            cx: 0.0,
            cy: 0.0,
            lat: 0.0,
            lon: 0.0,
            w: 0.0,
            h: 0.0,
            have: HashMap::new(),
            asked: HashSet::new(),
            epoch: 0,
        }
    }
}

impl MapView {
    /// Open on a point: centre there, forget the last page's grid.
    pub fn open(&mut self, lat: f64, lon: f64) {
        self.epoch = self.epoch.wrapping_add(1);
        self.z = 15;
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
    /// against it.
    pub fn pan(&mut self, dx: f64, dy: f64) {
        self.cx -= dx;
        self.cy -= dy;
        self.clamp();
    }

    /// A step in or out, holding the point at `(ax, ay)` in the view still —
    /// what a double tap on a map does. Pass the view's centre for the chips.
    pub fn zoom(&mut self, step: i32, ax: f64, ay: f64) {
        let z = (self.z as i32 + step).clamp(Z_MIN as i32, Z_MAX as i32) as u32;
        if z == self.z {
            return;
        }
        let s = 2f64.powi(z as i32 - self.z as i32);
        let (px, py) = (self.cx - self.w / 2.0 + ax, self.cy - self.h / 2.0 + ay);
        self.z = z;
        self.cx = px * s - ax + self.w / 2.0;
        self.cy = py * s - ay + self.h / 2.0;
        self.clamp();
    }

    /// East and west wrap; north and south stop at the sheet's edges.
    fn clamp(&mut self) {
        let world = world(self.z);
        self.cx = self.cx.rem_euclid(world);
        if world > self.h {
            self.cy = self.cy.clamp(self.h / 2.0, world - self.h / 2.0);
        } else {
            self.cy = world / 2.0;
        }
    }

    /// The view's top-left on the world sheet.
    fn origin(&self) -> (f64, f64) {
        (self.cx - self.w / 2.0, self.cy - self.h / 2.0)
    }

    /// Every tile the view touches, plus a ring around it so a drag has
    /// something to pull in. Tile x may run outside the world: the engine
    /// wraps it, and the placement wants the unwrapped one.
    pub fn wanted(&self) -> Vec<(i64, i64)> {
        if self.w <= 0.0 || self.h <= 0.0 {
            return Vec::new();
        }
        let (ox, oy) = self.origin();
        let n = 1i64 << self.z;
        let (x0, x1) = ((ox / TILE).floor() as i64 - 1, ((ox + self.w) / TILE).floor() as i64 + 1);
        let (y0, y1) = ((oy / TILE).floor() as i64 - 1, ((oy + self.h) / TILE).floor() as i64 + 1);
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
        ((tx as f64 * TILE - ox) as f32, (ty as f64 * TILE - oy) as f32)
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
        (x as f32, (py - oy) as f32)
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
    fn the_grid_covers_the_view_with_a_ring_to_spare() {
        let mut v = MapView::default();
        v.resize(TILE as f64, TILE as f64);
        v.open(0.0, 0.0);
        let want = v.wanted();
        // one tile of view plus a ring is three by three at most
        assert!(want.len() >= 9 && want.len() <= 16, "{}", want.len());
    }
}
