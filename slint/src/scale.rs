//! The UI's scale. The QML this port follows drew everything through the
//! host shell's spacing scale, so its numbers are "design pixels" that the
//! shell enlarged; Slint draws them at 1:1 unless told otherwise, which is
//! too small on a phone. The window's scale factor is the one knob that
//! enlarges everything together, fonts and geometry alike, so this keeps it
//! at `platform scale × UI_SCALE`.
//!
//! The backends set their own value (Android from the screen density, winit
//! from the monitor) at window creation and on configuration changes, and
//! nothing lets us multiply that at the source, so a light timer watches
//! the value and corrects it within a frame or two whenever a backend
//! resets it. `SIGIL_UI_SCALE` overrides the multiplier.

use slint::ComponentHandle;
use std::sync::OnceLock;

/// The multiplier over the platform's scale. 1.25 puts the QML's 12 px body
/// text at 15 px, which is where phone body text sits.
pub const UI_SCALE: f32 = 1.25;

#[cfg(target_os = "android")]
static ANDROID: OnceLock<slint::android::AndroidApp> = OnceLock::new();

#[cfg(target_os = "android")]
pub fn remember_android(app: slint::android::AndroidApp) {
    let _ = ANDROID.set(app);
}

fn multiplier() -> f32 {
    static M: OnceLock<f32> = OnceLock::new();
    *M.get_or_init(|| {
        std::env::var("SIGIL_UI_SCALE")
            .ok()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .filter(|v| *v > 0.25 && *v < 4.0)
            .unwrap_or(UI_SCALE)
    })
}

/// The scale the platform would use on its own.
fn platform_scale(current: f32, last_set: Option<f32>) -> f32 {
    #[cfg(target_os = "android")]
    {
        if let Some(app) = ANDROID.get() {
            if let Some(dpi) = app.config().density() {
                return dpi as f32 / 160.0;
            }
        }
    }
    // Elsewhere the backend's value is only visible when it is not ours:
    // whatever is current that we did not set is the platform's.
    match last_set {
        Some(ours) if (current - ours).abs() < 0.001 => current / multiplier(),
        _ => current,
    }
}

/// Keep the window at platform × multiplier for its whole life.
pub fn keep(win: &crate::AppWindow) {
    if (multiplier() - 1.0).abs() < 0.001 {
        return;
    }
    let weak: slint::Weak<crate::AppWindow> = win.as_weak();
    let last_set = std::rc::Rc::new(std::cell::Cell::new(None::<f32>));
    let last_size = std::rc::Rc::new(std::cell::Cell::new((0u32, 0u32)));
    let apply = {
        let weak = weak.clone();
        let last_set = last_set.clone();
        let last_size = last_size.clone();
        move || {
            let Some(w): Option<crate::AppWindow> = weak.upgrade() else { return };
            let current = w.window().scale_factor();
            let want = platform_scale(current, last_set.get()) * multiplier();
            let phys = w.window().size();
            let rescaled = (current - want).abs() > 0.001;
            if rescaled {
                w.window().dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged {
                    scale_factor: want,
                });
                last_set.set(Some(want));
            }
            // A scale change alone leaves the window's logical size where
            // the backend computed it (physical ÷ its own scale), so the
            // page lays out for a screen bigger than the real one and runs
            // off its right and bottom edges. The logical size follows:
            // physical ÷ our scale, re-sent whenever the surface changes.
            let want_w = phys.width as f32 / want;
            let want_h = phys.height as f32 / want;
            let laid_out = (w.get_logical_width(), w.get_logical_height());
            let off = (laid_out.0 - want_w).abs() > 0.5 || (laid_out.1 - want_h).abs() > 0.5;
            if phys.width > 0
                && phys.height > 0
                && (rescaled || off || last_size.get() != (phys.width, phys.height))
            {
                w.window().dispatch_event(slint::platform::WindowEvent::Resized {
                    size: slint::LogicalSize::new(want_w, want_h),
                });
                last_size.set((phys.width, phys.height));
            }
        }
    };
    apply();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(120),
        apply,
    );
    // the timer lives as long as the window
    std::mem::forget(timer);
}
