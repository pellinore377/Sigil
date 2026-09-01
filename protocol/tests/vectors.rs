//! Verifies `vectors/v1.json` against the implementation, and checks the
//! failure cases the spec requires.

use serde_json::Value;
use sigil_protocol::*;

fn vectors() -> Value {
    serde_json::from_str(include_str!("../vectors/v1.json")).unwrap()
}
fn hx(v: &Value) -> Vec<u8> {
    hex::decode(v.as_str().unwrap()).unwrap()
}
fn a32(v: &Value) -> [u8; 32] {
    hx(v).try_into().unwrap()
}
fn a24(v: &Value) -> [u8; 24] {
    hx(v).try_into().unwrap()
}

#[test]
fn kdf_vectors() {
    let v = &vectors()["kdf"];
    assert_eq!(
        hex::encode(kdf::hash(b"")),
        v["hash_empty"].as_str().unwrap()
    );
    assert_eq!(
        hex::encode(kdf::hash(b"sigil")),
        v["hash_sigil"].as_str().unwrap()
    );
    assert_eq!(hx(&v["kdf_out"]), kdf::kdf("sigil v1 test", b"abc"));
    let n64 = kdf::kdf_n("sigil v1 test", b"abc", 64);
    assert_eq!(hx(&v["kdf_n_64"]), n64);
    assert_eq!(
        &n64[..32],
        &kdf::kdf("sigil v1 test", b"abc"),
        "kdf_n prefix equals kdf"
    );
}

#[test]
fn kem_vectors() {
    let v = &vectors()["kem"];
    let sk = kem::keypair(&a32(&v["seed"]));
    assert_eq!(sk.public()[..], hx(&v["public_key"])[..]);
    let (ct, ss) = kem::encapsulate(sk.public(), &a32(&v["eseed"])).unwrap();
    assert_eq!(ct, hx(&v["ciphertext"]));
    assert_eq!(ss, a32(&v["shared_secret"]));
    assert_eq!(sk.decapsulate(&ct).unwrap(), ss);
    // a flipped ciphertext byte must not yield the same secret
    let mut bad = ct.clone();
    bad[40] ^= 1;
    assert_ne!(sk.decapsulate(&bad).unwrap_or([0; 32]), ss);
    assert_eq!(
        kem::encapsulate(&sk.public()[..100], &a32(&v["eseed"])),
        Err(Error::Length)
    );
}

#[test]
fn identity_vectors() {
    let v = &vectors()["identity"];
    let id = identity::Identity::from_seed(&a32(&v["seed"]));
    assert_eq!(id.public(), a32(&v["identity_pub"]));
    assert_eq!(id.kem.public()[..], hx(&v["kem_pub"])[..]);
    let fp = identity::fingerprint(&id.public());
    assert_eq!(fp, a32(&v["fingerprint"]));
    assert_eq!(
        identity::fingerprint_display(&fp),
        v["fingerprint_display"].as_str().unwrap()
    );
    let signed = hx(&v["card"]["signed"]);
    let card = identity::ContactCard::verify(&signed).unwrap();
    assert_eq!(card.username, v["card"]["username"].as_str().unwrap());
    assert_eq!(card.slot_server, v["card"]["slot_server"].as_str().unwrap());
    assert_eq!(card.flags as u64, v["card"]["flags"].as_u64().unwrap());
    assert_eq!(card.sign(&id), signed, "signing is deterministic");
    let mut tampered = signed.clone();
    tampered[10] ^= 1;
    assert_eq!(identity::ContactCard::verify(&tampered), Err(Error::Auth));
}

#[test]
fn name_vectors() {
    let v = &vectors()["names"];
    let pk = a32(&v["identity_pub"]);
    assert_eq!(names::shelf_address(&pk), a32(&v["shelf_address"]));
    assert_eq!(names::shelf_key(&pk), a32(&v["shelf_key"]));
    let period = v["requests_period"].as_u64().unwrap() as u32;
    let addr = names::requests_address(&pk, period);
    assert_eq!(addr, a32(&v["requests_address"]));
    let proof: [u8; 64] = hx(&v["requests_read_proof"]).try_into().unwrap();
    let nonce = a32(&v["requests_read_nonce"]);
    names::verify_requests_read_proof(&pk, &addr, &nonce, &proof).unwrap();
    let other = names::requests_address(&pk, period + 1);
    assert_eq!(
        names::verify_requests_read_proof(&pk, &other, &nonce, &proof),
        Err(Error::Auth)
    );
    for u in v["valid_usernames"].as_array().unwrap() {
        names::parse_username(u.as_str().unwrap()).unwrap();
    }
    for u in v["invalid_usernames"].as_array().unwrap() {
        assert_eq!(
            names::parse_username(u.as_str().unwrap()),
            Err(Error::Username),
            "{u}"
        );
    }
}

#[test]
fn epoch_and_envelope_vectors() {
    let v = vectors();
    let e = &v["epoch"];
    let ep = epoch::derive(&a32(&e["epoch_secret"]));
    assert_eq!(ep.slot_seed, a32(&e["slot_seed"]));
    assert_eq!(ep.read_cap, a32(&e["read_cap"]));
    assert_eq!(ep.write_key.to_bytes(), a32(&e["write_seed"]));
    assert_eq!(ep.write_pub, a32(&e["write_pub"]));
    assert_eq!(ep.address, a32(&e["address"]));
    assert_eq!(epoch::slot_address(&ep.read_cap, &ep.write_pub), ep.address);
    assert_eq!(ep.envelope_key, a32(&e["envelope_key"]));
    assert_eq!(ep.call_room, a32(&e["call_room"]));

    let n = &v["envelope"];
    let event = envelope::Event {
        kind: n["event"]["kind"].as_u64().unwrap() as u16,
        ts_ms: n["event"]["ts_ms"].as_u64().unwrap(),
        reference: hx(&n["event"]["reference"]),
        body: hx(&n["event"]["body"]),
    };
    let enc = event.encode();
    assert_eq!(enc, hx(&n["event_encoded"]));
    assert_eq!(envelope::Event::decode(&enc).unwrap(), event);
    assert_eq!(envelope::pad(&enc).unwrap(), hx(&n["padded"]));
    let sealed = envelope::seal(&ep.envelope_key, &ep.address, &a24(&n["nonce"]), &enc).unwrap();
    assert_eq!(sealed, hx(&n["envelope"]));
    assert_eq!(sealed.len(), 1024);
    assert_eq!(
        envelope::open(&ep.envelope_key, &ep.address, &sealed).unwrap(),
        enc
    );
    // wrong address in the associated data must fail
    let mut other = ep.address;
    other[0] ^= 1;
    assert_eq!(
        envelope::open(&ep.envelope_key, &other, &sealed),
        Err(Error::Auth)
    );
    let sig: [u8; 64] = hx(&n["put_signature"]).try_into().unwrap();
    epoch::verify_put(&ep.write_pub, &ep.address, &sealed, &sig).unwrap();
    assert_eq!(
        epoch::verify_put(&ep.write_pub, &other, &sealed, &sig),
        Err(Error::Auth)
    );
}

#[test]
fn envelope_buckets() {
    let key = [7u8; 32];
    let addr = [8u8; 32];
    let nonce = [9u8; 24];
    for (plain_len, want) in [
        (0, 1024),
        (983, 1024),
        (984, 4096),
        (4055, 4096),
        (4056, 16384),
        (16343, 16384),
    ] {
        let s = envelope::seal(&key, &addr, &nonce, &vec![1u8; plain_len]).unwrap();
        assert_eq!(s.len(), want, "plain {plain_len}");
        assert_eq!(envelope::open(&key, &addr, &s).unwrap().len(), plain_len);
    }
    assert_eq!(
        envelope::seal(&key, &addr, &nonce, &vec![1u8; 16344]),
        Err(Error::TooLarge)
    );
    assert_eq!(envelope::unpad(&[0, 0, 0]), Err(Error::Padding));
    assert_eq!(envelope::unpad(&[]), Err(Error::Padding));
    assert_eq!(envelope::unpad(&[0x80]).unwrap(), &[] as &[u8]);
}

#[test]
fn bag_vectors() {
    let v = &vectors()["bag"];
    let server = kem::keypair(&a32(&v["server_seed"]));
    assert_eq!(server.public()[..], hx(&v["server_public_key"])[..]);
    let (bag_bytes, k) = bag::seal_request(
        server.public(),
        &a32(&v["eseed"]),
        &a24(&v["request_nonce"]),
        &hx(&v["request"]),
    )
    .unwrap();
    assert_eq!(bag_bytes, hx(&v["bag"]));
    assert_eq!(k.request, a32(&v["request_key"]));
    assert_eq!(k.response, a32(&v["response_key"]));
    let (req, k2) = bag::open_request(&server, &bag_bytes).unwrap();
    assert_eq!(req, hx(&v["request"]));
    let resp = bag::seal_response(&k2, &a24(&v["response_nonce"]), &hx(&v["response"])).unwrap();
    assert_eq!(resp, hx(&v["response_sealed"]));
    assert_eq!(bag::open_response(&k, &resp).unwrap(), hx(&v["response"]));
    let mut bad = bag_bytes.clone();
    bad[2000] ^= 1;
    assert_eq!(bag::open_request(&server, &bad).err(), Some(Error::Auth));
    let other = kem::keypair(&[0x42; 32]);
    assert!(bag::open_request(&other, &bag_bytes).is_err());
}

#[test]
fn recovery_vectors() {
    let v = &vectors()["recovery"];
    let salt: [u8; 16] = hx(&v["salt"]).try_into().unwrap();
    let pw_key = recovery::password_key(v["password"].as_str().unwrap().as_bytes(), &salt);
    assert_eq!(pw_key, a32(&v["pw_key"]));
    let rk = a32(&v["recovery_key"]);
    let bk = recovery::backup_key(&pw_key, &rk);
    assert_eq!(bk, a32(&v["backup_key"]));
    let dk = a32(&v["data_key"]);
    assert_eq!(recovery::backup_label(&dk), a32(&v["backup_label"]));
    let wrap = recovery::wrap_data_key(&bk, &a24(&v["wrap_nonce"]), &dk);
    assert_eq!(wrap, hx(&v["wrapped_data_key"]));
    assert_eq!(recovery::unwrap_data_key(&bk, &wrap).unwrap(), dk);
    assert_eq!(recovery::tpm_auth(&pw_key), a32(&v["tpm_auth"]));
    let code = v["recovery_code"].as_str().unwrap();
    assert_eq!(recovery::recovery_code(&rk), code);
    assert_eq!(recovery::parse_recovery_code(code).unwrap(), rk);
    assert_eq!(
        recovery::parse_recovery_code(&code.to_uppercase().replace('-', " ")).unwrap(),
        rk
    );
    let mut wrong = code.to_string();
    wrong.replace_range(0..1, if code.starts_with('a') { "b" } else { "a" });
    assert_eq!(recovery::parse_recovery_code(&wrong), Err(Error::Auth));
    // wrong password: different backup key, unwrap fails
    let bk2 = recovery::backup_key(&recovery::password_key(b"wrong", &salt), &rk);
    assert_eq!(recovery::unwrap_data_key(&bk2, &wrap), Err(Error::Auth));
}

#[test]
fn linking_vectors() {
    let v = &vectors()["linking"];
    let new_dev = kem::keypair(&a32(&v["new_device_seed"]));
    let offer = linking::LinkOffer {
        kem_pub: new_dev.public().to_vec(),
        nonce: hx(&v["offer_nonce"]).try_into().unwrap(),
    };
    let enc = offer.encode();
    assert_eq!(enc, hx(&v["offer_encoded"]));
    let dec = linking::LinkOffer::decode(&enc).unwrap();
    assert_eq!(dec.kem_pub, offer.kem_pub);
    assert_eq!(
        linking::offer_rendezvous(&offer),
        a32(&v["offer_rendezvous"])
    );
    let (ct, ss) = kem::encapsulate(&offer.kem_pub, &a32(&v["existing_eseed"])).unwrap();
    assert_eq!(ct, hx(&v["ciphertext"]));
    assert_eq!(new_dev.decapsulate(&ct).unwrap(), ss);
    let lm = linking::derive(&ss, &offer);
    assert_eq!(lm.link_secret, a32(&v["link_secret"]));
    assert_eq!(lm.rendezvous, a32(&v["rendezvous"]));
    assert_eq!(lm.link_key, a32(&v["link_key"]));
    let idx: Vec<u8> = v["sas_indices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_u64().unwrap() as u8)
        .collect();
    assert_eq!(lm.sas.to_vec(), idx);
    assert_eq!(linking::sas_string(&lm.sas), v["sas"].as_str().unwrap());
    assert_eq!(emoji::TABLE.len(), 64);
    let mut uniq = emoji::TABLE.to_vec();
    uniq.sort();
    uniq.dedup();
    assert_eq!(uniq.len(), 64, "emoji table has duplicates");
}

#[test]
fn vectors_file_is_current() {
    // The committed file must be exactly what the generator produces.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_gen-vectors"))
        .output()
        .unwrap();
    assert!(out.status.success());
    let generated: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        generated,
        vectors(),
        "run `cargo run --bin gen-vectors > vectors/v1.json`"
    );
}
