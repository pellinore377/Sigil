#![forbid(unsafe_code)]
use crate::{
    geometry::Die, physics::Throw, Appearance, Lettering, Mode, Object, Renderer, Scene, FONT,
};
use eframe::{egui, egui_wgpu};
use std::time::Instant;

struct Lab {
    controls_open: bool,
    render_scale: f32,
    gpu: egui_wgpu::RenderState,
    renderer: Renderer,
    texture: egui::TextureId,
    scene: Scene,
    running: bool,
    last: Instant,
    label: String,
    redraw: bool,
    adapter: String,
    capture: Option<std::path::PathBuf>,
    capture_requested: bool,
    lettering: Lettering,
    seed: u32,
    pending: Option<std::sync::mpsc::Receiver<Throw>>,
}
impl Lab {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let gpu = cc.wgpu_render_state.clone().expect("wgpu required");
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "Newsreader".into(),
            egui::FontData::from_static(FONT).into(),
        );
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "Newsreader".into());
        cc.egui_ctx.set_fonts(fonts);
        let mut style = (*cc.egui_ctx.global_style()).clone();
        style.visuals.override_text_color = Some(egui::Color32::from_gray(220));
        style.visuals.selection.bg_fill = egui::Color32::from_gray(65);
        style.visuals.selection.stroke = egui::Stroke::new(1., egui::Color32::from_gray(220));
        for widget in [
            &mut style.visuals.widgets.inactive,
            &mut style.visuals.widgets.hovered,
            &mut style.visuals.widgets.active,
        ] {
            widget.corner_radius = egui::CornerRadius::same(10);
        }
        style.spacing.item_spacing = egui::vec2(10., 12.);
        style.spacing.button_padding = egui::vec2(14., 9.);
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(18.));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(18.));
        style
            .text_styles
            .insert(egui::TextStyle::Heading, egui::FontId::proportional(30.));
        cc.egui_ctx.set_global_style(style);
        let renderer = Renderer::new(&gpu.device, &gpu.queue, 256, 256);
        let texture = gpu.renderer.write().register_native_texture(
            &gpu.device,
            &renderer.view,
            wgpu::FilterMode::Linear,
        );
        let info = gpu.adapter.get_info();
        let adapter = format!("{} · {:?}", info.name, info.backend);
        Self {
            controls_open: false,
            render_scale: 1.,
            gpu,
            renderer,
            texture,
            scene: Scene::default(),
            running: false,
            last: Instant::now(),
            label: "The museum".into(),
            redraw: true,
            adapter,
            capture: std::env::args()
                .skip_while(|s| s != "--capture-ui")
                .nth(1)
                .map(Into::into),
            capture_requested: false,
            lettering: Lettering::Newsreader,
            seed: 1,
            pending: None,
        }
    }
    fn controls(&mut self, ui: &mut egui::Ui) {
        if self.pending.is_some() {
            ui.disable();
        }
        ui.heading("Materials");
        ui.label("A SigilText rendering experiment");
        ui.separator();
        ui.horizontal(|ui| {
            for (object, name) in [
                (Object::Die, "Die"),
                (Object::Card, "Card"),
                (Object::Coin, "Coin"),
            ] {
                if ui
                    .selectable_value(&mut self.scene.object, object, name)
                    .changed()
                {
                    self.scene.phase = 1.;
                    self.scene.yaw = 0.;
                    self.scene.pitch = 0.;
                    self.running = false;
                    self.scene.throw = None;
                    self.pending = None;
                    self.redraw = true;
                }
            }
        });
        if self.scene.object == Object::Die {
            ui.horizontal_wrapped(|ui| {
                for die in Die::ALL {
                    if ui
                        .selectable_value(&mut self.scene.die, die, format!("d{}", die.sides()))
                        .changed()
                    {
                        self.scene.throw = None;
                        self.pending = None;
                        self.scene.phase = 1.;
                        self.scene.yaw = 0.;
                        self.scene.pitch = 0.;
                        self.running = false;
                        self.redraw = true;
                    }
                }
            });
        }
        if self.scene.object != Object::Coin {
            ui.horizontal(|ui| {
                for (font, name) in [
                    (Lettering::Newsreader, "Newsreader"),
                    (Lettering::Sans, "Google Sans Flex"),
                ] {
                    if ui
                        .selectable_value(&mut self.lettering, font, name)
                        .changed()
                    {
                        self.renderer.set_label(&self.gpu.queue, &self.label, font);
                        self.redraw = true;
                    }
                }
            });
        }
        if self.scene.object != Object::Card {
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(self.scene.throw.is_none(), "Material view")
                    .clicked()
                {
                    self.scene.throw = None;
                    self.pending = None;
                    self.scene.phase = 1.;
                    self.running = false;
                    self.redraw = true;
                }
                if ui
                    .add_enabled(
                        self.pending.is_none(),
                        egui::Button::new(if self.scene.object == Object::Coin {
                            "Flip · top down"
                        } else {
                            "Roll · top down"
                        }),
                    )
                    .clicked()
                {
                    self.seed = self.seed.wrapping_add(1);
                    let (tx, rx) = std::sync::mpsc::channel();
                    let object = self.scene.object;
                    let die = self.scene.die;
                    let seed = self.seed;
                    std::thread::spawn(move || {
                        let _ = tx.send(Throw::new(object, die, seed));
                    });
                    self.pending = Some(rx);
                }
            });
            if self.pending.is_some() {
                ui.label("Simulating throw…");
            }
            if let Some(t) = &self.scene.throw {
                ui.small(format!(
                    "{} · {:.1}s replay",
                    if self.scene.phase >= 1. || self.scene.reduced {
                        t.result.as_str()
                    } else {
                        "In motion"
                    },
                    t.duration()
                ));
                ui.small("Sample bubbles and footer are fixed colliders. One object per throw.");
            }
        }
        ui.label("How objects appear to you");
        for (mode, name) in [
            (Mode::Personalized, "Personalized"),
            (Mode::Global, "Global accent"),
            (Mode::Conversation, "Conversation accent"),
        ] {
            self.redraw |= ui.radio_value(&mut self.scene.mode, mode, name).changed();
        }
        if self.scene.mode != Mode::Personalized {
            ui.horizontal(|ui| {
                ui.label("Global");
                self.redraw |= ui.color_edit_button_rgb(&mut self.scene.global).changed();
            });
            if self.scene.mode == Mode::Conversation {
                let mut custom = self.scene.conversation.is_some();
                if ui
                    .checkbox(&mut custom, "Conversation has its own accent")
                    .changed()
                {
                    self.scene.conversation = if custom {
                        Some([0.51, 0.25, 0.41])
                    } else {
                        None
                    };
                    self.redraw = true;
                }
                if let Some(color) = &mut self.scene.conversation {
                    self.redraw |= ui.color_edit_button_rgb(color).changed();
                } else {
                    ui.small("Inheriting global accent");
                }
            }
        }
        ui.separator();
        ui.label("My object");
        let a = &mut self.scene.appearances[self.scene.object as usize];
        ui.horizontal(|ui| {
            for (name, color, second) in [
                ("Amethyst", [0.44, 0.22, 0.73], [0.08, 0.56, 0.54]),
                ("Amber", [0.86, 0.48, 0.08], [0.96, 0.78, 0.26]),
                ("Ink", [0.10, 0.12, 0.15], [0.28, 0.32, 0.36]),
            ] {
                if ui.button(name).clicked() {
                    a.color = color;
                    a.second = second;
                    self.redraw = true;
                }
            }
        });
        for (name, color) in [
            ("Body", &mut a.color),
            ("Secondary", &mut a.second),
            ("Markings", &mut a.ink),
        ] {
            ui.horizontal(|ui| {
                ui.label(name);
                self.redraw |= ui.color_edit_button_rgb(color).changed();
            });
        }
        self.redraw |= ui
            .add(egui::Slider::new(&mut a.roughness, 0.045..=0.9).text("Roughness"))
            .changed();
        if self.scene.object == Object::Die {
            self.redraw |= ui
                .add(egui::Slider::new(&mut a.transmission, 0.0..=1.).text("Transmission"))
                .changed();
            self.redraw |= ui
                .add(egui::Slider::new(&mut a.absorption, 0.0..=8.).text("Color depth"))
                .changed();
            self.redraw |= ui
                .add(egui::Slider::new(&mut a.ior, 1.0..=1.8).text("Refraction index"))
                .changed();
            self.redraw |= ui
                .add(egui::Slider::new(&mut a.roundness, 0.025..=0.28).text("Rounded edges"))
                .changed();
            self.redraw |= ui
                .add(egui::Slider::new(&mut a.inclusions, 0.0..=1.).text("Inclusions"))
                .changed();
        }
        if self.scene.object != Object::Card {
            self.redraw |= ui
                .add(egui::Slider::new(&mut a.engraving, 0.0..=1.).text("Engraving depth"))
                .changed();
        }
        if self.scene.object == Object::Coin {
            ui.horizontal(|ui| {
                for (name, color) in [
                    ("Gold", [0.83, 0.62, 0.28]),
                    ("Silver", [0.85, 0.87, 0.9]),
                    ("Copper", [0.86, 0.48, 0.30]),
                ] {
                    if ui.button(name).clicked() {
                        a.color = color;
                        a.ink = color;
                        self.redraw = true;
                    }
                }
            });
            self.redraw |= ui
                .add(egui::Slider::new(&mut a.inclusions, 0.0..=1.).text("Surface wear"))
                .changed();
        }
        if self.scene.object == Object::Card {
            ui.label("Fixed sample choice");
            if ui
                .add(egui::TextEdit::singleline(&mut self.label).char_limit(80))
                .changed()
            {
                self.renderer
                    .set_label(&self.gpu.queue, &self.label, self.lettering);
                self.redraw = true;
            }
            if ui.button("Turn over").clicked() {
                self.scene.yaw += std::f32::consts::PI;
                self.redraw = true;
            }
            ui.horizontal(|ui| {
                for (border, name) in [(0, "Classic"), (1, "Petal"), (2, "Guilloché")] {
                    self.redraw |= ui
                        .selectable_value(&mut self.scene.border, border, name)
                        .changed();
                }
            });
        }
        if ui.button("Reset this material").clicked() {
            *a = Appearance::for_object(self.scene.object);
            self.redraw = true;
        }
        ui.separator();
        ui.label("Lighting & view");
        self.redraw |= ui
            .add(egui::Slider::new(&mut self.render_scale, 0.25..=1.).text("Render scale"))
            .changed();
        self.redraw |= ui
            .add(
                egui::Slider::new(
                    &mut self.scene.light,
                    -std::f32::consts::PI..=std::f32::consts::PI,
                )
                .text("Light angle"),
            )
            .changed();
        self.redraw |= ui
            .add(egui::Slider::new(&mut self.scene.exposure, 0.25..=3.).text("Exposure"))
            .changed();
        self.redraw |= ui
            .add(egui::Slider::new(&mut self.scene.zoom, 1.0..=3.5).text("Zoom"))
            .changed();
        self.redraw |= ui
            .checkbox(&mut self.scene.dark, "Dark background")
            .changed();
        ui.small("Built-in geometry and artwork. No remote assets, shaders or messaging data.");
    }
}
impl eframe::App for Lab {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Some(rx) = &self.pending {
            if let Ok(t) = rx.try_recv() {
                self.scene.throw = Some(std::sync::Arc::new(t));
                self.pending = None;
                self.scene.phase = if self.scene.reduced { 1. } else { 0. };
                self.running = !self.scene.reduced;
                self.redraw = true;
            } else {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(25));
            }
        }
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32().min(0.1);
        self.last = now;
        if self.running && !self.scene.reduced {
            self.scene.phase = (self.scene.phase + dt / self.scene.duration()).min(1.);
            self.redraw = true;
            self.running = self.scene.phase < 1.;
        }
        let compact = ui.available_width() < 720.;
        if compact {
            egui::Panel::top("materials").show(ui, |ui| {
                if ui.button("Materials").clicked() {
                    self.controls_open = !self.controls_open;
                }
            });
        }
        if !compact {
            egui::Panel::left("controls")
                .exact_size(340.)
                .resizable(false)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.controls(ui));
                });
        }
        if compact && self.controls_open {
            let mut open = true;
            egui::Window::new("Materials")
                .open(&mut open)
                .default_width(330.)
                .max_height(ui.available_height() * 0.8)
                .show(ui.ctx(), |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.controls(ui));
                });
            self.controls_open = open;
        }
        egui::CentralPanel::default().show(ui,|ui|{
            ui.heading("Sigil Materials");if !compact{ui.label(&self.adapter);}
            ui.horizontal_wrapped(|ui|{
                let replayable=self.scene.object==Object::Card||self.scene.throw.is_some();
                if ui.add_enabled(replayable,egui::Button::new("Replay")).clicked(){self.scene.phase=0.;self.running = !self.scene.reduced;self.redraw=true;}
                if ui.add_enabled(replayable&&self.scene.phase<1.&&!self.scene.reduced,egui::Button::new(if self.running{"Pause"}else{"Resume"})).clicked(){self.running = !self.running;}
                if ui.add_enabled(replayable,egui::Slider::new(&mut self.scene.phase,0.0..=1.).text("Progress")).changed(){self.running=false;self.redraw=true;}
                if ui.checkbox(&mut self.scene.reduced,"Reduced motion").changed(){self.running=false;self.scene.phase=1.;self.redraw=true;}
            });
            ui.small("Drag to inspect • Replay keeps the same sample result • Idle scenes stop rendering");
            let size=egui::vec2(ui.available_width().max(1.),(ui.available_height()-52.).max(1.));
            let scale=ui.ctx().pixels_per_point();
            let pixels=size*scale*self.render_scale;let fit=(1600./pixels.x).min(1200./pixels.y).min(1.);
            if self.renderer.resize(&self.gpu.device,(pixels.x*fit) as u32,(pixels.y*fit) as u32){self.gpu.renderer.write().update_egui_texture_from_wgpu_texture(&self.gpu.device,&self.renderer.view,wgpu::FilterMode::Linear,self.texture);self.redraw=true;}
            let response=ui.add(egui::Image::new((self.texture,size)).sense(egui::Sense::drag()));
            if response.dragged()&&self.scene.throw.is_none(){let delta=ui.input(|i|i.pointer.delta());self.scene.yaw+=delta.x*0.008;self.scene.pitch+=delta.y*0.008;self.redraw=true;}
            ui.label(if self.scene.throw.is_some(){"Recorded physics • Replay preserves the throw • Throw starts a new sample"}else{match self.scene.object {Object::Die=>"Drag to inspect every face • Throw opens the physics scene",Object::Card=>"Fixed choice • no reroll when replayed",Object::Coin=>"Heads: Sigil • Tails: Orbit • Drag to turn over"}});
            if self.redraw {self.renderer.render(&self.gpu.device,&self.gpu.queue,&self.scene);self.redraw=false;}
        });
        if self.running {
            ui.ctx().request_repaint();
        }
        if self.capture.is_some() && !self.capture_requested {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            self.capture_requested = true;
            ui.ctx().request_repaint();
        }
        let image = ui.input(|i| {
            i.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let (Some(image), Some(path)) = (image, self.capture.as_ref()) {
            let result = (|| -> Result<(), Box<dyn std::error::Error>> {
                let mut encoder = png::Encoder::new(
                    std::fs::File::create(path)?,
                    image.width() as u32,
                    image.height() as u32,
                );
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                let pixels: Vec<_> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                encoder.write_header()?.write_image_data(&pixels)?;
                Ok(())
            })();
            if let Err(error) = result {
                eprintln!("UI capture failed: {error}");
            }
            self.capture = None;
        }
    }
}
fn snapshots(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(path)?;
    let instance = wgpu::Instance::default();
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))?;
    println!("Adapter: {:?}", adapter.get_info());
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))?;
    let renderer = Renderer::new(&device, &queue, 1000, 800);
    let mut scene = Scene::default();
    for (object, name) in [
        (Object::Die, "die"),
        (Object::Card, "card"),
        (Object::Coin, "coin"),
    ] {
        scene.object = object;
        scene.yaw = 0.;
        scene.pitch = 0.;
        scene.phase = 1.;
        renderer.render(&device, &queue, &scene);
        renderer.save(
            &device,
            &queue,
            &std::path::Path::new(path).join(format!("{name}.png")),
        )?;
        scene.yaw = std::f32::consts::PI;
        renderer.render(&device, &queue, &scene);
        renderer.save(
            &device,
            &queue,
            &std::path::Path::new(path).join(format!("{name}-back.png")),
        )?;
        scene.yaw = 0.;
        let mut ms = Vec::new();
        for i in 0..32 {
            scene.phase = i as f32 / 31.;
            let start = Instant::now();
            renderer.render(&device, &queue, &scene);
            device.poll(wgpu::PollType::wait_indefinitely())?;
            ms.push(start.elapsed().as_secs_f64() * 1000.);
        }
        ms.sort_by(f64::total_cmp);
        println!(
            "{name}: synchronized render median {:0.2} ms, p95 {:0.2} ms ({}x{})",
            ms[16], ms[30], renderer.width, renderer.height
        );
    }
    scene = Scene::default();
    for die in Die::ALL {
        scene.die = die;
        for (font, name) in [
            (Lettering::Newsreader, "newsreader"),
            (Lettering::Sans, "sans"),
        ] {
            renderer.set_label(&queue, "The museum", font);
            renderer.render(&device, &queue, &scene);
            renderer.save(
                &device,
                &queue,
                &std::path::Path::new(path).join(format!("d{}-{name}.png", die.sides())),
            )?;
        }
    }
    for object in [Object::Die, Object::Coin] {
        scene = Scene {
            object,
            throw: Some(std::sync::Arc::new(Throw::new(object, Die::D6, 2))),
            ..Default::default()
        };
        for (phase, name) in [(0., "start"), (0.18, "bounce"), (1., "settled")] {
            scene.phase = phase;
            renderer.render(&device, &queue, &scene);
            renderer.save(
                &device,
                &queue,
                &std::path::Path::new(path).join(format!("physics-{object:?}-{name}.png")),
            )?;
        }
        println!(
            "Physics {object:?}: {}",
            scene.throw.as_ref().unwrap().result
        );
    }
    scene = Scene {
        object: Object::Card,
        ..Default::default()
    };
    for border in 0..3 {
        scene.border = border;
        scene.yaw = std::f32::consts::PI;
        renderer.render(&device, &queue, &scene);
        renderer.save(
            &device,
            &queue,
            &std::path::Path::new(path).join(format!("card-border-{border}.png")),
        )?;
    }
    Ok(())
}
pub fn desktop_main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|v| v == "--snapshots") {
        return snapshots(
            args.get(2)
                .map(String::as_str)
                .unwrap_or("/tmp/sigil-material-lab"),
        );
    }
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280., 880.])
            .with_min_inner_size([900., 640.]),
        ..Default::default()
    };
    run(options)?;
    Ok(())
}
pub fn run(options: eframe::NativeOptions) -> eframe::Result {
    eframe::run_native(
        "Sigil · Material playground",
        options,
        Box::new(move |cc| Ok(Box::new(Lab::new(cc)))),
    )
}
