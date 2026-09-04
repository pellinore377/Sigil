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

use std::collections::{HashMap, HashSet, VecDeque};

/// A tile's side where it is drawn, in logical pixels.
pub const TILE: f64 = 256.0;
/// The zoom range the engine's renderer accepts (composite.rs clamps to it).
pub const Z_MIN: u32 = 3;
pub const Z_MAX: u32 = 19;

/// How many levels up the placement will look for imagery to stand in for a
/// tile that has not arrived. Three is 8× magnification — soft, but it is the
/// difference between a blurred map and a hole in one, and it only shows for
/// as long as the real tile takes to land.
pub const SUB_UP: u32 = 3;

/// How many tiles one settle may put in the queue, over and above whatever is
/// already in hand.
///
/// On the phone the view itself is 25-35 tiles at the level it is drawing, so
/// once those have arrived the whole of this goes to the neighbours: about
/// half to the level out (a quarter as many tiles for the same ground, plus
/// its ring) and the rest to the level in, nearest the middle first. Twenty
/// odd tiles at the level in is the central quarter of the view, which is
/// exactly the ground a one-level pinch-in lands on.
///
/// It is a budget rather than a pyramid on purpose: the level below THAT is
/// four times the bill again for ground nobody reaches without a second
/// gesture — and by the time they take it, this has run again from wherever
/// they landed.
pub const PREFETCH_BUDGET: usize = 48;

/// How many requests may be with the engine at once.
///
/// The engine renders a tile on whichever of its runtime's worker threads
/// picked the request up, without handing the work to a blocking pool
/// (core/src/maps/render.rs), so forty requests in flight is forty threads of
/// tiny-skia and everything else the app wants to do queues behind it. Six
/// keeps the cores busy and — this is the part that matters — makes the order
/// mean something: with every fetch going through one queue, nearest the
/// middle first is a promise the engine actually keeps, instead of a list
/// handed over all at once for the runtime to shuffle.
pub const LANES: usize = 6;

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

/// The tile `d` levels above `(z, tx, ty)`, and which part of that tile's
/// picture is this one's ground.
///
/// A tile is one quarter of its parent — the quarter its lowest bit of x and
/// of y name — so `d` levels up it is one 2^d-th of the ancestor along each
/// axis, at the place the lowest `d` bits name. The crop comes back as
/// fractions of the ancestor's picture, which is all the caller needs: the
/// pictures arrive at 512 square today and the fractions do not care.
///
/// Tile x runs outside the world here — the ring the view asks for laps off
/// the left of the sheet — so the halving is floor division and the quadrant
/// is the Euclidean remainder. The two agree by construction: for tx = −1 and
/// d = 1 the ancestor is −1 and the quadrant is 1, the right half, which is
/// where the tile at −1 sits inside the tile at −1 one level up. The ancestor
/// key is wrapped in x and not in y, exactly as `MapView::key` wraps.
pub fn ancestor(z: u32, tx: i64, ty: i64, d: u32) -> ((u32, i64, i64), (f32, f32, f32, f32)) {
    let s = 1i64 << d;
    let az = z.saturating_sub(d);
    let n = 1i64 << az;
    let f = 1.0 / s as f32;
    (
        (az, tx.div_euclid(s).rem_euclid(n), ty.div_euclid(s)),
        (
            tx.rem_euclid(s) as f32 * f,
            ty.rem_euclid(s) as f32 * f,
            f,
            f,
        ),
    )
}

/// The four tiles one level below `(z, tx, ty)`, each with the quarter of this
/// tile's box it fills: `(i, j)` with i across and j down.
pub fn children(z: u32, tx: i64, ty: i64) -> [((u32, i64, i64), (i64, i64)); 4] {
    let n = 1i64 << (z + 1);
    let mut out = [((0u32, 0i64, 0i64), (0i64, 0i64)); 4];
    for (k, (i, j)) in [(0, 0), (1, 0), (0, 1), (1, 1)].into_iter().enumerate() {
        out[k] = (
            (z + 1, (tx * 2 + i).rem_euclid(n), ty * 2 + j),
            (i, j),
        );
    }
    out
}

/// One picture to draw, and where.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    /// Which rendered tile's picture this is — not necessarily the tile whose
    /// ground it is covering.
    pub key: (u32, i64, i64),
    /// The box it fills in the map area, in logical pixels.
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// The part of that picture to draw, as fractions of it: (0, 0, 1, 1) for
    /// a whole tile, a quarter of it where a parent is standing in for one of
    /// its four children, a sixteenth two levels up, and so on.
    pub fx: f32,
    pub fy: f32,
    pub fw: f32,
    pub fh: f32,
}

impl Placed {
    /// A tile drawn whole in its own box.
    fn whole(key: (u32, i64, i64), b: (f32, f32, f32, f32)) -> Placed {
        Placed { key, x: b.0, y: b.1, w: b.2, h: b.3, fx: 0.0, fy: 0.0, fw: 1.0, fh: 1.0 }
    }
}

/// The grid, ready to hand to the page.
pub struct Layout {
    /// Back to front: every stand-in first and the real tiles over them, so a
    /// tile that arrives covers the borrowed imagery under it — and covers the
    /// stand-in's seams with its own, which are the ones that were measured.
    pub rows: Vec<Placed>,
    /// Tiles the view is showing that have no picture of their own yet,
    /// nearest the middle of the view first.
    pub missing: Vec<(u32, i64, i64)>,
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
    /// Tiles with the engine right now and not yet answered, so nothing is
    /// asked for twice. `LANES` bounds it.
    pub asked: HashSet<(u32, i64, i64)>,
    /// Tiles to ask for, in the order they are wanted: what the view is
    /// showing a hole for at the head, then what a pinch either way will want,
    /// nearest the middle of the view outwards. Drained `LANES` at a time —
    /// see `next_fetch`.
    pub queue: VecDeque<(u32, i64, i64)>,
    /// The page is open: a reply for another point is stale.
    pub epoch: u64,
    /// Bumped every time the view moves, so the prefetch that runs when the
    /// moving stops can tell it is the last one. A drag reports every frame
    /// and the wish-list is only worth computing once the finger is off.
    pub settle: u64,
    /// The page has been on screen since it was opened.
    ///
    /// The window changes `nav` AFTER running the open action, so at the
    /// moment the page asks for its first tiles the window still says it is
    /// showing the conversation. Until this has been set once, a `nav` that is
    /// not the map means the page is on its way up; after it, the same reading
    /// means the page is gone and the queue is imagery for a gesture nobody
    /// will make.
    pub shown: bool,
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
            queue: VecDeque::new(),
            epoch: 0,
            settle: 0,
            shown: false,
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
        self.queue.clear();
        self.shown = false;
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
        self.cover_at(self.z, 1)
    }

    /// Every tile at `z` whose ground the view covers, plus `ring` tiles all
    /// round it. At the level being drawn with a ring of one this is `wanted`;
    /// at the neighbouring levels it is what a pinch will be asking for in a
    /// moment. The view's ground is the level-independent thing here, so the
    /// sheet is simply rescaled from the level in hand to the one asked about.
    pub fn cover_at(&self, z: u32, ring: i64) -> Vec<(i64, i64)> {
        if self.w <= 0.0 || self.h <= 0.0 || z < Z_MIN || z > Z_MAX {
            return Vec::new();
        }
        let k = 2f64.powi(z as i32 - self.z as i32);
        let (ox, oy) = self.origin();
        let (vw, vh) = self.span();
        let (ox, oy, vw, vh) = (ox * k, oy * k, vw * k, vh * k);
        let n = 1i64 << z;
        let (x0, x1) = (
            (ox / TILE).floor() as i64 - ring,
            ((ox + vw) / TILE).floor() as i64 + ring,
        );
        let (y0, y1) = (
            (oy / TILE).floor() as i64 - ring,
            ((oy + vh) / TILE).floor() as i64 + ring,
        );
        let mut out = Vec::new();
        'rows: for ty in y0.max(0)..=y1.min(n - 1) {
            for tx in x0..=x1 {
                out.push((tx, ty));
                // A stop, not a policy — the caller orders these and trims
                // them to the budget. It is here so that a freakishly large
                // window cannot turn a settle into a million-element sort.
                if out.len() >= 4096 {
                    break 'rows;
                }
            }
        }
        out
    }

    /// How far a tile's middle is from the middle of the view, at that tile's
    /// own level, in tiles squared. The order the fetches go out in.
    fn near(&self, z: u32, tx: i64, ty: i64) -> f64 {
        let k = 2f64.powi(z as i32 - self.z as i32) / TILE;
        let (dx, dy) = (
            tx as f64 + 0.5 - self.cx * k,
            ty as f64 + 0.5 - self.cy * k,
        );
        dx * dx + dy * dy
    }

    /// What to have in hand for where the view is standing: the level being
    /// drawn, then the level one out, then the level one in — each over the
    /// ground the view covers, each ordered from the middle outwards. The
    /// caller drops what it already has and trims the rest to the budget
    /// (`refill`); this is the whole ordered wish-list.
    ///
    /// Why those three levels and no more. A pinch can only go one way at a
    /// time, and one level is about as far as it gets before the fingers have
    /// to lift and start again, so the levels either side are exactly the
    /// imagery the next gesture asks for. A fourth level is four times the
    /// bill again for ground nobody reaches without a second gesture — and by
    /// the time they take it, this has run afresh from where they landed.
    ///
    /// The order is as much the point as the set is. A pinch magnifies about
    /// the fingers, which sit near the middle of the screen far more often
    /// than not, so the middle is what shows first and must therefore BE
    /// first. The level out comes before the level in because it is a quarter
    /// of the tiles for the same ground, and because those same tiles are what
    /// `layout` crops to cover a hole at the level being drawn — it pays for
    /// itself twice.
    pub fn prefetch(&self) -> Vec<(u32, i64, i64)> {
        let mut out: Vec<(u32, i64, i64)> = Vec::new();
        let mut seen: HashSet<(u32, i64, i64)> = HashSet::new();
        // The level on screen keeps the ring it always had, so a drag has
        // something to pull in, and so does the level out — its ring is what
        // stands in for the ring at this level. The level in takes the view's
        // ground and no more; there are four times as many of those and the
        // budget is better spent near the middle.
        let levels = [
            Some((self.z, 1i64)),
            (self.z > Z_MIN).then(|| (self.z - 1, 1)),
            (self.z < Z_MAX).then(|| (self.z + 1, 0)),
        ];
        for (z, ring) in levels.into_iter().flatten() {
            let mut lvl = self.cover_at(z, ring);
            lvl.sort_by(|a, b| self.near(z, a.0, a.1).total_cmp(&self.near(z, b.0, b.1)));
            let n = 1i64 << z;
            for (tx, ty) in lvl {
                let k = (z, tx.rem_euclid(n), ty);
                if seen.insert(k) {
                    out.push(k);
                }
            }
        }
        out
    }

    /// Rebuild the queue for where the view is now: the wish-list, less
    /// whatever is already in hand or already with the engine, trimmed to the
    /// budget.
    ///
    /// This is also how a level the fingers have pinched past is dropped. The
    /// queue is not added to, it is REPLACED, so tiles for a level the view
    /// has left are simply not in the new one — no bookkeeping, no stale work
    /// ahead of the fresh. Only what is already with the engine carries on,
    /// and `LANES` holds that to six.
    pub fn refill(&mut self) {
        self.queue.clear();
        for k in self.prefetch() {
            if self.queue.len() >= PREFETCH_BUDGET {
                break;
            }
            if !self.have.contains_key(&k) && !self.asked.contains(&k) {
                self.queue.push_back(k);
            }
        }
    }

    /// Put these at the head of the queue: tiles the view is showing a hole or
    /// a stand-in for right now, which outrank anything a gesture might want
    /// later. Given nearest-first they stay in that order, and a tile already
    /// promised further down is moved up rather than asked for twice.
    pub fn want_now(&mut self, keys: &[(u32, i64, i64)]) {
        for &k in keys.iter().rev() {
            if self.have.contains_key(&k) || self.asked.contains(&k) {
                continue;
            }
            self.queue.retain(|q| *q != k);
            self.queue.push_front(k);
        }
    }

    /// The next tile to ask the engine for, or nothing while the lanes are
    /// full. The caller loops on it and calls it again as each reply lands.
    pub fn next_fetch(&mut self) -> Option<(u32, i64, i64)> {
        if self.asked.len() >= LANES {
            return None;
        }
        while let Some(k) = self.queue.pop_front() {
            if self.have.contains_key(&k) || self.asked.contains(&k) {
                continue;
            }
            self.asked.insert(k);
            return Some(k);
        }
        None
    }

    /// The grid as it should be drawn, given what is in hand — `has` is the
    /// caller's cache, asked one key at a time so this stays arithmetic.
    ///
    /// Where a tile has arrived it is drawn whole in its own box. Where it has
    /// not, the classic slippy-map fallback keeps the ground covered rather
    /// than blank:
    ///
    /// * the nearest ANCESTOR in hand, up to `SUB_UP` levels up, cropped to
    ///   this tile's share of it — one picture, always complete, and after a
    ///   pinch inwards it is the very level the fingers just left, so it is
    ///   always there. The magnification is 2, 4 or 8, which is soft; it is
    ///   over the moment the real tile lands.
    /// * failing that, whichever of the four CHILDREN have arrived, each drawn
    ///   into its own quarter of the box — what covers a pinch outwards, where
    ///   the level left behind is the one below. Cover may be partial: three
    ///   children and a hole is still three quarters better than a hole.
    ///
    /// A child's quarters share an inner edge that `place` never measured, so
    /// it is rounded onto a whole device pixel here and the near child reaches
    /// one past it, for the same reason and by the same means as `place`'s own
    /// `BLEED`: a renderer that rounds a position and a size apart shows the
    /// ground through the join otherwise.
    pub fn layout(&self, dpr: f64, has: &dyn Fn((u32, i64, i64)) -> bool) -> Layout {
        let d = if dpr.is_finite() && dpr > 0.01 { dpr } else { 1.0 };
        let mut under: Vec<Placed> = Vec::new();
        let mut over: Vec<Placed> = Vec::new();
        let mut missing: Vec<(u32, i64, i64)> = Vec::new();
        let mut tiles = self.wanted();
        // Nearest the middle first, so `missing` comes out in the order the
        // fetches should go out in and the caller need not sort again.
        tiles.sort_by(|a, b| {
            self.near(self.z, a.0, a.1)
                .total_cmp(&self.near(self.z, b.0, b.1))
        });
        for (tx, ty) in tiles {
            let b = self.place(tx, ty, d);
            let key = self.key(tx, ty);
            if has(key) {
                over.push(Placed::whole(key, b));
                continue;
            }
            missing.push(key);
            // The nearest ancestor in hand covers the whole box on its own.
            let up = SUB_UP.min(self.z.saturating_sub(Z_MIN));
            if let Some(p) = (1..=up).find_map(|dd| {
                let (ak, (fx, fy, fw, fh)) = ancestor(self.z, tx, ty, dd);
                has(ak).then_some(Placed { key: ak, x: b.0, y: b.1, w: b.2, h: b.3, fx, fy, fw, fh })
            }) {
                under.push(p);
                continue;
            }
            if self.z >= Z_MAX {
                continue;
            }
            // Failing that, the children, each in its own quarter.
            let mid = |a: f32, s: f32| ((f64::from(a) + f64::from(s) / 2.0) * d).round() / d;
            let (mx, my) = (mid(b.0, b.2), mid(b.1, b.3));
            for (ck, (i, j)) in children(self.z, tx, ty) {
                if !has(ck) {
                    continue;
                }
                let (x0, x1) = if i == 0 {
                    (f64::from(b.0), mx + 1.0 / d)
                } else {
                    (mx, f64::from(b.0) + f64::from(b.2))
                };
                let (y0, y1) = if j == 0 {
                    (f64::from(b.1), my + 1.0 / d)
                } else {
                    (my, f64::from(b.1) + f64::from(b.3))
                };
                under.push(Placed::whole(
                    ck,
                    (x0 as f32, y0 as f32, (x1 - x0) as f32, (y1 - y0) as f32),
                ));
            }
        }
        under.extend(over);
        Layout { rows: under, missing }
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

    // ---------------------------------------------- the stand-in arithmetic

    #[test]
    fn a_tile_is_the_quarter_of_its_parent_that_its_low_bits_name() {
        let z = 15;
        let (px, py) = (9000i64, 6000i64);
        for (i, j) in [(0i64, 0i64), (1, 0), (0, 1), (1, 1)] {
            let (k, crop) = ancestor(z + 1, px * 2 + i, py * 2 + j, 1);
            assert_eq!(k, (z, px, py), "quadrant {i},{j}");
            assert_eq!(
                crop,
                (i as f32 * 0.5, j as f32 * 0.5, 0.5, 0.5),
                "quadrant {i},{j}"
            );
        }
    }

    #[test]
    fn the_four_quarters_of_a_parent_partition_it_exactly() {
        let z = 12;
        let (px, py) = (100i64, 200i64);
        let mut area = 0.0f32;
        let mut corners = HashSet::new();
        for (i, j) in [(0i64, 0i64), (1, 0), (0, 1), (1, 1)] {
            let (_, (fx, fy, fw, fh)) = ancestor(z + 1, px * 2 + i, py * 2 + j, 1);
            area += fw * fh;
            corners.insert(((fx * 4.0) as i32, (fy * 4.0) as i32));
        }
        // Four quarters, four different corners, and the whole of it covered.
        assert_eq!(corners.len(), 4);
        assert!((area - 1.0).abs() < 1e-6, "{area}");
    }

    #[test]
    fn further_up_the_crop_is_smaller_and_the_low_bits_still_place_it() {
        // Two levels up a tile is a sixteenth of its ancestor, at the column
        // and row its lowest two bits of x and y name.
        let (k, (fx, fy, fw, fh)) = ancestor(16, 0b1010, 0b1101, 2);
        assert_eq!(k, (14, 0b10, 0b11));
        assert!((fx - 0.5).abs() < 1e-6, "{fx}"); // low two bits of x = 2 of 4
        assert!((fy - 0.25).abs() < 1e-6, "{fy}"); // low two bits of y = 1 of 4
        assert!((fw - 0.25).abs() < 1e-6 && (fh - 0.25).abs() < 1e-6);
        // Three up, a sixty-fourth.
        let (_, (_, _, fw, _)) = ancestor(16, 0b1010, 0b1101, 3);
        assert!((fw - 0.125).abs() < 1e-6, "{fw}");
    }

    #[test]
    fn a_tile_off_the_left_of_the_sheet_still_names_its_parents_right_half() {
        // The ring the view asks for laps past the antimeridian, so negative
        // tile x is ordinary here: tile −1 is the RIGHT half of the tile at
        // −1 one level up, and the engine is asked for the wrapped key, x
        // only — exactly as `key` wraps.
        let (k, (fx, fy, fw, _)) = ancestor(4, -1, 3, 1);
        assert_eq!(k, (3, (1i64 << 3) - 1, 1));
        assert!((fx - 0.5).abs() < 1e-6 && (fy - 0.5).abs() < 1e-6 && (fw - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_tiles_children_are_the_four_below_it_in_their_own_quarters() {
        let kids = children(10, 5, 7);
        let mut seen = HashSet::new();
        for ((z, x, y), (i, j)) in kids {
            assert_eq!(z, 11);
            assert_eq!((x, y), (10 + i, 14 + j));
            // …and each names its parent back.
            assert_eq!(ancestor(11, x, y, 1).0, (10, 5, 7));
            seen.insert((i, j));
        }
        assert_eq!(seen.len(), 4);
    }

    /// A tile is missing, its parent is in hand: the hole must be filled by
    /// the parent's own quarter, in exactly the box the tile would have had.
    #[test]
    fn a_missing_tile_is_covered_by_its_parents_own_quarter() {
        let dpr = 3.0;
        // Every quadrant, by walking the hole around the grid: with the view
        // at an arbitrary point, four adjacent tiles are the four quadrants.
        for step in 0..4 {
            let mut v = MapView::default();
            v.resize(400.0, 600.0);
            v.open(51.5, -0.12);
            let all = v.wanted();
            let base = all[all.len() / 2];
            let hole = (base.0 + step % 2, base.1 + step / 2);
            let hole_k = v.key(hole.0, hole.1);
            let (par_k, crop) = ancestor(v.z, hole.0, hole.1, 1);
            let mut held: HashSet<(u32, i64, i64)> =
                all.iter().map(|&(x, y)| v.key(x, y)).collect();
            held.remove(&hole_k);
            held.insert(par_k);
            let plan = v.layout(dpr, &|k| held.contains(&k));
            assert_eq!(plan.missing, vec![hole_k], "step {step}");
            let sub: Vec<_> = plan.rows.iter().filter(|p| p.key == par_k).collect();
            assert_eq!(sub.len(), 1, "step {step}: {sub:?}");
            let p = sub[0];
            // The parent's picture, cropped to this tile's quarter …
            assert_eq!((p.fx, p.fy, p.fw, p.fh), crop, "step {step}");
            // … drawn in the very box `place` gives the tile it stands for.
            assert_eq!((p.x, p.y, p.w, p.h), v.place(hole.0, hole.1, dpr), "step {step}");
            // … and drawn UNDER the real tiles, so imagery arriving covers it.
            let first_real = plan.rows.iter().position(|r| r.key != par_k).unwrap();
            let stand_in = plan.rows.iter().position(|r| r.key == par_k).unwrap();
            assert!(stand_in < first_real, "step {step}");
            // Every other tile is itself, whole.
            for r in plan.rows.iter().filter(|r| r.key != par_k) {
                assert_eq!((r.fx, r.fy, r.fw, r.fh), (0.0, 0.0, 1.0, 1.0));
            }
        }
    }

    #[test]
    fn with_nothing_nearer_the_stand_in_comes_from_further_up() {
        let mut v = MapView::default();
        v.resize(400.0, 600.0);
        v.open(51.5, -0.12);
        let (tx, ty) = v.wanted()[0];
        for d in 1..=SUB_UP {
            let (k, crop) = ancestor(v.z, tx, ty, d);
            let held: HashSet<_> = [k].into_iter().collect();
            let plan = v.layout(3.0, &|q| held.contains(&q));
            // Three levels up an ancestor covers sixty-four tiles, so several
            // rows carry its key: the one wanted is the row standing in THIS
            // tile's box.
            let b = v.place(tx, ty, 3.0);
            let p = plan
                .rows
                .iter()
                .find(|p| p.key == k && (p.x, p.y, p.w, p.h) == b)
                .unwrap_or_else(|| panic!("no stand-in {d} levels up"));
            assert_eq!((p.fx, p.fy, p.fw, p.fh), crop, "{d} levels up");
        }
        // Beyond SUB_UP nothing is borrowed: an eighth of a tile's detail
        // blown up sixteen times is not imagery, it is a smear.
        let (k, _) = ancestor(v.z, tx, ty, SUB_UP + 1);
        let held: HashSet<_> = [k].into_iter().collect();
        let plan = v.layout(3.0, &|q| held.contains(&q));
        assert!(plan.rows.is_empty(), "{:?}", plan.rows);
    }

    #[test]
    fn four_children_tile_the_box_their_parent_would_have_filled() {
        let dpr = 3.0;
        let mut v = MapView::default();
        v.resize(400.0, 600.0);
        v.open(51.5, -0.12);
        // Nothing at this level, everything one level down — a pinch outwards,
        // where the level left behind is the one below.
        let all = v.wanted();
        let mut held = HashSet::new();
        for &(tx, ty) in &all {
            for (ck, _) in children(v.z, tx, ty) {
                held.insert(ck);
            }
        }
        let plan = v.layout(dpr, &|k| held.contains(&k));
        assert_eq!(plan.missing.len(), all.len());
        let (tx, ty) = all[all.len() / 2];
        let (bx, by, bw, bh) = v.place(tx, ty, dpr);
        let kids: HashSet<_> = children(v.z, tx, ty).iter().map(|c| c.0).collect();
        let mine: Vec<Placed> = plan
            .rows
            .iter()
            .filter(|p| kids.contains(&p.key) && p.x >= bx - 1.0 && p.x < bx + bw)
            .copied()
            .collect();
        assert_eq!(mine.len(), 4, "{mine:?}");
        // Each child is drawn whole — it is the right size already.
        for p in &mine {
            assert_eq!((p.fx, p.fy, p.fw, p.fh), (0.0, 0.0, 1.0, 1.0));
        }
        // Together they fill the parent's box and nothing outside it.
        let l = mine.iter().map(|p| p.x).fold(f32::MAX, f32::min);
        let t = mine.iter().map(|p| p.y).fold(f32::MAX, f32::min);
        let r = mine.iter().map(|p| p.x + p.w).fold(f32::MIN, f32::max);
        let b = mine.iter().map(|p| p.y + p.h).fold(f32::MIN, f32::max);
        assert!((l - bx).abs() < 1e-3 && (t - by).abs() < 1e-3, "{l},{t} vs {bx},{by}");
        assert!((r - (bx + bw)).abs() < 1e-3 && (b - (by + bh)).abs() < 1e-3);
        // …and they join the way `place` joins tiles: the inner edge sits on
        // a whole device pixel and the near child reaches one past it, so no
        // renderer can find a crack to show the ground through.
        let over = 1.0 / dpr as f32;
        let mut xs: Vec<f32> = mine.iter().map(|p| p.x).collect();
        xs.sort_by(f32::total_cmp);
        xs.dedup();
        assert_eq!(xs.len(), 2, "{xs:?}");
        let inner = f64::from(xs[1]) * dpr;
        assert!((inner - inner.round()).abs() < 1e-3, "{} is not a device pixel", xs[1]);
        let near_right = mine
            .iter()
            .filter(|p| (p.x - xs[0]).abs() < 1e-3)
            .map(|p| p.x + p.w)
            .fold(f32::MIN, f32::max);
        assert!((near_right - (xs[1] + over)).abs() < 1e-3, "{near_right} vs {}", xs[1]);
    }

    #[test]
    fn a_parent_is_preferred_to_the_children_when_both_are_in_hand() {
        let mut v = MapView::default();
        v.resize(400.0, 600.0);
        v.open(51.5, -0.12);
        let (tx, ty) = v.wanted()[0];
        let (par, _) = ancestor(v.z, tx, ty, 1);
        let mut held: HashSet<_> = children(v.z, tx, ty).iter().map(|c| c.0).collect();
        held.insert(par);
        let plan = v.layout(3.0, &|k| held.contains(&k));
        // One picture, not four: the parent covers the box on its own and is
        // one draw rather than four, and it can never be short a quarter.
        assert_eq!(plan.rows.len(), 1);
        assert_eq!(plan.rows[0].key, par);
    }

    // ------------------------------------------------------- the wish-list

    #[test]
    fn the_prefetch_covers_the_level_drawn_and_the_one_either_side_of_it() {
        let mut v = MapView::default();
        v.resize(400.0, 600.0);
        v.open(51.5, -0.12);
        let want = v.prefetch();
        let levels: HashSet<u32> = want.iter().map(|k| k.0).collect();
        assert_eq!(
            levels,
            [v.z - 1, v.z, v.z + 1].into_iter().collect::<HashSet<_>>()
        );
        // Nothing twice.
        assert_eq!(want.iter().copied().collect::<HashSet<_>>().len(), want.len());
        // The level on screen leads, whole; then the level out, then the
        // level in — the order the budget is spent in.
        let vis: HashSet<(u32, i64, i64)> = v.wanted().iter().map(|&(x, y)| v.key(x, y)).collect();
        assert_eq!(
            want.iter().take(vis.len()).copied().collect::<HashSet<_>>(),
            vis
        );
        let first = |z: u32| want.iter().position(|k| k.0 == z).unwrap();
        assert!(first(v.z) < first(v.z - 1), "the view must come first");
        assert!(first(v.z - 1) < first(v.z + 1), "the level out is a quarter the bill");
    }

    #[test]
    fn the_fetches_go_out_from_the_middle_of_the_view_outwards() {
        let mut v = MapView::default();
        v.resize(400.0, 600.0);
        v.open(51.5, -0.12);
        let want = v.prefetch();
        for z in [v.z - 1, v.z, v.z + 1] {
            let mut last = -1.0f64;
            let mut n = 0;
            for &(_, kx, ky) in want.iter().filter(|k| k.0 == z) {
                let d = v.near(z, kx, ky);
                assert!(d >= last - 1e-9, "level {z}: {d} came after {last}");
                last = d;
                n += 1;
            }
            assert!(n > 0, "level {z} empty");
        }
        // And the very first tile asked for is the one the middle of the
        // screen is standing on — where a pinch magnifies from.
        let (_, kx, ky) = want[0];
        let mid = v.wanted().into_iter().min_by(|a, b| {
            v.near(v.z, a.0, a.1).total_cmp(&v.near(v.z, b.0, b.1))
        });
        assert_eq!((kx, ky), mid.unwrap());
    }

    #[test]
    fn the_queue_is_bounded_and_goes_out_a_few_at_a_time() {
        let mut v = MapView::default();
        // A view big enough that the wish-list overruns the budget, so the
        // trim is the thing being measured and not an accident of the size.
        v.resize(1200.0, 1600.0);
        v.open(51.5, -0.12);
        assert!(v.prefetch().len() > PREFETCH_BUDGET, "the fixture is too small");
        v.refill();
        assert_eq!(v.queue.len(), PREFETCH_BUDGET);
        // What is queued is the head of the wish-list, in its order.
        assert_eq!(
            v.queue.iter().copied().collect::<Vec<_>>(),
            v.prefetch()[..PREFETCH_BUDGET].to_vec()
        );
        // Only LANES go to the engine at once, in that order …
        let mut sent = Vec::new();
        while let Some(k) = v.next_fetch() {
            sent.push(k);
        }
        assert_eq!(sent.len(), LANES);
        assert_eq!(sent, v.prefetch()[..LANES].to_vec());
        assert_eq!(v.asked.len(), LANES);
        // … and the next only once one of them has answered.
        assert!(v.next_fetch().is_none());
        v.asked.remove(&sent[0]);
        assert_eq!(v.next_fetch(), Some(v.prefetch()[LANES]));
    }

    #[test]
    fn a_tile_already_with_the_engine_is_not_asked_for_twice() {
        let mut v = MapView::default();
        v.resize(400.0, 600.0);
        v.open(51.5, -0.12);
        v.refill();
        let dup = v.queue[0];
        v.asked.insert(dup);
        v.refill();
        assert!(!v.queue.contains(&dup));
        // …nor is one the view is showing a hole for and has already sent.
        v.want_now(&[dup]);
        assert!(!v.queue.contains(&dup));
    }

    #[test]
    fn what_the_view_is_missing_now_goes_to_the_head_of_the_queue() {
        let mut v = MapView::default();
        v.resize(400.0, 600.0);
        v.open(51.5, -0.12);
        v.refill();
        // A tile from the far end of the wish-list — imagery for a gesture
        // not yet made — becomes visible: it must jump the whole queue, not
        // wait behind forty tiles nobody is looking at.
        let far = *v.queue.back().unwrap();
        let was = v.queue.len();
        v.want_now(&[far]);
        assert_eq!(v.queue[0], far);
        assert_eq!(v.queue.len(), was, "moved up, not asked for twice");
    }

    #[test]
    fn a_level_the_fingers_pinched_past_is_dropped_from_the_queue() {
        let mut v = MapView::default();
        v.resize(400.0, 600.0);
        v.open(51.5, -0.12);
        v.refill();
        let was = v.z;
        assert!(v.queue.iter().any(|k| k.0 == was - 1));
        // Two levels in. Whatever was queued for the level BELOW where the
        // view started is of no use to anybody now, and the fresh queue
        // simply does not contain it — the queue is replaced, not added to,
        // which is the whole of the cancelling.
        v.zoom(2, 200.0, 300.0);
        v.refill();
        assert_eq!(v.z, was + 2);
        assert!(!v.queue.iter().any(|k| k.0 == was - 1));
        assert!(v.queue.iter().all(|k| k.0 + 1 >= v.z && k.0 <= v.z + 1));
    }

    #[test]
    fn the_wish_list_stops_where_the_renderer_does() {
        for (z, levels) in [(Z_MIN, [Z_MIN, Z_MIN + 1].as_slice()), (Z_MAX, &[Z_MAX - 1, Z_MAX])] {
            let mut v = MapView::default();
            v.resize(400.0, 600.0);
            v.open(51.5, -0.12);
            v.z = z;
            v.clamp();
            let got: HashSet<u32> = v.prefetch().iter().map(|k| k.0).collect();
            assert_eq!(got, levels.iter().copied().collect::<HashSet<_>>(), "at {z}");
        }
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
