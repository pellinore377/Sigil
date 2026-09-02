fn main() {
    // EmbedFiles so the fonts travel inside the binary: an APK has no source
    // tree to load them from, and the desktop binary should not depend on one.
    let cfg = slint_build::CompilerConfiguration::new()
        .embed_resources(slint_build::EmbedResourcesKind::EmbedFiles);
    slint_build::compile_with_config("ui/app.slint", cfg).unwrap();
}
