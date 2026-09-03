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

/// Pick one file. Desktop: omarchy-file-select. Android: needs a SAF intent
/// round-trip NativeActivity cannot do without extra glue — returns None.
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
        tracing::warn!("file picking on Android needs the SAF seam; not wired yet");
        None
    }
}
