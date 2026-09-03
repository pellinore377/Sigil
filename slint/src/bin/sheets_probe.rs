// TEMPORARY scratch capture driver for the sheets agent; deleted after use. Not part of the tree.
//! Scratch driver: the sheet, its drawer/confirm, the overflow menu and the
//! list scrollbar, captured at chosen clock offsets (no auto-settle).
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{EventLoopProxy, Platform, PlatformError, WindowAdapter, WindowEvent, PointerEventButton};
use slint::{ComponentHandle, LogicalPosition, Model, Rgb8Pixel};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const W: u32 = 400;
const H: u32 = 820;
type Job = Box<dyn FnOnce() + Send>;
#[derive(Clone, Default)]
struct Queue(Arc<Mutex<Vec<Job>>>);
impl EventLoopProxy for Queue {
    fn quit_event_loop(&self) -> Result<(), slint::EventLoopError> { Ok(()) }
    fn invoke_from_event_loop(&self, e: Job) -> Result<(), slint::EventLoopError> { self.0.lock().unwrap().push(e); Ok(()) }
}
struct Plat { window: Rc<MinimalSoftwareWindow>, clock: Rc<Cell<Duration>>, queue: Queue }
impl Platform for Plat {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> { Ok(self.window.clone()) }
    fn duration_since_start(&self) -> Duration { self.clock.get() }
    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> { Some(Box::new(self.queue.clone())) }
}
struct Hn { window: Rc<MinimalSoftwareWindow>, clock: Rc<Cell<Duration>>, queue: Queue, out: std::path::PathBuf }
impl Hn {
    fn pump(&self) {
        let jobs: Vec<Job> = std::mem::take(&mut *self.queue.0.lock().unwrap());
        for j in jobs { j(); }
        slint::platform::update_timers_and_animations();
    }
    fn advance(&self, ms: u64) {
        self.clock.set(self.clock.get() + Duration::from_millis(ms));
        slint::platform::update_timers_and_animations();
    }
    fn settle(&self) { for _ in 0..4 { self.pump(); } self.advance(3000); }
    fn render(&self, name: &str) {
        self.pump();
        let mut px = vec![Rgb8Pixel::default(); (W * H) as usize];
        self.window.request_redraw();
        let drew = self.window.draw_if_needed(|r| { r.render(&mut px, W as usize); });
        assert!(drew, "nothing drawn for {name}");
        let mut bytes = Vec::with_capacity(px.len() * 3);
        for p in &px { bytes.extend_from_slice(&[p.r, p.g, p.b]); }
        let path = self.out.join(format!("{name}.png"));
        let f = std::fs::File::create(&path).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(f), W, H);
        enc.set_color(png::ColorType::Rgb); enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&bytes).unwrap();
        println!("{}", path.display());
    }
    fn shoot(&self, name: &str) { self.settle(); self.render(name); }
    fn click(&self, x: f32, y: f32) {
        let w = self.window.window();
        w.dispatch_event(WindowEvent::PointerMoved { position: LogicalPosition::new(x, y) });
        self.pump();
        w.dispatch_event(WindowEvent::PointerPressed { position: LogicalPosition::new(x, y), button: PointerEventButton::Left });
        self.pump();
        self.advance(30);
        w.dispatch_event(WindowEvent::PointerReleased { position: LogicalPosition::new(x, y), button: PointerEventButton::Left });
        self.pump();
    }
    fn hover(&self, x: f32, y: f32) {
        self.window.window().dispatch_event(WindowEvent::PointerMoved { position: LogicalPosition::new(x, y) });
        self.pump();
    }
    fn wheel(&self, x: f32, y: f32, dy: f32) {
        self.window.window().dispatch_event(WindowEvent::PointerScrolled { position: LogicalPosition::new(x, y), delta_x: 0.0, delta_y: dy });
        self.pump();
    }
}

fn main() -> anyhow::Result<()> {
    let out = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "probe-shots".into()));
    std::fs::create_dir_all(&out)?;
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let clock = Rc::new(Cell::new(Duration::from_secs(1)));
    let queue = Queue::default();
    slint::platform::set_platform(Box::new(Plat { window: window.clone(), clock: clock.clone(), queue: queue.clone() })).map_err(|e| anyhow::anyhow!("{e}"))?;
    let h = Hn { window, clock, queue, out };

    std::env::set_var("SIGIL_SLINT_DEMO", "1");
    std::env::set_var("SIGIL_SLINT_DEMO_CHAT", "1");
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let app = sigil_slint::AppWindow::new()?;
    app.window().set_size(slint::PhysicalSize::new(W, H));
    let icons = sigil_slint::rows::IconSet::from_window(&app);
    sigil_slint::bridge::start(&app, &rt, icons);
    app.show()?;
    h.settle();
    app.set_session("loggedIn".into());
    app.set_nav("chat".into());
    h.shoot("chat");

    let steps: Vec<String> = std::env::args().skip(2).collect();
    // Each step: "name:cmd:args" -- see below.
    for step in steps {
        let parts: Vec<&str> = step.split(':').collect();
        let name = parts[0];
        match parts[1] {
            "sheet" => {
                let n = app.get_items().row_count() as i32 - 3;
                app.invoke_debug_sheet(n, 250.0, 560.0, 130.0, 44.0);
            }
            "sheetat" => {
                // sheetat:ms  — open and render after ms without settling
                let n = app.get_items().row_count() as i32 - 3;
                app.invoke_debug_sheet(n, 250.0, 560.0, 130.0, 44.0);
                h.pump(); h.advance(parts[2].parse()?); h.render(name); continue;
            }
            "sheetidx" => { app.invoke_debug_sheet(parts[2].parse()?, 250.0, 560.0, 130.0, 44.0); }
            "close" => { app.invoke_debug_sheet_close(); }
            "closeat" => { app.invoke_debug_sheet_close(); h.pump(); h.advance(parts[2].parse()?); h.render(name); continue; }
            "click" => { h.click(parts[2].parse()?, parts[3].parse()?); }
            "clickat" => { h.click(parts[2].parse()?, parts[3].parse()?); h.advance(parts[4].parse()?); h.render(name); continue; }
            "hover" => { h.hover(parts[2].parse()?, parts[3].parse()?); }
            "wheel" => {
                // wheel:x:y:dy:count:gap-ms then render without settling
                let (x, y, dy): (f32, f32, f32) = (parts[2].parse()?, parts[3].parse()?, parts[4].parse()?);
                let n: u32 = parts[5].parse()?; let gap: u64 = parts[6].parse()?;
                for _ in 0..n { h.wheel(x, y, dy); h.advance(gap); }
                h.render(name); continue;
            }
            "nav" => { app.set_nav(parts[2].into()); }
            "home" => { app.invoke_back_to_home(); app.set_nav("home".into()); }
            "contact" => { app.set_contact_open(parts[2] == "1"); }
            "contactat" => { app.set_contact_open(parts[2] == "1"); h.pump(); h.advance(parts[3].parse()?); h.render(name); continue; }
            "settle" => { h.settle(); }
            "advance" => { h.advance(parts[2].parse()?); h.render(name); continue; }
            other => anyhow::bail!("unknown step {other}"),
        }
        if !name.is_empty() { h.shoot(name); }
    }
    Ok(())
}
