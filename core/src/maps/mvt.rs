//! Mapbox Vector Tile decoding — the format the discovered map server
//! publishes. Hand-walked protobuf (the schema is four small messages) so no
//! codegen or new dependencies ride along.

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Num(f64),
    Bool(bool),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GeomType {
    Point,
    Line,
    Polygon,
}

pub struct Feature {
    pub geom_type: GeomType,
    /// One ring/line per Vec, in tile extent units.
    pub paths: Vec<Vec<(f32, f32)>>,
    pub tags: HashMap<String, Value>,
}

pub struct Layer {
    pub name: String,
    pub extent: u32,
    pub features: Vec<Feature>,
}

// ---- protobuf primitives -------------------------------------------------

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    fn done(&self) -> bool {
        self.pos >= self.buf.len()
    }
    fn varint(&mut self) -> Option<u64> {
        let mut out = 0u64;
        let mut shift = 0;
        loop {
            let b = *self.buf.get(self.pos)?;
            self.pos += 1;
            out |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Some(out);
            }
            shift += 7;
            if shift > 63 {
                return None;
            }
        }
    }
    fn tag(&mut self) -> Option<(u64, u8)> {
        let v = self.varint()?;
        Some((v >> 3, (v & 7) as u8))
    }
    fn bytes(&mut self) -> Option<&'a [u8]> {
        let len = self.varint()? as usize;
        let end = self.pos.checked_add(len)?;
        let s = self.buf.get(self.pos..end)?;
        self.pos = end;
        Some(s)
    }
    fn skip(&mut self, wire: u8) -> Option<()> {
        match wire {
            0 => {
                self.varint()?;
            }
            1 => {
                self.pos = self.pos.checked_add(8).filter(|e| *e <= self.buf.len())?;
            }
            2 => {
                self.bytes()?;
            }
            5 => {
                self.pos = self.pos.checked_add(4).filter(|e| *e <= self.buf.len())?;
            }
            _ => return None,
        }
        Some(())
    }
}

fn zigzag(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

// ---- MVT messages --------------------------------------------------------

fn decode_value(buf: &[u8]) -> Option<Value> {
    let mut r = Reader::new(buf);
    let mut out = None;
    while !r.done() {
        let (field, wire) = r.tag()?;
        match (field, wire) {
            (1, 2) => out = Some(Value::Str(String::from_utf8_lossy(r.bytes()?).into_owned())),
            (2, 5) => {
                let b = r.buf.get(r.pos..r.pos + 4)?;
                r.pos += 4;
                out = Some(Value::Num(f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64));
            }
            (3, 1) => {
                let b = r.buf.get(r.pos..r.pos + 8)?;
                r.pos += 8;
                out = Some(Value::Num(f64::from_le_bytes([
                    b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
                ])));
            }
            (4, 0) => out = Some(Value::Num(r.varint()? as i64 as f64)),
            (5, 0) => out = Some(Value::Num(r.varint()? as f64)),
            (6, 0) => out = Some(Value::Num(zigzag(r.varint()?) as f64)),
            (7, 0) => out = Some(Value::Bool(r.varint()? != 0)),
            _ => r.skip(wire)?,
        }
    }
    out
}

/// Command stream → rings/lines. Commands: MoveTo=1, LineTo=2, ClosePath=7;
/// coordinates are zigzag deltas.
fn decode_geometry(cmds: &[u32]) -> Vec<Vec<(f32, f32)>> {
    let mut paths: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut cur: Vec<(f32, f32)> = Vec::new();
    let (mut x, mut y) = (0i64, 0i64);
    let mut i = 0usize;
    while i < cmds.len() {
        let cmd = cmds[i];
        i += 1;
        let op = cmd & 7;
        let count = (cmd >> 3) as usize;
        match op {
            1 | 2 => {
                for _ in 0..count {
                    if i + 2 > cmds.len() {
                        return paths;
                    }
                    let dx = zigzag(u64::from(cmds[i]));
                    let dy = zigzag(u64::from(cmds[i + 1]));
                    i += 2;
                    if op == 1 && !cur.is_empty() {
                        paths.push(std::mem::take(&mut cur));
                    }
                    x += dx;
                    y += dy;
                    cur.push((x as f32, y as f32));
                }
            }
            7 => {
                // ClosePath: repeat the first point of the ring.
                if let Some(&first) = cur.first() {
                    cur.push(first);
                }
            }
            _ => return paths,
        }
    }
    if !cur.is_empty() {
        paths.push(cur);
    }
    paths
}

fn decode_feature(buf: &[u8], keys: &[String], values: &[Value]) -> Option<Feature> {
    let mut r = Reader::new(buf);
    let mut geom_type = None;
    let mut tag_ids: Vec<u32> = Vec::new();
    let mut cmds: Vec<u32> = Vec::new();
    while !r.done() {
        let (field, wire) = r.tag()?;
        match (field, wire) {
            (2, 2) => {
                let mut pr = Reader::new(r.bytes()?);
                while !pr.done() {
                    tag_ids.push(pr.varint()? as u32);
                }
            }
            (3, 0) => {
                geom_type = match r.varint()? {
                    1 => Some(GeomType::Point),
                    2 => Some(GeomType::Line),
                    3 => Some(GeomType::Polygon),
                    _ => None,
                };
            }
            (4, 2) => {
                let mut pr = Reader::new(r.bytes()?);
                while !pr.done() {
                    cmds.push(pr.varint()? as u32);
                }
            }
            _ => r.skip(wire)?,
        }
    }
    let geom_type = geom_type?;
    let mut tags = HashMap::new();
    for pair in tag_ids.chunks_exact(2) {
        if let (Some(k), Some(v)) = (keys.get(pair[0] as usize), values.get(pair[1] as usize)) {
            tags.insert(k.clone(), v.clone());
        }
    }
    Some(Feature { geom_type, paths: decode_geometry(&cmds), tags })
}

fn decode_layer(buf: &[u8]) -> Option<Layer> {
    let mut r = Reader::new(buf);
    let mut name = String::new();
    let mut extent = 4096u32;
    let mut keys: Vec<String> = Vec::new();
    let mut values: Vec<Value> = Vec::new();
    let mut feature_bufs: Vec<&[u8]> = Vec::new();
    while !r.done() {
        let (field, wire) = r.tag()?;
        match (field, wire) {
            (1, 2) => name = String::from_utf8_lossy(r.bytes()?).into_owned(),
            (2, 2) => feature_bufs.push(r.bytes()?),
            (3, 2) => keys.push(String::from_utf8_lossy(r.bytes()?).into_owned()),
            (4, 2) => values.push(decode_value(r.bytes()?).unwrap_or(Value::Bool(false))),
            (5, 0) => extent = r.varint()? as u32,
            _ => r.skip(wire)?,
        }
    }
    let features =
        feature_bufs.iter().filter_map(|b| decode_feature(b, &keys, &values)).collect();
    Some(Layer { name, extent: extent.max(1), features })
}

/// The whole tile. Undecodable layers are dropped rather than failing the tile.
pub fn decode(buf: &[u8]) -> Vec<Layer> {
    let mut r = Reader::new(buf);
    let mut layers = Vec::new();
    while !r.done() {
        let Some((field, wire)) = r.tag() else { break };
        if field == 3 && wire == 2 {
            match r.bytes().and_then(decode_layer) {
                Some(l) => layers.push(l),
                None => break,
            }
        } else if r.skip(wire).is_none() {
            break;
        }
    }
    layers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zigzag_round_trip() {
        assert_eq!(zigzag(0), 0);
        assert_eq!(zigzag(1), -1);
        assert_eq!(zigzag(2), 1);
        assert_eq!(zigzag(3), -2);
    }

    #[test]
    fn geometry_square() {
        // MoveTo(1,1); LineTo(+2,0, 0,+2, -2,0); ClosePath — the spec's example shape.
        let cmds = [9, 2, 2, 26, 4, 0, 0, 4, 3, 0, 15];
        let paths = decode_geometry(&cmds);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].first(), Some(&(1.0, 1.0)));
        assert_eq!(paths[0].last(), Some(&(1.0, 1.0))); // closed
        assert_eq!(paths[0].len(), 5);
    }
}
