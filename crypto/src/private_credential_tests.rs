use super::*;

fn attributes() -> Attributes {
    Attributes::for_uid(&std::array::from_fn(|i| i as u8)).unwrap()
}

fn credential(issuer: &Issuer) -> Credential {
    let response = issuer
        .issue(&attributes(), 20_000, b"issuer.example/key/1")
        .unwrap();
    Credential::accept(
        &issuer.public(),
        attributes(),
        20_000,
        b"issuer.example/key/1",
        &response,
    )
    .unwrap()
}

#[test]
fn paper_equations_accept_independent_libsodium_fixture() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../tests/vectors/private-credentials.json")).unwrap();
    let field = |name: &str| -> Vec<u8> {
        fixture[name]
            .as_str()
            .unwrap()
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    };
    let issuer = Issuer(Zeroizing::new(std::array::from_fn(|i| {
        Scalar::from(i as u64 + 1)
    })));
    assert_eq!(issuer.public().to_bytes().as_slice(), field("issuer"));
    let a = attributes();
    assert_eq!(
        [a.0[0].compress().to_bytes(), a.0[1].compress().to_bytes()].concat(),
        field("attributes")
    );
    let accepted = Credential::accept(
        &issuer.public(),
        a,
        20_000,
        b"issuer.example/key/1",
        &field("issuance"),
    )
    .unwrap();
    let group = GroupKey::from_master(Secret32::from_bytes([1; 32])).unwrap();
    assert_eq!(group.public().as_slice(), field("group"));
    let ciphertext = issuer
        .verify_presentation(
            group.public(),
            20_000,
            b"group/1/read/nonce/1",
            &field("presentation"),
        )
        .unwrap();
    assert_eq!(ciphertext, group.ciphertext(&attributes()));
    assert_eq!(
        issuer
            .verify_presentation(
                group.public(),
                20_000,
                b"group/1/read/nonce/1",
                &accepted.present(&group, b"group/1/read/nonce/1").unwrap()
            )
            .unwrap(),
        ciphertext
    );
}

#[test]
fn encrypted_checkpoints_bind_issuer_identity_day_and_record_context() {
    let issuer = Issuer::generate().unwrap();
    let public = issuer.public();
    let key = StorageKey::new(Secret32::from_bytes([9; 32])).unwrap();
    let sealed = issuer
        .seal_checkpoint(&key, b"issuer.example/key/1")
        .unwrap();
    let restored =
        Issuer::open_checkpoint(&key, b"issuer.example/key/1", &sealed, &public).unwrap();
    assert_eq!(restored.public().to_bytes(), public.to_bytes());
    assert!(Issuer::open_checkpoint(&key, b"issuer.example/key/2", &sealed, &public).is_err());
    assert!(Issuer::open_checkpoint(
        &key,
        b"issuer.example/key/1",
        &sealed,
        &Issuer::generate().unwrap().public()
    )
    .is_err());
    let credential = credential(&issuer);
    let sealed = credential
        .seal_checkpoint(&key, b"account/device/day")
        .unwrap();
    let uid = std::array::from_fn(|i| i as u8);
    let restored_credential =
        Credential::open_checkpoint(&key, b"account/device/day", &sealed, &public, &uid, 20_000)
            .unwrap();
    let group = GroupKey::from_master(Secret32::from_bytes([1; 32])).unwrap();
    let presentation = restored_credential
        .present(&group, b"request/nonce")
        .unwrap();
    assert_eq!(
        restored
            .verify_presentation(group.public(), 20_000, b"request/nonce", &presentation)
            .unwrap(),
        group.ciphertext(&attributes())
    );
    assert!(
        Credential::open_checkpoint(&key, b"other/device/day", &sealed, &public, &uid, 20_000)
            .is_err()
    );
    assert!(Credential::open_checkpoint(
        &key,
        b"account/device/day",
        &sealed,
        &public,
        &[255; 16],
        20_000
    )
    .is_err());
    assert!(Credential::open_checkpoint(
        &key,
        b"account/device/day",
        &sealed,
        &public,
        &uid,
        20_001
    )
    .is_err());
    let mut malformed = key.open(&sealed, b"account/device/day").unwrap();
    malformed[140..172].fill(255);
    let invalid = key.seal(&malformed, b"account/device/day").unwrap();
    assert!(Credential::open_checkpoint(
        &key,
        b"account/device/day",
        &invalid,
        &public,
        &uid,
        20_000
    )
    .is_err());
}

#[test]
fn issuance_requires_exact_attributes_date_parameters_context_and_canonical_framing() {
    let issuer = Issuer::generate().unwrap();
    let response = issuer
        .issue(&attributes(), 20_000, b"issuer.example/key/1")
        .unwrap();
    assert_eq!(response.len(), 352);
    assert!(Credential::accept(
        &issuer.public(),
        attributes(),
        20_001,
        b"issuer.example/key/1",
        &response
    )
    .is_err());
    assert!(Credential::accept(
        &Issuer::generate().unwrap().public(),
        attributes(),
        20_000,
        b"issuer.example/key/1",
        &response
    )
    .is_err());
    assert!(Credential::accept(
        &issuer.public(),
        attributes(),
        20_000,
        b"issuer.example/key/2",
        &response
    )
    .is_err());
    for i in 0..2 {
        let mut changed = attributes();
        changed.0[i] = hash_point(b"other-person");
        assert!(Credential::accept(
            &issuer.public(),
            changed,
            20_000,
            b"issuer.example/key/1",
            &response
        )
        .is_err());
    }
    for i in 0..response.len() {
        let mut changed = response.to_vec();
        changed[i] ^= 1;
        assert!(
            Credential::accept(
                &issuer.public(),
                attributes(),
                20_000,
                b"issuer.example/key/1",
                &changed
            )
            .is_err(),
            "byte {i}"
        );
        assert!(Credential::accept(
            &issuer.public(),
            attributes(),
            20_000,
            b"issuer.example/key/1",
            &response[..i]
        )
        .is_err());
    }
    let mut changed = response.to_vec();
    changed.push(0);
    assert!(Credential::accept(
        &issuer.public(),
        attributes(),
        20_000,
        b"issuer.example/key/1",
        &changed
    )
    .is_err());
    changed = response.to_vec();
    changed[..32].fill(255);
    assert!(Credential::accept(
        &issuer.public(),
        attributes(),
        20_000,
        b"issuer.example/key/1",
        &changed
    )
    .is_err());
    assert!(IssuerPublic::from_bytes(&[0; 64]).is_err());
}

#[test]
fn uid_encoding_is_reversible_canonical_and_ciphertext_authenticated() {
    let group = GroupKey::from_master(Secret32::from_bytes([1; 32])).unwrap();
    for i in 0..256u16 {
        let uid = [i as u8; 16];
        let attributes = Attributes::for_uid(&uid).unwrap();
        assert_eq!(decode_uid(attributes.0[1]).unwrap(), uid);
        assert_eq!(
            group.decrypt_uid(&group.ciphertext(&attributes)).unwrap(),
            uid
        );
    }
    let uid = [0; 16];
    let mut noncanonical = encode_uid(&uid).unwrap().compress().to_bytes();
    let start = u16::from_le_bytes(noncanonical[17..19].try_into().unwrap());
    let alternative = ((start + 1)..u16::MAX)
        .find_map(|counter| {
            noncanonical[17..19].copy_from_slice(&counter.to_le_bytes());
            CompressedRistretto(noncanonical).decompress()
        })
        .unwrap();
    assert!(decode_uid(alternative).is_err());
    let ciphertext = group.ciphertext(&attributes());
    assert!(GroupKey::from_master(Secret32::from_bytes([2; 32]))
        .unwrap()
        .decrypt_uid(&ciphertext)
        .is_err());
    for i in 0..64 {
        let mut changed = ciphertext;
        changed[i] ^= 1;
        assert!(group.decrypt_uid(&changed).is_err());
        assert!(group.decrypt_uid(&ciphertext[..i]).is_err());
    }
    assert!(group.decrypt_uid(&[0; 64]).is_err());
}

#[test]
fn presentation_is_randomized_but_ciphertext_is_group_specific_and_statement_bound() {
    let issuer = Issuer::generate().unwrap();
    let credential = credential(&issuer);
    let group = GroupKey::from_master(Secret32::from_bytes([1; 32])).unwrap();
    let other = GroupKey::from_master(Secret32::from_bytes([2; 32])).unwrap();
    let first = credential.present(&group, b"group/1/read/nonce/1").unwrap();
    let second = credential.present(&group, b"group/1/read/nonce/1").unwrap();
    assert_ne!(first, second);
    assert_eq!(&first[192..256], &second[192..256]);
    assert_ne!(
        group.ciphertext(&attributes()),
        other.ciphertext(&attributes())
    );
    for proof in [&first, &second] {
        assert_eq!(
            issuer
                .verify_presentation(group.public(), 20_000, b"group/1/read/nonce/1", proof)
                .unwrap(),
            group.ciphertext(&attributes())
        );
        assert!(issuer
            .verify_presentation(other.public(), 20_000, b"group/1/read/nonce/1", proof)
            .is_err());
        assert!(issuer
            .verify_presentation(group.public(), 20_001, b"group/1/read/nonce/1", proof)
            .is_err());
        assert!(issuer
            .verify_presentation(group.public(), 20_000, b"group/1/write/nonce/1", proof)
            .is_err());
        assert!(issuer
            .verify_presentation(group.public(), 20_000, b"group/1/read/nonce/2", proof)
            .is_err());
        assert!(Issuer::generate()
            .unwrap()
            .verify_presentation(group.public(), 20_000, b"group/1/read/nonce/1", proof)
            .is_err());
    }
    for i in 0..first.len() {
        let mut changed = first.clone();
        changed[i] ^= 1;
        assert!(
            issuer
                .verify_presentation(group.public(), 20_000, b"group/1/read/nonce/1", &changed)
                .is_err(),
            "byte {i}"
        );
        assert!(issuer
            .verify_presentation(group.public(), 20_000, b"group/1/read/nonce/1", &first[..i])
            .is_err());
    }
    let mut changed = first.clone();
    changed.push(0);
    assert!(issuer
        .verify_presentation(group.public(), 20_000, b"group/1/read/nonce/1", &changed)
        .is_err());
    assert!(credential.present(&group, &[0; 4097]).is_err());
    assert!(issuer.issue(&attributes(), 20_000, &[0; 4097]).is_err());
}
