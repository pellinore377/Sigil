use crate::Error;
use serde_json::Value;
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};
pub struct Assets {
    root: PathBuf,
    style: Vec<u8>,
}
fn relative(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && !value.contains(['\\', '%', '?', '#', ':', '\0'])
        && value
            .split('/')
            .all(|part| !part.is_empty() && !matches!(part, "." | ".."))
}
fn local(value: &str) -> bool {
    value == "/client/v0/maps/tiles.json"
        || value
            .strip_prefix("/client/v0/maps/assets/")
            .is_some_and(relative)
}
pub(crate) fn attribution(value: &str) -> Result<String, Error> {
    if value.len() > 8192 {
        return Err(Error::Invalid);
    }
    let plain = html2text::config::plain_no_decorate()
        .link_footnotes(true)
        .string_from_read(value.as_bytes(), 4096)
        .map_err(|_| Error::Invalid)?;
    Ok(html_escape::encode_text(plain.trim()).into_owned())
}
fn links(value: &mut Value) -> Result<bool, Error> {
    let mut changed = false;
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if key == "attribution" {
                    let source = value.as_str().ok_or(Error::Invalid)?;
                    let normalized = attribution(source)?;
                    if normalized != source {
                        *value = Value::String(normalized);
                        changed = true;
                    }
                }
                if matches!(key.as_str(), "imports" | "models") {
                    return Err(Error::Invalid);
                }
                if matches!(
                    key.as_str(),
                    "url" | "urls" | "glyphs" | "sprite" | "data" | "tiles"
                ) {
                    match &*value {
                        Value::String(value) if !local(value) => return Err(Error::Invalid),
                        Value::Array(values) => {
                            for value in values {
                                if value.as_str().is_some_and(|value| !local(value)) {
                                    return Err(Error::Invalid);
                                }
                            }
                        }
                        _ => (),
                    }
                }
                changed |= links(value)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                changed |= links(value)?;
            }
        }
        _ => (),
    }
    Ok(changed)
}
fn read(root: &Path, name: &str) -> Result<Vec<u8>, Error> {
    if !relative(name) {
        return Err(Error::Invalid);
    }
    let path = root
        .join(name)
        .canonicalize()
        .map_err(|_| Error::Unavailable)?;
    if !path.starts_with(root) {
        return Err(Error::Invalid);
    }
    let file = File::open(path).map_err(|_| Error::Unavailable)?;
    let meta = file.metadata().map_err(|_| Error::Unavailable)?;
    if !meta.is_file() || meta.len() > 4 * 1024 * 1024 {
        return Err(Error::Invalid);
    }
    let mut bytes = Vec::new();
    file.take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Unavailable)?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(Error::Invalid);
    }
    Ok(bytes)
}
impl Assets {
    pub fn open(root: &Path) -> Result<Self, Error> {
        let root = root.canonicalize().map_err(|_| Error::Unavailable)?;
        let mut style = read(&root, "style.json")?;
        if style.len() > 1024 * 1024 {
            return Err(Error::Invalid);
        }
        let mut value: Value = serde_json::from_slice(&style).map_err(|_| Error::Invalid)?;
        if value["version"] != 8 || !value["sources"].is_object() || !value["layers"].is_array() {
            return Err(Error::Invalid);
        }
        if links(&mut value)? {
            style = serde_json::to_vec(&value).map_err(|_| Error::Invalid)?;
        }
        Ok(Self { root, style })
    }
    pub fn style(&self) -> &[u8] {
        &self.style
    }
    pub fn asset(&self, name: &str) -> Result<(Vec<u8>, &'static str), Error> {
        if name == "style.json" {
            return Ok((self.style.clone(), "application/json"));
        }
        let kind = match name.rsplit_once('.').map(|(_, ext)| ext) {
            Some("pbf") => "application/x-protobuf",
            Some("png") => "image/png",
            Some("json") => "application/json",
            _ => return Err(Error::Invalid),
        };
        let mut bytes = read(&self.root, name)?;
        if kind == "application/json" {
            let mut value: Value = serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
            if links(&mut value)? {
                bytes = serde_json::to_vec(&value).map_err(|_| Error::Invalid)?;
            }
        }
        Ok((bytes, kind))
    }
}
