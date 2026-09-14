use super::*;

#[no_mangle]
pub extern "system" fn Java_org_sigil_storage_NativeStorage_scanLinkQr(
    env: JNIEnv,
    _: JObject,
    width: jint,
    height: jint,
    bytes: JByteArray,
) -> jstring {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Option<String> {
        if !(64..=1280).contains(&width)
            || !(64..=1280).contains(&height)
            || env.get_array_length(&bytes).ok()? != width * height
        {
            return None;
        }
        let pixels = Zeroizing::new(env.convert_byte_array(&bytes).ok()?);
        sigil_client::link::scan_frame(width as usize, height as usize, &pixels)
    }));
    result
        .ok()
        .flatten()
        .and_then(|text| env.new_string(text).ok())
        .map(|text| text.into_raw())
        .unwrap_or(std::ptr::null_mut())
}
