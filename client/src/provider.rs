//! An OpenMLS provider whose storage persists to a file. The memory
//! storage is a map of bytes to bytes; we dump it after every operation.

use openmls_memory_storage::MemoryStorage;
use openmls_rust_crypto::RustCrypto;
use openmls_traits::OpenMlsProvider;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct SigilProvider {
    crypto: RustCrypto,
    storage: MemoryStorage,
    path: PathBuf,
}

impl OpenMlsProvider for SigilProvider {
    type CryptoProvider = RustCrypto;
    type RandProvider = RustCrypto;
    type StorageProvider = MemoryStorage;
    fn storage(&self) -> &MemoryStorage {
        &self.storage
    }
    fn crypto(&self) -> &RustCrypto {
        &self.crypto
    }
    fn rand(&self) -> &RustCrypto {
        &self.crypto
    }
}

impl SigilProvider {
    pub fn open(path: &Path) -> anyhow::Result<SigilProvider> {
        let storage = MemoryStorage::default();
        if path.exists() {
            let bytes = std::fs::read(path)?;
            let map: HashMap<String, String> = serde_json::from_slice(&bytes)?;
            let mut values = storage.values.write().unwrap();
            for (k, v) in map {
                values.insert(hex::decode(k)?, hex::decode(v)?);
            }
        }
        Ok(SigilProvider {
            crypto: RustCrypto::default(),
            storage,
            path: path.to_path_buf(),
        })
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let values = self.storage.values.read().unwrap();
        let map: HashMap<String, String> = values
            .iter()
            .map(|(k, v)| (hex::encode(k), hex::encode(v)))
            .collect();
        std::fs::write(&self.path, serde_json::to_vec(&map)?)?;
        Ok(())
    }
}
