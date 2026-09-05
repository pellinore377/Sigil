// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

// cSpell: ignore androidwindowadapter javahelper
#![doc = include_str!("README.md")]
#![doc(html_logo_url = "https://slint.dev/logo/slint-logo-square-light.svg")]
#![cfg_attr(not(target_os = "android"), allow(rustdoc::broken_intra_doc_links))]
#![cfg(target_os = "android")]
#![cfg_attr(slint_nightly_test, feature(non_exhaustive_omitted_patterns_lint))]
#![cfg_attr(slint_nightly_test, warn(non_exhaustive_omitted_patterns))]

mod androidwindowadapter;
mod javahelper;
mod vsync;

#[cfg(all(not(feature = "aa-06"), feature = "aa-05"))]
pub use android_activity_05 as android_activity;
#[cfg(feature = "aa-06")]
pub use android_activity_06 as android_activity;

pub use android_activity::AndroidApp;
use android_activity::PollEvent;
use androidwindowadapter::AndroidWindowAdapter;
use core::ops::ControlFlow;
use core::time::Duration;
use i_slint_core::api::{EventLoopError, PlatformError};
use i_slint_core::platform::{Clipboard, WindowAdapter};
use i_slint_renderer_skia::SkiaRendererExt;
use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::{Arc, Mutex};

thread_local! {
    static CURRENT_WINDOW: RefCell<Weak<AndroidWindowAdapter>> = RefCell::new(Default::default());
}

pub struct AndroidPlatform {
    app: AndroidApp,
    window: Rc<AndroidWindowAdapter>,
    event_listener: Option<Box<dyn Fn(&PollEvent<'_>)>>,
    /// SIGIL PATCH: the display's vsync, which paces every frame.
    vsync: vsync::Vsync,
}

impl AndroidPlatform {
    /// Instantiate a new Android backend given the [`android_activity::AndroidApp`]
    ///
    /// Pass the returned value to [`slint::platform::set_platform()`](`i_slint_core::platform::set_platform()`)
    ///
    /// # Example
    /// ```
    /// #[cfg(target_os = "android")]
    /// #[unsafe(no_mangle)]
    /// fn android_main(app: i_slint_backend_android_activity::AndroidApp) {
    ///     slint::platform::set_platform(Box::new(
    ///         i_slint_backend_android_activity::AndroidPlatform::new(app),
    ///     ))
    ///     .unwrap();
    ///     // ... your slint application ...
    /// }
    /// ```
    pub fn new(app: AndroidApp) -> Self {
        let window = AndroidWindowAdapter::new(app.clone());
        CURRENT_WINDOW.set(Rc::downgrade(&window));
        let vsync = vsync::Vsync::start(app.create_waker());
        Self { app, window, event_listener: None, vsync }
    }

    /// Instantiate a new Android backend given the [`android_activity::AndroidApp`]
    /// and a function to process the events.
    ///
    /// This is the same as [`AndroidPlatform::new()`], but it allow you to get notified
    /// of events.
    ///
    /// Pass the returned value to [`slint::platform::set_platform()`](`i_slint_core::platform::set_platform()`)
    ///
    /// # Example
    /// ```
    /// #[cfg(target_os = "android")]
    /// #[unsafe(no_mangle)]
    /// fn android_main(app: i_slint_backend_android_activity::AndroidApp) {
    ///     slint::platform::set_platform(Box::new(
    ///         i_slint_backend_android_activity::AndroidPlatform::new_with_event_listener(
    ///             app,
    ///             |event| { eprintln!("got event {event:?}") }
    ///         ),
    ///     ))
    ///     .unwrap();
    ///     // ... your slint application ...
    /// }
    /// ```
    pub fn new_with_event_listener(
        app: AndroidApp,
        listener: impl Fn(&PollEvent<'_>) + 'static,
    ) -> Self {
        let mut this = Self::new(app);
        this.event_listener = Some(Box::new(listener));
        this
    }
}

impl i_slint_core::platform::Platform for AndroidPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }
    fn run_event_loop(&self) -> Result<(), PlatformError> {
        loop {
            let timeout = i_slint_core::platform::duration_until_next_timer_update();
            // SIGIL PATCH: a frame is wanted while an animation runs or a
            // redraw is pending. The stock loop capped the poll at a fixed
            // 10ms for animations and rendered whenever it came back, which
            // put frames one, two or three vsyncs apart at random on a 120Hz
            // panel. Now the vsync helper wakes the poll at the display's
            // next frame and the render happens then — once per vsync, with
            // the animations advanced to frame time — and never otherwise.
            if self.window.window.has_active_animations() || self.window.pending_redraw.get() {
                self.vsync.request();
            }
            let mut r = Ok(ControlFlow::Continue(()));
            self.app.poll_events(timeout, |e| {
                i_slint_core::platform::update_timers_and_animations();
                r = self.window.process_event(&e);
                if let Some(event_listener) = &self.event_listener {
                    event_listener(&e)
                }
            });
            if r?.is_break() {
                break;
            }
            if self.vsync.take() {
                i_slint_core::platform::update_timers_and_animations();
                if self.window.window.has_active_animations() {
                    // the next frame is asked for before this one is drawn,
                    // so the request is in ahead of the vsync it is for
                    self.vsync.request();
                }
                if self.window.pending_redraw.take() || self.window.window.has_active_animations() {
                    // SIGIL PATCH: a frame that overran its vsync slot is
                    // named in the log, with its cost — the chop is in these.
                    let started = std::time::Instant::now();
                    self.window.do_render()?;
                    let took = started.elapsed();
                    if took > Duration::from_millis(9) {
                        i_slint_core::debug_log!("slow frame: {took:?}");
                    }
                }
            }
        }
        Ok(())
    }

    fn new_event_loop_proxy(&self) -> Option<Box<dyn i_slint_core::platform::EventLoopProxy>> {
        Some(Box::new(AndroidEventLoopProxy {
            event_queue: self.window.event_queue.clone(),
            waker: self.app.create_waker(),
        }))
    }

    fn set_clipboard_text(&self, text: &str, clipboard: Clipboard) {
        if clipboard == Clipboard::DefaultClipboard {
            self.window
                .java_helper
                .set_clipboard(text)
                .unwrap_or_else(|e| javahelper::print_jni_error(&self.app, e));
        }
    }

    fn clipboard_text(&self, clipboard: Clipboard) -> Option<String> {
        if clipboard == Clipboard::DefaultClipboard {
            Some(
                self.window
                    .java_helper
                    .get_clipboard()
                    .unwrap_or_else(|e| javahelper::print_jni_error(&self.app, e)),
            )
        } else {
            None
        }
    }

    fn bind_context(&self, ctx: i_slint_core::SlintContextWeak, _: i_slint_core::InternalToken) {
        let ctx = ctx.upgrade().expect("bind_context called while the SlintContext is still alive");
        let color_scheme = match self
            .window
            .java_helper
            .color_scheme()
            .unwrap_or_else(|e| javahelper::print_jni_error(&self.app, e))
        {
            0x10 => i_slint_core::items::ColorScheme::Light, // UI_MODE_NIGHT_NO
            0x20 => i_slint_core::items::ColorScheme::Dark,  // UI_MODE_NIGHT_YES
            _ => i_slint_core::items::ColorScheme::Unknown,
        };
        ctx.set_color_scheme(color_scheme);
        if let Ok(accent) = self.window.java_helper.accent_color() {
            ctx.set_accent_color(accent);
        }
        if let Ok(scale) = self.window.java_helper.font_scale()
            && let Some(size) = javahelper::font_scale_to_logical_length(scale)
        {
            ctx.set_platform_default_font_size(Some(size));
        }
    }

    fn long_press_interval(&self, _: i_slint_core::InternalToken) -> Duration {
        self.window.java_helper.long_press_timeout().unwrap_or(Duration::from_millis(500))
    }
}

/// SIGIL PATCH: the activity's Resume and Pause, for the app. Slint only
/// reports window focus, which a notification shade or a permission
/// dialog also takes; the app wants to know when it is truly in front
/// (prove the network, no notifications for the open room) and when it
/// has gone to the back. Called on the main thread with `true` on Resume
/// and `false` on Pause.
pub fn set_lifecycle_hook(hook: Box<dyn Fn(bool)>) {
    LIFECYCLE_HOOK.with(|h| *h.borrow_mut() = Some(hook));
}

thread_local! {
    // Main thread only, set and called: the hook may hold the app's own
    // (non-Send) handles.
    static LIFECYCLE_HOOK: RefCell<Option<Box<dyn Fn(bool)>>> = const { RefCell::new(None) };
}

pub(crate) fn lifecycle(in_front: bool) {
    LIFECYCLE_HOOK.with(|h| {
        if let Some(h) = h.borrow().as_ref() {
            h(in_front);
        }
    });
}

/// SIGIL PATCH: the app reads the phone's mode and scale and steers the
/// system-bar glyphs. All main-thread only (where the Slint UI runs).
pub fn night_mode() -> Option<bool> {
    let a = CURRENT_WINDOW.with_borrow(|x| x.upgrade())?;
    a.java_helper.color_scheme().ok().map(|m| m == 0x20)
}

pub fn system_font_scale() -> Option<f32> {
    let a = CURRENT_WINDOW.with_borrow(|x| x.upgrade())?;
    a.java_helper.font_scale().ok()
}

pub fn system_accent_color() -> Option<i_slint_core::Color> {
    let a = CURRENT_WINDOW.with_borrow(|x| x.upgrade())?;
    a.java_helper.accent_color().ok().filter(|c| c.alpha() > 0)
}

pub fn haptic_long_press() {
    if let Some(a) = CURRENT_WINDOW.with_borrow(|x| x.upgrade()) {
        let _ = a.java_helper.haptic_long_press();
    }
}

pub fn set_dark_system_bars(dark: bool) {
    if let Some(a) = CURRENT_WINDOW.with_borrow(|x| x.upgrade()) {
        let _ = a.java_helper.set_dark_system_bars(dark);
    }
}

enum Event {
    Quit,
    Other(Box<dyn FnOnce() + Send + 'static>),
}

type EventQueue = Arc<Mutex<Vec<Event>>>;

struct AndroidEventLoopProxy {
    event_queue: EventQueue,
    waker: android_activity::AndroidAppWaker,
}

impl i_slint_core::platform::EventLoopProxy for AndroidEventLoopProxy {
    fn quit_event_loop(&self) -> Result<(), EventLoopError> {
        self.event_queue.lock().unwrap().push(Event::Quit);
        self.waker.wake();
        Ok(())
    }

    fn invoke_from_event_loop(
        &self,
        event: Box<dyn FnOnce() + Send>,
    ) -> Result<(), EventLoopError> {
        self.event_queue.lock().unwrap().push(Event::Other(event));
        self.waker.wake();
        Ok(())
    }
}

pub fn set_requested_graphics_api(
    requested_graphics_api: Option<i_slint_core::graphics::RequestedGraphicsAPI>,
) -> Result<(), PlatformError> {
    let Some(adapter) = CURRENT_WINDOW.with_borrow(|x| x.upgrade()) else {
        return Err(format!("On Android a graphics API for Slint can only be requested after calling slint::android::init()").into());
    };
    adapter.set_requested_graphics_api(requested_graphics_api);
    Ok(())
}
