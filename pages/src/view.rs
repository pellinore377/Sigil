//! The UI's view of a page: paragraphs partition the body, runs are maximal attribute-equal spans, offsets are UTF-16.
use crate::delta::{Attrs, Component, Delta, Insert, EMBED_CHAR};
use crate::page::{Meta, Page};
use serde::Serialize;
use serde_json::{json, Map, Value};

#[derive(Serialize)]
pub struct PageView<'a> {
    pub head: u64,
    pub meta: &'a Meta,
    pub body: BodyView,
}
#[derive(Serialize)]
pub struct BodyView {
    pub text: String,
    pub cursor: usize,
    pub paragraphs: Vec<Paragraph>,
}
#[derive(Serialize)]
pub struct Paragraph {
    pub start: usize,
    pub end: usize,
    pub attrs: Attrs,
    pub runs: Vec<Run>,
    pub embeds: Vec<Value>,
}
#[derive(Serialize)]
pub struct Run {
    pub start: usize,
    pub end: usize,
    pub attrs: Attrs,
}

/// Runs of a scalar span, `start` its UTF-16 offset.
fn runs_of(scalars: &[(char, &Attrs, Option<&Value>)], start: usize) -> Vec<Run> {
    let (mut out, mut at, mut run_start) = (Vec::new(), start, start);
    let mut attrs: Option<&Attrs> = None;
    for (ch, a, _) in scalars {
        if attrs != Some(*a) {
            if let Some(previous) = attrs.filter(|_| at > run_start) {
                out.push(Run {
                    start: run_start,
                    end: at,
                    attrs: previous.clone(),
                });
            }
            (run_start, attrs) = (at, Some(*a));
        }
        at += ch.len_utf16();
    }
    if let Some(previous) = attrs.filter(|_| at > run_start) {
        out.push(Run {
            start: run_start,
            end: at,
            attrs: previous.clone(),
        });
    }
    out
}
/// A table embed with every cell as its text and runs.
fn table_view(table: &Map<String, Value>) -> Value {
    let cell = |cell: &Value| {
        let delta: Delta = serde_json::from_value(cell.clone()).unwrap_or_default();
        json!({ "text": delta.text(), "runs": runs_of(&delta.scalars(), 0) })
    };
    let row = |row: &Value| Value::Array(row.as_array().into_iter().flatten().map(&cell).collect());
    let rows: Vec<Value> = table
        .get("rows")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(row)
        .collect();
    let mut out = table.clone();
    out.insert(
        "id".to_owned(),
        table.get("id").cloned().unwrap_or_default(),
    );
    out.insert(
        "widths".to_owned(),
        table.get("widths").cloned().unwrap_or_else(|| json!([])),
    );
    out.insert("rows".to_owned(), Value::Array(rows));
    Value::Object(out)
}

/// The whole page; `cursor` is a scalar offset.
pub fn view(page: &Page, head: u64, cursor: u32) -> PageView<'_> {
    PageView {
        head,
        meta: &page.meta,
        body: body_view(&page.body, cursor),
    }
}
/// Paragraphs and runs of a body; `cursor` is a scalar offset.
pub fn body_view(body: &Delta, cursor: u32) -> BodyView {
    let scalars = body.scalars();
    let text = body.text();
    let (mut paragraphs, mut runs, mut embeds) = (Vec::new(), Vec::new(), Vec::new());
    let (mut at, mut para_start, mut run_start) = (0usize, 0usize, 0usize);
    let mut run_attrs: Option<&Attrs> = None;
    let mut cursor_units = 0;
    let close_run = |runs: &mut Vec<Run>, start: usize, end: usize, attrs: Option<&Attrs>| {
        if let Some(attrs) = attrs.filter(|_| end > start) {
            runs.push(Run {
                start,
                end,
                attrs: attrs.clone(),
            });
        }
    };
    for (i, (ch, attrs, embed)) in scalars.iter().enumerate() {
        if i == cursor as usize {
            cursor_units = at;
        }
        let units = ch.len_utf16();
        if *ch == '\n' {
            close_run(&mut runs, run_start, at, run_attrs);
            paragraphs.push(Paragraph {
                start: para_start,
                end: at + units,
                attrs: (*attrs).clone(),
                runs: std::mem::take(&mut runs),
                embeds: std::mem::take(&mut embeds),
            });
            para_start = at + units;
            run_start = para_start;
            run_attrs = None;
        } else {
            if run_attrs != Some(*attrs) {
                close_run(&mut runs, run_start, at, run_attrs);
                run_start = at;
                run_attrs = Some(*attrs);
            }
            if let Some(Value::Object(fields)) = embed {
                let mut e = Map::from_iter([("at".to_owned(), Value::from(at))]);
                match fields.get("table").and_then(Value::as_object) {
                    Some(table) => {
                        e.insert("table".to_owned(), table_view(table));
                    }
                    None => e.extend(fields.clone()),
                }
                embeds.push(Value::Object(e));
            }
        }
        at += units;
    }
    if cursor as usize >= scalars.len() {
        cursor_units = at;
    }
    if para_start < at {
        close_run(&mut runs, run_start, at, run_attrs);
        paragraphs.push(Paragraph {
            start: para_start,
            end: at,
            attrs: Attrs::new(),
            runs,
            embeds,
        });
    }
    BodyView {
        text,
        cursor: cursor_units,
        paragraphs,
    }
}
/// Each paragraph's fingerprint and scalar length, newline included.
pub fn prints(body: &Delta) -> Vec<(u64, u32)> {
    let mut out = Vec::new();
    let (mut hash, mut len) = (FNV, 0u32);
    for part in &body.ops {
        let Component::Insert { insert, attrs } = part else {
            continue;
        };
        match insert {
            Insert::Text(t) => {
                for (k, piece) in t.split('\n').enumerate() {
                    if k > 0 {
                        hash = mix_attrs(mix(hash, b"\n"), attrs);
                        out.push((hash, len + 1));
                        (hash, len) = (FNV, 0);
                    }
                    if !piece.is_empty() {
                        hash = mix_attrs(mix(hash, piece.as_bytes()), attrs);
                        len += piece.chars().count() as u32;
                    }
                }
            }
            Insert::Embed(e) => {
                hash = mix_attrs(mix_value(mix(hash, b"\x00"), e), attrs);
                len += 1;
            }
        }
    }
    if len > 0 {
        out.push((hash, len));
    }
    out
}
const FNV: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a over eight bytes at a time.
fn mix(mut h: u64, bytes: &[u8]) -> u64 {
    let (chunks, rest) = bytes.as_chunks::<8>();
    for c in chunks {
        h = (h ^ u64::from_le_bytes(*c)).wrapping_mul(0x100_0000_01b3);
    }
    for b in rest {
        h = (h ^ *b as u64).wrapping_mul(0x100_0000_01b3);
    }
    (h ^ bytes.len() as u64).wrapping_mul(0x100_0000_01b3)
}
fn mix_attrs(h: u64, attrs: &Attrs) -> u64 {
    attrs
        .iter()
        .fold(mix(h, b"{"), |h, (k, v)| mix_value(mix(h, k.as_bytes()), v))
}
fn mix_value(h: u64, v: &Value) -> u64 {
    match v {
        Value::Null => mix(h, b"n"),
        Value::Bool(b) => mix(h, if *b { b"t" } else { b"f" }),
        Value::Number(n) => mix(h, n.to_string().as_bytes()),
        Value::String(s) => mix(mix(h, b"s"), s.as_bytes()),
        Value::Array(a) => a.iter().fold(mix(h, b"["), mix_value),
        Value::Object(o) => o
            .iter()
            .fold(mix(h, b"{"), |h, (k, v)| mix_value(mix(h, k.as_bytes()), v)),
    }
}
/// The paragraphs changed since fingerprints `old`: count, kept head and foot, and the middle as its own view.
pub fn body_patch(body: &Delta, cursor: u32, old: &[u64]) -> (Value, Vec<u64>) {
    let now = prints(body);
    let fresh: Vec<u64> = now.iter().map(|p| p.0).collect();
    let head = fresh.iter().zip(old).take_while(|(a, b)| a == b).count();
    let room = fresh.len().min(old.len()) - head;
    let tail = fresh
        .iter()
        .rev()
        .zip(old.iter().rev())
        .take(room)
        .take_while(|(a, b)| a == b)
        .count();
    let start: u32 = now[..head].iter().map(|p| p.1).sum();
    let end: u32 = start + now[head..now.len() - tail].iter().map(|p| p.1).sum::<u32>();
    let middle = body_view(&body.slice(start, end), 0);
    let value = json!({ "cursor": body.utf16_from_scalar(cursor), "count": fresh.len(), "keep": [head, tail], "text": middle.text, "paragraphs": middle.paragraphs });
    (value, fresh)
}
/// The paragraph `[start, end)` in scalars (end after the newline) containing scalar `at`.
pub fn paragraph_at(text: &str, at: u32) -> (u32, u32) {
    let chars: Vec<char> = text.chars().collect();
    let at = (at as usize).min(chars.len());
    let start = chars[..at]
        .iter()
        .rposition(|c| *c == '\n')
        .map_or(0, |i| i + 1);
    let end = chars[at..]
        .iter()
        .position(|c| *c == '\n')
        .map_or(chars.len(), |i| at + i + 1);
    (start as u32, end as u32)
}
pub fn is_embed(ch: char) -> bool {
    ch == EMBED_CHAR
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::{apply, Op};
    use crate::page::Kind;
    fn attrs(v: Value) -> Attrs {
        serde_json::from_value(v).unwrap()
    }
    /// Paragraphs as a reader rebuilds them, offsets from the paragraph's own start.
    fn relative(v: &Value) -> Vec<Value> {
        let text: Vec<u16> = v["text"].as_str().unwrap().encode_utf16().collect();
        v["paragraphs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                let (s, e) = (p["start"].as_u64().unwrap() as usize, p["end"].as_u64().unwrap() as usize);
                let shift = |x: &Value| x.as_u64().unwrap() as usize - s;
                json!({ "text": String::from_utf16(&text[s..e]).unwrap(), "attrs": p["attrs"],
                    "runs": p["runs"].as_array().unwrap().iter().map(|r| json!([shift(&r["start"]), shift(&r["end"]), r["attrs"]])).collect::<Vec<_>>(),
                    "embeds": p["embeds"].as_array().unwrap().iter().map(|m| { let mut m = m.clone(); m["at"] = json!(shift(&m["at"])); m }).collect::<Vec<_>>() })
            })
            .collect()
    }
    #[test]
    fn a_patch_rebuilds_the_whole_view() {
        let b = |s: &str| attrs(json!({ "bold": s == "b" }));
        let base = Delta::new()
            .insert("One 😀\nTwo ")
            .insert_with("bold", b("b"))
            .insert("\n")
            .embed(json!({"hr": true}), Attrs::new())
            .insert("\nFour\nFive\n");
        let edits = [
            Delta::new().retain(12).insert("er").retain(15),
            Delta::new().retain(6).insert("\nnew").retain(21),
            Delta::new().retain(6).delete(10).retain(11),
            Delta::new().retain(21).retain_with(4, b("b")).retain(2),
            Delta::new().delete(27).insert("all new\n"),
            Delta::new().retain(27),
        ];
        for edit in edits {
            let old = serde_json::to_value(body_view(&base, 0)).unwrap();
            let next = edit.apply(&base).unwrap();
            let before: Vec<u64> = prints(&base).into_iter().map(|p| p.0).collect();
            let (patch, now) = body_patch(&next, 3, &before);
            let (head, tail) = (
                patch["keep"][0].as_u64().unwrap() as usize,
                patch["keep"][1].as_u64().unwrap() as usize,
            );
            let old = relative(&old);
            let mut rebuilt = old[..head].to_vec();
            rebuilt.extend(relative(&patch));
            rebuilt.extend(old[old.len() - tail..].to_vec());
            let whole = serde_json::to_value(body_view(&next, 3)).unwrap();
            assert_eq!(rebuilt, relative(&whole), "{edit:?}");
            assert_eq!(patch["count"], json!(rebuilt.len()));
            assert_eq!(patch["cursor"], whole["cursor"]);
            assert_eq!(now.len(), rebuilt.len());
        }
    }
    #[test]
    fn paragraphs_partition_multibyte_text_with_embeds() {
        let mut page = Page::new(Kind::List, "V");
        page.body = Delta::new()
            .insert_with("Hi", attrs(json!({"bold": true})))
            .insert(" 😀")
            .insert_with("\n", attrs(json!({"heading": 1})))
            .insert("a")
            .embed(
                json!({"block": {"id": "b1", "source": "chart::A|1"}}),
                attrs(json!({"link": "u"})),
            )
            .insert("b\n\n");
        let v = serde_json::to_value(view(&page, 7, 4)).unwrap();
        assert_eq!(v["head"], 7);
        assert_eq!(v["meta"], json!({ "title": "V", "kind": "list" }));
        let body = &v["body"];
        assert_eq!(body["text"], format!("Hi 😀\na{EMBED_CHAR}b\n\n"));
        assert_eq!(body["cursor"], 5);
        let paragraphs = body["paragraphs"].as_array().unwrap();
        assert_eq!(paragraphs.len(), 3);
        assert_eq!(
            (&paragraphs[0]["start"], &paragraphs[0]["end"]),
            (&json!(0), &json!(6))
        );
        assert_eq!(paragraphs[0]["attrs"], json!({"heading": 1}));
        assert_eq!(
            paragraphs[0]["runs"],
            json!([{ "start": 0, "end": 2, "attrs": {"bold": true} }, { "start": 2, "end": 5, "attrs": {} }])
        );
        assert_eq!(
            (&paragraphs[1]["start"], &paragraphs[1]["end"]),
            (&json!(6), &json!(10))
        );
        assert_eq!(
            paragraphs[1]["runs"],
            json!([{ "start": 6, "end": 7, "attrs": {} }, { "start": 7, "end": 8, "attrs": {"link": "u"} }, { "start": 8, "end": 9, "attrs": {} }])
        );
        assert_eq!(
            paragraphs[1]["embeds"],
            json!([{ "at": 7, "block": {"id": "b1", "source": "chart::A|1"} }])
        );
        assert_eq!(
            paragraphs[2],
            json!({ "start": 10, "end": 11, "attrs": {}, "runs": [], "embeds": [] })
        );
        assert_eq!(paragraph_at("ab\ncd\n", 4), (3, 6));
        assert_eq!(paragraph_at("ab\ncd\n", 6), (6, 6));
        assert!(is_embed(EMBED_CHAR) && !is_embed('a'));
    }
    /// A short op log replays to the expected view.
    #[test]
    fn an_op_log_replays_to_its_view() {
        let ops: Vec<Op> = serde_json::from_value(json!([
            { "delta": [ { "insert": "Cost" }, { "retain": 1 } ] },
            { "meta": { "title": "Report", "kind": "page" } },
            { "delta": [ { "retain": 4 }, { "insert": { "table": { "id": "7a1b", "widths": [3.25, 3.25], "rows": [[[], []]] } } }, { "retain": 1 } ] },
            { "cell": { "table": "7a1b", "row": 0, "col": 0 }, "delta": [ { "insert": "Item" } ] },
            { "cell": { "table": "7a1b", "row": 0, "col": 0 }, "delta": [ { "retain": 4, "attrs": { "bold": true } } ] },
            { "cell": { "table": "7a1b", "row": 0, "col": 1 }, "delta": [ { "insert": "Cost" } ] },
            { "table_row": { "table": "7a1b", "at": 1, "action": "insert" } },
            { "delta": [ { "retain": 4 }, { "insert": " ", "attrs": { "code": true } }, { "retain": 2 } ] }
        ]))
        .unwrap();
        let mut page = Page::new(Kind::List, "Draft");
        for op in &ops {
            apply(&mut page, op).unwrap();
        }
        let cell = |text: &str, attrs: Value| json!({ "text": text, "runs": if text.is_empty() { json!([]) } else { json!([{ "start": 0, "end": 4, "attrs": attrs }]) } });
        assert_eq!(
            serde_json::to_value(view(&page, ops.len() as u64, 5)).unwrap(),
            json!({
                "head": 8,
                "meta": { "title": "Report", "kind": "page" },
                "body": { "text": "Cost \u{FFFC}\n", "cursor": 5,
                    "paragraphs": [ { "start": 0, "end": 7, "attrs": {},
                        "runs": [ { "start": 0, "end": 4, "attrs": {} }, { "start": 4, "end": 5, "attrs": { "code": true } }, { "start": 5, "end": 6, "attrs": {} } ],
                        "embeds": [ { "at": 5, "table": { "id": "7a1b", "widths": [3.25, 3.25],
                            "rows": [ [ cell("Item", json!({ "bold": true })), cell("Cost", json!({})) ], [ cell("", json!({})), cell("", json!({})) ] ] } } ] } ] }
            })
        );
        assert_eq!(
            serde_json::from_value::<Vec<Op>>(serde_json::to_value(&ops).unwrap()).unwrap(),
            ops,
            "every op round-trips"
        );
    }
}
