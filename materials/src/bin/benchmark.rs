#![forbid(unsafe_code)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))?;
    let info = adapter.get_info();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
        ..Default::default()
    }))?;
    sigil_materials::performance::run(
        &device,
        &queue,
        &format!("{} · {:?} · {}", info.name, info.backend, info.driver_info),
        if std::env::args().any(|a| a == "--single-sample") {
            1
        } else {
            4
        },
        &std::sync::atomic::AtomicBool::new(false),
        |s| println!("{s}"),
    )
    .map_err(std::io::Error::other)?;
    Ok(())
}
