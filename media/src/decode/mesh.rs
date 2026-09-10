use super::*;
type Point = [f64; 3];
type Triangle = [Point; 3];
type Transform = [f64; 12];
const IDENTITY: Transform = [1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.];
fn transformed(p: Point, t: &Transform) -> Point {
    std::array::from_fn(|i| p[0] * t[i] + p[1] * t[3 + i] + p[2] * t[6 + i] + t[9 + i])
}
fn compose(parent: &Transform, child: &Transform) -> Transform {
    let mut result = IDENTITY;
    for i in 0..3 {
        for j in 0..3 {
            result[i * 3 + j] = (0..3).map(|k| child[i * 3 + k] * parent[k * 3 + j]).sum();
        }
    }
    result[9..].copy_from_slice(&transformed([child[9], child[10], child[11]], parent));
    result
}
fn add(triangles: &mut Vec<Triangle>, triangle: Triangle) -> Result<(), Error> {
    if triangles.len() == 200_000 {
        return Err(Error::Limit);
    }
    if triangle
        .iter()
        .flatten()
        .any(|v| !v.is_finite() || v.abs() > 1e12)
    {
        return Err(Error::Invalid);
    }
    triangles.push(triangle);
    Ok(())
}
fn three_mf(input: &File) -> Result<Vec<Triangle>, Error> {
    check_zip(input)?;
    preflight(input)?;
    let model =
        lib3mf::Model::from_reader(source(input)?).map_err(|_| Error::Invalid)?;
    let mut models = std::collections::HashMap::from([(String::new(), model)]);
    let mut archive =
        zip::ZipArchive::new(source(input)?).map_err(|_| Error::Invalid)?;
    for index in 0..archive.len() {
        let mut part = archive.by_index(index).map_err(|_| Error::Invalid)?;
        if !part.name().ends_with(".model") {
            continue;
        }
        if models.len() == 65 {
            return Err(Error::Limit);
        }
        let name = part.name().to_owned();
        let mut xml = String::new();
        part.read_to_string(&mut xml)?;
        let model = lib3mf::parser::parse_model_xml(&xml).map_err(|_| Error::Invalid)?;
        if models.insert(name, model).is_some() {
            return Err(Error::Invalid);
        }
    }
    fn part_name(current: &str, path: Option<&str>) -> Result<String, Error> {
        let Some(path) = path else {
            return Ok(current.into());
        };
        if path.len() > 512
            || path.contains(['\\', ':', '%'])
            || path.split('/').any(|s| s == ".." || s == ".")
        {
            return Err(Error::Invalid);
        }
        Ok(if let Some(absolute) = path.strip_prefix('/') {
            absolute.into()
        } else {
            let parent = current.rsplit_once('/').map_or("3D", |(parent, _)| parent);
            format!("{parent}/{path}")
        })
    }
    fn object(
        models: &std::collections::HashMap<String, lib3mf::Model>,
        part: &str,
        id: usize,
        transform: Transform,
        stack: &mut Vec<(String, usize)>,
        triangles: &mut Vec<Triangle>,
        steps: &mut u32,
    ) -> Result<(), Error> {
        *steps += 1;
        if *steps > 400_000 || stack.len() >= 32 {
            return Err(Error::Limit);
        }
        if stack.iter().any(|(p, i)| p == part && *i == id) {
            return Err(Error::Invalid);
        }
        let model = models.get(part).ok_or(Error::Unsupported)?;
        let object = model
            .resources
            .objects
            .iter()
            .find(|o| o.id == id)
            .ok_or(Error::Invalid)?;
        stack.push((part.into(), id));
        if let Some(mesh) = &object.mesh {
            for triangle in &mesh.triangles {
                let point = |id: usize| -> Result<Point, Error> {
                    let v = mesh.vertices.get(id).ok_or(Error::Invalid)?;
                    Ok(transformed([v.x, v.y, v.z], &transform))
                };
                add(
                    triangles,
                    [
                        point(triangle.v1)?,
                        point(triangle.v2)?,
                        point(triangle.v3)?,
                    ],
                )?;
            }
        }
        if object.displacement_mesh.is_some() || object.boolean_shape.is_some() {
            return Err(Error::Unsupported);
        }
        for component in &object.components {
            let target = part_name(part, component.path.as_deref())?;
            object_recurse(
                models,
                &target,
                component.objectid,
                compose(&transform, &component.transform.unwrap_or(IDENTITY)),
                stack,
                triangles,
                steps,
            )?;
        }
        stack.pop();
        Ok(())
    }
    // Separate name keeps the recursive call distinct from the selected object.
    use object as object_recurse;
    let mut triangles = Vec::new();
    let mut steps = 0;
    for item in &models[""].build.items {
        object(
            &models,
            &part_name("", item.production_path.as_deref())?,
            item.objectid,
            item.transform.unwrap_or(IDENTITY),
            &mut Vec::new(),
            &mut triangles,
            &mut steps,
        )?;
    }
    Ok(triangles)
}

fn preflight(input: &File) -> Result<(), Error> {
    let mut archive =
        zip::ZipArchive::new(source(input)?).map_err(|_| Error::Invalid)?;
    for index in 0..archive.len() {
        let mut part = archive.by_index(index).map_err(|_| Error::Invalid)?;
        if part.is_dir() {
            continue;
        }
        let mut bytes = Vec::new();
        part.read_to_end(&mut bytes)?;
        bounded_xml(&bytes)?;
    }
    Ok(())
}

fn bounded_xml(xml: &[u8]) -> Result<(), Error> {
    use quick_xml::{events::Event, Reader};
    let mut reader = Reader::from_reader(xml);
    loop {
        // The consumer validates syntax; binary textures may stop XML parsing early.
        let Ok(event) = reader.read_event() else {
            return Ok(());
        };
        match event {
            Event::Start(tag) | Event::Empty(tag) => {
                if tag.name().as_ref().len() > 128 {
                    return Err(Error::Limit);
                }
                // lib3mf's older XML parser performs quadratic duplicate checks.
                for (n, attribute) in tag.attributes().enumerate() {
                    let attribute = attribute.map_err(|_| Error::Invalid)?;
                    if n >= 128
                        || attribute.key.as_ref().len() > 128
                        || attribute.value.len() > 65536
                    {
                        return Err(Error::Limit);
                    }
                }
            }
            Event::DocType(_) => return Err(Error::Invalid),
            Event::Eof => return Ok(()),
            _ => (),
        }
    }
}

pub(super) fn render(
    input: &File,
    format: Format,
    yaw: f32,
    pitch: f32,
    width: u32,
) -> Result<Preview, Error> {
    let mut triangles = if format == Format::ThreeMf {
        three_mf(input)?
    } else {
        let mesh = stl_io::read_stl(&mut source(input)?).map_err(|_| Error::Invalid)?;
        let mut triangles = Vec::new();
        for face in mesh.faces {
            let mut triangle = [[0.; 3]; 3];
            for (out, index) in triangle.iter_mut().zip(face.vertices) {
                let vertex = mesh.vertices.get(index).ok_or(Error::Invalid)?;
                *out = [vertex[0] as f64, vertex[1] as f64, vertex[2] as f64];
            }
            add(&mut triangles, triangle)?;
        }
        triangles
    };
    if triangles.is_empty() {
        return Err(Error::Invalid);
    }
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for p in triangles.iter().flatten() {
        for i in 0..3 {
            min[i] = min[i].min(p[i]);
            max[i] = max[i].max(p[i]);
        }
    }
    let center: Point = std::array::from_fn(|i| (min[i] + max[i]) / 2.0);
    let radius = (0..3)
        .map(|i| (max[i] - min[i]).powi(2))
        .sum::<f64>()
        .sqrt()
        / 2.0;
    if radius <= 0.0 || !radius.is_finite() {
        return Err(Error::Invalid);
    }
    let (sy, cy) = (yaw as f64).to_radians().sin_cos();
    let (sp, cp) = (pitch as f64).to_radians().sin_cos();
    for p in triangles.iter_mut().flatten() {
        let [x, y, z] = std::array::from_fn(|i| (p[i] - center[i]) / radius);
        let rx = cy * x + sy * z;
        let rz = -sy * x + cy * z;
        *p = [rx, cp * y - sp * rz, sp * y + cp * rz];
    }
    triangles.sort_by(|a, b| {
        a.iter()
            .map(|v| v[2])
            .sum::<f64>()
            .total_cmp(&b.iter().map(|v| v[2]).sum::<f64>())
    });
    let mut pixmap = tiny_skia::Pixmap::new(width, width).ok_or(Error::Limit)?;
    pixmap.fill(tiny_skia::Color::from_rgba8(245, 245, 245, 255));
    for t in &triangles {
        let a: Point = std::array::from_fn(|i| t[1][i] - t[0][i]);
        let b: Point = std::array::from_fn(|i| t[2][i] - t[0][i]);
        let n = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ];
        let length = n.iter().map(|v| v * v).sum::<f64>().sqrt();
        if length <= 1e-15 {
            continue;
        }
        let light = (70.0 + 150.0 * (n[2] / length).abs()) as u8;
        let screen = |p: Point| {
            (
                (width as f64 * (0.5 + p[0] * 0.45)) as f32,
                (width as f64 * (0.5 - p[1] * 0.45)) as f32,
            )
        };
        let mut path = tiny_skia::PathBuilder::new();
        let (x, y) = screen(t[0]);
        path.move_to(x, y);
        for p in &t[1..] {
            let (x, y) = screen(*p);
            path.line_to(x, y);
        }
        path.close();
        let mut paint = tiny_skia::Paint::default();
        paint.set_color_rgba8(light, light, light, 255);
        pixmap.fill_path(
            &path.finish().ok_or(Error::Invalid)?,
            &paint,
            tiny_skia::FillRule::Winding,
            tiny_skia::Transform::identity(),
            None,
        );
    }
    Ok(Preview {
        content: Content::Mesh {
            width,
            height: width,
            triangles: triangles.len() as u32,
        },
        bytes: pixmap.take(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hostile_xml_is_rejected_before_the_legacy_parser() {
        let attributes = (0..10000)
            .map(|n| format!(" a{n}=\"0\""))
            .collect::<String>();
        assert!(matches!(
            bounded_xml(format!("<model{attributes}/>").as_bytes()),
            Err(Error::Limit)
        ));
        assert!(bounded_xml(b"<model unit=\"millimeter\"/>").is_ok());
        assert!(
            bounded_xml(b"<!DOCTYPE model [<!ENTITY x SYSTEM 'file:///synthetic'>]><model/>")
                .is_err()
        );
        assert!(bounded_xml(b"<model a=\"0\" a=\"1\"/>").is_err());
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut zip = zip::ZipWriter::new(file.reopen().unwrap());
        zip.start_file("3D/3dmodel.model", zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut zip, format!("<model{attributes}/>").as_bytes()).unwrap();
        zip.finish().unwrap();
        assert!(matches!(three_mf(file.as_file()), Err(Error::Limit)));
    }
}
