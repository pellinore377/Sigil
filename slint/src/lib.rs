//! Sigil's Slint frontend. The engine is a Rust dependency, not a daemon:
//! `bridge` hosts it on a tokio runtime and the whole JSON protocol crosses
//! one channel instead of a socket. `ui/` is the view; this crate is the glue.

slint::include_modules!();

pub mod actions;
pub mod bridge;
pub mod call;
pub mod composer;
pub mod frost;
pub mod fx;
pub mod headless;
pub mod mapview;
pub mod platform;
pub mod project;
pub mod qr;
pub mod rows;
pub mod scale;
pub mod video;

/// The app's two families, handed to Slint from the engine's one embedded
/// copy of each (sigil_engine::fonts) rather than imported in style.slint,
/// which would embed them a second time. Before the first window.
pub fn register_fonts() {
    use slint::fontique_010::fontique;
    // The process-wide collection every renderer resolves `font-family`
    // through (Slint 1.17 lays text out with parley on all of them).
    let mut collection = slint::fontique_010::shared_collection();
    for (name, bytes) in [
        ("Google Sans Flex", sigil_engine::fonts::SANS),
        ("Google Sans Code", sigil_engine::fonts::CODE),
    ] {
        let blob = fontique::Blob::new(std::sync::Arc::new(bytes));
        let fonts = collection.register_fonts(blob, None);
        if fonts.is_empty() {
            tracing::error!("font: {name} did not register");
        }
    }
}

pub fn run_app() -> anyhow::Result<()> {
    // The engine's own daemon runs 16 MB stacks; matrix-sdk wants the room.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(16 * 1024 * 1024)
        .build()?;

    // The device switches, read before anything starts: "demo" stands the
    // whole app up on the fixtures with no account and no server, which is
    // how the phone's own look is checked at its real size and density.
    //   adb shell 'echo demo > /data/local/tmp/sigil-flags'
    let flags = std::fs::read_to_string("/data/local/tmp/sigil-flags").unwrap_or_default();
    let flag = |name: &str| flags.lines().any(|l| l.trim() == name);
    if flag("demo") {
        std::env::set_var("SIGIL_SLINT_DEMO", "1");
        std::env::set_var("SIGIL_SLINT_DEMO_CHAT", "1");
    }
    // `perf`: Slint's own frame-time report in the log (its metrics collector
    // reads this at renderer creation), for telling a render-bound stutter
    // from an event-bound one on the device.
    if flag("perf") {
        std::env::set_var("SLINT_DEBUG_PERFORMANCE", "refresh_lazy,console");
    }

    register_fonts();
    let win = AppWindow::new()?;
    let icons = rows::IconSet::from_window(&win);
    let req = bridge::start(&win, &rt, icons);
    // The activity's Resume and Pause. In front: the engine proves its
    // Envoy socket at once (a phone's socket dies silently while it sleeps,
    // and a message held at the Envoy would otherwise wait for the dead
    // socket to be noticed), and no notification is posted for the room on
    // screen. At the back: notifications for everything.
    #[cfg(target_os = "android")]
    i_slint_backend_android_activity::set_lifecycle_hook(Box::new(move |front| {
        platform::FOREGROUND.store(front, std::sync::atomic::Ordering::Relaxed);
        req.fire("app.foreground", serde_json::json!({"on": front}));
    }));
    #[cfg(not(target_os = "android"))]
    drop(req);
    // For the phone-shaped desktop check: force the phone palette.
    if let Ok(m) = std::env::var("SIGIL_THEME_MODE") {
        win.global::<Theme>().set_mode(m.as_str().into());
        rows::DARK_SCHEME.store(m != "light", std::sync::atomic::Ordering::Relaxed);
    }
    // The rest of the device switches, one flag name per line.
    {
        let t = win.global::<Theme>();
        if flag("no-fx-copies") {
            t.set_dbg_no_fx_copies(true);
        }
        if flag("no-convo-clip") {
            t.set_dbg_no_convo_clip(true);
        }
        if flag("cache-bubbles") {
            t.set_dbg_cache_bubbles(true);
        }
        // "dark" / "light" force the palette a device check wants to see.
        if flag("dark") || flag("light") {
            let m = if flag("dark") { "dark" } else { "light" };
            t.set_mode(m.into());
            rows::DARK_SCHEME.store(m == "dark", std::sync::atomic::Ordering::Relaxed);
            scale::FORCED_MODE.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
    scale::keep(&win);
    // the desktop shows the card in its frame; a phone is the card
    #[cfg(not(target_os = "android"))]
    win.set_card_frame(true);
    // Android delivers the back gesture as a close request; unwind our nav
    // first and only let the OS have it from the home screen.
    #[cfg(target_os = "android")]
    {
        // Back arrives as a KEY (Key.Back, taken by app.slint's back-scope),
        // not as a close request: the request only fires when nothing in the
        // UI accepted the key. The camera's viewfinder is a window of its own
        // laid over the app (java/SigilCamera.java), not focusable, so the
        // key lands in the app underneath it — and the app's first move on
        // Back with a viewfinder up is to close that and stop. It used to
        // unwind the room as well, because the key went straight to go-back
        // while the camera was only checked on the (never fired) close path.
        win.on_camera_back(|| {
            if platform::camera_live() {
                tracing::info!("camera: back closed the viewfinder");
                platform::camera_close();
                true
            } else {
                false
            }
        });
        let weak = win.as_weak();
        win.window().on_close_requested(move || {
            if let Some(w) = weak.upgrade() {
                // The same guard for a close request, should one ever arrive
                // with the viewfinder up (a key nobody accepted).
                if platform::camera_live() {
                    platform::camera_close();
                    return slint::CloseRequestResponse::KeepWindowShown;
                }
                if w.get_nav() != "home"
                    || w.get_viewer_open()
                    || w.get_attach_open()
                    || w.get_recorder_open()
                {
                    if w.get_viewer_open() && w.get_viewer_picker_open() {
                        // the viewer's emoji drawer goes first, the viewer stays
                        w.set_viewer_picker_open(false);
                    } else if w.get_viewer_open() {
                        w.set_viewer_open(false);
                    } else if w.get_attach_open() {
                        w.set_attach_open(false);
                    } else if w.get_recorder_open() {
                        w.set_recorder_open(false);
                    } else {
                        w.invoke_go_back();
                    }
                    return slint::CloseRequestResponse::KeepWindowShown;
                }
            }
            slint::CloseRequestResponse::HideWindow
        });
    }
    win.run()?;
    Ok(())
}

/// Entry point for cargo-apk's NativeActivity loader.
#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: slint::android::AndroidApp) {
    // Paths first: the engine resolves its state and cache through XDG
    // variables (core/src/paths.rs), which do not exist on Android.
    let data = app
        .internal_data_path()
        .unwrap_or_else(|| std::path::PathBuf::from("/data/data/com.sigil.slint/files"));
    std::env::set_var("HOME", &data);
    std::env::set_var("XDG_STATE_HOME", data.join("state"));
    std::env::set_var("XDG_CACHE_HOME", data.join("cache"));
    std::fs::create_dir_all(data.join("state")).ok();
    std::fs::create_dir_all(data.join("cache")).ok();

    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::Layer as _;
    if let Ok(layer) = tracing_android::layer("sigil") {
        // Info and up: the dependency firehose at verbose drowned logcat, and
        // request traces carry ids that do not belong in a system log.
        let filter = tracing_subscriber::EnvFilter::new("info,hyper_util=warn,eyeball=warn");
        tracing_subscriber::registry()
            .with(layer.with_filter(filter))
            .try_init()
            .ok();
    }

    // Slint's runtime logs through the `log` crate (its frame-time report
    // under the `perf` flag, and its own warnings): route those to logcat
    // under the "slint" tag, quietly except for what is asked for.
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Debug)
            .with_tag("slint"),
    );

    // A panic on the UI thread ends the native thread and the activity
    // quietly drops to the background — its message goes to stderr, which
    // the phone never shows. Name it in logcat first.
    std::panic::set_hook(Box::new(|info| {
        tracing::error!("panic: {info}");
    }));

    scale::remember_android(app.clone());
    // The engine's location backend needs the Activity, not the application
    // context ndk-context carries: only an Activity can show the runtime
    // permission dialog. SAFETY: both are android-activity's own pointers,
    // valid for the life of the process.
    sigil_engine::geo::android::use_activity(app.vm_as_ptr(), app.activity_as_ptr());
    // The `perf` switch must be in the environment BEFORE the backend comes
    // up: Slint's metrics collector reads it when the renderer is created,
    // which happens in `init` below, not in `run_app`.
    if std::fs::read_to_string("/data/local/tmp/sigil-flags")
        .map(|f| f.lines().any(|l| l.trim() == "perf"))
        .unwrap_or(false)
    {
        std::env::set_var("SLINT_DEBUG_PERFORMANCE", "refresh_lazy,console");
    }
    slint::android::init(app).expect("slint android init");
    // The runtime's own messages — the frame report above all — through the
    // app's tracing logger, which does reach logcat. The `log` route they
    // take by default never showed on the device.
    let _ = i_slint_core::with_global_context(
        || Err(i_slint_core::api::PlatformError::NoPlatform),
        |ctx| {
            ctx.set_log_message_handler(Some(Box::new(|m| {
                tracing::info!(target: "slint", "{}", m.message_arguments());
            })));
        },
    );
    run_app().expect("sigil-slint");
}
