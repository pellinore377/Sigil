// SIGIL PATCH: frame pacing from the display's vsync.
//
// The stock event loop polled with a fixed 10ms timeout while an animation
// ran ("FIXME: we should not hardcode a value here") and rendered whenever
// the poll came back, so nothing tied a frame to the display. On a 120Hz
// panel (an 8.3ms period) the compositor recorded frames landing one, two or
// three vsyncs apart in a scramble — every animation and every scroll
// juddered. This asks the Choreographer for the next frame whenever the
// platform wants one and wakes the main loop when it comes, so the loop
// renders once per vsync, with the animations advanced to frame time, and
// never otherwise.
//
// The Choreographer delivers on the looper of the thread that owns it. That
// is not the main thread on purpose: android-activity's poll treats a
// callback ident from its own looper as spurious, so a helper thread with a
// looper of its own takes the callbacks and relays each one through the
// app's waker, which the main loop's poll returns for.

use std::os::raw::{c_int, c_long, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use crate::android_activity::AndroidAppWaker;

#[repr(C)]
struct AChoreographer {
    _private: [u8; 0],
}
#[repr(C)]
struct ALooper {
    _private: [u8; 0],
}
type FrameCallback = extern "C" fn(frame_time_nanos: c_long, data: *mut c_void);

#[link(name = "android")]
unsafe extern "C" {
    fn AChoreographer_getInstance() -> *mut AChoreographer;
    fn AChoreographer_postFrameCallback(
        choreographer: *mut AChoreographer,
        callback: FrameCallback,
        data: *mut c_void,
    );
    fn ALooper_prepare(opts: c_int) -> *mut ALooper;
    fn ALooper_pollOnce(
        timeout_millis: c_int,
        out_fd: *mut c_int,
        out_events: *mut c_int,
        out_data: *mut *mut c_void,
    ) -> c_int;
}

/// ALooper_wake is safe from any thread; the waker only carries the looper.
struct SendWaker(AndroidAppWaker);
unsafe impl Send for SendWaker {}
unsafe impl Sync for SendWaker {}

struct Shared {
    /// The main loop wants a frame (set by `request`, taken by the helper).
    wanted: Mutex<bool>,
    wanted_cv: Condvar,
    /// A frame callback has fired and the main loop has not yet seen it.
    fired: AtomicBool,
    /// The outstanding callback has been delivered (the helper stops pumping).
    delivered: AtomicBool,
    waker: SendWaker,
}

pub struct Vsync {
    shared: Arc<Shared>,
}

extern "C" fn on_frame(_frame_time_nanos: c_long, data: *mut c_void) {
    // SAFETY: `data` is the `Arc<Shared>` the helper keeps alive for the
    // life of the process (the helper thread never exits).
    let shared = unsafe { &*(data as *const Shared) };
    shared.fired.store(true, Ordering::Release);
    shared.delivered.store(true, Ordering::Release);
    shared.waker.0.wake();
}

impl Vsync {
    pub fn start(waker: AndroidAppWaker) -> Self {
        let shared = Arc::new(Shared {
            wanted: Mutex::new(false),
            wanted_cv: Condvar::new(),
            fired: AtomicBool::new(false),
            delivered: AtomicBool::new(false),
            waker: SendWaker(waker),
        });
        let helper = shared.clone();
        std::thread::Builder::new()
            .name("slint-vsync".into())
            .spawn(move || {
                // SAFETY: plain NDK calls on this thread, which owns the looper
                // and the Choreographer instance it prepares here.
                let choreographer = unsafe {
                    ALooper_prepare(0);
                    AChoreographer_getInstance()
                };
                if choreographer.is_null() {
                    i_slint_core::debug_log!("slint-vsync: no Choreographer on the helper thread");
                    return;
                }
                let data = Arc::as_ptr(&helper) as *mut c_void;
                loop {
                    {
                        let mut wanted = helper.wanted.lock().unwrap();
                        while !*wanted {
                            wanted = helper.wanted_cv.wait(wanted).unwrap();
                        }
                        *wanted = false;
                    }
                    helper.delivered.store(false, Ordering::Release);
                    unsafe { AChoreographer_postFrameCallback(choreographer, on_frame, data) };
                    while !helper.delivered.load(Ordering::Acquire) {
                        unsafe {
                            ALooper_pollOnce(
                                -1,
                                core::ptr::null_mut(),
                                core::ptr::null_mut(),
                                core::ptr::null_mut(),
                            )
                        };
                    }
                }
            })
            .expect("slint-vsync thread");
        Self { shared }
    }

    /// Ask for the next vsync. Idempotent: one callback stands at a time.
    pub fn request(&self) {
        let mut wanted = self.shared.wanted.lock().unwrap();
        if !*wanted {
            *wanted = true;
            self.shared.wanted_cv.notify_one();
        }
    }

    /// Whether a vsync has come since the last call.
    pub fn take(&self) -> bool {
        self.shared.fired.swap(false, Ordering::AcqRel)
    }
}

unsafe extern "C" {
    fn dlopen(filename: *const std::os::raw::c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const std::os::raw::c_char) -> *mut c_void;
}

/// Vote the window's frame rate at the display's peak. A window that votes
/// nothing is rendered at whatever the compositor infers, and on a panel with
/// an adaptive refresh rate that came out as a steady 60 for us while the
/// display itself ran at 120. `ANativeWindow_setFrameRate` is API 30 and the
/// `WithChangeStrategy` form API 31, so both are looked up at run time and an
/// older device is left as it was.
pub fn vote_frame_rate(window: *mut c_void, fps: f32) {
    const RTLD_NOW: c_int = 2;
    // ANATIVEWINDOW_FRAME_RATE_COMPATIBILITY_DEFAULT, ANATIVEWINDOW_CHANGE_FRAME_RATE_ALWAYS
    const COMPAT_DEFAULT: i8 = 0;
    const CHANGE_ALWAYS: i8 = 1;
    // SAFETY: dlopen/dlsym on libandroid, which the process already links;
    // the looked-up functions are called with the documented signatures on
    // a live ANativeWindow the caller owns for the duration of the call.
    unsafe {
        let lib = dlopen(c"libandroid.so".as_ptr(), RTLD_NOW);
        if lib.is_null() {
            return;
        }
        let with_strategy = dlsym(lib, c"ANativeWindow_setFrameRateWithChangeStrategy".as_ptr());
        if !with_strategy.is_null() {
            let f: extern "C" fn(*mut c_void, f32, i8, i8) -> i32 =
                core::mem::transmute(with_strategy);
            f(window, fps, COMPAT_DEFAULT, CHANGE_ALWAYS);
            return;
        }
        let plain = dlsym(lib, c"ANativeWindow_setFrameRate".as_ptr());
        if !plain.is_null() {
            let f: extern "C" fn(*mut c_void, f32, i8) -> i32 = core::mem::transmute(plain);
            f(window, fps, COMPAT_DEFAULT);
        }
    }
}
