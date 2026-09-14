use glam::Vec3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Die {
    D4,
    D6,
    D8,
    D10,
    D12,
    D20,
    D16,
    D24,
    D30,
}
impl Die {
    pub const ALL: [Self; 9] = [
        Self::D4,
        Self::D6,
        Self::D8,
        Self::D10,
        Self::D12,
        Self::D20,
        Self::D16,
        Self::D24,
        Self::D30,
    ];
    pub fn sides(self) -> usize {
        match self {
            Self::D4 => 4,
            Self::D6 => 6,
            Self::D8 => 8,
            Self::D10 => 10,
            Self::D12 => 12,
            Self::D20 => 20,
            Self::D16 => 16,
            Self::D24 => 24,
            Self::D30 => 30,
        }
    }
    pub fn hull(self) -> &'static Hull {
        static HULLS: std::sync::OnceLock<[Hull; 9]> = std::sync::OnceLock::new();
        &HULLS.get_or_init(|| Self::ALL.map(Self::build))[self as usize]
    }
    pub fn labels(self) -> &'static [[f32; 4]; 30] {
        static LABELS: std::sync::OnceLock<[[[f32; 4]; 30]; 9]> = std::sync::OnceLock::new();
        &LABELS.get_or_init(|| {
            Self::ALL.map(|die| {
                let hull = die.hull();
                let mut labels = [[0.; 4]; 30];
                for (i, f) in hull.faces.iter().enumerate() {
                    let n = Vec3::from_slice(&f[0][..3]);
                    let right = Vec3::from_slice(&f[1][..3]);
                    let up = Vec3::from_slice(&f[2][..3]);
                    let mut vertices = hull
                        .vertices
                        .iter()
                        .copied()
                        .filter(|v| (n.dot(*v) - f[0][3]).abs() < 0.0001)
                        .collect::<Vec<_>>();
                    let mut center = vertices.iter().copied().sum::<Vec3>() / vertices.len() as f32;
                    vertices.sort_by(|a, b| {
                        let a = *a - center;
                        let b = *b - center;
                        up.dot(a)
                            .atan2(right.dot(a))
                            .total_cmp(&up.dot(b).atan2(right.dot(b)))
                    });
                    let origin = center;
                    let mut area = 0.;
                    center = Vec3::ZERO;
                    for j in 0..vertices.len() {
                        let a = vertices[j];
                        let b = vertices[(j + 1) % vertices.len()];
                        let weight = (a - origin).cross(b - origin).length();
                        center += (origin + a + b) / 3. * weight;
                        area += weight;
                    }
                    center /= area;
                    let fitting_height = |half_width: f32| {
                        (0..vertices.len())
                            .map(|j| {
                                let a = vertices[j] - center;
                                let b = vertices[(j + 1) % vertices.len()] - center;
                                let normal = n.cross(b - a).normalize();
                                (a.cross(b).length() / a.distance(b))
                                    / (normal.dot(right).abs() * half_width
                                        + normal.dot(up).abs() * 0.5)
                            })
                            .fold(f32::INFINITY, f32::min)
                    };
                    labels[i] = [
                        center.dot(right),
                        center.dot(up),
                        fitting_height(0.55) * 1.25,
                        fitting_height(0.55) * 1.25,
                    ];
                }
                labels
            })
        })[self as usize]
    }
    fn build(self) -> Hull {
        let phi = (1.0 + 5.0_f32.sqrt()) / 2.0;
        let mut p = Vec::new();
        match self {
            Self::D16 => {
                p.extend([Vec3::Y, -Vec3::Y]);
                for i in 0..8 {
                    let a = i as f32 * std::f32::consts::TAU / 8.;
                    p.push(Vec3::new(a.cos() * 0.9, 0., a.sin() * 0.9));
                }
            }
            Self::D24 => {
                let a = 1. + 2_f32.sqrt();
                for x in [-1., 1.] {
                    for y in [-1., 1.] {
                        for z in [-a, a] {
                            p.extend([Vec3::new(x, y, z), Vec3::new(x, z, y), Vec3::new(z, x, y)]);
                        }
                    }
                }
                p = Hull::new(p)
                    .faces
                    .iter()
                    .map(|f| Vec3::from_slice(&f[0][..3]) / f[0][3])
                    .collect();
            }
            Self::D30 => {
                let h = Self::D20.build();
                let edge = h
                    .vertices
                    .iter()
                    .enumerate()
                    .flat_map(|(i, a)| h.vertices[i + 1..].iter().map(move |b| a.distance(*b)))
                    .fold(f32::INFINITY, f32::min);
                for (i, a) in h.vertices.iter().enumerate() {
                    for b in &h.vertices[i + 1..] {
                        if (a.distance(*b) - edge).abs() < 0.001 {
                            p.push((*a + *b) / 2.);
                        }
                    }
                }
                p = Hull::new(p)
                    .faces
                    .iter()
                    .map(|f| Vec3::from_slice(&f[0][..3]) / f[0][3])
                    .collect();
            }
            Self::D4 => {
                p = vec![
                    Vec3::new(1., 1., 1.),
                    Vec3::new(1., -1., -1.),
                    Vec3::new(-1., 1., -1.),
                    Vec3::new(-1., -1., 1.),
                ]
            }
            Self::D6 => {
                for x in [-1., 1.] {
                    for y in [-1., 1.] {
                        for z in [-1., 1.] {
                            p.push(Vec3::new(x, y, z));
                        }
                    }
                }
            }
            Self::D8 => {
                for a in [Vec3::X, Vec3::Y, Vec3::Z] {
                    p.extend([a, -a]);
                }
            }
            Self::D12 => {
                let h = Self::D20.build();
                p = h
                    .faces
                    .iter()
                    .map(|f| Vec3::from_slice(&f[0][..3]))
                    .collect();
            }
            Self::D20 => {
                for a in [-1., 1.] {
                    for b in [-phi, phi] {
                        p.extend([
                            Vec3::new(0., a, b),
                            Vec3::new(a, b, 0.),
                            Vec3::new(b, 0., a),
                        ]);
                    }
                }
            }
            Self::D10 => {
                let h = ((std::f32::consts::PI / 5.).cos()
                    - (2. * std::f32::consts::PI / 5.).cos())
                .sqrt()
                    / 2.0_f32.sqrt();
                for i in 0..10 {
                    let t = i as f32 * std::f32::consts::PI / 5.;
                    p.push(Vec3::new(t.cos(), if i % 2 == 0 { h } else { -h }, t.sin()));
                }
                let anti = Hull::new(p);
                p = anti
                    .faces
                    .iter()
                    .map(|f| Vec3::from_slice(&f[0][..3]) / f[0][3])
                    .collect();
            }
        }
        let radius = p.iter().map(|v| v.length()).fold(0., f32::max);
        for v in &mut p {
            *v *= 1.14 / radius;
        }
        let mut hull = Hull::new(p);
        if self == Self::D6 {
            for face in &mut hull.faces {
                let normal = Vec3::from_slice(&face[0][..3]);
                let up = if normal.y.abs() < 0.9 {
                    Vec3::Y
                } else {
                    Vec3::Z
                };
                let right = up.cross(normal);
                face[1][..3].copy_from_slice(&right.to_array());
                face[2][..3].copy_from_slice(&up.to_array());
            }
        }
        if self != Self::D4 {
            let mut ordered = vec![[[0.; 4]; 3]; hull.faces.len()];
            let mut remaining = hull.faces.clone();
            for i in 0..ordered.len() / 2 {
                let f = remaining.remove(0);
                let n = Vec3::from_slice(&f[0][..3]);
                let opposite = remaining
                    .iter()
                    .position(|g| n.dot(Vec3::from_slice(&g[0][..3])) < -0.9999)
                    .expect("opposite face");
                let last = ordered.len() - 1 - i;
                ordered[i] = f;
                ordered[last] = remaining.remove(opposite);
            }
            hull.faces = ordered;
        }
        hull
    }
}
#[derive(Clone)]
pub struct Hull {
    pub vertices: Vec<Vec3>,
    pub faces: Vec<[[f32; 4]; 3]>,
}
impl Hull {
    fn new(vertices: Vec<Vec3>) -> Self {
        let mut faces: Vec<[[f32; 4]; 3]> = Vec::new();
        for i in 0..vertices.len() {
            for j in i + 1..vertices.len() {
                for k in j + 1..vertices.len() {
                    let cross = (vertices[j] - vertices[i]).cross(vertices[k] - vertices[i]);
                    if cross.length() < 1e-5 {
                        continue;
                    }
                    let mut n = cross.normalize();
                    let mut h = n.dot(vertices[i]);
                    if h < 0. {
                        n = -n;
                        h = -h;
                    }
                    if vertices.iter().any(|v| n.dot(*v) > h + 1e-4)
                        || faces
                            .iter()
                            .any(|f| Vec3::from_slice(&f[0][..3]).dot(n) > 0.9999)
                    {
                        continue;
                    }
                    let center = n * h;
                    let face_vertices: Vec<_> = vertices
                        .iter()
                        .filter(|v| (n.dot(**v) - h).abs() < 1e-4)
                        .collect();
                    let tip = face_vertices
                        .iter()
                        .max_by(|a, b| {
                            (***a - center)
                                .length_squared()
                                .total_cmp(&(***b - center).length_squared())
                        })
                        .unwrap();
                    let up = (**tip - center).normalize();
                    let right = up.cross(n).normalize();
                    let scale = face_vertices
                        .iter()
                        .map(|v| (**v - center).length())
                        .fold(0., f32::max);
                    faces.push([
                        [n.x, n.y, n.z, h],
                        [right.x, right.y, right.z, scale],
                        [up.x, up.y, up.z, 0.],
                    ]);
                }
            }
        }
        Self { vertices, faces }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn polyhedra_have_expected_supporting_faces() {
        for die in Die::ALL {
            let h = die.hull();
            assert_eq!(h.faces.len(), die.sides(), "{die:?}");
            if die != Die::D4 {
                for i in 0..h.faces.len() / 2 {
                    assert!(
                        Vec3::from_slice(&h.faces[i][0][..3])
                            .dot(Vec3::from_slice(&h.faces[h.faces.len() - 1 - i][0][..3]))
                            < -0.9999
                    );
                }
            }
            for f in &h.faces {
                let n = Vec3::from_slice(&f[0][..3]);
                assert!((n.length() - 1.).abs() < 1e-5);
                assert!(h.vertices.iter().all(|v| n.dot(*v) <= f[0][3] + 1e-4));
            }
        }
    }
}
