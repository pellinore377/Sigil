//! Unix-socket JSON-lines server: `{"req","id",...}` → `{"reply",...}`, pushes `{"event",...}`.

pub mod client;
pub mod hub;
pub mod wire;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tracing::{info, warn};

use hub::Hub;
use wire::{Reply, Request};

pub const PROTOCOL_VERSION: u32 = 1;

pub type Server = crate::engine::Engine;

pub async fn serve(path: PathBuf) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        crate::paths::ensure_private_dir(parent).ok();
    }
    // Exclusive flock: two engines on one Matrix device fight over the LiveKit identity.
    let lock_path = path.with_extension("lock");
    let lock_file = std::fs::OpenOptions::new().create(true).write(true).open(&lock_path)?;
    let rc = unsafe { libc::flock(std::os::fd::AsRawFd::as_raw_fd(&lock_file), libc::LOCK_EX | libc::LOCK_NB) };
    if rc != 0 {
        anyhow::bail!("another engine already holds {}", lock_path.display());
    }
    std::mem::forget(lock_file); // hold the lock for the life of the process
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).with_context(|| format!("bind {}", path.display()))?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    info!("listening on {}", path.display());
    let server = crate::engine::Engine::new(Hub::new());
    {
        let s = server.clone();
        tokio::spawn(async move { s.startup().await });
    }
    loop {
        let (stream, _) = listener.accept().await?;
        if !peer_is_us(&stream) {
            warn!("rejecting connection from another uid");
            continue;
        }
        let server = server.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_client(server, stream).await {
                warn!("client error: {e:#}");
            }
        });
    }
}

fn peer_is_us(stream: &UnixStream) -> bool {
    match stream.peer_cred() {
        Ok(c) => c.uid() == unsafe { libc::getuid() },
        Err(_) => false,
    }
}

async fn handle_client(server: Arc<Server>, stream: UnixStream) -> anyhow::Result<()> {
    let (rd, mut wr) = stream.into_split();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(256);
    let sub = server.hub.subscribe(tx.clone());

    let hello = serde_json::json!({"event":"hello","protocol":PROTOCOL_VERSION,"engine":env!("CARGO_PKG_VERSION"),"pid":std::process::id()});
    tx.send(hello.to_string()).await.ok();
    for ev in server.greeting() {
        tx.send(ev.to_string()).await.ok();
    }

    let writer = tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            if wr.write_all(line.as_bytes()).await.is_err() || wr.write_all(b"\n").await.is_err() {
                break;
            }
        }
    });

    let mut lines = BufReader::with_capacity(1 << 16, rd).lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let req: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                warn!("bad request line: {e}");
                continue;
            }
        };
        let server = server.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            let id = req.id.clone();
            let reply = dispatch(&server, req).await;
            if let Some(id) = id {
                let r = reply.into_json(id);
                tx.send(r.to_string()).await.ok();
            }
        });
    }
    drop(sub);
    writer.abort();
    Ok(())
}

async fn dispatch(server: &Arc<Server>, req: Request) -> Reply {
    server.dispatch(req).await
}
