pub fn maps(root: &std::path::Path) -> sigil_server::maps::Settings {
    let archive = root.join("map.pmtiles");
    let assets = root.join("assets");
    std::fs::create_dir(&assets).unwrap();
    std::fs::write(assets.join("style.json"),br#"{"version":8,"sources":{"local":{"type":"vector","url":"/client/v0/maps/tiles.json"}},"layers":[]}"#).unwrap();
    std::fs::write(assets.join("sprite.json"), b"{}").unwrap();
    let mut bytes = vec![0u8; 16388];
    bytes[..8].copy_from_slice(b"PMTiles\x03");
    for (i, (offset, len)) in [(127u64, 5u64), (16384, 2), (16386, 0), (16386, 2)]
        .into_iter()
        .enumerate()
    {
        bytes[8 + 16 * i..16 + 16 * i].copy_from_slice(&offset.to_le_bytes());
        bytes[16 + 16 * i..24 + 16 * i].copy_from_slice(&len.to_le_bytes());
    }
    bytes[96..102].copy_from_slice(&[1, 1, 1, 1, 0, 0]);
    bytes[127..132].copy_from_slice(&[1, 0, 1, 2, 1]);
    bytes[16384..].copy_from_slice(&[b'{', b'}', 0x1a, 0]);
    std::fs::write(&archive, bytes).unwrap();
    sigil_server::maps::Settings {
        archive: archive.to_str().unwrap().into(),
        assets: assets.to_str().unwrap().into(),
    }
}
