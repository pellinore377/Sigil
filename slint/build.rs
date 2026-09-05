fn main() {
    // EmbedFiles so the fonts travel inside the binary: an APK has no source
    // tree to load them from, and the desktop binary should not depend on one.
    let cfg = slint_build::CompilerConfiguration::new()
        .embed_resources(slint_build::EmbedResourcesKind::EmbedFiles);
    slint_build::compile_with_config("ui/app.slint", cfg).unwrap();

    android_picker_dex();
}

/// Compile and dex `java/SigilFilePicker.java`, the Storage Access Framework
/// seam described in that file and used by `src/platform.rs`.
///
/// A file chooser is an Activity result, which needs Java: NativeActivity has
/// no onActivityResult of ours to override. The class is dexed here and
/// embedded in the library, and loaded at runtime through an
/// InMemoryDexClassLoader — cargo-apk packages no Java of its own, so there is
/// no APK dex to put it in.
///
/// The recipe is the one the Slint Android backend already runs for its own
/// helper (vendor/i-slint-backend-android-activity/build.rs), so a build that
/// can produce the APK at all can produce this too: same `android-build`
/// crate, same javac, same d8, same Android jar.
fn android_picker_dex() {
    // Android only, and the check has to be on TARGET rather than a cfg: the
    // build script itself is always compiled for the host.
    if !std::env::var("TARGET").unwrap_or_default().contains("android") {
        return;
    }

    // Every class goes in the one dex. The first four are reached through the
    // InMemoryDexClassLoader described above; the last four are ALSO in the
    // APK's own classes.dex (see the copy at the foot of this function),
    // because a Service and a BroadcastReceiver named in the manifest are
    // instantiated by the system with the app's own class loader and can come
    // from nowhere else.
    let srcs = [
        "java/SigilFilePicker.java",
        "java/SigilVideo.java",
        "java/SigilCamera.java",
        "java/SigilGallery.java",
        "java/SigilNative.java",
        "java/SigilNotify.java",
        "java/SigilReceiver.java",
        "java/SigilService.java",
    ];
    let src = "java/*.java";
    for s in srcs {
        println!("cargo:rerun-if-changed={s}");
    }

    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    // Its own two directories: OUT_DIR already holds Slint's generated code,
    // and d8 always names its output classes.dex.
    let classes = out.join("picker-classes");
    let dex = out.join("picker");
    let _ = std::fs::remove_dir_all(&classes);
    std::fs::create_dir_all(&classes).expect("create the picker class directory");
    std::fs::create_dir_all(&dex).expect("create the picker dex directory");

    let android_jar = android_build::android_jar(None)
        .expect("no Android platform found — set ANDROID_HOME (see android/build-engine.sh)");

    let release = std::env::var("PROFILE").as_deref() == Ok("release");

    // Java 8 bytecode: what d8 and the app's min_sdk of 26 expect.
    let mut build = android_build::JavaBuild::new();
    for s in srcs {
        build.file(s);
    }
    let javac = build
        .class_path(&android_jar)
        .classes_out_dir(&classes)
        .java_source_version(8)
        .java_target_version(8)
        .command()
        .expect("could not build the javac command")
        .args(["-encoding", "UTF-8"])
        .output()
        .expect("could not run javac");
    if !javac.status.success() {
        panic!("{src} did not compile: {}", String::from_utf8_lossy(&javac.stderr));
    }

    let d8 = android_build::Dexer::new()
        .android_jar(&android_jar)
        .class_path(&classes)
        .collect_classes(&classes)
        .expect("could not collect the picker classes")
        .release(release)
        .android_min_api(26) // matches min_sdk_version; one class, so one dex
        .out_dir(&dex)
        .command()
        .expect("could not build the d8 command")
        .output()
        .expect("could not run d8");
    if !d8.status.success() {
        panic!("{src} did not dex: {}", String::from_utf8_lossy(&d8.stderr));
    }

    // The same dex, at a path a human can write down. OUT_DIR is a hashed
    // directory that changes with the profile and the feature set, so
    // `[package.metadata.android] dex` in Cargo.toml — which cargo-apk reads
    // long before this script runs — cannot name it. Everything the system
    // instantiates by name (the foreground Service, the notification-action
    // Receiver) has to be in the APK's own classes.dex, and that is the copy
    // cargo-apk packs; `src/platform.rs` keeps loading the OUT_DIR one for the
    // helpers it reaches through InMemoryDexClassLoader.
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("target")
        });
    let stable = target_dir.join("java");
    std::fs::create_dir_all(&stable).expect("create the packaged dex directory");
    std::fs::copy(dex.join("classes.dex"), stable.join("classes.dex"))
        .expect("copy the dex to the packaged path");
}
