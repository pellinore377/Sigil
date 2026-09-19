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
mod preview;
mod qr;

#[no_mangle]
pub extern "system" fn Java_org_sigil_storage_NativeStorage_stageProfilePhoto(
    mut env: JNIEnv,
    _: JObject,
    directory: JString,
    key: JByteArray,
    data: JByteArray,
) -> jboolean {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Option<()> {
        if env.get_array_length(&data).ok()? > 128 * 1024 {
            return None;
        }
        let bytes = Zeroizing::new(env.convert_byte_array(&data).ok()?);
        open(&mut env, &directory, &key)?
            .mobile_stage_photo(&bytes)
            .ok()
    }));
    if result.ok().flatten().is_some() {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

#[no_mangle]
pub extern "system" fn Java_org_sigil_storage_NativeStorage_profilePhoto(
    mut env: JNIEnv,
    _: JObject,
    directory: JString,
    key: JByteArray,
    reference: JString,
) -> jbyteArray {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> Option<Zeroizing<Vec<u8>>> {
            let reference = String::from(env.get_string(&reference).ok()?);
            if reference.len() != 64 {
                return None;
            }
            open(&mut env, &directory, &key)?
                .mobile_profile_image(&reference)
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
pub extern "system" fn Java_org_sigil_storage_NativeStorage_readDraftChunk(
    mut env: JNIEnv,
    _: JObject,
    directory: JString,
    key: JByteArray,
    request: JString,
    index: jint,
) -> jbyteArray {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> Option<Zeroizing<Vec<u8>>> {
            if index < 0 {
                return None;
            }
            let request = String::from(env.get_string(&request).ok()?);
            if request.len() != 64 {
                return None;
            }
            open(&mut env, &directory, &key)?
                .mobile_file_draft_chunk(&request, index as u32)
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
/// Open stores are pooled while the app is in use, with the storage key held in native memory, so a command
/// neither unwraps the key nor opens the database again. `close_pool` drops both when the app leaves the foreground.
const POOL_LIMIT: usize = 4;
struct Pooled {
    directory: String,
    key: Zeroizing<[u8; 32]>,
    store: ClientStore,
}
struct Pool {
    idle: Vec<Pooled>,
    known: Option<(String, Zeroizing<[u8; 32]>)>,
}
static POOL: std::sync::Mutex<Pool> = std::sync::Mutex::new(Pool { idle: Vec::new(), known: None });
/// A store on loan from the pool; it goes back when dropped.
struct Store(Option<Pooled>);
impl std::ops::Deref for Store {
    type Target = ClientStore;
    fn deref(&self) -> &ClientStore { &self.0.as_ref().expect("store").store }
}
impl std::ops::DerefMut for Store {
    fn deref_mut(&mut self) -> &mut ClientStore { &mut self.0.as_mut().expect("store").store }
}
impl Store {
    /// Takes the store out of the pool's care, for a caller that keeps it.
    fn into_inner(mut self) -> ClientStore { self.0.take().expect("store").store }
}
impl Drop for Store {
    fn drop(&mut self) {
        if let Some(pooled) = self.0.take() {
            if let Ok(mut pool) = POOL.lock() {
                let current = pool.known.as_ref().is_some_and(|(d, k)| *d == pooled.directory && **k == *pooled.key);
                if current && pool.idle.len() < POOL_LIMIT { pool.idle.push(pooled); }
            }
        }
    }
}
fn valid_directory(directory: &str) -> bool { directory.len() <= 4096 && std::path::Path::new(directory).is_absolute() }
fn borrow(directory: String, key: Zeroizing<[u8; 32]>) -> Option<Store> {
    {
        let mut pool = POOL.lock().ok()?;
        let same = pool.known.as_ref().is_some_and(|(d, k)| *d == directory && **k == *key);
        if !same { pool.idle.clear(); pool.known = Some((directory.clone(), key.clone())); }
        if let Some(at) = pool.idle.iter().position(|p| p.directory == directory && *p.key == *key) {
            return Some(Store(Some(pool.idle.swap_remove(at))));
        }
    }
    let path = std::path::Path::new(&directory).join("client.db");
    let storage = StorageKey::new(Secret32::from_bytes(*key)).ok()?;
    let store = ClientStore::open(&path, storage).ok()?;
    Some(Store(Some(Pooled { directory, key, store })))
}
fn open(
    env: &mut JNIEnv<'_>,
    directory: &JString<'_>,
    key: &JByteArray<'_>,
) -> Option<Store> {
    if env.get_array_length(key).ok()? != 32 {
        return None;
    }
    let bytes = Zeroizing::new(env.convert_byte_array(key).ok()?);
    let key = Zeroizing::new(<[u8; 32]>::try_from(bytes.as_slice()).ok()?);
    let directory = String::from(env.get_string(directory).ok()?);
    if !valid_directory(&directory) {
        return None;
    }
    borrow(directory, key)
}
/// A store for a directory whose key is already known to the pool; none when the app has to unwrap the key first.
fn open_known(env: &mut JNIEnv<'_>, directory: &JString<'_>) -> Option<Store> {
    let directory = String::from(env.get_string(directory).ok()?);
    if !valid_directory(&directory) {
        return None;
    }
    let key = {
        let pool = POOL.lock().ok()?;
        match &pool.known { Some((d, k)) if *d == directory => k.clone(), _ => return None }
    };
    borrow(directory, key)
}
fn close_pool() {
    if let Ok(mut pool) = POOL.lock() { pool.idle.clear(); pool.known = None; }
}

#[no_mangle]
pub extern "system" fn Java_org_sigil_storage_NativeStorage_closeStore(_env: JNIEnv, _: JObject) {
    close_pool();
}

/// The keyless fast path: runs a command on a pooled store, or returns null when the key is not yet known.
#[no_mangle]
pub extern "system" fn Java_org_sigil_storage_NativeStorage_executeCached(
    mut env: JNIEnv,
    _: JObject,
    directory: JString,
    request: JString,
) -> jstring {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Option<String> {
        let mut store = open_known(&mut env, &directory)?;
        let request = Zeroizing::new(String::from(env.get_string(&request).ok()?));
        let ran = std::time::Instant::now();
        let response = store.mobile_command(&request);
        let run_ms = ran.elapsed().as_millis();
        if let Ok(tag) = std::ffi::CString::new("SigilTiming") {
            let marks = sigil_client::perf::drain().join(" ");
            if let Ok(line) = std::ffi::CString::new(format!("native pooled run={run_ms}ms {marks}")) {
                unsafe { __android_log_write(4, tag.as_ptr(), line.as_ptr()); }
            }
        }
        Some(response)
    }));
    match result.ok().flatten() {
        Some(response) => env.new_string(response).map(|v| v.into_raw()).unwrap_or(std::ptr::null_mut()),
        None => std::ptr::null_mut(),
    }
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

extern "C" { fn __android_log_write(prio: i32, tag: *const std::os::raw::c_char, text: *const std::os::raw::c_char) -> i32; }

#[no_mangle]
pub extern "system" fn Java_org_sigil_storage_NativeStorage_execute(
    mut env: JNIEnv,
    _: JObject,
    directory: JString,
    key: JByteArray,
    request: JString,
) -> jstring {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Option<String> {
        let opened = std::time::Instant::now();
        let mut store = open(&mut env, &directory, &key)?;
        let open_ms = opened.elapsed().as_millis();
        let request = Zeroizing::new(String::from(env.get_string(&request).ok()?));
        let ran = std::time::Instant::now();
        let response = store.mobile_command(&request);
        let run_ms = ran.elapsed().as_millis();
        if let Ok(tag) = std::ffi::CString::new("SigilTiming") {
            let marks = sigil_client::perf::drain().join(" ");
            if let Ok(line) = std::ffi::CString::new(format!("native open={open_ms}ms run={run_ms}ms {marks}")) {
                unsafe { __android_log_write(4, tag.as_ptr(), line.as_ptr()); }
            }
        }
        Some(response)
    }));
    let response = result.ok().flatten().unwrap_or_else(|| {
        r#"{"ok":false,"error":"Cannot open native storage. Stored keys have not been reset."}"#
            .into()
    });
    env.new_string(response)
        .map(|v| v.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

/// Holds a mailbox wait off the UI thread; the store is closed before waiting.
#[no_mangle]
pub extern "system" fn Java_org_sigil_storage_NativeStorage_mailboxWait(
    mut env: JNIEnv,
    _: JObject,
    directory: JString,
    key: JByteArray,
    seconds: jint,
) -> jboolean {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Option<bool> {
        let (client, after) = open(&mut env, &directory, &key)?.mailbox_watch().ok()?;
        client.mailbox_wait(after, seconds.clamp(1, 25) as u64).ok()
    }));
    if result.ok().flatten() == Some(true) { JNI_TRUE } else { JNI_FALSE }
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
