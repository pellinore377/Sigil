//! The per-platform seams, kept to two: where files live, and how a URL opens.
//! Everything else is the same code on every platform.

/// Open a URL in the platform browser. Used for the OIDC login page; the
/// engine finishes the flow on its localhost redirect, which the browser can
/// reach on-device on every platform we ship to.
#[cfg(not(target_os = "android"))]
pub fn open_url(url: &str) {
    if let Err(e) = std::process::Command::new("xdg-open").arg(url).spawn() {
        tracing::warn!("xdg-open: {e}");
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
    env.call_method(&activity, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)])?;
    Ok(())
}
