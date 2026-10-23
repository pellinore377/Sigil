#![no_main]
use libfuzzer_sys::fuzz_target;
use sigil_pages::sim::{self, Bytes, Xorshift};
use sigil_pages::{apply_batch, body_patch, invert_batch, prints, transform_batch, view, Op};
fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > 65536 {
        return;
    }
    if let Ok(ops) = serde_json::from_slice::<Vec<Op>>(bytes) {
        assert_eq!(
            serde_json::from_slice::<Vec<Op>>(&serde_json::to_vec(&ops).unwrap()).unwrap(),
            ops
        );
        let mut serial = 0;
        let before = sim::table_page(&mut Xorshift(7), &mut || {
            serial += 1;
            format!("t{serial}")
        });
        let mut page = before.clone();
        if let (Ok(back), Ok(())) = (invert_batch(&page, &ops), apply_batch(&mut page, &ops)) {
            // Any batch the page accepted transforms against any other valid on the same page.
            transform_batch(&ops, &ops).unwrap();
            let old: Vec<u64> = prints(&before.body).into_iter().map(|p| p.0).collect();
            let _ = (view(&page, 0, 0), body_patch(&page.body, 0, &old));
            apply_batch(&mut page, &back).unwrap();
            assert_eq!(page, before);
        }
    }
    let sites = 2 + bytes.first().map_or(0, |b| b % 3) as usize;
    if let Err(e) = sim::run(&mut Bytes(bytes), sites, 32) {
        panic!("{e}");
    }
});
