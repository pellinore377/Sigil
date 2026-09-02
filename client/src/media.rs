//! Media: a file is encrypted under its own random key, cut into 256 KiB
//! chunks, each stored as a blob; the message carries the manifest. The
//! server sees same-sized ciphertext bricks with no link to a slot.

use crate::{Link, State};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use sigil_protocol::wire::{Request, Response, CHUNK_LEN};
use std::path::Path;

/// What travels in an event of kind 9.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Manifest {
    pub filename: String,
    pub mime: String,
    pub size: u64,
    /// Hex file key.
    pub key: String,
    /// Hex blob ids, in order.
    pub chunks: Vec<String>,
    #[serde(default)]
    pub caption: String,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    /// A voice message: how long it plays, in milliseconds.
    #[serde(default)]
    pub duration_ms: Option<u64>,
    /// A voice message: loudness per slice, 0 to 100, for the bubble's bars.
    #[serde(default)]
    pub waveform: Vec<u8>,
}

fn nonce_for(index: u32) -> [u8; 24] {
    let mut n = [0u8; 24];
    n[..4].copy_from_slice(&index.to_le_bytes());
    n
}

/// The plaintext each chunk holds: CHUNK_LEN less the tag.
const PLAIN_PER_CHUNK: usize = CHUNK_LEN - 16;

/// Encrypt and upload a file; returns the manifest for the message.
pub async fn upload(
    link: &Link,
    st: &mut State,
    path: &Path,
    caption: &str,
) -> anyhow::Result<Manifest> {
    let data = std::fs::read(path)?;
    let filename = path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    let mime = mime_guess::from_path(path)
        .first_or_octet_stream()
        .to_string();
    let key: [u8; 32] = rand::random();
    let cipher = XChaCha20Poly1305::new((&key).into());
    let (width, height) = if mime.starts_with("image/") {
        image::image_dimensions(path)
            .ok()
            .map(|(w, h)| (Some(w), Some(h)))
            .unwrap_or((None, None))
    } else {
        (None, None)
    };
    let mut ids = Vec::new();
    let server = st.server();
    for (i, chunk) in data.chunks(PLAIN_PER_CHUNK).enumerate() {
        let ct = cipher
            .encrypt(
                XNonce::from_slice(&nonce_for(i as u32)),
                Payload {
                    msg: chunk,
                    aad: &(i as u32).to_le_bytes(),
                },
            )
            .map_err(|_| anyhow::anyhow!("seal"))?;
        // pad the final chunk to a 4 KiB multiple; full chunks are exactly CHUNK_LEN
        let mut c = ct;
        let padded = c.len().div_ceil(4096) * 4096;
        c.resize(padded, 0);
        let token = st.take_token()?;
        let resp = link
            .call(&server, &Request::BlobPut { chunk: c, token }, None)
            .await?;
        let Response::BlobPut { id } = resp else {
            anyhow::bail!("unexpected")
        };
        ids.push(hex::encode(id));
    }
    if data.is_empty() {
        anyhow::bail!("empty file");
    }
    Ok(Manifest {
        filename,
        mime,
        size: data.len() as u64,
        key: hex::encode(key),
        chunks: ids,
        caption: caption.to_string(),
        width,
        height,
        duration_ms: None,
        waveform: Vec::new(),
    })
}

/// Download and decrypt into `dest`.
pub async fn download(link: &Link, server: &str, m: &Manifest, dest: &Path) -> anyhow::Result<()> {
    let key: [u8; 32] = hex::decode(&m.key)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("bad key"))?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    let mut out = Vec::with_capacity(m.size as usize);
    let mut remaining = m.size as usize;
    for (i, id_hex) in m.chunks.iter().enumerate() {
        let id: [u8; 32] = hex::decode(id_hex)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("bad id"))?;
        let resp = link.call(server, &Request::BlobGet { id }, None).await?;
        let Response::Bytes(c) = resp else {
            anyhow::bail!("unexpected")
        };
        // ciphertext length is plaintext + 16; the last chunk carries padding
        let plain_len = remaining.min(PLAIN_PER_CHUNK);
        let ct = &c[..plain_len + 16];
        let p = cipher
            .decrypt(
                XNonce::from_slice(&nonce_for(i as u32)),
                Payload {
                    msg: ct,
                    aad: &(i as u32).to_le_bytes(),
                },
            )
            .map_err(|_| anyhow::anyhow!("chunk {i} does not decrypt"))?;
        remaining -= p.len();
        out.extend_from_slice(&p);
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(dest, &out)?;
    Ok(())
}
