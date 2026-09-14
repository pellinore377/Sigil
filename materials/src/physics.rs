use crate::{geometry::Die, Object};
use glam::{Quat, Vec3};
use rapier3d::prelude::*;

// Shared by the renderer and collision world: center, half extents, corner radius.
pub const SCALE: f32 = 0.7;
pub const OBSTACLES: [[[f32; 4]; 2]; 7] = [
    [[-2.6, 1., -2.4, 0.], [1.6, 2.25, 0.6, 0.18]],
    [[2.6, 1., 0.2, 0.], [1.6, 2.25, 0.65, 0.18]],
    [[-2.6, 1., 2.7, 0.], [1.6, 2.25, 0.5, 0.18]],
    [[0., 1., 5., 0.], [4.5, 2.25, 0.55, 0.2]],
    [[-4.7, 1., 0., 0.], [0.15, 2.25, 6., 0.08]],
    [[4.7, 1., 0., 0.], [0.15, 2.25, 6., 0.08]],
    [[0., 1., -5.9, 0.], [4.7, 2.25, 0.15, 0.08]],
];
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub position: Vec3,
    pub rotation: Quat,
}
#[derive(Clone)]
pub struct Throw {
    pub frames: Vec<Pose>,
    pub settled: bool,
    pub result: String,
    pub contacts: usize,
    pub obstacle_contacts: usize,
}
impl Throw {
    pub fn sample(&self, phase: f32) -> Pose {
        let p = if phase.is_finite() {
            phase.clamp(0., 1.)
        } else {
            1.
        } * (self.frames.len() - 1) as f32;
        let a = self.frames[p as usize];
        if p.fract() == 0. {
            return a;
        }
        let b = self.frames[(p as usize + 1).min(self.frames.len() - 1)];
        Pose {
            position: a.position.lerp(b.position, p.fract()),
            rotation: a.rotation.slerp(b.rotation, p.fract()),
        }
    }
    pub fn duration(&self) -> f32 {
        (self.frames.len() - 1) as f32 / 120.
    }
    pub fn new(object: Object, die: Die, seed: u32) -> Self {
        let mut bodies = RigidBodySet::new();
        let mut colliders = ColliderSet::new();
        colliders
            .insert(ColliderBuilder::cuboid(6., 0.1, 7.).translation(Vector::new(0., -1.35, 0.)));
        let mut obstacle_handles = Vec::new();
        for [p, b] in OBSTACLES {
            obstacle_handles.push(
                colliders.insert(
                    ColliderBuilder::round_cuboid(b[0] - b[3], b[1] - b[3], b[2] - b[3], b[3])
                        .translation(Vector::new(p[0], p[1], p[2]))
                        .friction(0.05)
                        .friction_combine_rule(CoefficientCombineRule::Min),
                ),
            );
        }
        let v = seed.wrapping_mul(747_796_405).rotate_left(13) as f32 / u32::MAX as f32;
        let body = bodies.insert(
            RigidBodyBuilder::dynamic()
                .translation(if object == Object::Coin {
                    Vector::new(0., 0.6, -1.2)
                } else {
                    Vector::new(-0.3, 1.1, -3.8)
                })
                .rotation(if object == Object::Coin {
                    Vector::new(std::f32::consts::FRAC_PI_2, 0., v * 0.1)
                } else {
                    Vector::new(0.6 + v, 0.2, 1.1)
                })
                .linvel(if object == Object::Coin {
                    Vector::new(0.9 - v * 1.8, 3.8, 0.7)
                } else {
                    Vector::new(3.5 - v, 1.8, 6.2)
                })
                .angvel(if object == Object::Coin {
                    Vector::new(14. + v * 4., 0., 0.)
                } else {
                    Vector::new(9. + v * 4., 3., 5.)
                })
                .linear_damping(0.18)
                .angular_damping(if object == Object::Coin { 0.8 } else { 0.35 })
                .ccd_enabled(true),
        );
        let hull = die.hull();
        let collider = if object == Object::Coin {
            let mut points = Vec::with_capacity(128);
            for i in 0..64 {
                let angle = i as f32 * std::f32::consts::TAU / 64.;
                for z in [-0.104, 0.104] {
                    points.push(Vector::new(angle.cos() * 0.972, angle.sin() * 0.972, z) * SCALE);
                }
            }
            ColliderBuilder::convex_hull(&points).expect("built-in convex coin")
        } else {
            let points: Vec<_> = hull
                .vertices
                .iter()
                .map(|p| Vector::new(p.x, p.y, p.z) * SCALE)
                .collect();
            ColliderBuilder::convex_hull(&points).expect("built-in convex die")
        };
        colliders.insert_with_parent(
            collider
                .restitution(if object == Object::Coin { 0.22 } else { 0.42 })
                .friction(0.8)
                .density(1.),
            body,
            &mut bodies,
        );
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
            // Recycled manifolds kept the thin coin jittering instead of sleeping.
            contact_recycling: false,
            ..Default::default()
        };
        let mut frames = Vec::with_capacity(1441);
        let mut contacts = 0;
        let mut obstacle_contacts = 0;
        for step in 0..2881 {
            let b = &bodies[body];
            let p = b.translation();
            let q = b.rotation();
            if step % 2 == 0 {
                frames.push(Pose {
                    position: Vec3::new(p.x, p.y, p.z),
                    rotation: Quat::from_xyzw(q.x, q.y, q.z, q.w),
                });
            }
            if (b.is_sleeping() && step % 2 == 0) || step == 2880 {
                break;
            }
            pipeline.step(
                Vector::new(0., -9.81, 0.),
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
            obstacle_contacts += narrow
                .contact_pairs()
                .filter(|p| {
                    p.has_any_active_contact()
                        && (obstacle_handles.contains(&p.collider1)
                            || obstacle_handles.contains(&p.collider2))
                })
                .count();
        }
        let settled = bodies[body].is_sleeping();
        let q = frames.last().unwrap().rotation;
        let result = if !settled {
            "Still moving — no settled result".into()
        } else if object == Object::Coin {
            let y = (q * Vec3::Z).y;
            if y.abs() < 0.9 {
                "Coin on edge".into()
            } else if y > 0. {
                "Heads".into()
            } else {
                "Tails".into()
            }
        } else {
            let (face, alignment) = hull
                .faces
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    (
                        i,
                        (q * Vec3::from_slice(&f[0][..3])).y
                            * if die == Die::D4 { -1. } else { 1. },
                    )
                })
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap();
            if alignment < 0.96 {
                "Cocked die — no clear result".into()
            } else {
                format!(
                    "{}{}",
                    face + 1,
                    if die == Die::D4 { " (bottom face)" } else { "" }
                )
            }
        };
        Self {
            frames,
            settled,
            result,
            contacts,
            obstacle_contacts,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_throws_are_bounded_and_replay_identically() {
        let cases = Die::ALL.into_iter().map(|die| (Object::Die, die, 1)).chain(
            [1, 2, 17, 42, 96]
                .into_iter()
                .map(|seed| (Object::Coin, Die::D6, seed)),
        );
        let mut coin_obstacles = 0;
        let mut settled_coins = 0;
        for (object, die, seed) in cases {
            let a = Throw::new(object, die, seed);
            let b = Throw::new(object, die, seed);
            assert_eq!(a.frames, b.frames);
            assert_eq!(a.result, b.result);
            assert!(a.contacts > 0);
            if !a.settled {
                assert_eq!(a.frames.len(), 1441);
                assert_eq!(a.result, "Still moving — no settled result");
            } else if object == Object::Coin {
                settled_coins += 1;
            }
            if object == Object::Die {
                assert!(a.obstacle_contacts > 0);
                let hull = die.hull();
                assert!(a
                    .frames
                    .iter()
                    .all(|p| hull
                        .vertices
                        .iter()
                        .all(|v| (p.rotation * *v * SCALE + p.position).y > -1.29)));
            } else {
                coin_obstacles += a.obstacle_contacts;
            }
            assert!(a.frames.iter().all(|p| p.position.is_finite()
                && p.rotation.is_finite()
                && p.position.y > -1.3
                && p.position.x.abs() < 5.2
                && p.position.z.abs() < 7.));
            assert_eq!(a.sample(1.), *a.frames.last().unwrap());
            println!(
                "{object:?} {die:?} seed {seed}: {} / settled={} / {} frames",
                a.result,
                a.settled,
                a.frames.len()
            );
        }
        assert!(
            coin_obstacles > 0,
            "coin throws must exercise bubble/footer collisions"
        );
        assert!(settled_coins >= 3, "ordinary coin throws should settle");
    }
}
