use crate::{geometry::Die, presentation::result_pose, Lettering, Mode, Object, Renderer, Scene};
use serde::Deserialize;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use wasm_bindgen::prelude::*;

struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    failed: Arc<AtomicBool>,
}
thread_local! {static VIEWS:Cell<u32>=const{Cell::new(0)};static GPU:RefCell<Option<Rc<Gpu>>>=const{RefCell::new(None)};}
fn fail(message: &str) -> JsValue {
    JsValue::from_str(message)
}
#[wasm_bindgen]
pub async fn initialize_materials() -> Result<bool, JsValue> {
    if let Some(hardware) = GPU.with(|v| {
        v.borrow()
            .as_ref()
            .map(|gpu| gpu.adapter.get_info().device_type != wgpu::DeviceType::Cpu)
    }) {
        return Ok(hardware);
    }
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::BROWSER_WEBGPU,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .await
        .map_err(|_| fail("WebGPU unavailable"))?;
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default())
        .await
        .map_err(|_| fail("WebGPU device unavailable"))?;
    let failed = Arc::new(AtomicBool::new(false));
    let flag = failed.clone();
    device.on_uncaptured_error(Arc::new(move |_: wgpu::Error| {
        flag.store(true, Ordering::Relaxed);
    }));
    GPU.with(|v| {
        if v.borrow().is_none() {
            *v.borrow_mut() = Some(Rc::new(Gpu {
                instance,
                adapter,
                device,
                queue,
                failed,
            }));
        }
    });
    Ok(GPU.with(|v| {
        v.borrow()
            .as_ref()
            .is_some_and(|gpu| gpu.adapter.get_info().device_type != wgpu::DeviceType::Cpu)
    }))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Frame {
    kind: u32,
    sides: u32,
    face: u32,
    font: u32,
    accent: u32,
    progress: f32,
    rotation: Option<[f32; 4]>,
    label: String,
    style: [f32; 13],
}
fn rgb(c: u32) -> [f32; 3] {
    [
        (c >> 16 & 255) as f32 / 255.,
        (c >> 8 & 255) as f32 / 255.,
        (c & 255) as f32 / 255.,
    ]
}
#[wasm_bindgen]
pub struct MaterialView {
    gpu: Rc<Gpu>,
    surface: wgpu::Surface<'static>,
    renderer: Renderer,
    label: Option<(u32, String)>,
    pending: Arc<AtomicBool>,
}
#[wasm_bindgen]
impl MaterialView {
    #[wasm_bindgen(constructor)]
    pub fn new(canvas: web_sys::HtmlCanvasElement) -> Result<MaterialView, JsValue> {
        if VIEWS.with(|v| v.get() >= 24) {
            return Err(fail("Material view limit"));
        }
        let gpu = GPU
            .with(|v| v.borrow().clone())
            .ok_or_else(|| fail("WebGPU not initialized"))?;
        if !(1..=512).contains(&canvas.width()) || !(1..=512).contains(&canvas.height()) {
            return Err(fail("Invalid canvas dimensions"));
        }
        let surface = gpu
            .instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
            .map_err(|_| fail("Canvas unavailable"))?;
        let mut config = surface
            .get_default_config(&gpu.adapter, canvas.width(), canvas.height())
            .ok_or_else(|| fail("Canvas format unavailable"))?;
        let caps = surface.get_capabilities(&gpu.adapter);
        config.format = caps
            .formats
            .into_iter()
            .find(|f| !f.is_srgb())
            .ok_or_else(|| fail("Canvas format unavailable"))?;
        config.alpha_mode = wgpu::CompositeAlphaMode::PreMultiplied;
        surface.configure(&gpu.device, &config);
        let renderer = Renderer::with_format(
            &gpu.device,
            &gpu.queue,
            config.width,
            config.height,
            config.format,
        );
        VIEWS.with(|v| v.set(v.get() + 1));
        Ok(Self {
            gpu,
            surface,
            renderer,
            label: None,
            pending: Arc::new(AtomicBool::new(false)),
        })
    }
    pub fn draw(&mut self, raw: &str) -> Result<bool, JsValue> {
        if self.pending.load(Ordering::Relaxed) {
            return Ok(false);
        }
        if self.gpu.failed.load(Ordering::Relaxed) {
            return Err(fail("Material renderer unavailable"));
        }
        if raw.len() > 8192 {
            return Err(fail("Invalid material frame"));
        }
        let frame: Frame = serde_json::from_str(raw).map_err(|_| fail("Invalid material frame"))?;
        if frame.font > 1
            || frame.label.len() > 4096
            || !frame.progress.is_finite()
            || !(0. ..=1.).contains(&frame.progress)
            || !frame.style.iter().all(|v| v.is_finite())
        {
            return Err(fail("Invalid material frame"));
        }
        let object = match frame.kind {
            0 => Object::Die,
            1 => Object::Coin,
            2 => Object::Card,
            _ => return Err(fail("Invalid object")),
        };
        let die = if object == Object::Die {
            Die::ALL
                .into_iter()
                .find(|d| d.sides() == frame.sides as usize)
                .ok_or_else(|| fail("Invalid die"))?
        } else {
            Die::D6
        };
        let mut pose = if object == Object::Card {
            crate::physics::Pose {
                position: glam::Vec3::ZERO,
                rotation: glam::Quat::IDENTITY,
            }
        } else {
            result_pose(object, die, frame.face, frame.progress)
                .ok_or_else(|| fail("Invalid result"))?
        };
        if let Some(q) = frame.rotation {
            let q = glam::Quat::from_array(q);
            if !q.is_finite() || (q.length() - 1.).abs() > 0.01 {
                return Err(fail("Invalid pose"));
            }
            pose.rotation = q.normalize();
            pose.position = glam::Vec3::ZERO;
        }
        if self.label.as_ref() != Some(&(frame.font, frame.label.clone())) {
            self.renderer.set_label(
                &self.gpu.queue,
                &frame.label,
                if frame.font == 0 {
                    Lettering::Newsreader
                } else {
                    Lettering::Sans
                },
            );
            self.label = Some((frame.font, frame.label.clone()));
        }
        let p = frame.style;
        if !p[..3]
            .iter()
            .all(|v| (0. ..=16777215.).contains(v) && v.fract() == 0.)
            || !(0. ..=2.).contains(&p[11])
        {
            return Err(fail("Invalid appearance"));
        }
        let mut scene = Scene {
            object,
            die,
            pose: Some(pose),
            transparent: true,
            orthographic: object != Object::Card,
            samples: if frame.progress < 1. { 1 } else { 4 },
            numbering: if object == Object::Die {
                match frame.label.as_str() {
                    "tens" => 1,
                    "units" => 2,
                    _ => 0,
                }
            } else {
                0
            },
            conversation: Some(rgb(frame.accent)),
            global: rgb(frame.accent),
            mode: match p[11] as u32 {
                0 => Mode::Personalized,
                1 => Mode::Global,
                _ => Mode::Conversation,
            },
            zoom: if object == Object::Card { 4.0 } else { 2.7 },
            ..Default::default()
        };
        let a = &mut scene.appearances[object as usize];
        a.color = rgb(p[0] as u32);
        a.second = rgb(p[1] as u32);
        a.ink = rgb(p[2] as u32);
        a.roughness = p[3];
        a.transmission = p[4];
        a.absorption = p[5];
        a.ior = p[6];
        a.roundness = p[7];
        a.engraving = p[8];
        a.inclusions = p[9];
        scene.border = p[10].clamp(0., 2.) as u32;
        scene.texture_style = p[12].clamp(0., 3.) as u32;
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f)
            | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(false)
            }
            _ => return Err(fail("Canvas unavailable")),
        };
        self.renderer.render_to(
            &self.gpu.device,
            &self.gpu.queue,
            &scene,
            &frame.texture.create_view(&Default::default()),
        );
        self.gpu.queue.present(frame);
        self.pending.store(true, Ordering::Relaxed);
        let pending = self.pending.clone();
        self.gpu
            .queue
            .on_submitted_work_done(move || pending.store(false, Ordering::Relaxed));
        Ok(true)
    }
}

#[wasm_bindgen]
pub fn material_record(raw: &str) -> Result<String, JsValue> {
    if raw.len() > 8192 {
        return Err(fail("Invalid flight"));
    }
    let data: Vec<f32> = serde_json::from_str(raw).map_err(|_| fail("Invalid flight"))?;
    let output = crate::timeline::record_packed(&data).ok_or_else(|| fail("Invalid flight"))?;
    serde_json::to_string(&output).map_err(|_| fail("Invalid flight"))
}
#[wasm_bindgen]
pub fn material_extent(sides: u32, face: u32, raw: &str, outgoing: bool) -> Result<f32, JsValue> {
    if sides == 0 {
        return Ok(0.972);
    }
    let die = Die::ALL
        .into_iter()
        .find(|d| d.sides() == sides as usize)
        .ok_or_else(|| fail("Invalid die"))?;
    let mut q = result_pose(Object::Die, die, face, 1.)
        .ok_or_else(|| fail("Invalid result"))?
        .rotation;
    if !raw.is_empty() {
        if raw.len() > 256 {
            return Err(fail("Invalid pose"));
        }
        let values: [f32; 4] = serde_json::from_str(raw).map_err(|_| fail("Invalid pose"))?;
        q = glam::Quat::from_array(values);
        if !q.is_finite() || (q.length() - 1.).abs() > 0.01 {
            return Err(fail("Invalid pose"));
        }
    }
    Ok(die
        .hull()
        .vertices
        .iter()
        .map(|v| (q * *v).x * if outgoing { 1. } else { -1. })
        .fold(0., f32::max))
}

impl Drop for MaterialView {
    fn drop(&mut self) {
        VIEWS.with(|v| v.set(v.get().saturating_sub(1)));
    }
}
