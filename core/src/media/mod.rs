//! Media cache: authenticated downloads (E2EE attachments decrypted by the SDK) into
//! ~/.cache/sigil/media/<sha256(mxc|spec)>.<ext>; async fills push `media.ready`.
use std::collections::HashSet;
use std::path::{Path, PathBuf};
pub mod audio;
pub mod av;
pub mod images;
pub mod player;
pub mod voice;


use matrix_sdk::media::{MediaFormat, MediaRequestParameters, MediaThumbnailSettings};
use matrix_sdk_ui::timeline::{MsgLikeKind, TimelineItemContent};
use parking_lot::Mutex;
use ruma::events::room::message::MessageType;
use ruma::events::room::MediaSource;
use ruma::{EventId, UInt};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tracing::{debug, warn};

use crate::engine::SharedEngine;
use crate::ipc::wire::Reply;

static IN_FLIGHT: Mutex<Option<HashSet<String>>> = Mutex::new(None);

fn in_flight_insert(key: &str) -> bool {
    let mut g = IN_FLIGHT.lock();
    g.get_or_insert_with(HashSet::new).insert(key.to_string())
}
fn in_flight_remove(key: &str) {
    if let Some(s) = IN_FLIGHT.lock().as_mut() { s.remove(key); }
}

fn media_dir() -> PathBuf {
    let d = crate::paths::cache_dir().join("media");
    let _ = crate::paths::ensure_private_dir(&d);
    d
}

fn spec_str(thumb: Option<(u32, u32)>) -> String {
    match thumb { Some((w, h)) => format!("t{w}x{h}"), None => "file".into() }
}

pub fn cache_key(mxc: &str, thumb: Option<(u32, u32)>) -> String {
    let mut h = Sha256::new();
    h.update(mxc.as_bytes());
    h.update(b"|");
    h.update(spec_str(thumb).as_bytes());
    format!("{:x}", h.finalize())
}

pub fn find_cached(key: &str) -> Option<PathBuf> {
    let dir = media_dir();
    for ext in ["png", "jpg", "webp", "gif", "bin", "pdf", "mp4", "ogg", "mp3", "txt"] {
        let p = dir.join(format!("{key}.{ext}"));
        if p.exists() { return Some(p); }
    }
    // Slow path: glob prefix.
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().starts_with(key) { return Some(e.path()); }
        }
    }
    None
}

fn ext_for(data: &[u8], mime_hint: Option<&str>) -> &'static str {
    if let Ok(f) = image::guess_format(data) {
        return match f {
            image::ImageFormat::Png => "png",
            image::ImageFormat::Jpeg => "jpg",
            image::ImageFormat::WebP => "webp",
            image::ImageFormat::Gif => "gif",
            image::ImageFormat::Tiff => "tiff",
            image::ImageFormat::Bmp => "bmp",
            image::ImageFormat::Ico => "ico",
            _ => "bin",
        };
    }
    // ffprobe and ffmpeg take format hints from the extension.
    match mime_hint.unwrap_or("") {
        "application/pdf" => "pdf",
        "image/svg+xml" => "svg",
        "image/avif" => "avif",
        "image/heic" | "image/heif" => "heic",
        "image/jxl" => "jxl",
        "video/mp4" => "mp4",
        "video/quicktime" => "mov",
        "video/webm" => "webm",
        "video/x-matroska" => "mkv",
        "video/x-msvideo" => "avi",
        "video/mpeg" => "mpg",
        "video/3gpp" => "3gp",
        "video/ogg" => "ogv",
        "audio/ogg" | "audio/opus" => "ogg",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/mp4" | "audio/m4a" | "audio/x-m4a" => "m4a",
        "audio/aac" => "aac",
        "audio/flac" | "audio/x-flac" => "flac",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        "audio/webm" => "weba",
        "audio/amr" => "amr",
        "audio/x-ms-wma" => "wma",
        "text/plain" => "txt",
        "text/csv" => "csv",
        "text/markdown" => "md",
        _ => "bin",
    }
}

fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = path.with_extension("part");
    {
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(data)?;
    }
    std::fs::rename(tmp, path)
}

fn source_mxc(src: &MediaSource) -> String {
    match src { MediaSource::Plain(u) => u.to_string(), MediaSource::Encrypted(f) => f.url.to_string() }
}

async fn fetch(engine: &SharedEngine, source: MediaSource, thumb: Option<(u32, u32)>, mime_hint: Option<String>) -> anyhow::Result<PathBuf> {
    let Some(client) = engine.client() else { anyhow::bail!("not logged in") };
    let mxc = source_mxc(&source);
    let key = cache_key(&mxc, thumb);
    if let Some(p) = find_cached(&key) { return Ok(p); }
    let encrypted = matches!(source, MediaSource::Encrypted(_));
    let format = match (thumb, encrypted) {
        (Some((w, h)), false) => MediaFormat::Thumbnail(MediaThumbnailSettings::new(UInt::from(w), UInt::from(h))),
        _ => MediaFormat::File,
    };
    let data = client.media().get_media_content(&MediaRequestParameters { source: source.clone(), format }, true).await?;
    // Encrypted media has no server-side thumbnails: downscale locally.
    let data = if let (Some((w, h)), true) = (thumb, encrypted) {
        match decode_limited(&data) {
            Ok(img) => {
                let small = img.thumbnail(w, h);
                let mut buf = std::io::Cursor::new(Vec::new());
                small.write_to(&mut buf, image::ImageFormat::Png)?;
                buf.into_inner()
            }
            Err(_) => data,
        }
    } else {
        data
    };
    // Normalise anything the view cannot paint into PNG.
    let data = if images::is_image(&data, mime_hint.as_deref(), None) {
        match images::to_displayable(&data, mime_hint.as_deref()) {
            Some(png) => png,
            None => data,
        }
    } else {
        data
    };
    let ext = ext_for(&data, mime_hint.as_deref());
    let path = media_dir().join(format!("{key}.{ext}"));
    write_atomic(&path, &data)?;
    debug!("cached {mxc} {} -> {}", spec_str(thumb), path.display());
    Ok(path)
}

/// Start a background fetch (deduplicated) and push `media.ready` when done.
fn spawn_fetch(engine: &SharedEngine, source: MediaSource, thumb: Option<(u32, u32)>, mime_hint: Option<String>, extra: Value) {
    let mxc = source_mxc(&source);
    let key = cache_key(&mxc, thumb);
    if !in_flight_insert(&key) { return; }
    let engine = engine.clone();
    tokio::spawn(async move {
        let res = fetch(&engine, source, thumb, mime_hint).await;
        in_flight_remove(&key);
        match res {
            Ok(path) => {
                let mut ev = json!({"event":"media.ready","mxc":mxc,"thumbnail": thumb.map(|(w,h)| format!("{w}x{h}")), "path": path.to_string_lossy()});
                if let (Some(o), Some(x)) = (ev.as_object_mut(), extra.as_object()) { for (k, v) in x { o.insert(k.clone(), v.clone()); } }
                engine.hub.broadcast(ev);
            }
            Err(e) => warn!("media fetch failed for {mxc}: {e:#}"),
        }
    });
}

/// Avatar thumbnail: the cached path, or "" with a fetch started.
pub async fn cached_avatar_path(engine: &SharedEngine, mxc: &str) -> String {
    if mxc.is_empty() { return String::new(); }
    let Ok(uri) = ruma::OwnedMxcUri::try_from(mxc.to_string()) else { return String::new() };
    if !uri.is_valid() { return String::new(); }
    let key = cache_key(mxc, Some((96, 96)));
    if let Some(p) = find_cached(&key) { return p.to_string_lossy().into_owned(); }
    spawn_fetch(engine, MediaSource::Plain(uri), Some((96, 96)), None, json!({"kind":"avatar"}));
    String::new()
}

/// Thumbnail for a timeline media item; "" if not cached yet (fetch started).
pub async fn thumbnail_path_or_fetch(engine: &SharedEngine, room_id: &str, event_id: Option<&str>, source: &MediaSource, thumb: (u32, u32), mime: Option<String>) -> String {
    let mxc = source_mxc(source);
    let key = cache_key(&mxc, Some(thumb));
    if let Some(p) = find_cached(&key) { return p.to_string_lossy().into_owned(); }
    spawn_fetch(engine, source.clone(), Some(thumb), mime, json!({"kind":"thumbnail","roomId":room_id,"eventId":event_id}));
    String::new()
}

/// A still from a video already in the cache; never downloads to make one.
pub fn poster_if_cached(source: &MediaSource) -> String {
    let key = cache_key(&source_mxc(source), None);
    let Some(path) = find_cached(&key) else { return String::new() };
    match av::poster(&path, (800, 600)) {
        Some(p) => p.to_string_lossy().into_owned(),
        None => String::new(),
    }
}

pub fn file_path_if_cached(source: &MediaSource) -> String {
    let key = cache_key(&source_mxc(source), None);
    find_cached(&key).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

fn source_of_content(c: &TimelineItemContent) -> Option<(MediaSource, String, Option<String>)> {
    let m = c.as_msglike()?;
    match &m.kind {
        MsgLikeKind::Message(msg) => match msg.msgtype() {
            MessageType::Image(i) => Some((i.source.clone(), i.filename.clone().unwrap_or_else(|| i.body.clone()), i.info.as_ref().and_then(|x| x.mimetype.clone()))),
            MessageType::File(f) => Some((f.source.clone(), f.filename.clone().unwrap_or_else(|| f.body.clone()), f.info.as_ref().and_then(|x| x.mimetype.clone()))),
            MessageType::Video(v) => Some((v.source.clone(), v.filename.clone().unwrap_or_else(|| v.body.clone()), v.info.as_ref().and_then(|x| x.mimetype.clone()))),
            MessageType::Audio(a) => Some((a.source.clone(), a.filename.clone().unwrap_or_else(|| a.body.clone()), a.info.as_ref().and_then(|x| x.mimetype.clone()))),
            _ => None,
        },
        MsgLikeKind::Sticker(s) => {
            let src = match &s.content().source {
                ruma::events::sticker::StickerMediaSource::Plain(u) => MediaSource::Plain(u.clone()),
                ruma::events::sticker::StickerMediaSource::Encrypted(f) => MediaSource::Encrypted(f.clone()),
                #[allow(unreachable_patterns)]
                _ => return None,
            };
            Some((src, s.content().body.clone(), s.content().info.mimetype.clone()))
        }
        _ => None,
    }
}

async fn locate(engine: &SharedEngine, room_id: &str, event_id: &str) -> Result<(MediaSource, String, Option<String>), Reply> {
    let Some(open) = crate::timeline::get(engine, room_id) else { return Err(Reply::err("bad_request", "room is not open")) };
    let eid = EventId::parse(event_id).map_err(|_| Reply::err("bad_request", "invalid eventId"))?;
    let Some(item) = open.timeline.item_by_event_id(&eid).await else { return Err(Reply::err("unknown_event", "event not in timeline")) };
    source_of_content(item.content()).ok_or_else(|| Reply::err("bad_request", "event has no media"))
}

/// `doc.preview {roomId, eventId}` — a document in the viewer's structured shape.
pub async fn doc_preview(engine: SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    let room_id = p.get("roomId").and_then(Value::as_str).unwrap_or("").to_string();
    let event_id = p.get("eventId").and_then(Value::as_str).unwrap_or("").to_string();
    let (source, filename, mime) = match locate(&engine, &room_id, &event_id).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    if !crate::docs::previewable(&filename, mime.as_deref()) {
        return Reply::err("unsupported", "there is no preview for this kind of file")
    }
    let path = match fetch(&engine, source, None, mime.clone()).await {
        Ok(p) => p,
        Err(e) => return Reply::err("download_failed", format!("could not fetch the file: {e}")),
    };
    // Blocking CPU work on an untrusted file; keep it off the runtime threads.
    let fname = filename.clone();
    let m = mime.clone();
    let is_pdf = crate::docs::kind_of(&filename, mime.as_deref()) == "pdf";
    let p2 = path.clone();
    let res = tokio::task::spawn_blocking(move || {
        let prev = crate::docs::preview(&path, &fname, m.as_deref());
        // A PDF hayro cannot open falls back to text; the reader needs to know before layout.
        let info = if is_pdf { std::fs::read(&p2).ok().and_then(crate::docs::raster::info) } else { None };
        (prev, info)
    }).await;
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

/// Thumbnail budget: enough lines to recognise a document, no more.
const THUMB_LINES: usize = 12;
const THUMB_COLS: usize = 6;
/// Above this, the bubble keeps a plain file row until `doc.preview` runs.
const DOC_THUMB_MAX_BYTES: u64 = 8 * 1024 * 1024;

/// `doc.thumb {roomId, eventId, size?}` — `size` only gates the download; `docs::preview` bounds the file.
pub async fn doc_thumb(engine: SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    let room_id = p.get("roomId").and_then(Value::as_str).unwrap_or("").to_string();
    let event_id = p.get("eventId").and_then(Value::as_str).unwrap_or("").to_string();
    let declared = p.get("size").and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())));
    let (source, filename, mime) = match locate(&engine, &room_id, &event_id).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    if !crate::docs::previewable(&filename, mime.as_deref()) {
        return Reply::err("unsupported", "there is no preview for this kind of file")
    }
    let source2 = source.clone();
    let cached = find_cached(&cache_key(&source_mxc(&source), None)).is_some();
    if !cached && declared.unwrap_or(u64::MAX) > DOC_THUMB_MAX_BYTES {
        return Reply::err("too_big", "not fetching a large file just to draw a thumbnail")
    }
    let path = match fetch(&engine, source, None, mime.clone()).await {
        Ok(p) => p,
        Err(e) => return Reply::err("download_failed", format!("could not fetch the file: {e}")),
    };
    // A PDF is typeset: draw the page, and fall back to its text only if that fails.
    if crate::docs::kind_of(&filename, mime.as_deref()) == "pdf" {
        let out = page_cache_path(&source_mxc(&source2), 0, PAGE_W_THUMB);
        let p2 = path.clone();
        let drawn = tokio::task::spawn_blocking(move || {
            let bytes = std::fs::read(&p2).ok()?;
            let info = crate::docs::raster::info(bytes.clone());
            let made = crate::docs::raster::render_page(bytes, 0, PAGE_W_THUMB, &out)?;
            let (w, h) = image::image_dimensions(&made).ok()?;
            Some((made, w, h, info.map(|i| i.count).unwrap_or(1)))
        }).await.ok().flatten();
        if let Some((made, w, h, count)) = drawn {
            return Reply::ok(json!({
                "kind": "pdf", "title": filename, "pages": count, "lines": [],
                "imagePath": made.to_string_lossy(), "imageWidth": w, "imageHeight": h,
            }))
        }
    }

    let fname = filename.clone();
    let m = mime.clone();
    let res = tokio::task::spawn_blocking(move || crate::docs::preview(&path, &fname, m.as_deref())).await;
    match res {
        Ok(Ok(prev)) => Reply::ok(thumb_json(&prev)),
        Ok(Err(e)) => Reply::err("preview_failed", e.to_string()),
        Err(_) => Reply::err("preview_failed", "the preview reader stopped unexpectedly"),
    }
}

fn cover_cache_path(mxc: &str) -> PathBuf {
    let mut h = Sha256::new();
    h.update(mxc.as_bytes());
    h.update(b"|cover");
    media_dir().join(format!("{:x}.png", h.finalize()))
}

/// Above this, an audio bubble keeps a plain file row until it is played.
const AUDIO_INFO_MAX_BYTES: u64 = 60 * 1024 * 1024;

/// `audio.info {roomId, eventId, size?}` → `{artPath, accent, duration}`; senders often omit duration.
pub async fn audio_info(engine: SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    let room_id = p.get("roomId").and_then(Value::as_str).unwrap_or("").to_string();
    let event_id = p.get("eventId").and_then(Value::as_str).unwrap_or("").to_string();
    let declared = p.get("size").and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())));
    let (source, _filename, mime) = match locate(&engine, &room_id, &event_id).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let mxc = source_mxc(&source);
    let cached = find_cached(&cache_key(&mxc, None)).is_some();
    if !cached && declared.unwrap_or(u64::MAX) > AUDIO_INFO_MAX_BYTES {
        return Reply::err("too_big", "not fetching a large track just to draw a bubble")
    }
    let path = match fetch(&engine, source, None, mime).await {
        Ok(p) => p,
        Err(e) => return Reply::err("download_failed", format!("could not fetch the file: {e}")),
    };
    let art_out = cover_cache_path(&mxc);
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

/// Rendered-page cache path, keyed on page index and render width.
fn page_cache_path(mxc: &str, index: usize, width: u32) -> PathBuf {
    let mut h = Sha256::new();
    h.update(mxc.as_bytes());
    h.update(format!("|page{index}@{width}").as_bytes());
    media_dir().join(format!("{:x}.png", h.finalize()))
}

/// Render widths in device pixels: bubble, then reader.
const PAGE_W_THUMB: u32 = 700;
const PAGE_W_READER: u32 = 1_000;

/// `doc.page {roomId, eventId, index, width?}` → `{path, width, height}`, rendered on demand.
pub async fn doc_page(engine: SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    let room_id = p.get("roomId").and_then(Value::as_str).unwrap_or("").to_string();
    let event_id = p.get("eventId").and_then(Value::as_str).unwrap_or("").to_string();
    let index = p.get("index").and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))).unwrap_or(0) as usize;
    let width = p.get("width").and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
        .unwrap_or(PAGE_W_READER as u64) as u32;
    let (source, filename, mime) = match locate(&engine, &room_id, &event_id).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    if crate::docs::kind_of(&filename, mime.as_deref()) != "pdf" {
        return Reply::err("unsupported", "only a PDF has pages to draw")
    }
    let out = page_cache_path(&source_mxc(&source), index, width);
    if out.exists() {
        return match image::image_dimensions(&out) {
            Ok((w, h)) => Reply::ok(json!({"path": out.to_string_lossy(), "width": w, "height": h})),
            Err(_) => Reply::err("render_failed", "the cached page could not be read"),
        }
    }
    let path = match fetch(&engine, source, None, mime.clone()).await {
        Ok(p) => p,
        Err(e) => return Reply::err("download_failed", format!("could not fetch the file: {e}")),
    };
    let res = tokio::task::spawn_blocking(move || {
        let bytes = std::fs::read(&path).ok()?;
        let made = crate::docs::raster::render_page(bytes, index, width, &out)?;
        let (w, h) = image::image_dimensions(&made).ok()?;
        Some((made, w, h))
    }).await;
    match res {
        Ok(Some((path, w, h))) => Reply::ok(json!({"path": path.to_string_lossy(), "width": w, "height": h})),
        Ok(None) => Reply::err("render_failed", "this page could not be drawn"),
        Err(_) => Reply::err("render_failed", "the renderer stopped unexpectedly"),
    }
}

/// `vcard.read {roomId, eventId}` → `{cards, photos}`; empty when unparseable.
pub async fn vcard_read(engine: SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    let room_id = p.get("roomId").and_then(Value::as_str).unwrap_or("").to_string();
    let event_id = p.get("eventId").and_then(Value::as_str).unwrap_or("").to_string();
    let (source, filename, mime) = match locate(&engine, &room_id, &event_id).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    if !crate::timeline::items::is_vcard(&filename, mime.as_deref()) {
        return Reply::err("unsupported", "not a vCard")
    }
    let mxc = source_mxc(&source);
    let path = match fetch(&engine, source, None, mime).await {
        Ok(p) => p,
        Err(e) => return Reply::err("download_failed", format!("could not fetch the file: {e}")),
    };
    let res = tokio::task::spawn_blocking(move || {
        let raw = std::fs::read(&path).ok()?;
        // Lossy UTF-8: do not refuse a card over one bad byte.
        let text = String::from_utf8_lossy(&raw).into_owned();
        let cards = crate::timeline::vcard::parse(&text);
        if cards.is_empty() { return None }
        // Photos go out as files, not base64 across the socket.
        let photos: Vec<String> = cards.iter().enumerate().map(|(i, c)| {
            if c.photo.is_empty() { return String::new() }
            let Some(bytes) = crate::timeline::vcard::photo_bytes(&c.photo) else { return String::new() };
            let out = vcard_photo_path(&mxc, i);
            match std::fs::write(&out, &bytes) {
                Ok(_) => out.to_string_lossy().into_owned(),
                Err(_) => String::new(),
            }
        }).collect();
        Some((cards, photos))
    }).await;
    match res {
        Ok(Some((cards, photos))) => Reply::ok(json!({
            "cards": crate::timeline::vcard::to_json(&cards),
            "photos": photos,
        })),
        Ok(None) => Reply::err("unreadable", "this file is not a readable vCard"),
        Err(_) => Reply::err("internal", "the vCard reader stopped unexpectedly"),
    }
}

fn vcard_photo_path(mxc: &str, index: usize) -> PathBuf {
    let mut h = Sha256::new();
    h.update(mxc.as_bytes());
    h.update(format!("|vcard{index}").as_bytes());
    media_dir().join(format!("{:x}.img", h.finalize()))
}

/// `contact.vcf {userId, displayName}` → a `.vcf` written to the cache.
pub async fn contact_vcf(_engine: SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    let user_id = p.get("userId").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if user_id.is_empty() { return Reply::err("bad_request", "a vCard needs a userId") }
    let name = p.get("displayName").and_then(Value::as_str).unwrap_or("").to_string();
    let body = crate::timeline::vcard::to_vcf(&name, &user_id);
    // The recipient sees this file name: prefer the display name, then the localpart.
    let label = if name.trim().is_empty() {
        user_id.trim_start_matches('@').split(':').next().unwrap_or("contact").to_string()
    } else {
        name.trim().to_string()
    };
    let safe: String = label
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let safe = safe.trim().trim_matches('_').trim();
    let safe = if safe.is_empty() { "contact" } else { safe };
    // `download` means keep it: the downloads dir is not swept by the cache GC.
    let dir = if p.get("download").and_then(|v| v.as_bool().or_else(|| v.as_str().map(|s| s == "true"))).unwrap_or(false) {
        crate::paths::download_dir()
    } else {
        media_dir()
    };
    let out = dir.join(format!("{safe}.vcf"));
    match std::fs::write(&out, body.as_bytes()) {
        Ok(_) => Reply::ok(json!({"path": out.to_string_lossy(), "filename": format!("{safe}.vcf")})),
        Err(e) => Reply::err("internal", format!("could not write the vCard: {e}")),
    }
}

fn thumb_json(prev: &crate::docs::Preview) -> Value {
    use crate::docs::Block;
    let mut lines: Vec<Value> = Vec::new();
    // A spreadsheet reads as a grid, so its first sheet wins over the blocks.
    if let Some((_, rows)) = prev.sheets.first() {
        for row in rows.iter().take(THUMB_LINES) {
            let cells: Vec<String> = row.iter().take(THUMB_COLS).cloned().collect();
            lines.push(json!({"t": "row", "cells": cells}));
        }
    } else {
        for b in prev.blocks.iter() {
            if lines.len() >= THUMB_LINES { break }
            match b {
                Block::Para { text, level, .. } => {
                    let t = text.trim();
                    if t.is_empty() { continue }
                    lines.push(json!({"t": "p", "text": t, "level": level}));
                }
                Block::Section { title } => {
                    let t = title.trim();
                    if !t.is_empty() { lines.push(json!({"t": "p", "text": t, "level": 1})) }
                }
                Block::Table { rows } => {
                    for row in rows.iter() {
                        if lines.len() >= THUMB_LINES { break }
                        let cells: Vec<String> = row.iter().take(THUMB_COLS).cloned().collect();
                        lines.push(json!({"t": "row", "cells": cells}));
                    }
                }
            }
        }
    }
    json!({
        "kind": prev.kind,
        "title": prev.title,
        "pages": prev.pages,
        "lines": lines,
    })
}

pub async fn video_play(engine: SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    let room_id = p.get("roomId").and_then(Value::as_str).unwrap_or("").to_string();
    let event_id = p.get("eventId").and_then(Value::as_str).unwrap_or("").to_string();
    let with_audio = p.get("audio").and_then(Value::as_bool).unwrap_or(true);
    let (source, _filename, mime) = match locate(&engine, &room_id, &event_id).await { Ok(v) => v, Err(r) => return r };
    let file = match fetch(&engine, source, None, mime).await {
        Ok(path) => path.to_string_lossy().into_owned(),
        Err(e) => return Reply::err("network", format!("{e:#}")),
    };
    video_stop(engine.clone(), p).await;
    match player::start("video-play", &file, with_audio) {
        Ok(pb) => {
            let out = json!({"path": pb.path, "width": pb.width, "height": pb.height, "duration": pb.duration, "startAt": pb.start_at});
            *engine.playback.lock() = Some(pb);
            Reply::ok(out)
        }
        Err(e) => Reply::err("internal", e.to_string()),
    }
}

/// `voice.start` — begin recording; `voice.level` events stream while it runs.
pub async fn voice_start(engine: SharedEngine, _p: &serde_json::Map<String, Value>) -> Reply {
    if engine.recording.lock().is_some() { return Reply::err("bad_request", "already recording"); }
    match voice::start(&engine) {
        Ok(rec) => { *engine.recording.lock() = Some(rec); Reply::ok(json!({})) }
        Err(e) => Reply::err("internal", format!("{e:#}")),
    }
}

/// `voice.stop` → {path, duration, waveform}
pub async fn voice_stop(engine: SharedEngine, _p: &serde_json::Map<String, Value>) -> Reply {
    let Some(rec) = engine.recording.lock().take() else { return Reply::err("bad_request", "not recording") };
    let (path, secs, wave) = voice::stop(rec);
    Reply::ok(json!({"path": path.to_string_lossy(), "duration": secs, "waveform": wave}))
}

/// `voice.cancel` — stop and discard.
pub async fn voice_cancel(engine: SharedEngine, _p: &serde_json::Map<String, Value>) -> Reply {
    if let Some(rec) = engine.recording.lock().take() { voice::cancel(rec); }
    Reply::ok(json!({}))
}

/// `voice.send {roomId, path, duration, waveform, caption?}`
pub async fn voice_send(engine: SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    use matrix_sdk::attachment::{AttachmentInfo, BaseAudioInfo};
    use matrix_sdk_ui::timeline::AttachmentConfig;
    let room_id = p.get("roomId").and_then(Value::as_str).unwrap_or("").to_string();
    let path = PathBuf::from(p.get("path").and_then(Value::as_str).unwrap_or(""));
    let secs = p.get("duration").and_then(Value::as_f64).unwrap_or(0.0);
    let wave: Vec<f32> = p.get("waveform").and_then(Value::as_array).map(|a| {
        a.iter().filter_map(Value::as_f64).map(|v| v as f32).collect()
    }).unwrap_or_default();
    let caption = p.get("caption").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).map(|s| s.to_string());
    let Some(open) = crate::timeline::get(&engine, &room_id) else { return Reply::err("bad_request", "room is not open") };
    let meta = match std::fs::metadata(&path) { Ok(m) if m.is_file() => m, _ => return Reply::err("bad_request", "recording not found") };
    let info = AttachmentInfo::Voice(BaseAudioInfo {
        duration: Some(std::time::Duration::from_secs_f64(secs)),
        size: Some(UInt::new_saturating(meta.len())),
        waveform: if wave.is_empty() { None } else { Some(wave) },
    });
    let config = AttachmentConfig {
        txn_id: None,
        info: Some(info),
        thumbnail: None,
        caption: caption.map(ruma::events::room::message::TextMessageEventContent::plain),
        mentions: None,
        in_reply_to: None,
    };
    let mime: mime::Mime = "audio/ogg".parse().unwrap();
    match open.timeline.send_attachment(path, mime, config).use_send_queue().await {
        Ok(()) => Reply::ok(json!({})),
        Err(e) => Reply::err("network", e.to_string()),
    }
}

/// `audio.play {roomId, eventId, seek?}` / `audio.stop` — voice-note playback.
pub async fn audio_play(engine: SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    let room_id = p.get("roomId").and_then(Value::as_str).unwrap_or("").to_string();
    let event_id = p.get("eventId").and_then(Value::as_str).unwrap_or("").to_string();
    let seek = p.get("seek").and_then(Value::as_f64).unwrap_or(0.0).max(0.0);
    let (source, _f, mime) = match locate(&engine, &room_id, &event_id).await { Ok(v) => v, Err(r) => return r };
    let file = match fetch(&engine, source, None, mime).await {
        Ok(pth) => pth,
        Err(e) => return Reply::err("network", format!("{e:#}")),
    };
    if let Some(mut a) = engine.audio_play.lock().take() { a.stop(); }
    match voice::play(&file, &event_id, seek) {
        Ok(a) => { *engine.audio_play.lock() = Some(a); Reply::ok(json!({"seek": seek})) }
        Err(e) => Reply::err("internal", format!("{e:#}")),
    }
}

/// `audio.playFile {path, seek?}` — preview a local clip (pending recording).
pub async fn audio_play_file(engine: SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    let path = PathBuf::from(p.get("path").and_then(Value::as_str).unwrap_or(""));
    let seek = p.get("seek").and_then(Value::as_f64).unwrap_or(0.0).max(0.0);
    if !path.is_file() { return Reply::err("bad_request", "file not found"); }
    if let Some(mut a) = engine.audio_play.lock().take() { a.stop(); }
    match voice::play(&path, "local", seek) {
        Ok(a) => { *engine.audio_play.lock() = Some(a); Reply::ok(json!({})) }
        Err(e) => Reply::err("internal", format!("{e:#}")),
    }
}

pub async fn audio_stop(engine: SharedEngine, _p: &serde_json::Map<String, Value>) -> Reply {
    if let Some(mut a) = engine.audio_play.lock().take() { a.stop(); }
    Reply::ok(json!({}))
}

/// `video.seek {seconds}` — restart the decoder at the requested offset.
pub async fn video_seek(engine: SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    let secs = p.get("seconds").and_then(Value::as_f64).unwrap_or(0.0).max(0.0);
    let file = { engine.playback.lock().as_ref().map(|pb| pb.file.clone()) };
    let Some(file) = file else { return Reply::err("bad_request", "nothing playing") };
    if let Some(mut pb) = engine.playback.lock().take() { pb.stop(); }
    match player::start_at("video-play", &file, true, secs) {
        Ok(pb) => {
            let out = json!({"path": pb.path, "width": pb.width, "height": pb.height, "duration": pb.duration, "startAt": pb.start_at});
            *engine.playback.lock() = Some(pb);
            Reply::ok(out)
        }
        Err(e) => Reply::err("internal", e.to_string()),
    }
}

pub async fn video_stop(engine: SharedEngine, _p: &serde_json::Map<String, Value>) -> Reply {
    if let Some(mut pb) = engine.playback.lock().take() { pb.stop(); }
    Reply::ok(json!({}))
}

/// `link.preview {url}` → {url,title,description,siteName,imagePath}; the homeserver fetches, not us.
pub async fn link_preview(engine: SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    let url = p.get("url").and_then(Value::as_str).unwrap_or("").to_string();
    if url.is_empty() { return Reply::err("bad_request", "missing url"); }
    let Some(client) = engine.client() else { return Reply::err("not_logged_in", "not logged in") };
    let req = ruma::api::client::media::get_media_preview::v3::Request::new(url.clone());
    let resp = match client.send(req).await { Ok(r) => r, Err(e) => return Reply::err("network", e.to_string()) };
    let data: serde_json::Value = match resp.data {
        Some(raw) => serde_json::from_str(raw.get()).unwrap_or(serde_json::Value::Null),
        None => serde_json::Value::Null,
    };
    let get = |k: &str| data.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let title = get("og:title");
    let description = get("og:description");
    let site_name = get("og:site_name");
    let image_mxc = get("og:image");
    let mut image_path = String::new();
    let (mut iw, mut ih) = (0u32, 0u32);
    let mut accent = String::new();
    if !image_mxc.is_empty() {
        if let Ok(mxc) = ruma::OwnedMxcUri::try_from(image_mxc.as_str()) {
            let source = matrix_sdk::ruma::events::room::MediaSource::Plain(mxc);
            image_path = cached_or_fetch_thumb(&engine, &source, (800, 800)).await;
            if !image_path.is_empty() {
                if let Ok(img) = image::open(&image_path) {
                    iw = img.width();
                    ih = img.height();
                    accent = dominant_colour(&img);
                }
            }
        }
    }
    // YouTube serves scrapers a JS shell; fall back to the canonical thumbnail.
    let mut used_fallback = false;
    if image_path.is_empty() {
        if let Some(id) = youtube_id(&url) {
            let thumb_url = format!("https://i.ytimg.com/vi/{id}/maxresdefault.jpg");
            let req = ruma::api::client::media::get_media_preview::v3::Request::new(thumb_url.clone());
            if let Ok(r2) = client.send(req).await {
                if let Some(raw) = r2.data {
                    if let Ok(d2) = serde_json::from_str::<serde_json::Value>(raw.get()) {
                        if let Some(mxc_s) = d2.get("og:image").and_then(Value::as_str) {
                            if let Ok(mxc) = ruma::OwnedMxcUri::try_from(mxc_s) {
                                let source = matrix_sdk::ruma::events::room::MediaSource::Plain(mxc);
                                image_path = cached_or_fetch_thumb(&engine, &source, (800, 800)).await;
                                if !image_path.is_empty() {
                                    used_fallback = true;
                                    if let Ok(img) = image::open(&image_path) {
                                        iw = img.width();
                                        ih = img.height();
                                        accent = dominant_colour(&img);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    // A title of just "- YouTube" (or the site name alone) is scraper junk.
    let title = {
        let t = title.trim().trim_start_matches('-').trim().to_string();
        let host = url.split("//").nth(1).and_then(|h| h.split('/').next()).unwrap_or("").trim_start_matches("www.").to_string();
        let host_label = host.split('.').next().unwrap_or("").to_string();
        if t.is_empty()
            || t.eq_ignore_ascii_case(&site_name)
            || t.eq_ignore_ascii_case(&host)
            || t.eq_ignore_ascii_case(&host_label)
        { String::new() } else { t }
    };

    // The fallback only fires when the page gave nothing real, so any title or description is boilerplate.
    let (title, description) = if used_fallback { (String::new(), String::new()) } else { (title, description) };

    // Video links get a play badge (og:type, or the usual short-form hosts).
    let og_type = data.get("og:type").and_then(Value::as_str).unwrap_or("").to_lowercase();
    let lower = url.to_lowercase();
    let is_video = og_type.contains("video")
        || lower.contains("youtube.com/watch") || lower.contains("youtu.be/")
        || lower.contains("/shorts/") || lower.contains("vimeo.com/")
        || lower.contains("tiktok.com/") || lower.contains("/clip/");
    Reply::ok(json!({
        "url": url, "title": title, "description": description,
        "siteName": site_name, "imagePath": image_path,
        "imageWidth": iw, "imageHeight": ih,
        "accent": accent, "isVideo": is_video,
    }))
}

fn youtube_id(url: &str) -> Option<String> {
    let u = url.split('#').next()?;
    let host_ok = u.contains("youtube.com") || u.contains("youtu.be");
    if !host_ok { return None; }
    let id = if let Some(i) = u.find("v=") {
        u[i + 2..].split(['&', '?']).next()?.to_string()
    } else if let Some(i) = u.find("/shorts/") {
        u[i + 8..].split(['&', '?', '/']).next()?.to_string()
    } else if let Some(i) = u.find("/embed/") {
        u[i + 7..].split(['&', '?', '/']).next()?.to_string()
    } else if u.contains("youtu.be/") {
        let i = u.find("youtu.be/")? + 9;
        u[i..].split(['&', '?', '/']).next()?.to_string()
    } else {
        return None;
    };
    if id.len() >= 6 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') { Some(id) } else { None }
}


/// Decode with limits: `image::load_from_memory` has none, so a tiny file can declare enormous dimensions.
fn decode_limited(data: &[u8]) -> Result<image::DynamicImage, image::ImageError> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(data))
        .with_guessed_format()
        .map_err(image::ImageError::IoError)?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    reader.decode()
}

fn dominant_colour(img: &image::DynamicImage) -> String {
    use image::GenericImageView;
    let small = img.thumbnail(24, 24).to_rgb8();
    let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
    for px in small.pixels() {
        let (pr, pg, pb) = (px[0] as u64, px[1] as u64, px[2] as u64);
        let mx = pr.max(pg).max(pb);
        let mn = pr.min(pg).min(pb);
        // skip near-white / near-black pixels: they wash the average out
        if mx < 24 || mn > 232 { continue; }
        // weight saturated pixels higher so the tint keeps the artwork's hue
        let w = 1 + (mx - mn) / 24;
        r += pr * w; g += pg * w; b += pb * w; n += w;
    }
    if n == 0 { let _ = img.dimensions(); return String::new(); }
    let (mut r, mut g, mut b) = ((r / n) as f32, (g / n) as f32, (b / n) as f32);
    // darken toward a readable card background
    let scale = 0.42_f32;
    r *= scale; g *= scale; b *= scale;
    format!("#{:02x}{:02x}{:02x}", r as u8, g as u8, b as u8)
}

async fn cached_or_fetch_thumb(engine: &SharedEngine, source: &MediaSource, thumb: (u32, u32)) -> String {
    match fetch(engine, source.clone(), Some(thumb), None).await {
        Ok(p) => p.to_string_lossy().into_owned(),
        Err(_) => String::new(),
    }
}

/// `media.get {roomId, eventId, thumbnail?: {width,height}}` → {path, filename, mime}
pub async fn get(engine: SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    let room_id = p.get("roomId").and_then(Value::as_str).unwrap_or("");
    let event_id = p.get("eventId").and_then(Value::as_str).unwrap_or("");
    let thumb = p.get("thumbnail").and_then(|t| Some((t.get("width")?.as_u64()? as u32, t.get("height")?.as_u64()? as u32)));
    let (source, filename, mime) = match locate(&engine, room_id, event_id).await { Ok(v) => v, Err(r) => return r };
    match fetch(&engine, source, thumb, mime.clone()).await {
        Ok(path) => Reply::ok(json!({"path": path.to_string_lossy(), "filename": filename, "mime": mime})),
        Err(e) => Reply::err("network", format!("{e:#}")),
    }
}

/// `location.map {geoUri, width, height, zoom?}` → {path, width, height}.
/// A static map for the location card: OSM raster tiles fetched around the
/// point, stitched and cropped engine-side (the toolkit has no map widget).
/// Cached by rounded coordinates + geometry; tiles carry a proper User-Agent.
pub async fn location_map(engine: SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    let uri = p.get("geoUri").and_then(Value::as_str).unwrap_or("");
    let want_w = p.get("width").and_then(Value::as_u64).unwrap_or(640).clamp(64, 1280) as u32;
    let want_h = p.get("height").and_then(Value::as_u64).unwrap_or(400).clamp(64, 1280) as u32;
    let zoom = p.get("zoom").and_then(Value::as_u64).unwrap_or(15).clamp(3, 19) as u32;
    let Some((lat, lon)) = crate::timeline::items::geo_of(uri) else {
        return Reply::err("bad_request", "geoUri has no coordinates");
    };
    if !crate::geo::valid_coords(lat, lon) {
        return Reply::err("bad_request", "coordinates out of range");
    }

    let key = format!("map-{:.5}-{:.5}-{zoom}-{want_w}x{want_h}", lat, lon);
    let out = media_dir().join(format!("{key}.png"));
    if out.exists() {
        return Reply::ok(json!({"path": out.to_string_lossy(), "width": want_w, "height": want_h}));
    }
    let _ = std::fs::create_dir_all(media_dir());

    // Slippy-map maths: the fractional tile the point lands on.
    let n = f64::from(1u32 << zoom);
    let xf = (lon + 180.0) / 360.0 * n;
    let lat_r = lat.to_radians();
    let yf = (1.0 - (lat_r.tan() + 1.0 / lat_r.cos()).ln() / std::f64::consts::PI) / 2.0 * n;

    // Pixel of the point in world coordinates (256px tiles).
    let cx = xf * 256.0;
    let cy = yf * 256.0;
    let left = (cx - f64::from(want_w) / 2.0).floor() as i64;
    let top = (cy - f64::from(want_h) / 2.0).floor() as i64;
    let tx0 = left.div_euclid(256);
    let ty0 = top.div_euclid(256);
    let tx1 = (left + i64::from(want_w)).div_euclid(256);
    let ty1 = (top + i64::from(want_h)).div_euclid(256);

    let client = reqwest::Client::builder()
        .user_agent(format!("Sigil/{} (Matrix client; location cards)", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string());
    let client = match client { Ok(c) => c, Err(e) => return Reply::err("internal", e) };

    let mut canvas = image::RgbaImage::from_pixel(want_w, want_h, image::Rgba([0x4b, 0x4b, 0x4b, 0xff]));
    for ty in ty0..=ty1 {
        for tx in tx0..=tx1 {
            if ty < 0 || ty >= i64::from(1u32 << zoom) { continue; }
            let wrapped_x = tx.rem_euclid(i64::from(1u32 << zoom));
            let url = format!("https://tile.openstreetmap.org/{zoom}/{wrapped_x}/{ty}.png");
            let bytes = match client.get(&url).send().await {
                Ok(r) if r.status().is_success() => match r.bytes().await { Ok(b) => b, Err(_) => continue },
                _ => continue,
            };
            let Ok(tile) = image::load_from_memory(&bytes) else { continue };
            let tile = tile.to_rgba8();
            let ox = tx * 256 - left;
            let oy = ty * 256 - top;
            image::imageops::overlay(&mut canvas, &tile, ox, oy);
        }
    }
    let _ = engine; // fetches are plain HTTP; nothing session-bound
    match canvas.save_with_format(&out, image::ImageFormat::Png) {
        Ok(()) => Reply::ok(json!({"path": out.to_string_lossy(), "width": want_w, "height": want_h})),
        Err(e) => Reply::err("internal", e.to_string()),
    }
}

/// `media.gifFrames {roomId, eventId}` → {frames: [paths], delays: [ms], width, height}.
/// Decodes an animated GIF into frame PNGs a view without animated-image
/// support can cycle. Capped at 64 frames and 480px on the long edge; the
/// cache is keyed on the source file, so repeat calls cost a stat.
pub async fn gif_frames(engine: SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    let room_id = p.get("roomId").and_then(Value::as_str).unwrap_or("");
    let event_id = p.get("eventId").and_then(Value::as_str).unwrap_or("");
    let (source, _filename, mime) = match locate(&engine, room_id, event_id).await { Ok(v) => v, Err(r) => return r };
    let src = match fetch(&engine, source, None, mime).await { Ok(p) => p, Err(e) => return Reply::err("network", format!("{e:#}")) };

    let mut h = Sha256::new();
    h.update(src.to_string_lossy().as_bytes());
    if let Ok(m) = std::fs::metadata(&src) { h.update(m.len().to_le_bytes()); }
    let dir = media_dir().join(format!("gif-{:x}", h.finalize()));
    let meta = dir.join("frames.json");
    if let Ok(cached) = std::fs::read_to_string(&meta) {
        if let Ok(v) = serde_json::from_str::<Value>(&cached) {
            return Reply::ok(v);
        }
    }

    let decoded = tokio::task::spawn_blocking(move || -> anyhow::Result<Value> {
        use image::AnimationDecoder;
        let file = std::io::BufReader::new(std::fs::File::open(&src)?);
        let frames = image::codecs::gif::GifDecoder::new(file)?.into_frames();
        std::fs::create_dir_all(&dir)?;
        let mut paths: Vec<String> = Vec::new();
        let mut delays: Vec<u32> = Vec::new();
        let (mut w, mut hgt) = (0u32, 0u32);
        for (i, frame) in frames.take(64).enumerate() {
            let frame = frame?;
            let (num, den) = frame.delay().numer_denom_ms();
            let img = image::DynamicImage::ImageRgba8(frame.into_buffer());
            let img = if img.width().max(img.height()) > 480 {
                img.resize(480, 480, image::imageops::FilterType::Triangle)
            } else { img };
            (w, hgt) = (img.width(), img.height());
            let path = dir.join(format!("{i:03}.png"));
            img.save_with_format(&path, image::ImageFormat::Png)?;
            paths.push(path.to_string_lossy().into_owned());
            delays.push((num / den.max(1)).clamp(20, 1000));
        }
        anyhow::ensure!(paths.len() > 1, "not animated");
        let v = json!({"frames": paths, "delays": delays, "width": w, "height": hgt});
        std::fs::write(dir.join("frames.json"), serde_json::to_vec(&v)?)?;
        Ok(v)
    }).await;
    match decoded {
        Ok(Ok(v)) => Reply::ok(v),
        Ok(Err(e)) => Reply::err("bad_media", format!("{e:#}")),
        Err(e) => Reply::err("internal", e.to_string()),
    }
}

/// `media.saveAs {roomId, eventId, dest}` — dest is a directory or a full file path.
pub async fn save_as(engine: SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    let room_id = p.get("roomId").and_then(Value::as_str).unwrap_or("");
    let event_id = p.get("eventId").and_then(Value::as_str).unwrap_or("");
    let dest = p.get("dest").and_then(Value::as_str).unwrap_or("").to_string();
    if dest.is_empty() { return Reply::err("bad_request", "dest is required"); }
    let (source, filename, mime) = match locate(&engine, room_id, event_id).await { Ok(v) => v, Err(r) => return r };
    let src = match fetch(&engine, source, None, mime).await { Ok(p) => p, Err(e) => return Reply::err("network", format!("{e:#}")) };
    let mut target = PathBuf::from(&dest);
    if target.is_dir() { target = target.join(sanitize(&filename)); }
    match std::fs::copy(&src, &target) {
        Ok(_) => Reply::ok(json!({"path": target.to_string_lossy()})),
        Err(e) => Reply::err("internal", e.to_string()),
    }
}

fn sanitize(name: &str) -> String {
    let s: String = name.chars().map(|c| if c == '/' || c == '\0' { '_' } else { c }).collect();
    if s.is_empty() { "file".into() } else { s }
}

/// Poster path for the user's own file, keyed on path, size and mtime.
fn outgoing_poster_path(src: &PathBuf) -> PathBuf {
    let mut h = Sha256::new();
    h.update(src.to_string_lossy().as_bytes());
    if let Ok(m) = std::fs::metadata(src) {
        h.update(m.len().to_le_bytes());
        if let Ok(t) = m.modified() {
            if let Ok(d) = t.duration_since(std::time::UNIX_EPOCH) { h.update(d.as_secs().to_le_bytes()); }
        }
    }
    media_dir().join(format!("outgoing-{:x}.poster.png", h.finalize()))
}

/// Poster for a video we are about to send: without one every client shows a blank rectangle.
fn video_thumbnail(path: &PathBuf) -> Option<matrix_sdk::attachment::Thumbnail> {
    thumbnail_from(&av::poster_to(path, (800, 600), &outgoing_poster_path(path))?)
}

fn audio_thumbnail(path: &PathBuf) -> Option<matrix_sdk::attachment::Thumbnail> {
    thumbnail_from(&av::cover(path, (800, 600), &outgoing_poster_path(path))?)
}

fn thumbnail_from(png: &PathBuf) -> Option<matrix_sdk::attachment::Thumbnail> {
    let data = std::fs::read(png).ok()?;
    let (w, h) = image::image_dimensions(png).ok()?;
    Some(matrix_sdk::attachment::Thumbnail {
        size: UInt::new_saturating(data.len() as u64),
        data,
        content_type: mime::IMAGE_PNG,
        width: UInt::from(w),
        height: UInt::from(h),
    })
}

pub async fn send_attachment(engine: SharedEngine, p: &serde_json::Map<String, Value>) -> Reply {
    use matrix_sdk::attachment::{AttachmentInfo, BaseAudioInfo, BaseFileInfo, BaseImageInfo, BaseVideoInfo};
    use matrix_sdk_ui::timeline::AttachmentConfig;
    let room_id = p.get("roomId").and_then(Value::as_str).unwrap_or("").to_string();
    let path = PathBuf::from(p.get("path").and_then(Value::as_str).unwrap_or(""));
    let caption = p.get("caption").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).map(|s| s.to_string());
    let Some(open) = crate::timeline::get(&engine, &room_id) else { return Reply::err("bad_request", "room is not open") };
    let meta = match std::fs::metadata(&path) { Ok(m) if m.is_file() => m, _ => return Reply::err("bad_request", "file not found") };
    let mime = mime_guess::from_path(&path).first_or_octet_stream();
    let size = Some(UInt::new_saturating(meta.len()));

    let (info, thumbnail) = if mime.type_() == mime::IMAGE {
        let p2 = path.clone();
        let dims = tokio::task::spawn_blocking(move || image::image_dimensions(&p2).ok()).await.ok().flatten();
        let (w, h) = dims.map(|(w, h)| (Some(UInt::from(w)), Some(UInt::from(h)))).unwrap_or((None, None));
        (AttachmentInfo::Image(BaseImageInfo { width: w, height: h, size, blurhash: None, is_animated: None }), None)
    } else if mime.type_() == mime::VIDEO {
        let p2 = path.clone();
        let made = tokio::task::spawn_blocking(move || (av::probe(&p2), video_thumbnail(&p2))).await.ok();
        let (probe, thumb) = made.unwrap_or((None, None));
        if thumb.is_none() { warn!("media: no poster frame for {} — sending it without one", path.display()); }
        let vi = BaseVideoInfo {
            duration: probe.as_ref().map(|p| std::time::Duration::from_millis(p.duration_ms)),
            width: probe.as_ref().map(|p| UInt::from(p.width)),
            height: probe.as_ref().map(|p| UInt::from(p.height)),
            size,
            blurhash: None,
        };
        (AttachmentInfo::Video(vi), thumb)
    } else if mime.type_() == mime::AUDIO {
        // Duration lets the bubble draw a scrubber before anything downloads.
        let p2 = path.clone();
        let made = tokio::task::spawn_blocking(move || (av::probe(&p2), audio_thumbnail(&p2))).await.ok();
        let (probe, thumb) = made.unwrap_or((None, None));
        let ai = BaseAudioInfo {
            duration: probe.as_ref().map(|p| std::time::Duration::from_millis(p.duration_ms)),
            size,
            waveform: None,
        };
        (AttachmentInfo::Audio(ai), thumb)
    } else {
        (AttachmentInfo::File(BaseFileInfo { size }), None)
    };

    let config = AttachmentConfig {
        txn_id: None,
        info: Some(info),
        thumbnail,
        caption: caption.map(ruma::events::room::message::TextMessageEventContent::plain),
        mentions: None,
        in_reply_to: None,
    };
    match open.timeline.send_attachment(path.clone(), mime, config).use_send_queue().await {
        Ok(()) => Reply::ok(json!({})),
        Err(e) => Reply::err("network", e.to_string()),
    }
}

/// Keep the cache under a size budget (oldest first).
pub fn gc(max_bytes: u64) {
    let dir = media_dir();
    let Ok(rd) = std::fs::read_dir(&dir) else { return };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = rd.flatten().filter_map(|e| {
        let m = e.metadata().ok()?;
        Some((m.modified().ok()?, m.len(), e.path()))
    }).collect();
    let total: u64 = files.iter().map(|f| f.1).sum();
    if total <= max_bytes { return; }
    files.sort_by_key(|f| f.0);
    let mut freed = 0;
    for f in files {
        if total - freed <= max_bytes { break; }
        if std::fs::remove_file(&f.2).is_ok() { freed += f.1; }
    }
}
