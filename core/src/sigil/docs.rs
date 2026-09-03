//! Documents and music on the Sigil backend: the readers in `crate::docs`
//! and `crate::media::audio` are transport-free, so all this does is find
//! the local file behind an event (downloading it once if needed) and hand
//! it over off the runtime threads. Rendered pages and cover art land in
//! the cache directory, keyed by the file's blob id.

use super::SigilSession;
use crate::ipc::wire::Reply;
use serde_json::{json, Value};
use std::path::PathBuf;

/// Render widths in device pixels: bubble, then reader.
const PAGE_W_THUMB: u32 = 700;
const PAGE_W_READER: u32 = 1_000;
const THUMB_LINES: usize = 12;
const THUMB_COLS: usize = 6;
/// Above this, an audio bubble keeps a plain file row until it is played.
const AUDIO_INFO_MAX_BYTES: u64 = 60 * 1024 * 1024;

fn n(p: &serde_json::Map<String, Value>, k: &str) -> Option<u64> {
    p.get(k)
        .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
}

fn derived_path(key: &str, tag: &str, ext: &str) -> PathBuf {
    let dir = crate::paths::cache_dir().join("derived");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("{key}-{tag}.{ext}"))
}

impl SigilSession {
    /// The file behind an event: its local path (downloaded once), name,
    /// mime, declared size, and a cache key.
    pub(super) async fn locate(
        &self,
        room_id: &str,
        event_id: &str,
    ) -> Result<(PathBuf, String, String, u64, String), Reply> {
        let Some(item) = self.item_by_id(room_id, event_id) else {
            return Err(Reply::err("unknown_event", "no such event"));
        };
        let Some(m) = item
            .get("manifest")
            .and_then(|v| serde_json::from_value::<sigil_client::media::Manifest>(v.clone()).ok())
        else {
            return Err(Reply::err("bad_request", "not a file"));
        };
        // Our own file, or one already fetched: no download, and a key of its own
        // while the upload is still in flight and the manifest has no chunks.
        if let Some(local) = item["media"]["path"].as_str().map(PathBuf::from).filter(|p| p.is_file()) {
            let key = m.chunks.first().cloned().unwrap_or_else(|| {
                hex::encode(sigil_protocol::kdf::hash(format!("{}:{}", local.display(), m.size).as_bytes()))
            });
            return Ok((local, m.filename.clone(), m.mime.clone(), m.size, key));
        }
        let path = Self::media_path(&m);
        if !path.is_file() {
            let server = self
                .conversation(room_id)
                .await
                .map(|c| c.slot_server)
                .unwrap_or_default();
            if let Err(e) = sigil_client::media::download(&self.link, &server, &m, &path).await {
                return Err(Reply::err("network", format!("{e:#}")));
            }
        }
        let key = m
            .chunks
            .first()
            .cloned()
            .unwrap_or_else(|| hex::encode(sigil_protocol::kdf::hash(m.filename.as_bytes())));
        Ok((path, m.filename.clone(), m.mime.clone(), m.size, key))
    }

    pub(super) async fn doc_preview(&self, p: &serde_json::Map<String, Value>) -> Reply {
        let (room_id, event_id) = (super::param(p, "roomId"), super::param(p, "eventId"));
        let (path, filename, mime, _, _) = match self.locate(&room_id, &event_id).await {
            Ok(v) => v,
            Err(r) => return r,
        };
        if !crate::docs::previewable(&filename, Some(&mime)) {
            return Reply::err("unsupported", "there is no preview for this kind of file");
        }
        let is_pdf = crate::docs::kind_of(&filename, Some(&mime)) == "pdf";
        let (p2, fname, m) = (path.clone(), filename.clone(), mime.clone());
        let res = tokio::task::spawn_blocking(move || {
            let prev = crate::docs::preview(&path, &fname, Some(&m));
            let info = if is_pdf {
                std::fs::read(&p2).ok().and_then(crate::docs::raster::info)
            } else {
                None
            };
            (prev, info)
        })
        .await;
        match res {
            Ok((Ok(prev), info)) => {
                let mut v = prev.to_json();
                if let (Some(obj), Some(i)) = (v.as_object_mut(), info) {
                    obj.insert("rasterisable".into(), json!(true));
                    obj.insert("pageCount".into(), json!(i.count));
                    obj.insert("pageW".into(), json!(i.width));
                    obj.insert("pageH".into(), json!(i.height));
                }
                Reply::ok(v)
            }
            Ok((Err(e), _)) => Reply::err("preview_failed", e.to_string()),
            Err(_) => Reply::err("preview_failed", "the preview reader stopped unexpectedly"),
        }
    }

    pub(super) async fn doc_thumb(&self, p: &serde_json::Map<String, Value>) -> Reply {
        let (room_id, event_id) = (super::param(p, "roomId"), super::param(p, "eventId"));
        let (path, filename, mime, _, key) = match self.locate(&room_id, &event_id).await {
            Ok(v) => v,
            Err(r) => return r,
        };
        if !crate::docs::previewable(&filename, Some(&mime)) {
            return Reply::err("unsupported", "there is no preview for this kind of file");
        }
        // A PDF is typeset: draw the page, and fall back to its text only if that fails.
        if crate::docs::kind_of(&filename, Some(&mime)) == "pdf" {
            let out = derived_path(&key, "page0", "png");
            let p2 = path.clone();
            let drawn = tokio::task::spawn_blocking(move || {
                let bytes = std::fs::read(&p2).ok()?;
                let info = crate::docs::raster::info(bytes.clone());
                let made = crate::docs::raster::render_page(bytes, 0, PAGE_W_THUMB, &out)?;
                let (w, h) = image::image_dimensions(&made).ok()?;
                Some((made, w, h, info.map(|i| i.count).unwrap_or(1)))
            })
            .await
            .ok()
            .flatten();
            if let Some((made, w, h, count)) = drawn {
                return Reply::ok(json!({
                    "kind": "pdf", "title": filename, "pages": count, "lines": [],
                    "imagePath": made.to_string_lossy(), "imageWidth": w, "imageHeight": h,
                }));
            }
        }
        let (fname, m) = (filename.clone(), mime.clone());
        let res =
            tokio::task::spawn_blocking(move || crate::docs::preview(&path, &fname, Some(&m))).await;
        match res {
            Ok(Ok(prev)) => Reply::ok(thumb_json(&prev)),
            Ok(Err(e)) => Reply::err("preview_failed", e.to_string()),
            Err(_) => Reply::err("preview_failed", "the preview reader stopped unexpectedly"),
        }
    }

    pub(super) async fn doc_page(&self, p: &serde_json::Map<String, Value>) -> Reply {
        let (room_id, event_id) = (super::param(p, "roomId"), super::param(p, "eventId"));
        let index = n(p, "index").unwrap_or(0) as usize;
        let width = n(p, "width").unwrap_or(PAGE_W_READER as u64) as u32;
        let (path, filename, mime, _, key) = match self.locate(&room_id, &event_id).await {
            Ok(v) => v,
            Err(r) => return r,
        };
        if crate::docs::kind_of(&filename, Some(&mime)) != "pdf" {
            return Reply::err("unsupported", "only a PDF has pages to draw");
        }
        let out = derived_path(&key, &format!("page{index}w{width}"), "png");
        if out.exists() {
            return match image::image_dimensions(&out) {
                Ok((w, h)) => Reply::ok(json!({"path": out.to_string_lossy(), "width": w, "height": h})),
                Err(_) => Reply::err("render_failed", "the cached page could not be read"),
            };
        }
        let res = tokio::task::spawn_blocking(move || {
            let bytes = std::fs::read(&path).ok()?;
            let made = crate::docs::raster::render_page(bytes, index, width, &out)?;
            let (w, h) = image::image_dimensions(&made).ok()?;
            Some((made, w, h))
        })
        .await;
        match res {
            Ok(Some((path, w, h))) => Reply::ok(json!({"path": path.to_string_lossy(), "width": w, "height": h})),
            Ok(None) => Reply::err("render_failed", "this page could not be drawn"),
            Err(_) => Reply::err("render_failed", "the renderer stopped unexpectedly"),
        }
    }

    pub(super) async fn audio_info(&self, p: &serde_json::Map<String, Value>) -> Reply {
        let (room_id, event_id) = (super::param(p, "roomId"), super::param(p, "eventId"));
        let (path, _filename, _mime, size, key) = match self.locate(&room_id, &event_id).await {
            Ok(v) => v,
            Err(r) => return r,
        };
        if size > AUDIO_INFO_MAX_BYTES {
            return Reply::err("too_big", "not reading a large track just to draw a bubble");
        }
        let art_out = derived_path(&key, "cover", "png");
        let res = tokio::task::spawn_blocking(move || crate::media::audio::analyse(&path, &art_out)).await;
        match res {
            Ok(t) => Reply::ok(json!({
                "artPath": t.art.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
                "accent": t.accent,
                "duration": t.duration_ms,
            })),
            Err(_) => Reply::err("internal", "the track reader stopped unexpectedly"),
        }
    }
}

/// Enough lines to recognise a document in a bubble, no more.
fn thumb_json(prev: &crate::docs::Preview) -> Value {
    use crate::docs::Block;
    let mut lines: Vec<Value> = Vec::new();
    if let Some((_, rows)) = prev.sheets.first() {
        for row in rows.iter().take(THUMB_LINES) {
            let cells: Vec<String> = row.iter().take(THUMB_COLS).cloned().collect();
            lines.push(json!({"t": "row", "cells": cells}));
        }
    } else {
        for b in prev.blocks.iter() {
            if lines.len() >= THUMB_LINES {
                break;
            }
            match b {
                Block::Para { text, level, .. } => {
                    let t = text.trim();
                    if !t.is_empty() {
                        lines.push(json!({"t": "p", "text": t, "level": level}));
                    }
                }
                Block::Section { title } => {
                    let t = title.trim();
                    if !t.is_empty() {
                        lines.push(json!({"t": "p", "text": t, "level": 1}));
                    }
                }
                Block::Table { rows } => {
                    for row in rows.iter() {
                        if lines.len() >= THUMB_LINES {
                            break;
                        }
                        let cells: Vec<String> = row.iter().take(THUMB_COLS).cloned().collect();
                        lines.push(json!({"t": "row", "cells": cells}));
                    }
                }
            }
        }
    }
    json!({"kind": prev.kind, "title": prev.title, "pages": prev.pages, "lines": lines, "imagePath": ""})
}
