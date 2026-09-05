//! A Slint platform with no window system: the software renderer into a
//! pixel buffer, a hand-pumped event queue, and a clock the caller moves.
//! The screenshot harness and the end-to-end driver both run the real
//! components on it, so every page can be exercised and captured without
//! a display.

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{EventLoopProxy, Platform, PlatformError, WindowAdapter};
use slint::Rgb8Pixel;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const WIDTH: u32 = 400;
pub const HEIGHT: u32 = 820;

type Job = Box<dyn FnOnce() + Send>;

/// Closures posted with `invoke_from_event_loop` wait here until pumped.
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

struct HeadlessPlatform {
    window: Rc<MinimalSoftwareWindow>,
    clock: Rc<Cell<Duration>>,
    queue: Queue,
}

impl Platform for HeadlessPlatform {
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

pub struct Harness {
    window: Rc<MinimalSoftwareWindow>,
    clock: Rc<Cell<Duration>>,
    queue: Queue,
    pub out: std::path::PathBuf,
}

impl Harness {
    /// Install the platform. Must run before any component is created.
    pub fn install(out: impl Into<std::path::PathBuf>) -> anyhow::Result<Harness> {
        let out = out.into();
        std::fs::create_dir_all(&out)?;
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        let clock = Rc::new(Cell::new(Duration::from_secs(1)));
        let queue = Queue::default();
        slint::platform::set_platform(Box::new(HeadlessPlatform {
            window: window.clone(),
            clock: clock.clone(),
            queue: queue.clone(),
        }))
        .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(Harness {
            window,
            clock,
            queue,
            out,
        })
    }

    /// Run everything queued so far and tick timers once.
    pub fn pump(&self) {
        let jobs: Vec<Job> = std::mem::take(&mut *self.queue.0.lock().unwrap());
        for j in jobs {
            j();
        }
        slint::platform::update_timers_and_animations();
    }

    /// Move the mock clock forward (timers fire on the next pump).
    pub fn advance(&self, d: Duration) {
        self.clock.set(self.clock.get() + d);
    }

    /// Pump a few rounds, then jump the clock past every animation.
    pub fn settle(&self) {
        for _ in 0..4 {
            self.pump();
        }
        self.clock.set(self.clock.get() + Duration::from_secs(3));
        slint::platform::update_timers_and_animations();
    }

    /// Keep pumping, wall-clock time passing for real, until `pred` holds.
    pub fn wait_until(
        &self,
        what: &str,
        timeout: Duration,
        mut pred: impl FnMut() -> bool,
    ) -> anyhow::Result<()> {
        let start = Instant::now();
        loop {
            self.pump();
            if pred() {
                return Ok(());
            }
            anyhow::ensure!(start.elapsed() < timeout, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(40));
            self.clock.set(self.clock.get() + Duration::from_millis(40));
        }
    }

    pub fn shoot(&self, name: &str) -> anyhow::Result<std::path::PathBuf> {
        self.settle();
        self.frame(name)
    }

    /// The window as it is at this instant of the mock clock — animations
    /// mid-flight, timers unfired — for checking motion frame by frame.
    pub fn frame(&self, name: &str) -> anyhow::Result<std::path::PathBuf> {
        // The window's size as it is now, not the phone's default: a scene
        // may have turned it on its side.
        let size = self.window.size();
        let (w, h) = (size.width.max(1), size.height.max(1));
        let mut pixels = vec![Rgb8Pixel::default(); (w * h) as usize];
        self.window.request_redraw();
        let drew = self.window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, w as usize);
        });
        anyhow::ensure!(drew, "nothing to draw for {name}");
        let mut bytes = Vec::with_capacity(pixels.len() * 3);
        for p in &pixels {
            bytes.extend_from_slice(&[p.r, p.g, p.b]);
        }
        let path = self.out.join(format!("{name}.png"));
        let file = std::fs::File::create(&path)?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()?.write_image_data(&bytes)?;
        println!("{}", path.display());
        Ok(path)
    }
}
