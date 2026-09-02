//! The screenshot harness: every page rendered headless with the demo
//! fixtures, one PNG each, so a phase can be checked against the QML side by
//! side without a display. Runs the real Slint components through the
//! software renderer on a platform that has no window system at all.
//!
//!   cargo run --bin shots -- [out-dir]        (default: shots/)
//!
//! Animations are driven by a fake clock that jumps forward before each
//! capture, so every slide-in and fade is captured settled, and the event
//! loop is a queue drained by hand between frames.

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{EventLoopProxy, Platform, PlatformError, WindowAdapter};
use slint::ComponentHandle;
use slint::Rgb8Pixel;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const WIDTH: u32 = 400;
const HEIGHT: u32 = 820;

type Job = Box<dyn FnOnce() + Send>;

/// Closures posted with `invoke_from_event_loop` wait here until the harness
/// pumps them; there is no real loop.
#[derive(Clone, Default)]
struct Queue(Arc<Mutex<Vec<Job>>>);

impl EventLoopProxy for Queue {
    fn quit_event_loop(&self) -> Result<(), slint::EventLoopError> {
        Ok(())
    }
    fn invoke_from_event_loop(&self, event: Job) -> Result<(), slint::EventLoopError> {
        self.0.lock().unwrap().push(event);
        Ok(())
    }
}

struct Headless {
    window: Rc<MinimalSoftwareWindow>,
    clock: Rc<Cell<Duration>>,
    queue: Queue,
}

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }
    fn duration_since_start(&self) -> Duration {
        self.clock.get()
    }
    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(self.queue.clone()))
    }
}

struct Harness {
    window: Rc<MinimalSoftwareWindow>,
    clock: Rc<Cell<Duration>>,
    queue: Queue,
    out: std::path::PathBuf,
}

impl Harness {
    /// Run queued closures, advance the clock past every animation, redraw.
    fn settle(&self) {
        for _ in 0..4 {
            let jobs: Vec<Job> = std::mem::take(&mut *self.queue.0.lock().unwrap());
            for j in jobs {
                j();
            }
            slint::platform::update_timers_and_animations();
        }
        self.clock.set(self.clock.get() + Duration::from_secs(3));
        slint::platform::update_timers_and_animations();
    }

    fn shoot(&self, name: &str) -> anyhow::Result<()> {
        self.settle();
        let mut pixels = vec![Rgb8Pixel::default(); (WIDTH * HEIGHT) as usize];
        self.window.request_redraw();
        let drew = self.window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, WIDTH as usize);
        });
        anyhow::ensure!(drew, "nothing to draw for {name}");
        let mut bytes = Vec::with_capacity(pixels.len() * 3);
        for p in &pixels {
            bytes.extend_from_slice(&[p.r, p.g, p.b]);
        }
        let path = self.out.join(format!("{name}.png"));
        let file = std::fs::File::create(&path)?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), WIDTH, HEIGHT);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()?.write_image_data(&bytes)?;
        println!("{}", path.display());
        Ok(())
    }
}

fn main() -> anyhow::Result<()> {
    let out = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "shots".into()));
    std::fs::create_dir_all(&out)?;
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let clock = Rc::new(Cell::new(Duration::from_secs(1)));
    let queue = Queue::default();
    slint::platform::set_platform(Box::new(Headless {
        window: window.clone(),
        clock: clock.clone(),
        queue: queue.clone(),
    }))
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    let h = Harness {
        window: window.clone(),
        clock,
        queue,
        out,
    };

    // the demo fixtures instead of a live engine, with the first room open
    std::env::set_var("SIGIL_SLINT_DEMO", "1");
    std::env::set_var("SIGIL_SLINT_DEMO_CHAT", "1");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let app = sigil_slint::AppWindow::new()?;
    app.window()
        .set_size(slint::PhysicalSize::new(WIDTH, HEIGHT));
    let icons = sigil_slint::rows::IconSet::from_window(&app);
    sigil_slint::bridge::start(&app, &rt, icons);
    app.show()?;
    h.settle();

    // the door, as a fresh install would show it
    app.set_session("loggedOut".into());
    h.shoot("login")?;
    app.set_session("loggedIn".into());

    // the open conversation and the pages that hang off it
    app.set_nav("chat".into());
    h.shoot("chat")?;
    for page in ["search", "forward", "chattheme", "roomsettings"] {
        app.set_nav(page.into());
        h.shoot(page)?;
    }
    app.set_nav("chat".into());
    h.settle();

    // home and what starts from it
    app.invoke_back_to_home();
    app.set_nav("home".into());
    h.shoot("home")?;
    app.set_nav("start".into());
    h.shoot("start")?;
    app.set_nav("home".into());
    h.settle();
    app.invoke_set_home_tab(1);
    h.shoot("requests")?;
    Ok(())
}
