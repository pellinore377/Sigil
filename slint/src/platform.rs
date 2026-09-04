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
#[cfg(target_os = "android")]
const CAMERA: &str = "android.permission.CAMERA";
/// The requestPermissions code; the result comes back as a lifecycle event we
/// do not consume — the person taps record again once granted.
#[cfg(target_os = "android")]
const MIC_REQUEST_CODE: i32 = 7001;
/// The camera page's own code. Same story: nothing consumes the result, and
/// the page's poll notices the grant on its next pass and opens the camera.
#[cfg(target_os = "android")]
const CAMERA_REQUEST_CODE: i32 = 7002;

#[cfg(target_os = "android")]
fn mic_permission_android() -> anyhow::Result<bool> {
    permission_android(RECORD_AUDIO)
}

#[cfg(target_os = "android")]
fn request_mic_permission_android() -> anyhow::Result<()> {
    request_permissions_android(&[RECORD_AUDIO], MIC_REQUEST_CODE)
}

/// The camera is a runtime grant of the same shape as the microphone, and the
/// page that wants it also records video — so the two are asked for together
/// and the person sees one pair of dialogs rather than one now and one at the
/// moment they press record.
#[cfg(target_os = "android")]
pub fn has_camera_permission() -> bool {
    match permission_android(CAMERA) {
        Ok(granted) => granted,
        Err(e) => {
            tracing::warn!("camera permission check: {e:#}");
            false
        }
    }
}

#[cfg(target_os = "android")]
pub fn request_camera_permission() {
    if let Err(e) = request_permissions_android(&[CAMERA, RECORD_AUDIO], CAMERA_REQUEST_CODE) {
        tracing::warn!("camera permission request: {e:#}");
    }
}

/// checkSelfPermission(name) == PERMISSION_GRANTED (0).
#[cfg(target_os = "android")]
fn permission_android(name: &str) -> anyhow::Result<bool> {
    use jni::objects::{JObject, JValue};
    let app = crate::scale::android().ok_or_else(|| anyhow::anyhow!("no Android app handle"))?;
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }?;
    let mut env = vm.attach_current_thread()?;
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };

    let perm = env.new_string(name)?;
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

/// requestPermissions(new String[]{…}, code) — shows the dialog(s).
#[cfg(target_os = "android")]
fn request_permissions_android(names: &[&str], code: i32) -> anyhow::Result<()> {
    use jni::objects::{JObject, JValue};
    let app = crate::scale::android().ok_or_else(|| anyhow::anyhow!("no Android app handle"))?;
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }?;
    let mut env = vm.attach_current_thread()?;
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };

    let empty = env.new_string("")?;
    let perms = env.new_object_array(names.len() as i32, "java/lang/String", &empty)?;
    for (i, name) in names.iter().enumerate() {
        let s = env.new_string(name)?;
        env.set_object_array_element(&perms, i as i32, &s)?;
    }
    let result = env.call_method(
        &activity,
        "requestPermissions",
        "([Ljava/lang/String;I)V",
        &[JValue::Object(perms.as_ref()), JValue::Int(code)],
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
/// The attach sheet's camera page opens a viewfinder of our own (see the
/// camera section at the foot of this file); while that is up, the shutter is
/// this call, and it drives that session. With no viewfinder up — the tile
/// tapped on a build or a device where the page could not open one — Android
/// hands the job to whatever camera app the phone has, as it always did.
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
        if camera_live() {
            return tokio::task::spawn_blocking(move || capture_in_app(video))
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!("capture: the task did not finish: {e}");
                    None
                });
        }
        android_pick(if video { "video" } else { "photo" })
            .await
            .into_iter()
            .next()
    }
}

/// The shutter, when the page's own viewfinder is what is on screen.
///
/// A still is one call and a wait for the file. A clip is two presses of the
/// same button: the first starts it and stays here holding the wait, the
/// second is a second call that finds a recording already running, stops it,
/// and answers with nothing — the file goes back through the call that started
/// it, so only one of them stages anything.
#[cfg(target_os = "android")]
fn capture_in_app(video: bool) -> Option<String> {
    use std::time::{Duration, Instant, SystemTime};

    let dir = sigil_engine::paths::cache_dir().join("picked");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!("capture: create {}: {e}", dir.display());
        return None;
    }
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let poll = Duration::from_millis(100);

    if video {
        if camera_state().map(|s| s.recording).unwrap_or(false) {
            camera_stop_video();
            return None;
        }
        let path = dir.join(format!("clip-{stamp}.mp4"));
        let path = path.to_string_lossy().to_string();
        camera_start_video(&path);
        // Rolling within a few seconds, or the session never configured.
        let start = Instant::now();
        loop {
            std::thread::sleep(poll);
            match camera_state() {
                Some(s) if s.recording => break,
                Some(s) if s.state == "error" => return None,
                None => return None,
                _ => {}
            }
            if start.elapsed() > Duration::from_secs(5) {
                tracing::warn!("capture: recording never started");
                return None;
            }
        }
        // A clip runs until the shutter is pressed again, the page is left, or
        // the phone decides otherwise; ten minutes is the outer bound.
        let start = Instant::now();
        loop {
            std::thread::sleep(poll);
            let Some(s) = camera_state() else { return None };
            if !s.recording {
                return (s.path == path && std::fs::metadata(&path).is_ok()).then_some(path);
            }
            if start.elapsed() > Duration::from_secs(600) {
                camera_stop_video();
            }
        }
    }

    let path = dir.join(format!("photo-{stamp}.jpg"));
    let path = path.to_string_lossy().to_string();
    camera_capture(&path);
    let start = Instant::now();
    loop {
        std::thread::sleep(poll);
        let Some(s) = camera_state() else { return None };
        if s.state == "error" {
            tracing::warn!("capture: {}", s.failure.unwrap_or_default());
            return None;
        }
        if s.path == path {
            return Some(path);
        }
        if start.elapsed() > Duration::from_secs(15) {
            tracing::warn!("capture: no photo after 15 s");
            return None;
        }
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
    if let Some(class) = PICKER_CLASS.get() {
        return Ok(class);
    }
    let global = dex_class(env, activity, "SigilFilePicker")?;
    tracing::info!("file pick: SigilFilePicker is loaded");
    Ok(PICKER_CLASS.get_or_init(|| global))
}

/// The one loader over the embedded dex, made on first use and kept.
#[cfg(target_os = "android")]
static DEX_LOADER: std::sync::OnceLock<jni::objects::GlobalRef> = std::sync::OnceLock::new();

/// A class out of the embedded dex, by name. The loader is built once, over
/// the activity's own so the framework classes resolve through it.
#[cfg(target_os = "android")]
fn dex_class(
    env: &mut jni::JNIEnv,
    activity: &jni::objects::JObject,
    name: &str,
) -> anyhow::Result<jni::objects::GlobalRef> {
    use jni::objects::JValue;

    let loader = match DEX_LOADER.get() {
        Some(l) => l.clone(),
        None => {
            let parent =
                jni_call(env, activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?.l()?;
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
            let global = env.new_global_ref(&loader?)?;
            DEX_LOADER.get_or_init(|| global).clone()
        }
    };
    let jname = env.new_string(name)?;
    let class = jni_call(
        env,
        loader.as_obj(),
        "loadClass",
        "(Ljava/lang/String;)Ljava/lang/Class;",
        &[JValue::Object(&jname)],
    )?
    .l()?;
    anyhow::ensure!(!class.is_null(), "{name} is not in the embedded dex");
    Ok(env.new_global_ref(&class)?)
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

// ---------------------------------------------------------------------------
// Video on the phone: the platform's own view, laid over the app's surface
// (java/SigilVideo.java). The app has no decoder for video on Android and
// draws through one surface, so a playing clip is that view placed exactly
// where the viewer would have drawn the picture, and taken away with it.
// ---------------------------------------------------------------------------

/// Where the phone's player stands: milliseconds in, milliseconds long,
/// whether it is running, whether it has run out, and any failure.
#[derive(Clone, Debug, Default)]
pub struct VideoState {
    pub position_ms: i32,
    pub duration_ms: i32,
    pub playing: bool,
    pub ended: bool,
    pub failure: Option<String>,
}

/// Lay the phone's player over the app at a rectangle in physical pixels
/// and start `path`. Nothing happens off Android.
pub fn video_show(path: &str, x: i32, y: i32, w: i32, h: i32) -> bool {
    #[cfg(target_os = "android")]
    {
        match video_call(|env, class, activity| {
            use jni::objects::JValue;
            let jpath = env.new_string(path)?;
            jni_call_static(
                env,
                class,
                "show",
                "(Landroid/app/Activity;Ljava/lang/String;IIII)V",
                &[
                    JValue::Object(activity),
                    JValue::Object(&jpath),
                    JValue::Int(x),
                    JValue::Int(y),
                    JValue::Int(w),
                    JValue::Int(h),
                ],
            )?;
            Ok(())
        }) {
            Ok(()) => return true,
            Err(e) => {
                tracing::warn!("video: the phone's player did not open: {e:#}");
                return false;
            }
        }
    }
    #[allow(unreachable_code)]
    {
        let _ = (path, x, y, w, h);
        false
    }
}

/// The picture moved or resized under the player: follow it.
pub fn video_move(x: i32, y: i32, w: i32, h: i32) {
    #[cfg(target_os = "android")]
    {
        let _ = video_call(|env, class, _| {
            use jni::objects::JValue;
            jni_call_static(
                env,
                class,
                "move",
                "(IIII)V",
                &[JValue::Int(x), JValue::Int(y), JValue::Int(w), JValue::Int(h)],
            )?;
            Ok(())
        });
    }
    #[cfg(not(target_os = "android"))]
    let _ = (x, y, w, h);
}

pub fn video_pause() {
    #[cfg(target_os = "android")]
    let _ = video_call(|env, class, _| {
        jni_call_static(env, class, "pause", "()V", &[])?;
        Ok(())
    });
}

pub fn video_resume() {
    #[cfg(target_os = "android")]
    let _ = video_call(|env, class, _| {
        jni_call_static(env, class, "resume", "()V", &[])?;
        Ok(())
    });
}

pub fn video_seek(ms: i32) {
    #[cfg(target_os = "android")]
    let _ = video_call(|env, class, _| {
        jni_call_static(env, class, "seekTo", "(I)V", &[jni::objects::JValue::Int(ms)])?;
        Ok(())
    });
    #[cfg(not(target_os = "android"))]
    let _ = ms;
}

/// Take the player away.
pub fn video_hide() {
    #[cfg(target_os = "android")]
    let _ = video_call(|env, class, _| {
        jni_call_static(env, class, "hide", "()V", &[])?;
        Ok(())
    });
}

/// Where the player is; `None` off Android or when it cannot be asked.
pub fn video_state() -> Option<VideoState> {
    #[cfg(target_os = "android")]
    {
        return video_call(|env, class, _| {
            let position_ms = jni_call_static(env, class, "position", "()I", &[])?.i()?;
            let duration_ms = jni_call_static(env, class, "duration", "()I", &[])?.i()?;
            let playing = jni_call_static(env, class, "isPlaying", "()Z", &[])?.z()?;
            let ended = jni_call_static(env, class, "hasEnded", "()Z", &[])?.z()?;
            let failure = jni_call_static(env, class, "failure", "()Ljava/lang/String;", &[])?.l()?;
            let failure = if failure.is_null() {
                None
            } else {
                Some(env.get_string(&jni::objects::JString::from(failure))?.to_string_lossy().to_string())
            };
            Ok(VideoState { position_ms, duration_ms, playing, ended, failure })
        })
        .ok();
    }
    #[allow(unreachable_code)]
    None
}

#[cfg(target_os = "android")]
static VIDEO_CLASS: std::sync::OnceLock<jni::objects::GlobalRef> = std::sync::OnceLock::new();

/// Attach, find the class once, and run `f` with it and the Activity.
#[cfg(target_os = "android")]
fn video_call<T>(
    f: impl FnOnce(
        &mut jni::JNIEnv,
        &'static jni::objects::GlobalRef,
        &jni::objects::JObject,
    ) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    use anyhow::Context as _;
    let app = crate::scale::android().ok_or_else(|| anyhow::anyhow!("no Android app handle"))?;
    // SAFETY: android-activity's own pointers, valid for the life of the process.
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }?;
    let mut guard = vm.attach_current_thread().context("attach to the JVM")?;
    let env = &mut *guard;
    let activity = unsafe { jni::objects::JObject::from_raw(app.activity_as_ptr().cast()) };
    let class = match VIDEO_CLASS.get() {
        Some(c) => c,
        None => {
            let global = dex_class(env, &activity, "SigilVideo")?;
            tracing::info!("video: SigilVideo is loaded");
            VIDEO_CLASS.get_or_init(|| global)
        }
    };
    f(env, &class, &activity)
}

// ---------------------------------------------------------------------------
// The camera on the phone: the same trick as video, the other way round.
// java/SigilCamera.java lays a SurfaceView over the app's own surface and runs
// a Camera2 preview on it, so the attach sheet's camera page can show a live
// viewfinder in a rectangle it hands down — the app draws through one surface
// and could never have drawn a camera frame itself.
//
// The page's controls are state, not commands: it says where the box is, which
// way the camera faces, what the zoom is and whether the torch is on, and the
// bridge's poll (actions.rs camera_pass) carries changes down. The shutter is the one
// command, and it comes through capture_media above, so a shot lands on the
// staging page by the route every other attachment takes.
// ---------------------------------------------------------------------------

/// Where the phone's camera stands. `state` is one of idle / opening / ready /
/// capturing / recording / error; `path` is the last file written.
#[derive(Clone, Debug, Default)]
pub struct CameraState {
    pub state: String,
    pub path: String,
    pub zoom_min: f32,
    pub zoom_max: f32,
    pub has_flash: bool,
    pub front: bool,
    pub recording: bool,
    pub failure: Option<String>,
}

/// Whether a viewfinder is up. Kept on this side rather than asked of Java:
/// `capture_media` has to decide between our camera and the phone's camera app
/// before it does anything, and that decision must not depend on a JNI attach.
static CAMERA_LIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn camera_live() -> bool {
    CAMERA_LIVE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Open a viewfinder over the app inside a rectangle in physical pixels.
/// `front` picks the selfie camera. Nothing happens off Android.
pub fn camera_open(x: i32, y: i32, w: i32, h: i32, front: bool) -> bool {
    #[cfg(target_os = "android")]
    {
        match camera_call(|env, class, activity| {
            use jni::objects::JValue;
            let facing = env.new_string(if front { "front" } else { "back" })?;
            jni_call_static(
                env,
                class,
                "open",
                "(Landroid/app/Activity;IIIILjava/lang/String;)V",
                &[
                    JValue::Object(activity),
                    JValue::Int(x),
                    JValue::Int(y),
                    JValue::Int(w),
                    JValue::Int(h),
                    JValue::Object(&facing),
                ],
            )?;
            Ok(())
        }) {
            Ok(()) => {
                CAMERA_LIVE.store(true, std::sync::atomic::Ordering::Relaxed);
                return true;
            }
            Err(e) => {
                tracing::warn!("camera: the viewfinder did not open: {e:#}");
                return false;
            }
        }
    }
    #[allow(unreachable_code)]
    {
        let _ = (x, y, w, h, front);
        false
    }
}

/// The page's preview box moved or resized: follow it.
pub fn camera_move(x: i32, y: i32, w: i32, h: i32) {
    #[cfg(target_os = "android")]
    {
        let _ = camera_call(|env, class, _| {
            use jni::objects::JValue;
            jni_call_static(
                env,
                class,
                "move",
                "(IIII)V",
                &[JValue::Int(x), JValue::Int(y), JValue::Int(w), JValue::Int(h)],
            )?;
            Ok(())
        });
    }
    #[cfg(not(target_os = "android"))]
    let _ = (x, y, w, h);
}

/// Swap the facing camera, keeping the view where it is.
pub fn camera_flip() {
    #[cfg(target_os = "android")]
    let _ = camera_call(|env, class, _| {
        jni_call_static(env, class, "flip", "()V", &[])?;
        Ok(())
    });
}

pub fn camera_zoom(ratio: f32) {
    #[cfg(target_os = "android")]
    let _ = camera_call(|env, class, _| {
        jni_call_static(
            env,
            class,
            "setZoom",
            "(F)V",
            &[jni::objects::JValue::Float(ratio)],
        )?;
        Ok(())
    });
    #[cfg(not(target_os = "android"))]
    let _ = ratio;
}

pub fn camera_torch(on: bool) {
    #[cfg(target_os = "android")]
    let _ = camera_call(|env, class, _| {
        jni_call_static(
            env,
            class,
            "torch",
            "(Z)V",
            &[jni::objects::JValue::Bool(on as u8)],
        )?;
        Ok(())
    });
    #[cfg(not(target_os = "android"))]
    let _ = on;
}

/// Take one still into `path`; the file is there once `camera_state().path`
/// is that name.
pub fn camera_capture(path: &str) {
    #[cfg(target_os = "android")]
    let _ = camera_call(|env, class, _| {
        let jpath = env.new_string(path)?;
        jni_call_static(
            env,
            class,
            "capture",
            "(Ljava/lang/String;)V",
            &[jni::objects::JValue::Object(&jpath)],
        )?;
        Ok(())
    });
    #[cfg(not(target_os = "android"))]
    let _ = path;
}

pub fn camera_start_video(path: &str) {
    #[cfg(target_os = "android")]
    let _ = camera_call(|env, class, _| {
        let jpath = env.new_string(path)?;
        jni_call_static(
            env,
            class,
            "startVideo",
            "(Ljava/lang/String;)V",
            &[jni::objects::JValue::Object(&jpath)],
        )?;
        Ok(())
    });
    #[cfg(not(target_os = "android"))]
    let _ = path;
}

pub fn camera_stop_video() {
    #[cfg(target_os = "android")]
    let _ = camera_call(|env, class, _| {
        jni_call_static(env, class, "stopVideo", "()V", &[])?;
        Ok(())
    });
}

/// Take the viewfinder away and give the sensor back.
pub fn camera_close() {
    CAMERA_LIVE.store(false, std::sync::atomic::Ordering::Relaxed);
    #[cfg(target_os = "android")]
    let _ = camera_call(|env, class, _| {
        jni_call_static(env, class, "close", "()V", &[])?;
        Ok(())
    });
}

/// Where the camera is; `None` off Android or when it cannot be asked.
pub fn camera_state() -> Option<CameraState> {
    #[cfg(target_os = "android")]
    {
        // A frame of its own: this is asked twenty times a second while the
        // page is up, and every answer of it makes a local reference.
        return camera_call(|env, class, _| {
            env.with_local_frame(8, |env| -> anyhow::Result<CameraState> {
            let state = jni_string(env, class, "state")?.unwrap_or_default();
            let path = jni_string(env, class, "lastPath")?.unwrap_or_default();
            let failure = jni_string(env, class, "failure")?;
            let zoom_min = jni_call_static(env, class, "zoomMin", "()F", &[])?.f()?;
            let zoom_max = jni_call_static(env, class, "zoomMax", "()F", &[])?.f()?;
            let has_flash = jni_call_static(env, class, "hasFlash", "()Z", &[])?.z()?;
            let front = jni_call_static(env, class, "isFront", "()Z", &[])?.z()?;
            Ok(CameraState {
                recording: state == "recording",
                state,
                path,
                zoom_min,
                zoom_max,
                has_flash,
                front,
                failure,
            })
            })
        })
        .ok();
    }
    #[allow(unreachable_code)]
    None
}

/// A no-argument static returning a String, as an `Option<String>` — `null`
/// (no failure yet, no file yet) is `None`.
#[cfg(target_os = "android")]
fn jni_string(
    env: &mut jni::JNIEnv,
    class: &'static jni::objects::GlobalRef,
    name: &str,
) -> anyhow::Result<Option<String>> {
    let obj = jni_call_static(env, class, name, "()Ljava/lang/String;", &[])?.l()?;
    if obj.is_null() {
        return Ok(None);
    }
    let s: String = env.get_string(&jni::objects::JString::from(obj))?.into();
    Ok((!s.is_empty()).then_some(s))
}

#[cfg(target_os = "android")]
static CAMERA_CLASS: std::sync::OnceLock<jni::objects::GlobalRef> = std::sync::OnceLock::new();

/// Attach, find the class once, and run `f` with it and the Activity.
#[cfg(target_os = "android")]
fn camera_call<T>(
    f: impl FnOnce(
        &mut jni::JNIEnv,
        &'static jni::objects::GlobalRef,
        &jni::objects::JObject,
    ) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    use anyhow::Context as _;
    let app = crate::scale::android().ok_or_else(|| anyhow::anyhow!("no Android app handle"))?;
    // SAFETY: android-activity's own pointers, valid for the life of the process.
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }?;
    let mut guard = vm.attach_current_thread().context("attach to the JVM")?;
    let env = &mut *guard;
    let activity = unsafe { jni::objects::JObject::from_raw(app.activity_as_ptr().cast()) };
    let class = match CAMERA_CLASS.get() {
        Some(c) => c,
        None => {
            let global = dex_class(env, &activity, "SigilCamera")?;
            tracing::info!("camera: SigilCamera is loaded");
            CAMERA_CLASS.get_or_init(|| global)
        }
    };
    f(env, class, &activity)
}
