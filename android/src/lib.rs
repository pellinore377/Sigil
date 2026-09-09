#![deny(unsafe_op_in_unsafe_fn)]
//! Narrow JVM entrypoints for storage and native media transport.
use jni::{
    objects::{JByteArray, JObject, JString},
    sys::{jboolean, jbyteArray, jint, jstring, JNI_FALSE, JNI_TRUE},
    JNIEnv,
};
use sigil_client::ClientStore;
use sigil_crypto::{storage::StorageKey, Secret32};
use zeroize::Zeroizing;
mod calls;

#[no_mangle]
pub extern "system" fn Java_org_sigil_storage_NativeStorage_setWallpaper(
    mut env: JNIEnv,
    _: JObject,
    directory: JString,
    key: JByteArray,
    peer: JString,
    data: JByteArray,
) -> jboolean {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Option<()> {
        if env.get_array_length(&data).ok()? > 2 * 1024 * 1024 {
            return None;
        }
        let peer = String::from(env.get_string(&peer).ok()?);
        if peer.len() > 70 {
            return None;
        }
        let bytes = Zeroizing::new(env.convert_byte_array(&data).ok()?);
        open(&mut env, &directory, &key)?
            .mobile_set_wallpaper(&peer, &bytes)
            .ok()
    }));
    if result.ok().flatten().is_some() {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

#[no_mangle]
pub extern "system" fn Java_org_sigil_storage_NativeStorage_wallpaper(
    mut env: JNIEnv,
    _: JObject,
    directory: JString,
    key: JByteArray,
    peer: JString,
) -> jbyteArray {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> Option<Zeroizing<Vec<u8>>> {
            let peer = String::from(env.get_string(&peer).ok()?);
            if peer.len() > 70 {
                return None;
            }
            open(&mut env, &directory, &key)?
                .mobile_wallpaper(&peer)
                .ok()
                .flatten()
        },
    ));
    result
        .ok()
        .flatten()
        .and_then(|bytes| env.byte_array_from_slice(&bytes).ok())
        .map(|v| v.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

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

#[no_mangle]
pub extern "system" fn Java_org_sigil_storage_NativeStorage_stageFile(
    mut env: JNIEnv,
    _: JObject,
    directory: JString,
    key: JByteArray,
    request: JString,
    index: jint,
    data: JByteArray,
) -> jboolean {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Option<()> {
        if index < 0 || env.get_array_length(&data).ok()? > 1024 * 1024 {
            return None;
        }
        let request = String::from(env.get_string(&request).ok()?);
        if request.len() != 64 {
            return None;
        }
        let mut store = open(&mut env, &directory, &key)?;
        let bytes = Zeroizing::new(env.convert_byte_array(&data).ok()?);
        store.mobile_file_stage(&request, index as u32, &bytes).ok()
    }));
    if result.ok().flatten().is_some() {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

#[no_mangle]
pub extern "system" fn Java_org_sigil_storage_NativeStorage_readFileChunk(
    mut env: JNIEnv,
    _: JObject,
    directory: JString,
    key: JByteArray,
    peer: JString,
    author: JString,
    message: JString,
    index: jint,
) -> jbyteArray {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> Option<Zeroizing<Vec<u8>>> {
            if index < 0 {
                return None;
            }
            let peer = String::from(env.get_string(&peer).ok()?);
            let author = String::from(env.get_string(&author).ok()?);
            let message = String::from(env.get_string(&message).ok()?);
            if peer.len() > 70 || author.len() != 64 || message.len() != 64 {
                return None;
            }
            open(&mut env, &directory, &key)?
                .mobile_file_chunk(&peer, &author, &message, index as u32)
                .ok()
        },
    ));
    result
        .ok()
        .flatten()
        .and_then(|bytes| env.byte_array_from_slice(&bytes).ok())
        .map(|v| v.into_raw())
        .unwrap_or(std::ptr::null_mut())
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
pub extern "system" fn Java_org_sigil_storage_NativeStorage_mapResource(
    mut env: JNIEnv,
    _: JObject,
    directory: JString,
    key: JByteArray,
    path: JString,
) -> jbyteArray {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> Option<Zeroizing<Vec<u8>>> {
            let path = String::from(env.get_string(&path).ok()?);
            open(&mut env, &directory, &key)?
                .mobile_map_resource(&path)
                .ok()
        },
    ));
    result
        .ok()
        .flatten()
        .and_then(|bytes| env.byte_array_from_slice(&bytes).ok())
        .map(|v| v.into_raw())
        .unwrap_or(std::ptr::null_mut())
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
