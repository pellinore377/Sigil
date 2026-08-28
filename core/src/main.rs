//! sigil-engine — the daemon wrapper around the `sigil_engine` library:
//! a unix socket speaking JSON lines. The Matrix logic lives in the library.

use sigil_engine::{docs, engine, geo, ipc, maps, media, notify, paths, presence, session, shm, sync, timeline};


use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "sigil-engine", version, about)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the daemon (default).
    Daemon {
        #[arg(long)]
        socket: Option<std::path::PathBuf>,
        #[arg(long, default_value = "info")]
        log_level: String,
    },
    /// Send one request to a running daemon and print the reply.
    Cli {
        req: String,
        /// key=value params (JSON values with key:=json)
        params: Vec<String>,
        #[arg(long)]
        follow: bool,
        #[arg(long)]
        socket: Option<std::path::PathBuf>,
    },
    /// Print environment diagnostics.
    Doctor,
    /// Write a moving test pattern into a video shm file (video-test.shm) for the QML plugin.
    Shmtest {
        #[arg(long, default_value_t = 640)]
        width: u32,
        #[arg(long, default_value_t = 360)]
        height: u32,
        #[arg(long, default_value_t = 30)]
        fps: u32,
        #[arg(long, default_value_t = 20)]
        seconds: u32,
        /// At this frame, double the frame size (exercises the grow/replace path).
        #[arg(long, default_value_t = 0)]
        grow_at: u32,
    },
    /// Dump the latest frame of a video shm file to a PNG.
    Shmdump {
        /// Track key (file is video-<key>.shm) or a full path
        key: String,
        out: std::path::PathBuf,
    },
}

fn main() -> anyhow::Result<()> {
    sigil_engine::init_crypto();
    let cli = Cli::parse();
    match cli.cmd.unwrap_or(Cmd::Daemon { socket: None, log_level: "info".into() }) {
        Cmd::Daemon { socket, log_level } => {
            use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
            let state = paths::state_dir();
            paths::ensure_private_dir(&state).ok();
            // The log records room, user, event and session ids: keep it 0600, roll at 8 MB.
            let log_path = state.join("engine.log");
            if std::fs::metadata(&log_path).map(|m| m.len() > 8 * 1024 * 1024).unwrap_or(false) {
                let _ = std::fs::rename(&log_path, state.join("engine.log.1"));
            }
            let file = tracing_appender::rolling::never(&state, "engine.log");
            let (file_nb, guard) = tracing_appender::non_blocking(file);
            std::mem::forget(guard);
            let filter = EnvFilter::try_new(&log_level).unwrap_or_else(|_| EnvFilter::new("info"));
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().with_writer(std::io::stderr))
                .with(fmt::layer().with_ansi(false).with_writer(file_nb))
                .init();
            for name in ["engine.log", "engine.log.1"] {
                let p = state.join(name);
                if p.exists() {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
                }
            }
            // matrix-sdk creates its sqlite stores with the umask; the crypto store must not be readable.
            paths::tighten_store_permissions();

            std::panic::set_hook(Box::new(|info| {
                let bt = std::backtrace::Backtrace::force_capture();
                tracing::error!("PANIC: {info}\n{bt}");
            }));
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_stack_size(16 * 1024 * 1024)
                .build()?;
            rt.block_on(ipc::serve(socket.unwrap_or_else(paths::socket_path)))
        }
        Cmd::Cli { req, params, follow, socket } => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(ipc::client::run(socket.unwrap_or_else(paths::socket_path), req, params, follow))
        }
        Cmd::Doctor => doctor(),
        Cmd::Shmtest { width, height, fps, seconds, grow_at } => shmtest(width, height, fps, seconds, grow_at),
        Cmd::Shmdump { key, out } => shmdump(&key, &out),
    }
}

fn doctor() -> anyhow::Result<()> {
    println!("sigil-engine {}", env!("CARGO_PKG_VERSION"));
    println!("socket:  {}", paths::socket_path().display());
    println!("state:   {}", paths::state_dir().display());
    println!("cache:   {}", paths::cache_dir().display());
    println!("shm dir: {}", paths::shm_dir().display());
    #[cfg(feature = "calls")]
    println!("calls:   enabled (livekit {})", livekit_version());
    #[cfg(not(feature = "calls"))]
    println!("calls:   disabled at build time");
    Ok(())
}

#[cfg(feature = "calls")]
fn livekit_version() -> &'static str {
    // Touch the crate so the prebuilt libwebrtc link is exercised.
    let _ = std::mem::size_of::<livekit::RoomOptions>();
    "0.8.3"
}

fn shmtest(w: u32, h: u32, fps: u32, seconds: u32, grow_at: u32) -> anyhow::Result<()> {
    let mut wr = shm::ShmWriter::create("test", w, h)?;
    println!("writing {} ({w}x{h} @{fps} for {seconds}s)", wr.path().display());
    let frames = fps * seconds;
    let period = std::time::Duration::from_micros(1_000_000 / fps as u64);
    let start = std::time::Instant::now();
    let (mut w, mut h) = (w, h);
    for f in 0..frames {
        if grow_at != 0 && f == grow_at {
            (w, h) = (w * 2, h * 2);
            wr.ensure_capacity(w, h)?;
            println!("grew to {w}x{h} (generation bump) at frame {f}");
        }
        wr.write_with(w, h, false, |dst, stride| {
            for y in 0..h as usize {
                let row = &mut dst[y * stride..y * stride + w as usize * 4];
                for (x, px) in row.chunks_exact_mut(4).enumerate() {
                    px[0] = ((x * 255 / w as usize) as u32 + f * 8) as u8;
                    px[1] = (y * 255 / h as usize) as u8;
                    px[2] = 255 - (x * 255 / w as usize) as u8;
                    px[3] = 255;
                }
            }
        });
        let t = start + period * (f + 1);
        if let Some(d) = t.checked_duration_since(std::time::Instant::now()) { std::thread::sleep(d); }
    }
    println!("done {frames} frames");
    Ok(())
}

fn shmdump(key: &str, out: &std::path::Path) -> anyhow::Result<()> {
    let path = if key.contains('/') { std::path::PathBuf::from(key) } else { paths::shm_dir().join(format!("video-{key}.shm")) };
    let data = std::fs::read(&path)?;
    let u32_at = |o: usize| u32::from_le_bytes(data[o..o + 4].try_into().unwrap());
    let u64_at = |o: usize| u64::from_le_bytes(data[o..o + 8].try_into().unwrap());
    anyhow::ensure!(u32_at(0) == shm::MAGIC, "bad magic");
    let header_size = u32_at(0x08) as usize;
    let slot_stride = u32_at(0x10) as usize;
    let latest = u64_at(0x28);
    anyhow::ensure!(latest != 0, "no frame yet");
    let slot = (latest & 0xff) as usize;
    let base = header_size + slot * slot_stride;
    let (w, h, stride) = (u32_at(base + 4), u32_at(base + 8), u32_at(base + 12) as usize);
    let px = base + shm::SLOT_HDR;
    let mut img = image::RgbaImage::new(w, h);
    for y in 0..h as usize {
        let row = &data[px + y * stride..px + y * stride + w as usize * 4];
        for x in 0..w as usize { img.put_pixel(x as u32, y as u32, image::Rgba([row[x * 4], row[x * 4 + 1], row[x * 4 + 2], 255])); }
    }
    img.save(out)?;
    println!("{}x{} frame seq {} → {}", w, h, latest >> 8, out.display());
    Ok(())
}
