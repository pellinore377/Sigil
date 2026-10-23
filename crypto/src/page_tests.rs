use super::*;
use sigil_protocol::pages::Bytes;

fn hex(value: &serde_json::Value) -> Vec<u8> {
    let text = value.as_str().unwrap();
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}
fn vectors() -> serde_json::Value {
    serde_json::from_str(include_str!("../tests/vectors/pages.json")).unwrap()
}
fn key() -> PageKey {
    PageKey::new([1; 32], 1, [2; 32]).unwrap()
}
const REQUEST: Request = Bytes([5; 16]);
const SLOT: Slot = Bytes([6; 16]);
fn contexts() -> [Context<'static>; 4] {
    [
        Context::Op {
            base: 3,
            request: &REQUEST,
        },
        Context::Snapshot { seq: 3 },
        Context::Presence { slot: &SLOT },
        Context::Label { version: 3 },
    ]
}

#[test]
fn subkeys_and_sealed_values_match_independent_vectors() {
    let v = vectors();
    let key = PageKey::new(
        hex(&v["id"]).try_into().unwrap(),
        v["epoch"].as_u64().unwrap(),
        hex(&v["key"]).try_into().unwrap(),
    )
    .unwrap();
    let nonce: [u8; 12] = hex(&v["nonce"]).try_into().unwrap();
    let request = Bytes(hex(&v["request"]).try_into().unwrap());
    let slot = Bytes(hex(&v["slot"]).try_into().unwrap());
    let cases = v["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 4);
    for (case, (purpose, context)) in cases.iter().zip([
        (
            Purpose::Op,
            Context::Op {
                base: 7,
                request: &request,
            },
        ),
        (Purpose::Snapshot, Context::Snapshot { seq: 9 }),
        (Purpose::Presence, Context::Presence { slot: &slot }),
        (Purpose::Label, Context::Label { version: 4 }),
    ]) {
        assert_eq!(case["purpose"], purpose.name());
        assert_eq!(
            key.subkey(purpose).unwrap().0.as_slice(),
            hex(&case["subkey"])
        );
        let plaintext = hex(&case["plaintext"]);
        let sealed = key.seal_with_nonce(context, &plaintext, &nonce).unwrap();
        assert_eq!(sealed, hex(&case["sealed"]));
        assert_eq!(sealed.len(), plaintext.len() + OVERHEAD);
        assert_eq!(key.open(context, &sealed).unwrap().as_slice(), plaintext);
    }
}

#[test]
fn op_statement_matches_independent_vector_and_signatures_attribute() {
    let v = vectors();
    let s = &v["statement"];
    let key = PageKey::new(
        hex(&v["id"]).try_into().unwrap(),
        v["epoch"].as_u64().unwrap(),
        [9; 32],
    )
    .unwrap();
    let request = Bytes(hex(&v["request"]).try_into().unwrap());
    let base = s["base"].as_u64().unwrap();
    let identity = IdentityKey::generate().unwrap();
    let batch = Batch::sign(
        &identity,
        &key,
        base,
        &request,
        hex(&s["device"]).try_into().unwrap(),
        s["created"].as_u64().unwrap(),
        hex(&s["ops"]),
    )
    .unwrap();
    assert_eq!(
        batch.statement(key.id, key.epoch, base, &request),
        hex(&s["bytes"])
    );
    batch
        .verify(&identity.public_key(), &key, base, &request)
        .unwrap();
    let other = IdentityKey::generate().unwrap();
    assert!(batch
        .verify(&other.public_key(), &key, base, &request)
        .is_err());
    assert!(batch
        .verify(&identity.public_key(), &key, base + 1, &request)
        .is_err());
    assert!(batch
        .verify(&identity.public_key(), &key, base, &Bytes([0; 16]))
        .is_err());
    let next = PageKey::new(key.id, key.epoch + 1, [9; 32]).unwrap();
    assert!(batch
        .verify(&identity.public_key(), &next, base, &request)
        .is_err());
    for change in [
        |b: &mut Batch| b.ops.push(b' '),
        |b: &mut Batch| b.created += 1,
        |b: &mut Batch| b.device[0] ^= 1,
        |b: &mut Batch| b.signature[0] ^= 1,
    ] {
        let mut changed = batch.clone();
        change(&mut changed);
        assert!(changed
            .verify(&identity.public_key(), &key, base, &request)
            .is_err());
    }
}

#[test]
fn batches_seal_and_open_and_unsigned_edits_still_apply() {
    let key = key();
    let identity = IdentityKey::generate().unwrap();
    let batch = Batch::sign(&identity, &key, 3, &REQUEST, [7; 32], 10, b"[1]".to_vec()).unwrap();
    let sealed = key.seal_batch(3, &REQUEST, &batch).unwrap();
    let opened = key.open_batch(3, &REQUEST, &sealed).unwrap();
    assert_eq!(opened, batch);
    assert!(key.open_batch(4, &REQUEST, &sealed).is_err());
    assert!(key.open_batch(3, &Bytes([0; 16]), &sealed).is_err());
    let mut forged = batch.clone();
    forged.signature = [0; 64];
    let sealed = key.seal_batch(3, &REQUEST, &forged).unwrap();
    let opened = key.open_batch(3, &REQUEST, &sealed).unwrap();
    assert_eq!(opened.ops, b"[1]");
    assert!(opened
        .verify(&identity.public_key(), &key, 3, &REQUEST)
        .is_err());
    let bytes = batch.to_bytes().unwrap();
    assert_eq!(Batch::from_bytes(&bytes).unwrap(), batch);
    for n in 0..=BATCH_HEADER {
        assert!(Batch::from_bytes(&bytes[..n]).is_err());
    }
    let mut magic = bytes.to_vec();
    magic[5] = 2;
    assert!(Batch::from_bytes(&magic).is_err());
    let mut empty = batch.clone();
    empty.ops.clear();
    assert!(empty.to_bytes().is_err());
    empty.ops = vec![0; Batch::MAX_OPS];
    let sealed = key.seal_batch(3, &REQUEST, &empty).unwrap();
    assert_eq!(sealed.len(), MAX_OP_BYTES);
    empty.ops.push(0);
    assert!(key.seal_batch(3, &REQUEST, &empty).is_err());
    for (device, created) in [([0; 32], 1), ([1; 32], 0), ([1; 32], u64::MAX)] {
        assert!(Batch::sign(&identity, &key, 0, &REQUEST, device, created, vec![1]).is_err());
    }
}

#[test]
fn tampering_and_every_binding_mismatch_fail() {
    let key = key();
    let plaintext = b"synthetic page content";
    for context in contexts() {
        let sealed = key.seal(context, plaintext).unwrap();
        assert_ne!(sealed, key.seal(context, plaintext).unwrap());
        assert_eq!(key.open(context, &sealed).unwrap().as_slice(), plaintext);
        for i in 0..sealed.len() {
            let mut bad = sealed.clone();
            bad[i] ^= 1;
            assert!(key.open(context, &bad).is_err(), "byte {i}");
        }
        for n in 0..sealed.len() {
            assert!(key.open(context, &sealed[..n]).is_err());
        }
        let mut extra = sealed.clone();
        extra.push(0);
        assert!(key.open(context, &extra).is_err());
        let other_page = PageKey::new([9; 32], 1, [2; 32]).unwrap();
        let other_epoch = PageKey::new([1; 32], 2, [2; 32]).unwrap();
        let other_key = PageKey::new([1; 32], 1, [3; 32]).unwrap();
        for wrong in [other_page, other_epoch, other_key] {
            assert_eq!(wrong.open(context, &sealed), Err(Error::Authentication));
        }
        for other in contexts() {
            if other != context {
                assert!(key.open(other, &sealed).is_err());
            }
        }
    }
    let sealed = key
        .seal(
            Context::Op {
                base: 3,
                request: &REQUEST,
            },
            plaintext,
        )
        .unwrap();
    for context in [
        Context::Op {
            base: 4,
            request: &REQUEST,
        },
        Context::Op {
            base: 3,
            request: &Bytes([0; 16]),
        },
    ] {
        assert_eq!(key.open(context, &sealed), Err(Error::Authentication));
    }
    let sealed = key.seal(Context::Snapshot { seq: 3 }, plaintext).unwrap();
    assert!(key.open(Context::Snapshot { seq: 2 }, &sealed).is_err());
    let sealed = key
        .seal(Context::Presence { slot: &SLOT }, plaintext)
        .unwrap();
    assert!(key
        .open(
            Context::Presence {
                slot: &Bytes([0; 16])
            },
            &sealed
        )
        .is_err());
    let sealed = key.seal(Context::Label { version: 3 }, plaintext).unwrap();
    assert!(key.open(Context::Label { version: 4 }, &sealed).is_err());
    let mut retagged = key.seal(Context::Snapshot { seq: 3 }, plaintext).unwrap();
    retagged[7] = 1;
    assert_eq!(
        key.open(
            Context::Op {
                base: 3,
                request: &REQUEST
            },
            &retagged
        ),
        Err(Error::Authentication)
    );
}

#[test]
fn keys_grants_limits_and_tokens() {
    assert!(PageKey::new([1; 32], 0, [2; 32]).is_err());
    assert!(PageKey::new([0; 32], 1, [2; 32]).is_err());
    assert!(PageKey::new([1; 32], u64::MAX, [2; 32]).is_err());
    let generated = PageKey::generate([1; 32], 1).unwrap();
    assert_ne!(
        *generated.key_bytes(),
        *PageKey::generate([1; 32], 1).unwrap().key_bytes()
    );
    let grant = Grant {
        host: "chat.example".into(),
        id: [1; 32],
        epoch: 1,
        key: [2; 32],
        access: [3; 32],
        manage: None,
    };
    let from = PageKey::from_grant(&grant).unwrap();
    let sealed = key().seal(Context::Snapshot { seq: 0 }, b"x").unwrap();
    assert_eq!(
        from.open(Context::Snapshot { seq: 0 }, &sealed)
            .unwrap()
            .as_slice(),
        b"x"
    );
    assert!(PageKey::from_grant(&Grant { epoch: 0, ..grant }).is_err());
    let key = key();
    for (context, limit) in [
        (Context::Snapshot { seq: 0 }, MAX_SNAPSHOT_BYTES),
        (Context::Presence { slot: &SLOT }, MAX_PRESENCE_BYTES),
        (Context::Label { version: 1 }, MAX_LABEL_BYTES),
        (
            Context::Op {
                base: 0,
                request: &REQUEST,
            },
            MAX_OP_BYTES,
        ),
    ] {
        let sealed = key.seal(context, &vec![0; limit - OVERHEAD]).unwrap();
        assert_eq!(sealed.len(), limit);
        assert_eq!(
            key.seal(context, &vec![0; limit - OVERHEAD + 1]),
            Err(Error::Limit)
        );
    }
    const { assert!(MAX_SNAPSHOT_BYTES - OVERHEAD >= sigil_protocol::pages::MAX_BODY_PLAINTEXT) };
    let token = token().unwrap();
    assert_ne!(*token, *super::token().unwrap());
    assert_eq!(
        token_hash(b"abc"),
        hex(&serde_json::json!(
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        ))
        .as_slice()
    );
}
