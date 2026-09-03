//! Sigil's Slint frontend. The engine is a Rust dependency, not a daemon:
//! `bridge` hosts it on a tokio runtime and the whole JSON protocol crosses
//! one channel instead of a socket. `ui/` is the view; this crate is the glue.

slint::include_modules!();

pub mod actions;
pub mod bridge;
pub mod call;
pub mod frost;
pub mod fx;
pub mod headless;
pub mod platform;
pub mod project;
pub mod qr;
pub mod rows;

pub fn run_app() -> anyhow::Result<()> {
    // The engine's own daemon runs 16 MB stacks; matrix-sdk wants the room.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(16 * 1024 * 1024)
        .build()?;

    let win = AppWindow::new()?;
    let icons = rows::IconSet::from_window(&win);
    bridge::start(&win, &rt, icons);
    // Android delivers the back gesture as a close request; unwind our nav
    // first and only let the OS have it from the home screen.
    #[cfg(target_os = "android")]
    {
        let weak = win.as_weak();
        win.window().on_close_requested(move || {
            if let Some(w) = weak.upgrade() {
                if w.get_nav() != "home"
                    || w.get_viewer_open()
                    || w.get_attach_open()
                    || w.get_recorder_open()
                {
                    if w.get_viewer_open() {
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

    slint::android::init(app).expect("slint android init");
    run_app().expect("sigil-slint");
}
