//! Page ops: body and cell deltas, table lines, block and body sets, metadata; transform, apply, invert.
use crate::delta::{Component, Delta, Insert, MAX_BODY, MAX_REPLAY};
use crate::page::{
    embed_key, embed_ok, embeds, ids_unique, inserts_embed, inserts_keyed, Kind, Page, MAX_TITLE,
};
use serde::de::{DeserializeOwned, Error as _};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Insert,
    Delete,
}

/// A row or column inserted or deleted in one table; `width` is a new column's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Line {
    pub table: String,
    pub at: u32,
    pub action: Action,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<Value>,
}
impl Line {
    pub fn new(table: &str, at: u32, action: Action) -> Self {
        Line {
            table: table.into(),
            at,
            action,
            width: None,
        }
    }
    fn inserts(&self) -> bool {
        self.action == Action::Insert
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellRef {
    pub table: String,
    pub row: u32,
    pub col: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Block {
    pub id: String,
    pub source: String,
}

/// Metadata fields to set; absent ones keep their value.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MetaOp {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<Kind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
}

/// Serialized as one object whose keys name the variant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Op {
    Delta {
        delta: Delta,
    },
    Cell {
        cell: CellRef,
        delta: Delta,
    },
    TableRow {
        table_row: Line,
    },
    TableCol {
        table_col: Line,
    },
    BlockSet {
        block_set: Block,
    },
    /// Content-carrying: voids every body op it meets.
    BodySet {
        body_set: Delta,
    },
    Meta {
        meta: MetaOp,
    },
}
fn take<T: DeserializeOwned, E: serde::de::Error>(
    map: &mut Map<String, Value>,
    key: &str,
) -> Result<T, E> {
    serde_json::from_value(map.remove(key).unwrap_or_default()).map_err(E::custom)
}
impl<'de> Deserialize<'de> for Op {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut map = Map::<String, Value>::deserialize(d)?;
        let mut keys: Vec<&str> = map.keys().map(String::as_str).collect();
        keys.sort_unstable();
        Ok(match keys.as_slice() {
            ["delta"] => Op::Delta {
                delta: take(&mut map, "delta")?,
            },
            ["cell", "delta"] => Op::Cell {
                cell: take(&mut map, "cell")?,
                delta: take(&mut map, "delta")?,
            },
            ["table_row"] => Op::TableRow {
                table_row: take(&mut map, "table_row")?,
            },
            ["table_col"] => Op::TableCol {
                table_col: take(&mut map, "table_col")?,
            },
            ["block_set"] => Op::BlockSet {
                block_set: take(&mut map, "block_set")?,
            },
            ["body_set"] => Op::BodySet {
                body_set: take(&mut map, "body_set")?,
            },
            ["meta"] => Op::Meta {
                meta: take(&mut map, "meta")?,
            },
            _ => return Err(D::Error::custom("unknown page op")),
        })
    }
}
impl Op {
    pub fn delta(delta: Delta) -> Self {
        Op::Delta { delta }
    }
    pub fn cell(table: &str, row: u32, col: u32, delta: Delta) -> Self {
        Op::Cell {
            cell: CellRef {
                table: table.into(),
                row,
                col,
            },
            delta,
        }
    }
    /// The identity: what a voided op becomes.
    pub fn noop() -> Self {
        Op::delta(Delta::new())
    }
    pub fn is_noop(&self) -> bool {
        match self {
            Op::Delta { delta } | Op::Cell { delta, .. } => delta.is_noop(),
            Op::Meta { meta } => *meta == MetaOp::default(),
            _ => false,
        }
    }
    /// Whether the op touches the body; only metadata does not.
    fn on_body(&self) -> bool {
        !matches!(self, Op::Meta { .. })
    }
    /// The keyed embed the op edits.
    fn target(&self) -> Option<(&'static str, &str)> {
        match self {
            Op::Cell { cell, .. } => Some(("table", &cell.table)),
            Op::TableRow { table_row: l } | Op::TableCol { table_col: l } => {
                Some(("table", &l.table))
            }
            Op::BlockSet { block_set } => Some(("block", &block_set.id)),
            _ => None,
        }
    }
}

/// The table embed with this id.
pub fn table<'a>(page: &'a Page, id: &str) -> Option<&'a Map<String, Value>> {
    page.body.find_embed("table", id)
}
/// One table cell as a delta.
pub fn table_cell(page: &Page, id: &str, row: u32, col: u32) -> Option<Delta> {
    serde_json::from_value(
        table(page, id)?
            .get("rows")?
            .get(row as usize)?
            .get(col as usize)?
            .clone(),
    )
    .ok()
}
/// Rows and columns of a table.
pub fn table_shape(page: &Page, id: &str) -> Option<(usize, usize)> {
    let t = table(page, id)?;
    Some((
        t.get("rows")?.as_array()?.len(),
        t.get("widths")
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
    ))
}
/// A block embed's source.
pub fn block_source<'a>(page: &'a Page, id: &str) -> Option<&'a str> {
    page.body.find_embed("block", id)?.get("source")?.as_str()
}
/// Cells of one row or column.
fn table_line(page: &Page, line: &Line, row: bool) -> Vec<Delta> {
    let Some(rows) = table(page, &line.table).and_then(|t| t.get("rows")?.as_array()) else {
        return Vec::new();
    };
    let cells: Vec<&Value> = match row {
        true => rows
            .get(line.at as usize)
            .and_then(Value::as_array)
            .map(|r| r.iter().collect())
            .unwrap_or_default(),
        false => rows
            .iter()
            .filter_map(|r| r.get(line.at as usize))
            .collect(),
    };
    cells
        .into_iter()
        .map(|c| serde_json::from_value(c.clone()).unwrap_or_default())
        .collect()
}
/// Inserts or deletes `at` in `items`; out of range does nothing.
fn splice(items: &mut Vec<Value>, at: usize, line: &Line, fill: impl FnOnce() -> Value) {
    match line.inserts() {
        true if at <= items.len() => items.insert(at, fill()),
        false if at < items.len() => {
            items.remove(at);
        }
        _ => {}
    }
}

/// Applies one op from the log whole or not at all; ops naming a missing table, cell or block do nothing.
pub fn apply(page: &mut Page, op: &Op) -> Result<(), &'static str> {
    apply_within(page, op, MAX_REPLAY)
}
/// Applies one op this device is making, under the author's size limit.
pub fn apply_local(page: &mut Page, op: &Op) -> Result<(), &'static str> {
    apply_within(page, op, MAX_BODY)
}
fn apply_within(page: &mut Page, op: &Op, limit: usize) -> Result<(), &'static str> {
    match op {
        Op::Delta { delta } if embeds(delta).next().is_some() => {
            if !embeds(delta).all(embed_ok) {
                return Err("invalid embed");
            }
            let next = delta.apply_within(&page.body, limit)?;
            if !ids_unique(&next) {
                return Err("duplicate embed id");
            }
            page.body = next;
        }
        Op::Delta { delta } => delta.apply_to_within(&mut page.body, limit)?,
        Op::BodySet { body_set } => {
            let next = Delta::replace_all(0, body_set).apply_within(&Delta::new(), limit)?;
            if !embeds(&next).all(embed_ok) {
                return Err("invalid embed");
            }
            if !ids_unique(&next) {
                return Err("duplicate embed id");
            }
            page.body = next;
        }
        Op::Cell { cell, delta } => {
            if inserts_keyed(delta) || !embeds(delta).all(embed_ok) {
                return Err("invalid embed in a table cell");
            }
            if let Some(body) = table_cell(page, &cell.table, cell.row, cell.col) {
                let next = serde_json::to_value(delta.apply_within(&body, limit)?)
                    .map_err(|_| "invalid table cell")?;
                let slot = page
                    .body
                    .find_embed_mut("table", &cell.table)
                    .and_then(|t| {
                        t.get_mut("rows")?
                            .get_mut(cell.row as usize)?
                            .get_mut(cell.col as usize)
                    });
                if let Some(slot) = slot {
                    *slot = next;
                }
            }
        }
        Op::TableRow { table_row: line } => {
            if let Some(t) = page.body.find_embed_mut("table", &line.table) {
                let cols = t
                    .get("widths")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
                if let Some(rows) = t.get_mut("rows").and_then(Value::as_array_mut) {
                    splice(rows, line.at as usize, line, || {
                        Value::Array(vec![json!([]); cols])
                    });
                }
            }
        }
        Op::TableCol { table_col: line } => {
            if line.width.as_ref().is_some_and(|w| !w.is_number()) {
                return Err("invalid column width");
            }
            if let Some(t) = page.body.find_embed_mut("table", &line.table) {
                let at = line.at as usize;
                if let Some(widths) = t.get_mut("widths").and_then(Value::as_array_mut) {
                    splice(widths, at, line, || {
                        line.width.clone().unwrap_or(json!(1.0))
                    });
                }
                for row in t
                    .get_mut("rows")
                    .and_then(Value::as_array_mut)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_array_mut)
                {
                    splice(row, at, line, || json!([]));
                }
            }
        }
        Op::BlockSet { block_set } => {
            if block_set.source.len() > MAX_BODY {
                return Err("block too large");
            }
            if let Some(b) = page.body.find_embed_mut("block", &block_set.id) {
                b.insert("source".into(), Value::String(block_set.source.clone()));
            }
        }
        Op::Meta { meta } => {
            if meta
                .title
                .as_ref()
                .is_some_and(|t| t.chars().count() > MAX_TITLE)
            {
                return Err("title too long");
            }
            if let Some(title) = &meta.title {
                page.meta.title = title.clone();
            }
            if let Some(kind) = meta.kind {
                page.meta.kind = kind;
            }
            if let Some(archived) = meta.archived {
                page.meta.archived = archived;
            }
        }
    }
    Ok(())
}
/// Applies a batch from the log whole or not at all.
pub fn apply_batch(page: &mut Page, ops: &[Op]) -> Result<(), &'static str> {
    batch_within(page, ops, MAX_REPLAY)
}
/// Applies a batch this device is making, under the author's size limit.
pub fn apply_local_batch(page: &mut Page, ops: &[Op]) -> Result<(), &'static str> {
    batch_within(page, ops, MAX_BODY)
}
fn batch_within(page: &mut Page, ops: &[Op], limit: usize) -> Result<(), &'static str> {
    if let [op] = ops {
        return apply_within(page, op, limit);
    }
    let mut next = page.clone();
    for op in ops {
        apply_within(&mut next, op, limit)?;
    }
    *page = next;
    Ok(())
}

/// An index past `line`; `None` when `line` deletes it.
fn shifted(index: u32, line: &Line) -> Option<u32> {
    match (line.inserts(), line.at) {
        (true, at) if at <= index => Some(index.saturating_add(1)),
        (false, at) if at == index => None,
        (false, at) if at < index => Some(index.saturating_sub(1)),
        _ => Some(index),
    }
}
/// Where an insert lands past `line`; `first` wins a tie.
fn insert_at(at: u32, line: &Line, first: bool) -> u32 {
    match (line.inserts(), line.at) {
        (true, i) if i < at || (i == at && first) => at.saturating_add(1),
        (false, d) if d < at => at.saturating_sub(1),
        _ => at,
    }
}
/// A line op past another on the same axis; `None` when its line is gone.
fn line_past(line: &Line, other: &Line, first: bool) -> Option<Line> {
    let at = if line.inserts() {
        Some(insert_at(line.at, other, first))
    } else {
        shifted(line.at, other)
    };
    at.map(|at| Line { at, ..line.clone() })
}
/// A cell op past a row or column change: it follows its cell, voided when the cell goes.
fn cell_past(cell: &CellRef, delta: &Delta, line: &Line, rows: bool) -> Op {
    let moved = if rows {
        shifted(cell.row, line).map(|row| CellRef {
            row,
            ..cell.clone()
        })
    } else {
        shifted(cell.col, line).map(|col| CellRef {
            col,
            ..cell.clone()
        })
    };
    moved.map_or_else(Op::noop, |cell| Op::Cell {
        cell,
        delta: delta.clone(),
    })
}
fn line_op(line: Option<Line>, rows: bool) -> Op {
    match (line, rows) {
        (None, _) => Op::noop(),
        (Some(table_row), true) => Op::TableRow { table_row },
        (Some(table_col), false) => Op::TableCol { table_col },
    }
}

/// Body deltas; when both insert one keyed embed, the earlier copy wins and the later is dropped.
fn transform_body(later: &Delta, earlier: &Delta) -> Result<(Delta, Delta), &'static str> {
    let (l, e) = Delta::transform(later, earlier)?;
    if !inserts_keyed(later) || !inserts_keyed(earlier) {
        return Ok((l, e));
    }
    let theirs: BTreeSet<_> = earlier
        .ops
        .iter()
        .filter_map(|p| match p {
            Component::Insert {
                insert: Insert::Embed(v),
                ..
            } => embed_key(v),
            _ => None,
        })
        .collect();
    let clash = |p: &Component| matches!(p, Component::Insert { insert: Insert::Embed(v), .. } if embed_key(v).is_some_and(|k| theirs.contains(&k)));
    // Positions of the dropped copies in the later delta's result, which the mirror deletes.
    let (mut at, mut gone) = (0u32, Vec::new());
    for p in &later.ops {
        if clash(p) {
            gone.push(at);
        }
        at = at.saturating_add(match p {
            Component::Insert { insert, .. } => insert.len(),
            Component::Retain { retain, .. } => *retain,
            Component::Delete { .. } => 0,
        });
    }
    if gone.is_empty() {
        return Ok((l, e));
    }
    let l = l
        .ops
        .into_iter()
        .filter(|p| !clash(p))
        .fold(Delta::new(), Delta::push);
    let (mut out, mut at, mut next) = (Delta::new(), 0u32, gone.into_iter().peekable());
    for p in e.ops {
        match p {
            Component::Retain { retain, attrs } => {
                let end = at.saturating_add(retain);
                while let Some(g) = next.next_if(|g| *g < end) {
                    out = out.retain_with(g - at, attrs.clone()).delete(1);
                    at = g + 1;
                }
                out = out.retain_with(end - at, attrs);
                at = end;
            }
            Component::Delete { delete } => {
                at = at.saturating_add(delete);
                out = out.delete(delete);
            }
            insert => out = out.push(insert),
        }
    }
    Ok((l, out))
}
/// `(later', earlier')` for two ops on one page; `earlier` wins ties and attribute conflicts.
pub fn transform(later: &Op, earlier: &Op) -> Result<(Op, Op), &'static str> {
    use Op::*;
    Ok(match (later, earlier) {
        (Delta { delta: a }, Delta { delta: b }) => {
            let (l, e) = transform_body(a, b)?;
            (Op::delta(l), Op::delta(e))
        }
        // Lengths differ only where the cell is missing and both are no-ops, so the later is voided.
        (Cell { cell: ca, delta: a }, Cell { cell: cb, delta: b }) if ca == cb => {
            match crate::delta::Delta::transform(a, b) {
                Ok((l, e)) => (
                    Cell {
                        cell: ca.clone(),
                        delta: l,
                    },
                    Cell {
                        cell: cb.clone(),
                        delta: e,
                    },
                ),
                Err(_) => (Op::noop(), earlier.clone()),
            }
        }
        (Cell { cell, delta }, TableRow { table_row: line }) if cell.table == line.table => {
            (cell_past(cell, delta, line, true), earlier.clone())
        }
        (Cell { cell, delta }, TableCol { table_col: line }) if cell.table == line.table => {
            (cell_past(cell, delta, line, false), earlier.clone())
        }
        (TableRow { table_row: line }, Cell { cell, delta }) if cell.table == line.table => {
            (later.clone(), cell_past(cell, delta, line, true))
        }
        (TableCol { table_col: line }, Cell { cell, delta }) if cell.table == line.table => {
            (later.clone(), cell_past(cell, delta, line, false))
        }
        (TableRow { table_row: l }, TableRow { table_row: e }) if l.table == e.table => (
            line_op(line_past(l, e, false), true),
            line_op(line_past(e, l, true), true),
        ),
        (TableCol { table_col: l }, TableCol { table_col: e }) if l.table == e.table => (
            line_op(line_past(l, e, false), false),
            line_op(line_past(e, l, true), false),
        ),
        // Whole replacements: the later one wins and the earlier is voided.
        (BlockSet { block_set: l }, BlockSet { block_set: e }) if l.id == e.id => {
            (later.clone(), Op::noop())
        }
        (BodySet { .. }, e) if e.on_body() => (later.clone(), Op::noop()),
        (l, BodySet { .. }) if l.on_body() => (Op::noop(), earlier.clone()),
        // A delta re-inserting a table or block carries its content and voids edits to the old one.
        (l, Delta { delta })
            if l.target()
                .is_some_and(|(kind, id)| inserts_embed(delta, kind, id)) =>
        {
            (Op::noop(), earlier.clone())
        }
        (Delta { delta }, e)
            if e.target()
                .is_some_and(|(kind, id)| inserts_embed(delta, kind, id)) =>
        {
            (later.clone(), Op::noop())
        }
        (Meta { meta: l }, Meta { meta: e }) => {
            let e = MetaOp {
                title: e.title.clone().filter(|_| l.title.is_none()),
                kind: e.kind.filter(|_| l.kind.is_none()),
                archived: e.archived.filter(|_| l.archived.is_none()),
            };
            (later.clone(), Meta { meta: e })
        }
        _ => (later.clone(), earlier.clone()),
    })
}
/// Transforms op sequences: `(later', earlier')`.
pub fn transform_batch(later: &[Op], earlier: &[Op]) -> Result<(Vec<Op>, Vec<Op>), &'static str> {
    let mut later = later.to_vec();
    let mut out = Vec::with_capacity(earlier.len());
    for e in earlier {
        let mut e = e.clone();
        for l in later.iter_mut() {
            let (l2, e2) = transform(l, &e)?;
            *l = l2;
            e = e2;
        }
        out.push(e);
    }
    Ok((later, out))
}

/// The ops undoing `op` on the page it is about to apply to; empty when it would change nothing.
pub fn invert(page: &Page, op: &Op) -> Result<Vec<Op>, &'static str> {
    Ok(match op {
        Op::Delta { delta } => vec![Op::delta(delta.invert(&page.body)?)],
        Op::BodySet { .. } => vec![Op::BodySet {
            body_set: page.body.clone(),
        }],
        Op::Cell { cell, delta } => match table_cell(page, &cell.table, cell.row, cell.col) {
            Some(body) => vec![Op::Cell {
                cell: cell.clone(),
                delta: delta.invert(&body)?,
            }],
            None => Vec::new(),
        },
        Op::TableRow { table_row: line } | Op::TableCol { table_col: line } => {
            let rows = matches!(op, Op::TableRow { .. });
            let Some((height, width)) = table_shape(page, &line.table) else {
                return Ok(Vec::new());
            };
            let len = if rows { height } else { width } as u32;
            if (line.inserts() && line.at > len) || (!line.inserts() && line.at >= len) {
                return Ok(Vec::new());
            }
            let mut back = Line::new(
                &line.table,
                line.at,
                if line.inserts() {
                    Action::Delete
                } else {
                    Action::Insert
                },
            );
            if !rows && !line.inserts() {
                back.width = table(page, &line.table)
                    .and_then(|t| t.get("widths")?.get(line.at as usize).cloned());
            }
            let mut out = vec![line_op(Some(back), rows)];
            if !line.inserts() {
                for (i, cell) in table_line(page, line, rows)
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| !c.is_empty())
                {
                    let (r, c) = if rows {
                        (line.at, i as u32)
                    } else {
                        (i as u32, line.at)
                    };
                    out.push(Op::cell(&line.table, r, c, Delta::replace_all(0, cell)));
                }
            }
            out
        }
        Op::BlockSet { block_set } => match block_source(page, &block_set.id) {
            Some(source) => vec![Op::BlockSet {
                block_set: Block {
                    id: block_set.id.clone(),
                    source: source.into(),
                },
            }],
            None => Vec::new(),
        },
        Op::Meta { meta } => {
            let m = &page.meta;
            vec![Op::Meta {
                meta: MetaOp {
                    title: meta.title.as_ref().map(|_| m.title.clone()),
                    kind: meta.kind.map(|_| m.kind),
                    archived: meta.archived.map(|_| m.archived),
                },
            }]
        }
    })
}
/// The ops undoing a batch, last op first.
pub fn invert_batch(page: &Page, ops: &[Op]) -> Result<Vec<Op>, &'static str> {
    let mut at = page.clone();
    let mut groups = Vec::with_capacity(ops.len());
    for op in ops {
        groups.push(invert(&at, op)?);
        apply(&mut at, op)?;
    }
    Ok(groups.into_iter().rev().flatten().collect())
}

#[cfg(test)]
#[allow(clippy::cloned_ref_to_slice_refs)]
mod tests {
    use super::*;
    use crate::delta::Attrs;
    fn op(v: Value) -> Op {
        serde_json::from_value(v).unwrap()
    }
    fn page(body: Delta) -> Page {
        Page {
            body,
            ..Page::new(Kind::Page, "Doc")
        }
    }
    fn apply_all(page: &mut Page, ops: &[Op]) {
        for o in ops {
            apply(page, o).unwrap();
        }
    }
    /// Both orders of a concurrent pair reach one page, which is returned.
    fn converge(base: &Page, later: &[Op], earlier: &[Op]) -> Page {
        let (l2, e2) = transform_batch(later, earlier).unwrap();
        let mut left = base.clone();
        apply_all(&mut left, earlier);
        apply_all(&mut left, &l2);
        let mut right = base.clone();
        apply_all(&mut right, later);
        apply_all(&mut right, &e2);
        assert_eq!(left, right);
        left
    }
    fn table_value(id: &str) -> Value {
        json!({ "table": { "id": id, "widths": [2.0, 2.0], "rows": [[[{ "insert": "a" }], [{ "insert": "b" }]], [[], []]] } })
    }
    fn with_table(id: &str) -> Page {
        page(
            Delta::new()
                .embed(table_value(id), Attrs::new())
                .insert("\n"),
        )
    }
    fn undoes(base: &Page, ops: &[Op]) {
        let back = invert_batch(base, ops).unwrap();
        let mut p = base.clone();
        apply_all(&mut p, ops);
        apply_all(&mut p, &back);
        assert_eq!(&p, base, "{ops:?}");
    }

    #[test]
    fn ops_serialize_as_documented() {
        for v in [
            json!({ "delta": [{ "insert": "hi", "attrs": { "bold": true } }] }),
            json!({ "cell": { "table": "t", "row": 1, "col": 0 }, "delta": [{ "retain": 2 }] }),
            json!({ "table_row": { "table": "t", "at": 0, "action": "insert" } }),
            json!({ "table_col": { "table": "t", "at": 2, "action": "insert", "width": 1.5 } }),
            json!({ "block_set": { "id": "b", "source": "chart::A|1" } }),
            json!({ "body_set": [{ "insert": "x\n" }] }),
            json!({ "meta": { "title": "Trip", "kind": "list", "archived": true } }),
            json!({ "meta": {} }),
        ] {
            assert_eq!(serde_json::to_value(op(v.clone())).unwrap(), v);
        }
        assert!(matches!(op(json!({ "delta": [] })), Op::Delta { .. }));
        for bad in [
            json!({ "tab": "a", "delta": [] }),
            json!({ "delta": [], "extra": 1 }),
            json!({ "cell": { "table": "t", "row": 0, "col": 0, "x": 1 }, "delta": [] }),
            json!({ "table_row": { "table": "t", "at": 0, "action": "move" } }),
            json!({ "meta": { "colour": "red" } }),
            json!({ "meta": { "kind": "wiki" } }),
            json!({ "reset": { "period": 1 } }),
            json!([]),
        ] {
            assert!(serde_json::from_value::<Op>(bad.clone()).is_err(), "{bad}");
        }
    }
    #[test]
    fn table_cells_transform_like_bodies_and_follow_row_and_column_changes() {
        let mut doc = with_table("7f");
        let cell = |row, col, delta| Op::cell("7f", row, col, delta);
        let (mine, theirs) = (
            cell(0, 0, Delta::replace(1, 1, 1, "X").unwrap()),
            cell(0, 0, Delta::replace(1, 0, 0, "Y").unwrap()),
        );
        let left = converge(&doc, &[mine], &[theirs]);
        assert_eq!(table_cell(&left, "7f", 0, 0).unwrap().text(), "YaX");
        let insert = op(json!({ "table_row": { "table": "7f", "at": 0, "action": "insert" } }));
        assert_eq!(
            transform(&cell(1, 0, Delta::replace(1, 1, 1, "Z").unwrap()), &insert)
                .unwrap()
                .0,
            cell(2, 0, Delta::replace(1, 1, 1, "Z").unwrap())
        );
        let delete = op(json!({ "table_col": { "table": "7f", "at": 1, "action": "delete" } }));
        assert!(
            transform(&cell(0, 1, Delta::replace(1, 1, 1, "Z").unwrap()), &delete)
                .unwrap()
                .0
                .is_noop()
        );
        assert_eq!(
            transform(&cell(0, 0, Delta::replace(1, 1, 1, "Z").unwrap()), &delete)
                .unwrap()
                .0,
            cell(0, 0, Delta::replace(1, 1, 1, "Z").unwrap())
        );
        apply(&mut doc, &insert).unwrap();
        let back = invert(&doc, &delete).unwrap();
        apply(&mut doc, &delete).unwrap();
        assert_eq!(table_shape(&doc, "7f"), Some((3, 1)));
        assert_eq!(table(&doc, "7f").unwrap()["widths"], json!([2.0]));
        assert_eq!(table_cell(&doc, "7f", 1, 0).unwrap().text(), "a");
        apply_all(&mut doc, &back);
        assert_eq!(
            table_cell(&doc, "7f", 1, 1).unwrap().text(),
            "b",
            "the deleted column comes back with its cells"
        );
        assert_eq!(table(&doc, "7f").unwrap()["widths"], json!([2.0, 2.0]));
    }
    #[test]
    fn batches_transform_in_sequence() {
        let base = Page::new(Kind::Page, "Doc");
        let mine = [
            Op::delta(Delta::replace(1, 0, 0, "ab").unwrap()),
            Op::delta(Delta::replace(3, 2, 2, "c").unwrap()),
        ];
        let theirs = [Op::delta(Delta::replace(1, 0, 0, "X").unwrap())];
        assert_eq!(converge(&base, &mine, &theirs).body.text(), "Xabc\n");
    }
    #[test]
    fn two_devices_setting_one_body_do_not_double_it() {
        let base = page(Delta::new().insert("abcd\n"));
        let set = [Op::BodySet {
            body_set: Delta::new().insert("hello\n"),
        }];
        assert_eq!(converge(&base, &set, &set).body.text(), "hello\n");
    }
    #[test]
    fn a_body_set_then_an_edit_keeps_the_edit_against_another_set() {
        let base = page(Delta::new().insert("abcd\n"));
        let mine = [
            Op::BodySet {
                body_set: Delta::new().insert("one\n"),
            },
            Op::delta(Delta::replace(4, 3, 3, "!").unwrap()),
        ];
        let theirs = [Op::BodySet {
            body_set: Delta::new().insert("two\n"),
        }];
        assert_eq!(converge(&base, &mine, &theirs).body.text(), "one!\n");
        assert_eq!(converge(&base, &theirs, &mine).body.text(), "two\n");
    }
    #[test]
    fn a_body_set_voids_every_body_op_and_keeps_meta() {
        let base = with_table("t");
        let set = Op::BodySet {
            body_set: Delta::new().insert("fresh\n"),
        };
        let meta = Op::Meta {
            meta: MetaOp {
                title: Some("New".into()),
                ..Default::default()
            },
        };
        for other in [
            Op::delta(Delta::replace(2, 1, 1, "x").unwrap()),
            Op::cell("t", 0, 0, Delta::new().retain(1).insert("y")),
            Op::TableRow {
                table_row: Line::new("t", 0, Action::Delete),
            },
            Op::TableCol {
                table_col: Line::new("t", 0, Action::Insert),
            },
        ] {
            assert!(transform(&other, &set).unwrap().0.is_noop());
            assert!(transform(&set, &other).unwrap().1.is_noop());
            let end = converge(&base, &[other.clone(), meta.clone()], &[set.clone()]);
            assert_eq!(
                (end.body.text().as_str(), end.meta.title.as_str()),
                ("fresh\n", "New")
            );
        }
    }
    #[test]
    fn a_delta_carrying_a_table_voids_edits_to_the_old_one() {
        let base = with_table("t");
        let carries = Op::delta(
            Delta::new()
                .delete(1)
                .embed(table_value("t"), Attrs::new())
                .retain(1),
        );
        for edit in [
            Op::cell("t", 0, 0, Delta::new().retain(1).insert("y")),
            Op::TableRow {
                table_row: Line::new("t", 1, Action::Delete),
            },
            Op::TableCol {
                table_col: Line {
                    width: Some(json!(3.0)),
                    ..Line::new("t", 2, Action::Insert)
                },
            },
        ] {
            assert!(transform(&edit, &carries).unwrap().0.is_noop());
            assert!(transform(&carries, &edit).unwrap().1.is_noop());
            assert_eq!(
                converge(&base, &[edit.clone()], &[carries.clone()]),
                converge(&base, &[carries.clone()], &[edit])
            );
        }
    }
    #[test]
    fn two_deltas_inserting_one_table_keep_the_earlier_copy() {
        let base = page(Delta::new().insert("ab\n"));
        let at = |i: u32, rows: Value| {
            Op::delta(
                Delta::new()
                    .retain(i)
                    .embed(
                        json!({ "table": { "id": "t", "widths": [1.0], "rows": rows } }),
                        Attrs::new(),
                    )
                    .retain(3 - i),
            )
        };
        let (first, second) = (
            at(1, json!([[[{ "insert": "E" }]]])),
            at(3, json!([[[{ "insert": "L" }]]])),
        );
        let end = converge(
            &base,
            &[
                second.clone(),
                Op::cell("t", 0, 0, Delta::new().retain(1).insert("!")),
            ],
            &[first.clone()],
        );
        assert_eq!(end.body.text(), format!("a{}b\n", crate::delta::EMBED_CHAR));
        assert_eq!(table_cell(&end, "t", 0, 0).unwrap().text(), "E");
        let end = converge(&base, &[first], &[second]);
        assert_eq!(table_cell(&end, "t", 0, 0).unwrap().text(), "L");
    }
    #[test]
    fn block_sets_resolve_by_server_order() {
        let base = page(
            Delta::new()
                .embed(
                    json!({ "block": { "id": "b", "source": "chart::A|1" } }),
                    Attrs::new(),
                )
                .insert("\n"),
        );
        let set = |s: &str| Op::BlockSet {
            block_set: Block {
                id: "b".into(),
                source: s.into(),
            },
        };
        assert_eq!(
            block_source(&converge(&base, &[set("later")], &[set("earlier")]), "b"),
            Some("later")
        );
        let carries = Op::delta(
            Delta::new()
                .delete(1)
                .embed(
                    json!({ "block": { "id": "b", "source": "kept" } }),
                    Attrs::new(),
                )
                .retain(1),
        );
        assert_eq!(
            block_source(&converge(&base, &[set("lost")], &[carries.clone()]), "b"),
            Some("kept")
        );
        assert_eq!(
            block_source(&converge(&base, &[carries], &[set("lost")]), "b"),
            Some("kept")
        );
        undoes(&base, &[set("new")]);
        let mut missing = base.clone();
        apply(
            &mut missing,
            &Op::BlockSet {
                block_set: Block {
                    id: "x".into(),
                    source: "y".into(),
                },
            },
        )
        .unwrap();
        assert_eq!(missing, base);
    }
    #[test]
    fn meta_fields_resolve_by_server_order_and_invert() {
        let base = Page::new(Kind::Page, "Start");
        let l = Op::Meta {
            meta: MetaOp {
                title: Some("Later".into()),
                kind: Some(Kind::List),
                archived: None,
            },
        };
        let e = Op::Meta {
            meta: MetaOp {
                title: Some("Earlier".into()),
                kind: None,
                archived: Some(true),
            },
        };
        let end = converge(&base, &[l.clone()], &[e.clone()]);
        assert_eq!(
            end.meta,
            crate::page::Meta {
                title: "Later".into(),
                kind: Kind::List,
                archived: true
            }
        );
        undoes(&base, &[l, e]);
        let long = Op::Meta {
            meta: MetaOp {
                title: Some("x".repeat(MAX_TITLE + 1)),
                ..Default::default()
            },
        };
        assert_eq!(apply(&mut base.clone(), &long), Err("title too long"));
    }
    #[test]
    fn a_row_insert_survives_a_concurrent_delete_at_the_same_index() {
        let base = page(
            Delta::new()
                .embed(
                    json!({ "table": { "id": "t", "widths": [1.0], "rows": [[[]], [[]], [[]]] } }),
                    Attrs::new(),
                )
                .insert("\n"),
        );
        let end = converge(
            &base,
            &[Op::TableRow {
                table_row: Line::new("t", 1, Action::Insert),
            }],
            &[Op::TableRow {
                table_row: Line::new("t", 1, Action::Delete),
            }],
        );
        assert_eq!(
            table_shape(&end, "t"),
            Some((3, 1)),
            "one row went, one arrived"
        );
    }
    #[test]
    fn a_line_op_carries_an_earlier_cell_op_past_it() {
        let base = page(
            Delta::new()
                .embed(
                    json!({ "table": { "id": "t", "widths": [1.0], "rows": [[[]], [[]]] } }),
                    Attrs::new(),
                )
                .insert("\n"),
        );
        let end = converge(
            &base,
            &[Op::TableRow {
                table_row: Line::new("t", 0, Action::Insert),
            }],
            &[Op::cell("t", 1, 0, Delta::new().insert("X"))],
        );
        assert_eq!(table_cell(&end, "t", 2, 0).unwrap().text(), "X");
    }
    #[test]
    fn a_new_column_carries_its_width_whatever_the_order() {
        let base = page(
            Delta::new()
                .embed(
                    json!({ "table": { "id": "t", "widths": [1.0, 2.0], "rows": [[[], []]] } }),
                    Attrs::new(),
                )
                .insert("\n"),
        );
        let insert = Op::TableCol {
            table_col: Line {
                width: Some(json!(3.0)),
                ..Line::new("t", 1, Action::Insert)
            },
        };
        let delete = Op::TableCol {
            table_col: Line::new("t", 0, Action::Delete),
        };
        let end = converge(&base, &[insert], &[delete]);
        assert_eq!(table(&end, "t").unwrap()["widths"], json!([3.0, 2.0]));
    }
    #[test]
    fn inverses_restore_every_op_kind_exactly() {
        let base = page(
            Delta::new()
                .insert_with("Ti", Attrs::from([("bold".into(), json!(true))]))
                .insert("\n")
                .embed(table_value("t"), Attrs::new())
                .insert("\n"),
        );
        for ops in [
            vec![Op::delta(
                Delta::new().retain(1).delete(2).insert("x").retain(2),
            )],
            vec![Op::BodySet {
                body_set: Delta::new().insert("x\n"),
            }],
            vec![Op::cell("t", 0, 1, Delta::new().delete(1).insert("q"))],
            vec![Op::TableRow {
                table_row: Line::new("t", 0, Action::Delete),
            }],
            vec![Op::TableRow {
                table_row: Line::new("t", 2, Action::Insert),
            }],
            vec![Op::TableCol {
                table_col: Line::new("t", 1, Action::Delete),
            }],
            vec![
                Op::TableCol {
                    table_col: Line {
                        width: Some(json!(4.0)),
                        ..Line::new("t", 0, Action::Insert)
                    },
                },
                Op::cell("t", 0, 0, Delta::new().insert("n")),
            ],
        ] {
            undoes(&base, &ops);
        }
        for stray in [
            Op::TableRow {
                table_row: Line::new("t", 2, Action::Delete),
            },
            Op::TableCol {
                table_col: Line::new("t", 3, Action::Insert),
            },
            Op::cell("gone", 0, 0, Delta::new().insert("x")),
        ] {
            assert!(invert(&base, &stray).unwrap().is_empty(), "{stray:?}");
            let mut p = base.clone();
            apply(&mut p, &stray).unwrap();
            assert_eq!(p, base);
        }
    }
    #[test]
    fn invalid_ops_leave_the_page_untouched() {
        let base = with_table("t");
        let bad = [
            vec![Op::delta(Delta::new().retain(9))],
            vec![Op::delta(
                Delta::new().retain(2).embed(table_value("t"), Attrs::new()),
            )],
            vec![Op::BodySet {
                body_set: Delta::new()
                    .embed(table_value("t"), Attrs::new())
                    .embed(table_value("t"), Attrs::new()),
            }],
            vec![Op::BodySet {
                body_set: Delta::new().retain(1),
            }],
            vec![Op::cell(
                "t",
                0,
                0,
                Delta::new().retain(1).embed(
                    json!({ "block": { "id": "b", "source": "" } }),
                    Attrs::new(),
                ),
            )],
            vec![Op::cell("t", 0, 0, Delta::new().retain(5))],
            vec![
                Op::delta(Delta::new().retain(2).insert("ok")),
                Op::delta(Delta::new().retain(1)),
            ],
            vec![Op::delta(
                Delta::new()
                    .embed(json!({ "hr": true, "block": { "id": "t" } }), Attrs::new())
                    .retain(2),
            )],
            vec![Op::BodySet {
                body_set: Delta::new().embed(json!(7), Attrs::new()),
            }],
            vec![Op::delta(
                Delta::new()
                    .embed(
                        json!({ "table": { "id": "u", "widths": [1.0], "rows": [[]] } }),
                        Attrs::new(),
                    )
                    .retain(2),
            )],
            vec![Op::TableCol {
                table_col: Line {
                    width: Some(json!("wide")),
                    ..Line::new("t", 0, Action::Insert)
                },
            }],
            vec![Op::BlockSet {
                block_set: Block {
                    id: "b".into(),
                    source: "x".repeat(MAX_BODY + 1),
                },
            }],
        ];
        for ops in bad {
            let mut p = base.clone();
            assert!(apply_batch(&mut p, &ops).is_err(), "{ops:?}");
            assert_eq!(p, base, "{ops:?}");
        }
    }
    #[test]
    fn a_cell_edit_accepted_on_a_missing_cell_is_voided_against_a_mismatched_one() {
        let base = with_table("t");
        let mut gone = base.clone();
        apply(&mut gone, &Op::delta(Delta::new().delete(1).retain(1))).unwrap();
        let wild = Op::cell("t", 0, 0, Delta::new().retain(999));
        let honest = Op::cell("t", 0, 0, Delta::new().retain(1).insert("b"));
        for (l, e) in [(&wild, &honest), (&honest, &wild)] {
            let (l2, e2) = transform(l, e).unwrap();
            assert!(l2.is_noop());
            assert_eq!(&e2, e);
            converge(&gone, &[l.clone()], &[e.clone()]);
        }
        let same = transform(&honest, &honest).unwrap();
        assert!(!same.0.is_noop() && !same.1.is_noop());
    }
    #[test]
    fn authors_stop_at_the_body_limit_and_replay_has_room_for_concurrent_growth() {
        let base = page(Delta::new().insert("\n"));
        let big = |c: &str| Op::delta(Delta::new().insert(&c.repeat(MAX_BODY / 2 + 1)).retain(1));
        let mut p = base.clone();
        apply_local(&mut p, &big("a")).unwrap();
        let (b2, _) = transform_batch(&[big("b")], &[big("a")]).unwrap();
        assert_eq!(
            apply_local_batch(&mut p.clone(), &b2),
            Err("document too large")
        );
        apply_batch(&mut p, &b2).unwrap();
        assert!(p.body.size() > MAX_BODY && p.body.size() <= MAX_REPLAY);
        let cell = Op::cell(
            "t",
            0,
            0,
            Delta::new().insert(&"x".repeat(MAX_BODY)).retain(1),
        );
        assert_eq!(
            apply_local(&mut with_table("t"), &cell),
            Err("document too large")
        );
        apply(&mut with_table("t"), &cell).unwrap();
        let set = Op::BodySet {
            body_set: Delta::new().insert(&"x".repeat(MAX_BODY + 1)),
        };
        assert_eq!(
            apply_local(&mut base.clone(), &set),
            Err("document too large")
        );
        apply(&mut base.clone(), &set).unwrap();
    }
    #[test]
    fn null_attributes_never_reach_a_body() {
        let base = page(Delta::new().insert("ab\n"));
        let mut p = base.clone();
        apply(
            &mut p,
            &Op::delta(
                Delta::new()
                    .retain(1)
                    .insert_with("x", Attrs::from([("bold".into(), Value::Null)]))
                    .retain(2),
            ),
        )
        .unwrap();
        assert_eq!(p.body, Delta::new().insert("axb\n"));
    }
}
