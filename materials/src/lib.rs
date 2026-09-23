#![forbid(unsafe_code)]

use ab_glyph::{point, Font, FontRef, PxScale, ScaleFont};
use bytemuck::{Pod, Zeroable};
use glam::{Quat, Vec3};
use wgpu::util::DeviceExt;
#[cfg(all(not(target_os = "android"), not(target_arch = "wasm32")))]
pub mod app;
pub mod geometry;
pub mod performance;
pub mod physics;
pub mod presentation;
pub mod timeline;
use geometry::Die;
pub const SANS: &[u8] =
    include_bytes!("../../shared/src/commonMain/composeResources/font/google_sans_flex.ttf");
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Lettering {
    Newsreader,
    Sans,
}
impl Lettering {
    fn data(self) -> &'static [u8] {
        match self {
            Self::Newsreader => FONT,
            Self::Sans => SANS,
        }
    }
}

pub const FONT: &[u8] =
    include_bytes!("../../shared/src/commonMain/composeResources/font/newsreader.ttf");
const LOGO: &[u8] =
    include_bytes!("../../shared/src/commonMain/composeResources/drawable/sigil_dark.svg");

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Object {
    Die,
    Card,
    Coin,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    Personalized,
    Global,
    Conversation,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Appearance {
    pub color: [f32; 3],
    pub second: [f32; 3],
    pub ink: [f32; 3],
    pub roughness: f32,
    pub transmission: f32,
    pub absorption: f32,
    pub ior: f32,
    pub roundness: f32,
    pub engraving: f32,
    pub inclusions: f32,
}
impl Appearance {
    pub fn for_object(object: Object) -> Self {
        let (color, second, ink) = match object {
            Object::Die => (
                [0.44, 0.22, 0.73],
                [0.484, 0.352, 0.658],
                [0.95, 0.82, 0.51],
            ),
            Object::Card => ([0.19, 0.16, 0.28], [0.37, 0.22, 0.46], [0.89, 0.78, 0.53]),
            Object::Coin => ([0.27, 0.27, 0.26], [0.27, 0.27, 0.26], [1.0, 0.8, 0.5]),
        };
        Self {
            color,
            second,
            ink,
            roughness: 0.19,
            transmission: 0.85,
            absorption: 2.4,
            ior: 1.49,
            roundness: if object == Object::Die { 0.055 } else { 0.12 },
            engraving: 0.7,
            inclusions: 0.3,
        }
    }
}
#[derive(Clone)]
pub struct Scene {
    pub die: Die,
    pub border: u32,
    pub texture_style: u32,
    pub throw: Option<std::sync::Arc<physics::Throw>>,
    pub object: Object,
    pub appearances: [Appearance; 3],
    pub mode: Mode,
    pub global: [f32; 3],
    pub conversation: Option<[f32; 3]>,
    pub phase: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub light: f32,
    pub exposure: f32,
    pub zoom: f32,
    pub dark: bool,
    pub reduced: bool,
    pub samples: u32,
    pub pose: Option<physics::Pose>,
    pub backdrop: Option<[f32; 3]>,
    pub transparent: bool,
    pub orthographic: bool,
    pub numbering: u32,
}
impl Default for Scene {
    fn default() -> Self {
        Self {
            die: Die::D6,
            border: 1,
            texture_style: 0,
            throw: None,
            object: Object::Die,
            appearances: [
                Appearance::for_object(Object::Die),
                Appearance::for_object(Object::Card),
                Appearance::for_object(Object::Coin),
            ],
            mode: Mode::Personalized,
            global: [0.23, 0.42, 0.56],
            conversation: Some([0.51, 0.25, 0.41]),
            phase: 1.,
            yaw: 0.,
            pitch: 0.,
            light: 0.,
            exposure: 1.,
            zoom: 2.1,
            dark: true,
            reduced: false,
            samples: 4,
            pose: None,
            backdrop: None,
            transparent: false,
            orthographic: false,
            numbering: 0,
        }
    }
}
fn bounded(value: f32, min: f32, max: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}
fn linear(rgb: [f32; 3]) -> [f32; 3] {
    rgb.map(|v| {
        let v = bounded(v, 0., 1., 0.5);
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    })
}
impl Scene {
    pub fn palette(&self) -> ([f32; 3], [f32; 3]) {
        let a = &self.appearances[self.object as usize];
        let accent = match self.mode {
            Mode::Personalized => return (a.color, a.second),
            Mode::Global => self.global,
            Mode::Conversation => self.conversation.unwrap_or(self.global),
        };
        if self.object == Object::Coin {
            // An accent only tints the dark field; the metal stays the coin's own.
            let field = [0, 1, 2].map(|i| 0.27 + (accent[i] - 0.27) * 0.14);
            return (field, field);
        }
        (accent, accent.map(|v| (v * 0.6 + 0.22).clamp(0., 1.)))
    }
    pub fn duration(&self) -> f32 {
        if let Some(t) = &self.throw {
            return t.duration();
        }
        2.6
    }
    pub fn params(&self, width: u32, height: u32) -> Params {
        let a = &self.appearances[self.object as usize];
        let (body, second) = self.palette();
        let phase = if self.reduced {
            1.
        } else {
            bounded(self.phase, 0., 1., 1.)
        };
        let mut pos = Vec3::ZERO;
        let mut rotation = match self.object {
            Object::Die => Quat::from_euler(glam::EulerRot::YXZ, -0.52, 0.37, 0.),
            Object::Card => {
                pos.y = (phase * std::f32::consts::PI).sin() * 0.2;
                Quat::from_rotation_y(std::f32::consts::PI * (1. - smooth((phase - 0.35) / 0.5)))
            }
            Object::Coin => Quat::from_rotation_y(-0.20) * Quat::from_rotation_x(-0.32),
        };
        rotation = Quat::from_rotation_y(bounded(self.yaw, -20., 20., 0.))
            * Quat::from_rotation_x(bounded(self.pitch, -20., 20., 0.))
            * rotation;
        if let Some(t) = &self.throw {
            let pose = t.sample(phase);
            pos = pose.position;
            rotation = pose.rotation;
        }
        if let Some(pose) = self.pose {
            rotation = pose.rotation;
            pos = pose.position;
        }
        let hull = self.die.hull();
        let mut faces = [[[0.; 4]; 3]; 30];
        faces[..hull.faces.len()].copy_from_slice(&hull.faces);
        Params {
            backdrop: self
                .backdrop
                .map(|c| {
                    [
                        bounded(c[0], 0., 1., 0.),
                        bounded(c[1], 0., 1., 0.),
                        bounded(c[2], 0., 1., 0.),
                        if self.transparent { -1. } else { 1. },
                    ]
                })
                .unwrap_or([0., 0., 0., if self.transparent { -1. } else { 0. }]),
            faces,
            labels: *self.die.labels(),
            options: [
                hull.faces.len() as f32,
                if self.numbering > 0 {
                    1. + self.numbering.min(2) as f32
                } else {
                    f32::from(self.die != Die::D6)
                },
                self.border.min(2) as f32,
                if self.orthographic {
                    2.
                } else {
                    f32::from(self.throw.is_some())
                },
            ],
            obstacles: physics::OBSTACLES,
            viewport: [
                width as f32,
                height as f32,
                self.object as u32 as f32,
                f32::from(self.dark),
            ],
            body: rgba(linear(body)),
            secondary: rgba(linear(second)),
            ink: rgba(linear(a.ink)),
            material: [
                bounded(a.roughness, 0.045, 0.9, 0.2),
                bounded(a.transmission, 0., 1., 0.8),
                bounded(a.absorption, 0., 8., 2.),
                bounded(a.ior, 1., 1.8, 1.49),
            ],
            details: [
                bounded(a.roundness, 0.025, 0.28, 0.12),
                bounded(a.engraving, 0., 1., 0.7),
                bounded(a.inclusions, 0., 1., 0.3),
                self.texture_style.min(3) as f32,
            ],
            rotation: rotation.to_array(),
            position: [
                pos.x,
                pos.y,
                pos.z,
                if self.throw.is_some() {
                    physics::SCALE
                } else {
                    1.
                },
            ],
            lighting: [
                bounded(self.light, -10., 10., 0.),
                bounded(self.exposure, 0.25, 3., 1.),
                bounded(self.zoom, 1., 3.5, 2.1),
                if self.samples >= 4 { 4. } else { 1. },
            ],
        }
    }
}
fn smooth(v: f32) -> f32 {
    let v = v.clamp(0., 1.);
    v * v * (3. - 2. * v)
}
fn rgba(v: [f32; 3]) -> [f32; 4] {
    [v[0], v[1], v[2], 1.]
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Params {
    viewport: [f32; 4],
    body: [f32; 4],
    secondary: [f32; 4],
    ink: [f32; 4],
    material: [f32; 4],
    details: [f32; 4],
    rotation: [f32; 4],
    position: [f32; 4],
    lighting: [f32; 4],
    options: [f32; 4],
    backdrop: [f32; 4],
    faces: [[[f32; 4]; 3]; 30],
    labels: [[f32; 4]; 30],
    obstacles: [[[f32; 4]; 2]; 7],
}

pub struct RendererResources {
    pipeline: wgpu::RenderPipeline,
    logo: wgpu::Texture,
    orbit: wgpu::Texture,
    sampler: wgpu::Sampler,
    format: wgpu::TextureFormat,
}
impl RendererResources {
    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let logo = mask_texture(device);
        write_mask(queue, &logo, &logo_mask(LOGO));
        let orbit = mask_texture(device);
        write_mask(
            queue,
            &orbit,
            &logo_mask(include_bytes!("../assets/orbit.svg")),
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bounded material study"),
            source: wgpu::ShaderSource::Wgsl(include_str!("material.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("material study"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            logo,
            orbit,
            sampler,
            format,
        }
    }
}

pub struct Renderer {
    profiler: Option<performance::Profiler>,
    pipeline: wgpu::RenderPipeline,
    bindings: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    label: wgpu::Texture,
    digits: wgpu::Texture,
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    format: wgpu::TextureFormat,
    pub width: u32,
    pub height: u32,
}
fn target(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("material preview"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}
fn mask_texture(device: &wgpu::Device) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("trusted artwork mask"),
        size: wgpu::Extent3d {
            width: 512,
            height: 512,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}
fn write_mask(queue: &wgpu::Queue, texture: &wgpu::Texture, data: &[u8]) {
    queue.write_texture(
        texture.as_image_copy(),
        data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(512),
            rows_per_image: Some(512),
        },
        wgpu::Extent3d {
            width: 512,
            height: 512,
            depth_or_array_layers: 1,
        },
    );
}
fn logo_mask(svg: &[u8]) -> Vec<u8> {
    let tree =
        resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default()).expect("bundled logo");
    let mut pix = resvg::tiny_skia::Pixmap::new(512, 512).expect("fixed mask size");
    let scale = 380. / tree.size().height().max(tree.size().width());
    let x = (512. - tree.size().width() * scale) / 2.;
    let y = (512. - tree.size().height() * scale) / 2.;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_translate(x, y).pre_scale(scale, scale),
        &mut pix.as_mut(),
    );
    pix.data().as_chunks::<4>().0.iter().map(|p| p[3]).collect()
}
pub fn label_mask(text: &str) -> Vec<u8> {
    text_mask(text, Lettering::Newsreader)
}
fn text_mask(text: &str, lettering: Lettering) -> Vec<u8> {
    let font = FontRef::try_from_slice(lettering.data()).expect("bundled font");
    let text: String = text
        .chars()
        .filter(|c| !c.is_control() || *c == ' ')
        .take(80)
        .collect();
    let mut chosen = (Vec::new(), 36.);
    for size in (26..=90).rev() {
        let scaled = font.as_scaled(PxScale::from(size as f32));
        let mut lines = vec![String::new()];
        for word in text.split_whitespace() {
            let line = lines.last_mut().unwrap();
            let candidate = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if text_width(&scaled, &candidate) > 365. && !line.is_empty() {
                lines.push(word.to_owned());
            } else {
                *line = candidate;
            }
        }
        if lines.len() <= 4 && lines.iter().all(|l| text_width(&scaled, l) <= 365.) {
            chosen = (lines, size as f32);
            break;
        }
    }
    if chosen.0.is_empty() {
        chosen = (vec!["Label too long".into()], 28.);
    }
    let scaled = font.as_scaled(PxScale::from(chosen.1));
    let height = chosen.1 * 1.35;
    let top = 256. - chosen.0.len() as f32 * height / 2.;
    let mut data = vec![0u8; 512 * 512];
    for (i, line) in chosen.0.iter().enumerate() {
        let mut x = (512. - text_width(&scaled, line)) / 2.;
        let mut previous = None;
        for c in line.chars() {
            let id = scaled.glyph_id(c);
            if let Some(prev) = previous {
                x += scaled.kern(prev, id);
            }
            let glyph = id.with_scale_and_position(
                chosen.1,
                point(x, top + i as f32 * height + scaled.ascent()),
            );
            if let Some(outline) = font.outline_glyph(glyph) {
                let bounds = outline.px_bounds();
                outline.draw(|gx, gy, v| {
                    let px = gx as i32 + bounds.min.x as i32;
                    let py = gy as i32 + bounds.min.y as i32;
                    if (0..512).contains(&px) && (0..512).contains(&py) {
                        let cell = &mut data[py as usize * 512 + px as usize];
                        *cell = (*cell).max((v * 255.) as u8);
                    }
                });
            }
            x += scaled.h_advance(id);
            previous = Some(id);
        }
    }
    data
}
fn text_width<F: Font>(font: &impl ScaleFont<F>, text: &str) -> f32 {
    let mut total = 0.;
    let mut last = None;
    for c in text.chars() {
        let id = font.glyph_id(c);
        if let Some(p) = last {
            total += font.kern(p, id);
        }
        total += font.h_advance(id);
        last = Some(id);
    }
    total
}
fn digit_mask(lettering: Lettering) -> &'static [u8] {
    static SERIF: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    static SANS: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    match lettering {
        Lettering::Newsreader => SERIF.get_or_init(|| raster_digits(Lettering::Newsreader)),
        Lettering::Sans => SANS.get_or_init(|| raster_digits(Lettering::Sans)),
    }
}
fn raster_digits(lettering: Lettering) -> Vec<u8> {
    let mut atlas = vec![0; 512 * 512];
    for i in 0..41 {
        let text = match i {
            30 => "0".to_owned(),
            31..=39 => ((i - 30) * 10).to_string(),
            40 => "00".to_owned(),
            _ => (i + 1).to_string(),
        };
        let mut mask = text_mask(&text, lettering);
        let ink: Vec<_> = mask
            .iter()
            .enumerate()
            .filter(|(_, v)| **v > 16)
            .map(|(p, _)| (p % 512, p / 512))
            .collect();
        let left = ink.iter().map(|p| p.0).min().unwrap();
        let right = ink.iter().map(|p| p.0).max().unwrap();
        let top = ink.iter().map(|p| p.1).min().unwrap();
        let bottom = ink.iter().map(|p| p.1).max().unwrap();
        if i == 5 || i == 8 {
            for y in bottom + 5..bottom + 8 {
                for x in left..=right {
                    mask[y * 512 + x] = 255;
                }
            }
        }
        for y in 0..73 {
            for x in 0..73 {
                let scale = (34. / (bottom - top + 1) as f32).min(58. / (right - left + 1) as f32);
                let sx = ((left + right) as f32 / 2. + (x as f32 - 36.) / scale)
                    .round()
                    .clamp(1., 510.) as usize;
                let sy = ((top + bottom) as f32 / 2. + (y as f32 - 36.) / scale)
                    .round()
                    .clamp(1., 510.) as usize;
                atlas[(i / 7 * 73 + y) * 512 + i % 7 * 73 + x] = (sy - 1..=sy + 1)
                    .flat_map(|yy| (sx - 1..=sx + 1).map(move |xx| (yy, xx)))
                    .map(|(yy, xx)| mask[yy * 512 + xx])
                    .max()
                    .unwrap();
            }
        }
    }
    atlas
}
impl Renderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32) -> Self {
        Self::with_format(
            device,
            queue,
            width,
            height,
            wgpu::TextureFormat::Rgba8Unorm,
        )
    }
    pub fn with_format(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Self {
        let resources = RendererResources::new(device, queue, format);
        Self::with_resources(device, queue, width, height, &resources)
    }
    pub fn with_resources(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        resources: &RendererResources,
    ) -> Self {
        let format = resources.format;
        let width = width.clamp(1, 1600);
        let height = height.clamp(1, 1200);
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("bounded appearance"),
            contents: bytemuck::bytes_of(&Scene::default().params(width, height)),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let pipeline = resources.pipeline.clone();
        let logo = &resources.logo;
        let orbit = &resources.orbit;
        let sampler = &resources.sampler;
        let digits = mask_texture(device);
        write_mask(queue, &digits, digit_mask(Lettering::Newsreader));
        let label = mask_texture(device);
        write_mask(queue, &label, &label_mask("The museum"));
        let texture = target(device, width, height, format);
        let view = texture.create_view(&Default::default());
        let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("material bindings"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &logo.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        &label.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(
                        &orbit.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(
                        &digits.create_view(&Default::default()),
                    ),
                },
            ],
        });
        Self {
            profiler: None,
            pipeline,
            bindings,
            uniform,
            label,
            digits,
            texture,
            view,
            format,
            width,
            height,
        }
    }
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) -> bool {
        let width = width.clamp(1, 1600);
        let height = height.clamp(1, 1200);
        if width == self.width && height == self.height {
            return false;
        }
        self.width = width;
        self.height = height;
        self.texture = target(device, width, height, self.format);
        self.view = self.texture.create_view(&Default::default());
        true
    }
    pub fn set_label(&self, queue: &wgpu::Queue, text: &str, lettering: Lettering) {
        write_mask(queue, &self.label, &text_mask(text, lettering));
        write_mask(queue, &self.digits, digit_mask(lettering));
    }
    pub fn render(&self, device: &wgpu::Device, queue: &wgpu::Queue, scene: &Scene) {
        self.render_to(device, queue, scene, &self.view);
    }
    pub fn render_to(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &Scene,
        view: &wgpu::TextureView,
    ) {
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&scene.params(self.width, self.height)),
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("material preview"),
                timestamp_writes: self.profiler.as_ref().map(|p| p.writes()),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bindings, &[]);
            pass.draw(0..3, 0..1);
        }
        if let Some(p) = &self.profiler {
            p.resolve(&mut encoder);
        }
        queue.submit([encoder.finish()]);
    }
    pub fn save(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        path: &std::path::Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let row = (self.width * 4).div_ceil(256) * 256;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("preview readback"),
            size: row as u64 * self.height as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device.poll(wgpu::PollType::wait_indefinitely())?;
        rx.recv()??;
        let data = buffer.slice(..).get_mapped_range()?;
        let pixels: Vec<u8> = data
            .chunks_exact(row as usize)
            .flat_map(|r| r[..self.width as usize * 4].iter().copied())
            .collect();
        let mut encoder = png::Encoder::new(std::fs::File::create(path)?, self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&pixels)?;
        drop(data);
        buffer.unmap();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modes_do_not_mutate_authored_materials() {
        let mut s = Scene::default();
        let original = s.appearances.clone();
        s.mode = Mode::Conversation;
        s.conversation = None;
        assert_eq!(s.palette().0, s.global);
        s.conversation = Some([0.1, 0.2, 0.3]);
        assert_eq!(s.palette().0, [0.1, 0.2, 0.3]);
        s.mode = Mode::Personalized;
        assert_eq!(s.appearances, original);
        assert_eq!(s.palette().0, original[0].color);
    }
    #[test]
    fn malformed_parameters_cannot_reach_shader() {
        let mut s = Scene {
            phase: f32::NAN,
            yaw: f32::INFINITY,
            ..Scene::default()
        };
        s.appearances[0].ior = 0.;
        s.appearances[0].color = [f32::NAN, -1., 4.];
        let p = s.params(800, 600);
        assert!(bytemuck::cast_slice::<Params, f32>(&[p])
            .iter()
            .all(|v| v.is_finite()));
        assert!(p.material[3] >= 1.);
    }
    #[test]
    fn reduced_motion_has_same_terminal_pose() {
        let mut s = Scene::default();
        for object in [Object::Die, Object::Card, Object::Coin] {
            s.object = object;
            s.phase = 1.;
            let expected = s.params(800, 600);
            s.phase = 0.23;
            s.reduced = true;
            let actual = s.params(800, 600);
            assert_eq!(expected.rotation, actual.rotation);
            assert_eq!(expected.position, actual.position);
            s.reduced = false;
        }
    }
    #[test]
    fn labels_are_bounded_and_visible() {
        for s in [
            "The museum",
            "A quiet afternoon at the botanical garden",
            "éclair",
            &"x".repeat(20000),
        ] {
            let pixels = label_mask(s);
            assert_eq!(pixels.len(), 512 * 512);
            assert!(pixels.iter().any(|v| *v > 0));
        }
    }
    #[test]
    fn both_fonts_and_each_number_are_rasterized() {
        let serif = digit_mask(Lettering::Newsreader);
        let sans = digit_mask(Lettering::Sans);
        assert_ne!(serif, sans);
        for atlas in [&serif, &sans] {
            for i in 0..41 {
                assert!((0..73)
                    .any(|y| (0..73).any(|x| atlas[(i / 7 * 73 + y) * 512 + i % 7 * 73 + x] > 0)));
                if i != 5 && i != 8 {
                    let pixels: Vec<_> = (0..73)
                        .flat_map(|y| (0..73).map(move |x| (x, y)))
                        .filter(|(x, y)| atlas[(i / 7 * 73 + y) * 512 + i % 7 * 73 + x] > 32)
                        .collect();
                    for axis in [0, 1] {
                        let values: Vec<_> = pixels
                            .iter()
                            .map(|p| if axis == 0 { p.0 } else { p.1 })
                            .collect();
                        assert!(
                            (values.iter().min().unwrap() + values.iter().max().unwrap())
                                .abs_diff(72)
                                <= 2,
                            "glyph {i} is not centered"
                        );
                    }
                }
            }
        }
        assert_ne!(
            text_mask("The museum", Lettering::Newsreader),
            text_mask("The museum", Lettering::Sans)
        );
    }
}

#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(target_arch = "wasm32")]
mod web_worker;

#[cfg(all(test, not(target_os = "android"), not(target_arch = "wasm32")))]
#[test]
#[ignore = "requires a local Vulkan adapter; reports renderer startup latency"]
fn renderer_startup_benchmark() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .unwrap();
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    eprintln!("adapter={:?}", adapter.get_info());
    let mut times = Vec::new();
    for _ in 0..6 {
        let started = std::time::Instant::now();
        let renderer =
            Renderer::with_format(&device, &queue, 256, 256, wgpu::TextureFormat::Rgba8Unorm);
        queue.submit([]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        times.push(started.elapsed().as_secs_f64() * 1000.);
        drop(renderer);
    }
    eprintln!("separate pipeline startup ms={times:?}");
    let resources = RendererResources::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let mut times = Vec::new();
    for _ in 0..6 {
        let started = std::time::Instant::now();
        let renderer = Renderer::with_resources(&device, &queue, 256, 256, &resources);
        queue.submit([]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        times.push(started.elapsed().as_secs_f64() * 1000.);
        drop(renderer);
    }
    eprintln!("shared resources startup ms={times:?}");
    let a = Renderer::with_resources(&device, &queue, 96, 128, &resources);
    let b = Renderer::with_resources(&device, &queue, 96, 128, &resources);
    let mut scene = Scene::default();
    scene.object = Object::Card;
    scene.yaw = 0.;
    scene.pitch = 0.;
    let dir = std::env::temp_dir().join(format!("sigil-renderer-isolation-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for object in [Object::Card, Object::Die] {
        scene.object = object;
        scene.die = Die::D20;
        let snapshot = |renderer: &Renderer, name: &str| {
            renderer.render(&device, &queue, &scene);
            let path = dir.join(name);
            renderer.save(&device, &queue, &path).unwrap();
            std::fs::read(path).unwrap()
        };
        a.set_label(&queue, "ALPHA", Lettering::Newsreader);
        let prior = snapshot(&a, "before.png");
        b.set_label(&queue, "BRAVO", Lettering::Sans);
        assert_eq!(
            prior,
            snapshot(&a, "after.png"),
            "another view changed this view's label or font"
        );
        assert_ne!(
            prior,
            snapshot(&b, "other.png"),
            "independent labels must render differently"
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}
