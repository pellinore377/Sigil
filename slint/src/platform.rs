//! The per-platform seams, kept to two: where files live, and how a URL opens.
//! Everything else is the same code on every platform.

/// Open a URL in the platform browser. Used for the OIDC login page; the
/// engine finishes the flow on its localhost redirect, which the browser can
/// reach on-device on every platform we ship to.
#[cfg(not(target_os = "android"))]
pub fn open_url(url: &str) {
    // SIGIL_BROWSER names another opener (the tests use curl, headless).
    let opener = std::env::var("SIGIL_BROWSER").unwrap_or_else(|_| "xdg-open".into());
    if let Err(e) = std::process::Command::new(&opener).arg(url).spawn() {
        tracing::warn!("{opener}: {e}");
    }
}

/// One JNI call: startActivity(Intent(ACTION_VIEW, Uri.parse(url))).
/// android-activity has already parked the VM and activity in ndk-context.
#[cfg(target_os = "android")]
pub fn open_url(url: &str) {
    if let Err(e) = open_url_android(url) {
        tracing::warn!("open url: {e:#}");
    }
}

#[cfg(target_os = "android")]
fn open_url_android(url: &str) -> anyhow::Result<()> {
    use jni::objects::{JObject, JValue};
    let ctx = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }?;
    let mut env = vm.attach_current_thread()?;
    let activity = unsafe { JObject::from_raw(ctx.context().cast()) };

    let action = env.new_string("android.intent.action.VIEW")?;
    let url = env.new_string(url)?;
    let uri = env
        .call_static_method(
            "android/net/Uri",
            "parse",
            "(Ljava/lang/String;)Landroid/net/Uri;",
            &[JValue::Object(&url)],
        )?
        .l()?;
    let intent = env.new_object(
        "android/content/Intent",
        "(Ljava/lang/String;Landroid/net/Uri;)V",
        &[JValue::Object(&action), JValue::Object(&uri)],
    )?;
    // ndk-context hands us the *application* context, not the Activity, and
    // startActivity from there demands FLAG_ACTIVITY_NEW_TASK (0x10000000).
    // Without it Android throws, and an uncaught Java exception on a Rust
    // thread kills the whole process — which read as "the button crashes".
    env.call_method(
        &intent,
        "setFlags",
        "(I)Landroid/content/Intent;",
        &[JValue::Int(0x1000_0000)],
    )?;
    let result = env.call_method(
        &activity,
        "startActivity",
        "(Landroid/content/Intent;)V",
        &[JValue::Object(&intent)],
    );
    if env.exception_check().unwrap_or(false) {
        env.exception_describe().ok();
        env.exception_clear().ok(); // a pending exception at detach is fatal
    }
    result?;
    Ok(())
}

/// Put text on the clipboard. Desktop: wl-copy. Android: ClipboardManager.
pub fn copy_text(text: &str) {
    #[cfg(not(target_os = "android"))]
    {
        use std::io::Write;
        if let Ok(mut child) = std::process::Command::new("wl-copy")
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            if let Some(stdin) = child.stdin.as_mut() {
                let _ = stdin.write_all(text.as_bytes());
            }
            return;
        }
        tracing::warn!("wl-copy unavailable; clipboard write dropped");
    }
    #[cfg(target_os = "android")]
    if let Err(e) = copy_text_android(text) {
        tracing::warn!("clipboard: {e:#}");
    }
}

/// getSystemService(CLIPBOARD_SERVICE).setPrimaryClip(ClipData.newPlainText(...)).
#[cfg(target_os = "android")]
fn copy_text_android(text: &str) -> anyhow::Result<()> {
    use jni::objects::{JObject, JValue};
    let ctx = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }?;
    let mut env = vm.attach_current_thread()?;
    let context = unsafe { JObject::from_raw(ctx.context().cast()) };

    let service = env.new_string("clipboard")?;
    let manager = env
        .call_method(
            &context,
            "getSystemService",
            "(Ljava/lang/String;)Ljava/lang/Object;",
            &[JValue::Object(&service)],
        )?
        .l()?;
    anyhow::ensure!(!manager.is_null(), "no clipboard service");

    let label = env.new_string("Sigil")?;
    let value = env.new_string(text)?;
    let clip = env
        .call_static_method(
            "android/content/ClipData",
            "newPlainText",
            "(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Landroid/content/ClipData;",
            &[JValue::Object(&label), JValue::Object(&value)],
        )?
        .l()?;
    let result = env.call_method(
        &manager,
        "setPrimaryClip",
        "(Landroid/content/ClipData;)V",
        &[JValue::Object(&clip)],
    );
    if env.exception_check().unwrap_or(false) {
        env.exception_describe().ok();
        env.exception_clear().ok(); // a pending exception at detach is fatal
    }
    result?;
    Ok(())
}

/// The microphone on Android 6+ is a runtime grant; the manifest entry only
/// declares it. Both calls go through the Activity that android-activity
/// parked in scale.rs — ndk-context only carries the application context,
/// which cannot show the permission dialog.
#[cfg(target_os = "android")]
pub fn has_mic_permission() -> bool {
    match mic_permission_android() {
        Ok(granted) => granted,
        Err(e) => {
            tracing::warn!("mic permission check: {e:#}");
            false
        }
    }
}

#[cfg(target_os = "android")]
pub fn request_mic_permission() {
    if let Err(e) = request_mic_permission_android() {
        tracing::warn!("mic permission request: {e:#}");
    }
}

#[cfg(target_os = "android")]
const RECORD_AUDIO: &str = "android.permission.RECORD_AUDIO";
/// The requestPermissions code; the result comes back as a lifecycle event we
/// do not consume — the person taps record again once granted.
#[cfg(target_os = "android")]
const MIC_REQUEST_CODE: i32 = 7001;

/// checkSelfPermission(RECORD_AUDIO) == PERMISSION_GRANTED (0).
#[cfg(target_os = "android")]
fn mic_permission_android() -> anyhow::Result<bool> {
    use jni::objects::{JObject, JValue};
    let app = crate::scale::android().ok_or_else(|| anyhow::anyhow!("no Android app handle"))?;
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }?;
    let mut env = vm.attach_current_thread()?;
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };

    let perm = env.new_string(RECORD_AUDIO)?;
    let result = env.call_method(
        &activity,
        "checkSelfPermission",
        "(Ljava/lang/String;)I",
        &[JValue::Object(&perm)],
    );
    if env.exception_check().unwrap_or(false) {
        env.exception_describe().ok();
        env.exception_clear().ok(); // a pending exception at detach is fatal
    }
    Ok(result?.i()? == 0)
}

/// requestPermissions(new String[]{RECORD_AUDIO}, code) — shows the dialog.
#[cfg(target_os = "android")]
fn request_mic_permission_android() -> anyhow::Result<()> {
    use jni::objects::{JObject, JValue};
    let app = crate::scale::android().ok_or_else(|| anyhow::anyhow!("no Android app handle"))?;
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }?;
    let mut env = vm.attach_current_thread()?;
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };

    let perm = env.new_string(RECORD_AUDIO)?;
    let perms = env.new_object_array(1, "java/lang/String", &perm)?;
    let result = env.call_method(
        &activity,
        "requestPermissions",
        "([Ljava/lang/String;I)V",
        &[JValue::Object(perms.as_ref()), JValue::Int(MIC_REQUEST_CODE)],
    );
    if env.exception_check().unwrap_or(false) {
        env.exception_describe().ok();
        env.exception_clear().ok(); // a pending exception at detach is fatal
    }
    result?;
    Ok(())
}

/// Pick one file, and answer with a path the engine can send.
///
/// Desktop: omarchy-file-select. Android: the Storage Access Framework, over
/// the glue in java/SigilFilePicker.java — a file chooser there is an Activity
/// result, which NativeActivity gives us no way to receive on its own.
pub async fn pick_file() -> Option<String> {
    #[cfg(not(target_os = "android"))]
    {
        let out = tokio::process::Command::new("omarchy-file-select")
            .arg("--title")
            .arg("Choose picture")
            .output()
            .await
            .ok()?;
        let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if path.is_empty() {
            None
        } else {
            Some(path)
        }
    }
    #[cfg(target_os = "android")]
    {
        android_pick("file").await.into_iter().next()
    }
}

/// Pick pictures and video from the gallery, several at a time.
///
/// Desktop: the same chooser as everything else, which picks one. Android: the
/// system photo picker where the phone has one, and the document picker
/// filtered to images and video where it does not.
pub async fn pick_media() -> Vec<String> {
    #[cfg(not(target_os = "android"))]
    {
        // omarchy-file-select has no multiple-selection mode and no type
        // filter; on the desktop the gallery tile is the file chooser.
        pick_file().await.into_iter().collect()
    }
    #[cfg(target_os = "android")]
    {
        android_pick("media").await
    }
}

/// Take a photo, or record a video, and answer with the file.
///
/// Android hands the job to whatever camera app the phone has: an app that only
/// asks for a picture needs no camera permission of its own, and the shot lands
/// in the phone's gallery the way any camera app's would.
pub async fn capture_media(video: bool) -> Option<String> {
    #[cfg(not(target_os = "android"))]
    {
        // No capture stack on the desktop: calls own the only camera path and
        // it is v4l2 (android/build-engine.sh), which is not wired to sending.
        let _ = video;
        tracing::warn!("capture: there is no camera on this platform");
        None
    }
    #[cfg(target_os = "android")]
    {
        android_pick(if video { "video" } else { "photo" })
            .await
            .into_iter()
            .next()
    }
}

/// One run of the Android picker, off the UI thread.
///
/// JNI is blocking, and the person may spend a minute looking through Drive or
/// framing a shot. Neither the UI thread nor the runtime's workers may wait on
/// that, so the whole conversation runs on a blocking thread — the same shape
/// as core/src/geo/android.rs.
#[cfg(target_os = "android")]
async fn android_pick(mode: &'static str) -> Vec<String> {
    match tokio::task::spawn_blocking(move || pick_android(mode)).await {
        Ok(paths) => paths,
        Err(e) => {
            tracing::warn!("file pick: the task did not finish: {e}");
            Vec::new()
        }
    }
}

// ------------------------------------------------------- Android file picking
//
// Why there is a dex in here at all. Choosing a file is
// `startActivityForResult`, and the answer arrives at `Activity`'s
// `onActivityResult`. The app's activity is `android.app.NativeActivity`, a
// framework class cargo-apk names in the manifest, so there is no subclass of
// ours to override that in; android-activity 0.6 surfaces no activity result
// either, and the Slint backend's Java helper is not an activity. The way
// through is a throwaway `android.app.Fragment`, which the framework hands the
// result to directly — see java/SigilFilePicker.java, which build.rs compiles
// and dexes into this library.

/// java/SigilFilePicker.java, compiled and dexed by build.rs.
#[cfg(target_os = "android")]
const PICKER_DEX: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/picker/classes.dex"));

/// `SigilFilePicker.WAITING`: a pick is open. Every other state ends the wait.
#[cfg(target_os = "android")]
const PICK_WAITING: i32 = 1;

/// How long to leave the picker open before giving up on it. Generous, because
/// the person may go hunting through a cloud provider; if they take longer than
/// this the pick is simply dropped, and the attach sheet has closed already.
#[cfg(target_os = "android")]
const PICK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

#[cfg(target_os = "android")]
const PICK_POLL: std::time::Duration = std::time::Duration::from_millis(200);

/// The loaded class, kept for the life of the process: loading a dex is not
/// free, and the class's statics are where the pick's state lives.
#[cfg(target_os = "android")]
static PICKER_CLASS: std::sync::OnceLock<jni::objects::GlobalRef> = std::sync::OnceLock::new();

/// How the Java side joins several paths into the one string it hands back.
#[cfg(target_os = "android")]
const PICK_SEP: char = '\u{1f}';

#[cfg(target_os = "android")]
fn pick_android(mode: &str) -> Vec<String> {
    match run_pick_android(mode) {
        Ok(paths) => paths,
        Err(e) => {
            tracing::warn!("file pick ({mode}): {e:#}");
            Vec::new()
        }
    }
}

/// A pending Java exception is fatal at detach, so every call clears it and
/// turns it into a plain Rust error.
#[cfg(target_os = "android")]
fn jni_check(env: &mut jni::JNIEnv, what: &str) -> anyhow::Result<()> {
    if env.exception_check().unwrap_or(false) {
        env.exception_describe().ok();
        env.exception_clear().ok();
        anyhow::bail!("{what} threw");
    }
    Ok(())
}

#[cfg(target_os = "android")]
fn jni_call<'l>(
    env: &mut jni::JNIEnv<'l>,
    obj: &jni::objects::JObject,
    name: &str,
    sig: &str,
    args: &[jni::objects::JValue],
) -> anyhow::Result<jni::objects::JValueOwned<'l>> {
    use anyhow::Context as _;
    let r = env.call_method(obj, name, sig, args);
    jni_check(env, name)?;
    r.with_context(|| name.to_string())
}

/// A static on `SigilFilePicker`. `&GlobalRef` is a class descriptor in its own
/// right, so the loaded class can be handed straight to the call.
#[cfg(target_os = "android")]
fn jni_call_static<'l>(
    env: &mut jni::JNIEnv<'l>,
    class: &'static jni::objects::GlobalRef,
    name: &str,
    sig: &str,
    args: &[jni::objects::JValue],
) -> anyhow::Result<jni::objects::JValueOwned<'l>> {
    use anyhow::Context as _;
    let r = env.call_static_method(class, name, sig, args);
    jni_check(env, name)?;
    r.with_context(|| format!("SigilFilePicker.{name}"))
}

/// Load `SigilFilePicker` out of the embedded dex, once.
///
/// It cannot come from the APK — cargo-apk packages no Java of ours — so we
/// build the loader ourselves. `InMemoryDexClassLoader` arrived in API 26,
/// which is the app's `min_sdk_version`. The activity's own loader is its
/// parent, so the framework classes the fragment names resolve through it.
#[cfg(target_os = "android")]
fn picker_class(
    env: &mut jni::JNIEnv,
    activity: &jni::objects::JObject,
) -> anyhow::Result<&'static jni::objects::GlobalRef> {
    use jni::objects::JValue;

    if let Some(class) = PICKER_CLASS.get() {
        return Ok(class);
    }

    let parent = jni_call(env, activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?.l()?;
    anyhow::ensure!(!parent.is_null(), "the activity has no class loader");

    // SAFETY: new_direct_byte_buffer lends the memory to the JVM.
    // InMemoryDexClassLoader reads the dex and never writes to it, and
    // PICKER_DEX is 'static, so the bytes outlive every use of the buffer.
    let dex = unsafe {
        env.new_direct_byte_buffer(PICKER_DEX.as_ptr().cast_mut(), PICKER_DEX.len())
    }?;

    let loader = env.new_object(
        "dalvik/system/InMemoryDexClassLoader",
        "(Ljava/nio/ByteBuffer;Ljava/lang/ClassLoader;)V",
        &[JValue::Object(&dex), JValue::Object(&parent)],
    );
    jni_check(env, "InMemoryDexClassLoader")?;
    let loader = loader?;

    let name = env.new_string("SigilFilePicker")?;
    let class = jni_call(
        env,
        &loader,
        "loadClass",
        "(Ljava/lang/String;)Ljava/lang/Class;",
        &[JValue::Object(&name)],
    )?
    .l()?;
    anyhow::ensure!(!class.is_null(), "SigilFilePicker is not in the embedded dex");

    let global = env.new_global_ref(&class)?;
    tracing::info!("file pick: SigilFilePicker is loaded");
    Ok(PICKER_CLASS.get_or_init(|| global))
}

/// Open the picker and wait for the answer.
///
/// The answer is polled rather than pushed, for the same reason the location
/// permission is in core/src/geo/android.rs: registering a Java callback would
/// mean handing Java an object of ours, and the Java side already has somewhere
/// to put the answer down.
#[cfg(target_os = "android")]
fn run_pick_android(mode: &str) -> anyhow::Result<Vec<String>> {
    use anyhow::Context as _;
    use jni::objects::{JObject, JString, JValue};

    let app = crate::scale::android().ok_or_else(|| anyhow::anyhow!("no Android app handle"))?;
    // SAFETY: android-activity's own pointers, valid for the life of the process.
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }?;
    let mut guard = vm.attach_current_thread().context("attach to the JVM")?;
    let env = &mut *guard;
    // The Activity, not the application context ndk-context carries: only an
    // Activity can start something for a result, and only an Activity has the
    // FragmentManager the answer is routed through.
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };

    // Somewhere of ours for the copy to land. The cache directory, so Android
    // can reclaim it: the engine has taken its own copy by upload time.
    let dir = sigil_engine::paths::cache_dir().join("picked");
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;

    let class = picker_class(env, &activity)?;

    let dir_arg = env.new_string(dir.to_string_lossy().as_ref())?;
    let mode_arg = env.new_string(mode)?;
    jni_call_static(
        env,
        class,
        "start",
        "(Landroid/app/Activity;Ljava/lang/String;Ljava/lang/String;)V",
        &[
            JValue::Object(&activity),
            JValue::Object(&dir_arg),
            JValue::Object(&mode_arg),
        ],
    )?;
    tracing::info!("file pick: asked for {mode}");

    let deadline = std::time::Instant::now() + PICK_TIMEOUT;
    loop {
        std::thread::sleep(PICK_POLL);

        // A frame per pass: the loop would otherwise pile up local references
        // for as long as the picker stays open.
        let state = env.with_local_frame(8, |env| -> anyhow::Result<i32> {
            Ok(jni_call_static(env, class, "state", "()I", &[])?.i()?)
        })?;

        if state != PICK_WAITING {
            let joined = env.with_local_frame(8, |env| -> anyhow::Result<String> {
                let s = jni_call_static(env, class, "paths", "()Ljava/lang/String;", &[])?.l()?;
                if s.is_null() {
                    return Ok(String::new());
                }
                Ok(env.get_string(&JString::from(s))?.into())
            })?;
            let paths: Vec<String> = joined
                .split(PICK_SEP)
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .collect();
            if paths.is_empty() {
                tracing::info!("file pick: nothing was chosen");
                return Ok(Vec::new());
            }
            tracing::info!("file pick: {} file(s): {}", paths.len(), paths.join(", "));
            return Ok(paths);
        }

        if std::time::Instant::now() >= deadline {
            tracing::warn!(
                "file pick: still open after {} s; giving up on it",
                PICK_TIMEOUT.as_secs()
            );
            return Ok(Vec::new());
        }
    }
}
