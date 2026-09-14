#![deny(unsafe_op_in_unsafe_fn)]
#![allow(clippy::missing_const_for_thread_local)] // Android TLS expansion; initializer is already const.
use jni::{
    objects::{JClass, JObject},
    sys::{jfloat, jint, jstring},
    JNIEnv,
};
use ndk::native_window::NativeWindow;
use raw_window_handle::{AndroidDisplayHandle, HasWindowHandle, RawDisplayHandle};
use sigil_materials::{geometry::Die, physics::Throw, Lettering, Mode, Object, Renderer, Scene};
use std::{
    cell::RefCell,
    sync::Arc,
    time::{Duration, Instant},
};
mod messages;
thread_local! { static STATE: RefCell<Option<State>> = const { RefCell::new(None) }; }
struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}
fn gpu() -> Result<&'static Gpu, String> {
    static GPU: std::sync::OnceLock<Result<Gpu, String>> = std::sync::OnceLock::new();
    GPU.get_or_init(|| {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .map_err(|e| e.to_string())?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .map_err(|e| e.to_string())?;
        Ok(Gpu {
            instance,
            adapter,
            device,
            queue,
        })
    })
    .as_ref()
    .map_err(Clone::clone)
}
struct Input {
    kind: i32,
    die: i32,
    font: i32,
    mode: i32,
    border: i32,
    action: i32,
    sequence: i32,
    yaw: f32,
    pitch: f32,
    reduced: bool,
    quality: bool,
}
struct State {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Renderer,
    _window: NativeWindow,
    scene: Scene,
    started: Instant,
    running: bool,
    selection: (i32, i32, i32),
    label: String,
    sequence: i32,
    seed: u32,
}
fn wait(device: &wgpu::Device) -> Result<(), String> {
    let start = Instant::now();
    while device
        .poll(wgpu::PollType::Poll)
        .map_err(|e| e.to_string())?
        != wgpu::PollStatus::QueueEmpty
    {
        if start.elapsed() > Duration::from_secs(5) {
            return Err("GPU completion exceeded 5 seconds".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}
impl State {
    fn new(window: NativeWindow, width: u32, height: u32) -> Result<Self, String> {
        let gpu = gpu()?;
        let instance = &gpu.instance;
        // The acquired native window is owned below and dropped after the surface.
        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(RawDisplayHandle::Android(AndroidDisplayHandle::new())),
                raw_window_handle: window.window_handle().map_err(|e| e.to_string())?.as_raw(),
            })
        }
        .map_err(|e| e.to_string())?;
        let adapter = &gpu.adapter;
        let device = gpu.device.clone();
        let queue = gpu.queue.clone();
        let mut config = surface
            .get_default_config(adapter, width, height)
            .ok_or("No surface configuration")?;
        config.format = surface
            .get_capabilities(adapter)
            .formats
            .into_iter()
            .find(|f| !f.is_srgb())
            .ok_or("No linear surface format")?;
        config.present_mode = wgpu::PresentMode::Fifo;
        let renderer = Renderer::with_format(&device, &queue, width, height, config.format);
        queue.submit([]);
        wait(&device)?;
        surface.configure(&device, &config);
        Ok(Self {
            surface,
            device,
            queue,
            renderer,
            _window: window,
            scene: Scene::default(),
            started: Instant::now(),
            running: false,
            selection: (-1, -1, -1),
            label: String::new(),
            sequence: 0,
            seed: 1,
        })
    }
    fn frame(&mut self, input: Input) -> Result<String, String> {
        let Input {
            kind,
            die,
            font,
            mode,
            border,
            action,
            sequence,
            yaw,
            pitch,
            reduced,
            quality,
        } = input;
        if self.selection.0 < 0 {
            self.sequence = sequence;
        }
        if (self.selection.0, self.selection.1) != (kind, die) {
            self.scene.object = match kind {
                1 => Object::Card,
                2 => Object::Coin,
                _ => Object::Die,
            };
            self.scene.die = Die::ALL[die.clamp(0, 5) as usize];
            self.scene.throw = None;
            self.scene.phase = 1.;
            self.running = false;
        }
        if self.selection.2 != font {
            self.renderer.set_label(
                &self.queue,
                "The museum",
                if font == 0 {
                    Lettering::Newsreader
                } else {
                    Lettering::Sans
                },
            );
        }
        self.selection = (kind, die, font);
        self.scene.mode = match mode {
            1 => Mode::Global,
            2 => Mode::Conversation,
            _ => Mode::Personalized,
        };
        self.scene.border = border.clamp(0, 2) as u32;
        let dragged = self.scene.yaw != yaw || self.scene.pitch != pitch;
        self.scene.yaw = yaw;
        self.scene.pitch = pitch;
        self.scene.reduced = reduced;
        if sequence != self.sequence {
            self.sequence = sequence;
            match action {
                1 => {
                    self.seed = self.seed.wrapping_add(1);
                    self.scene.throw = if self.scene.object == Object::Card {
                        None
                    } else {
                        Some(Arc::new(Throw::new(
                            self.scene.object,
                            self.scene.die,
                            self.seed,
                        )))
                    };
                    self.started = Instant::now();
                    self.running = true;
                }
                2 => {
                    self.started = Instant::now();
                    self.running = self.scene.object == Object::Card || self.scene.throw.is_some();
                }
                3 => {
                    self.scene.throw = None;
                    self.running = false;
                    self.scene.phase = 1.;
                }
                _ => {}
            }
        }
        if self.running {
            self.scene.phase =
                (self.started.elapsed().as_secs_f32() / self.scene.duration()).min(1.);
            self.running = self.scene.phase < 1. && !reduced;
        }
        if reduced {
            self.scene.phase = 1.;
            self.running = false;
        }
        self.scene.samples = if quality || (!self.running && !dragged) {
            4
        } else {
            1
        };
        wait(&self.device)?;
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f)
            | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok("1|Waiting for display".into())
            }
            other => return Err(format!("Surface needs reopening: {other:?}")),
        };
        self.renderer.render_to(
            &self.device,
            &self.queue,
            &self.scene,
            &frame.texture.create_view(&Default::default()),
        );
        self.queue.present(frame);
        let result = self
            .scene
            .throw
            .as_ref()
            .map(|t| t.result.as_str())
            .unwrap_or("Inspect");
        Ok(format!(
            "{}|{}",
            u8::from(self.running || (dragged && !quality && !reduced)),
            if self.running { "Rolling…" } else { result }
        ))
    }
}
fn reply(mut env: JNIEnv, work: impl FnOnce() -> Result<String, String>) -> jstring {
    let message = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)) {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => format!("error|{e}"),
        Err(_) => "error|Renderer failed".into(),
    };
    match env.new_string(message) {
        Ok(s) => s.into_raw(),
        Err(_) => {
            let _ = env.throw_new(
                "java/lang/IllegalStateException",
                "Cannot return renderer status",
            );
            std::ptr::null_mut()
        }
    }
}
#[no_mangle]
pub extern "system" fn Java_org_sigil_materials_Native_create(
    env: JNIEnv,
    _class: JClass,
    surface: JObject,
    width: jint,
    height: jint,
) -> jstring {
    // JNI arguments are live for this call; from_surface acquires its own native reference.
    let window = unsafe { NativeWindow::from_surface(env.get_raw(), surface.as_raw()) };
    reply(env, || {
        if !(1..=1600).contains(&width) || !(1..=1200).contains(&height) {
            return Err("Invalid viewport".into());
        }
        let state = State::new(
            window.ok_or("Missing surface")?,
            width as u32,
            height as u32,
        )?;
        STATE.with(|s| *s.borrow_mut() = Some(state));
        Ok("0|Ready".into())
    })
}
#[no_mangle]
pub extern "system" fn Java_org_sigil_materials_Native_frame(
    env: JNIEnv,
    _class: JClass,
    kind: jint,
    die: jint,
    font: jint,
    mode: jint,
    border: jint,
    action: jint,
    sequence: jint,
    yaw: jfloat,
    pitch: jfloat,
    reduced: jint,
    quality: jint,
) -> jstring {
    reply(env, || {
        STATE.with(|s| {
            s.borrow_mut()
                .as_mut()
                .ok_or("Renderer is closed".to_string())?
                .frame(Input {
                    kind,
                    die,
                    font,
                    mode,
                    border,
                    action,
                    sequence,
                    yaw,
                    pitch,
                    reduced: reduced != 0,
                    quality: quality != 0,
                })
        })
    })
}
#[no_mangle]
pub extern "system" fn Java_org_sigil_materials_Native_destroy(_env: JNIEnv, _class: JClass) {
    let _ = std::panic::catch_unwind(|| {
        STATE.with(|s| {
            if let Some(state) = s.borrow_mut().take() {
                let _ = wait(&state.device);
                drop(state);
            }
        })
    });
}
