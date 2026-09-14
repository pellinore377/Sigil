use crate::{geometry::Die, physics::Pose, Object};
use glam::{Mat3, Quat, Vec3};

// Decorative reveal of an already stored result; this does not roll or sample an outcome.
pub fn result_pose(object: Object, die: Die, face: u32, progress: f32) -> Option<Pose> {
    let target = match object {
        Object::Die => {
            let f = die.hull().faces.get(face.checked_sub(1)? as usize)?;
            let q = Quat::from_mat3(&Mat3::from_cols(
                Vec3::from_slice(&f[1][..3]),
                Vec3::from_slice(&f[2][..3]),
                Vec3::from_slice(&f[0][..3]),
            ))
            .inverse();
            if die == Die::D4 {
                Quat::from_rotation_x(std::f32::consts::PI) * q
            } else {
                q
            }
        }
        Object::Coin if face <= 1 => Quat::from_rotation_y(face as f32 * std::f32::consts::PI),
        _ => return None,
    };
    let p = if progress.is_finite() {
        progress.clamp(0., 1.)
    } else {
        1.
    };
    let remaining = (1. - p).powi(3);
    Some(Pose {
        position: Vec3::new(0., (p * std::f32::consts::PI).sin() * 0.2, 0.),
        rotation: Quat::from_euler(
            glam::EulerRot::XYZ,
            remaining * std::f32::consts::TAU * 2.,
            remaining * std::f32::consts::TAU,
            0.,
        ) * target,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_stored_face_is_visible_at_rest() {
        for die in Die::ALL {
            for face in 1..=die.sides() as u32 {
                let pose = result_pose(Object::Die, die, face, 1.).unwrap();
                let normal = Vec3::from_slice(&die.hull().faces[face as usize - 1][0][..3]);
                assert!(
                    (pose.rotation * normal * if die == Die::D4 { -1. } else { 1. } - Vec3::Z)
                        .length()
                        < 1e-5
                );
                assert_eq!(
                    pose.rotation,
                    result_pose(Object::Die, die, face, f32::NAN)
                        .unwrap()
                        .rotation
                );
            }
            assert!(result_pose(Object::Die, die, 0, 1.).is_none());
            assert!(result_pose(Object::Die, die, die.sides() as u32 + 1, 1.).is_none());
        }
        for face in 0..=1 {
            let normal = if face == 0 { Vec3::Z } else { -Vec3::Z };
            assert!(
                (result_pose(Object::Coin, Die::D6, face, 1.)
                    .unwrap()
                    .rotation
                    * normal
                    - Vec3::Z)
                    .length()
                    < 1e-5
            );
        }
        assert!(result_pose(Object::Coin, Die::D6, 2, 1.).is_none());
    }
}
