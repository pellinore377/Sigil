//! Adversarial review: inputs and op mixes the convergence suite does not generate.
#![allow(clippy::cloned_ref_to_slice_refs)]
use serde_json::{json, Value};
use sigil_pages::{
    apply, apply_batch, invert_batch, transform_batch, Attrs, Delta, Kind, Op, Page,
};
use std::panic::{catch_unwind, AssertUnwindSafe};

fn op(v: Value) -> Op {
    serde_json::from_value(v).unwrap()
}
fn page(body: Delta) -> Page {
    Page {
        body,
        ..Page::new(Kind::Page, "Doc")
    }
}
/// Applies without unwinding; a panic is reported as the failure.
fn apply_no_panic(p: &Page, o: &Op) -> Result<(), String> {
    let mut p = p.clone();
    match catch_unwind(AssertUnwindSafe(|| apply(&mut p, o))) {
        Ok(r) => r.map_err(str::to_owned),
        Err(_) => Err("PANIC".into()),
    }
}

#[test]
fn a_zero_length_retain_beside_an_embed_must_not_panic() {
    let hr = json!({ "hr": true });
    let base = page(Delta::new().embed(hr.clone(), Attrs::new()).insert("\n"));
    let mut panics = Vec::new();
    for wire in [
        json!({ "delta": [{ "retain": 0 }, { "delete": 1 }, { "retain": 1 }] }),
        json!({ "delta": [{ "delete": 0 }, { "retain": 2 }] }),
        json!({ "delta": [{ "retain": 0 }, { "insert": { "hr": true } }, { "retain": 2 }] }),
    ] {
        let r = apply_no_panic(&base, &op(wire.clone()));
        panics.extend((r == Err("PANIC".into())).then(|| wire.to_string()));
    }
    let table = json!({ "table": { "id": "t", "widths": [1.0], "rows": [[[{ "insert": { "hr": true } }]]] } });
    let base = page(Delta::new().embed(table, Attrs::new()).insert("\n"));
    let cell = op(
        json!({ "cell": { "table": "t", "row": 0, "col": 0 }, "delta": [{ "retain": 0 }, { "delete": 1 }] }),
    );
    if apply_no_panic(&base, &cell) == Err("PANIC".into()) {
        panics.push("cell".into());
    }
    assert!(panics.is_empty(), "panicked on {panics:?}");
}

#[test]
fn retains_summing_past_u32_are_rejected_not_wrapped() {
    let base = page(Delta::new().insert("\n"));
    let wire = op(json!({ "delta": [{ "retain": 4294967295u32 }, { "retain": 2 }] }));
    let r = apply_no_panic(&base, &wire);
    assert!(
        r.is_err() && r != Err("PANIC".into()),
        "overflowing delta: {r:?}"
    );
}

#[test]
fn component_json_is_strict() {
    for bad in [
        json!({ "delta": [{ "retain": 1, "delete": 4 }] }),
        json!({ "delta": [{ "insert": "x", "retain": 9 }] }),
        json!({ "delta": [{ "delete": 1, "attrs": { "bold": true } }] }),
    ] {
        assert!(serde_json::from_value::<Op>(bad.clone()).is_err(), "{bad}");
    }
}

#[test]
fn a_loaded_page_is_canonical_so_replicas_compare_equal() {
    let snapshot = json!({ "meta": { "title": "Doc", "kind": "page" }, "body": [{ "insert": "a" }, { "insert": "b" }, { "insert": "c\n" }] });
    let loaded: Result<Page, _> = serde_json::from_value(snapshot);
    let edit = Op::delta(Delta::new().retain(3).insert("z").retain(1));
    let mut replayed = page(Delta::new().insert("abc\n"));
    apply(&mut replayed, &edit).unwrap();
    if let Ok(mut loaded) = loaded {
        apply(&mut loaded, &edit).unwrap();
        assert_eq!(
            loaded, replayed,
            "same text, different structure after one edit"
        );
    }
    for bad in [
        json!({ "meta": { "title": "", "kind": "page" }, "body": [{ "retain": 1 }] }),
        json!({ "meta": { "title": "", "kind": "page" }, "body": [{ "insert": { "block": { "id": "b", "source": "" } } }, { "insert": { "block": { "id": "b", "source": "" } } }] }),
        json!({ "meta": { "title": "", "kind": "page" }, "body": [{ "insert": 7 }] }),
    ] {
        assert!(
            serde_json::from_value::<Page>(bad.clone()).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn two_concurrent_moves_of_one_table_converge() {
    let t = json!({ "table": { "id": "t", "widths": [1.0], "rows": [[[{ "insert": "a" }]]] } });
    let base = page(
        Delta::new()
            .insert("x\n")
            .embed(t.clone(), Attrs::new())
            .insert("y\n"),
    );
    let to_start = Op::delta(
        Delta::new()
            .embed(t.clone(), Attrs::new())
            .retain(2)
            .delete(1)
            .retain(2),
    );
    let to_end = Op::delta(
        Delta::new()
            .retain(2)
            .delete(1)
            .retain(2)
            .embed(t, Attrs::new()),
    );
    for (l, e) in [(&to_start, &to_end), (&to_end, &to_start)] {
        let (l2, e2) = transform_batch(&[l.clone()], &[e.clone()]).unwrap();
        let mut left = base.clone();
        apply_batch(&mut left, &[e.clone()]).unwrap();
        apply_batch(&mut left, &l2).unwrap();
        let mut right = base.clone();
        apply_batch(&mut right, &[l.clone()]).unwrap();
        apply_batch(&mut right, &e2).unwrap();
        assert_eq!(left, right);
        assert_eq!(left.body.text().matches('\u{FFFC}').count(), 1);
    }
}

#[cfg(feature = "sim")]
mod random {
    use super::*;
    use sigil_pages::sim::{any_page, check_tp1, random_batch, run, Source, Xorshift};
    fn var(name: &str, default: u64) -> u64 {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    }
    fn seeds() -> std::ops::Range<u64> {
        let from = var("ADV_FROM", 7_000_000);
        from..from + var("ADV_ITERATIONS", 200)
    }
    /// Both authors undo concurrently after their batches were ordered A then B; the undos must apply and satisfy TP1.
    #[test]
    fn concurrent_undos_apply_and_converge() {
        for seed in seeds() {
            let mut src = Xorshift(seed.wrapping_mul(0xd1b5_4a32_d192_ed03) | 1);
            let (mut a, mut b, mut c) = (0, 0, 0);
            let p = any_page(&mut src, &mut || {
                c += 1;
                format!("i{c}")
            });
            let (ba, _) = random_batch(&mut src, &p, &mut || {
                a += 1;
                format!("a{a}")
            });
            let (bb, _) = random_batch(&mut src, &p, &mut || {
                b += 1;
                format!("b{b}")
            });
            let ua = invert_batch(&p, &ba).unwrap();
            let ub = invert_batch(&p, &bb).unwrap();
            let (bb2, ba2) = transform_batch(&bb, &ba).unwrap();
            let mut pa = p.clone();
            apply_batch(&mut pa, &ba).unwrap();
            let mut pab = pa.clone();
            apply_batch(&mut pab, &bb2).unwrap();
            let ua2 = transform_batch(&ua, &bb2).unwrap().0;
            let mut pb = p.clone();
            apply_batch(&mut pb, &bb).unwrap();
            let ub2 = transform_batch(&ub, &ba2).unwrap().0;
            for (name, u) in [("A", &ua2), ("B", &ub2)] {
                let mut q = pab.clone();
                if let Err(e) = apply_batch(&mut q, u) {
                    panic!("seed {seed}: undo {name} does not apply: {e}\nbase {p:?}\nA {ba:?}\nB {bb:?}\nundo {u:?}");
                }
            }
            if let Err(e) = check_tp1(&pab, &ua2, &ub2) {
                panic!("seed {seed}: {e}");
            }
            // A third site's fresh edit against both undos.
            let (bc, _) = random_batch(&mut src, &pab, &mut || {
                c += 1;
                format!("c{c}")
            });
            for u in [&ua2, &ub2] {
                if let Err(e) = check_tp1(&pab, u, &bc) {
                    panic!("seed {seed}: {e}");
                }
            }
        }
    }
    /// Longer sessions with more sites than the crate's own test.
    #[test]
    fn long_sessions_with_many_sites_converge() {
        for seed in seeds() {
            let mut src = Xorshift(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ 0xabcdef | 1);
            let sites = 2 + src.below(5) as usize;
            let steps = var("ADV_STEPS", 400) as usize;
            if let Err(e) = run(&mut src, sites, steps) {
                panic!("seed {seed}, {sites} sites: {e}");
            }
        }
    }
}

#[test]
fn honest_concurrent_inserts_under_the_limit_both_apply_in_either_order() {
    let base = page(Delta::new().insert("\n"));
    let big = |c: &str| Op::delta(Delta::new().insert(&c.repeat(600 * 1024)).retain(1));
    let (a, b) = (big("a"), big("b"));
    for (l, e) in [(&a, &b), (&b, &a)] {
        let (l2, e2) = transform_batch(&[l.clone()], &[e.clone()]).unwrap();
        let mut left = base.clone();
        apply_batch(&mut left, &[e.clone()]).unwrap();
        let left_ok = apply_batch(&mut left, &l2);
        let mut right = base.clone();
        apply_batch(&mut right, &[l.clone()]).unwrap();
        let right_ok = apply_batch(&mut right, &e2);
        assert!(
            left_ok.is_ok() && right_ok.is_ok(),
            "server order {left_ok:?}, author rebase {right_ok:?}"
        );
    }
}

#[test]
fn an_accepted_no_op_cell_edit_cannot_block_an_honest_concurrent_batch() {
    let t = json!({ "table": { "id": "t", "widths": [1.0], "rows": [[[{ "insert": "a" }]]] } });
    let base = page(Delta::new().embed(t, Attrs::new()).insert("\n"));
    let poison = [
        Op::delta(Delta::new().delete(1).retain(1)),
        op(json!({ "cell": { "table": "t", "row": 0, "col": 0 }, "delta": [{ "retain": 999 }] })),
    ];
    let mut p = base.clone();
    apply_batch(&mut p, &poison).expect("accepted: the cell edit names a table that is gone");
    let honest = [
        Op::delta(Delta::new().insert("typed").retain(2)),
        Op::cell("t", 0, 0, Delta::new().retain(1).insert("b")),
    ];
    let r = transform_batch(&honest, &poison);
    assert!(r.is_ok(), "honest batch cannot be ordered after it: {r:?}");
}

#[cfg(not(debug_assertions))]
#[test]
fn a_wrapped_length_delta_is_not_accepted_then_used_to_block_honest_edits() {
    let base = page(Delta::new().insert("\n"));
    let wrapped = op(json!({ "delta": [{ "retain": 4294967295u32 }, { "retain": 2 }] }));
    let mut p = base.clone();
    let accepted = apply(&mut p, &wrapped);
    let honest = [Op::delta(Delta::new().insert("x").retain(1))];
    let ordered = transform_batch(&honest, &[wrapped]);
    assert!(
        accepted.is_err() || ordered.is_ok(),
        "accepted {accepted:?}, honest batch after it {ordered:?}"
    );
}
