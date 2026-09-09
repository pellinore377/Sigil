#![deny(unsafe_op_in_unsafe_fn)]
//! Narrow JVM entrypoints; handles originate from the JVM and never escape.
use jni::{
    objects::{JByteArray, JObject, JString},
    sys::{jboolean, jstring, JNI_FALSE, JNI_TRUE},
    JNIEnv,
};
use sigil_client::ClientStore;
use sigil_crypto::{storage::StorageKey, Secret32};
use zeroize::Zeroizing;

fn check(env: &mut JNIEnv<'_>, directory: &JString<'_>, key: &JByteArray<'_>) -> Option<()> {
    let mut store = open(env, directory, key)?;
    let identity = store.identity().ok()?;
    drop(store);
    let mut store = open(env, directory, key)?;
    if store.identity().ok()? != identity {
        return None;
    }
    Some(())
}
fn open(
    env: &mut JNIEnv<'_>,
    directory: &JString<'_>,
    key: &JByteArray<'_>,
) -> Option<ClientStore> {
    if env.get_array_length(key).ok()? != 32 {
        return None;
    }
    let bytes = Zeroizing::new(env.convert_byte_array(key).ok()?);
    let directory = String::from(env.get_string(directory).ok()?);
    if directory.len() > 4096 || !std::path::Path::new(&directory).is_absolute() {
        return None;
    }
    let path = std::path::Path::new(&directory).join("client.db");
    let storage = StorageKey::new(Secret32::from_bytes(bytes.as_slice().try_into().ok()?)).ok()?;
    ClientStore::open(&path, storage).ok()
}

#[no_mangle]
pub extern "system" fn Java_org_sigil_storage_NativeStorage_execute(
    mut env: JNIEnv,
    _: JObject,
    directory: JString,
    key: JByteArray,
    request: JString,
) -> jstring {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Option<String> {
        let mut store = open(&mut env, &directory, &key)?;
        let request = Zeroizing::new(String::from(env.get_string(&request).ok()?));
        Some(store.mobile_command(&request))
    }));
    let response = result.ok().flatten().unwrap_or_else(|| {
        r#"{"ok":false,"error":"Cannot open native storage. Stored keys have not been reset."}"#
            .into()
    });
    env.new_string(response)
        .map(|v| v.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
pub extern "system" fn Java_org_sigil_storage_NativeStorage_checkStore(
    mut env: JNIEnv,
    _: JObject,
    directory: JString,
    key: JByteArray,
) -> jboolean {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        check(&mut env, &directory, &key)
    })) {
        Ok(Some(())) => JNI_TRUE,
        _ => JNI_FALSE,
    }
}
