pub const TILE_BYTES: usize = 2 * 1024 * 1024;
fn varint(bytes: &[u8], at: &mut usize) -> Result<u64, ()> {
    let mut value = 0;
    for shift in (0..70).step_by(7) {
        let byte = *bytes.get(*at).ok_or(())?;
        *at += 1;
        if shift == 63 && byte > 1 {
            return Err(());
        }
        value |= ((byte & 127) as u64) << shift;
        if byte < 128 {
            return Ok(value);
        }
    }
    Err(())
}
fn fields<'a>(
    bytes: &'a [u8],
    mut visit: impl FnMut(u64, u64, &'a [u8]) -> Result<(), ()>,
) -> Result<(), ()> {
    let mut at = 0;
    while at < bytes.len() {
        let key = varint(bytes, &mut at)?;
        if key >> 3 == 0 {
            return Err(());
        }
        match key & 7 {
            0 => {
                let start = at;
                varint(bytes, &mut at)?;
                visit(key >> 3, 0, &bytes[start..at])?;
            }
            1 => at = at.checked_add(8).ok_or(())?,
            5 => at = at.checked_add(4).ok_or(())?,
            2 => {
                let length = usize::try_from(varint(bytes, &mut at)?).map_err(|_| ())?;
                let end = at.checked_add(length).ok_or(())?;
                visit(key >> 3, 2, bytes.get(at..end).ok_or(())?)?;
                at = end;
            }
            _ => return Err(()),
        }
        if at > bytes.len() {
            return Err(());
        }
    }
    Ok(())
}
pub fn preflight(bytes: &[u8]) -> Result<(), ()> {
    if bytes.len() > TILE_BYTES {
        return Err(());
    }
    let (mut layers, mut features, mut integers, mut properties) = (0, 0, 0, 0);
    fields(bytes, |field, wire, layer| {
        if field != 3 || wire != 2 {
            return Ok(());
        }
        layers += 1;
        if layers > 32 {
            return Err(());
        }
        fields(layer, |field, wire, value| {
            if wire != 2 { return Ok(()); }
            match field {
                2 => {
                    features += 1;
                    if features > 8000 {
                        return Err(());
                    }
                    fields(value, |field, wire, packed| {
                        if (field == 4 || field == 2) && (wire == 0 || wire == 2) {
                            let mut at = 0;
                            while at < packed.len() {
                                varint(packed, &mut at)?;
                                integers += 1;
                                if integers > 192000 {
                                    return Err(());
                                }
                            }
                        }
                        Ok(())
                    })?;
                }
                3 | 4 => {
                    properties += 1;
                    if properties > 16000 || value.len() > 256 {
                        return Err(());
                    }
                }
                _ => (),
            }
            Ok(())
        })
    })
}
pub fn world(lat: f64, lon: f64) -> (f64, f64) {
    (
        (lon + 180.0) / 360.0,
        (1.0 - lat.to_radians().tan().asinh() / std::f64::consts::PI) / 2.0,
    )
}
pub fn geographic(x: f64, y: f64) -> (f64, f64) {
    (
        ((1.0 - 2.0 * y) * std::f64::consts::PI)
            .sinh()
            .atan()
            .to_degrees(),
        (x * 360.0).rem_euclid(360.0) - 180.0,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn projection_roundtrips() {
        for (lat, lon) in [
            (0., 0.),
            (42., -87.),
            (-33.8, 151.2),
            (85.051128, 179.999),
            (-85.051128, -179.999),
        ] {
            let (x, y) = world(lat, lon);
            let (a, b) = geographic(x, y);
            assert!((a - lat).abs() < 1e-7);
            assert!((b - lon).abs() < 1e-7);
            assert!((0.0..=1.0).contains(&y));
        }
    }
    #[test]
    fn rejects_truncated_and_oversize() {
        for bytes in [&[0x1a, 0xff][..], &[0][..], &[0x1a, 4, 1][..], &[0x1f][..]] {
            assert!(preflight(bytes).is_err())
        }
        assert!(preflight(&vec![0; TILE_BYTES + 1]).is_err());
    }
    #[test]
    fn rejects_many_layers() {
        assert!(preflight(&[0x1a, 0].repeat(33)).is_err());
        assert!(preflight(&[0x1a, 0]).is_ok());
    }
    #[test]
    fn rejects_geometry_budget_before_decode() {
        let packed = vec![0; 192001];
        fn length(mut n: usize) -> Vec<u8> {
            let mut r = vec![];
            while n >= 128 {
                r.push((n as u8) | 128);
                n >>= 7
            }
            r.push(n as u8);
            r
        }
        fn field(id: u8, b: Vec<u8>) -> Vec<u8> {
            let mut r = vec![id];
            r.extend(length(b.len()));
            r.extend(b);
            r
        }
        assert!(preflight(&field(26, field(18, field(34, packed)))).is_err());
        assert!(preflight(&field(26, field(18, [32, 0].repeat(192001)))).is_err());
        assert!(preflight(&field(26, field(18, [16, 0].repeat(192001)))).is_err());
        let mut points = vec![24, 1];
        points.extend([32, 9, 32, 0, 32, 0].repeat(64001));
        assert!(preflight(&field(26, field(18, points))).is_err());
        assert!(preflight(&field(26, field(18, vec![32, 9, 32, 0, 32, 0]))).is_ok());
    }
}
