//! sigil-jq: the JSON query the shell tests use.
//!
//!   sigil-jq [-f FILE] [--stream] [--assert EXPR]... [--push PATH JSON] [PATH]
//!
//! Reads one JSON document from FILE or stdin (`--stream`: a run of
//! concatenated documents, gathered into an array up to the first cut-off
//! one, as `cli ping --follow` leaves them) and prints the value at PATH:
//! strings bare, anything else as JSON. PATH is a run of segments:
//!
//!   key  .key       object key; a leading key needs no dot
//!   ["!r:id"]       bracketed key, for keys with punctuation
//!   [3]  [-1]       array index, negatives from the end
//!   []              every element: later segments apply to each and the
//!                   result is the collection; on a collection [i] picks one
//!   [?f] [?!f]      the first element (of a collection: all elements) whose
//!   [?f=="v"] [?f!="v"]   field f is truthy / falsy / equal / unequal
//!   .length         length of an array, string, object or collection
//!
//! Each `--assert` is `PATH`, `!PATH` (truthiness), `PATH == JSON` or
//! `PATH != JSON`; the first that fails prints the value and its parent to
//! stderr and exits 1. A missing key or index, or no matching element,
//! is an error too. `--push` appends JSON to the array at PATH (keys and
//! indexes only) and rewrites FILE, or prints the document without one.

use serde_json::Value;
use std::io::Read;

#[derive(Debug, PartialEq)]
enum Seg { Key(String), Index(i64), Each, Filter(String, Option<Value>, bool), Length }

// the `]` closing the bracket opened before `from`, minding quoted strings
fn bracket_end(s: &[u8], from: usize) -> Option<usize> {
    let (mut q, mut j) = (false, from);
    while j < s.len() {
        match s[j] { b'"' => q = !q, b'\\' if q => j += 1, b']' if !q => return Some(j), _ => {} }
        j += 1;
    }
    None
}

fn literal(s: &str) -> Value {
    serde_json::from_str(s).unwrap_or_else(|_| Value::String(s.to_string()))
}

fn parse(path: &str) -> Result<Vec<Seg>, String> {
    let (s, mut i, mut segs) = (path.as_bytes(), 0, Vec::new());
    while i < s.len() {
        match s[i] {
            b'.' => i += 1,
            b'[' => {
                let end = bracket_end(s, i + 1).ok_or_else(|| format!("unclosed [ in {path}"))?;
                let inner = path[i + 1..end].trim();
                segs.push(if inner.is_empty() {
                    Seg::Each
                } else if let Some(f) = inner.strip_prefix('?') {
                    match f.split_once("==").map(|(a, b)| (a, b, false)).or_else(|| f.split_once("!=").map(|(a, b)| (a, b, true))) {
                        Some((a, b, not)) => Seg::Filter(a.trim().into(), Some(literal(b.trim())), not),
                        None => match f.strip_prefix('!') { Some(a) => Seg::Filter(a.trim().into(), None, true), None => Seg::Filter(f.trim().into(), None, false) },
                    }
                } else if inner.starts_with('"') {
                    Seg::Key(serde_json::from_str(inner).map_err(|e| format!("bad key {inner}: {e}"))?)
                } else {
                    Seg::Index(inner.parse().map_err(|_| format!("bad index {inner}"))?)
                });
                i = end + 1;
            }
            _ => {
                let start = i;
                while i < s.len() && !matches!(s[i], b'.' | b'[') { i += 1 }
                let word = &path[start..i];
                segs.push(if word == "length" { Seg::Length } else { Seg::Key(word.into()) });
            }
        }
    }
    Ok(segs)
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

fn hits(v: &Value, field: &str, want: &Option<Value>, not: bool) -> bool {
    let x = v.get(field).unwrap_or(&Value::Null);
    (match want { Some(w) => x == w, None => truthy(x) }) != not
}

fn index(a: &[Value], i: i64) -> Result<Value, String> {
    let n = if i < 0 { a.len() as i64 + i } else { i };
    usize::try_from(n).ok().and_then(|n| a.get(n)).cloned().ok_or_else(|| format!("no index {i} in {} elements", a.len()))
}

fn eval(root: &Value, segs: &[Seg]) -> Result<Value, String> {
    let (mut cur, mut many) = (vec![root.clone()], false);
    for seg in segs {
        match seg {
            Seg::Key(k) if many => cur = cur.iter().map(|v| v.get(k).cloned().unwrap_or(Value::Null)).collect(),
            Seg::Key(k) => cur = vec![cur[0].get(k).cloned().ok_or_else(|| format!("no key {k} in {}", cur[0]))?],
            Seg::Index(i) if many => { cur = vec![index(&cur, *i)?]; many = false }
            Seg::Index(i) => cur = vec![index(cur[0].as_array().ok_or("indexing a non-array")?, *i)?],
            Seg::Each => { cur = cur.iter().flat_map(|v| v.as_array().cloned().unwrap_or_default()).collect(); many = true }
            Seg::Filter(f, want, not) if many => cur.retain(|v| hits(v, f, want, *not)),
            Seg::Filter(f, want, not) => {
                let a = cur[0].as_array().ok_or("filtering a non-array")?;
                cur = vec![a.iter().find(|v| hits(v, f, want, *not)).cloned().ok_or_else(|| format!("no element with {f}"))?];
            }
            Seg::Length => {
                let n = if many { cur.len() } else {
                    match &cur[0] { Value::Array(a) => a.len(), Value::String(s) => s.chars().count(), Value::Object(o) => o.len(), v => return Err(format!("no length for {v}")) }
                };
                cur = vec![n.into()];
                many = false;
            }
        }
    }
    Ok(if many { Value::Array(cur) } else { cur.remove(0) })
}

fn walk_mut<'a>(mut v: &'a mut Value, segs: &[Seg]) -> Result<&'a mut Value, String> {
    for seg in segs {
        v = match seg {
            Seg::Key(k) => v.get_mut(k.as_str()).ok_or_else(|| format!("no key {k}"))?,
            Seg::Index(i) => {
                let n = if *i < 0 { v.as_array().map_or(0, |a| a.len() as i64) + i } else { *i };
                usize::try_from(n).ok().and_then(|n| v.get_mut(n)).ok_or_else(|| format!("no index {i}"))?
            }
            _ => return Err("--push takes keys and indexes only".into()),
        };
    }
    Ok(v)
}

// `PATH`, or `PATH == JSON` / `PATH != JSON` split at the top-level operator
fn split_assert(e: &str) -> (&str, Option<(bool, &str)>) {
    let (s, mut q, mut d, mut j) = (e.as_bytes(), false, 0, 0);
    while j + 1 < s.len() {
        match s[j] {
            b'"' => q = !q,
            b'\\' if q => j += 1,
            b'[' if !q => d += 1,
            b']' if !q => d -= 1,
            b'=' | b'!' if !q && d == 0 && s[j + 1] == b'=' => return (&e[..j], Some((s[j] == b'=', &e[j + 2..]))),
            _ => {}
        }
        j += 1;
    }
    (e, None)
}

fn check(root: &Value, expr: &str) -> Result<(), String> {
    let (expr, not) = match expr.trim().strip_prefix('!') { Some(r) => (r, true), None => (expr.trim(), false) };
    let (path, rhs) = split_assert(expr);
    let segs = parse(path.trim())?;
    let got = eval(root, &segs)?;
    let ok = match rhs { Some((eq, r)) => (got == literal(r.trim())) == eq, None => truthy(&got) };
    if ok != not { return Ok(()) }
    let parent = eval(root, &segs[..segs.len().saturating_sub(1)]).unwrap_or(Value::Null);
    Err(format!("assertion failed: {}\n  got: {got}\n  in: {}", expr.trim(), serde_json::to_string_pretty(&parent).unwrap_or_default()))
}

fn run() -> Result<(), String> {
    let (mut file, mut stream, mut asserts, mut push, mut path) = (None, false, Vec::new(), None, None);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-f" => file = args.next(),
            "--stream" => stream = true,
            "--assert" => asserts.push(args.next().ok_or("--assert needs an expression")?),
            "--push" => push = Some((args.next().ok_or("--push needs a path")?, args.next().ok_or("--push needs a value")?)),
            "-h" | "--help" => { print!("{}", include_str!("jq.rs").lines().take_while(|l| l.starts_with("//!")).map(|l| format!("{}\n", &l[3..].trim_start_matches(' '))).collect::<String>()); return Ok(()) }
            _ => path = Some(a),
        }
    }
    let mut text = String::new();
    match &file { Some(f) => text = std::fs::read_to_string(f).map_err(|e| format!("{f}: {e}"))?, None => { std::io::stdin().read_to_string(&mut text).map_err(|e| e.to_string())?; } }
    let mut root = if stream {
        Value::Array(serde_json::Deserializer::from_str(&text).into_iter::<Value>().map_while(Result::ok).collect())
    } else {
        serde_json::from_str(&text).map_err(|e| format!("not JSON: {e}"))?
    };
    for a in &asserts { check(&root, a)? }
    if let Some((p, v)) = push {
        walk_mut(&mut root, &parse(&p)?)?.as_array_mut().ok_or("pushing onto a non-array")?.push(literal(&v));
        return match file {
            Some(f) => std::fs::write(&f, serde_json::to_string_pretty(&root).unwrap_or_default()).map_err(|e| format!("{f}: {e}")),
            None => { println!("{root}"); Ok(()) }
        };
    }
    if let Some(p) = path.or_else(|| asserts.is_empty().then(String::new)) {
        match eval(&root, &parse(&p)?)? { Value::String(s) => println!("{s}"), v => println!("{v}") }
    }
    Ok(())
}

fn main() {
    if let Err(e) = run() { eprintln!("sigil-jq: {e}"); std::process::exit(1) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn grammar() {
        assert_eq!(parse("result.roomId").unwrap(), vec![Seg::Key("result".into()), Seg::Key("roomId".into())]);
        assert_eq!(parse(r#"["!r:id"][-1].body"#).unwrap(), vec![Seg::Key("!r:id".into()), Seg::Index(-1), Seg::Key("body".into())]);
        assert_eq!(parse("rooms[?isInvite].id").unwrap(), vec![Seg::Key("rooms".into()), Seg::Filter("isInvite".into(), None, false), Seg::Key("id".into())]);
        assert_eq!(parse(r#"[?id=="a]b"][?!x].length"#).unwrap(), vec![Seg::Filter("id".into(), Some(json!("a]b")), false), Seg::Filter("x".into(), None, true), Seg::Length]);
        assert_eq!(parse("[][?n!=1][0]").unwrap(), vec![Seg::Each, Seg::Filter("n".into(), Some(json!(1)), true), Seg::Index(0)]);
        assert!(parse("a[1").is_err() && parse("a[x]").is_err());
        assert_eq!(split_assert(r#"[?id=="=="].name == "x""#), (r#"[?id=="=="].name "#, Some((true, r#" "x""#))));
        assert_eq!(split_assert("a.b != null"), ("a.b ", Some((false, " null"))));
        assert_eq!(split_assert("a.b"), ("a.b", None));
    }

    #[test]
    fn evaluation() {
        let v = json!({"ok": true, "h": {"!r:x": [{"b": "one", "k": "file"}, {"b": "two", "k": "text", "e": []}]}, "rooms": [{"id": "1", "inv": false}, {"id": "2", "inv": true}]});
        let q = |p: &str| eval(&v, &parse(p).unwrap());
        assert_eq!(q(r#"h["!r:x"][-1].b"#).unwrap(), json!("two"));
        assert_eq!(q("rooms[?inv].id").unwrap(), json!("2"));
        assert_eq!(q(r#"rooms[?id=="1"].inv"#).unwrap(), json!(false));
        assert_eq!(q(r#"h["!r:x"][][?k!="file"].b"#).unwrap(), json!(["two"]));
        assert_eq!(q(r#"h["!r:x"][][?!e][0].b"#).unwrap(), json!("one"));
        assert_eq!(q(r#"h["!r:x"].length"#).unwrap(), json!(2));
        assert_eq!(q("rooms[][?inv].length").unwrap(), json!(1));
        assert!(q("rooms[?nope]").is_err() && q("h.nope").is_err() && q("rooms[5]").is_err());
        assert!(check(&v, "ok").is_ok() && check(&v, r#"rooms[?inv].id == "2""#).is_ok() && check(&v, "! rooms[0].inv").is_ok());
        assert!(check(&v, r#"rooms[0].id != "1""#).is_err() && check(&v, "rooms[0].inv").is_err());
        let mut m = v.clone();
        walk_mut(&mut m, &parse(r#"h["!r:x"][-1].e"#).unwrap()).unwrap().as_array_mut().unwrap().push(json!(1));
        assert_eq!(eval(&m, &parse(r#"h["!r:x"][1].e"#).unwrap()).unwrap(), json!([1]));
        assert!(walk_mut(&mut m, &parse("rooms[?inv]").unwrap()).is_err());
    }
}
