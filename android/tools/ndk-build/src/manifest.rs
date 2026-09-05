use crate::error::NdkError;
use serde::{Deserialize, Serialize, Serializer};
use std::{fs::File, path::Path};

/// Android [manifest element](https://developer.android.com/guide/topics/manifest/manifest-element), containing an [`Application`] element.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename = "manifest")]
pub struct AndroidManifest {
    #[serde(rename(serialize = "xmlns:android"))]
    #[serde(default = "default_namespace")]
    ns_android: String,
    #[serde(default)]
    pub package: String,
    #[serde(rename(serialize = "android:sharedUserId"))]
    pub shared_user_id: Option<String>,
    #[serde(rename(serialize = "android:versionCode"))]
    pub version_code: Option<u32>,
    #[serde(rename(serialize = "android:versionName"))]
    pub version_name: Option<String>,

    #[serde(rename(serialize = "uses-sdk"))]
    #[serde(default)]
    pub sdk: Sdk,

    #[serde(rename(serialize = "uses-feature"))]
    #[serde(default)]
    pub uses_feature: Vec<Feature>,
    #[serde(rename(serialize = "uses-permission"))]
    #[serde(default)]
    pub uses_permission: Vec<Permission>,

    #[serde(default)]
    pub queries: Option<Queries>,

    #[serde(default)]
    pub application: Application,
}

impl Default for AndroidManifest {
    fn default() -> Self {
        Self {
            ns_android: default_namespace(),
            package: Default::default(),
            shared_user_id: Default::default(),
            version_code: Default::default(),
            version_name: Default::default(),
            sdk: Default::default(),
            uses_feature: Default::default(),
            uses_permission: Default::default(),
            queries: Default::default(),
            application: Default::default(),
        }
    }
}

impl AndroidManifest {
    pub fn write_to(&self, dir: &Path) -> Result<(), NdkError> {
        let file = File::create(dir.join("AndroidManifest.xml"))?;
        let w = std::io::BufWriter::new(file);
        quick_xml::se::to_writer(w, &self)?;
        Ok(())
    }
}

/// Android [application element](https://developer.android.com/guide/topics/manifest/application-element), containing an [`Activity`] element.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Application {
    #[serde(rename(serialize = "android:debuggable"))]
    pub debuggable: Option<bool>,
    #[serde(rename(serialize = "android:theme"))]
    pub theme: Option<String>,
    // SIGIL PATCH: `has_code` is read straight from
    // `[package.metadata.android.application] has_code = true` (the rename is
    // serialize-only, so the TOML key is the field name), and nothing forces
    // it back to false for native-only apps. `cargo-apk` additionally turns it
    // on by itself whenever `[package.metadata.android] dex = "..."` names a
    // dex to pack, so the two can never disagree.
    #[serde(rename(serialize = "android:hasCode"))]
    #[serde(default)]
    pub has_code: bool,
    #[serde(rename(serialize = "android:icon"))]
    pub icon: Option<String>,
    #[serde(rename(serialize = "android:label"))]
    #[serde(default)]
    pub label: String,
    #[serde(rename(serialize = "android:extractNativeLibs"))]
    pub extract_native_libs: Option<bool>,
    #[serde(rename(serialize = "android:allowBackup"))]
    pub allow_backup: Option<bool>,
    #[serde(rename(serialize = "android:usesCleartextTraffic"))]
    pub uses_cleartext_traffic: Option<bool>,

    #[serde(rename(serialize = "meta-data"))]
    #[serde(default)]
    pub meta_data: Vec<MetaData>,
    #[serde(default)]
    pub activity: Activity,
    // SIGIL PATCH: the app needs components the system instantiates itself — a
    // foreground Service and a BroadcastReceiver for notification actions —
    // and those only exist if they are declared here.
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub service: Vec<Service>,
    // SIGIL PATCH: see `service` above.
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub receiver: Vec<Receiver>,
}

/// Android [activity element](https://developer.android.com/guide/topics/manifest/activity-element).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Activity {
    #[serde(rename(serialize = "android:configChanges"))]
    #[serde(default = "default_config_changes")]
    pub config_changes: Option<String>,
    #[serde(rename(serialize = "android:label"))]
    pub label: Option<String>,
    #[serde(rename(serialize = "android:launchMode"))]
    pub launch_mode: Option<String>,
    #[serde(rename(serialize = "android:name"))]
    #[serde(default = "default_activity_name")]
    pub name: String,
    #[serde(rename(serialize = "android:screenOrientation"))]
    pub orientation: Option<String>,
    #[serde(rename(serialize = "android:exported"))]
    pub exported: Option<bool>,
    #[serde(rename(serialize = "android:resizeableActivity"))]
    pub resizeable_activity: Option<bool>,
    #[serde(rename(serialize = "android:alwaysRetainTaskState"))]
    pub always_retain_task_state: Option<bool>,

    #[serde(rename(serialize = "meta-data"))]
    #[serde(default)]
    pub meta_data: Vec<MetaData>,
    /// If no `MAIN` action exists in any intent filter, a default `MAIN` filter is serialized by `cargo-apk`.
    #[serde(rename(serialize = "intent-filter"))]
    #[serde(default)]
    pub intent_filter: Vec<IntentFilter>,
}

impl Default for Activity {
    fn default() -> Self {
        Self {
            config_changes: default_config_changes(),
            label: None,
            launch_mode: None,
            name: default_activity_name(),
            orientation: None,
            exported: None,
            resizeable_activity: None,
            always_retain_task_state: None,
            meta_data: Default::default(),
            intent_filter: Default::default(),
        }
    }
}

// SIGIL PATCH: `<service>` and `<receiver>`. Upstream `cargo-apk` only ever
// describes the one NativeActivity, because a native-only app never has a
// class for the system to instantiate. A foreground service and a
// notification-action receiver are both instantiated by the system from their
// manifest name, so they have to be declared here and live in the APK's own
// `classes.dex` (see the `dex` key handled in `cargo-apk`).

/// Android [service element](https://developer.android.com/guide/topics/manifest/service-element).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Service {
    #[serde(rename(serialize = "android:name"))]
    pub name: String,
    #[serde(rename(serialize = "android:exported"))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exported: Option<bool>,
    #[serde(rename(serialize = "android:foregroundServiceType"))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub foreground_service_type: Option<String>,
    #[serde(rename(serialize = "android:permission"))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,

    #[serde(rename(serialize = "intent-filter"))]
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub intent_filter: Vec<IntentFilter>,
}

/// Android [receiver element](https://developer.android.com/guide/topics/manifest/receiver-element).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Receiver {
    #[serde(rename(serialize = "android:name"))]
    pub name: String,
    #[serde(rename(serialize = "android:exported"))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exported: Option<bool>,
    #[serde(rename(serialize = "android:permission"))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,

    #[serde(rename(serialize = "intent-filter"))]
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub intent_filter: Vec<IntentFilter>,
}

/// Android [intent filter element](https://developer.android.com/guide/topics/manifest/intent-filter-element).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct IntentFilter {
    /// Serialize strings wrapped in `<action android:name="..." />`
    #[serde(serialize_with = "serialize_actions")]
    #[serde(rename(serialize = "action"))]
    #[serde(default)]
    pub actions: Vec<String>,
    /// Serialize as vector of structs for proper xml formatting
    #[serde(serialize_with = "serialize_catergories")]
    #[serde(rename(serialize = "category"))]
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub data: Vec<IntentFilterData>,
}

fn serialize_actions<S>(actions: &[String], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    use serde::ser::SerializeSeq;

    #[derive(Serialize)]
    struct Action {
        #[serde(rename = "android:name")]
        name: String,
    }
    let mut seq = serializer.serialize_seq(Some(actions.len()))?;
    for action in actions {
        seq.serialize_element(&Action {
            name: action.clone(),
        })?;
    }
    seq.end()
}

fn serialize_catergories<S>(categories: &[String], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    use serde::ser::SerializeSeq;

    #[derive(Serialize)]
    struct Category {
        #[serde(rename = "android:name")]
        pub name: String,
    }

    let mut seq = serializer.serialize_seq(Some(categories.len()))?;
    for category in categories {
        seq.serialize_element(&Category {
            name: category.clone(),
        })?;
    }
    seq.end()
}

/// Android [intent filter data element](https://developer.android.com/guide/topics/manifest/data-element).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct IntentFilterData {
    #[serde(rename(serialize = "android:scheme"))]
    pub scheme: Option<String>,
    #[serde(rename(serialize = "android:host"))]
    pub host: Option<String>,
    #[serde(rename(serialize = "android:port"))]
    pub port: Option<String>,
    #[serde(rename(serialize = "android:path"))]
    pub path: Option<String>,
    #[serde(rename(serialize = "android:pathPattern"))]
    pub path_pattern: Option<String>,
    #[serde(rename(serialize = "android:pathPrefix"))]
    pub path_prefix: Option<String>,
    #[serde(rename(serialize = "android:mimeType"))]
    pub mime_type: Option<String>,
}

/// Android [meta-data element](https://developer.android.com/guide/topics/manifest/meta-data-element).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct MetaData {
    #[serde(rename(serialize = "android:name"))]
    pub name: String,
    #[serde(rename(serialize = "android:value"))]
    pub value: String,
}

/// Android [uses-feature element](https://developer.android.com/guide/topics/manifest/uses-feature-element).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Feature {
    #[serde(rename(serialize = "android:name"))]
    pub name: Option<String>,
    #[serde(rename(serialize = "android:required"))]
    pub required: Option<bool>,
    /// The `version` field is currently used for the following features:
    ///
    /// - `name="android.hardware.vulkan.compute"`: The minimum level of compute features required. See the [Android documentation](https://developer.android.com/reference/android/content/pm/PackageManager#FEATURE_VULKAN_HARDWARE_COMPUTE)
    ///   for available levels and the respective Vulkan features required/provided.
    ///
    /// - `name="android.hardware.vulkan.level"`: The minimum Vulkan requirements. See the [Android documentation](https://developer.android.com/reference/android/content/pm/PackageManager#FEATURE_VULKAN_HARDWARE_LEVEL)
    ///   for available levels and the respective Vulkan features required/provided.
    ///
    /// - `name="android.hardware.vulkan.version"`: Represents the value of Vulkan's `VkPhysicalDeviceProperties::apiVersion`. See the [Android documentation](https://developer.android.com/reference/android/content/pm/PackageManager#FEATURE_VULKAN_HARDWARE_VERSION)
    ///    for available levels and the respective Vulkan features required/provided.
    #[serde(rename(serialize = "android:version"))]
    pub version: Option<u32>,
    #[serde(rename(serialize = "android:glEsVersion"))]
    #[serde(serialize_with = "serialize_opengles_version")]
    pub opengles_version: Option<(u8, u8)>,
}

fn serialize_opengles_version<S>(
    version: &Option<(u8, u8)>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match version {
        Some(version) => {
            let opengles_version = format!("0x{:04}{:04}", version.0, version.1);
            serializer.serialize_some(&opengles_version)
        }
        None => serializer.serialize_none(),
    }
}

/// Android [uses-permission element](https://developer.android.com/guide/topics/manifest/uses-permission-element).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Permission {
    #[serde(rename(serialize = "android:name"))]
    pub name: String,
    #[serde(rename(serialize = "android:maxSdkVersion"))]
    pub max_sdk_version: Option<u32>,
}

/// Android [package element](https://developer.android.com/guide/topics/manifest/queries-element#package).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Package {
    #[serde(rename(serialize = "android:name"))]
    pub name: String,
}

/// Android [provider element](https://developer.android.com/guide/topics/manifest/queries-element#provider).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct QueryProvider {
    #[serde(rename(serialize = "android:authorities"))]
    pub authorities: String,

    // The specs say only an `authorities` attribute is required for providers contained in a `queries` element
    // however this is required for aapt support and should be made optional if/when cargo-apk migrates to aapt2
    #[serde(rename(serialize = "android:name"))]
    pub name: String,
}

/// Android [queries element](https://developer.android.com/guide/topics/manifest/queries-element).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Queries {
    #[serde(default)]
    pub package: Vec<Package>,
    #[serde(default)]
    pub intent: Vec<IntentFilter>,
    #[serde(default)]
    pub provider: Vec<QueryProvider>,
}

/// Android [uses-sdk element](https://developer.android.com/guide/topics/manifest/uses-sdk-element).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Sdk {
    #[serde(rename(serialize = "android:minSdkVersion"))]
    pub min_sdk_version: Option<u32>,
    #[serde(rename(serialize = "android:targetSdkVersion"))]
    pub target_sdk_version: Option<u32>,
    #[serde(rename(serialize = "android:maxSdkVersion"))]
    pub max_sdk_version: Option<u32>,
}

impl Default for Sdk {
    fn default() -> Self {
        Self {
            min_sdk_version: Some(23),
            target_sdk_version: None,
            max_sdk_version: None,
        }
    }
}

fn default_namespace() -> String {
    "http://schemas.android.com/apk/res/android".to_string()
}

fn default_activity_name() -> String {
    "android.app.NativeActivity".to_string()
}

fn default_config_changes() -> Option<String> {
    Some("orientation|keyboardHidden|screenSize".to_string())
}

// SIGIL PATCH: proof that a service and a receiver reach the XML as real
// children of `<application>`, with their attributes in the `android:`
// namespace and their intent filters nested inside them.
#[cfg(test)]
mod sigil_tests {
    use super::*;

    fn app() -> Application {
        Application {
            label: "Sigil".to_string(),
            has_code: true,
            service: vec![Service {
                name: "com.sigil.slint.SigilCallService".to_string(),
                exported: Some(false),
                foreground_service_type: Some("microphone|phoneCall".to_string()),
                permission: None,
                intent_filter: vec![],
            }],
            receiver: vec![Receiver {
                name: "com.sigil.slint.SigilCallReceiver".to_string(),
                exported: Some(false),
                permission: None,
                intent_filter: vec![IntentFilter {
                    actions: vec!["com.sigil.slint.ANSWER".to_string()],
                    categories: vec![],
                    data: vec![],
                }],
            }],
            ..Default::default()
        }
    }

    #[test]
    fn service_and_receiver_serialize_as_application_children() {
        let xml = quick_xml::se::to_string(&app()).unwrap();

        assert!(xml.contains("android:hasCode=\"true\""), "{}", xml);
        assert!(
            xml.contains(
                "<service android:name=\"com.sigil.slint.SigilCallService\" \
                 android:exported=\"false\" \
                 android:foregroundServiceType=\"microphone|phoneCall\"/>"
            ),
            "{}",
            xml
        );
        assert!(
            xml.contains(
                "<receiver android:name=\"com.sigil.slint.SigilCallReceiver\" \
                 android:exported=\"false\"><intent-filter>\
                 <action android:name=\"com.sigil.slint.ANSWER\"/>\
                 </intent-filter></receiver>"
            ),
            "{}",
            xml
        );
        // Absent options and empty filters leave no trace.
        assert!(!xml.contains("android:permission"), "{}", xml);
    }

    #[test]
    fn empty_service_and_receiver_lists_serialize_to_nothing() {
        let xml = quick_xml::se::to_string(&Application::default()).unwrap();
        assert!(!xml.contains("<service"), "{}", xml);
        assert!(!xml.contains("<receiver"), "{}", xml);
    }

    /// The shape another crate's `Cargo.toml` metadata actually uses.
    #[test]
    fn service_and_receiver_parse_from_cargo_metadata() {
        let toml = r#"
label = "Sigil"
has_code = true

[[service]]
name = "com.sigil.slint.SigilCallService"
exported = false
foreground_service_type = "microphone|phoneCall"

[[receiver]]
name = "com.sigil.slint.SigilCallReceiver"
exported = false

[[receiver.intent_filter]]
actions = ["com.sigil.slint.ANSWER"]
"#;
        let app: Application = toml::from_str(toml).unwrap();
        assert!(app.has_code);
        assert_eq!(app.service.len(), 1);
        assert_eq!(
            app.service[0].foreground_service_type.as_deref(),
            Some("microphone|phoneCall")
        );
        assert_eq!(app.receiver.len(), 1);
        assert_eq!(app.receiver[0].intent_filter.len(), 1);
        assert_eq!(
            app.receiver[0].intent_filter[0].actions,
            vec!["com.sigil.slint.ANSWER".to_string()]
        );
    }
}
