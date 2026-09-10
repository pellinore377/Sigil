use jni::{
    JNIEnv,
    objects::{JObject, JString},
    sys::{JNI_FALSE, JNI_TRUE, jboolean, jint},
};
use sigil_media::{Request, decode, formats::Format};
use std::{fs::File, os::fd::FromRawFd};

fn duplicate(fd: jint) -> Option<File> {
    if fd < 0 {
        return None;
    }
    // dup validates the descriptor and gives this call independent ownership.
    let copy = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
    if copy < 0 {
        None
    } else {
        Some(unsafe { File::from_raw_fd(copy) })
    }
}

#[no_mangle]
pub extern "system" fn Java_org_sigil_compose_NativePreview_render(
    mut env: JNIEnv,
    _: JObject,
    input: jint,
    output: jint,
    format: JString,
    request: JString,
) -> jboolean {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Option<()> {
        let format: String = env.get_string(&format).ok()?.into();
        let request: String = env.get_string(&request).ok()?.into();
        if format.len() > 32 || request.len() > 1024 {
            return None;
        }
        let format: Format = serde_json::from_value(serde_json::Value::String(format)).ok()?;
        let request: Request = serde_json::from_str(&request).ok()?;
        let preview = decode::render_portable(duplicate(input)?, format, &request).ok()?;
        preview.write(duplicate(output)?).ok()
    }));
    if result.ok().flatten().is_some() {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}
