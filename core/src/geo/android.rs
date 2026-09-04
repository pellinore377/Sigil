//! Android's own position, through `android.location.LocationManager` over JNI.
//!
//! The platform framework API, not Google Play services: Play services is a separate
//! dependency that a de-Googled phone does not have, and the framework's fused provider
//! (API 31+) already merges GPS with the network estimate. Nothing here needs a key,
//! a network round trip, or a WiFi scan of our own.
//!
//! The shape of the answer is the ladder's: `Fix` with `Source::Platform`, or `None` so
//! `super::resolve` moves to the next rung. Every refusal — no permission, location
//! switched off, no provider — is a log line and a `None`, never an error the ladder
//! has to understand.
//!
//! Two things Android makes awkward, and how they are handled here:
//!
//! * **A fresh fix needs a Java callback.** `getCurrentLocation` and
//!   `requestSingleUpdate` both take a Java object (a `Consumer`, a `LocationListener`)
//!   that Rust cannot hand them without shipping a compiled class. The
//!   `requestLocationUpdates(String, long, float, PendingIntent)` overload takes no
//!   object at all: it wakes the provider, which is the part we need. We never receive
//!   the broadcast — we read `getLastKnownLocation` while the provider runs, and stop
//!   the updates as soon as something newer than the request arrives.
//! * **The permission dialog needs an Activity.** `checkSelfPermission` works on any
//!   Context, so the *check* runs off the application context android-activity parks in
//!   `ndk-context`. `requestPermissions` is a method on `Activity`, which only the
//!   frontend holds — it hands the pointer over once through `use_activity`.

use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU8, Ordering};
use std::time::Duration;

use anyhow::{bail, Context as _, Result};
use jni::objects::{JObject, JValue, JValueOwned};
use jni::JNIEnv;
use tracing::{debug, info, warn};

use super::{now_ms, valid_coords, Fix, Source, MAX_FIX_AGE};

const FINE: &str = "android.permission.ACCESS_FINE_LOCATION";
const COARSE: &str = "android.permission.ACCESS_COARSE_LOCATION";

/// `requestPermissions` wants a request code; the answer comes back to the Activity as a
/// lifecycle callback we do not consume — we poll `checkSelfPermission` instead.
const PERMISSION_REQUEST_CODE: i32 = 7002;

/// How long to hold the dialog open before giving up and letting the ladder continue.
/// The person may ignore it; the next refresh asks nothing and simply reads the grant.
const WAIT_FOR_ANSWER: Duration = Duration::from_secs(20);

/// How long to leave the provider running for a fix newer than our request. A warm GPS
/// or the network provider answers in a second or two; a cold GPS will not make it, and
/// the stale last-known answer plus the next refresh cover that.
const WAIT_FOR_FRESH: Duration = Duration::from_secs(12);

const POLL: Duration = Duration::from_millis(500);

/// `Context.LOCATION_SERVICE`.
const LOCATION_SERVICE: &str = "location";

/// `PendingIntent.FLAG_UPDATE_CURRENT | FLAG_MUTABLE`. Mutable because the system fills
/// the intent in with the location; immutable throws on API 31+.
const PENDING_FLAGS: i32 = 0x0800_0000 | 0x0200_0000;

/// Our own broadcast action. Nothing receives it — declaring a receiver would only add a
/// second path to the same Location the provider has already recorded.
const TICK_ACTION: &str = "com.sigil.geo.LOCATION_TICK";

// The Activity, handed over by the frontend that owns it (the Slint app does it in
// `android_main`). Both are unowned JNI global references that live as long as the
// process, so a raw pointer is the whole of the ownership story.
static VM: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static ACTIVITY: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());

/// Ask for the permission once per run: the ladder re-resolves every fifteen minutes and
/// a dialog on every pass would be a nuisance, not a prompt.
static ASKED: AtomicBool = AtomicBool::new(false);

/// Hand the engine the Activity and the VM. Called once, from the frontend's Android
/// entry point, before anything asks for a position.
///
/// # Safety
/// Both pointers must be the ones `android-activity` reports (`vm_as_ptr`,
/// `activity_as_ptr`): a JavaVM and an unowned global reference to the Activity, both
/// valid for the life of the process.
pub fn use_activity(vm: *mut c_void, activity: *mut c_void) {
    VM.store(vm, Ordering::Release);
    ACTIVITY.store(activity, Ordering::Release);
    debug!("geo: the Android Activity is available for the location permission");
}

// What the last attempt ran into, so `describe()` can say it in the UI's error line.
const STATE_UNKNOWN: u8 = 0;
const STATE_OK: u8 = 1;
const STATE_NO_PERMISSION: u8 = 2;
const STATE_SWITCHED_OFF: u8 = 3;
const STATE_NO_CONTEXT: u8 = 4;
const STATE_NOTHING_YET: u8 = 5;
static STATE: AtomicU8 = AtomicU8::new(STATE_UNKNOWN);

fn note(state: u8) {
    STATE.store(state, Ordering::Relaxed);
}

/// What this build can do and, once it has tried, what stopped it. Never empty.
pub fn describe() -> &'static str {
    match STATE.load(Ordering::Relaxed) {
        STATE_OK => "Android: LocationManager (GPS and network)",
        STATE_NO_PERMISSION => "Android: location permission not granted — Settings ▸ Apps ▸ Sigil ▸ Permissions ▸ Location",
        STATE_SWITCHED_OFF => "Android: location is switched off for the phone",
        STATE_NO_CONTEXT => "Android: the app context is not available to the engine",
        STATE_NOTHING_YET => "Android: LocationManager has no fix yet (indoors?)",
        _ => "Android: LocationManager (GPS and network)",
    }
}

/// The platform's answer. JNI is blocking and the providers take seconds, so the whole
/// conversation runs on a blocking thread: the UI thread never waits on it, and neither
/// does the runtime's worker pool.
pub async fn fix() -> Option<Fix> {
    match tokio::task::spawn_blocking(look).await {
        Ok(f) => f,
        Err(e) => {
            warn!("geo: the Android location task did not finish: {e}");
            None
        }
    }
}

fn look() -> Option<Fix> {
    match ask_the_phone() {
        Ok(f) => f,
        Err(e) => {
            warn!("geo: Android location: {e:#}");
            None
        }
    }
}

/// The JavaVM: the frontend's if it handed one over, otherwise the one android-activity
/// parked in ndk-context.
pub(crate) fn vm() -> Result<jni::JavaVM> {
    let stored = VM.load(Ordering::Acquire);
    let raw = if stored.is_null() { ndk_context::android_context().vm() } else { stored };
    if raw.is_null() {
        note(STATE_NO_CONTEXT);
        bail!("no JavaVM (is this running inside the Android app?)");
    }
    // SAFETY: the pointer is android-activity's JavaVM, valid for the process.
    unsafe { jni::JavaVM::from_raw(raw.cast()) }.context("JavaVM::from_raw")
}

/// The Activity if the frontend gave us one — it is a Context too, and the only object
/// that can show the permission dialog. Otherwise ndk-context's *application* context,
/// which is enough for `getSystemService` and `checkSelfPermission`.
fn context<'a>() -> Result<(JObject<'a>, bool)> {
    let activity = ACTIVITY.load(Ordering::Acquire);
    if !activity.is_null() {
        // SAFETY: an unowned JNI global reference, valid for the process.
        return Ok((unsafe { JObject::from_raw(activity.cast()) }, true));
    }
    let app = ndk_context::android_context().context();
    if app.is_null() {
        note(STATE_NO_CONTEXT);
        bail!("no Android context");
    }
    // SAFETY: the application global reference android-activity published.
    Ok((unsafe { JObject::from_raw(app.cast()) }, false))
}

/// A pending Java exception is fatal at detach, so every call clears it and turns it
/// into a plain Rust error.
fn check(env: &mut JNIEnv, what: &str) -> Result<()> {
    if env.exception_check().unwrap_or(false) {
        env.exception_describe().ok();
        env.exception_clear().ok();
        bail!("{what} threw");
    }
    Ok(())
}

fn call<'l>(
    env: &mut JNIEnv<'l>,
    obj: &JObject,
    name: &str,
    sig: &str,
    args: &[JValue],
) -> Result<JValueOwned<'l>> {
    let r = env.call_method(obj, name, sig, args);
    check(env, name)?;
    r.with_context(|| name.to_string())
}

fn call_static<'l>(
    env: &mut JNIEnv<'l>,
    class: &str,
    name: &str,
    sig: &str,
    args: &[JValue],
) -> Result<JValueOwned<'l>> {
    let r = env.call_static_method(class, name, sig, args);
    check(env, name)?;
    r.with_context(|| format!("{class}.{name}"))
}

fn ask_the_phone() -> Result<Option<Fix>> {
    let vm = vm()?;
    // attach_current_thread detaches when the guard drops, which is what a pooled
    // blocking thread wants: no JNI state outlives the call.
    let mut guard = vm.attach_current_thread().context("attach to the JVM")?;
    let env = &mut *guard;
    let (ctx, have_activity) = context()?;

    let sdk = env
        .get_static_field("android/os/Build$VERSION", "SDK_INT", "I")
        .ok()
        .and_then(|v| v.i().ok())
        .unwrap_or(26);

    match permission(env, &ctx)? {
        Grant::Fine => {}
        Grant::Coarse => debug!("geo: only the coarse location permission is granted"),
        Grant::None => {
            note(STATE_NO_PERMISSION);
            if !have_activity {
                warn!(
                    "geo: no location permission and no Activity to ask with — \
                     grant it in Settings ▸ Apps ▸ Sigil ▸ Permissions ▸ Location"
                );
                return Ok(None);
            }
            if ASKED.swap(true, Ordering::SeqCst) {
                warn!("geo: the location permission was refused this run; not asking again");
                return Ok(None);
            }
            request(env, &ctx)?;
            match wait_for_grant(env, &ctx)? {
                Grant::None => {
                    warn!("geo: the location permission was not granted; the ladder moves on");
                    return Ok(None);
                }
                g => info!("geo: location permission granted ({})", g.name()),
            }
        }
    }

    let manager = {
        let name = env.new_string(LOCATION_SERVICE)?;
        call(
            env,
            &ctx,
            "getSystemService",
            "(Ljava/lang/String;)Ljava/lang/Object;",
            &[JValue::Object(&name)],
        )?
        .l()?
    };
    if manager.is_null() {
        bail!("this phone has no LocationManager");
    }

    let fine = matches!(permission_cached(), Grant::Fine);
    let providers = providers(sdk, fine);
    let live: Vec<&str> = providers
        .iter()
        .copied()
        .filter(|p| enabled(env, &manager, p))
        .collect();
    if live.is_empty() {
        note(STATE_SWITCHED_OFF);
        warn!("geo: every location provider is switched off ({})", providers.join(", "));
        return Ok(None);
    }

    // The instant answer. Cheap, and usually right if the phone was moved recently.
    let known = best_known(env, &manager, &providers);
    if let Some(f) = known {
        if fresh(&f) {
            note(STATE_OK);
            debug!(
                "geo: Android last-known fix {:.0} m, {} s old",
                f.accuracy,
                now_ms().saturating_sub(f.at_ms) / 1000
            );
            return Ok(Some(f));
        }
        debug!("geo: the last-known fix is {} s old; waking a provider", now_ms().saturating_sub(f.at_ms) / 1000);
    }

    // Nothing fresh: run the providers and read what they record.
    match wake(env, &ctx, &manager, sdk, &live, &providers)? {
        Some(f) => {
            note(STATE_OK);
            Ok(Some(f))
        }
        None => {
            if known.is_some() {
                note(STATE_OK);
                info!("geo: no new fix in {} s; using the last known one", WAIT_FOR_FRESH.as_secs());
            } else {
                note(STATE_NOTHING_YET);
                info!("geo: LocationManager has recorded no position yet");
            }
            Ok(known)
        }
    }
}

/// Providers worth asking, best first. `fused` merges the others but only exists from
/// API 31; `passive` costs nothing and picks up whatever another app asked for.
/// Without the fine permission the phone would only coarsen a GPS fix, so skip it.
fn providers(sdk: i32, fine: bool) -> Vec<&'static str> {
    let mut v = Vec::new();
    if sdk >= 31 {
        v.push("fused");
    }
    if fine {
        v.push("gps");
    }
    v.push("network");
    v.push("passive");
    v
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Grant {
    Fine,
    Coarse,
    None,
}

impl Grant {
    fn name(self) -> &'static str {
        match self {
            Grant::Fine => "fine",
            Grant::Coarse => "coarse",
            Grant::None => "none",
        }
    }
}

/// The grant the last check saw, so the provider list does not have to re-enter JNI.
static GRANT: AtomicU8 = AtomicU8::new(2);

fn permission_cached() -> Grant {
    match GRANT.load(Ordering::Relaxed) {
        0 => Grant::Fine,
        1 => Grant::Coarse,
        _ => Grant::None,
    }
}

/// `Context.checkSelfPermission(name) == PackageManager.PERMISSION_GRANTED` (0). Works on
/// the application context, so the check never needs the Activity.
fn permission(env: &mut JNIEnv, ctx: &JObject) -> Result<Grant> {
    let granted = |env: &mut JNIEnv, name: &str| -> Result<bool> {
        let s = env.new_string(name)?;
        Ok(call(env, ctx, "checkSelfPermission", "(Ljava/lang/String;)I", &[JValue::Object(&s)])?
            .i()?
            == 0)
    };
    let g = if granted(env, FINE)? {
        Grant::Fine
    } else if granted(env, COARSE)? {
        Grant::Coarse
    } else {
        Grant::None
    };
    GRANT.store(match g { Grant::Fine => 0, Grant::Coarse => 1, Grant::None => 2 }, Ordering::Relaxed);
    Ok(g)
}

/// `Activity.requestPermissions(new String[]{FINE, COARSE}, code)` — the system dialog.
/// targetSdk is 35, so the manifest entry alone grants nothing.
fn request(env: &mut JNIEnv, activity: &JObject) -> Result<()> {
    let fine = env.new_string(FINE)?;
    let coarse = env.new_string(COARSE)?;
    let array = env.new_object_array(2, "java/lang/String", &fine)?;
    env.set_object_array_element(&array, 1, &coarse)?;
    check(env, "new String[]")?;
    info!("geo: asking for the location permission");
    call(
        env,
        activity,
        "requestPermissions",
        "([Ljava/lang/String;I)V",
        &[JValue::Object(array.as_ref()), JValue::Int(PERMISSION_REQUEST_CODE)],
    )?;
    Ok(())
}

/// The dialog's answer arrives on the Activity, not here, so watch the grant instead.
fn wait_for_grant(env: &mut JNIEnv, ctx: &JObject) -> Result<Grant> {
    let deadline = std::time::Instant::now() + WAIT_FOR_ANSWER;
    loop {
        std::thread::sleep(POLL);
        let g = env.with_local_frame(8, |env| permission(env, ctx))?;
        if g != Grant::None {
            return Ok(g);
        }
        if std::time::Instant::now() >= deadline {
            return Ok(Grant::None);
        }
    }
}

/// `LocationManager.isProviderEnabled(provider)`. Unknown providers throw, which `call`
/// turns into a plain false.
fn enabled(env: &mut JNIEnv, manager: &JObject, provider: &str) -> bool {
    let Ok(name) = env.new_string(provider) else { return false };
    matches!(
        call(env, manager, "isProviderEnabled", "(Ljava/lang/String;)Z", &[JValue::Object(&name)])
            .and_then(|v| Ok(v.z()?)),
        Ok(true)
    )
}

/// `LocationManager.getLastKnownLocation(provider)` over every provider, keeping the best.
fn best_known(env: &mut JNIEnv, manager: &JObject, providers: &[&str]) -> Option<Fix> {
    let mut best: Option<Fix> = None;
    for p in providers {
        // A frame each: the loop would otherwise pile up local references.
        let got: Option<Fix> = env
            .with_local_frame(16, |env| -> Result<Option<Fix>> {
                let name = env.new_string(p)?;
                let loc = call(
                    env,
                    manager,
                    "getLastKnownLocation",
                    "(Ljava/lang/String;)Landroid/location/Location;",
                    &[JValue::Object(&name)],
                )?
                .l()?;
                if loc.is_null() {
                    return Ok(None);
                }
                read(env, &loc)
            })
            .unwrap_or(None);
        if let Some(f) = got {
            if best.map_or(true, |b| tighter(&f, &b)) {
                best = Some(f);
            }
        }
    }
    best
}

/// Run the providers until one records something newer than this call, then stop them.
/// `requestLocationUpdates` with a PendingIntent is the one overload that needs no Java
/// object of ours; we never read the broadcast, only the Location it leaves behind.
fn wake(
    env: &mut JNIEnv,
    ctx: &JObject,
    manager: &JObject,
    sdk: i32,
    live: &[&str],
    all: &[&str],
) -> Result<Option<Fix>> {
    let since = now_ms();
    let pending = pending_intent(env, ctx)?;

    // Ask the merged provider on its own where there is one; otherwise the real two.
    let asked: Vec<&str> = if live.contains(&"fused") && sdk >= 31 {
        vec!["fused"]
    } else {
        live.iter().copied().filter(|p| *p == "gps" || *p == "network").collect()
    };
    if asked.is_empty() {
        return Ok(None);
    }
    for p in &asked {
        let name = env.new_string(p)?;
        if let Err(e) = call(
            env,
            manager,
            "requestLocationUpdates",
            "(Ljava/lang/String;JFLandroid/app/PendingIntent;)V",
            &[
                JValue::Object(&name),
                JValue::Long(0),
                JValue::Float(0.0),
                JValue::Object(&pending),
            ],
        ) {
            warn!("geo: {p} would not start: {e:#}");
        }
    }
    debug!("geo: waiting up to {} s on {}", WAIT_FOR_FRESH.as_secs(), asked.join(", "));

    let deadline = std::time::Instant::now() + WAIT_FOR_FRESH;
    let mut out = None;
    loop {
        std::thread::sleep(POLL);
        if let Some(f) = best_known(env, manager, all) {
            // A second of slack: the provider's clock and ours are not the same one.
            if f.at_ms + 1_000 >= since {
                out = Some(f);
                break;
            }
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
    }

    // Always stop: a live request drains the battery for as long as the process runs.
    if let Err(e) = call(env, manager, "removeUpdates", "(Landroid/app/PendingIntent;)V",
                         &[JValue::Object(&pending)]) {
        warn!("geo: could not stop the location updates: {e:#}");
    }
    Ok(out)
}

/// `PendingIntent.getBroadcast(ctx, 0, new Intent(TICK).setPackage(us), FLAGS)`.
fn pending_intent<'l>(env: &mut JNIEnv<'l>, ctx: &JObject) -> Result<JObject<'l>> {
    let action = env.new_string(TICK_ACTION)?;
    let intent = env.new_object(
        "android/content/Intent",
        "(Ljava/lang/String;)V",
        &[JValue::Object(&action)],
    )?;
    check(env, "new Intent")?;
    // Implicit broadcasts are refused from API 26; naming our own package keeps it explicit.
    let package = call(env, ctx, "getPackageName", "()Ljava/lang/String;", &[])?.l()?;
    call(
        env,
        &intent,
        "setPackage",
        "(Ljava/lang/String;)Landroid/content/Intent;",
        &[JValue::Object(&package)],
    )?;
    let pending = call_static(
        env,
        "android/app/PendingIntent",
        "getBroadcast",
        "(Landroid/content/Context;ILandroid/content/Intent;I)Landroid/app/PendingIntent;",
        &[
            JValue::Object(ctx),
            JValue::Int(0),
            JValue::Object(&intent),
            JValue::Int(PENDING_FLAGS),
        ],
    )?
    .l()?;
    if pending.is_null() {
        bail!("PendingIntent.getBroadcast returned null");
    }
    Ok(pending)
}

/// An `android.location.Location` as our `Fix`. `getTime` is the fix's own UTC
/// millisecond stamp, which is what the ladder's staleness rules want — not now.
fn read(env: &mut JNIEnv, loc: &JObject) -> Result<Option<Fix>> {
    let lat = call(env, loc, "getLatitude", "()D", &[])?.d()?;
    let lon = call(env, loc, "getLongitude", "()D", &[])?.d()?;
    if !valid_coords(lat, lon) {
        return Ok(None);
    }
    // (0, 0) is the Atlantic; every provider that means "I do not know" says it this way.
    if lat == 0.0 && lon == 0.0 {
        return Ok(None);
    }
    let accuracy = if call(env, loc, "hasAccuracy", "()Z", &[])?.z()? {
        call(env, loc, "getAccuracy", "()F", &[])?.f()? as f64
    } else {
        0.0
    };
    let at_ms = call(env, loc, "getTime", "()J", &[])?.j()?.max(0) as u64;
    Ok(Some(Fix { lat, lon, accuracy, at_ms, source: Source::Platform }))
}

/// Young enough to answer on its own, by the same bound the beacons use.
fn fresh(f: &Fix) -> bool {
    now_ms().saturating_sub(f.at_ms) <= MAX_FIX_AGE.as_millis() as u64
}

/// Fresh beats stale; between two of the same age class the tighter radius wins, and an
/// unknown radius ranks last.
fn tighter(a: &Fix, b: &Fix) -> bool {
    let rank = |f: &Fix| (!fresh(f), if f.accuracy > 0.0 { f.accuracy } else { f64::MAX });
    let (af, aa) = rank(a);
    let (bf, ba) = rank(b);
    (af, aa) < (bf, ba)
}
