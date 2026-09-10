#[cfg(not(target_arch = "wasm32"))]
use jni::{
    objects::{JObject, JString},
    sys::jstring,
    JNIEnv,
};

#[cfg_attr(target_arch = "wasm32", wasm_bindgen::prelude::wasm_bindgen)]
pub fn redact(input: &str) -> String {
    sigil_text::parse(input, Default::default())
        .map(|text| text.body().to_owned())
        .unwrap_or_else(|_| "Invalid SigilText; send blocked".into())
}

#[cfg(not(target_arch = "wasm32"))]
#[no_mangle]
pub extern "system" fn Java_org_sigil_NativeCore_redact(
    mut env: JNIEnv,
    _: JObject,
    input: JString,
) -> jstring {
    let result = match env.get_string(&input) {
        Ok(value) => redact(&String::from(value)),
        Err(_) => return std::ptr::null_mut(),
    };
    env.new_string(result)
        .map(JString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

#[cfg(not(target_arch = "wasm32"))]
#[no_mangle]
pub extern "system" fn Java_org_sigil_NativeCore_analyze(
    mut env: JNIEnv,
    _: JObject,
    input: JString,
) -> jstring {
    let result = match env.get_string(&input) {
        Ok(value) => analyze(&String::from(value)),
        Err(_) => return std::ptr::null_mut(),
    };
    env.new_string(result)
        .map(JString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

#[cfg(test)]
mod tests {
    use super::redact;
    #[test]
    fn strips_multiple_secrets_without_damaging_unicode() {
        assert_eq!(
            redact("👩🏽‍💻 redact::first; שלום redact::second;"),
            "👩🏽‍💻 [REDACTED] שלום [REDACTED]"
        );
    }
    #[test]
    fn line_scoped_redaction_and_invalid_input_use_the_shared_parser() {
        assert_eq!(redact("hello redact::secret"), "hello [REDACTED]");
        assert_eq!(
            redact(&"a".repeat(32769)),
            "Invalid SigilText; send blocked"
        );
        assert_eq!(redact("`redact::literal;`"), "redact::literal;");
    }
}
mod composer;
mod temporal;

#[cfg_attr(target_arch = "wasm32", wasm_bindgen::prelude::wasm_bindgen)]
pub fn temporal_preview(input: &str) -> String {
    temporal::preview(input)
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen::prelude::wasm_bindgen)]
pub fn help_catalog(input: &str) -> String {
    sigil_text::help::catalog(input)
}

#[cfg(not(target_arch = "wasm32"))]
#[no_mangle]
pub extern "system" fn Java_org_sigil_NativeCore_helpCatalog(
    mut env: JNIEnv,
    _: JObject,
    input: JString,
) -> jstring {
    let result = match env.get_string(&input) {
        Ok(value) => help_catalog(&String::from(value)),
        Err(_) => return std::ptr::null_mut(),
    };
    env.new_string(result)
        .map(JString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

#[cfg(not(target_arch = "wasm32"))]
#[no_mangle]
pub extern "system" fn Java_org_sigil_NativeCore_temporalPreview(
    mut env: JNIEnv,
    _: JObject,
    input: JString,
) -> jstring {
    let result = match env.get_string(&input) {
        Ok(value) => temporal_preview(&String::from(value)),
        Err(_) => return std::ptr::null_mut(),
    };
    env.new_string(result)
        .map(JString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen::prelude::wasm_bindgen)]
pub fn analyze(input: &str) -> String {
    composer::analyze(input)
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen::prelude::wasm_bindgen)]
pub fn editor(input: &str) -> String {
    composer::editor(input)
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen::prelude::wasm_bindgen)]
pub fn motion_seeds(input: &str) -> String {
    let Some((id, count)) = input.split_once('/') else {
        return String::new();
    };
    let Ok(count) = count.parse::<u32>() else {
        return String::new();
    };
    if id.len() != 64 || count == 0 || count > 192 || !id.is_ascii() {
        return String::new();
    }
    let mut message = [0; 32];
    for (i, byte) in message.iter_mut().enumerate() {
        let Ok(value) = u8::from_str_radix(&id[i * 2..i * 2 + 2], 16) else {
            return String::new();
        };
        *byte = value;
    }
    (0..count)
        .map(|unit| (sigil_text::motion::seed(&message, unit, 0) as u32).to_string())
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
#[test]
fn motion_seed_adapter_is_bounded_deterministic_and_message_scoped() {
    let first = format!("{}/192", "01".repeat(32));
    let result = motion_seeds(&first);
    assert_eq!(result.split(',').count(), 192);
    assert_eq!(result, motion_seeds(&first));
    assert_ne!(result, motion_seeds(&format!("{}/192", "02".repeat(32))));
    for invalid in [
        "".to_owned(),
        format!("{}/1", "é".repeat(32)),
        format!("{}/0", "01".repeat(32)),
        format!("{}/193", "01".repeat(32)),
        format!("{}/1", "zz".repeat(32)),
    ] {
        assert!(motion_seeds(&invalid).is_empty());
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[no_mangle]
pub extern "system" fn Java_org_sigil_NativeCore_motionSeeds(
    mut env: JNIEnv,
    _: JObject,
    input: JString,
) -> jstring {
    let result = match env.get_string(&input) {
        Ok(value) => motion_seeds(&String::from(value)),
        Err(_) => return std::ptr::null_mut(),
    };
    env.new_string(result)
        .map(JString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

#[cfg(not(target_arch = "wasm32"))]
#[no_mangle]
pub extern "system" fn Java_org_sigil_NativeCore_editor(
    mut env: JNIEnv,
    _: JObject,
    input: JString,
) -> jstring {
    let result = match env.get_string(&input) {
        Ok(value) => editor(&String::from(value)),
        Err(_) => return std::ptr::null_mut(),
    };
    env.new_string(result)
        .map(JString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

mod theme;

#[cfg_attr(target_arch = "wasm32", wasm_bindgen::prelude::wasm_bindgen)]
pub fn palette(accent: u32, dark: bool) -> String {
    theme::palette(accent, dark)
        .map(|c| format!("{c:06x}"))
        .join(",")
}

#[cfg(not(target_arch = "wasm32"))]
#[no_mangle]
pub extern "system" fn Java_org_sigil_NativeCore_palette(
    env: JNIEnv,
    _: JObject,
    accent: jni::sys::jint,
    dark: jni::sys::jboolean,
) -> jstring {
    env.new_string(palette(accent as u32, dark != 0))
        .map(JString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}
