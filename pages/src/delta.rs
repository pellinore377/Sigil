//! Attributed text deltas: retain, insert (text or embed), delete over Unicode scalars; an embed is one scalar.
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// Text bytes of one body or cell an author may produce; an embed counts one.
pub const MAX_BODY: usize = 1024 * 1024;
/// Replay limit, with room for concurrent growth that each author kept under `MAX_BODY`.
pub const MAX_REPLAY: usize = 2 * MAX_BODY;
/// U+FFFC stands in for an embed in flattened text so offsets line up.
pub const EMBED_CHAR: char = '\u{FFFC}';

pub type Attrs = BTreeMap<String, Value>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Insert {
    Text(String),
    Embed(Value),
}
impl Insert {
    pub fn len(&self) -> u32 {
        match self {
            Insert::Text(t) => t.chars().count() as u32,
            Insert::Embed(_) => 1,
        }
    }
    pub fn is_empty(&self) -> bool {
        matches!(self, Insert::Text(t) if t.is_empty())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Component {
    Retain {
        retain: u32,
        #[serde(default, skip_serializing_if = "Attrs::is_empty")]
        attrs: Attrs,
    },
    Insert {
        insert: Insert,
        #[serde(default, skip_serializing_if = "Attrs::is_empty")]
        attrs: Attrs,
    },
    Delete {
        delete: u32,
    },
}
/// Exactly one of `retain`, `insert` or `delete`, with `attrs` only beside `retain` or `insert`.
impl<'de> Deserialize<'de> for Component {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut map = Map::<String, Value>::deserialize(d)?;
        let attrs = match map.remove("attrs") {
            Some(v) => Some(serde_json::from_value::<Attrs>(v).map_err(D::Error::custom)?),
            None => None,
        };
        let mut keys: Vec<&str> = map.keys().map(String::as_str).collect();
        keys.sort_unstable();
        let count = |v: Value| serde_json::from_value::<u32>(v).map_err(D::Error::custom);
        Ok(match (keys.as_slice(), attrs) {
            (["retain"], attrs) => Component::Retain {
                retain: count(map.remove("retain").unwrap_or_default())?,
                attrs: attrs.unwrap_or_default(),
            },
            (["insert"], attrs) => Component::Insert {
                insert: serde_json::from_value(map.remove("insert").unwrap_or_default())
                    .map_err(D::Error::custom)?,
                attrs: attrs.unwrap_or_default(),
            },
            (["delete"], None) => Component::Delete {
                delete: count(map.remove("delete").unwrap_or_default())?,
            },
            _ => return Err(D::Error::custom("invalid delta component")),
        })
    }
}
impl Component {
    fn len(&self) -> u32 {
        match self {
            Component::Retain { retain, .. } => *retain,
            Component::Insert { insert, .. } => insert.len(),
            Component::Delete { delete } => *delete,
        }
    }
    /// The first `n` scalars, leaving the rest in `self`; `n` must be less than `len`.
    fn split(&mut self, n: u32) -> Component {
        match self {
            Component::Retain { retain, attrs } => {
                *retain -= n;
                Component::Retain {
                    retain: n,
                    attrs: attrs.clone(),
                }
            }
            Component::Delete { delete } => {
                *delete -= n;
                Component::Delete { delete: n }
            }
            Component::Insert {
                insert: Insert::Text(text),
                attrs,
            } => {
                let at = text
                    .char_indices()
                    .nth(n as usize)
                    .map_or(text.len(), |(i, _)| i);
                let head = text[..at].to_owned();
                text.drain(..at);
                Component::Insert {
                    insert: Insert::Text(head),
                    attrs: attrs.clone(),
                }
            }
            Component::Insert { .. } => unreachable!("embed split"),
        }
    }
}

/// Later attributes over earlier ones; `null` clears when `keep_null`, removes the key otherwise.
pub fn compose_attrs(earlier: &Attrs, later: &Attrs, keep_null: bool) -> Attrs {
    let mut out = earlier.clone();
    for (k, v) in later {
        out.insert(k.clone(), v.clone());
    }
    if !keep_null {
        out.retain(|_, v| !v.is_null());
    }
    out
}
/// Attributes of the later op after the earlier one wins every conflict.
fn transform_attrs(later: &Attrs, earlier: &Attrs) -> Attrs {
    later
        .iter()
        .filter(|(k, _)| !earlier.contains_key(*k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// An empty delta is the identity on any body.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Delta {
    pub ops: Vec<Component>,
}
/// Read through `push`, so zero-length parts drop and neighbours merge as built locally.
impl<'de> Deserialize<'de> for Delta {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Vec::<Component>::deserialize(d)?
            .into_iter()
            .fold(Delta::new(), Delta::push))
    }
}
impl Delta {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn push(mut self, mut part: Component) -> Self {
        if part.len() == 0 {
            return self;
        }
        // An insert has nothing for `null` to clear.
        if let Component::Insert { attrs, .. } = &mut part {
            attrs.retain(|_, v| !v.is_null());
        }
        // Counts that would overflow stay separate parts.
        match (self.ops.last_mut(), part) {
            (
                Some(Component::Retain { retain, attrs }),
                Component::Retain {
                    retain: n,
                    attrs: a,
                },
            ) if *attrs == a && retain.checked_add(n).is_some() => *retain += n,
            (Some(Component::Delete { delete }), Component::Delete { delete: n })
                if delete.checked_add(n).is_some() =>
            {
                *delete += n
            }
            (
                Some(Component::Insert {
                    insert: Insert::Text(text),
                    attrs,
                }),
                Component::Insert {
                    insert: Insert::Text(t),
                    attrs: a,
                },
            ) if *attrs == a => text.push_str(&t),
            (_, part) => self.ops.push(part),
        }
        self
    }
    pub fn retain(self, n: u32) -> Self {
        self.push(Component::Retain {
            retain: n,
            attrs: Attrs::new(),
        })
    }
    pub fn retain_with(self, n: u32, attrs: Attrs) -> Self {
        self.push(Component::Retain { retain: n, attrs })
    }
    pub fn insert(self, text: &str) -> Self {
        self.insert_with(text, Attrs::new())
    }
    pub fn insert_with(self, text: &str, attrs: Attrs) -> Self {
        self.push(Component::Insert {
            insert: Insert::Text(text.to_owned()),
            attrs,
        })
    }
    pub fn embed(self, embed: Value, attrs: Attrs) -> Self {
        self.push(Component::Insert {
            insert: Insert::Embed(embed),
            attrs,
        })
    }
    pub fn delete(self, n: u32) -> Self {
        self.push(Component::Delete { delete: n })
    }
    /// Scalars the delta reads; saturates, so an overflowing delta matches no body.
    pub fn base_len(&self) -> u32 {
        self.ops
            .iter()
            .map(|p| match p {
                Component::Retain { retain, .. } => *retain,
                Component::Delete { delete } => *delete,
                Component::Insert { .. } => 0,
            })
            .fold(0, u32::saturating_add)
    }
    /// Scalars the delta leaves; saturates like `base_len`.
    pub fn target_len(&self) -> u32 {
        self.ops
            .iter()
            .map(|p| match p {
                Component::Retain { retain, .. } => *retain,
                Component::Insert { insert, .. } => insert.len(),
                Component::Delete { .. } => 0,
            })
            .fold(0, u32::saturating_add)
    }
    /// Text bytes of a body, an embed counting one.
    pub fn size(&self) -> usize {
        bytes(&self.ops)
    }
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
    pub fn is_noop(&self) -> bool {
        self.ops
            .iter()
            .all(|p| matches!(p, Component::Retain { attrs, .. } if attrs.is_empty()))
    }
    /// A body: inserts only.
    pub fn is_body(&self) -> bool {
        self.ops
            .iter()
            .all(|p| matches!(p, Component::Insert { .. }))
    }
    /// Scalars of a body, embeds as `EMBED_CHAR`.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for p in &self.ops {
            match p {
                Component::Insert {
                    insert: Insert::Text(t),
                    ..
                } => out.push_str(t),
                Component::Insert {
                    insert: Insert::Embed(_),
                    ..
                } => out.push(EMBED_CHAR),
                _ => {}
            }
        }
        out
    }
    /// Every scalar of a body with its attributes and, for embeds, the embed.
    pub fn scalars(&self) -> Vec<(char, &Attrs, Option<&Value>)> {
        let mut out = Vec::new();
        for p in &self.ops {
            match p {
                Component::Insert {
                    insert: Insert::Text(t),
                    attrs,
                } => out.extend(t.chars().map(|c| (c, attrs, None))),
                Component::Insert {
                    insert: Insert::Embed(e),
                    attrs,
                } => out.push((EMBED_CHAR, attrs, Some(e))),
                _ => {}
            }
        }
        out
    }
    /// A replacement of the scalar range `[start, end)` in a body of `len` scalars.
    pub fn replace(len: u32, start: u32, end: u32, text: &str) -> Result<Self, &'static str> {
        if start > end || end > len {
            return Err("range out of bounds");
        }
        Ok(Self::new()
            .retain(start)
            .delete(end - start)
            .insert(text)
            .retain(len - end))
    }
    /// A delta turning any body of `len` scalars into `body`.
    pub fn replace_all(len: u32, body: &Delta) -> Self {
        body.ops
            .iter()
            .cloned()
            .fold(Self::new().delete(len), Delta::push)
    }
    /// This delta applied to a body, under the replay limit.
    pub fn apply(&self, body: &Delta) -> Result<Delta, &'static str> {
        self.apply_within(body, MAX_REPLAY)
    }
    /// This delta applied to a body whose result may hold at most `limit` bytes.
    pub fn apply_within(&self, body: &Delta, limit: usize) -> Result<Delta, &'static str> {
        if self.is_empty() {
            return Ok(body.clone());
        }
        let out = body.compose(self)?;
        if !out.is_body() {
            return Err("operation shorter than document");
        }
        if out.size() > limit {
            return Err("document too large");
        }
        Ok(out)
    }
    /// `self` then `other` as one delta.
    pub fn compose(&self, other: &Delta) -> Result<Delta, &'static str> {
        if self.target_len() != other.base_len() {
            return Err("compose length mismatch");
        }
        let mut out = Delta::new();
        let (mut a, mut b) = (nonzero(&self.ops), nonzero(&other.ops));
        let (mut ca, mut cb) = (a.next(), b.next());
        loop {
            match (ca.take(), cb.take()) {
                (None, None) => return Ok(out),
                (Some(x @ Component::Delete { .. }), y) => {
                    out = out.push(x);
                    ca = a.next();
                    cb = y;
                }
                (x, Some(y @ Component::Insert { .. })) => {
                    out = out.push(y);
                    ca = x;
                    cb = b.next();
                }
                (None, _) | (_, None) => return Err("compose length mismatch"),
                (Some(mut x), Some(mut y)) => {
                    let (lx, ly) = (x.len(), y.len());
                    let n = lx.min(ly);
                    let (xs, ys) = (
                        if lx > n { x.split(n) } else { x.clone() },
                        if ly > n { y.split(n) } else { y.clone() },
                    );
                    match (xs, ys) {
                        (
                            Component::Retain { attrs: ax, .. },
                            Component::Retain { attrs: ay, .. },
                        ) => out = out.retain_with(n, compose_attrs(&ax, &ay, true)),
                        (
                            Component::Insert { insert, attrs: ax },
                            Component::Retain { attrs: ay, .. },
                        ) => {
                            out = out.push(Component::Insert {
                                insert,
                                attrs: compose_attrs(&ax, &ay, false),
                            })
                        }
                        (Component::Retain { .. }, Component::Delete { .. }) => out = out.delete(n),
                        _ => {}
                    }
                    ca = if lx > n { Some(x) } else { a.next() };
                    cb = if ly > n { Some(y) } else { b.next() };
                }
            }
        }
    }
    /// `apply` in place, under the replay limit.
    pub fn apply_to(&self, body: &mut Delta) -> Result<(), &'static str> {
        self.apply_to_within(body, MAX_REPLAY)
    }
    /// `apply_within` in place, recomposing only the stretch between the outer plain retains.
    pub fn apply_to_within(&self, body: &mut Delta, limit: usize) -> Result<(), &'static str> {
        let plain = |p: Option<&Component>| match p {
            Some(Component::Retain { retain, attrs }) if attrs.is_empty() => *retain,
            _ => 0,
        };
        let lead = plain(self.ops.first());
        let trail = if self.ops.len() > 1 {
            plain(self.ops.last())
        } else {
            0
        };
        let len = body.target_len();
        if !body.is_body()
            || self.base_len() != len
            || u64::from(lead) + u64::from(trail) > u64::from(len)
        {
            *body = self.apply_within(body, limit)?;
            return Ok(());
        }
        let inner = Delta {
            ops: self.ops[usize::from(lead > 0)..self.ops.len() - usize::from(trail > 0)].to_vec(),
        };
        if inner.is_empty() {
            return Ok(());
        }
        let ((ia, ka), (ib, kb)) = (body.locate(lead), body.locate(len - trail));
        let middle = body.slice(lead, len - trail);
        let replaced = middle.compose(&inner)?;
        if !replaced.is_body() {
            return Err("operation shorter than document");
        }
        if body.size() - middle.size() + replaced.size() > limit {
            return Err("document too large");
        }
        // Edge remnants around the new stretch, merged at both seams.
        let mut piece = Vec::with_capacity(replaced.ops.len() + 2);
        if ka > 0 {
            let mut head = body.ops[ia].clone();
            let rest = head.len() - ka;
            if rest > 0 {
                head = head.split(ka);
            }
            piece.push(head);
        }
        piece.extend(replaced.ops);
        let end = if kb > 0 {
            let mut tail = body.ops[ib].clone();
            if kb < tail.len() {
                tail.split(kb);
                piece.push(tail);
            }
            ib + 1
        } else {
            ib
        };
        let seam = ia.saturating_sub(1);
        let count = piece.len();
        body.ops.splice(ia..end, piece);
        body.merge(seam, (ia + count + 1).min(body.ops.len()));
        Ok(())
    }
    /// The scalars `[start, end)` of a body as a body of their own.
    pub fn slice(&self, start: u32, end: u32) -> Delta {
        let ((ia, ka), (ib, kb)) = (self.locate(start), self.locate(end));
        let mut out = Delta::new();
        for i in ia..=ib.min(self.ops.len().saturating_sub(1)) {
            if i == ib && kb == 0 {
                break;
            }
            let mut part = self.ops[i].clone();
            if i == ib && kb < part.len() {
                part = part.split(kb);
            }
            if i == ia && ka > 0 {
                part.split(ka);
            }
            out = out.push(part);
        }
        out
    }
    /// Component and scalar offset inside it at body position `at`; past the end is `(len, 0)`.
    fn locate(&self, at: u32) -> (usize, u32) {
        let mut rest = at;
        for (i, part) in self.ops.iter().enumerate() {
            let n = part.len();
            if rest < n {
                return (i, rest);
            }
            rest -= n;
        }
        (self.ops.len(), 0)
    }
    /// Joins neighbouring text of equal attributes in `ops[from..to]`, as `push` would have.
    fn merge(&mut self, from: usize, to: usize) {
        let mut i = from;
        let mut to = to.min(self.ops.len());
        while i + 1 < to {
            let joined = match (&self.ops[i], &self.ops[i + 1]) {
                (
                    Component::Insert {
                        insert: Insert::Text(_),
                        attrs: a,
                    },
                    Component::Insert {
                        insert: Insert::Text(_),
                        attrs: b,
                    },
                ) => a == b,
                _ => false,
            };
            if joined {
                if let Component::Insert {
                    insert: Insert::Text(t),
                    ..
                } = self.ops.remove(i + 1)
                {
                    if let Component::Insert {
                        insert: Insert::Text(head),
                        ..
                    } = &mut self.ops[i]
                    {
                        head.push_str(&t);
                    }
                }
                to -= 1;
            } else {
                i += 1;
            }
        }
        self.ops.retain(|p| p.len() > 0);
    }
    /// The scalar at `at` in a body, with its attributes and, for an embed, the embed.
    pub fn scalar_at(&self, at: u32) -> Option<(char, &Attrs, Option<&Value>)> {
        let (i, k) = self.locate(at);
        match self.ops.get(i)? {
            Component::Insert {
                insert: Insert::Text(t),
                attrs,
            } => t.chars().nth(k as usize).map(|c| (c, attrs, None)),
            Component::Insert {
                insert: Insert::Embed(e),
                attrs,
            } => Some((EMBED_CHAR, attrs, Some(e))),
            _ => None,
        }
    }
    /// The first newline at or after scalar `at` in a body: its position and attributes.
    pub fn newline_from(&self, at: u32) -> Option<(u32, &Attrs)> {
        let (mut pos, mut skip) = (0u32, at);
        for part in &self.ops {
            let n = part.len();
            if let Component::Insert {
                insert: Insert::Text(t),
                attrs,
            } = part
            {
                if skip < n {
                    if let Some(k) = t.chars().skip(skip as usize).position(|c| c == '\n') {
                        return Some((pos + skip + k as u32, attrs));
                    }
                }
            }
            pos += n;
            skip = skip.saturating_sub(n);
        }
        None
    }
    /// A UTF-16 offset in a body as a scalar position, as `scalar_from_utf16` reads the body's text.
    pub fn scalar_from_utf16(&self, offset: usize) -> u32 {
        let (mut units, mut index) = (0usize, 0u32);
        for part in &self.ops {
            if units >= offset {
                return index;
            }
            match part {
                Component::Insert {
                    insert: Insert::Text(t),
                    ..
                } => {
                    let u = t.encode_utf16().count();
                    if units + u <= offset {
                        units += u;
                        index += t.chars().count() as u32;
                        continue;
                    }
                    for ch in t.chars() {
                        if units >= offset {
                            return index;
                        }
                        units += ch.len_utf16();
                        index += 1;
                    }
                }
                Component::Insert { .. } => {
                    units += 1;
                    index += 1;
                }
                _ => {}
            }
        }
        index
    }
    /// A scalar position in a body as a UTF-16 offset.
    pub fn utf16_from_scalar(&self, at: u32) -> usize {
        let (mut units, mut rest) = (0usize, at);
        for part in &self.ops {
            if rest == 0 {
                break;
            }
            match part {
                Component::Insert {
                    insert: Insert::Text(t),
                    ..
                } => {
                    let n = t.chars().count() as u32;
                    if rest >= n {
                        units += t.encode_utf16().count();
                        rest -= n;
                    } else {
                        units += t
                            .chars()
                            .take(rest as usize)
                            .map(char::len_utf16)
                            .sum::<usize>();
                        rest = 0;
                    }
                }
                Component::Insert { .. } => {
                    units += 1;
                    rest -= 1;
                }
                _ => {}
            }
        }
        units
    }
    /// The delta that undoes this one, against the body it was applied to.
    pub fn invert(&self, base: &Delta) -> Result<Delta, &'static str> {
        let mut at = Walk {
            ops: &base.ops,
            i: 0,
            byte: 0,
        };
        let mut out = Delta::new();
        for part in &self.ops {
            match part {
                Component::Retain { retain, attrs } if attrs.is_empty() => {
                    at.skip(*retain)?;
                    out = out.retain(*retain);
                }
                Component::Retain { retain, attrs } => {
                    for _ in 0..*retain {
                        let old = at.next()?.1;
                        out = out.retain_with(
                            1,
                            attrs
                                .keys()
                                .map(|k| (k.clone(), old.get(k).cloned().unwrap_or(Value::Null)))
                                .collect(),
                        );
                    }
                }
                Component::Delete { delete } => {
                    for _ in 0..*delete {
                        let (ch, attrs, embed) = at.next()?;
                        out = match embed {
                            Some(e) => out.embed(e.clone(), attrs.clone()),
                            None => out.insert_with(ch.encode_utf8(&mut [0; 4]), attrs.clone()),
                        };
                    }
                }
                Component::Insert { insert, .. } => out = out.delete(insert.len()),
            }
        }
        Ok(out)
    }
    /// The embed object of this kind carrying this id.
    pub fn find_embed(&self, kind: &str, id: &str) -> Option<&Map<String, Value>> {
        self.ops.iter().find_map(|p| match p {
            Component::Insert {
                insert: Insert::Embed(Value::Object(fields)),
                ..
            } => fields
                .get(kind)?
                .as_object()
                .filter(|o| o.get("id").and_then(Value::as_str) == Some(id)),
            _ => None,
        })
    }
    /// The embed object of this kind carrying this id, for in-place edits.
    pub fn find_embed_mut(&mut self, kind: &str, id: &str) -> Option<&mut Map<String, Value>> {
        self.ops.iter_mut().find_map(|p| match p {
            Component::Insert {
                insert: Insert::Embed(Value::Object(fields)),
                ..
            } => fields
                .get_mut(kind)?
                .as_object_mut()
                .filter(|o| o.get("id").and_then(Value::as_str) == Some(id)),
            _ => None,
        })
    }
    /// Transform two deltas against the same body: `(later', earlier')`; `earlier` wins insertion ties and attribute conflicts.
    pub fn transform(later: &Delta, earlier: &Delta) -> Result<(Delta, Delta), &'static str> {
        if later.is_empty() || earlier.is_empty() {
            return Ok((later.clone(), earlier.clone()));
        }
        if later.base_len() != earlier.base_len() {
            return Err("transform length mismatch");
        }
        let (mut a_prime, mut b_prime) = (Delta::new(), Delta::new());
        let (mut a, mut b) = (nonzero(&earlier.ops), nonzero(&later.ops));
        let (mut ca, mut cb) = (a.next(), b.next());
        loop {
            match (ca.take(), cb.take()) {
                (None, None) => return Ok((b_prime, a_prime)),
                (Some(x @ Component::Insert { .. }), y) => {
                    b_prime = b_prime.retain(x.len());
                    a_prime = a_prime.push(x);
                    ca = a.next();
                    cb = y;
                }
                (x, Some(y @ Component::Insert { .. })) => {
                    a_prime = a_prime.retain(y.len());
                    b_prime = b_prime.push(y);
                    ca = x;
                    cb = b.next();
                }
                (None, _) | (_, None) => return Err("transform length mismatch"),
                (Some(mut x), Some(mut y)) => {
                    let (lx, ly) = (x.len(), y.len());
                    let n = lx.min(ly);
                    let (xs, ys) = (
                        if lx > n { x.split(n) } else { x.clone() },
                        if ly > n { y.split(n) } else { y.clone() },
                    );
                    match (xs, ys) {
                        (
                            Component::Retain { attrs: ax, .. },
                            Component::Retain { attrs: ay, .. },
                        ) => {
                            a_prime = a_prime.retain_with(n, ax.clone());
                            b_prime = b_prime.retain_with(n, transform_attrs(&ay, &ax));
                        }
                        (Component::Delete { .. }, Component::Retain { .. }) => {
                            a_prime = a_prime.delete(n)
                        }
                        (Component::Retain { .. }, Component::Delete { .. }) => {
                            b_prime = b_prime.delete(n)
                        }
                        _ => {}
                    }
                    ca = if lx > n { Some(x) } else { a.next() };
                    cb = if ly > n { Some(y) } else { b.next() };
                }
            }
        }
    }
    /// Move a cursor (scalar offset) through this delta.
    pub fn transform_cursor(&self, cursor: u32) -> u32 {
        let (mut index, mut result) = (0u32, cursor);
        for part in &self.ops {
            match part {
                Component::Retain { retain, .. } => index = index.saturating_add(*retain),
                Component::Insert { insert, .. } => {
                    if index < cursor {
                        result = result.saturating_add(insert.len());
                    }
                }
                Component::Delete { delete } => {
                    if index < cursor {
                        result = result.saturating_sub((*delete).min(cursor - index));
                    }
                    index = index.saturating_add(*delete);
                }
            }
        }
        result
    }
}

/// Parts that read or write at least one scalar, so `split` never meets an empty part.
fn nonzero(ops: &[Component]) -> impl Iterator<Item = Component> + '_ {
    ops.iter().filter(|p| p.len() > 0).cloned()
}
fn bytes(ops: &[Component]) -> usize {
    ops.iter()
        .map(|p| match p {
            Component::Insert {
                insert: Insert::Text(t),
                ..
            } => t.len(),
            _ => 1,
        })
        .sum()
}

/// Reads a body's scalars without collecting them.
struct Walk<'a> {
    ops: &'a [Component],
    i: usize,
    byte: usize,
}
impl<'a> Walk<'a> {
    fn skip(&mut self, mut n: u32) -> Result<(), &'static str> {
        while n > 0 {
            match self.ops.get(self.i).ok_or("invert past the body")? {
                Component::Insert {
                    insert: Insert::Text(t),
                    ..
                } => {
                    let rest = &t[self.byte..];
                    let count = rest.chars().count() as u32;
                    if count <= n {
                        n -= count;
                        (self.i, self.byte) = (self.i + 1, 0);
                    } else {
                        self.byte += rest
                            .char_indices()
                            .nth(n as usize)
                            .map_or(rest.len(), |(b, _)| b);
                        n = 0;
                    }
                }
                Component::Insert { .. } => {
                    n -= 1;
                    self.i += 1;
                }
                _ => self.i += 1,
            }
        }
        Ok(())
    }
    fn next(&mut self) -> Result<(char, &'a Attrs, Option<&'a Value>), &'static str> {
        loop {
            match self.ops.get(self.i).ok_or("invert past the body")? {
                Component::Insert {
                    insert: Insert::Text(t),
                    attrs,
                } => {
                    if let Some(ch) = t[self.byte..].chars().next() {
                        self.byte += ch.len_utf8();
                        if self.byte >= t.len() {
                            (self.i, self.byte) = (self.i + 1, 0);
                        }
                        return Ok((ch, attrs, None));
                    }
                    (self.i, self.byte) = (self.i + 1, 0);
                }
                Component::Insert {
                    insert: Insert::Embed(e),
                    attrs,
                } => {
                    self.i += 1;
                    return Ok((EMBED_CHAR, attrs, Some(e)));
                }
                _ => self.i += 1,
            }
        }
    }
}

pub fn scalar_len(source: &str) -> u32 {
    source.chars().count() as u32
}
/// Convert a UTF-16 offset from the editor into a scalar position.
pub fn scalar_from_utf16(source: &str, offset: usize) -> u32 {
    let mut units = 0;
    for (index, ch) in source.chars().enumerate() {
        if units >= offset {
            return index as u32;
        }
        units += ch.len_utf16();
    }
    source.chars().count() as u32
}
pub fn utf16_from_scalar(source: &str, at: u32) -> usize {
    source.chars().take(at as usize).map(char::len_utf16).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn body(s: &str) -> Delta {
        Delta::new().insert(s)
    }
    fn attrs(v: Value) -> Attrs {
        serde_json::from_value(v).unwrap()
    }
    fn converge(base: &Delta, a: Delta, b: Delta) -> Delta {
        let (a2, b2) = Delta::transform(&a, &b).unwrap();
        let left = b2.apply(&a.apply(base).unwrap()).unwrap();
        let right = a2.apply(&b.apply(base).unwrap()).unwrap();
        assert_eq!(left, right);
        left
    }
    #[test]
    fn concurrent_inserts_converge_with_earlier_first() {
        let a = Delta::replace(11, 5, 5, ",").unwrap();
        let b = Delta::replace(11, 11, 11, "!").unwrap();
        assert_eq!(converge(&body("hello world"), a, b).text(), "hello, world!");
        let same_spot_later = Delta::replace(3, 1, 1, "L").unwrap();
        let same_spot_earlier = Delta::replace(3, 1, 1, "E").unwrap();
        assert_eq!(
            converge(&body("abc"), same_spot_later, same_spot_earlier).text(),
            "aELbc"
        );
    }
    #[test]
    fn overlapping_deletes_and_inserts_converge() {
        assert_eq!(
            converge(
                &body("abcdefgh"),
                Delta::replace(8, 1, 5, "").unwrap(),
                Delta::replace(8, 3, 7, "").unwrap()
            )
            .text(),
            "ah"
        );
        assert_eq!(
            converge(
                &body("abcdef"),
                Delta::replace(6, 1, 5, "").unwrap(),
                Delta::replace(6, 3, 3, "XY").unwrap()
            )
            .text(),
            "aXYf"
        );
    }
    #[test]
    fn compose_matches_sequential_application() {
        let base = body("The quick fox");
        let a = Delta::replace(13, 4, 9, "slow").unwrap();
        let b = Delta::replace(12, 12, 12, " sleeps").unwrap();
        let composed = a.compose(&b).unwrap();
        assert_eq!(
            composed.apply(&base).unwrap(),
            b.apply(&a.apply(&base).unwrap()).unwrap()
        );
        assert_eq!(composed.apply(&base).unwrap().text(), "The slow fox sleeps");
    }
    #[test]
    fn cursors_follow_edits() {
        let op = Delta::replace(10, 2, 4, "abcd").unwrap();
        assert_eq!(op.transform_cursor(1), 1);
        assert_eq!(op.transform_cursor(3), 2);
        assert_eq!(op.transform_cursor(8), 10);
    }
    #[test]
    fn scalar_positions_handle_astral_characters() {
        let s = "a😀b";
        assert_eq!(scalar_from_utf16(s, 3), 2);
        assert_eq!(utf16_from_scalar(s, 2), 3);
        assert_eq!(
            Delta::replace(3, 1, 2, "")
                .unwrap()
                .apply(&body(s))
                .unwrap()
                .text(),
            "ab"
        );
    }
    #[test]
    fn serializes_compactly() {
        let op = Delta::replace(5, 1, 2, "x").unwrap();
        assert_eq!(
            serde_json::to_string(&op).unwrap(),
            r#"[{"retain":1},{"delete":1},{"insert":"x"},{"retain":3}]"#
        );
        assert_eq!(
            serde_json::from_str::<Delta>(&serde_json::to_string(&op).unwrap()).unwrap(),
            op
        );
        let styled = Delta::new()
            .retain_with(2, attrs(json!({"bold": true})))
            .embed(json!({"hr": true}), Attrs::new());
        let text = serde_json::to_string(&styled).unwrap();
        assert_eq!(
            text,
            r#"[{"retain":2,"attrs":{"bold":true}},{"insert":{"hr":true}}]"#
        );
        assert_eq!(serde_json::from_str::<Delta>(&text).unwrap(), styled);
    }
    #[test]
    fn compose_merges_attributes_and_null_clears() {
        let base = body("abcd\n");
        let bold = Delta::new()
            .retain_with(4, attrs(json!({"bold": true, "italic": true})))
            .retain(1);
        let unbold = Delta::new()
            .retain(1)
            .retain_with(2, attrs(json!({"bold": null, "color": "red3"})))
            .retain(2);
        let composed = bold.compose(&unbold).unwrap();
        assert_eq!(
            composed.ops[1],
            Component::Retain {
                retain: 2,
                attrs: attrs(json!({"bold": null, "color": "red3", "italic": true}))
            }
        );
        let doc = composed.apply(&base).unwrap();
        assert_eq!(doc, unbold.apply(&bold.apply(&base).unwrap()).unwrap());
        let scalars = doc.scalars();
        assert_eq!(*scalars[0].1, attrs(json!({"bold": true, "italic": true})));
        assert_eq!(
            *scalars[1].1,
            attrs(json!({"color": "red3", "italic": true}))
        );
        assert!(scalars[4].1.is_empty());
        assert_eq!(doc.ops.len(), 4);
    }
    #[test]
    fn attribute_transform_lets_the_earlier_op_win() {
        let base = body("abc");
        let later = Delta::new().retain_with(3, attrs(json!({"bold": true, "size": 12})));
        let earlier = Delta::new()
            .retain_with(2, attrs(json!({"bold": null, "italic": true})))
            .retain(1);
        let doc = converge(&base, later.clone(), earlier.clone());
        let scalars = doc.scalars();
        assert_eq!(*scalars[0].1, attrs(json!({"italic": true, "size": 12})));
        assert_eq!(*scalars[2].1, attrs(json!({"bold": true, "size": 12})));
        let (later2, earlier2) = Delta::transform(&later, &earlier).unwrap();
        assert_eq!(
            later2.ops[0],
            Component::Retain {
                retain: 2,
                attrs: attrs(json!({"size": 12}))
            }
        );
        assert_eq!(earlier2, earlier);
    }
    #[test]
    fn inserts_carry_attributes_through_transform_and_deletes_drop_formatting() {
        let base = body("abcd");
        let insert = Delta::new()
            .retain(2)
            .insert_with("X", attrs(json!({"bold": true})))
            .retain(2);
        let format = Delta::new().retain_with(4, attrs(json!({"italic": true})));
        let doc = converge(&base, insert.clone(), format);
        assert_eq!(doc.text(), "abXcd");
        assert_eq!(*doc.scalars()[2].1, attrs(json!({"bold": true})));
        assert_eq!(*doc.scalars()[3].1, attrs(json!({"italic": true})));
        let delete = Delta::replace(4, 1, 3, "").unwrap();
        let format = Delta::new()
            .retain(1)
            .retain_with(2, attrs(json!({"bold": true})))
            .retain(1);
        assert_eq!(converge(&base, format, delete).text(), "ad");
    }
    #[test]
    fn embeds_are_one_scalar() {
        let base = Delta::new()
            .insert("a")
            .embed(
                json!({"image": {"id": "f1", "width": 10, "height": 5}}),
                Attrs::new(),
            )
            .insert("b\n");
        assert_eq!(base.target_len(), 4);
        assert_eq!(base.text(), format!("a{EMBED_CHAR}b\n"));
        let cut = Delta::replace(4, 1, 2, "").unwrap().apply(&base).unwrap();
        assert_eq!(cut.text(), "ab\n");
        let link = Delta::new()
            .retain(1)
            .retain_with(1, attrs(json!({"link": "https://example.test"})))
            .retain(2);
        let linked = link.apply(&base).unwrap();
        assert_eq!(
            linked.scalars()[1].2,
            Some(&json!({"image": {"id": "f1", "width": 10, "height": 5}}))
        );
        assert_eq!(
            *linked.scalars()[1].1,
            attrs(json!({"link": "https://example.test"}))
        );
        assert_eq!(
            converge(
                &base,
                Delta::replace(4, 1, 1, "x").unwrap(),
                Delta::replace(4, 2, 2, "y").unwrap()
            )
            .text(),
            format!("ax{EMBED_CHAR}yb\n")
        );
    }
    #[test]
    fn invert_restores_text_embeds_and_attributes() {
        let base = Delta::new()
            .insert_with("bold", attrs(json!({"bold": true})))
            .embed(json!({"hr": true}), Attrs::new())
            .insert(" plain\n");
        let op = Delta::new()
            .retain(2)
            .delete(3)
            .insert("X")
            .retain_with(3, attrs(json!({"bold": null, "italic": true})))
            .retain(4);
        let edited = op.apply(&base).unwrap();
        assert_eq!(edited.text(), "boX pl".to_owned() + "ain\n");
        let back = op.invert(&base).unwrap().apply(&edited).unwrap();
        assert_eq!(back, base, "inverting restores the body exactly");
        assert_eq!(back.scalars()[4].2, Some(&json!({"hr": true})));
        let format = Delta::new()
            .retain_with(4, attrs(json!({"bold": null, "color": "red3"})))
            .retain(8);
        let formatted = format.apply(&base).unwrap();
        assert!(formatted.scalars()[0].1.get("bold").is_none());
        assert_eq!(
            format.invert(&base).unwrap().apply(&formatted).unwrap(),
            base
        );
    }
    #[test]
    fn in_place_apply_matches_compose_and_reads_match_scalars() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = |n: u32| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as u32
        };
        let styles = [
            Attrs::new(),
            attrs(json!({"bold": true})),
            attrs(json!({"italic": true, "size": 14})),
        ];
        for _ in 0..400 {
            let mut body = Delta::new();
            for _ in 0..1 + next(8) {
                body = match next(5) {
                    0 => body.embed(json!({"hr": true}), styles[next(3) as usize].clone()),
                    1 => body.insert_with("\n", styles[next(3) as usize].clone()),
                    _ => body.insert_with(
                        ["ab", "cdé", "f😀g", "hij\nk"][next(4) as usize],
                        styles[next(3) as usize].clone(),
                    ),
                };
            }
            let len = body.target_len();
            let mut edit = Delta::new();
            let mut left = len;
            while left > 0 {
                let n = 1 + next(left.min(4));
                edit = match next(5) {
                    0 => edit.delete(n),
                    1 => edit.retain_with(n, styles[next(3) as usize].clone()),
                    2 => edit
                        .insert_with(
                            ["x", "\n", "yz"][next(3) as usize],
                            styles[next(3) as usize].clone(),
                        )
                        .retain(n),
                    _ => edit.retain(n),
                };
                left -= n;
            }
            if next(3) == 0 {
                edit = edit.insert("tail");
            }
            let whole = edit.apply(&body).unwrap();
            let mut in_place = body.clone();
            if let Err(e) = edit.apply_to(&mut in_place) {
                panic!("{e}: {body:?} then {edit:?}");
            }
            assert_eq!(in_place, whole, "{body:?} then {edit:?}");
            assert_eq!(edit.invert(&body).unwrap().apply(&whole).unwrap(), body);
            let scalars = whole.scalars();
            let text = whole.text();
            for i in 0..=scalars.len() as u32 {
                assert_eq!(
                    whole.scalar_at(i).map(|s| (s.0, s.1.clone())),
                    scalars.get(i as usize).map(|s| (s.0, s.1.clone()))
                );
                assert_eq!(
                    whole.newline_from(i).map(|n| n.0),
                    text.chars()
                        .enumerate()
                        .skip(i as usize)
                        .find(|c| c.1 == '\n')
                        .map(|c| c.0 as u32)
                );
                let units = utf16_from_scalar(&text, i);
                assert_eq!(whole.utf16_from_scalar(i), units);
                assert_eq!(
                    whole.scalar_from_utf16(units),
                    scalar_from_utf16(&text, units)
                );
                assert_eq!(
                    whole.scalar_from_utf16(units + 1),
                    scalar_from_utf16(&text, units + 1)
                );
            }
        }
    }
    #[test]
    fn wire_deltas_read_as_push_builds_them() {
        let read = |v: Value| serde_json::from_value::<Delta>(v);
        assert_eq!(
            read(json!([{ "retain": 0 }, { "retain": 1 }, { "retain": 2 }, { "delete": 0 }, { "insert": "" }, { "insert": "a" }, { "insert": "b", "attrs": {} }])).unwrap(),
            Delta::new().retain(3).insert("ab")
        );
        assert_eq!(
            read(json!([{ "insert": "x", "attrs": { "bold": null } }])).unwrap(),
            Delta::new().insert("x")
        );
        for bad in [
            json!([{ "retain": 1, "delete": 4 }]),
            json!([{ "insert": "x", "retain": 9 }]),
            json!([{ "delete": 1, "attrs": { "bold": true } }]),
            json!([{ "retain": 1, "attrs": 3 }]),
            json!([{ "retain": -1 }]),
            json!([{}]),
            json!([{ "attrs": {} }]),
        ] {
            assert!(read(bad.clone()).is_err(), "{bad}");
        }
    }
    #[test]
    fn zero_length_parts_never_split_an_embed() {
        let base = Delta::new()
            .embed(json!({ "hr": true }), Attrs::new())
            .insert("\n");
        let zero = Component::Retain {
            retain: 0,
            attrs: Attrs::new(),
        };
        let built = Delta {
            ops: vec![
                zero.clone(),
                Component::Delete { delete: 1 },
                zero.clone(),
                Component::Retain {
                    retain: 1,
                    attrs: Attrs::new(),
                },
            ],
        };
        assert_eq!(built.apply(&base).unwrap().text(), "\n");
        let mut in_place = base.clone();
        built.apply_to(&mut in_place).unwrap();
        assert_eq!(in_place.text(), "\n");
        let other = Delta {
            ops: vec![
                zero,
                Component::Retain {
                    retain: 2,
                    attrs: Attrs::new(),
                },
                Component::Delete { delete: 0 },
            ],
        };
        assert!(Delta::transform(&built, &other).is_ok());
        assert!(Delta::transform(&other, &built).is_ok());
    }
    #[test]
    fn overflowing_lengths_saturate_and_match_no_body() {
        let huge = Delta::new().retain(u32::MAX).retain(2);
        assert_eq!(huge.ops.len(), 2, "an overflowing merge stays two parts");
        assert_eq!((huge.base_len(), huge.target_len()), (u32::MAX, u32::MAX));
        let mut b = body("\n");
        assert!(huge.apply_to(&mut b).is_err());
        assert!(huge.apply(&body("\n")).is_err());
        assert!(Delta::transform(&Delta::new().insert("x").retain(1), &huge).is_err());
        assert_eq!(huge.transform_cursor(5), 5);
        let deletes = Delta::new().delete(u32::MAX).delete(3);
        assert_eq!(deletes.base_len(), u32::MAX);
    }
    #[test]
    fn size_limits_differ_for_authors_and_replay() {
        let base = body("\n");
        let big = Delta::new().insert(&"a".repeat(MAX_BODY)).retain(1);
        assert_eq!(big.apply_within(&base, MAX_BODY), Err("document too large"));
        let mut b = base.clone();
        assert_eq!(
            big.apply_to_within(&mut b, MAX_BODY),
            Err("document too large")
        );
        assert_eq!(b, base);
        assert_eq!(big.apply(&base).unwrap().size(), MAX_BODY + 1);
        let past = Delta::new().insert(&"a".repeat(MAX_REPLAY)).retain(1);
        assert_eq!(past.apply(&base), Err("document too large"));
    }
    #[test]
    fn newline_carries_paragraph_attributes() {
        let base = Delta::new()
            .insert("Title")
            .insert_with("\n", attrs(json!({"heading": 1})))
            .insert("Body\n");
        let split = Delta::new()
            .retain(2)
            .insert_with("\n", attrs(json!({"heading": 1})))
            .retain(9);
        let doc = split.apply(&base).unwrap();
        let scalars = doc.scalars();
        assert_eq!(doc.text(), "Ti\ntle\nBody\n");
        assert_eq!(*scalars[2].1, attrs(json!({"heading": 1})));
        assert_eq!(*scalars[6].1, attrs(json!({"heading": 1})));
        assert!(scalars[11].1.is_empty());
        let bold = Delta::new()
            .retain_with(5, attrs(json!({"bold": true})))
            .retain(6);
        let styled = bold.apply(&base).unwrap();
        assert_eq!(*styled.scalars()[5].1, attrs(json!({"heading": 1})));
    }
}
