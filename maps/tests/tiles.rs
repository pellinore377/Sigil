use sigil_maps::{Error, Tiles};
use std::{io::Write, os::unix::fs::FileExt};
fn fixture(root: &[u8], leaves: &[u8], compressed: bool) -> Vec<u8> {
    let pack = |bytes: &[u8]| {
        if !compressed {
            return bytes.to_vec();
        }
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        gz.write_all(bytes).unwrap();
        gz.finish().unwrap()
    };
    let root = pack(root);
    let metadata = pack(br#"{"vector_layers":[{"id":"synthetic","fields":{}}]}"#);
    let mut bytes = vec![0; 16384];
    bytes[..8].copy_from_slice(b"PMTiles\x03");
    let regions = [
        (127, root.len() as u64),
        (16384, metadata.len() as u64),
        (16384 + metadata.len() as u64, leaves.len() as u64),
        (16384 + metadata.len() as u64 + leaves.len() as u64, 2),
    ];
    for (i, (offset, len)) in regions.into_iter().enumerate() {
        bytes[8 + i * 16..16 + i * 16].copy_from_slice(&offset.to_le_bytes());
        bytes[16 + i * 16..24 + i * 16].copy_from_slice(&len.to_le_bytes());
    }
    bytes[96..102].copy_from_slice(&[1, if compressed { 2 } else { 1 }, 1, 1, 0, 1]);
    bytes[127..127 + root.len()].copy_from_slice(&root);
    bytes.extend(metadata);
    bytes.extend(leaves);
    bytes.extend([0x1a, 0]);
    bytes
}
#[tokio::test]
async fn embedded_martin_serves_root_leaf_missing_and_compressed_tiles() {
    for (root, leaf, compressed) in [
        (&[1, 0, 1, 2, 1][..], &[][..], false),
        (&[1, 0, 0, 5, 1][..], &[1, 0, 1, 2, 1][..], false),
        (&[1, 0, 1, 2, 1][..], &[][..], true),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("map.pmtiles");
        std::fs::write(&path, fixture(root, leaf, compressed)).unwrap();
        let tiles = Tiles::open(&path).await.unwrap();
        assert_eq!(tiles.tile(0, 0, 0).await.unwrap(), [0x1a, 0]);
        assert!(tiles.tile(1, 1, 1).await.unwrap().is_empty());
        assert_eq!(tiles.content_type(), "application/x-protobuf");
        assert_eq!(tiles.tile(27, 0, 0).await, Err(Error::Invalid));
        assert_eq!(tiles.tile(1, 2, 0).await, Err(Error::Invalid));
        assert_eq!(
            tiles.metadata().unwrap()["vector_layers"][0]["id"],
            "synthetic"
        );
    }
}
#[tokio::test]
async fn bounds_reject_allocation_bombs_bad_ranges_and_modified_sources() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("map.pmtiles");
    for root in [
        &[255, 255, 255, 255, 255, 255, 255, 255, 127][..],
        &[1, 0, 1, 127, 1][..],
    ] {
        std::fs::write(&path, fixture(root, &[], false)).unwrap();
        assert!(matches!(Tiles::open(&path).await, Err(Error::Invalid)));
    }
    let mut bomb = fixture(&[1, 0, 1, 2, 1], &[], false);
    bomb[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
    std::fs::write(&path, bomb).unwrap();
    assert!(matches!(Tiles::open(&path).await, Err(Error::Invalid)));
    std::fs::write(&path, fixture(&[1, 0, 1, 2, 1], &[], false)).unwrap();
    let tiles = Tiles::open(&path).await.unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .write_all_at(b"bad", 0)
        .unwrap();
    assert_eq!(tiles.tile(0, 0, 0).await, Err(Error::Unavailable));
    let huge = vec![0; 2 * 1024 * 1024];
    std::fs::write(&path, fixture(&huge, &[], true)).unwrap();
    assert!(matches!(Tiles::open(&path).await, Err(Error::Invalid)));
}
