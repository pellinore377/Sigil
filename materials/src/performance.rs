use crate::{geometry::Die, physics::Throw, Object, Renderer, Scene};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::Instant,
};

fn wait_gpu(device: &wgpu::Device) -> Result<(), String> {
    // Blocking waits timed out on the tested Android Vulkan driver; poll with a deadline.
    let start = Instant::now();
    loop {
        if device
            .poll(wgpu::PollType::Poll)
            .map_err(|e| e.to_string())?
            == wgpu::PollStatus::QueueEmpty
        {
            return Ok(());
        }
        if start.elapsed().as_secs() >= 30 {
            return Err("GPU submission did not finish within 30 seconds".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}
pub(crate) struct Profiler {
    query: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    read: wgpu::Buffer,
}
impl Profiler {
    pub fn new(device: &wgpu::Device) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        Some(Self {
            query: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("material pass timing"),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 16,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            read: device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 16,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
        })
    }
    pub fn writes(&self) -> wgpu::RenderPassTimestampWrites<'_> {
        wgpu::RenderPassTimestampWrites {
            query_set: &self.query,
            beginning_of_pass_write_index: Some(0),
            end_of_pass_write_index: Some(1),
        }
    }
    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.resolve_query_set(&self.query, 0..2, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.read, 0, 16);
    }
    fn milliseconds(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Result<f64, String> {
        let (tx, rx) = mpsc::channel();
        self.read
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
        wait_gpu(device)?;
        rx.recv()
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        let values = self
            .read
            .slice(..)
            .get_mapped_range()
            .map_err(|e| e.to_string())?;
        let start = u64::from_le_bytes(values[..8].try_into().unwrap());
        let end = u64::from_le_bytes(values[8..16].try_into().unwrap());
        let ms =
            end.saturating_sub(start) as f64 * queue.get_timestamp_period() as f64 / 1_000_000.;
        drop(values);
        self.read.unmap();
        Ok(ms)
    }
}
fn percentiles(samples: &mut [f64]) -> (f64, f64) {
    samples.sort_by(f64::total_cmp);
    (
        (samples[(samples.len() - 1) / 2] + samples[samples.len() / 2]) / 2.,
        samples[(samples.len() * 95).div_ceil(100).saturating_sub(1)],
    )
}
pub fn run(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    info: &str,
    samples: u32,
    cancel: &AtomicBool,
    mut report: impl FnMut(String),
) -> Result<(), String> {
    report(format!("{info}\nOptimized build: {}. GPU timestamps: {}. {samples} samples/pixel.\nGPU pass time excludes UI/presentation; wall time includes submission and 1 ms completion polling.",!cfg!(debug_assertions),device.features().contains(wgpu::Features::TIMESTAMP_QUERY)));
    let start = Instant::now();
    let mut renderer = Renderer::new(device, queue, 256, 256);
    renderer.profiler = Profiler::new(device);
    report(format!(
        "Renderer creation {:.1} ms (driver cache may be warm)",
        start.elapsed().as_secs_f64() * 1000.
    ));
    let mut throws = Vec::new();
    for object in [Object::Die, Object::Coin] {
        let start = Instant::now();
        let t = Throw::new(object, Die::D6, 2);
        report(format!(
            "{object:?} simulation {:.1} ms for {:.2}s replay",
            start.elapsed().as_secs_f64() * 1000.,
            t.duration()
        ));
        throws.push(Arc::new(t));
    }
    for (width, height) in [(256, 256), (512, 512), (960, 720)] {
        renderer.resize(device, width, height);
        for (kind, name) in [
            (0, "d6 resin"),
            (1, "d20 resin"),
            (2, "d20 opaque"),
            (3, "card"),
            (4, "coin"),
            (5, "dice physics"),
            (6, "coin physics"),
        ] {
            if cancel.load(Ordering::Relaxed) {
                return Err("Cancelled".into());
            }
            report(format!("Measuring {name} at {width}x{height}…"));
            let mut scene = Scene {
                samples,
                ..Default::default()
            };
            match kind {
                1 | 2 => {
                    scene.die = Die::D20;
                    if kind == 2 {
                        scene.appearances[0].transmission = 0.;
                    }
                }
                3 => scene.object = Object::Card,
                4 => scene.object = Object::Coin,
                5 | 6 => {
                    scene.object = if kind == 5 { Object::Die } else { Object::Coin };
                    scene.throw = Some(throws[kind - 5].clone());
                }
                _ => {}
            }
            let mut gpu = Vec::new();
            let mut wall = Vec::new();
            let started = Instant::now();
            for frame in 0..36 {
                if cancel.load(Ordering::Relaxed) {
                    return Err("Cancelled".into());
                }
                scene.phase = frame as f32 / 35.;
                if scene.throw.is_none() {
                    scene.yaw = frame as f32 * 0.12;
                }
                let start = Instant::now();
                renderer.render(device, queue, &scene);
                wait_gpu(device)?;
                let elapsed = start.elapsed().as_secs_f64() * 1000.;
                let time = renderer
                    .profiler
                    .as_ref()
                    .map(|p| p.milliseconds(device, queue))
                    .transpose()?;
                if frame >= 4 {
                    wall.push(elapsed);
                    if let Some(t) = time {
                        gpu.push(t);
                    }
                }
                if frame >= 11 && started.elapsed().as_secs_f32() > 6. {
                    break;
                }
            }
            let (median, p95) = percentiles(&mut wall);
            let timings = if gpu.is_empty() {
                "unavailable".into()
            } else {
                let (m, p) = percentiles(&mut gpu);
                format!("{m:.2}/{p:.2} ms")
            };
            report(format!("{name} {width}x{height}: GPU median/p95 {timings}; wall {median:.2}/{p95:.2} ms; n={}",wall.len()));
        }
    }
    Ok(())
}
