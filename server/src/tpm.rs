//! The TPM as a dumb pipe. The client runs its own encrypted TPM session
//! (protocol design B12, Path 2); this side only hands raw command bytes
//! to `/dev/tpmrm0` and returns the raw response. Nothing here interprets
//! a command, so nothing here can learn a password or a key.

use std::io::{Read, Write};

const DEVICE: &str = "/dev/tpmrm0";

pub fn available() -> bool {
    std::path::Path::new(DEVICE).exists()
}

/// One command in, one response out. The resource manager serialises
/// access across processes.
pub fn relay(command: &[u8]) -> anyhow::Result<Vec<u8>> {
    if command.len() < 10 || command.len() > 4096 {
        anyhow::bail!("bad command length");
    }
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(DEVICE)?;
    f.write_all(command)?;
    let mut buf = vec![0u8; 4096];
    let n = f.read(&mut buf)?;
    buf.truncate(n);
    Ok(buf)
}

/// The endorsement key's public part and certificate chain, read once
/// via `TPM2_ReadPublic` and the EK certificate NV index. Not yet wired:
/// returns None until the client-side session exists to consume it.
pub fn info() -> Option<(Vec<u8>, Vec<u8>)> {
    None
}
