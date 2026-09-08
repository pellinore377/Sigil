use bytes::Bytes;
use futures_util::{stream, stream::BoxStream, StreamExt};
use object_store::{path::Path, *};
use std::{fs::File, io::Read, ops::Range, os::unix::fs::FileExt, sync::Arc, time::SystemTime};

pub(crate) const MAX_TILE: u64 = 4 * 1024 * 1024;
const MAX_DIRECTORY: u64 = 1024 * 1024;
fn invalid() -> Error {
    Error::Generic {
        store: "local map",
        source: std::io::Error::other("invalid or changed map archive").into(),
    }
}
fn unsupported<T>() -> Result<T> {
    Err(Error::NotSupported {
        source: std::io::Error::other("read-only map archive").into(),
    })
}
fn range(bytes: &[u8], at: usize, size: u64) -> Result<Range<u64>> {
    let start = u64::from_le_bytes(bytes[at..at + 8].try_into().map_err(|_| invalid())?);
    let len = u64::from_le_bytes(bytes[at + 8..at + 16].try_into().map_err(|_| invalid())?);
    let end = start
        .checked_add(len)
        .filter(|end| *end <= size)
        .ok_or_else(invalid)?;
    if start < 127 {
        return Err(invalid());
    }
    Ok(start..end)
}
fn includes(outer: &Range<u64>, inner: &Range<u64>) -> bool {
    outer.start <= inner.start && inner.start <= inner.end && inner.end <= outer.end
}
fn unpack(bytes: &[u8], compression: u8, limit: u64) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    match compression {
        1 => output.extend_from_slice(bytes),
        2 => {
            flate2::read::GzDecoder::new(bytes)
                .take(limit + 1)
                .read_to_end(&mut output)
                .map_err(|_| invalid())?;
        }
        _ => return Err(invalid()),
    }
    if output.len() as u64 > limit {
        return Err(invalid());
    }
    Ok(output)
}
fn varint(bytes: &mut &[u8]) -> Result<u64> {
    let mut n = 0u64;
    for shift in (0..70).step_by(7) {
        let (&b, rest) = bytes.split_first().ok_or_else(invalid)?;
        *bytes = rest;
        if shift == 63 && b > 1 {
            return Err(invalid());
        }
        n |= u64::from(b & 127) << shift;
        if b < 128 {
            return Ok(n);
        }
    }
    Err(invalid())
}
// Guard allocation counts and offsets before the dependency decodes a directory.
fn directory(mut bytes: &[u8], leaf: u64, data: u64) -> Result<()> {
    let count = varint(&mut bytes)?;
    if count > 32768 || count > bytes.len() as u64 / 4 {
        return Err(invalid());
    }
    let mut id = 0u64;
    for i in 0..count {
        let delta = varint(&mut bytes)?;
        if i > 0 && delta == 0 {
            return Err(invalid());
        }
        id = id
            .checked_add(delta)
            .filter(|v| *v < (1u64 << 63))
            .ok_or_else(invalid)?;
    }
    let mut entries = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let run = u32::try_from(varint(&mut bytes)?).map_err(|_| invalid())?;
        entries.push((run, 0));
    }
    for (_, length) in &mut entries {
        *length = varint(&mut bytes)?;
        if *length == 0 || *length > MAX_TILE {
            return Err(invalid());
        }
    }
    let mut end = None;
    for (run, length) in entries {
        let offset = varint(&mut bytes)?;
        let start = if offset == 0 {
            end.ok_or_else(invalid)?
        } else {
            offset - 1
        };
        let next = start.checked_add(length).ok_or_else(invalid)?;
        if next > if run == 0 { leaf } else { data } || (run == 0 && length > MAX_DIRECTORY) {
            return Err(invalid());
        }
        end = Some(next);
    }
    if !bytes.is_empty() {
        return Err(invalid());
    }
    Ok(())
}
#[derive(Debug)]
pub(crate) struct Archive {
    file: Arc<File>,
    size: u64,
    modified: SystemTime,
    root: Range<u64>,
    metadata: Range<u64>,
    leaf: Range<u64>,
    data: Range<u64>,
    compression: u8,
    slots: Arc<tokio::sync::Semaphore>,
}
impl Archive {
    pub(crate) fn open(path: &std::path::Path) -> Result<Self> {
        let file = File::open(path).map_err(|_| invalid())?;
        let meta = file.metadata().map_err(|_| invalid())?;
        let size = meta.len();
        if !meta.is_file() || !(127..=1024 * 1024 * 1024 * 1024).contains(&size) {
            return Err(invalid());
        }
        let mut header = [0; 127];
        file.read_exact_at(&mut header, 0).map_err(|_| invalid())?;
        if &header[..8] != b"PMTiles\x03"
            || !matches!(header[97], 1 | 2)
            || header[100] > header[101]
            || header[101] > 26
        {
            return Err(invalid());
        }
        let root = range(&header, 8, size)?;
        let metadata = range(&header, 24, size)?;
        let leaf = range(&header, 40, size)?;
        let data = range(&header, 56, size)?;
        if root.end > 16384 || root.is_empty() || metadata.end - metadata.start > MAX_DIRECTORY {
            return Err(invalid());
        }
        let ranges = [&root, &metadata, &leaf, &data];
        for (i, a) in ranges.iter().enumerate() {
            for b in &ranges[i + 1..] {
                if !a.is_empty() && !b.is_empty() && a.start < b.end && b.start < a.end {
                    return Err(invalid());
                }
            }
        }
        Ok(Self {
            file: Arc::new(file),
            size,
            modified: meta.modified().map_err(|_| invalid())?,
            root,
            metadata,
            leaf,
            data,
            compression: header[97],
            slots: Arc::new(tokio::sync::Semaphore::new(4)),
        })
    }
    fn check(&self) -> Result<()> {
        let meta = self.file.metadata().map_err(|_| invalid())?;
        if meta.len() != self.size || meta.modified().map_err(|_| invalid())? != self.modified {
            return Err(invalid());
        }
        Ok(())
    }
    async fn read(&self, range: Range<u64>) -> Result<Bytes> {
        self.check()?;
        let file = self.file.clone();
        let start = range.start;
        let length = usize::try_from(range.end - range.start).map_err(|_| invalid())?;
        let permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| invalid())?;
        let bytes = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut bytes = vec![0; length];
            file.read_exact_at(&mut bytes, start)
                .map_err(|_| invalid())?;
            Ok::<_, Error>(bytes)
        })
        .await
        .map_err(|_| invalid())??;
        self.check()?;
        if range.start == 0 {
            let content = unpack(
                &bytes[self.root.start as usize..self.root.end as usize],
                self.compression,
                MAX_DIRECTORY,
            )?;
            directory(
                &content,
                self.leaf.end - self.leaf.start,
                self.data.end - self.data.start,
            )?;
        } else if includes(&self.leaf, &range) {
            let content = unpack(&bytes, self.compression, MAX_DIRECTORY)?;
            directory(
                &content,
                self.leaf.end - self.leaf.start,
                self.data.end - self.data.start,
            )?;
        } else if range == self.metadata {
            let content = unpack(&bytes, self.compression, MAX_DIRECTORY)?;
            serde_json::from_slice::<serde_json::Value>(&content).map_err(|_| invalid())?;
        }
        Ok(bytes.into())
    }
}
impl std::fmt::Display for Archive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("local map")
    }
}
#[async_trait::async_trait]
impl ObjectStore for Archive {
    async fn get_opts(&self, path: &Path, options: GetOptions) -> Result<GetResult> {
        if path.as_ref() != "archive" || options.head {
            return Err(invalid());
        }
        let Some(GetRange::Bounded(mut range)) = options.range else {
            return Err(invalid());
        };
        if range.start == 0 && range.end == 16384 {
            range.end = range.end.min(self.size);
        }
        if range.start >= range.end
            || range.end - range.start > MAX_TILE
            || range.end > self.size
            || !((range.start == 0 && range.end == self.size.min(16384))
                || range == self.metadata
                || includes(&self.leaf, &range)
                || includes(&self.data, &range))
        {
            return Err(invalid());
        }
        let bytes = self.read(range.clone()).await?;
        Ok(GetResult {
            payload: GetResultPayload::Stream(stream::once(async move { Ok(bytes) }).boxed()),
            meta: ObjectMeta {
                location: path.clone(),
                last_modified: self.modified.into(),
                size: self.size,
                e_tag: Some(format!("{:?}", self.modified)),
                version: None,
            },
            range,
            attributes: Attributes::default(),
            extensions: Default::default(),
        })
    }
    async fn put_opts(&self, _: &Path, _: PutPayload, _: PutOptions) -> Result<PutResult> {
        unsupported()
    }
    async fn put_multipart_opts(
        &self,
        _: &Path,
        _: PutMultipartOptions,
    ) -> Result<Box<dyn MultipartUpload>> {
        unsupported()
    }
    fn delete_stream(
        &self,
        _: BoxStream<'static, Result<Path>>,
    ) -> BoxStream<'static, Result<Path>> {
        stream::once(async { unsupported() }).boxed()
    }
    fn list(&self, _: Option<&Path>) -> BoxStream<'static, Result<ObjectMeta>> {
        stream::once(async { unsupported() }).boxed()
    }
    async fn list_with_delimiter(&self, _: Option<&Path>) -> Result<ListResult> {
        unsupported()
    }
    async fn copy_opts(&self, _: &Path, _: &Path, _: CopyOptions) -> Result<()> {
        unsupported()
    }
}
