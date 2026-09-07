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

#[cfg_attr(target_arch = "wasm32", wasm_bindgen::prelude::wasm_bindgen)]
pub fn analyze(input: &str) -> String {
    composer::analyze(input)
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
