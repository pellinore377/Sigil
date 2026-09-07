use super::*;

#[test]
fn chunks_authenticate_every_byte_and_bind_key_file_position_and_length() {
    let key = FileKey::generate(23).unwrap();
    let text = b"synthetic file contents";
    let first = key.seal_chunk(0, text).unwrap();
    let second = key.seal_chunk(0, text).unwrap();
    assert_ne!(first, second);
    assert_eq!(key.open_chunk(0, &first).unwrap().as_slice(), text);
    for i in 0..first.len() {
        let mut bad = first.clone();
        bad[i] ^= 1;
        assert!(key.open_chunk(0, &bad).is_err());
    }
    for n in 0..first.len() {
        assert!(key.open_chunk(0, &first[..n]).is_err());
    }
    let mut trailing = first.clone();
    trailing.push(0);
    assert!(key.open_chunk(0, &trailing).is_err());
    assert!(key.open_chunk(1, &first).is_err());
    assert!(FileKey::generate(23)
        .unwrap()
        .open_chunk(0, &first)
        .is_err());
    let wrong = FileKey {
        shape: key.shape,
        master: Secret32::from_bytes([0; 32]),
    };
    assert!(wrong.open_chunk(0, &first).is_err());
    assert!(key.seal_chunk(0, b"changed size").is_err());
    assert!(key.seal_chunk(1, text).is_err());
}

#[test]
fn empty_full_and_final_chunks_obey_exact_bounds_without_whole_file_allocation() {
    let empty = FileKey::generate(0).unwrap();
    assert_eq!(empty.shape().chunks().unwrap(), 1);
    let chunk = empty.seal_chunk(0, b"").unwrap();
    assert_eq!(chunk.len(), CHUNK_OVERHEAD);
    assert!(empty.open_chunk(0, &chunk).unwrap().is_empty());
    for length in [CHUNK_SIZE as u64, CHUNK_SIZE as u64 + 17] {
        let key = FileKey::generate(length).unwrap();
        let first = vec![5; CHUNK_SIZE];
        let bytes = key.seal_chunk(0, &first).unwrap();
        assert_eq!(bytes.len(), CHUNK_SIZE + CHUNK_OVERHEAD);
        assert_eq!(key.open_chunk(0, &bytes).unwrap().as_slice(), first);
        if length > CHUNK_SIZE as u64 {
            assert_eq!(key.shape.chunk_length(1).unwrap(), 17);
            assert!(key.seal_chunk(1, &first).is_err());
            let last = key.seal_chunk(1, &[6; 17]).unwrap();
            assert!(key.open_chunk(0, &last).is_err());
            assert_eq!(key.open_chunk(1, &last).unwrap().as_slice(), [6; 17]);
        } else {
            assert!(key.shape.chunk_length(1).is_err());
        }
    }
    let largest = Shape {
        file: [1; 32],
        length: MAX_FILE_BYTES,
    };
    assert_eq!(largest.chunks().unwrap(), 1048576);
    assert_eq!(largest.chunk_length(1048575).unwrap(), CHUNK_SIZE);
    assert!(largest.chunk_length(1048576).is_err());
    assert!(FileKey::generate(MAX_FILE_BYTES + 1).is_err());
    assert!(Shape {
        file: [1; 32],
        length: u64::MAX
    }
    .ciphertext_length()
    .is_err());
}

#[test]
fn descriptor_and_ordered_commitment_reject_truncation_incomplete_coverage_and_substitution() {
    let key = FileKey::generate(CHUNK_SIZE as u64 + 3).unwrap();
    let first = key.seal_chunk(0, &vec![1; CHUNK_SIZE]).unwrap();
    let last = key.seal_chunk(1, b"end").unwrap();
    assert!(CiphertextList::new(key.shape).unwrap().finish().is_err());
    let mut list = CiphertextList::new(key.shape).unwrap();
    assert!(list.push(1, &last).is_err());
    let first_hash = list.push(0, &first).unwrap();
    assert!(list.push(0, &first).is_err());
    list.push(1, &last).unwrap();
    assert!(list.push_hash(2, [0; 32]).is_err());
    let root = list.finish().unwrap();
    let mut restored = CiphertextList::new(key.shape).unwrap();
    restored.push_hash(0, first_hash).unwrap();
    restored.push(1, &last).unwrap();
    assert_eq!(restored.finish().unwrap(), root);
    let mut replaced = CiphertextList::new(key.shape).unwrap();
    replaced.push_hash(0, [0; 32]).unwrap();
    replaced.push(1, &last).unwrap();
    assert_ne!(replaced.finish().unwrap(), root);
    let descriptor = key.descriptor(root);
    assert_eq!(descriptor.len(), DESCRIPTOR_SIZE);
    let (decoded, expected) = FileKey::from_descriptor(&descriptor).unwrap();
    assert_eq!(expected, root);
    assert_eq!(decoded.shape, key.shape);
    assert_eq!(decoded.open_chunk(1, &last).unwrap().as_slice(), b"end");
    for n in 0..descriptor.len() {
        assert!(FileKey::from_descriptor(&descriptor[..n]).is_err());
    }
    let mut trailing = descriptor.to_vec();
    trailing.push(0);
    assert!(FileKey::from_descriptor(&trailing).is_err());
    for offset in (0..8).chain(48..52) {
        let mut bad = descriptor.to_vec();
        bad[offset] ^= 128;
        assert!(FileKey::from_descriptor(&bad).is_err());
    }
    let mut bad = descriptor.to_vec();
    bad[40..48].copy_from_slice(&u64::MAX.to_be_bytes());
    assert!(FileKey::from_descriptor(&bad).is_err());
}

#[test]
fn chunks_and_file_commitments_match_independent_openssl_fixtures() {
    fn bytes(value: &serde_json::Value) -> Vec<u8> {
        value
            .as_str()
            .unwrap()
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
            .collect()
    }
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("../tests/vectors/attachments.json")).unwrap();
    for case in cases.as_array().unwrap() {
        let length = case["length"].as_u64().unwrap();
        let key = FileKey {
            shape: Shape {
                file: [6; 32],
                length,
            },
            master: Secret32::from_bytes([7; 32]),
        };
        let mut list = CiphertextList::new(key.shape).unwrap();
        for (index, part) in case["chunks"].as_array().unwrap().iter().enumerate() {
            let index = index as u32;
            let plaintext: Vec<u8> = (0..key.shape.chunk_length(index).unwrap())
                .map(|i| {
                    (i as u8)
                        .wrapping_mul(29)
                        .wrapping_add(17)
                        .wrapping_add(index as u8)
                })
                .collect();
            let nonce: [u8; 12] = bytes(&part["nonce"]).try_into().unwrap();
            let encoded = key.seal_with_nonce(index, &plaintext, &nonce).unwrap();
            assert_eq!(
                key.chunk_key(index).unwrap().0.as_ref(),
                bytes(&part["key"])
            );
            assert_eq!(Sha256::digest(&encoded).as_slice(), bytes(&part["hash"]));
            if part["ciphertext"].is_string() {
                let independent = bytes(&part["ciphertext"]);
                assert_eq!(encoded, independent);
                assert_eq!(
                    key.open_chunk(index, &independent).unwrap().as_slice(),
                    plaintext
                );
            }
            list.push(index, &encoded).unwrap();
        }
        let root = list.finish().unwrap();
        assert_eq!(root.as_slice(), bytes(&case["root"]));
        assert_eq!(key.descriptor(root).as_slice(), bytes(&case["descriptor"]));
    }
}
