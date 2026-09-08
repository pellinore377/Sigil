#![forbid(unsafe_code)]
mod archive;
mod assets;
pub use assets::Assets;
use martin_core::{
    tiles::{
        pmtiles::{PmtCache, PmtCacheInstance, PmtilesSource},
        Source,
    },
    CacheZoomRange,
};
use martin_tile_utils::TileCoord;
use std::path::Path;
use tokio::sync::Semaphore;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Unavailable,
    Busy,
}
pub struct Tiles {
    source: PmtilesSource,
    slots: Semaphore,
}
impl Tiles {
    pub async fn open(path: &Path) -> Result<Self, Error> {
        let archive = archive::Archive::open(path).map_err(|_| Error::Invalid)?;
        let source = PmtilesSource::new(
            PmtCacheInstance::new_auto_id(PmtCache::new(8 * 1024 * 1024, None, None)),
            "local".into(),
            Box::new(archive),
            "archive",
            CacheZoomRange::disabled(),
        )
        .await
        .map_err(|_| Error::Invalid)?;
        Ok(Self {
            source,
            slots: Semaphore::new(4),
        })
    }
    pub async fn tile(&self, z: u8, x: u32, y: u32) -> Result<Vec<u8>, Error> {
        if z > 26 || x >= 1 << z || y >= 1 << z {
            return Err(Error::Invalid);
        }
        let _slot = self.slots.try_acquire().map_err(|_| Error::Busy)?;
        self.source
            .get_tile(TileCoord { z, x, y }, None)
            .await
            .map_err(|_| Error::Unavailable)
    }
    pub fn metadata(&self) -> Result<serde_json::Value, Error> {
        let mut value =
            serde_json::to_value(self.source.get_tilejson()).map_err(|_| Error::Invalid)?;
        if let Some(source) = value.get("attribution").and_then(serde_json::Value::as_str) {
            value["attribution"] = assets::attribution(source)?.into();
        }
        Ok(value)
    }
    pub fn content_type(&self) -> String {
        self.source.get_tile_info().format.content_type().into()
    }
    pub fn encoding(&self) -> Option<&'static str> {
        self.source.get_tile_info().encoding.compression()
    }
}
