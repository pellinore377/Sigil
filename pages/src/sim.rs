//! Random pages, ops and multi-site sessions in server order, for convergence tests and fuzzing.
use crate::delta::{Attrs, Component, Delta, Insert};
use crate::ops::{
    apply_batch, apply_local_batch, invert, invert_batch, table_cell, table_shape, transform_batch,
    Action, Block, Line, MetaOp, Op,
};
use crate::page::{Kind, Page};
use serde_json::{json, Value};

/// Choices for the generators.
pub trait Source {
    /// A number below `n`; `n` is at least 1.
    fn below(&mut self, n: u32) -> u32;
    fn chance(&mut self, one_in: u32) -> bool {
        self.below(one_in) == 0
    }
    /// Whether choices have run out, which ends a session's random steps.
    fn spent(&self) -> bool {
        false
    }
}
pub struct Xorshift(pub u64);
impl Source for Xorshift {
    fn below(&mut self, n: u32) -> u32 {
        let s = &mut self.0;
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        (*s % n as u64) as u32
    }
}
/// Fuzzer bytes as choices; zero once spent.
pub struct Bytes<'a>(pub &'a [u8]);
impl Source for Bytes<'_> {
    fn below(&mut self, n: u32) -> u32 {
        let (v, rest) = match self.0 {
            [a, b, rest @ ..] => (u16::from_le_bytes([*a, *b]) as u32, rest),
            [a] => (*a as u32, &[][..]),
            [] => (0, &[][..]),
        };
        self.0 = rest;
        v % n
    }
    fn spent(&self) -> bool {
        self.0.is_empty()
    }
}

fn attrs(v: Value) -> Attrs {
    serde_json::from_value(v).unwrap()
}
fn run_attrs(src: &mut impl Source) -> Attrs {
    attrs(match src.below(7) {
        0 => json!({ "bold": true }),
        1 => json!({ "bold": null }),
        2 => json!({ "italic": true, "color": "red3" }),
        3 => json!({ "link": "https://example.test" }),
        4 => json!({ "code": true, "italic": null }),
        _ => json!({}),
    })
}
fn para_attrs(src: &mut impl Source) -> Attrs {
    attrs(match src.below(5) {
        0 => json!({ "heading": 1 }),
        1 => json!({ "list": "check", "checked": true }),
        2 => json!({ "list": "bullet", "indent": 1 }),
        _ => json!({}),
    })
}
fn text(src: &mut impl Source, newlines: bool) -> &'static str {
    let words = ["a", "bc", "é", "😀", " ", "xyz"];
    if newlines && src.chance(4) {
        "\n"
    } else {
        words[src.below(words.len() as u32) as usize]
    }
}
/// One insert, delete or format at a random place in a body of `len` scalars.
fn text_edit(src: &mut impl Source, len: u32, newlines: bool) -> Delta {
    let at = src.below(len + 1);
    let rest = len - at;
    match src.below(if rest > 0 { 3 } else { 1 }) {
        0 => {
            let t = text(src, newlines);
            let a = if t == "\n" {
                para_attrs(src)
            } else {
                run_attrs(src)
            };
            Delta::new().retain(at).insert_with(t, a).retain(rest)
        }
        1 => {
            let n = 1 + src.below(rest.min(6));
            Delta::new().retain(at).delete(n).retain(rest - n)
        }
        _ => {
            let n = 1 + src.below(rest.min(8));
            Delta::new()
                .retain(at)
                .retain_with(n, run_attrs(src))
                .retain(rest - n)
        }
    }
}
fn cell(src: &mut impl Source) -> Value {
    let body = if src.chance(2) {
        Delta::new()
    } else {
        Delta::new().insert_with(text(src, false), run_attrs(src))
    };
    serde_json::to_value(body).unwrap()
}
fn table(src: &mut impl Source, id: &str) -> Value {
    let (rows, cols) = (1 + src.below(3), 1 + src.below(3));
    let widths: Vec<Value> = (0..cols)
        .map(|_| json!([1.0, 2.0, 3.5][src.below(3) as usize]))
        .collect();
    let rows: Vec<Value> = (0..rows)
        .map(|_| Value::Array((0..cols).map(|_| cell(src)).collect()))
        .collect();
    json!({ "table": { "id": id, "rows": rows, "widths": widths } })
}
fn embed(src: &mut impl Source, fresh: &mut dyn FnMut() -> String) -> Value {
    match src.below(3) {
        0 => table(src, &fresh()),
        1 => json!({ "block": { "id": fresh(), "source": "chart::A|1" } }),
        _ => json!({ "hr": true }),
    }
}
/// Ids of the keyed embeds of one kind in a body.
fn ids(page: &Page, kind: &str) -> Vec<String> {
    page.body
        .scalars()
        .into_iter()
        .filter_map(|(_, _, e)| Some(e?.get(kind)?.get("id")?.as_str()?.to_owned()))
        .collect()
}
fn pick<T: Clone>(src: &mut impl Source, items: &[T]) -> Option<T> {
    (!items.is_empty()).then(|| items[src.below(items.len() as u32) as usize].clone())
}
/// A random page with text, paragraphs and keyed embeds.
pub fn random_page(src: &mut impl Source, fresh: &mut dyn FnMut() -> String) -> Page {
    let mut page = Page::new(Kind::Page, "Plan");
    for _ in 0..src.below(8) {
        let len = page.body.target_len();
        let edit = if src.chance(3) {
            Delta::new()
                .retain(src.below(len + 1))
                .embed(embed(src, fresh), Attrs::new())
        } else {
            text_edit(src, len, true)
        };
        let rest = len - edit.base_len();
        let edit = edit.retain(rest);
        page.body = edit.apply(&page.body).unwrap();
    }
    page
}
/// A short page around one table, so table ops meet often.
pub fn table_page(src: &mut impl Source, fresh: &mut dyn FnMut() -> String) -> Page {
    let mut page = Page::new(Kind::List, "Table");
    page.body = Delta::new()
        .insert("x\n")
        .embed(table(src, &fresh()), Attrs::new())
        .insert("\n");
    page
}
/// Either kind of starting page.
pub fn any_page(src: &mut impl Source, fresh: &mut dyn FnMut() -> String) -> Page {
    if src.chance(2) {
        table_page(src, fresh)
    } else {
        random_page(src, fresh)
    }
}
/// One op valid against `page`.
pub fn random_op(src: &mut impl Source, page: &Page, fresh: &mut dyn FnMut() -> String) -> Op {
    let len = page.body.target_len();
    let tables = ids(page, "table");
    match src.below(28) {
        24 => table_move(src, page).unwrap_or_else(|| Op::delta(text_edit(src, len, true))),
        25 => {
            // Accepted as a no-op whatever its length: a missing table or a row past the end.
            let wild = Delta::new().retain(src.below(999)).insert("q");
            match pick(src, &tables) {
                Some(id) if src.chance(2) => {
                    let (rows, _) = table_shape(page, &id).unwrap();
                    Op::cell(&id, rows as u32 + src.below(2), 0, wild)
                }
                _ => Op::cell("missing", src.below(3), src.below(3), wild),
            }
        }
        26 => {
            let line = Line::new("missing", src.below(3), Action::Delete);
            if src.chance(2) {
                Op::TableRow { table_row: line }
            } else {
                Op::TableCol { table_col: line }
            }
        }
        0..=2 => {
            let at = src.below(len + 1);
            Op::delta(
                Delta::new()
                    .retain(at)
                    .embed(embed(src, fresh), Attrs::new())
                    .retain(len - at),
            )
        }
        3..=6 => {
            let Some(id) = pick(src, &tables) else {
                return Op::delta(text_edit(src, len, true));
            };
            let (rows, cols) = table_shape(page, &id).unwrap();
            if rows == 0 || cols == 0 {
                return Op::delta(text_edit(src, len, true));
            }
            let (row, col) = (src.below(rows as u32), src.below(cols as u32));
            let body = table_cell(page, &id, row, col).unwrap();
            Op::cell(&id, row, col, text_edit(src, body.target_len(), false))
        }
        7..=9 => {
            let Some(id) = pick(src, &tables) else {
                return Op::delta(text_edit(src, len, true));
            };
            let (rows, cols) = table_shape(page, &id).unwrap();
            let rows_axis = src.chance(2);
            let n = if rows_axis { rows } else { cols } as u32;
            let action = if n > 0 && src.chance(2) {
                Action::Delete
            } else {
                Action::Insert
            };
            let at = src.below(if action == Action::Insert { n + 1 } else { n });
            let mut line = Line::new(&id, at, action);
            if rows_axis {
                Op::TableRow { table_row: line }
            } else {
                if action == Action::Insert {
                    line.width = Some(json!([1.0, 2.0, 3.5][src.below(3) as usize]));
                }
                Op::TableCol { table_col: line }
            }
        }
        10..=11 => {
            let blocks = ids(page, "block");
            let id = pick(src, &blocks)
                .filter(|_| !src.chance(6))
                .unwrap_or_else(|| "missing".into());
            Op::BlockSet {
                block_set: Block {
                    id,
                    source: ["chart::B|2", "math::block x", ""][src.below(3) as usize].into(),
                },
            }
        }
        12 => {
            let copy = text_edit(src, len, true).apply(&page.body).unwrap();
            if src.chance(2) {
                Op::BodySet { body_set: copy }
            } else {
                Op::delta(Delta::replace_all(len, &copy))
            }
        }
        13 => Op::BodySet {
            body_set: random_page(src, fresh).body,
        },
        14..=15 => {
            let meta = MetaOp {
                title: src
                    .chance(2)
                    .then(|| ["Trip", "Groceries", "Notes"][src.below(3) as usize].into()),
                kind: src.chance(2).then(|| {
                    if src.chance(2) {
                        Kind::List
                    } else {
                        Kind::Page
                    }
                }),
                archived: src.chance(3).then(|| src.chance(2)),
            };
            Op::Meta { meta }
        }
        _ => Op::delta(text_edit(src, len, true)),
    }
}
/// One delta deleting a table and inserting it again elsewhere.
fn table_move(src: &mut impl Source, page: &Page) -> Option<Op> {
    let len = page.body.target_len();
    let found: Vec<(u32, Value, Attrs)> = page
        .body
        .scalars()
        .into_iter()
        .enumerate()
        .filter_map(|(i, (_, a, e))| Some((i as u32, e?.clone(), a.clone())))
        .filter(|(_, e, _)| e.get("table").is_some())
        .collect();
    let (p, v, a) = pick(src, &found)?;
    let q = src.below(len + 1);
    Some(Op::delta(if q <= p {
        Delta::new()
            .retain(q)
            .embed(v, a)
            .retain(p - q)
            .delete(1)
            .retain(len - p - 1)
    } else {
        Delta::new()
            .retain(p)
            .delete(1)
            .retain(q - p - 1)
            .embed(v, a)
            .retain(len - q)
    }))
}
/// A delta as another client might send it: parts split and zero-length parts strewn in.
pub fn wire_shaped(src: &mut impl Source, delta: &Delta) -> Value {
    let zero = |src: &mut dyn FnMut(u32) -> u32| match src(3) {
        0 => json!({ "retain": 0 }),
        1 => json!({ "delete": 0 }),
        _ => json!({ "insert": "" }),
    };
    let mut out = Vec::new();
    for part in &delta.ops {
        if src.chance(3) {
            out.push(zero(&mut |n| src.below(n)));
        }
        match part {
            Component::Retain { retain, attrs } if *retain > 1 && src.chance(2) => {
                let k = 1 + src.below(retain - 1);
                for n in [k, retain - k] {
                    out.push(
                        serde_json::to_value(Component::Retain {
                            retain: n,
                            attrs: attrs.clone(),
                        })
                        .unwrap(),
                    );
                }
            }
            Component::Delete { delete } if *delete > 1 && src.chance(2) => {
                let k = 1 + src.below(delete - 1);
                out.extend([json!({ "delete": k }), json!({ "delete": delete - k })]);
            }
            Component::Insert {
                insert: Insert::Text(t),
                attrs,
            } if t.chars().count() > 1 && src.chance(2) => {
                let k = 1 + src.below(t.chars().count() as u32 - 1) as usize;
                let at = t.char_indices().nth(k).unwrap().0;
                for piece in [&t[..at], &t[at..]] {
                    out.push(
                        serde_json::to_value(Component::Insert {
                            insert: Insert::Text(piece.into()),
                            attrs: attrs.clone(),
                        })
                        .unwrap(),
                    );
                }
            }
            p => out.push(serde_json::to_value(p).unwrap()),
        }
    }
    if src.chance(3) {
        out.push(zero(&mut |n| src.below(n)));
    }
    Value::Array(out)
}
/// The op read back from a wire-shaped copy of itself.
fn reshaped(src: &mut impl Source, op: &Op) -> Op {
    let mut v = serde_json::to_value(op).unwrap();
    for key in ["delta", "body_set"] {
        if let Some(d) = v.get(key).cloned() {
            let d: Delta = serde_json::from_value(d).unwrap();
            v[key] = wire_shaped(src, &d);
        }
    }
    serde_json::from_value(v).unwrap()
}
/// One to three ops, each valid after the ones before it; sometimes followed by its undo.
pub fn random_batch(
    src: &mut impl Source,
    page: &Page,
    fresh: &mut dyn FnMut() -> String,
) -> (Vec<Op>, Page) {
    let mut at = page.clone();
    let mut ops = Vec::new();
    for _ in 0..1 + src.below(3) {
        let op = random_op(src, &at, fresh);
        let wire = reshaped(src, &op);
        if wire != op {
            panic!("wire copy reads differently: {op:?} vs {wire:?}");
        }
        let back = invert(&at, &op).unwrap();
        if let Err(e) = apply_local_batch(&mut at, std::slice::from_ref(&op)) {
            panic!("generated op does not apply: {e}: {op:?}");
        }
        ops.push(op);
        if src.chance(6) {
            if let Err(e) = apply_local_batch(&mut at, &back) {
                panic!("undo does not apply: {e}: {back:?}");
            }
            ops.extend(back);
        }
    }
    (ops, at)
}

fn applied(base: &Page, a: &[Op], b: &[Op]) -> Result<Page, String> {
    let mut page = base.clone();
    apply_batch(&mut page, a).map_err(|e| format!("{e}: {a:?}"))?;
    apply_batch(&mut page, b).map_err(|e| format!("{e}: {b:?}"))?;
    Ok(page)
}
/// TP1 both ways for two batches on `base`: either order of application reaches one page.
pub fn check_tp1(base: &Page, a: &[Op], b: &[Op]) -> Result<Page, String> {
    let mut out = None;
    for (later, earlier) in [(a, b), (b, a)] {
        let (l2, e2) = transform_batch(later, earlier).map_err(|e| format!("transform: {e}"))?;
        let left = applied(base, earlier, &l2)?;
        let right = applied(base, later, &e2)?;
        if left != right {
            return Err(format!("TP1 fails\nbase {base:?}\nlater {later:?}\nearlier {earlier:?}\nleft {left:?}\nright {right:?}"));
        }
        out.get_or_insert(left);
    }
    Ok(out.unwrap())
}

struct Entry {
    base: usize,
    author: usize,
    ops: Vec<Op>,
}
struct Site {
    canon: Page,
    window: Vec<Vec<Op>>,
    inflight: Vec<Op>,
    pending: Vec<Op>,
    spec: Page,
}
impl Site {
    fn local(&self) -> Vec<Op> {
        self.inflight.iter().chain(&self.pending).cloned().collect()
    }
    fn push(&mut self, me: usize, log: &mut Vec<Entry>) {
        if self.inflight.is_empty() && !self.pending.is_empty() {
            self.inflight = std::mem::take(&mut self.pending);
            log.push(Entry {
                base: self.window.len(),
                author: me,
                ops: self.inflight.clone(),
            });
        }
    }
    fn pull(&mut self, me: usize, log: &[Entry]) -> Result<(), String> {
        match log.get(self.window.len()) {
            Some(entry) => self.receive(me, self.window.len(), entry),
            None => Ok(()),
        }
    }
    fn receive(&mut self, me: usize, seq: usize, entry: &Entry) -> Result<(), String> {
        let mut eff = entry.ops.clone();
        for w in &self.window[entry.base..seq] {
            eff = transform_batch(&eff, w)
                .map_err(|e| format!("replay: {e}"))?
                .0;
        }
        let before = self.canon.clone();
        apply_batch(&mut self.canon, &eff).map_err(|e| format!("canonical apply: {e}: {eff:?}"))?;
        self.window.push(eff.clone());
        if entry.author == me {
            if eff != self.inflight {
                return Err(format!(
                    "own batch replays differently\nlocal {:?}\nreplayed {eff:?}",
                    self.inflight
                ));
            }
            self.inflight.clear();
        } else {
            let local = self.local();
            check_tp1(&before, &local, &eff)?;
            let (local2, eff2) =
                transform_batch(&local, &eff).map_err(|e| format!("rebase: {e}"))?;
            apply_batch(&mut self.spec, &eff2).map_err(|e| format!("speculative apply: {e}"))?;
            let k = self.inflight.len();
            (self.inflight, self.pending) = (local2[..k].to_vec(), local2[k..].to_vec());
        }
        let mut rebuilt = self.canon.clone();
        apply_batch(&mut rebuilt, &self.local()).map_err(|e| format!("rebuild: {e}"))?;
        if rebuilt != self.spec {
            return Err(format!(
                "speculative page drifted\nspec {:?}\nrebuilt {rebuilt:?}",
                self.spec
            ));
        }
        Ok(())
    }
}

/// Several sites editing one page through a server-ordered log; returns the converged page.
pub fn run(src: &mut impl Source, sites: usize, steps: usize) -> Result<Page, String> {
    let mut serial = 0u32;
    let initial = any_page(src, &mut || {
        serial += 1;
        format!("i{serial}")
    });
    let mut log: Vec<Entry> = Vec::new();
    let mut all: Vec<Site> = (0..sites)
        .map(|_| Site {
            canon: initial.clone(),
            window: Vec::new(),
            inflight: Vec::new(),
            pending: Vec::new(),
            spec: initial.clone(),
        })
        .collect();
    let mut counters = vec![0u32; sites];
    for _ in 0..steps {
        if src.spent() {
            break;
        }
        let who = src.below(sites as u32) as usize;
        match src.below(5) {
            0 | 1 => {
                let n = &mut counters[who];
                let (ops, next) = random_batch(src, &all[who].spec, &mut || {
                    *n += 1;
                    format!("s{who}n{n}")
                });
                all[who].pending.extend(ops);
                all[who].spec = next;
            }
            2 => all[who].push(who, &mut log),
            _ => all[who].pull(who, &log)?,
        }
    }
    while !all
        .iter()
        .all(|s| s.inflight.is_empty() && s.pending.is_empty() && s.window.len() == log.len())
    {
        for (who, site) in all.iter_mut().enumerate() {
            while site.window.len() < log.len() {
                site.pull(who, &log)?;
            }
            site.push(who, &mut log);
        }
    }
    let end = all[0].canon.clone();
    for (i, s) in all.iter().enumerate() {
        if s.canon != end || s.spec != end {
            return Err(format!("site {i} diverged\n{:?}\nvs {end:?}", s.canon));
        }
    }
    let mut at = initial;
    for batch in &all[0].window {
        let back = invert_batch(&at, batch).map_err(|e| format!("invert: {e}"))?;
        let before = at.clone();
        apply_batch(&mut at, batch).map_err(|e| format!("replay: {e}"))?;
        let mut undone = at.clone();
        apply_batch(&mut undone, &back).map_err(|e| format!("undo: {e}: {back:?}"))?;
        if undone != before {
            return Err(format!(
                "inverse does not restore\nbatch {batch:?}\nbefore {before:?}\nundone {undone:?}"
            ));
        }
    }
    Ok(end)
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Seeds `PAGES_SIM_FROM..+PAGES_SIM_ITERATIONS`, defaults 0 and 300.
    fn seeds() -> std::ops::Range<u64> {
        let var = |name: &str, default: u64| {
            std::env::var(name)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        };
        let from = var("PAGES_SIM_FROM", 0);
        from..from + var("PAGES_SIM_ITERATIONS", 300)
    }
    fn source(seed: u64) -> Xorshift {
        Xorshift(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1)
    }
    #[test]
    fn concurrent_sites_converge_in_server_order() {
        for seed in seeds() {
            let mut src = source(seed);
            let sites = 2 + src.below(3) as usize;
            if let Err(e) = run(&mut src, sites, 120) {
                panic!("seed {seed}, {sites} sites: {e}");
            }
        }
    }
    #[test]
    fn concurrent_batch_pairs_satisfy_tp1_both_ways() {
        for seed in seeds() {
            let mut src = source(seed ^ 0x5eed);
            let (mut a, mut b, mut c) = (0, 0, 0);
            let page = any_page(&mut src, &mut || {
                c += 1;
                format!("i{c}")
            });
            for _ in 0..20 {
                let (x, _) = random_batch(&mut src, &page, &mut || {
                    a += 1;
                    format!("a{a}")
                });
                let (y, _) = random_batch(&mut src, &page, &mut || {
                    b += 1;
                    format!("b{b}")
                });
                if let Err(e) = check_tp1(&page, &x, &y) {
                    panic!("seed {seed}: {e}");
                }
            }
        }
    }
    /// A wild component: zero, huge, split or an embed.
    fn wild(src: &mut impl Source) -> Value {
        match src.below(10) {
            0 => json!({ "retain": 0 }),
            1 => json!({ "delete": 0 }),
            2 => json!({ "retain": u32::MAX }),
            3 => json!({ "delete": u32::MAX - src.below(3) }),
            4 => json!({ "insert": { "hr": true } }),
            5 => json!({ "insert": "" }),
            6 => json!({ "retain": 1 + src.below(4), "attrs": { "bold": null } }),
            7 => json!({ "insert": "w", "attrs": { "bold": null } }),
            8 => json!({ "delete": 1 + src.below(3) }),
            _ => json!({ "retain": 1 + src.below(6) }),
        }
    }
    #[test]
    fn wire_ops_never_panic_and_accepted_ones_always_transform() {
        let mut accepted = 0;
        for seed in seeds() {
            let mut src = source(seed ^ 0x3a7e);
            let mut c = 0;
            let page = any_page(&mut src, &mut || {
                c += 1;
                format!("i{c}")
            });
            for _ in 0..20 {
                let parts: Vec<Value> = (0..src.below(7)).map(|_| wild(&mut src)).collect();
                let tables = ids(&page, "table");
                let v = match (src.below(3), pick(&mut src, &tables)) {
                    (0, Some(t)) => {
                        json!({ "cell": { "table": t, "row": src.below(3), "col": src.below(3) }, "delta": parts })
                    }
                    (1, _) => {
                        json!({ "cell": { "table": "missing", "row": 0, "col": 0 }, "delta": parts })
                    }
                    _ => json!({ "delta": parts }),
                };
                let Ok(op) = serde_json::from_value::<Op>(v) else {
                    continue;
                };
                let mut p = page.clone();
                if apply_batch(&mut p, std::slice::from_ref(&op)).is_err() {
                    continue;
                }
                accepted += 1;
                let (honest, _) = random_batch(&mut src, &page, &mut || {
                    c += 1;
                    format!("h{c}")
                });
                if let Err(e) = check_tp1(&page, &[op], &honest) {
                    panic!("seed {seed}: {e}");
                }
            }
        }
        assert!(accepted > 0);
    }
    #[test]
    fn fuzzer_bytes_drive_a_session() {
        let bytes: Vec<u8> = (0..4096u32)
            .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
            .collect();
        run(&mut Bytes(&bytes), 3, 200).unwrap();
        run(&mut Bytes(&[]), 2, 10).unwrap();
    }
}
