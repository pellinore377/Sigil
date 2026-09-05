use crate::error::Error;
use ndk_build::apk::StripConfig;
use ndk_build::manifest::AndroidManifest;
use ndk_build::target::Target;
use serde::Deserialize;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Inheritable<T> {
    Value(T),
    Inherited { workspace: bool },
}

pub(crate) struct Manifest {
    pub(crate) version: Inheritable<String>,
    pub(crate) apk_name: Option<String>,
    pub(crate) android_manifest: AndroidManifest,
    pub(crate) build_targets: Vec<Target>,
    pub(crate) assets: Option<PathBuf>,
    pub(crate) resources: Option<PathBuf>,
    pub(crate) runtime_libs: Option<PathBuf>,
    /// SIGIL PATCH: a prebuilt `classes.dex` to pack at the APK root, relative
    /// to the crate root. The app needs Java the system can instantiate by
    /// name (a foreground Service, a notification-action Receiver), which a
    /// runtime `InMemoryDexClassLoader` cannot provide.
    pub(crate) dex: Option<PathBuf>,
    /// Maps profiles to keystores
    pub(crate) signing: HashMap<String, Signing>,
    pub(crate) reverse_port_forward: HashMap<String, String>,
    pub(crate) strip: StripConfig,
}

impl Manifest {
    pub(crate) fn parse_from_toml(path: &Path) -> Result<Self, Error> {
        let toml = Root::parse_from_toml(path)?;
        // Unlikely to fail as cargo-subcommand should give us a `Cargo.toml` containing
        // a `[package]` table (with a matching `name` when requested by the user)
        let package = toml
            .package
            .unwrap_or_else(|| panic!("Manifest `{:?}` must contain a `[package]`", path));
        let metadata = package
            .metadata
            .unwrap_or_default()
            .android
            .unwrap_or_default();
        Ok(Self {
            version: package.version,
            apk_name: metadata.apk_name,
            android_manifest: metadata.android_manifest,
            build_targets: metadata.build_targets,
            assets: metadata.assets,
            resources: metadata.resources,
            runtime_libs: metadata.runtime_libs,
            dex: metadata.dex,
            signing: metadata.signing,
            reverse_port_forward: metadata.reverse_port_forward,
            strip: metadata.strip,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Root {
    pub(crate) package: Option<Package>,
    pub(crate) workspace: Option<Workspace>,
}

impl Root {
    pub(crate) fn parse_from_toml(path: &Path) -> Result<Self, Error> {
        let contents = std::fs::read_to_string(path)?;
        toml::from_str(&contents).map_err(|e| e.into())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Package {
    pub(crate) version: Inheritable<String>,
    pub(crate) metadata: Option<PackageMetadata>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Workspace {
    pub(crate) package: Option<WorkspacePackage>,
}

/// Almost the same as [`Package`], except that this must provide
/// root values instead of possibly inheritable values
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct WorkspacePackage {
    pub(crate) version: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct PackageMetadata {
    android: Option<AndroidMetadata>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct AndroidMetadata {
    apk_name: Option<String>,
    #[serde(flatten)]
    android_manifest: AndroidManifest,
    #[serde(default)]
    build_targets: Vec<Target>,
    assets: Option<PathBuf>,
    resources: Option<PathBuf>,
    runtime_libs: Option<PathBuf>,
    // SIGIL PATCH: see `Manifest::dex`.
    dex: Option<PathBuf>,
    /// Maps profiles to keystores
    #[serde(default)]
    signing: HashMap<String, Signing>,
    /// Set up reverse port forwarding before launching the application
    #[serde(default)]
    reverse_port_forward: HashMap<String, String>,
    #[serde(default)]
    strip: StripConfig,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct Signing {
    pub(crate) path: PathBuf,
    pub(crate) keystore_password: String,
}

// SIGIL PATCH: the `dex` key and the `service`/`receiver` tables, read through
// the real `Cargo.toml` parser and out again as the XML `aapt` is handed.
#[cfg(test)]
mod sigil_tests {
    use super::*;

    const METADATA: &str = r#"
[package]
name = "sigil-slint"
version = "0.1.4"

[package.metadata.android]
package = "com.sigil.slint"
apk_name = "sigil-slint"
dex = "target/java/classes.dex"

[package.metadata.android.sdk]
min_sdk_version = 26
target_sdk_version = 35

[package.metadata.android.application]
label = "Sigil"

[[package.metadata.android.application.service]]
name = "com.sigil.slint.SigilCallService"
exported = false
foreground_service_type = "microphone|phoneCall"

[[package.metadata.android.application.receiver]]
name = "com.sigil.slint.SigilCallReceiver"
exported = false

[[package.metadata.android.application.receiver.intent_filter]]
actions = ["com.sigil.slint.ANSWER", "com.sigil.slint.DECLINE"]
"#;

    fn parse() -> Manifest {
        let dir = std::env::temp_dir().join("sigil-cargo-apk-metadata-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Cargo.toml");
        std::fs::write(&path, METADATA).unwrap();
        Manifest::parse_from_toml(&path).unwrap()
    }

    #[test]
    fn dex_service_and_receiver_are_read_from_metadata() {
        let manifest = parse();
        assert_eq!(
            manifest.dex.as_deref(),
            Some(std::path::Path::new("target/java/classes.dex"))
        );

        let app = &manifest.android_manifest.application;
        assert_eq!(app.service.len(), 1);
        assert_eq!(app.service[0].name, "com.sigil.slint.SigilCallService");
        assert_eq!(app.service[0].exported, Some(false));
        assert_eq!(
            app.service[0].foreground_service_type.as_deref(),
            Some("microphone|phoneCall")
        );
        assert_eq!(app.receiver.len(), 1);
        assert_eq!(app.receiver[0].name, "com.sigil.slint.SigilCallReceiver");
        assert_eq!(
            app.receiver[0].intent_filter[0].actions,
            vec![
                "com.sigil.slint.ANSWER".to_string(),
                "com.sigil.slint.DECLINE".to_string()
            ]
        );
    }

    /// What `create_apk` writes to disk for `aapt`, with `hasCode` on the way
    /// `ApkBuilder::build` sets it once a dex is configured.
    #[test]
    fn the_written_manifest_declares_them() {
        let mut manifest = parse().android_manifest;
        manifest.application.has_code = true;
        let dir = std::env::temp_dir().join("sigil-cargo-apk-manifest-test");
        std::fs::create_dir_all(&dir).unwrap();
        manifest.write_to(&dir).unwrap();
        let xml = std::fs::read_to_string(dir.join("AndroidManifest.xml")).unwrap();
        println!("{}", xml);

        assert!(xml.contains("android:hasCode=\"true\""), "{}", xml);
        assert!(
            xml.contains("<service android:name=\"com.sigil.slint.SigilCallService\""),
            "{}",
            xml
        );
        assert!(
            xml.contains("android:foregroundServiceType=\"microphone|phoneCall\""),
            "{}",
            xml
        );
        assert!(
            xml.contains("<receiver android:name=\"com.sigil.slint.SigilCallReceiver\""),
            "{}",
            xml
        );
        assert!(
            xml.contains("<action android:name=\"com.sigil.slint.DECLINE\"/>"),
            "{}",
            xml
        );
    }
}
