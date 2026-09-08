use sigil_maps::{Assets, Error};
#[test]
fn local_assets_preserve_bytes_and_reject_remote_urls_and_path_escape() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let style=br#"{"version":8,"sources":{"local":{"type":"vector","url":"/client/v0/maps/tiles.json"}},"glyphs":"/client/v0/maps/assets/fonts/{fontstack}/{range}.pbf","sprite":"/client/v0/maps/assets/sprite","layers":[]}"#;
    std::fs::write(root.path().join("style.json"), style).unwrap();
    std::fs::create_dir_all(root.path().join("fonts/Synthetic")).unwrap();
    std::fs::write(root.path().join("fonts/Synthetic/0-255.pbf"), b"synthetic").unwrap();
    std::fs::write(outside.path().join("hidden.json"), b"{}").unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("hidden.json"),
        root.path().join("escape.json"),
    )
    .unwrap();
    let assets = Assets::open(root.path()).unwrap();
    assert_eq!(assets.style(), style);
    assert_eq!(
        assets.asset("fonts/Synthetic/0-255.pbf").unwrap().0,
        b"synthetic"
    );
    for path in [
        "../hidden.json",
        "/etc/hidden.json",
        "fonts/%2e%2e/hidden.json",
        "escape.json",
    ] {
        assert!(matches!(assets.asset(path), Err(Error::Invalid)));
    }
    std::fs::write(root.path().join("style.json"),br#"{"version":8,"sources":{"leak":{"type":"vector","url":"https://external.example/tiles"}},"layers":[]}"#).unwrap();
    assert!(matches!(Assets::open(root.path()), Err(Error::Invalid)));
    std::fs::write(root.path().join("style.json"),br#"{"version":8,"sources":{"local":{"type":"vector","url":"/client/v0/maps/tiles.json","attribution":"<img src='https://external.example/track'><a href='https://credits.example'>Synthetic credits</a> &lt;literal&gt;"}},"layers":[]}"#).unwrap();
    let assets = Assets::open(root.path()).unwrap();
    let value: serde_json::Value = serde_json::from_slice(assets.style()).unwrap();
    let credits = value["sources"]["local"]["attribution"].as_str().unwrap();
    assert!(!credits.contains('<'));
    assert!(credits.contains("Synthetic credits"));
    assert!(credits.contains("credits.example"));
}
