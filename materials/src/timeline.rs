use crate::{
    geometry::Die,
    physics::{Pose, SCALE},
    Object,
};
use glam::{Mat3, Quat, Vec3};
use rapier3d::prelude::*;

#[derive(Clone, Copy, Debug)]
pub struct Item {
    pub object: Object,
    pub die: Die,
    pub face: u32,
}
#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}
impl Rect {
    fn valid(self) -> bool {
        [self.left, self.top, self.right, self.bottom]
            .iter()
            .all(|v| v.is_finite() && v.abs() < 200.)
            && self.right > self.left
            && self.bottom > self.top
    }
    fn contains(self, x: f32, z: f32, r: f32) -> bool {
        x > self.left - r && x < self.right + r && z > self.top - r && z < self.bottom + r
    }
}
pub struct Playback {
    pub frames: Vec<Vec<Pose>>,
    pub contacts: usize,
    pub return_frame: usize,
}
pub fn view_rotation(rotation: Quat) -> Quat {
    Quat::from_rotation_x(std::f32::consts::FRAC_PI_2) * rotation
}

// A local symmetry changes only which existing face is presented, not the hull or trajectory.
fn symmetry(item: Item, rotation: Quat) -> Option<Quat> {
    if item.object == Object::Coin {
        if item.face > 1 || (rotation * Vec3::Z).y.abs() < 0.96 {
            return None;
        }
        let desired = if item.face == 0 { 1. } else { -1. };
        let mut correction = if (rotation * Vec3::Z).y * desired > 0. {
            Quat::IDENTITY
        } else {
            Quat::from_rotation_x(std::f32::consts::PI)
        };
        if (view_rotation(rotation * correction) * Vec3::Y).y < 0. {
            correction *= Quat::from_rotation_z(std::f32::consts::PI);
        }
        return Some(correction);
    }
    let hull = item.die.hull();
    let source = Vec3::from_slice(&hull.faces.get(item.face.checked_sub(1)? as usize)?[0][..3]);
    let sign = if item.die == Die::D4 { -1. } else { 1. };
    let target = hull
        .faces
        .iter()
        .map(|f| Vec3::from_slice(&f[0][..3]))
        .max_by(|a, b| ((rotation * *a).y * sign).total_cmp(&((rotation * *b).y * sign)))?;
    if (rotation * target).y * sign < 0.96 {
        return None;
    }
    let basis = |n: Vec3, v: Vec3| {
        let u = (v - n * v.dot(n)).normalize();
        Mat3::from_cols(u, n.cross(u), n)
    };
    let a = *hull
        .vertices
        .iter()
        .find(|v| v.cross(source).length() > 0.1)?;
    let up = Vec3::from_slice(&hull.faces[item.face as usize - 1][2][..3]);
    let mut best = None;
    let mut score = f32::NEG_INFINITY;
    for &b in &hull.vertices {
        if b.cross(target).length() < 0.1 {
            continue;
        }
        let q = Quat::from_mat3(&(basis(target, b) * basis(source, a).transpose())).normalize();
        if hull
            .vertices
            .iter()
            .all(|v| hull.vertices.iter().any(|w| (q * *v - *w).length() < 0.002))
        {
            let alignment = (view_rotation(rotation * q) * up).y;
            if alignment > score {
                score = alignment;
                best = Some(q);
            }
        }
    }
    best
}
fn collider(item: Item) -> Option<ColliderBuilder> {
    if item.object == Object::Coin {
        let mut points = Vec::with_capacity(128);
        for i in 0..64 {
            let angle = i as f32 * std::f32::consts::TAU / 64.;
            for z in [-0.104, 0.104] {
                points.push(Vector::new(angle.cos() * 0.972, angle.sin() * 0.972, z) * SCALE);
            }
        }
        ColliderBuilder::convex_hull(&points)
    } else {
        ColliderBuilder::convex_hull(
            &item
                .die
                .hull()
                .vertices
                .iter()
                .map(|v| Vector::new(v.x, v.y, v.z) * SCALE)
                .collect::<Vec<_>>(),
        )
    }
}
fn clear(bounds: Rect, obstacles: &[Rect], x: f32, z: f32, r: f32) -> bool {
    x >= bounds.left + r
        && x <= bounds.right - r
        && z >= bounds.top + r
        && z <= bounds.bottom - r
        && !obstacles.iter().any(|o| o.contains(x, z, r))
}
// Bounded grid routing keeps the post-rest placement away from message rectangles.
fn route(bounds: Rect, start: Vec3, end: Vec3, valid: impl Fn(Vec3) -> bool) -> Option<Vec<Vec3>> {
    const N: usize = 48;
    let at = |i: usize| {
        Vec3::new(
            bounds.left + (i % N) as f32 / (N - 1) as f32 * (bounds.right - bounds.left),
            start.y,
            bounds.top + (i / N) as f32 / (N - 1) as f32 * (bounds.bottom - bounds.top),
        )
    };
    let segment = |a: Vec3, b: Vec3| {
        let steps = ((a - b).length() / 0.1).ceil().max(1.) as usize;
        (0..=steps).all(|i| valid(a.lerp(b, i as f32 / steps as f32)))
    };
    if segment(start, end) {
        return Some(vec![start, end]);
    }
    let near = |p: Vec3| {
        let mut candidates = (0..N * N).collect::<Vec<_>>();
        candidates.sort_by(|a, b| {
            at(*a)
                .distance_squared(p)
                .total_cmp(&at(*b).distance_squared(p))
        });
        candidates.into_iter().take(64).find(|i| segment(p, at(*i)))
    };
    let first = near(start)?;
    let last = near(end)?;
    let mut parent = vec![usize::MAX; N * N];
    parent[first] = first;
    let mut queue = std::collections::VecDeque::from([first]);
    while let Some(i) = queue.pop_front() {
        if i == last {
            break;
        }
        for (dx, dz) in [
            (1, 0),
            (-1, 0),
            (0, 1),
            (0, -1),
            (1, 1),
            (1, -1),
            (-1, 1),
            (-1, -1),
        ] {
            let x = (i % N) as i32 + dx;
            let z = (i / N) as i32 + dz;
            if x < 0 || z < 0 || x >= N as i32 || z >= N as i32 {
                continue;
            }
            let j = z as usize * N + x as usize;
            if parent[j] == usize::MAX && segment(at(i), at(j)) {
                parent[j] = i;
                queue.push_back(j);
            }
        }
    }
    if parent[last] == usize::MAX {
        return None;
    }
    let mut points = vec![end];
    let mut i = last;
    loop {
        points.push(at(i));
        if i == first {
            break;
        }
        i = parent[i];
    }
    points.push(start);
    points.reverse();
    let mut simple = vec![start];
    let mut i = 0;
    while i + 1 < points.len() {
        let j = (i + 1..points.len())
            .rev()
            .find(|j| segment(points[i], points[*j]))?;
        simple.push(points[j]);
        i = j;
    }
    Some(simple)
}
fn rounded_path(path: &[Vec3], valid: impl Fn(Vec3) -> bool) -> Vec<Vec3> {
    let mut out = vec![path[0]];
    for p in path.windows(3) {
        let mut radius = p[0].distance(p[1]).min(p[1].distance(p[2])) * 0.35;
        let mut corner = None;
        for _ in 0..8 {
            let a = p[1] + (p[0] - p[1]).normalize() * radius;
            let b = p[1] + (p[2] - p[1]).normalize() * radius;
            let steps = (radius * 2. / 0.04).ceil().max(8.) as usize;
            let curve = (0..=steps)
                .map(|i| {
                    let t = i as f32 / steps as f32;
                    a.lerp(p[1], t).lerp(p[1].lerp(b, t), t)
                })
                .collect::<Vec<_>>();
            if curve.iter().all(|p| valid(*p)) {
                corner = Some(curve);
                break;
            }
            radius *= 0.5;
        }
        if let Some(curve) = corner {
            out.extend(curve);
        } else {
            out.push(p[1]);
        }
    }
    out.push(*path.last().unwrap());
    out
}
fn placement(path: &[Vec3]) -> Vec<Vec3> {
    let mut distances = vec![0.];
    for p in path.windows(2) {
        distances.push(distances.last().unwrap() + p[0].distance(p[1]));
    }
    let length = *distances.last().unwrap();
    if length < 0.001 {
        return vec![];
    }
    let count = (length * 7.).ceil().clamp(24., 180.) as usize;
    (1..=count)
        .map(|i| {
            let t = i as f32 / count as f32;
            let distance = length * t * t * t * (t * (t * 6. - 15.) + 10.);
            let j = distances
                .partition_point(|d| *d < distance)
                .clamp(1, path.len() - 1);
            path[j - 1].lerp(
                path[j],
                ((distance - distances[j - 1])
                    / (distances[j] - distances[j - 1]).max(f32::EPSILON))
                .clamp(0., 1.),
            )
        })
        .collect()
}
pub fn record(
    items: &[Item],
    bounds: Rect,
    obstacles: &[Rect],
    targets: &[Vec3],
    outgoing: bool,
    seed: u32,
) -> Option<Playback> {
    record_from(items, bounds, obstacles, targets, outgoing, seed, None)
}
fn record_from(
    items: &[Item],
    bounds: Rect,
    obstacles: &[Rect],
    targets: &[Vec3],
    outgoing: bool,
    seed: u32,
    starts: Option<&[Vec3]>,
) -> Option<Playback> {
    if items.is_empty()
        || items.len() > 6
        || items.len() != targets.len()
        || !bounds.valid()
        || obstacles.len() > 64
        || obstacles.iter().any(|r| !r.valid())
        || targets.iter().any(|p| !p.is_finite())
        || starts.is_some_and(|s| {
            s.len() != items.len()
                || s.iter()
                    .any(|p| !p.is_finite() || !clear(bounds, &[], p.x, p.z, 0.78))
        })
    {
        return None;
    }
    for (item, target) in items.iter().zip(targets) {
        crate::presentation::result_pose(item.object, item.die, item.face, 1.)?;
        if !clear(bounds, obstacles, target.x, target.z, 0.78) {
            return None;
        }
    }
    for attempt in 0..4 {
        if let Some(playback) = simulate(
            items,
            bounds,
            obstacles,
            targets,
            outgoing,
            seed.wrapping_add(attempt * 971),
            starts,
        ) {
            return Some(playback);
        }
    }
    None
}
fn launch_landing(bounds: Rect, obstacles: &[Rect], index: usize, count: usize) -> Option<Vec3> {
    let columns = count.min(3);
    let rows = count.div_ceil(columns);
    let center = Vec3::new(
        (bounds.left + bounds.right) * 0.5
            + ((index % columns) as f32 - (columns - 1) as f32 * 0.5) * 2.,
        0.,
        bounds.top
            + (bounds.bottom - bounds.top) * 0.48
            + ((index / columns) as f32 - (rows - 1) as f32 * 0.5) * 2.4,
    );
    (0..=24)
        .flat_map(|z| {
            (0..=16).map(move |x| {
                Vec3::new(
                    bounds.left + (bounds.right - bounds.left) * x as f32 / 16.,
                    0.,
                    bounds.top + (bounds.bottom - bounds.top) * z as f32 / 24.,
                )
            })
        })
        .filter(|p| clear(bounds, obstacles, p.x, p.z, 1.1))
        .min_by(|a, b| {
            a.distance_squared(center)
                .total_cmp(&b.distance_squared(center))
        })
}
fn simulate(
    items: &[Item],
    bounds: Rect,
    obstacles: &[Rect],
    targets: &[Vec3],
    outgoing: bool,
    seed: u32,
    starts: Option<&[Vec3]>,
) -> Option<Playback> {
    let mut bodies = RigidBodySet::new();
    let mut colliders = ColliderSet::new();
    colliders.insert(
        ColliderBuilder::cuboid(
            (bounds.right - bounds.left) / 2. + 1.,
            0.1,
            (bounds.bottom - bounds.top) / 2. + 1.,
        )
        .translation(Vector::new(
            (bounds.left + bounds.right) / 2.,
            -1.35,
            (bounds.top + bounds.bottom) / 2.,
        )),
    );
    for o in obstacles.iter().copied().chain([
        Rect {
            left: bounds.left - 1.,
            right: bounds.left,
            top: bounds.top - 1.,
            bottom: bounds.bottom + 1.,
        },
        Rect {
            left: bounds.right,
            right: bounds.right + 1.,
            top: bounds.top - 1.,
            bottom: bounds.bottom + 1.,
        },
        Rect {
            left: bounds.left - 1.,
            right: bounds.right + 1.,
            top: bounds.top - 1.,
            bottom: bounds.top,
        },
        Rect {
            left: bounds.left - 1.,
            right: bounds.right + 1.,
            top: bounds.bottom,
            bottom: bounds.bottom + 1.,
        },
    ]) {
        colliders.insert(
            ColliderBuilder::cuboid((o.right - o.left) / 2., 4., (o.bottom - o.top) / 2.)
                .translation(Vector::new(
                    (o.left + o.right) / 2.,
                    2.,
                    (o.top + o.bottom) / 2.,
                ))
                .friction(0.1),
        );
    }
    let mut handles = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let v = seed
            .wrapping_add(i as u32 * 7919)
            .wrapping_mul(747796405)
            .rotate_left(13) as f32
            / u32::MAX as f32;
        // A second stream so the three spin axes and the lateral fan never move in lockstep.
        let w = seed
            .wrapping_add(i as u32 * 6151)
            .wrapping_mul(2_654_435_761)
            .rotate_left(7) as f32
            / u32::MAX as f32;
        let fan = if i % 2 == 0 { 1. } else { -1. };
        let coin = item.object == Object::Coin;
        let start = starts.map_or(targets[i], |s| s[i]);
        let landing = launch_landing(bounds, obstacles, i, items.len()).unwrap_or(targets[i]);
        let direction = Vec3::new(landing.x - start.x, 0., landing.z - start.z);
        let travel = if coin {
            (direction * 1.15).clamp_length_max(18.)
        } else {
            direction.normalize_or_zero() * (direction.length() * 24.).sqrt().min(19.)
        };
        let rotation = if starts.is_some() {
            let preview = crate::presentation::result_pose(
                item.object,
                item.die,
                if coin { 0 } else { 1 },
                1.,
            )?;
            (Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2) * preview.rotation)
                .to_scaled_axis()
        } else if coin {
            Vec3::new(-std::f32::consts::FRAC_PI_2, 0., 0.)
        } else {
            Vec3::new(0.6 + v, 0.2 + w * 2.4, 1.1 + v * 1.7)
        };
        let height = if starts.is_some() {
            let orientation = Quat::from_scaled_axis(rotation);
            let support = if coin {
                -0.104 * SCALE
            } else {
                item.die
                    .hull()
                    .vertices
                    .iter()
                    .map(|v| (orientation * *v * SCALE).y)
                    .fold(f32::INFINITY, f32::min)
            };
            -1.25 - support + 0.012
        } else if coin {
            -1.16
        } else {
            0.5 + i as f32 * 0.14
        };
        let h = bodies.insert(
            RigidBodyBuilder::dynamic()
                .translation(Vector::new(start.x, height, start.z))
                .rotation(Vector::new(rotation.x, rotation.y, rotation.z))
                .enabled_translations(!coin || starts.is_some(), true, !coin || starts.is_some())
                .enabled_rotations(true, !coin, !coin)
                .linvel(Vector::new(
                    if starts.is_some() {
                        travel.x
                    } else if coin {
                        0.
                    } else if outgoing {
                        -7.5 - v * 3.5
                    } else {
                        7.5 + v * 3.5
                    },
                    if coin {
                        9.
                    } else if starts.is_some() {
                        3.2
                    } else {
                        2.4 + w * 1.6
                    },
                    if starts.is_some() {
                        travel.z
                    } else if coin {
                        0.
                    } else {
                        fan * (4. + w * 2.4)
                    },
                ))
                .angvel(if coin {
                    Vector::new(29. + v * 3., 0., 0.)
                } else if starts.is_some() {
                    Vector::new(travel.z * 1.2, 1.5, -travel.x * 1.2)
                } else {
                    Vector::new(
                        -9. - v * 8.,
                        (w - 0.5) * 12.,
                        (if outgoing { 8. } else { -8. }) + (v - 0.5) * 14.,
                    )
                })
                .linear_damping(0.25)
                .angular_damping(if coin { 0.3 } else { 0.42 })
                .ccd_enabled(true),
        );
        if coin {
            let activation = bodies[h].activation_mut();
            activation.normalized_linear_threshold = 0.08;
            activation.angular_threshold = 0.4;
        }
        colliders.insert_with_parent(
            collider(*item)?
                .restitution(if coin { 0.22 } else { 0.2 })
                .friction(0.85)
                .density(1.),
            h,
            &mut bodies,
        );
        handles.push(h);
    }
    let mut pipeline = PhysicsPipeline::new();
    let mut islands = IslandManager::new();
    let mut broad = BroadPhaseBvh::new();
    let mut narrow = NarrowPhase::new();
    let mut impulse = ImpulseJointSet::new();
    let mut multi = MultibodyJointSet::new();
    let mut ccd = CCDSolver::new();
    let params = IntegrationParameters {
        dt: 1. / 240.,
        max_ccd_substeps: 4,
        num_solver_iterations: 8,
        contact_recycling: false,
        ..Default::default()
    };
    let mut frames = Vec::new();
    let mut contacts = 0;
    for step in 0..2881 {
        if step % 4 == 0 {
            frames.push(
                handles
                    .iter()
                    .map(|h| {
                        let b = &bodies[*h];
                        let p = b.translation();
                        let q = b.rotation();
                        Pose {
                            position: Vec3::new(p.x, p.y, p.z),
                            rotation: Quat::from_xyzw(q.x, q.y, q.z, q.w),
                        }
                    })
                    .collect::<Vec<_>>(),
            );
        }
        if handles.iter().all(|h| bodies[*h].is_sleeping()) {
            break;
        }
        pipeline.step(
            Vector::new(
                0.,
                if items[0].object == Object::Coin {
                    -18.5
                } else {
                    -30.
                },
                0.,
            ),
            &params,
            &mut islands,
            &mut broad,
            &mut narrow,
            &mut bodies,
            &mut colliders,
            &mut impulse,
            &mut multi,
            &mut ccd,
            &(),
            &(),
        );
        contacts += narrow
            .contact_pairs()
            .filter(|p| p.has_any_active_contact())
            .count();
    }
    if !handles.iter().all(|h| bodies[*h].is_sleeping()) {
        return None;
    }
    let last = frames.last()?.clone();
    let adjustments = items
        .iter()
        .zip(&last)
        .map(|(item, p)| symmetry(*item, p.rotation))
        .collect::<Option<Vec<_>>>()?;
    for frame in &mut frames {
        for (p, q) in frame.iter_mut().zip(&adjustments) {
            p.rotation *= *q;
        }
    }
    let rest = frames.last()?.clone();
    if starts.is_some() && items[0].object != Object::Coin {
        let moving = frames
            .iter()
            .rposition(|frame| {
                frame.iter().zip(&rest).any(|(a, b)| {
                    a.position.distance(b.position) > 0.003
                        || a.rotation.angle_between(b.rotation) > 0.01
                })
            })
            .unwrap_or(0);
        frames.truncate((moving + 7).min(frames.len()));
        frames.push(rest.clone());
    }
    let hold = if starts.is_some() && items[0].object != Object::Coin {
        18
    } else {
        51
    };
    for _ in 0..hold {
        frames.push(rest.clone());
    }
    let return_frame = frames.len() - 1;
    let mut placed = rest.clone();
    let mut routes = Vec::new();
    for i in 0..items.len() {
        let start = placed[i].position;
        let end = Vec3::new(targets[i].x, start.y, targets[i].z);
        let position = |p: Pose| {
            rapier3d::math::Pose::from_parts(
                Vector::new(p.position.x, p.position.y, p.position.z),
                Rotation::from_xyzw(p.rotation.x, p.rotation.y, p.rotation.z, p.rotation.w),
            )
        };
        let blocked = obstacles
            .iter()
            .map(|o| {
                ColliderBuilder::cuboid((o.right - o.left) / 2., 4., (o.bottom - o.top) / 2.)
                    .translation(Vector::new(
                        (o.left + o.right) / 2.,
                        2.,
                        (o.top + o.bottom) / 2.,
                    ))
                    .build()
            })
            .collect::<Vec<_>>();
        let shape = collider(items[i])?.build();
        let extent = if items[i].object == Object::Coin {
            glam::Vec2::splat(0.681)
        } else {
            items[i]
                .die
                .hull()
                .vertices
                .iter()
                .map(|v| {
                    let p = placed[i].rotation * *v * SCALE;
                    glam::Vec2::new(p.x.abs(), p.z.abs())
                })
                .fold(glam::Vec2::ZERO, |a, b| a.max(b))
        };
        let pose = placed[i];
        let valid = |p: Vec3| {
            if p.x < bounds.left + extent.x - 0.005
                || p.x > bounds.right - extent.x + 0.005
                || p.z < bounds.top + extent.y - 0.005
                || p.z > bounds.bottom - extent.y + 0.005
            {
                return false;
            }
            let at = position(Pose {
                position: p,
                ..pose
            });
            blocked.iter().all(|b| {
                rapier3d::parry::query::contact(&at, shape.shape(), b.position(), b.shape(), 0.)
                    .is_ok_and(|c| c.is_none_or(|c| c.dist >= -0.005))
            })
        };
        let path = rounded_path(&route(bounds, start, end, valid)?, valid);
        let mut points = vec![start];
        for point in placement(&path) {
            let from = placed[i].position;
            let steps = (from.distance(point) / 0.04).ceil().max(1.) as usize;
            if !(0..=steps).all(|j| valid(from.lerp(point, j as f32 / steps as f32))) {
                return None;
            }
            placed[i].position = point;
            points.push(point);
        }
        routes.push(points);
    }
    let shapes = items
        .iter()
        .map(|i| Some(collider(*i)?.build()))
        .collect::<Option<Vec<_>>>()?;
    let pose_at = |p: Pose| {
        rapier3d::math::Pose::from_parts(
            Vector::new(p.position.x, p.position.y, p.position.z),
            Rotation::from_xyzw(p.rotation.x, p.rotation.y, p.rotation.z, p.rotation.w),
        )
    };
    let walls = obstacles
        .iter()
        .map(|o| {
            ColliderBuilder::cuboid((o.right - o.left) / 2., 4., (o.bottom - o.top) / 2.)
                .translation(Vector::new(
                    (o.left + o.right) / 2.,
                    2.,
                    (o.top + o.bottom) / 2.,
                ))
                .build()
        })
        .collect::<Vec<_>>();
    let count = routes.iter().map(Vec::len).max()?.max(2) - 1;
    let mut coordinated = None;
    for attempt in 0..64u32 {
        let mut candidate = Vec::new();
        let mut clear = true;
        for step in 1..=count * 8 {
            let t = step as f32 / (count * 8) as f32;
            let frame = routes
                .iter()
                .enumerate()
                .map(|(i, path)| {
                    let exponent = if attempt == 0 {
                        1.
                    } else {
                        0.65 + ((attempt.wrapping_mul(7919) ^ (i as u32 + 1).wrapping_mul(104729))
                            .wrapping_mul(2654435761)
                            % 1024) as f32
                            / 1024.
                            * 1.1
                    };
                    let at = t.powf(exponent) * (path.len() - 1) as f32;
                    let a = (at as usize).min(path.len() - 1);
                    let b = (a + 1).min(path.len() - 1);
                    Pose {
                        position: path[a].lerp(path[b], at - a as f32),
                        ..rest[i]
                    }
                })
                .collect::<Vec<_>>();
            for i in 0..items.len() {
                let at = pose_at(frame[i]);
                let collides = |other: &Collider| {
                    rapier3d::parry::query::contact(
                        &at,
                        shapes[i].shape(),
                        other.position(),
                        other.shape(),
                        0.,
                    )
                    .map_or(true, |c| c.is_some_and(|c| c.dist < -0.005))
                };
                if walls.iter().any(collides)
                    || (0..i).any(|j| {
                        rapier3d::parry::query::contact(
                            &at,
                            shapes[i].shape(),
                            &pose_at(frame[j]),
                            shapes[j].shape(),
                            0.,
                        )
                        .map_or(true, |c| c.is_some_and(|c| c.dist < -0.005))
                    })
                {
                    clear = false;
                    break;
                }
            }
            if !clear {
                break;
            }
            if step % 8 == 0 {
                candidate.push(frame);
            }
        }
        if clear {
            coordinated = Some(candidate);
            break;
        }
    }
    frames.extend(coordinated?);
    if frames.len() > 1400 {
        return None;
    }
    Some(Playback {
        frames,
        contacts,
        return_frame,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn placement_rounds_bends_without_stopping_or_crossing_obstacles() {
        let path = [Vec3::ZERO, Vec3::new(4., 0., 0.), Vec3::new(4., 0., 4.)];
        let valid = |p: Vec3| !(p.x < 3. && p.z > 1.);
        let curve = rounded_path(&path, valid);
        let frames = placement(&curve);
        assert!(frames.iter().all(|p| valid(*p)));
        assert!(curve.iter().any(|p| p.x < 4. && p.z > 0.));
        assert!(frames.last().unwrap().distance(path[2]) < 0.001);
        let steps = frames.windows(2).map(|p| p[1] - p[0]).collect::<Vec<_>>();
        let middle = &steps[steps.len() / 4..steps.len() * 3 / 4];
        assert!(middle.iter().all(|v| v.length() > 0.08));
        assert!(middle
            .windows(2)
            .all(|v| v[0].normalize().dot(v[1].normalize()) > 0.9));
        let tight = |p: Vec3| p.z <= 0.05 || p.x >= 3.95;
        assert!(rounded_path(&path, tight).iter().all(|p| tight(*p)));
    }
    #[test]
    fn face_mapping_is_a_shape_symmetry() {
        for die in Die::ALL {
            for natural in 1..=die.sides() {
                let q = crate::presentation::result_pose(Object::Die, die, natural as u32, 1.)
                    .unwrap()
                    .rotation;
                let q = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2) * q;
                if die == Die::D4 {
                    continue;
                }
                for face in 1..=die.sides() {
                    let item = Item {
                        object: Object::Die,
                        die,
                        face: face as u32,
                    };
                    let s = symmetry(item, q).unwrap();
                    let n = Vec3::from_slice(&die.hull().faces[face - 1][0][..3]);
                    assert!((q * s * n).y > 0.99);
                }
            }
        }
    }
    #[test]
    fn preview_physics_leaves_the_tray_before_the_return() {
        let bounds = Rect {
            left: 0.,
            top: 0.,
            right: 14.,
            bottom: 24.,
        };
        let obstacles = [Rect {
            left: 9.,
            top: 7.,
            right: 13.,
            bottom: 14.,
        }];
        let start = Vec3::new(7., 0., 20.);
        for object in [Object::Coin, Object::Die] {
            let item = Item {
                object,
                die: Die::D6,
                face: if object == Object::Coin { 0 } else { 3 },
            };
            let scene = record_from(
                &[item],
                bounds,
                &obstacles,
                &[Vec3::new(11., 0., 18.)],
                true,
                123,
                Some(&[start]),
            )
            .expect("preview trajectory");
            let resting = scene.frames[scene.return_frame][0].position;
            assert!(
                scene.frames[30][0].position.z < 18.,
                "visible motion before return"
            );
            assert!(clear(bounds, &obstacles, resting.x, resting.z, 0.78));
            assert!((resting.x - 7.).abs() < 3., "central landing");
            assert!(resting.z < 17., "clear of the preview tray");
        }
    }
    #[test]
    fn preview_launch_starts_in_its_actual_position_and_keeps_the_result_target() {
        for sides in [0., 6.] {
            let face = if sides == 0. { 0. } else { 3. };
            let packed = [14., 24., 1., 0., 1., 123., sides, face, 11., 10., 7., 20.];
            let scene = record_packed(&packed).expect("source-aware launch");
            assert_eq!(scene[1], 7.);
            assert_eq!(scene[3], 20.);
            let last = &scene[scene.len() - 7..];
            assert!((last[0] - 11.).abs() < 0.01);
            assert!((last[2] - 10.).abs() < 0.01);
            assert!(scene[1..].as_chunks::<7>().0.iter().any(|p| p[2] < 18.));
            assert!(record_packed(&packed[..10]).is_some());
            assert!(record_packed(&packed[..11]).is_none());
            let mut invalid = packed;
            invalid[10] = f32::NAN;
            assert!(record_packed(&invalid).is_none());
            invalid[10] = -1.;
            assert!(record_packed(&invalid).is_none());
            invalid = packed;
            invalid[2] = f32::MAX;
            assert!(record_packed(&invalid).is_none());
        }
    }
    #[test]
    fn enlarged_dice_fit_a_phone_and_return_together() {
        let items = [Die::D4, Die::D6, Die::D8, Die::D10, Die::D12, Die::D20].map(|die| Item {
            object: Object::Die,
            die,
            face: 1,
        });
        let targets = (0..6)
            .map(|i| Vec3::new(2.5 + (i % 3) as f32 * 1.85, 0., 11. + (i / 3) as f32 * 2.6))
            .collect::<Vec<_>>();
        let scene = record(
            &items,
            Rect {
                left: 0.,
                top: 0.,
                right: 7.2,
                bottom: 16.,
            },
            &[Rect {
                left: 0.,
                top: 4.,
                right: 3.8,
                bottom: 5.4,
            }],
            &targets,
            true,
            47,
        )
        .expect("Enlarged dice must have a joint return on a phone");
        let returning = &scene.frames[scene.return_frame + 1..];
        assert!(returning.windows(2).any(|p| p[0]
            .iter()
            .zip(&p[1])
            .all(|(a, b)| a.position.distance(b.position) > 0.001)));
        assert!(scene.frames.len() <= 720);
    }
    #[test]
    fn all_dice_shapes_launch_from_a_two_row_preview() {
        let items = [Die::D4, Die::D6, Die::D8, Die::D10, Die::D12, Die::D20].map(|die| Item {
            object: Object::Die,
            die,
            face: 1,
        });
        let starts = (0..6)
            .map(|i| Vec3::new(4. + (i % 3) as f32 * 2., 0., 20. + (i / 3) as f32 * 2.4))
            .collect::<Vec<_>>();
        let targets = (0..6)
            .map(|i| Vec3::new(7. + (i % 3) as f32 * 2., 0., 17. + (i / 3) as f32 * 2.4))
            .collect::<Vec<_>>();
        let scene = record_from(
            &items,
            Rect {
                left: 0.,
                top: 0.,
                right: 14.,
                bottom: 26.,
            },
            &[],
            &targets,
            true,
            47,
            Some(&starts),
        )
        .expect("six-shape launch");
        for (i, start) in starts.iter().enumerate() {
            assert!(scene.frames[30][i].position.z < start.z - 2.);
            let final_position = scene.frames.last().unwrap()[i].position;
            assert!(
                (final_position.x - targets[i].x).abs() < 0.01
                    && (final_position.z - targets[i].z).abs() < 0.01
            );
        }
    }
    #[test]
    fn six_dice_can_return_to_their_message_slots() {
        let items = [Die::D4, Die::D6, Die::D8, Die::D10, Die::D12, Die::D20].map(|die| Item {
            object: Object::Die,
            die,
            face: 1,
        });
        let targets = (0..6)
            .map(|i| Vec3::new(4. + (i % 3) as f32 * 2.4, 0., 12. + (i / 3) as f32 * 2.5))
            .collect::<Vec<_>>();
        let scene = record(
            &items,
            Rect {
                left: 0.,
                top: 0.,
                right: 11.,
                bottom: 18.,
            },
            &[Rect {
                left: 0.,
                top: 3.,
                right: 6.,
                bottom: 5.,
            }],
            &targets,
            true,
            47,
        )
        .expect("six settled shapes with collision-aware placement");
        assert!(scene.contacts > 0);
        let returning = &scene.frames[scene.return_frame + 1..];
        assert!(
            returning.windows(2).any(|pair| pair[0]
                .iter()
                .zip(&pair[1])
                .all(|(a, b)| a.position.distance(b.position) > 0.001)),
            "All six dice must travel concurrently"
        );
        for ((p, t), item) in scene.frames.last().unwrap().iter().zip(targets).zip(items) {
            assert!((p.position.x - t.x).abs() < 0.01 && (p.position.z - t.z).abs() < 0.01);
            let normal = Vec3::from_slice(&item.die.hull().faces[item.face as usize - 1][0][..3]);
            assert!(
                (view_rotation(p.rotation) * normal).z * if item.die == Die::D4 { -1. } else { 1. }
                    > 0.96
            );
        }
    }
    #[test]
    fn stored_results_replay_with_bounded_contacts() {
        let bounds = Rect {
            left: -5.,
            right: 5.,
            top: -7.,
            bottom: 7.,
        };
        let obstacles = [Rect {
            left: -5.,
            right: 1.,
            top: -4.,
            bottom: -2.,
        }];
        for (object, die, face) in [(Object::Die, Die::D6, 5), (Object::Coin, Die::D6, 1)] {
            let item = Item { object, die, face };
            let a = record(
                &[item],
                bounds,
                &obstacles,
                &[Vec3::new(3., 0., 4.)],
                true,
                11,
            )
            .unwrap();
            let b = record(
                &[item],
                bounds,
                &obstacles,
                &[Vec3::new(3., 0., 4.)],
                true,
                11,
            )
            .unwrap();
            assert_eq!(a.frames, b.frames);
            assert!(a.contacts > 0);
            assert!(a.frames.len() <= 1400);
            let end = a.frames.last().unwrap()[0];
            if object == Object::Coin {
                assert!((view_rotation(end.rotation) * -Vec3::Z).z > 0.96);
                assert!((view_rotation(end.rotation) * Vec3::Y).y > 0.96);
                assert!(a.frames.iter().all(|f| (f[0].position.x - 3.).abs() < 0.001
                    && (f[0].position.z - 4.).abs() < 0.001));
                assert!(
                    a.frames
                        .iter()
                        .map(|f| f[0].position.y)
                        .fold(f32::NEG_INFINITY, f32::max)
                        - end.position.y
                        > 1.
                );
            }
            assert!((end.position.x - 3.).abs() < 0.001);
            assert!((end.position.z - 4.).abs() < 0.001);
        }
    }
    #[test]
    fn dice_tumble_and_take_separate_paths_instead_of_moving_in_lockstep() {
        let bounds = Rect {
            left: -5.,
            right: 5.,
            top: -7.,
            bottom: 7.,
        };
        let items = [Die::D6, Die::D20, Die::D8].map(|die| Item {
            object: Object::Die,
            die,
            face: 1,
        });
        let targets = [
            Vec3::new(-2., 0., 2.),
            Vec3::new(0., 0., 2.),
            Vec3::new(2., 0., 2.),
        ];
        let play = record(&items, bounds, &[], &targets, false, 17).unwrap();
        let apex = |i: usize| {
            play.frames
                .iter()
                .map(|f| f[i].position.y)
                .fold(f32::NEG_INFINITY, f32::max)
        };
        let spin = |i: usize| {
            play.frames
                .windows(2)
                .map(|w| w[0][i].rotation.angle_between(w[1][i].rotation))
                .sum::<f32>()
        };
        // Lateral launch must fan, so neighbouring dice never leave along the same line.
        let leave = |i: usize| play.frames[6][i].position.z - play.frames[0][i].position.z;
        for i in 0..items.len() {
            assert!(apex(i) - play.frames.last().unwrap()[i].position.y > 0.8);
            assert!(spin(i) > std::f32::consts::PI);
            assert!(i == 0 || leave(i - 1) * leave(i) < 0.);
        }
    }
}

pub fn record_packed(a: &[f32]) -> Option<Vec<f32>> {
    if !(8..=300).contains(&a.len()) {
        return None;
    }
    if !a.iter().all(|v| v.is_finite()) {
        return None;
    }
    let n = a[2] as usize;
    let m = a[3] as usize;
    if n == 0 || n > 6 || m > 64 {
        return None;
    }
    let base = 6 + n * 4 + m * 4;
    if a.len() != base && a.len() != base + n * 2 {
        return None;
    }
    let mut items = Vec::new();
    let mut targets = Vec::new();
    for v in a[6..6 + n * 4].as_chunks::<4>().0 {
        let coin = v[0] == 0.;
        let die = if coin {
            Die::D6
        } else {
            Die::ALL.into_iter().find(|d| d.sides() as f32 == v[0])?
        };
        items.push(Item {
            object: if coin { Object::Coin } else { Object::Die },
            die,
            face: v[1] as u32,
        });
        targets.push(glam::Vec3::new(v[2], 0., v[3]));
    }
    let rect = |v: &[f32]| Rect {
        left: v[0],
        top: v[1],
        right: v[2],
        bottom: v[3],
    };
    let obstacles = a[6 + n * 4..base]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|v| rect(v))
        .collect::<Vec<_>>();
    let starts = (a.len() != base).then(|| {
        a[base..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|v| Vec3::new(v[0], 0., v[1]))
            .collect::<Vec<_>>()
    });
    let playback = record_from(
        &items,
        rect(&[0., 0., a[0], a[1]]),
        &obstacles,
        &targets,
        a[4] != 0.,
        a[5] as u32,
        starts.as_deref(),
    )?;
    let mut output = Vec::with_capacity(1 + playback.frames.len() * n * 7);
    output.push(playback.return_frame as f32);
    for frame in playback.frames {
        for p in frame {
            output.extend_from_slice(&p.position.to_array());
            output.extend_from_slice(&view_rotation(p.rotation).to_array());
        }
    }
    Some(output)
}
