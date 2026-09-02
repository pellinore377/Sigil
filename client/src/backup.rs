//! Backup and recovery. The backup is the whole client state (account,
//! MLS store, caller extra such as history) sealed under a random data
//! key and stored on the home server under a label derived from that key.
//! The data key is wrapped under `backup_key = KDF(pw_key ‖ recovery_key)`
//! and the wrap stored by username, so restoring needs the username, the
//! password and the recovery key (from another device, the TPM path, or
//! the printed code). The server holds none of the three.

use crate::provider::SigilProvider;
use crate::{Link, State};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use sigil_protocol::encoding::{Reader, Writer};
use sigil_protocol::recovery;
use sigil_protocol::wire::{Request, Response, CHUNK_LEN};
use std::path::Path;

/// Per-account recovery material, kept in the account state.
#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
pub struct Recovery {
    pub salt: String,
    pub recovery_key: String,
    pub data_key: String,
}

fn pw_key(password: &str, salt: &[u8; 16]) -> [u8; 32] {
    // NFC normalisation: the protocol requires it; for the character sets a
    // password is likely to use, UTF-8 of the string as typed is NFC already.
    recovery::password_key(password.as_bytes(), salt)
}

/// First-time setup: choose the keys, wrap the data key, store the wrap.
pub async fn enable(link: &Link, st: &mut State, password: &str) -> anyhow::Result<()> {
    let salt: [u8; 16] = rand::random();
    let recovery_key: [u8; 32] = rand::random();
    let data_key: [u8; 32] = rand::random();
    st.recovery = Some(Recovery {
        salt: hex::encode(salt),
        recovery_key: hex::encode(recovery_key),
        data_key: hex::encode(data_key),
    });
    st.save()?;
    put_wrap(link, st, password).await
}

async fn put_wrap(link: &Link, st: &State, password: &str) -> anyhow::Result<()> {
    let r = st
        .recovery
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("recovery not set up"))?;
    let salt: [u8; 16] = hex::decode(&r.salt)?.try_into().unwrap();
    let recovery_key: [u8; 32] = hex::decode(&r.recovery_key)?.try_into().unwrap();
    let data_key: [u8; 32] = hex::decode(&r.data_key)?.try_into().unwrap();
    let bk = recovery::backup_key(&pw_key(password, &salt), &recovery_key);
    let wrap = recovery::wrap_data_key(&bk, &rand::random(), &data_key);
    let mut msg = b"sigil v1 wrap put".to_vec();
    msg.extend_from_slice(&salt);
    msg.extend_from_slice(&wrap);
    let sig = ed25519_dalek::Signer::sign(&st.identity().signing, &msg).to_bytes();
    link.call(
        &st.server(),
        &Request::WrapPut {
            username: st.username.clone(),
            salt,
            wrap,
            sig,
        },
        None,
    )
    .await?;
    Ok(())
}

/// Change the password: only the wrap changes.
pub async fn set_password(link: &Link, st: &State, password: &str) -> anyhow::Result<()> {
    put_wrap(link, st, password).await
}

/// The printed recovery code.
pub fn code(st: &State) -> Option<String> {
    let r = st.recovery.as_ref()?;
    let k: [u8; 32] = hex::decode(&r.recovery_key).ok()?.try_into().ok()?;
    Some(recovery::recovery_code(&k))
}

/// Seal everything into chunks and upload them under the backup label.
/// `extra` is the caller's (the engine's history). Returns the chunk count.
pub async fn upload(
    link: &Link,
    st: &mut State,
    provider: &SigilProvider,
    extra: &[u8],
) -> anyhow::Result<u32> {
    let r = st
        .recovery
        .clone()
        .ok_or_else(|| anyhow::anyhow!("recovery not set up"))?;
    let data_key: [u8; 32] = hex::decode(&r.data_key)?.try_into().unwrap();
    let label = recovery::backup_label(&data_key);
    provider.save()?;
    let account = serde_json::to_vec(&*st)?;
    let mls = std::fs::read(st.mls_path()).unwrap_or_default();
    let plain = Writer::new()
        .u8(1)
        .bytes(&account)
        .bytes(&mls)
        .bytes(extra)
        .finish();
    let nonce: [u8; 24] = rand::random();
    let sealed = XChaCha20Poly1305::new((&data_key).into())
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &plain,
                aad: b"sigil v1 backup",
            },
        )
        .map_err(|_| anyhow::anyhow!("seal"))?;
    let mut payload = nonce.to_vec();
    payload.extend_from_slice(&sealed);
    // count ‖ nonce ‖ sealed, cut into CHUNK_LEN pieces, the last padded to
    // a 4 KiB multiple so the server sees only a few sizes
    let count = (payload.len() + 4).div_ceil(CHUNK_LEN) as u32;
    let mut body = count.to_le_bytes().to_vec();
    body.extend_from_slice(&payload);
    for (i, chunk) in body.chunks(CHUNK_LEN).enumerate() {
        let mut c = chunk.to_vec();
        let padded = c.len().div_ceil(4096) * 4096;
        c.resize(padded, 0);
        let token = st.take_token()?;
        link.call(
            &st.server(),
            &Request::BackupPut {
                label,
                index: i as u32,
                chunk: c,
                token,
            },
            None,
        )
        .await?;
    }
    Ok(count)
}

/// Restore on a fresh device from username, password and recovery key.
/// Writes the account and MLS files and returns the state plus `extra`.
pub async fn restore(
    path: &Path,
    envoy: &str,
    username: &str,
    password: &str,
    recovery_key: &[u8; 32],
) -> anyhow::Result<(State, Vec<u8>)> {
    let (_, server) = sigil_protocol::names::parse_username(username)
        .map_err(|_| anyhow::anyhow!("bad username"))?;
    let device_id = hex::encode(rand::random::<[u8; 32]>());
    let link = Link::connect(envoy, &device_id).await?;
    let resp = link
        .call(
            server,
            &Request::WrapGet {
                username: username.to_string(),
            },
            None,
        )
        .await?;
    let Response::WrapGet { salt, wrap } = resp else {
        anyhow::bail!("unexpected")
    };
    let bk = recovery::backup_key(&pw_key(password, &salt), recovery_key);
    let data_key = recovery::unwrap_data_key(&bk, &wrap)
        .map_err(|_| anyhow::anyhow!("wrong password or recovery code"))?;
    let label = recovery::backup_label(&data_key);
    let first = fetch(&link, server, &label, 0)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no backup on the server"))?;
    let count = u32::from_le_bytes(first[..4].try_into().unwrap());
    let mut body = first;
    for i in 1..count {
        body.extend_from_slice(
            &fetch(&link, server, &label, i)
                .await?
                .ok_or_else(|| anyhow::anyhow!("backup chunk {i} missing"))?,
        );
    }
    let body = body[4..].to_vec();
    // strip padding: the sealed length is unknown, so try shrinking zero
    // tails until the tag verifies (at most 4095 bytes of padding)
    let (nonce, rest) = body.split_at(24);
    let mut end = rest.len();
    let plain = loop {
        if let Ok(p) = XChaCha20Poly1305::new((&data_key).into()).decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: &rest[..end],
                aad: b"sigil v1 backup",
            },
        ) {
            break p;
        }
        if end == 0 || rest.len() - end > 4096 || rest[end - 1] != 0 {
            anyhow::bail!("backup does not decrypt");
        }
        end -= 1;
    };
    let mut r = Reader::new(&plain);
    if r.u8().map_err(|_| anyhow::anyhow!("short"))? != 1 {
        anyhow::bail!("unknown backup version");
    }
    let e = |_| anyhow::anyhow!("malformed backup");
    let account = r.bytes().map_err(e)?.to_vec();
    let mls = r.bytes().map_err(e)?.to_vec();
    let extra = r.bytes().map_err(e)?.to_vec();
    let mut st: State = serde_json::from_slice(&account)?;
    st.path = path.to_path_buf();
    st.device_id = device_id;
    st.envoy = envoy.to_string();
    // The lost device may have spent any of the tokens in the backup.
    st.tokens.clear();
    st.save()?;
    if !mls.is_empty() {
        std::fs::write(st.mls_path(), &mls)?;
    }
    if let Err(e) = crate::account::draw_tokens(&link, &mut st, 20).await {
        tracing_warn(&format!("could not draw tokens after restore: {e:#}"));
    }
    Ok((st, extra))
}

fn tracing_warn(msg: &str) {
    eprintln!("warning: {msg}");
}

async fn fetch(
    link: &Link,
    server: &str,
    label: &[u8; 32],
    index: u32,
) -> anyhow::Result<Option<Vec<u8>>> {
    match link
        .call(
            server,
            &Request::BackupGet {
                label: *label,
                index,
            },
            None,
        )
        .await
    {
        Ok(Response::Bytes(b)) => Ok(Some(b)),
        Ok(_) => Ok(None),
        Err(e) if e.to_string().contains("NotFound") => Ok(None),
        Err(e) => Err(e),
    }
}
